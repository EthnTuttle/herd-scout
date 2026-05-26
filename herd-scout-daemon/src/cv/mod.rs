//! Computer-vision module: YOLOv5n inference over the live decoded
//! video frame stream, with bounding-box + class-count outputs consumed
//! by the egui paint loop.
//!
//! Wave 3 deliverable, refactored in Wave 6.5: inference moved to a
//! Python sidecar (`deploy/cv-sidecar/`) because the in-process `ort`
//! crate consistently deadlocks during static init on this project's
//! Pascal hardware. The daemon talks to the sidecar over a Unix
//! socket; the sidecar runs YOLOv5n on `onnxruntime-gpu` and ships
//! detections back. See `model::Detector` for the wire protocol.
//!
//! ## Layout
//!
//! * [`model`] — `Detector` (now a sidecar wire client), plus
//!   `Detection` and `CocoClass`.
//! * [`state`] — shared `DetectionSnapshot` between inference task and
//!   the egui paint loop.
//! * [`task`] — the top-level "spawn the inference task" glue called
//!   from `main.rs`.
//!
//! `preprocess` and `postprocess` modules from the in-process build
//! are retained on disk for reference but are no longer wired in;
//! the sidecar handles both phases.

pub mod model;
pub mod state;
pub mod task;

// Re-export the public surface used by `main.rs`. The other types
// stay reachable as `cv::model::Detection` etc. for tests and
// future plumbing.
#[allow(unused_imports, reason = "kept for reuse by future headless dump-to-disk path")]
pub use state::SharedSnapshot;
pub use task::spawn_cv_task;
