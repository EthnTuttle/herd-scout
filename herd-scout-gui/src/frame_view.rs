//! Decode JPEG preview bytes from the daemon and turn them into an
//! `egui::TextureHandle` ready to paint.
//!
//! Wave 6 replaces `moq-media-egui::FrameView` (which expected a raw
//! `VideoFrame`) — the GUI no longer pulls in iroh-live or moq-media,
//! so the texture path is now plain JPEG bytes through the `image`
//! crate.

use std::fmt;

use anyhow::{Context, Result};
use egui::{ColorImage, Context as EguiCtx, TextureHandle, TextureOptions};

pub struct FrameView {
    name: String,
    /// Cached texture, rebuilt when a new frame is decoded.
    texture: Option<TextureHandle>,
    /// `pts_ms` of the last frame uploaded; used to dedupe.
    last_pts_ms: u64,
    /// Last source dimensions, kept so the CV overlay can project box
    /// coords from source-frame space onto the rendered rect.
    pub last_dims: Option<(u32, u32)>,
}

impl fmt::Debug for FrameView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameView")
            .field("name", &self.name)
            .field("last_pts_ms", &self.last_pts_ms)
            .field("last_dims", &self.last_dims)
            .field("has_texture", &self.texture.is_some())
            .finish()
    }
}

impl FrameView {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            texture: None,
            last_pts_ms: 0,
            last_dims: None,
        }
    }

    /// Returns the cached texture handle, if a frame has ever been
    /// uploaded.
    pub fn texture(&self) -> Option<&TextureHandle> {
        self.texture.as_ref()
    }

    /// Decode `jpeg` (if newer than the last upload) and replace the
    /// cached texture. Returns `true` when the texture was rebuilt.
    pub fn ingest(
        &mut self,
        ctx: &EguiCtx,
        jpeg: &[u8],
        pts_ms: u64,
        width: u16,
        height: u16,
    ) -> Result<bool> {
        if pts_ms != 0 && pts_ms == self.last_pts_ms && self.texture.is_some() {
            return Ok(false);
        }
        let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
            .context("decoding JPEG preview")?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let pixels = rgba
            .pixels()
            .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect::<Vec<_>>();
        let color_image = ColorImage {
            size: [w as usize, h as usize],
            pixels,
            source_size: egui::vec2(w as f32, h as f32),
        };
        let handle = ctx.load_texture(&self.name, color_image, TextureOptions::LINEAR);
        self.texture = Some(handle);
        self.last_pts_ms = pts_ms;
        // Prefer the daemon's reported width/height for projection
        // since that's the JPEG canvas — the CV bounding boxes are
        // already in source-frame space, but the daemon downscales to
        // ≤720p so its `width`/`height` *is* the source for our
        // purposes.
        self.last_dims = Some((u32::from(width), u32::from(height)));
        Ok(true)
    }
}
