//! Batch upload pipeline (Wave 13 / desktop-video-upload).
//!
//! The daemon's live-phone path streams BGR24 frames to the CV sidecar
//! and forwards detections to the GUI. This module is its **batch
//! sibling**: a clip is staged on disk under `<data_dir>/uploads/<blake3>/`,
//! the sidecar decodes it via `cv2.VideoCapture`, and per-frame
//! detections flow back through the same wire format. After the
//! terminator response, [`report::ClipReport::build`] aggregates the
//! detection stream into a persistent `report.json` artifact.
//!
//! The Phase 3 surface is intentionally a pure-logic module — no I/O
//! beyond the explicit `write_atomic` call, no time, no globals. The
//! daemon (Wave B) wires the upload processor and the iroh-blobs ALPN
//! around it.

pub mod handler;
pub mod processor;
pub mod protocol;
pub mod queue;
pub mod report;
pub mod store;
