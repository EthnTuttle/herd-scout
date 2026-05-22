//! YOLOv5 raw output tensor → filtered, NMS-ed `Vec<Detection>` in
//! original-frame pixel coordinates.
//!
//! Output layout (`[1, 25200, 85]`):
//!
//! Each of the 25 200 anchor rows is `[cx, cy, w, h, obj_conf, c0, c1,
//! …, c79]`. Coordinates live in 640×640 input space (the model has a
//! built-in `Detect` head). We:
//!
//! 1. Mask all classes other than the three we care about.
//! 2. Reject rows with `obj_conf * max_class_score < 0.25`.
//! 3. Convert (cx,cy,w,h) → (x1,y1,x2,y2) and scale back to source
//!    dimensions using the straight-stretch ratio.
//! 4. Run per-class NMS (IoU 0.45) over the survivors.

use anyhow::{Result, bail};
use ndarray::ArrayViewD;

use super::model::{CocoClass, Detection};
use super::preprocess::INPUT_SIZE;

/// Confidence threshold below which detections are dropped pre-NMS.
const CONF_THRESHOLD: f32 = 0.25;

/// IoU threshold for NMS. Pairs with IoU above this are suppressed.
const IOU_THRESHOLD: f32 = 0.45;

/// Number of class scores in COCO80.
const NUM_CLASSES: usize = 80;

/// Length of one anchor row in the output tensor: `cx, cy, w, h,
/// obj_conf` plus 80 class scores.
const ROW_LEN: usize = 5 + NUM_CLASSES;

/// Decode a YOLOv5 output tensor into per-class detections in source
/// pixel coordinates.
///
/// Errors if the output shape is unexpected (e.g. someone swapped in a
/// non-YOLOv5 model). Callers should treat `Err` as "CV disabled" per
/// the design doc's failure-mode list.
pub fn decode_yolov5(
    output: &ArrayViewD<'_, f32>,
    src_w: u32,
    src_h: u32,
) -> Result<Vec<Detection>> {
    // Shape sanity — accept either [1, N, 85] or [N, 85] (some
    // exporters strip the batch dim).
    let (rows, row_len) = match output.shape() {
        [1, n, k] => (*n, *k),
        [n, k] => (*n, *k),
        other => bail!("unexpected output shape: {other:?}"),
    };
    if row_len != ROW_LEN {
        bail!("unexpected row length: {row_len} (want {ROW_LEN})");
    }

    let scale_x = src_w as f32 / INPUT_SIZE as f32;
    let scale_y = src_h as f32 / INPUT_SIZE as f32;

    // Flatten to `[rows, ROW_LEN]` 2-D view for per-row indexing.
    let flat = output
        .view()
        .into_shape_with_order((rows, row_len))
        .map_err(|e| anyhow::anyhow!("output reshape failed: {e}"))?;

    let mut candidates: Vec<Detection> = Vec::new();

    for r in 0..rows {
        let row = flat.row(r);
        let obj_conf = row[4];
        if obj_conf < CONF_THRESHOLD {
            continue;
        }

        // Argmax over the three classes we care about. Anything else
        // is masked — explicitly do not consider those scores.
        let mut best: Option<(CocoClass, f32)> = None;
        for &(idx, class) in &[
            (17usize, CocoClass::Horse),
            (18, CocoClass::Sheep),
            (19, CocoClass::Cow),
        ] {
            let cls_score = row[5 + idx];
            let final_score = obj_conf * cls_score;
            if final_score < CONF_THRESHOLD {
                continue;
            }
            if best.is_none_or(|(_, s)| final_score > s) {
                best = Some((class, final_score));
            }
        }

        let Some((class, score)) = best else {
            continue;
        };

        let cx = row[0];
        let cy = row[1];
        let w = row[2];
        let h = row[3];

        let x1 = (cx - w / 2.0) * scale_x;
        let y1 = (cy - h / 2.0) * scale_y;
        let x2 = (cx + w / 2.0) * scale_x;
        let y2 = (cy + h / 2.0) * scale_y;

        // Clamp to source frame bounds.
        let x1 = x1.clamp(0.0, src_w as f32);
        let y1 = y1.clamp(0.0, src_h as f32);
        let x2 = x2.clamp(0.0, src_w as f32);
        let y2 = y2.clamp(0.0, src_h as f32);

        if x2 <= x1 || y2 <= y1 {
            continue;
        }

        candidates.push(Detection {
            class,
            bbox: [x1, y1, x2, y2],
            score,
        });
    }

    Ok(non_max_suppression(candidates, IOU_THRESHOLD))
}

/// Class-aware non-max suppression. Keeps the highest-scoring box per
/// class and suppresses any same-class candidate with IoU > `iou`.
fn non_max_suppression(mut dets: Vec<Detection>, iou: f32) -> Vec<Detection> {
    // Stable-sort by descending score.
    dets.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    let mut kept: Vec<Detection> = Vec::with_capacity(dets.len());
    for cand in dets {
        let suppressed = kept
            .iter()
            .any(|k| k.class == cand.class && box_iou(&k.bbox, &cand.bbox) > iou);
        if !suppressed {
            kept.push(cand);
        }
    }
    kept
}

fn box_iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let inter_x1 = a[0].max(b[0]);
    let inter_y1 = a[1].max(b[1]);
    let inter_x2 = a[2].min(b[2]);
    let inter_y2 = a[3].min(b[3]);
    let iw = (inter_x2 - inter_x1).max(0.0);
    let ih = (inter_y2 - inter_y1).max(0.0);
    let inter = iw * ih;
    let area_a = (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0);
    let area_b = (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
    let union = area_a + area_b - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

#[cfg(test)]
mod tests {
    use ndarray::Array3;

    use super::*;

    /// Build a synthetic `[1, 25200, 85]` output where exactly two
    /// rows score above threshold for the cow class — and they overlap
    /// >90 %, so NMS should keep just the higher-confidence one.
    #[test]
    fn nms_collapses_duplicate_cow() {
        let rows = 25_200;
        let mut data = Array3::<f32>::zeros((1, rows, ROW_LEN));

        // Row 0: a high-confidence cow box.
        data[[0, 0, 0]] = 320.0; // cx
        data[[0, 0, 1]] = 320.0; // cy
        data[[0, 0, 2]] = 200.0; // w
        data[[0, 0, 3]] = 200.0; // h
        data[[0, 0, 4]] = 0.9; // obj
        data[[0, 0, 5 + 19]] = 0.95; // cow

        // Row 1: a near-duplicate, slightly lower score.
        data[[0, 1, 0]] = 322.0;
        data[[0, 1, 1]] = 318.0;
        data[[0, 1, 2]] = 198.0;
        data[[0, 1, 3]] = 200.0;
        data[[0, 1, 4]] = 0.85;
        data[[0, 1, 5 + 19]] = 0.9;

        let view = data.view().into_dyn();
        let dets = decode_yolov5(&view, 1280, 720).expect("decode");
        assert_eq!(dets.len(), 1, "expected NMS to collapse the duplicate");
        assert_eq!(dets[0].class, CocoClass::Cow);
    }

    #[test]
    fn class_mask_drops_other_classes() {
        let rows = 25_200;
        let mut data = Array3::<f32>::zeros((1, rows, ROW_LEN));

        // High-confidence "cat" (class 15) — must be dropped.
        data[[0, 0, 0]] = 100.0;
        data[[0, 0, 1]] = 100.0;
        data[[0, 0, 2]] = 80.0;
        data[[0, 0, 3]] = 80.0;
        data[[0, 0, 4]] = 0.99;
        data[[0, 0, 5 + 15]] = 0.99;

        let view = data.view().into_dyn();
        let dets = decode_yolov5(&view, 640, 640).expect("decode");
        assert!(dets.is_empty(), "expected non-target class to be masked out");
    }

    #[test]
    fn unexpected_shape_errors() {
        let bad = Array3::<f32>::zeros((1, 10, 20));
        let view = bad.view().into_dyn();
        assert!(decode_yolov5(&view, 640, 640).is_err());
    }
}
