//! YOLOv5n inference via the cv-sidecar process.
//!
//! Wave 6.5 pivot: the in-process `ort` Rust crate consistently deadlocks
//! at `Detector::new()` on this project's Pascal hardware regardless of
//! ORT version, build mode, or host glibc. Python `onnxruntime-gpu` 1.23
//! on the same box loads in 360 ms and runs YOLOv5n at 59 FPS. We
//! offload inference to a small Python sidecar (`deploy/cv-sidecar/`)
//! and talk to it over a Unix socket with a fixed-width binary protocol.
//!
//! The public types in this module — [`Detector`], [`Detection`],
//! [`CocoClass`] — keep the same shape as the previous in-process
//! version so [`super::task::spawn_cv_task`] doesn't change.
//!
//! ## Wire protocol
//!
//! Daemon -> sidecar (request):
//! ```text
//! u32 frame_id  u32 width  u32 height  u32 payload_len (= w*h*3)
//! [payload_len bytes: BGR24, row-major top-to-bottom, contiguous]
//! ```
//!
//! Sidecar -> daemon (response):
//! ```text
//! u32 frame_id  u32 n_dets
//! For each det: u32 class_id  u32 track_id  f32 conf  f32 x1  f32 y1  f32 x2  f32 y2
//! ```
//!
//! `class_id` is the wire enum (0=horse, 1=sheep, 2=cow), already
//! filtered + class-mapped by the sidecar. `track_id` is the
//! ByteTrack-assigned persistent ID for the detection across frames;
//! `0xFFFFFFFF` means the tracker has not yet assigned an ID. Bounding
//! box is in source-frame pixel space. The sidecar handles
//! preprocessing (resize/normalize/dtype-cast) and postprocessing (NMS,
//! class filtering, tracking).

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh_live::media::format::VideoFrame;

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

    /// Map the sidecar's wire class id (0=horse, 1=sheep, 2=cow) to a
    /// `CocoClass` enum. Returns `None` for any other id.
    fn from_wire(idx: u32) -> Option<Self> {
        match idx {
            0 => Some(Self::Horse),
            1 => Some(Self::Sheep),
            2 => Some(Self::Cow),
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
    /// Persistent track id assigned by the sidecar's ByteTrack; `None`
    /// when the tracker has not yet attached an ID (sentinel value
    /// `0xFFFFFFFF` on the wire).
    pub track_id: Option<u32>,
}

/// Wire sentinel for "no track ID assigned yet" — must match the
/// sidecar's `NO_TRACK_ID` constant.
const NO_TRACK_ID: u32 = 0xFFFF_FFFF;

/// Default sidecar socket path. Override with the `CV_SIDECAR_SOCKET`
/// env var. Matches the path the systemd unit binds in
/// `deploy/systemd/herd-scout-cv-sidecar.service`.
const DEFAULT_SIDECAR_SOCKET: &str = "/run/herd-scout/cv.sock";

/// Per-frame I/O timeout. The sidecar's steady-state inference is
/// ~17 ms on a GTX 1060; we give it 5s to absorb cold-start CUDA JIT
/// on the first frame.
const SIDECAR_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared handle to the sidecar's Unix-domain socket connection. The
/// live CV path and the upload processor (Wave 13) both grab the
/// `Mutex` to send a request — live takes it per-frame, upload holds
/// it for the duration of one clip.
///
/// `std::sync::Mutex` is correct here: the I/O is synchronous and
/// every caller does the work inside `spawn_blocking`, so there's no
/// `.await` while holding the lock and `tokio::sync::Mutex` would
/// just bounce the wakers needlessly.
pub type SidecarHandle = Arc<StdMutex<UnixStream>>;

/// Wire client for the cv-sidecar Python process.
///
/// Construction can fail if the sidecar isn't running (the systemd
/// dependency `BindsTo=` should normally prevent this). Callers that
/// want soft-fail behaviour should match on the `Result` returned by
/// [`Detector::new`] and continue with a "CV disabled" snapshot.
///
/// The socket is wrapped in [`SidecarHandle`] (an `Arc<Mutex<_>>`) so
/// the upload pipeline can borrow exclusive sidecar access for the
/// duration of a clip without disturbing the live frame path.
pub struct Detector {
    socket_path: PathBuf,
    handle: SidecarHandle,
    next_frame_id: AtomicU32,
    /// Reusable scratch buffer for the BGR conversion. Avoids
    /// per-frame allocation in the hot path.
    bgr_buf: Vec<u8>,
}

impl std::fmt::Debug for Detector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Detector")
            .field("socket_path", &self.socket_path)
            .finish_non_exhaustive()
    }
}

impl Detector {
    /// Connect to the cv-sidecar Unix socket. Errors if the socket
    /// doesn't exist or the connection is refused.
    pub fn new() -> Result<Self> {
        let socket_path = PathBuf::from(
            env::var("CV_SIDECAR_SOCKET")
                .unwrap_or_else(|_| DEFAULT_SIDECAR_SOCKET.to_string()),
        );

        tracing::info!(
            "CV: connecting to cv-sidecar at {}",
            socket_path.display()
        );

        let stream = UnixStream::connect(&socket_path).with_context(|| {
            format!("connect to cv-sidecar socket at {}", socket_path.display())
        })?;
        stream
            .set_read_timeout(Some(SIDECAR_TIMEOUT))
            .context("set sidecar read timeout")?;
        stream
            .set_write_timeout(Some(SIDECAR_TIMEOUT))
            .context("set sidecar write timeout")?;

        tracing::info!("CV: cv-sidecar connected (GPU inference via Python ORT)");

        Ok(Self {
            socket_path,
            handle: Arc::new(StdMutex::new(stream)),
            next_frame_id: AtomicU32::new(0),
            bgr_buf: Vec::new(),
        })
    }

    /// Cloneable handle to the sidecar's UnixStream. The upload
    /// processor takes one of these to drive file-mode (`0x01`)
    /// requests; live frame requests go through [`Detector::infer`]
    /// which uses the same handle internally so the two paths are
    /// serialised by the inner `Mutex`.
    pub fn handle(&self) -> SidecarHandle {
        self.handle.clone()
    }

    /// Send one frame to the sidecar and read back its detections.
    ///
    /// `&mut self` is required because the underlying Unix socket is
    /// not `Sync`; the calling code already serializes via a Tokio
    /// `Mutex` (see `cv/task.rs`).
    ///
    /// **Wave 13 wire-protocol bump:** every request is now prefixed
    /// with a `request_kind: u32 = 0x00` selector so the sidecar can
    /// dispatch live frame requests vs file-mode (`0x01`) requests on
    /// the same socket. The response framing is unchanged for live
    /// (no `pts_ms` field — that's file-mode-only).
    pub fn infer(&mut self, frame: &VideoFrame) -> Result<Vec<Detection>> {
        let w = frame.width();
        let h = frame.height();
        if w == 0 || h == 0 {
            anyhow::bail!("video frame has zero dimension: {w}×{h}");
        }

        // Pull RGBA from VideoFrame (lazily decoded + cached on the
        // VideoFrame). Convert to BGR24 in place.
        let rgba = frame.rgba_image();
        let raw = rgba.as_raw();
        debug_assert_eq!(raw.len(), (w as usize) * (h as usize) * 4);

        let bgr_len = (w as usize) * (h as usize) * 3;
        self.bgr_buf.clear();
        self.bgr_buf.reserve(bgr_len);
        // RGBA -> BGR: swap R<->B, drop alpha
        for chunk in raw.chunks_exact(4) {
            self.bgr_buf.push(chunk[2]); // B
            self.bgr_buf.push(chunk[1]); // G
            self.bgr_buf.push(chunk[0]); // R
        }
        debug_assert_eq!(self.bgr_buf.len(), bgr_len);

        let frame_id = self.next_frame_id.fetch_add(1, Ordering::Relaxed);
        let payload_len = bgr_len as u32;

        // Header: u32 request_kind = 0x00, then 4 × u32 LE = 16 bytes,
        // then the BGR24 payload.
        let mut prefixed_hdr = [0u8; 20];
        prefixed_hdr[0..4].copy_from_slice(&0u32.to_le_bytes()); // REQ_KIND_FRAME
        prefixed_hdr[4..8].copy_from_slice(&frame_id.to_le_bytes());
        prefixed_hdr[8..12].copy_from_slice(&w.to_le_bytes());
        prefixed_hdr[12..16].copy_from_slice(&h.to_le_bytes());
        prefixed_hdr[16..20].copy_from_slice(&payload_len.to_le_bytes());

        let mut stream = self
            .handle
            .lock()
            .map_err(|e| anyhow::anyhow!("sidecar handle poisoned: {e}"))?;

        stream
            .write_all(&prefixed_hdr)
            .context("write request header to sidecar")?;
        stream
            .write_all(&self.bgr_buf)
            .context("write request payload to sidecar")?;

        // Response header: u32 frame_id + u32 n_dets
        let mut resp_hdr = [0u8; 8];
        stream
            .read_exact(&mut resp_hdr)
            .context("read response header from sidecar")?;
        let recv_frame_id = u32::from_le_bytes(resp_hdr[0..4].try_into().unwrap());
        let n_dets = u32::from_le_bytes(resp_hdr[4..8].try_into().unwrap()) as usize;

        if recv_frame_id != frame_id {
            anyhow::bail!(
                "sidecar frame_id mismatch: sent {frame_id}, got {recv_frame_id}"
            );
        }

        // Each det: 2 × u32 (class, track_id) + 5 × f32 (conf, xyxy) = 28 bytes
        const DET_BYTES: usize = 28;
        if n_dets > 1024 {
            // Sanity guard against runaway / corrupt response
            anyhow::bail!("sidecar reported absurd n_dets={n_dets}; aborting");
        }
        let mut det_buf = vec![0u8; n_dets * DET_BYTES];
        if !det_buf.is_empty() {
            stream
                .read_exact(&mut det_buf)
                .context("read detections from sidecar")?;
        }

        // Drop the sidecar lock as soon as I/O is done — parsing the
        // detection rows doesn't need exclusive socket access and
        // releasing early keeps the upload processor's wait time tight.
        drop(stream);

        let mut out = Vec::with_capacity(n_dets);
        for i in 0..n_dets {
            let off = i * DET_BYTES;
            let cls_wire = u32::from_le_bytes(det_buf[off..off + 4].try_into().unwrap());
            let track_id_wire = u32::from_le_bytes(det_buf[off + 4..off + 8].try_into().unwrap());
            let conf = f32::from_le_bytes(det_buf[off + 8..off + 12].try_into().unwrap());
            let x1 = f32::from_le_bytes(det_buf[off + 12..off + 16].try_into().unwrap());
            let y1 = f32::from_le_bytes(det_buf[off + 16..off + 20].try_into().unwrap());
            let x2 = f32::from_le_bytes(det_buf[off + 20..off + 24].try_into().unwrap());
            let y2 = f32::from_le_bytes(det_buf[off + 24..off + 28].try_into().unwrap());
            if let Some(class) = CocoClass::from_wire(cls_wire) {
                let track_id = (track_id_wire != NO_TRACK_ID).then_some(track_id_wire);
                out.push(Detection {
                    class,
                    bbox: [x1, y1, x2, y2],
                    score: conf,
                    track_id,
                });
            } else {
                tracing::warn!("CV: sidecar returned unknown class id {cls_wire}; skipping");
            }
        }

        Ok(out)
    }
}

/// Path to where the model file *should* live on disk. Useful for
/// log messages even though we bake it in at compile time.
#[allow(dead_code, reason = "kept for future logging / `--print-model-path` CLI")]
pub fn model_path_for_logging() -> PathBuf {
    PathBuf::from("herd-scout-daemon/assets/yolov5n.onnx")
}
