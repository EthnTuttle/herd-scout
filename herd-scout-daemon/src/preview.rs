//! JPEG preview encoder for IPC fan-out to the GUI.
//!
//! - Downscale incoming RGBA frames to fit within 1280x720.
//! - JPEG-encode at quality 80.
//! - Cap the emit rate at 15 FPS to keep one CPU core free on Pi-class
//!   hardware (per design doc Risk #3).
//!
//! The CV path runs on the original full-resolution RGBA before the
//! downscale, so detection accuracy is unaffected by preview pacing.

use std::io::Cursor;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use iroh_live::media::format::VideoFrame;

/// Maximum width/height the preview will encode to.
pub const PREVIEW_MAX_W: u32 = 1280;
pub const PREVIEW_MAX_H: u32 = 720;

/// JPEG quality used for previews.
pub const PREVIEW_QUALITY: u8 = 80;

/// Minimum interval between encoded preview frames.
pub const MIN_INTERVAL: Duration = Duration::from_millis(66); // ~15 FPS

/// State for the preview-rate limiter.
#[derive(Debug, Default)]
pub struct PreviewLimiter {
    last_emit: Option<Instant>,
}

impl PreviewLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the caller should emit a new preview now.
    pub fn should_emit(&mut self, now: Instant) -> bool {
        match self.last_emit {
            Some(t) if now.duration_since(t) < MIN_INTERVAL => false,
            _ => {
                self.last_emit = Some(now);
                true
            }
        }
    }
}

/// JPEG-encoded preview ready for `ServerMsg::Frame`.
#[derive(Debug, Clone)]
pub struct EncodedPreview {
    pub jpeg: Vec<u8>,
    pub width: u16,
    pub height: u16,
    pub pts_ms: u64,
}

/// Encode a frame to a JPEG preview, downscaled if necessary.
///
/// Designed to be called inside `tokio::task::spawn_blocking`. Does
/// not allocate the input frame's RGBA cache (the caller does that
/// once via `frame.rgba_image()`).
pub fn encode_preview(frame: &VideoFrame) -> Result<EncodedPreview> {
    let src = frame.rgba_image();
    let src_w = src.width();
    let src_h = src.height();
    if src_w == 0 || src_h == 0 {
        anyhow::bail!("preview encode: zero-dim frame {src_w}x{src_h}");
    }

    // Compute target dims preserving aspect ratio inside the cap.
    let scale = (PREVIEW_MAX_W as f32 / src_w as f32)
        .min(PREVIEW_MAX_H as f32 / src_h as f32)
        .min(1.0);
    let dst_w = ((src_w as f32) * scale).round().max(1.0) as u32;
    let dst_h = ((src_h as f32) * scale).round().max(1.0) as u32;

    // Resize when downscaling; pass through when source is already
    // small enough.
    let resized;
    let (rgba, w, h) = if dst_w == src_w && dst_h == src_h {
        (src.as_raw().as_slice(), src_w, src_h)
    } else {
        resized = image::imageops::resize(
            src,
            dst_w,
            dst_h,
            image::imageops::FilterType::Triangle,
        );
        // `resized` is RGBA8 ImageBuffer; store its raw before borrow.
        let raw = resized.as_raw().as_slice();
        // raw is borrowed from `resized`; the variable lives until
        // function return so this is fine.
        (raw, dst_w, dst_h)
    };

    // RGBA → RGB stripping alpha. JpegEncoder accepts RGB(8) directly.
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for chunk in rgba.chunks_exact(4) {
        rgb.push(chunk[0]);
        rgb.push(chunk[1]);
        rgb.push(chunk[2]);
    }

    let mut out: Vec<u8> = Vec::with_capacity(rgb.len() / 4);
    {
        let mut cursor = Cursor::new(&mut out);
        let encoder = JpegEncoder::new_with_quality(&mut cursor, PREVIEW_QUALITY);
        encoder
            .write_image(&rgb, w, h, image::ExtendedColorType::Rgb8)
            .context("jpeg encode failed")?;
    }

    Ok(EncodedPreview {
        jpeg: out,
        width: w as u16,
        height: h as u16,
        pts_ms: frame.timestamp.as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn limiter_caps_to_15_fps() {
        let mut lim = PreviewLimiter::new();
        let t0 = Instant::now();
        assert!(lim.should_emit(t0));
        // Anything within MIN_INTERVAL is rejected.
        assert!(!lim.should_emit(t0 + Duration::from_millis(33)));
        // After MIN_INTERVAL it allows again.
        assert!(lim.should_emit(t0 + Duration::from_millis(80)));
    }

    #[test]
    fn encode_small_frame_yields_jpeg() {
        // 64x48 mid-gray RGBA frame.
        let raw = vec![128u8; 64 * 48 * 4];
        let frame = VideoFrame::new_cpu(raw, 64, 48, Duration::ZERO);
        let p = encode_preview(&frame).unwrap();
        // JPEG SOI marker.
        assert!(p.jpeg.starts_with(&[0xff, 0xd8]));
        assert_eq!(p.width, 64);
        assert_eq!(p.height, 48);
    }

    #[test]
    fn encode_downscales_to_cap() {
        // 4K-ish source — should be capped to PREVIEW_MAX_W.
        let w = 3840u32;
        let h = 2160u32;
        let raw = vec![128u8; (w * h * 4) as usize];
        let frame = VideoFrame::new_cpu(raw, w, h, Duration::ZERO);
        let p = encode_preview(&frame).unwrap();
        assert!(p.width as u32 <= PREVIEW_MAX_W);
        assert!(p.height as u32 <= PREVIEW_MAX_H);
    }
}
