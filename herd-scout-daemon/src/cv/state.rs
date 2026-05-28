//! Shared state between the CV inference task and the egui paint loop.
//!
//! The design doc picks "shared state, not a channel" because the
//! display task always wants the **latest** detections, never the full
//! event stream. We use [`parking_lot::RwLock`] over `std::sync::RwLock`
//! to avoid `.unwrap()`-ing on poisoning every frame.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use super::model::{CocoClass, Detection};

/// Width of the rolling-window for class counts. Per the design doc:
/// "rolling 1-second window max … hides per-frame jitter."
pub const COUNT_WINDOW: Duration = Duration::from_secs(1);

/// Latest detection result plus liveness metadata.
#[derive(Debug, Clone)]
pub struct DetectionSnapshot {
    /// Detections from the most recent successful inference, in source
    /// (original frame) pixel coordinates.
    pub detections: Vec<Detection>,
    /// Source frame timestamp the detections came from. Useful when we
    /// later want to project boxes onto the same frame the user is
    /// seeing.
    pub frame_pts: Duration,
    /// Wall-clock instant when inference completed.
    pub inferred_at: Instant,
    /// `true` if we've already failed to construct/run the session.
    /// When set, the UI shows a discreet "CV disabled" hint and skips
    /// the overlay; video keeps playing normally.
    pub disabled: bool,
    /// Optional banner string (e.g. "CV: model output shape unexpected").
    /// `None` in the happy path.
    pub banner: Option<String>,
    /// Rolling 1-second window of per-class counts, used for the
    /// top-right counter so it doesn't flicker.
    counts_window: VecDeque<CountSample>,
}

#[derive(Debug, Clone, Copy)]
struct CountSample {
    at: Instant,
    counts: ClassCounts,
}

/// Per-class count for the egui counter panel.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClassCounts {
    pub horse: u32,
    pub sheep: u32,
    pub cow: u32,
}

impl ClassCounts {
    pub fn from_detections(dets: &[Detection]) -> Self {
        let mut c = Self::default();
        for d in dets {
            match d.class {
                CocoClass::Horse => c.horse += 1,
                CocoClass::Sheep => c.sheep += 1,
                CocoClass::Cow => c.cow += 1,
            }
        }
        c
    }

    /// Element-wise max with another sample. Used to build the
    /// "max-over-window" rolling counter.
    fn max(self, other: Self) -> Self {
        Self {
            horse: self.horse.max(other.horse),
            sheep: self.sheep.max(other.sheep),
            cow: self.cow.max(other.cow),
        }
    }
}

impl Default for DetectionSnapshot {
    fn default() -> Self {
        Self {
            detections: Vec::new(),
            frame_pts: Duration::ZERO,
            inferred_at: Instant::now(),
            disabled: false,
            banner: None,
            counts_window: VecDeque::new(),
        }
    }
}

impl DetectionSnapshot {
    /// Replaces the detection list and pushes a new sample into the
    /// rolling-window count history. Also drops samples older than
    /// `COUNT_WINDOW`.
    pub fn update(&mut self, dets: Vec<Detection>, frame_pts: Duration, now: Instant) {
        let counts = ClassCounts::from_detections(&dets);
        self.detections = dets;
        self.frame_pts = frame_pts;
        self.inferred_at = now;
        self.banner = None;
        self.disabled = false;

        self.counts_window.push_back(CountSample { at: now, counts });
        let cutoff = now - COUNT_WINDOW;
        while self
            .counts_window
            .front()
            .is_some_and(|s| s.at < cutoff)
        {
            self.counts_window.pop_front();
        }
    }

    /// Mark CV as disabled with a human-readable reason that will be
    /// rendered as a top-of-screen banner.
    pub fn disable(&mut self, reason: impl Into<String>) {
        self.detections.clear();
        self.disabled = true;
        self.banner = Some(reason.into());
    }

    /// Returns the rolling-window per-class max counts.
    pub fn rolling_counts(&self) -> ClassCounts {
        self.counts_window
            .iter()
            .map(|s| s.counts)
            .fold(ClassCounts::default(), ClassCounts::max)
    }

    /// `true` if we have neither detected anything recently nor failed.
    /// The UI uses this to draw a faint "CV idle" hint after 2 s.
    pub fn is_idle(&self, now: Instant, threshold: Duration) -> bool {
        !self.disabled && now.saturating_duration_since(self.inferred_at) > threshold
    }
}

/// Type alias for the shared lock-protected snapshot. Cloning this is
/// cheap (it's just an `Arc` clone).
pub type SharedSnapshot = Arc<RwLock<DetectionSnapshot>>;

/// Construct a fresh shared snapshot wrapped in `Arc<RwLock<_>>`.
pub fn new_shared_snapshot() -> SharedSnapshot {
    Arc::new(RwLock::new(DetectionSnapshot::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cow(score: f32) -> Detection {
        Detection {
            class: CocoClass::Cow,
            bbox: [0.0, 0.0, 10.0, 10.0],
            score,
            track_id: None,
        }
    }

    #[test]
    fn rolling_counts_take_max_over_window() {
        let mut snap = DetectionSnapshot::default();
        let t0 = Instant::now();
        snap.update(vec![cow(0.9), cow(0.8)], Duration::ZERO, t0);
        // Drop one cow on the next sample within window → rolling max
        // should still be 2.
        snap.update(vec![cow(0.85)], Duration::ZERO, t0 + Duration::from_millis(100));
        let counts = snap.rolling_counts();
        assert_eq!(counts.cow, 2);
    }

    #[test]
    fn rolling_counts_evict_old_samples() {
        let mut snap = DetectionSnapshot::default();
        let t0 = Instant::now();
        snap.update(vec![cow(0.9), cow(0.8), cow(0.7)], Duration::ZERO, t0);
        // 1.5 s later — old sample should be evicted; new max is 1.
        snap.update(
            vec![cow(0.85)],
            Duration::ZERO,
            t0 + Duration::from_millis(1500),
        );
        let counts = snap.rolling_counts();
        assert_eq!(counts.cow, 1);
    }

    #[test]
    fn disable_clears_detections_and_sets_banner() {
        let mut snap = DetectionSnapshot::default();
        snap.update(vec![cow(0.9)], Duration::ZERO, Instant::now());
        snap.disable("session init failed");
        assert!(snap.disabled);
        assert!(snap.detections.is_empty());
        assert_eq!(snap.banner.as_deref(), Some("session init failed"));
    }
}
