//! Computer-vision module: YOLOv5n inference over the live decoded
//! video frame stream, with bounding-box + class-count outputs consumed
//! by the egui paint loop.
//!
//! Wave 3 deliverable. The design doc lives at
//! `desktop/docs/cv-design.md`; this module implements the "Wave 3
//! concrete tasks" section.
//!
//! ## Layout
//!
//! * [`model`] — `Detector` owning the `ort::Session`, plus `Detection`
//!   and `CocoClass`.
//! * [`preprocess`] — `VideoFrame` → `ndarray::Array4<f32>` (HWC→CHW,
//!   normalize, straight-stretch resize 640×640).
//! * [`postprocess`] — output tensor → `Vec<Detection>` (NMS, conf
//!   filter, class mask).
//! * [`state`] — shared `DetectionSnapshot` between inference task and
//!   the egui paint loop.
//! * [`task`] — the top-level "spawn the inference task" glue called
//!   from `main.rs`.

pub mod model;
pub mod postprocess;
pub mod preprocess;
pub mod state;
pub mod task;

// Re-export the public surface used by `main.rs` and `ui.rs`. The
// other types stay reachable as `cv::model::Detection` etc. for tests
// and future plumbing.
pub use state::SharedSnapshot;
pub use task::spawn_cv_task;
