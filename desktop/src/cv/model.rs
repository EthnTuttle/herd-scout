//! ONNX Runtime YOLOv5n session wrapper plus the `Detection` /
//! `CocoClass` data types shared between the inference task and the
//! egui paint loop.

use std::path::PathBuf;

use anyhow::Result;
use iroh_live::media::format::VideoFrame;
use ort::session::{Session, builder::GraphOptimizationLevel};

use super::{postprocess, preprocess};

/// COCO class indices we care about. The full COCO 80-class list is
/// available at `ultralytics/yolov5/data/coco.yaml`; only these three
/// are surfaced in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CocoClass {
    /// COCO class 17.
    Horse,
    /// COCO class 18.
    Sheep,
    /// COCO class 19.
    Cow,
}

impl CocoClass {
    /// Map a COCO80 class id to the subset herd-scout cares about.
    /// Returns `None` for any of the other 77 classes; postprocess
    /// uses this to mask the model output.
    #[allow(dead_code, reason = "kept for symmetry with the postprocess class mask + future use")]
    pub fn from_index(idx: usize) -> Option<Self> {
        match idx {
            17 => Some(Self::Horse),
            18 => Some(Self::Sheep),
            19 => Some(Self::Cow),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Horse => "horse",
            Self::Sheep => "sheep",
            Self::Cow => "cow",
        }
    }

    /// Per-class colour for the egui overlay. Tuple is `(r, g, b)` —
    /// the alpha is added at draw time.
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            // orange
            Self::Cow => (255, 165, 0),
            // cyan
            Self::Horse => (0, 200, 255),
            // magenta
            Self::Sheep => (240, 50, 230),
        }
    }
}

/// A single detection in source-frame pixel coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Detection {
    pub class: CocoClass,
    /// `[x1, y1, x2, y2]` in original frame pixel space (top-left origin).
    pub bbox: [f32; 4],
    pub score: f32,
}

/// Bytes of the YOLOv5n ONNX model, baked into the binary.
///
/// `include_bytes!` will refuse to compile if the file is missing —
/// that is the deliberate fail-loudly point per the design doc. See
/// `desktop/assets/README.md` for how to obtain the model.
const MODEL_BYTES: &[u8] = include_bytes!("../../assets/yolov5n.onnx");

/// Owning wrapper around the ORT inference session.
///
/// Construction can fail (corrupt model bytes, ORT init issue, etc.)
/// but the streaming pipeline must keep running without overlays in
/// that case. Callers that want soft-fail behaviour should match on
/// the `Result` returned by [`Detector::new`] and continue with a
/// "CV disabled" snapshot.
pub struct Detector {
    session: Session,
}

impl std::fmt::Debug for Detector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Detector").finish_non_exhaustive()
    }
}

impl Detector {
    /// Builds a CPU-only ORT session from the embedded YOLOv5n model.
    ///
    /// Errors if `ort` cannot initialize, the model bytes are corrupt,
    /// or graph optimization fails. The caller is expected to log + set
    /// "CV disabled" if this returns `Err`.
    pub fn new() -> Result<Self> {
        // `ort::Error` is not `Send + Sync + StdError`, so anyhow's
        // `Context` extension doesn't apply directly. Stringify the
        // ort error and re-wrap with anyhow.
        let session = Session::builder()
            .map_err(|e| anyhow::anyhow!("Session::builder failed: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|e| anyhow::anyhow!("set optimization level failed: {e}"))?
            .with_intra_threads(1)
            .map_err(|e| anyhow::anyhow!("set intra threads failed: {e}"))?
            .commit_from_memory(MODEL_BYTES)
            .map_err(|e| anyhow::anyhow!("commit_from_memory failed (corrupt yolov5n.onnx?): {e}"))?;

        Ok(Self { session })
    }

    /// Runs the model on a single frame. Returns the kept detections in
    /// **original frame** pixel coordinates after class mask + NMS.
    ///
    /// `&mut self` because `ort::Session::run` requires `&mut`.
    pub fn infer(&mut self, frame: &VideoFrame) -> Result<Vec<Detection>> {
        let orig_w = frame.width();
        let orig_h = frame.height();

        let input = preprocess::frame_to_chw_tensor(frame)?;

        let input_tensor = ort::value::TensorRef::from_array_view(input.view())
            .map_err(|e| anyhow::anyhow!("wrap ndarray as TensorRef failed: {e}"))?;

        let outputs = self
            .session
            .run(ort::inputs![input_tensor])
            .map_err(|e| anyhow::anyhow!("session.run failed: {e}"))?;

        // YOLOv5 ONNX export has a single output; index by position.
        let out = &outputs[0];
        let array = out
            .try_extract_array::<f32>()
            .map_err(|e| anyhow::anyhow!("output[0] tensor extract failed: {e}"))?;

        let dets = postprocess::decode_yolov5(&array, orig_w, orig_h)?;

        Ok(dets)
    }
}

/// Path to where the model file *should* live on disk. Useful for
/// log messages even though we bake it in at compile time.
#[allow(dead_code, reason = "kept for future logging / `--print-model-path` CLI")]
pub fn model_path_for_logging() -> PathBuf {
    PathBuf::from("desktop/assets/yolov5n.onnx")
}
