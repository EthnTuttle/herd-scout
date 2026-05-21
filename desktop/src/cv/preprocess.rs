//! Video frame → ONNX-friendly `Array4<f32>` tensor.
//!
//! Pipeline (locked in `cv-design.md`):
//!
//! 1. Materialize the frame's RGBA buffer (`VideoFrame::rgba_image`,
//!    lazy + cached).
//! 2. Straight-stretch resize to 640×640 with `image::imageops::resize`
//!    using `FilterType::Triangle`. Letterboxing is deferred — design
//!    doc accepts the aspect distortion for MVP.
//! 3. Drop alpha (RGBA → RGB), normalize to `f32 / 255.0`, transpose
//!    HWC → CHW, prepend a batch dim. Final shape `[1, 3, 640, 640]`.

use anyhow::Result;
use image::imageops::FilterType;
use iroh_live::media::format::VideoFrame;
use ndarray::Array4;

/// Side length of the network input (square).
pub const INPUT_SIZE: u32 = 640;

/// Convert a decoded video frame into a contiguous `[1, 3, 640, 640]`
/// f32 tensor in CHW layout, normalized to `[0, 1]`.
///
/// Errors if the frame is genuinely degenerate (zero width or height —
/// shouldn't happen in practice from rusty-codecs but we guard anyway).
pub fn frame_to_chw_tensor(frame: &VideoFrame) -> Result<Array4<f32>> {
    let w = frame.width();
    let h = frame.height();
    if w == 0 || h == 0 {
        anyhow::bail!("video frame has zero dimension: {w}×{h}");
    }

    // 1. Lazy materialize the RGBA buffer (cached on the VideoFrame).
    let rgba = frame.rgba_image();

    // 2. Straight-stretch resize to 640×640. Triangle gives a good
    // speed/quality trade for downscale; for ~30 FPS source → 10 FPS
    // inference on a 2020+ CPU this is well within budget.
    let resized = image::imageops::resize(rgba, INPUT_SIZE, INPUT_SIZE, FilterType::Triangle);

    // 3. RGBA → RGB → CHW f32 normalized.
    //
    // The output ndarray is allocated contiguous; we fill in CHW order
    // directly to avoid a separate transpose pass.
    let size = INPUT_SIZE as usize;
    let mut tensor = Array4::<f32>::zeros((1, 3, size, size));

    // image::ImageBuffer stores rows top-to-bottom, pixels left-to-right,
    // RGBA in 4-byte chunks.
    let raw = resized.as_raw();
    debug_assert_eq!(raw.len(), size * size * 4);

    for y in 0..size {
        for x in 0..size {
            let i = (y * size + x) * 4;
            let r = raw[i] as f32 / 255.0;
            let g = raw[i + 1] as f32 / 255.0;
            let b = raw[i + 2] as f32 / 255.0;
            // alpha (raw[i + 3]) is dropped intentionally
            tensor[[0, 0, y, x]] = r;
            tensor[[0, 1, y, x]] = g;
            tensor[[0, 2, y, x]] = b;
        }
    }

    Ok(tensor)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn shape_is_one_three_six_forty_six_forty() {
        let raw = vec![128u8; 1280 * 720 * 4];
        let frame = VideoFrame::new_cpu(raw, 1280, 720, Duration::ZERO);
        let tensor = frame_to_chw_tensor(&frame).expect("preprocess succeeds");
        assert_eq!(tensor.shape(), &[1, 3, 640, 640]);
    }

    #[test]
    fn pixel_values_are_normalized_into_zero_one() {
        let raw = vec![255u8; 64 * 64 * 4];
        let frame = VideoFrame::new_cpu(raw, 64, 64, Duration::ZERO);
        let tensor = frame_to_chw_tensor(&frame).expect("preprocess succeeds");
        // After upscale all channels should remain ~1.0.
        let v = tensor[[0, 0, 0, 0]];
        assert!((v - 1.0).abs() < 1e-3, "pixel value {v} not ~1.0");
    }

    #[test]
    fn zero_dimension_errors() {
        // Avoid using `bytes::Bytes` directly to keep test code free
        // of the `bytes` direct dependency. `new_cpu` accepts `Vec<u8>`.
        let frame = VideoFrame::new_cpu(Vec::new(), 0, 0, Duration::ZERO);
        assert!(frame_to_chw_tensor(&frame).is_err());
    }
}
