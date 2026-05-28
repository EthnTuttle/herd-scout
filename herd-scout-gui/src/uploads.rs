//! Uploads-panel state for the desktop GUI (Phase 5 of the
//! desktop-video-upload plan).
//!
//! The GUI has two paths to learn about an upload:
//!  1. The user drops / picks a video file. We hash it on a worker
//!     thread and insert a "local-pending" row so the panel reflects
//!     the action immediately, before the daemon has acknowledged.
//!  2. The daemon emits [`ServerMsg::UploadStatus`] frames as the clip
//!     progresses. We merge those into the same row (matched by
//!     blake3_hex), upgrading it to daemon-tracked.
//!
//! State lives behind a single [`parking_lot::RwLock`] so the egui
//! paint loop can take a cheap read-snapshot every frame.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::RwLock;

use herd_scout_ipc::{UploadState, UploadSummaryInline};

/// One row in the uploads panel.
#[derive(Debug, Clone)]
pub struct UploadRow {
    pub blake3_hex: String,
    pub filename: String,
    pub size_bytes: u64,
    pub state: UploadState,
    pub progress_pct: u8,
    pub eta_ms: Option<u64>,
    pub summary: Option<UploadSummaryInline>,
    /// "Local-only" rows are ones the GUI knows about because the user
    /// just dropped a file; they haven't been acknowledged by the daemon
    /// yet. Once we get an `UploadStatus` for the same blake3, we
    /// upgrade the row to daemon-tracked.
    pub local_pending: bool,
    /// Set on local-only rows so the row stays sorted by drop time
    /// before the daemon assigns its own queued_ts.
    pub local_added_at: std::time::Instant,
}

impl UploadRow {
    /// Headline string for the panel — e.g. `"cow 47 (CI [44, 51])"`.
    /// Returns `None` when the row is not `Done` or has no inlined
    /// summary; per-class fields with `count == 0` are omitted on the
    /// secondary line.
    pub fn headline(&self) -> Option<String> {
        if !matches!(self.state, UploadState::Done) {
            return None;
        }
        let s = self.summary.as_ref()?;
        // Pick the dominant class for the headline: the per-class field
        // with the largest count. Fall back to "total" wording when
        // every class is zero.
        let mut classes: Vec<(&str, u32)> = vec![
            ("cow", s.cow),
            ("horse", s.horse),
            ("sheep", s.sheep),
        ];
        classes.sort_by(|a, b| b.1.cmp(&a.1));
        let nonzero: Vec<(&str, u32)> = classes
            .iter()
            .copied()
            .filter(|(_, n)| *n > 0)
            .collect();
        let [lo, hi] = s.bootstrap_ci_95_total;
        let head = if let Some((label, n)) = nonzero.first() {
            format!("{label} {n} (CI [{lo}, {hi}])")
        } else {
            format!(
                "total {} (CI [{lo}, {hi}])",
                s.median_active_count_total
            )
        };
        if nonzero.len() > 1 {
            let extras: Vec<String> = nonzero
                .iter()
                .skip(1)
                .map(|(label, n)| format!("{label} {n}"))
                .collect();
            Some(format!("{head} · {}", extras.join(" · ")))
        } else {
            Some(head)
        }
    }
}

/// Shared uploads state. Cheap to clone; behind a single RwLock so the
/// egui paint loop can read in O(1) per frame.
#[derive(Debug, Default, Clone)]
pub struct UploadsState {
    inner: Arc<RwLock<UploadsInner>>,
}

#[derive(Debug, Default)]
struct UploadsInner {
    /// Indexed by `blake3_hex`.
    rows: BTreeMap<String, UploadRow>,
}

impl UploadsState {
    /// Currently used by tests; the production wiring constructs an
    /// `UploadsState` via `SharedClientState`'s `#[derive(Default)]`.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a "user just dropped this" row before the daemon
    /// acknowledges it. The blake3 here was computed locally.
    pub fn add_local(&self, row: UploadRow) {
        let mut g = self.inner.write();
        // If the daemon already pushed a status for this blake3 (e.g.
        // a duplicate re-drop of a previously processed clip), keep
        // the daemon's row — it has the authoritative state.
        g.rows.entry(row.blake3_hex.clone()).or_insert(row);
    }

    /// Apply a daemon-side `UploadStatus`. Merges with any existing
    /// local-pending row (matched by blake3_hex).
    pub fn apply_status(
        &self,
        blake3_hex: String,
        filename: String,
        state: UploadState,
        progress_pct: u8,
        eta_ms: Option<u64>,
        summary: Option<UploadSummaryInline>,
    ) {
        let mut g = self.inner.write();
        match g.rows.get_mut(&blake3_hex) {
            Some(existing) => {
                existing.state = state;
                existing.progress_pct = progress_pct;
                existing.eta_ms = eta_ms;
                existing.local_pending = false;
                if !filename.is_empty() {
                    existing.filename = filename;
                }
                if summary.is_some() {
                    existing.summary = summary;
                }
            }
            None => {
                g.rows.insert(
                    blake3_hex.clone(),
                    UploadRow {
                        blake3_hex,
                        filename,
                        size_bytes: 0,
                        state,
                        progress_pct,
                        eta_ms,
                        summary,
                        local_pending: false,
                        local_added_at: std::time::Instant::now(),
                    },
                );
            }
        }
    }

    /// Snapshot for rendering. Sorted: pending/decoding first
    /// (newest-dropped at top), then done (newest-finished first), then
    /// failed.
    pub fn snapshot(&self) -> Vec<UploadRow> {
        let g = self.inner.read();
        let mut rows: Vec<UploadRow> = g.rows.values().cloned().collect();
        rows.sort_by(|a, b| {
            let ord_a = state_order(&a.state);
            let ord_b = state_order(&b.state);
            ord_a
                .cmp(&ord_b)
                .then_with(|| b.local_added_at.cmp(&a.local_added_at))
        });
        rows
    }

    pub fn remove(&self, blake3_hex: &str) {
        let mut g = self.inner.write();
        g.rows.remove(blake3_hex);
    }
}

/// Sort key for a state: lower comes first. Pending/Decoding bubble to
/// the top; Done sits in the middle; Failed sinks to the bottom so the
/// active work is always visible at a glance.
fn state_order(s: &UploadState) -> u8 {
    match s {
        UploadState::Queued => 0,
        UploadState::Decoding => 1,
        UploadState::Done => 2,
        UploadState::Failed { .. } => 3,
    }
}

/// Hash a file with BLAKE3 in a background thread and return the hex
/// string + size_bytes.
///
/// The egui paint thread should *never* block on hashing — call this
/// from a worker spawned at drop time and feed the result back via the
/// IPC `ClientMsg::UploadHandoff` channel.
pub fn hash_file_blocking(path: &Path) -> std::io::Result<(String, u64)> {
    use std::io::Read;
    let meta = std::fs::metadata(path)?;
    let size = meta.len();
    let mut f = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hex = hasher.finalize().to_hex().to_string();
    Ok((hex, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_row(hex: &str, name: &str, state: UploadState) -> UploadRow {
        UploadRow {
            blake3_hex: hex.to_string(),
            filename: name.to_string(),
            size_bytes: 1234,
            state,
            progress_pct: 0,
            eta_ms: None,
            summary: None,
            local_pending: true,
            local_added_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn add_local_then_apply_status_merges() {
        let st = UploadsState::new();
        let local = mk_row("aa".repeat(32).as_str(), "drop.mp4", UploadState::Queued);
        let hex = local.blake3_hex.clone();
        st.add_local(local);
        // Daemon ack: same hex, daemon-supplied filename + Decoding.
        st.apply_status(
            hex.clone(),
            "drop.mp4".to_string(),
            UploadState::Decoding,
            42,
            Some(15_000),
            None,
        );
        let snap = st.snapshot();
        assert_eq!(snap.len(), 1);
        let row = &snap[0];
        assert_eq!(row.blake3_hex, hex);
        assert!(!row.local_pending, "row should no longer be local-pending");
        assert_eq!(row.state, UploadState::Decoding);
        assert_eq!(row.progress_pct, 42);
        assert_eq!(row.eta_ms, Some(15_000));
        // Local-pending row still carries its original size_bytes.
        assert_eq!(row.size_bytes, 1234);
    }

    #[test]
    fn snapshot_orders_pending_decoding_done_failed() {
        let st = UploadsState::new();
        st.add_local(mk_row(&"11".repeat(32), "queued.mp4", UploadState::Queued));
        st.add_local(mk_row(&"22".repeat(32), "decoding.mp4", UploadState::Decoding));
        st.add_local(mk_row(&"33".repeat(32), "done.mp4", UploadState::Done));
        st.add_local(mk_row(
            &"44".repeat(32),
            "failed.mp4",
            UploadState::Failed {
                reason: "boom".into(),
            },
        ));
        let snap = st.snapshot();
        assert_eq!(snap.len(), 4);
        assert_eq!(snap[0].state, UploadState::Queued);
        assert_eq!(snap[1].state, UploadState::Decoding);
        assert_eq!(snap[2].state, UploadState::Done);
        assert!(matches!(snap[3].state, UploadState::Failed { .. }));
    }

    #[test]
    fn remove_drops_row() {
        let st = UploadsState::new();
        let hex = "55".repeat(32);
        st.add_local(mk_row(&hex, "x.mp4", UploadState::Done));
        assert_eq!(st.snapshot().len(), 1);
        st.remove(&hex);
        assert!(st.snapshot().is_empty());
    }

    #[test]
    fn hash_file_matches_canonical() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "herd-scout-gui-uploads-test-{}.bin",
            std::process::id()
        ));
        let payload: &[u8] = b"the quick brown fox jumps over the lazy dog";
        std::fs::write(&path, payload).expect("write fixture");
        let (hex, size) =
            hash_file_blocking(&path).expect("hash_file_blocking");
        let want = blake3::hash(payload).to_hex().to_string();
        assert_eq!(hex, want);
        assert_eq!(size, payload.len() as u64);
        // Best-effort cleanup.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn summary_renders_per_class_when_nonzero() {
        let row = UploadRow {
            blake3_hex: "ab".repeat(32),
            filename: "x.mp4".into(),
            size_bytes: 0,
            state: UploadState::Done,
            progress_pct: 100,
            eta_ms: None,
            summary: Some(UploadSummaryInline {
                median_active_count_total: 47,
                bootstrap_ci_95_total: [44, 51],
                horse: 0,
                sheep: 2,
                cow: 47,
                frame_count: 2624,
                duration_ms: 87_520,
            }),
            local_pending: false,
            local_added_at: std::time::Instant::now(),
        };
        let head = row.headline().expect("done row should produce a headline");
        // Dominant class first.
        assert!(
            head.starts_with("cow 47 (CI [44, 51])"),
            "headline was {head:?}"
        );
        // Sheep is non-zero so it must appear.
        assert!(head.contains("sheep 2"), "headline was {head:?}");
        // Horse is zero so it must NOT appear.
        assert!(!head.contains("horse"), "headline was {head:?}");
    }

    #[test]
    fn headline_returns_none_when_not_done() {
        let row = UploadRow {
            blake3_hex: "ab".repeat(32),
            filename: "x.mp4".into(),
            size_bytes: 0,
            state: UploadState::Decoding,
            progress_pct: 50,
            eta_ms: None,
            summary: Some(UploadSummaryInline {
                median_active_count_total: 5,
                bootstrap_ci_95_total: [4, 6],
                horse: 0,
                sheep: 0,
                cow: 5,
                frame_count: 100,
                duration_ms: 1000,
            }),
            local_pending: false,
            local_added_at: std::time::Instant::now(),
        };
        assert!(row.headline().is_none());
    }
}
