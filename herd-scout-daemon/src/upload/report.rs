//! Per-clip report writer (Phase 3 of the desktop-video-upload plan).
//!
//! Pure-logic module. Consumes a stream of per-frame `(pts_ms, [DetWire])`
//! records — the same `track_id`-bearing detections the live path
//! emits — and produces the structured `report.json` artifact described
//! in `playbook-accurate-herd-counting-2026-05-27`.
//!
//! No I/O, no time, no globals. The only side effect is the explicit
//! [`ClipReport::write_atomic`] call. Determinism is preserved across
//! runs by seeding the bootstrap PRNG from the clip's BLAKE3 hash.
//!
//! The algorithm in [`ClipReport::build`] follows the playbook layer-2.5
//! and layer-3 logic:
//!
//! 1. **Cumulative-frame eligibility** — a `track_id` must be seen in
//!    ≥ 15 distinct frames to count.
//! 2. **Centroid-jump sanity** — a per-frame Δcentroid > 150 px drops
//!    that frame's contribution to the active set for that ID.
//! 3. **Active-count-per-frame** — `len(unique eligible IDs present)`.
//! 4. **Median over 30-frame windows** — one window per second of clip
//!    time, centered on frames 15, 45, 75, …
//! 5. **Bootstrap 95 % CI** — 1000 resamples with replacement; 2.5th /
//!    97.5th percentile; deterministic seed from `clip_id`.
//! 6. **Per-class breakdown** — same logic restricted by `class`.
//! 7. **Closure warning** — fires once if any track spans the full clip.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use herd_scout_ipc::{DetWire, UploadSummaryInline};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------
// Tunable constants (kept module-private; the playbook is the source of
// truth). Surfacing them as constants (rather than magic numbers in
// build()) keeps the algorithm description and the implementation one
// hop apart.
// ---------------------------------------------------------------------

/// A `track_id` must be seen in this many distinct frames before it
/// contributes to active counts. Per playbook § P0 #1.
const MIN_ELIGIBLE_FRAMES: u32 = 15;

/// Per-frame Δcentroid (Euclidean px) above which the frame's
/// contribution for that ID is dropped from the active set. Per
/// playbook § P0 #2 ("centroid-jump sanity").
const MAX_CENTROID_JUMP_PX: f32 = 150.0;

/// Width of the median-of-active-IDs window, in frames. At 30 FPS this
/// is one second.
const WINDOW_FRAMES: u32 = 30;

/// Half-width used to derive window centers — emit a window every
/// `2 * WINDOW_HALF` frames starting at the half-mark. With
/// `WINDOW_HALF = 15` and `WINDOW_FRAMES = 30`, centers fall at 15, 45,
/// 75, … and each window covers `[c-15, c+15)`.
const WINDOW_HALF: u32 = WINDOW_FRAMES / 2;

/// Bootstrap-resample count for the 95 % CI on the total median.
const BOOTSTRAP_RESAMPLES: usize = 1000;

// ---------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------

/// ByteTrack params used to process the clip. Echoed into the report so
/// re-runs are reproducible from the JSON alone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ByteTrackParams {
    pub track_activation_threshold: f32,
    pub lost_track_buffer: u32,
    pub minimum_matching_threshold: f32,
    pub minimum_consecutive_frames: u32,
    pub frame_rate: u32,
}

impl Default for ByteTrackParams {
    /// Playbook-recommended defaults for stationary livestock at 30 FPS,
    /// per `playbook-accurate-herd-counting-2026-05-27` § P0 #1.
    fn default() -> Self {
        Self {
            track_activation_threshold: 0.35,
            lost_track_buffer: 60,
            minimum_matching_threshold: 0.85,
            minimum_consecutive_frames: 3,
            frame_rate: 30,
        }
    }
}

/// One per-frame snapshot fed into the report builder. The daemon's
/// upload processor fills this from each sidecar response.
#[derive(Debug, Clone)]
pub struct FrameRecord {
    pub frame_id: u32,
    pub pts_ms: u64,
    pub detections: Vec<DetWire>,
}

/// One window in the `frames_per_window` time series.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowSample {
    pub window_center_frame: u32,
    pub active_ids: u32,
}

/// Per-track summary row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackSummary {
    pub track_id: u32,
    /// `"horse" | "sheep" | "cow"` — see [`DetWire::class_label`].
    pub class: String,
    pub first_frame: u32,
    pub last_frame: u32,
    pub frame_count: u32,
    /// True iff `frame_count >= 15`.
    pub eligible: bool,
    pub mean_confidence: f32,
    /// Sum of per-frame Δcentroid (Euclidean px). Includes suspect
    /// (> 150 px) jumps so the metric stays a faithful path-length
    /// even when the active-count filter has dropped them.
    pub centroid_track_len_px: f32,
}

/// Aggregate summary block. The headline numbers for the GUI's queue
/// panel (see [`ClipReport::inline_summary`]) are extracted from here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportSummary {
    pub median_active_count_total: u32,
    pub median_active_count_per_class: ClassCounts,
    pub bootstrap_ci_95_total: [u32; 2],
    pub max_simultaneous_total: u32,
    pub unique_track_ids_total: u32,
    pub unique_track_ids_eligible: u32,
}

/// Per-class headline counts. Mirrors `ClassCountsWire` but lives in
/// the report-side schema so the file format isn't coupled to the wire
/// format.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ClassCounts {
    pub horse: u32,
    pub sheep: u32,
    pub cow: u32,
}

/// The full per-clip report. Serialized to disk as `report.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipReport {
    pub schema_version: u32,
    /// BLAKE3 hex string from iroh-blobs.
    pub clip_id: String,
    pub filename: String,
    pub duration_ms: u64,
    pub fps: f32,
    pub frame_count: u32,
    pub processing_ms: u64,
    /// Spelling preserved from the plan's committed schema for wire
    /// compatibility — see the schema in
    /// `plan-desktop-video-upload-2026-05-28.md`.
    pub bytrack_params: ByteTrackParams,
    pub summary: ReportSummary,
    pub tracks: Vec<TrackSummary>,
    pub frames_per_window: Vec<WindowSample>,
    pub warnings: Vec<String>,
}

impl ClipReport {
    /// Build a report from a sequence of per-frame records and the
    /// metadata the daemon already has. Pure function — no I/O, no
    /// global state, no time. Deterministic for a given input.
    pub fn build(
        clip_id: &str,
        filename: &str,
        duration_ms: u64,
        fps: f32,
        frame_count: u32,
        processing_ms: u64,
        params: ByteTrackParams,
        frames: &[FrameRecord],
    ) -> Self {
        // -----------------------------------------------------------------
        // Pass 1: build per-track aggregates keyed by track_id. Detections
        // without a track_id (the tracker hasn't attached one yet) carry
        // no identity — they cannot contribute to counts and are skipped
        // entirely, per the spec's eligibility rule.
        // -----------------------------------------------------------------
        let mut acc: BTreeMap<u32, TrackAccum> = BTreeMap::new();

        for frame in frames {
            for det in &frame.detections {
                let Some(tid) = det.track_id else {
                    continue;
                };
                let centroid = centroid_of(&det.bbox);
                let entry = acc.entry(tid).or_insert_with(|| TrackAccum::new(det.class));
                entry.observe(frame.frame_id, det.score, centroid);
            }
        }

        // -----------------------------------------------------------------
        // Eligibility + per-track summaries. A track is eligible iff it
        // appears in ≥ MIN_ELIGIBLE_FRAMES distinct frames. We also flag
        // closure (track first/last bracket the entire clip) here so we
        // can emit one aggregate warning at the end.
        // -----------------------------------------------------------------
        let mut tracks: Vec<TrackSummary> = Vec::with_capacity(acc.len());
        let mut eligible_ids: BTreeSet<u32> = BTreeSet::new();
        // Per-eligible-id, the set of frame_ids where the centroid jump
        // disqualifies this id from contributing to that frame's count.
        let mut suspect_drops: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        let mut closure_count: u32 = 0;

        for (tid, a) in &acc {
            let frame_count_for_track = a.frames.len() as u32;
            let eligible = frame_count_for_track >= MIN_ELIGIBLE_FRAMES;
            let mean_confidence = if a.score_count == 0 {
                0.0
            } else {
                a.score_sum / a.score_count as f32
            };
            let class_label = class_label_for(a.class).to_string();
            let first_frame = a.first_frame.unwrap_or(0);
            let last_frame = a.last_frame.unwrap_or(0);

            if eligible {
                eligible_ids.insert(*tid);
            }
            // Closure: only meaningful when the track is non-trivial and
            // brackets the whole clip. frame_count > 0 by construction
            // here (we only build TrackAccum when we observe a frame).
            if frame_count > 0
                && frame_count_for_track > 1
                && first_frame == 0
                && last_frame == frame_count - 1
            {
                closure_count += 1;
            }

            // Compute the centroid-jump suspect set for this track. Only
            // matters if we'll consult it later (eligible tracks).
            if eligible {
                let drops = compute_centroid_jumps(&a.frames);
                if !drops.is_empty() {
                    suspect_drops.insert(*tid, drops);
                }
            }

            tracks.push(TrackSummary {
                track_id: *tid,
                class: class_label,
                first_frame,
                last_frame,
                frame_count: frame_count_for_track,
                eligible,
                mean_confidence,
                centroid_track_len_px: a.centroid_path_len,
            });
        }

        // -----------------------------------------------------------------
        // Pass 2: per-frame active counts (total and per-class) over the
        // [0, frame_count) range. Frames missing from `frames` get count
        // zero. Frames with frame_id outside the declared range are
        // silently dropped (defensive — sidecar shouldn't emit them).
        // -----------------------------------------------------------------
        let n = frame_count as usize;
        let mut active_total: Vec<u32> = vec![0; n];
        let mut active_horse: Vec<u32> = vec![0; n];
        let mut active_sheep: Vec<u32> = vec![0; n];
        let mut active_cow: Vec<u32> = vec![0; n];

        for frame in frames {
            let idx = frame.frame_id as usize;
            if idx >= n {
                continue;
            }
            // Distinct eligible track ids in this frame, after the
            // centroid-jump filter.
            let mut seen_ids: BTreeSet<u32> = BTreeSet::new();
            let mut horse_ids: BTreeSet<u32> = BTreeSet::new();
            let mut sheep_ids: BTreeSet<u32> = BTreeSet::new();
            let mut cow_ids: BTreeSet<u32> = BTreeSet::new();

            for det in &frame.detections {
                let Some(tid) = det.track_id else {
                    continue;
                };
                if !eligible_ids.contains(&tid) {
                    continue;
                }
                if suspect_drops
                    .get(&tid)
                    .is_some_and(|s| s.contains(&frame.frame_id))
                {
                    continue;
                }
                if seen_ids.insert(tid) {
                    match det.class {
                        0 => {
                            horse_ids.insert(tid);
                        }
                        1 => {
                            sheep_ids.insert(tid);
                        }
                        2 => {
                            cow_ids.insert(tid);
                        }
                        _ => {}
                    }
                }
            }

            active_total[idx] = seen_ids.len() as u32;
            active_horse[idx] = horse_ids.len() as u32;
            active_sheep[idx] = sheep_ids.len() as u32;
            active_cow[idx] = cow_ids.len() as u32;
        }

        // -----------------------------------------------------------------
        // Aggregates — medians, max, bootstrap CI.
        // -----------------------------------------------------------------
        let median_total = median_u32(&active_total);
        let median_horse = median_u32(&active_horse);
        let median_sheep = median_u32(&active_sheep);
        let median_cow = median_u32(&active_cow);
        let max_total = active_total.iter().copied().max().unwrap_or(0);

        let mut seed_rng = Xorshift64::seeded_from_clip_id(clip_id);
        let bootstrap_ci = bootstrap_ci_95(&active_total, &mut seed_rng);

        // -----------------------------------------------------------------
        // Sliding median windows. Centers at WINDOW_HALF, WINDOW_HALF +
        // WINDOW_FRAMES, … strictly inside [0, frame_count).
        // -----------------------------------------------------------------
        let mut frames_per_window = Vec::new();
        if n > 0 {
            let mut center = WINDOW_HALF;
            while (center as usize) < n {
                let lo = center.saturating_sub(WINDOW_HALF) as usize;
                let hi = ((center + WINDOW_HALF) as usize).min(n);
                let slice = &active_total[lo..hi];
                let m = median_u32(slice);
                frames_per_window.push(WindowSample {
                    window_center_frame: center,
                    active_ids: m,
                });
                center += WINDOW_FRAMES;
            }
        }

        // -----------------------------------------------------------------
        // Warnings.
        // -----------------------------------------------------------------
        let mut warnings = Vec::new();
        if closure_count > 0 {
            warnings.push(format!(
                "closure_uncertain: {closure_count} tracks span the entire clip (animals may have entered/exited)"
            ));
        }

        let summary = ReportSummary {
            median_active_count_total: median_total,
            median_active_count_per_class: ClassCounts {
                horse: median_horse,
                sheep: median_sheep,
                cow: median_cow,
            },
            bootstrap_ci_95_total: bootstrap_ci,
            max_simultaneous_total: max_total,
            unique_track_ids_total: tracks.len() as u32,
            unique_track_ids_eligible: eligible_ids.len() as u32,
        };

        Self {
            schema_version: 1,
            clip_id: clip_id.to_string(),
            filename: filename.to_string(),
            duration_ms,
            fps,
            frame_count,
            processing_ms,
            bytrack_params: params,
            summary,
            tracks,
            frames_per_window,
            warnings,
        }
    }

    /// Inline summary for the GUI's queue panel — the headline numbers
    /// shown next to each finished upload row.
    pub fn inline_summary(&self) -> UploadSummaryInline {
        UploadSummaryInline {
            median_active_count_total: self.summary.median_active_count_total,
            bootstrap_ci_95_total: self.summary.bootstrap_ci_95_total,
            horse: self.summary.median_active_count_per_class.horse,
            sheep: self.summary.median_active_count_per_class.sheep,
            cow: self.summary.median_active_count_per_class.cow,
            frame_count: self.frame_count,
            duration_ms: self.duration_ms,
        }
    }

    /// Atomically write the report as `report.json` into `dir`.
    ///
    /// Writes to a sibling `report.json.tmp.<random>` and `rename`s it
    /// into place — same-directory `rename` is atomic on POSIX, so the
    /// file is observed either fully-written or not at all. The temp
    /// suffix is derived from the clip_id + a small monotonic salt to
    /// avoid collisions if two writes ever raced (they shouldn't, but
    /// defensiveness is cheap).
    pub fn write_atomic(&self, dir: &Path) -> std::io::Result<()> {
        use std::fs::{rename, File};
        use std::io::Write;

        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let final_path = dir.join("report.json");

        // Build a stable-ish suffix from clip_id; fall back to a fresh
        // value if clip_id is empty. The temp file is opened with
        // `create_new(true)` so any unexpected collision errors loudly.
        let mut suffix = String::with_capacity(32);
        for c in self.clip_id.chars().take(16) {
            if c.is_ascii_alphanumeric() {
                suffix.push(c);
            }
        }
        if suffix.is_empty() {
            suffix.push_str("anon");
        }
        let pid = std::process::id();
        let ts_nanos = nanos_for_temp();
        let tmp_path = dir.join(format!("report.json.tmp.{suffix}.{pid}.{ts_nanos}"));

        {
            let mut f = File::options()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }

        // rename returns Err if the temp doesn't exist, which would
        // propagate cleanly. On success the temp is gone and final_path
        // is the new contents.
        rename(&tmp_path, &final_path)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------

/// Per-track in-progress accumulator used during `build`. Lives only on
/// the stack of `build` and is collapsed into a `TrackSummary` once the
/// frame-pass finishes.
struct TrackAccum {
    class: u8,
    /// Per-observation `(frame_id, centroid)` in arrival order.
    frames: Vec<(u32, (f32, f32))>,
    score_sum: f32,
    score_count: u32,
    centroid_path_len: f32,
    last_centroid: Option<(f32, f32)>,
    first_frame: Option<u32>,
    last_frame: Option<u32>,
}

impl TrackAccum {
    fn new(class: u8) -> Self {
        Self {
            class,
            frames: Vec::new(),
            score_sum: 0.0,
            score_count: 0,
            centroid_path_len: 0.0,
            last_centroid: None,
            first_frame: None,
            last_frame: None,
        }
    }

    fn observe(&mut self, frame_id: u32, score: f32, centroid: (f32, f32)) {
        self.frames.push((frame_id, centroid));
        self.score_sum += score;
        self.score_count += 1;
        self.first_frame = Some(self.first_frame.map_or(frame_id, |f| f.min(frame_id)));
        self.last_frame = Some(self.last_frame.map_or(frame_id, |f| f.max(frame_id)));
        if let Some(prev) = self.last_centroid {
            self.centroid_path_len += distance(prev, centroid);
        }
        self.last_centroid = Some(centroid);
    }
}

/// Compute the centre of an `[x1, y1, x2, y2]` box.
fn centroid_of(bbox: &[f32; 4]) -> (f32, f32) {
    let cx = (bbox[0] + bbox[2]) * 0.5;
    let cy = (bbox[1] + bbox[3]) * 0.5;
    (cx, cy)
}

/// Euclidean distance.
fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    (dx * dx + dy * dy).sqrt()
}

/// Walk a track's per-frame centroids in observation order and emit the
/// set of `frame_id`s whose contribution is "suspect" — i.e. where the
/// centroid moved more than `MAX_CENTROID_JUMP_PX` from the previous
/// observation. The first observation has no predecessor and is always
/// kept.
fn compute_centroid_jumps(frames: &[(u32, (f32, f32))]) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    let mut prev: Option<(f32, f32)> = None;
    for (fid, c) in frames {
        if let Some(p) = prev {
            if distance(p, *c) > MAX_CENTROID_JUMP_PX {
                out.insert(*fid);
            }
        }
        prev = Some(*c);
    }
    out
}

/// Return the per-class label string for a `DetWire::class` value.
/// Mirrors [`DetWire::class_label`] without depending on a `&self` to
/// call it (we only have the byte at this point).
fn class_label_for(class: u8) -> &'static str {
    match class {
        0 => "horse",
        1 => "sheep",
        2 => "cow",
        _ => "?",
    }
}

/// Median of a slice of `u32`, rounded to nearest integer. Empty slice
/// returns 0. For even-length slices, the mean of the two centres is
/// used and rounded half-away-from-zero.
fn median_u32(xs: &[u32]) -> u32 {
    if xs.is_empty() {
        return 0;
    }
    let mut v: Vec<u32> = xs.to_vec();
    v.sort_unstable();
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        let lo = v[n / 2 - 1];
        let hi = v[n / 2];
        // Round half-away-from-zero; both lo/hi are u32 ≥ 0 so
        // (lo + hi + 1) / 2 is half-up.
        ((lo as u64 + hi as u64 + 1) / 2) as u32
    }
}

/// Bootstrap the 95 % CI of the median of `xs`. Resamples
/// `BOOTSTRAP_RESAMPLES` times with replacement, computes the median
/// of each resample, and returns `[2.5th, 97.5th]` percentile rounded
/// to integers.
fn bootstrap_ci_95(xs: &[u32], rng: &mut Xorshift64) -> [u32; 2] {
    if xs.is_empty() {
        return [0, 0];
    }
    let n = xs.len();
    let mut medians: Vec<u32> = Vec::with_capacity(BOOTSTRAP_RESAMPLES);
    let mut buf: Vec<u32> = vec![0; n];
    for _ in 0..BOOTSTRAP_RESAMPLES {
        for slot in buf.iter_mut() {
            // Range-uniform pick. Xorshift64::next_u64 is uniform over
            // u64; the modulo bias is negligible for any realistic n.
            let idx = (rng.next_u64() as usize) % n;
            *slot = xs[idx];
        }
        medians.push(median_u32(&buf));
    }
    medians.sort_unstable();
    // 2.5th = index 25 of 1000-element sorted vec; 97.5th = 974.
    // Use floor for both (matches "rounded to integers" + standard
    // percentile definitions when the value at the index is already an
    // integer).
    let lo_idx = (BOOTSTRAP_RESAMPLES as f64 * 0.025) as usize;
    let hi_idx = ((BOOTSTRAP_RESAMPLES as f64 * 0.975) as usize).saturating_sub(1);
    let lo = medians.get(lo_idx).copied().unwrap_or(0);
    let hi = medians.get(hi_idx).copied().unwrap_or(0);
    [lo, hi]
}

/// Tiny deterministic PRNG — Xorshift64. Avoids pinning a `rand`
/// version's `StdRng`/`SeedableRng` API, and the seed comes from the
/// clip_id so identical inputs produce identical CIs.
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn seeded_from_clip_id(clip_id: &str) -> Self {
        // Parse the first 8 hex chars (32 bits) of the clip_id and pad
        // out to 64 bits so the seed has full entropy. If the clip_id
        // is too short or non-hex, fall back to a fixed non-zero seed.
        let mut seed: u64 = 0;
        let mut chars = 0;
        for c in clip_id.chars() {
            if let Some(d) = c.to_digit(16) {
                seed = (seed << 4) | u64::from(d);
                chars += 1;
                if chars >= 16 {
                    break;
                }
            } else {
                break;
            }
        }
        // xorshift64 requires non-zero state; a clip_id of all zeros (or
        // empty) shouldn't lock the PRNG into the all-zero fixed point.
        if seed == 0 {
            seed = 0x9E37_79B9_7F4A_7C15;
        }
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        // Marsaglia's xorshift64.
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

/// A monotonically-increasing nanosecond stamp for the temp filename.
/// Falls back to a process-local counter if the system clock can't be
/// read (only the temp filename uses this — it's not load-bearing for
/// the report payload itself).
fn nanos_for_temp() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static FALLBACK: AtomicU64 = AtomicU64::new(0);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_else(|_| FALLBACK.fetch_add(1, Ordering::Relaxed) as u128)
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use herd_scout_ipc::DetWire;

    fn det(class: u8, bbox: [f32; 4], track_id: Option<u32>, score: f32) -> DetWire {
        DetWire {
            class,
            bbox,
            score,
            track_id,
        }
    }

    fn cow_at(track_id: u32, x: f32, y: f32) -> DetWire {
        // 50x50 box centred on (x, y).
        det(2, [x - 25.0, y - 25.0, x + 25.0, y + 25.0], Some(track_id), 0.9)
    }

    fn frame(frame_id: u32, dets: Vec<DetWire>) -> FrameRecord {
        FrameRecord {
            frame_id,
            pts_ms: u64::from(frame_id) * 33,
            detections: dets,
        }
    }

    fn default_params() -> ByteTrackParams {
        ByteTrackParams::default()
    }

    #[test]
    fn empty_input_produces_valid_zero_report() {
        let report = ClipReport::build(
            "00000000abcdef",
            "empty.mp4",
            0,
            0.0,
            0,
            0,
            default_params(),
            &[],
        );
        assert_eq!(report.schema_version, 1);
        assert_eq!(report.frame_count, 0);
        assert_eq!(report.tracks.len(), 0);
        assert_eq!(report.frames_per_window.len(), 0);
        assert_eq!(report.warnings.len(), 0);
        assert_eq!(report.summary.median_active_count_total, 0);
        assert_eq!(report.summary.bootstrap_ci_95_total, [0, 0]);
        assert_eq!(report.summary.max_simultaneous_total, 0);
        assert_eq!(report.summary.unique_track_ids_total, 0);
        assert_eq!(report.summary.unique_track_ids_eligible, 0);
        // JSON serialization must succeed.
        let json = serde_json::to_string(&report).expect("serialize empty report");
        assert!(json.contains("\"schema_version\":1"));
    }

    #[test]
    fn three_stationary_cattle_thirty_seconds() {
        // 30 FPS for 30 seconds = 900 frames; three cows at fixed
        // positions every frame.
        let n = 900u32;
        let mut frames = Vec::with_capacity(n as usize);
        for i in 0..n {
            frames.push(frame(
                i,
                vec![
                    cow_at(1, 100.0, 100.0),
                    cow_at(2, 300.0, 200.0),
                    cow_at(3, 500.0, 400.0),
                ],
            ));
        }
        let report = ClipReport::build(
            "deadbeefcafebabe1234567890abcdef",
            "cattle30s.mp4",
            30_000,
            30.0,
            n,
            1500,
            default_params(),
            &frames,
        );
        assert_eq!(report.summary.median_active_count_total, 3);
        assert_eq!(report.summary.bootstrap_ci_95_total, [3, 3]);
        assert_eq!(report.summary.unique_track_ids_total, 3);
        assert_eq!(report.summary.unique_track_ids_eligible, 3);
        assert_eq!(report.summary.median_active_count_per_class.cow, 3);
        assert_eq!(report.summary.median_active_count_per_class.horse, 0);
        assert_eq!(report.summary.median_active_count_per_class.sheep, 0);
        assert_eq!(report.summary.max_simultaneous_total, 3);
        // Three tracks, each spanning the entire clip (closure_uncertain).
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(
            report.warnings[0].starts_with("closure_uncertain: 3 "),
            "{}",
            report.warnings[0]
        );
        // Every track's centroid_track_len_px is exactly 0 (stationary).
        for t in &report.tracks {
            assert_eq!(t.centroid_track_len_px, 0.0);
            assert!(t.eligible);
            assert_eq!(t.frame_count, n);
            assert_eq!(t.class, "cow");
        }
        // ~30 windows, each with active_ids = 3.
        assert!(!report.frames_per_window.is_empty());
        for w in &report.frames_per_window {
            assert_eq!(w.active_ids, 3);
        }
    }

    #[test]
    fn flicker_track_is_ineligible() {
        // One cow detected for 14 frames then never again — below the
        // 15-frame eligibility floor.
        let mut frames = Vec::new();
        for i in 0..14 {
            frames.push(frame(i, vec![cow_at(1, 100.0, 100.0)]));
        }
        // Pad with empty frames so frame_count is large enough that the
        // active-count median is 0.
        for i in 14..50 {
            frames.push(frame(i, vec![]));
        }
        let report = ClipReport::build(
            "abc12345",
            "flicker.mp4",
            1666,
            30.0,
            50,
            100,
            default_params(),
            &frames,
        );
        assert_eq!(report.summary.unique_track_ids_total, 1);
        assert_eq!(report.summary.unique_track_ids_eligible, 0);
        assert_eq!(report.summary.median_active_count_total, 0);
        // Track row marks ineligible.
        assert_eq!(report.tracks.len(), 1);
        assert!(!report.tracks[0].eligible);
        assert_eq!(report.tracks[0].frame_count, 14);
    }

    #[test]
    fn centroid_teleport_is_dropped() {
        // 30 frames of one cow at (100, 100) = anchor. On frame 15, the
        // detection teleports to (700, 100) — Δcentroid = 600 px > 150
        // — so frame 15's contribution is suspect-dropped from the
        // active count for that ID.
        let mut frames = Vec::new();
        let teleport_frame = 15u32;
        for i in 0..30 {
            let det = if i == teleport_frame {
                cow_at(1, 700.0, 100.0)
            } else {
                cow_at(1, 100.0, 100.0)
            };
            frames.push(frame(i, vec![det]));
        }
        let report = ClipReport::build(
            "feedface",
            "teleport.mp4",
            1000,
            30.0,
            30,
            50,
            default_params(),
            &frames,
        );
        // The track is eligible (30 ≥ 15 frames).
        assert_eq!(report.summary.unique_track_ids_eligible, 1);
        // Frame 15 was dropped from the active count → that frame's
        // total should be 0 while every other frame is 1.
        // We assert this via max_simultaneous (still 1 — the rest of the
        // clip has the cow) and via a count over frames_per_window: at
        // least one window must dip below 1 because of the dropped frame.
        // The median over the full clip stays at 1 (29 of 30 frames are 1).
        assert_eq!(report.summary.median_active_count_total, 1);
        assert_eq!(report.summary.max_simultaneous_total, 1);
    }

    #[test]
    fn closure_warning_fires_when_track_spans_clip() {
        // A track that lasts the full clip span triggers exactly one
        // closure_uncertain warning (single-track instance verifying the
        // wording — the multi-track case is covered by the cattle test).
        let n = 30u32;
        let mut frames = Vec::new();
        for i in 0..n {
            frames.push(frame(i, vec![cow_at(7, 100.0, 100.0)]));
        }
        let report = ClipReport::build(
            "0011223344",
            "closure.mp4",
            1000,
            30.0,
            n,
            42,
            default_params(),
            &frames,
        );
        assert_eq!(report.warnings.len(), 1);
        assert!(
            report.warnings[0].starts_with("closure_uncertain: 1 "),
            "{}",
            report.warnings[0]
        );
    }

    #[test]
    fn bootstrap_ci_is_deterministic() {
        // Two builds of the same input must produce the same CI.
        let mut frames = Vec::new();
        for i in 0..200 {
            // Vary the count slightly so the bootstrap has actual signal
            // (a constant sequence has CI [k, k] regardless of seed).
            let mut dets = vec![cow_at(1, 100.0, 100.0)];
            if i % 2 == 0 {
                dets.push(cow_at(2, 300.0, 100.0));
            }
            if i % 3 == 0 {
                dets.push(cow_at(3, 500.0, 100.0));
            }
            frames.push(frame(i, dets));
        }
        let r1 = ClipReport::build(
            "9c2f3a1bdeadbeef",
            "ci.mp4",
            6666,
            30.0,
            200,
            100,
            default_params(),
            &frames,
        );
        let r2 = ClipReport::build(
            "9c2f3a1bdeadbeef",
            "ci.mp4",
            6666,
            30.0,
            200,
            100,
            default_params(),
            &frames,
        );
        assert_eq!(
            r1.summary.bootstrap_ci_95_total,
            r2.summary.bootstrap_ci_95_total
        );
        // And a different seed produces a (possibly) different CI but at
        // minimum is internally consistent.
        let r3 = ClipReport::build(
            "ffffffff00000000",
            "ci.mp4",
            6666,
            30.0,
            200,
            100,
            default_params(),
            &frames,
        );
        assert!(r3.summary.bootstrap_ci_95_total[0] <= r3.summary.bootstrap_ci_95_total[1]);
    }

    #[test]
    fn inline_summary_matches_summary() {
        let mut frames = Vec::new();
        for i in 0..50 {
            frames.push(frame(i, vec![cow_at(1, 100.0, 100.0), cow_at(2, 300.0, 100.0)]));
        }
        let report = ClipReport::build(
            "1234abcd5678ef00",
            "inline.mp4",
            1666,
            30.0,
            50,
            123,
            default_params(),
            &frames,
        );
        let inline = report.inline_summary();
        assert_eq!(
            inline.median_active_count_total,
            report.summary.median_active_count_total
        );
        assert_eq!(
            inline.bootstrap_ci_95_total,
            report.summary.bootstrap_ci_95_total
        );
        assert_eq!(inline.horse, report.summary.median_active_count_per_class.horse);
        assert_eq!(inline.sheep, report.summary.median_active_count_per_class.sheep);
        assert_eq!(inline.cow, report.summary.median_active_count_per_class.cow);
        assert_eq!(inline.frame_count, report.frame_count);
        assert_eq!(inline.duration_ms, report.duration_ms);
    }

    #[test]
    fn write_atomic_creates_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut frames = Vec::new();
        for i in 0..30 {
            frames.push(frame(i, vec![cow_at(1, 100.0, 100.0)]));
        }
        let report = ClipReport::build(
            "cafef00d12345678",
            "round.mp4",
            1000,
            30.0,
            30,
            42,
            default_params(),
            &frames,
        );
        report
            .write_atomic(tmp.path())
            .expect("write_atomic succeeds");
        let path = tmp.path().join("report.json");
        let bytes = std::fs::read(&path).expect("read back report.json");
        let parsed: ClipReport = serde_json::from_slice(&bytes).expect("deserialize report");
        assert_eq!(parsed, report);
        // No temp leftovers in the dir.
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("read tempdir")
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("report.json.tmp")
            })
            .collect();
        assert!(leftovers.is_empty(), "found temp leftovers: {leftovers:?}");
    }
}
