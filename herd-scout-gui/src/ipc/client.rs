//! Async IPC client task — connects to the daemon, reads
//! `ServerMsg`s, and pushes them into shared state the egui paint
//! loop reads via `parking_lot::RwLock`.
//!
//! On disconnect we set an error flag (`disconnected`) so the GUI can
//! show a "daemon offline" overlay; reconnect is handled by the
//! caller (typically by spawning a fresh `run_client` after a delay).

#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use herd_scout_ipc::{ClassCountsWire, ClientMsg, ConnectionStatus, DetWire, ServerMsg};
use parking_lot::RwLock;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::frame::{read_frame, write_frame};
use crate::records::RecordsState;
use crate::uploads::UploadsState;

/// Snapshot of the latest preview frame the daemon emitted.
#[derive(Debug, Clone, Default)]
pub struct LatestFrame {
    pub jpeg: Vec<u8>,
    pub width: u16,
    pub height: u16,
    pub pts_ms: u64,
    /// Wall-clock time when this frame *arrived* at the GUI. Used for
    /// the "frame age" overlay and the "stale → reconnect" trigger.
    pub received_at: Option<Instant>,
}

/// Detections plus rolling counts the GUI overlays on top of the
/// preview texture.
#[derive(Debug, Clone, Default)]
pub struct LatestDetections {
    pub frame_pts_ms: u64,
    pub dets: Vec<DetWire>,
    pub counts: ClassCountsWire,
    pub cv_banner: Option<String>,
    pub cv_disabled: bool,
}

/// Shared GUI-side state populated by the IPC reader task.
///
/// Cheap to clone (`Arc`); held by the eframe `App` and the
/// background task in parallel.
#[derive(Debug, Default)]
pub struct SharedClientState {
    pub status: RwLock<ConnectionStatus>,
    pub last_frame_age_ms: RwLock<Option<u64>>,
    pub current_ticket: RwLock<Option<String>>,
    pub latest_frame: RwLock<LatestFrame>,
    pub latest_dets: RwLock<LatestDetections>,
    /// `true` after the reader task has seen the socket close; cleared
    /// by reconnects.
    pub disconnected: RwLock<bool>,
    /// Daemon's reported version, populated from the first `Hello`.
    pub daemon_version: RwLock<Option<String>>,
    /// Phase 5: drag-drop / file-picker upload pipeline state. Cheap
    /// to share — internally `Arc<RwLock<…>>` — and read by the egui
    /// paint loop on every repaint.
    pub uploads: UploadsState,
    /// Plan-FMS Phase 4: cached asset lists keyed by kind, plus a
    /// per-tab error slot. `Arc` because the IPC dispatcher and the
    /// egui paint loop both read it; the `RwLock`s inside live behind
    /// the `Arc`.
    pub records: Arc<RecordsState>,
}

impl SharedClientState {
    pub fn arc() -> Arc<Self> {
        Arc::new(Self {
            status: RwLock::new(ConnectionStatus::Idle),
            records: RecordsState::arc(),
            ..Default::default()
        })
    }
}

/// Handle returned by [`run_client`]; drop it to disconnect.
#[derive(Debug)]
pub struct IpcClientHandle {
    pub state: Arc<SharedClientState>,
    pub send: mpsc::Sender<ClientMsg>,
}

impl IpcClientHandle {
    pub fn try_send(&self, msg: ClientMsg) {
        if let Err(e) = self.send.try_send(msg) {
            warn!("ipc send dropped (channel full or closed): {e}");
        }
    }
}

/// Connect to the daemon's UDS and start the reader/writer pair.
///
/// `repaint_cb` is invoked whenever the reader receives a message that
/// would change rendered state — wire it to `egui::Context::request_repaint`.
pub async fn run_client(
    socket_path: &Path,
    state: Arc<SharedClientState>,
    repaint_cb: impl Fn() + Send + Sync + 'static,
) -> Result<IpcClientHandle> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connecting to daemon socket at {}", socket_path.display()))?;
    info!(path = %socket_path.display(), "GUI: connected to daemon");
    *state.disconnected.write() = false;

    let (read_half, write_half) = tokio::io::split(stream);
    let (tx, rx) = mpsc::channel::<ClientMsg>(32);

    // Spawn writer.
    {
        let state = state.clone();
        tokio::spawn(async move {
            run_writer(write_half, rx).await;
            // Mark disconnected once writer exits.
            *state.disconnected.write() = true;
        });
    }

    // Spawn reader.
    {
        let state = state.clone();
        let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(repaint_cb);
        tokio::spawn(async move {
            run_reader(read_half, state.clone(), cb.clone()).await;
            *state.disconnected.write() = true;
            cb();
        });
    }

    // Send our Hello immediately so the daemon can re-publish state.
    let _ = tx
        .send(ClientMsg::Hello {
            gui_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .await;

    Ok(IpcClientHandle { state, send: tx })
}

async fn run_writer(mut write_half: WriteHalf<UnixStream>, mut rx: mpsc::Receiver<ClientMsg>) {
    while let Some(msg) = rx.recv().await {
        let bytes = match serde_json::to_vec(&msg) {
            Ok(b) => b,
            Err(e) => {
                warn!("GUI: failed to serialise ClientMsg: {e}");
                continue;
            }
        };
        if let Err(e) = write_frame(&mut write_half, &bytes).await {
            debug!("GUI: write to daemon failed: {e}");
            return;
        }
    }
}

async fn run_reader(
    mut read_half: ReadHalf<UnixStream>,
    state: Arc<SharedClientState>,
    repaint: Arc<dyn Fn() + Send + Sync>,
) {
    loop {
        match read_frame(&mut read_half).await {
            Ok(Some(bytes)) => match serde_json::from_slice::<ServerMsg>(&bytes) {
                Ok(msg) => {
                    apply_msg(&state, msg);
                    repaint();
                }
                Err(e) => {
                    warn!("GUI: undecodable ServerMsg: {e}");
                }
            },
            Ok(None) => {
                debug!("GUI: daemon closed socket");
                return;
            }
            Err(e) => {
                debug!("GUI: read from daemon failed: {e}");
                return;
            }
        }
    }
}

fn apply_msg(state: &SharedClientState, msg: ServerMsg) {
    match msg {
        ServerMsg::Hello { daemon_version, .. } => {
            *state.daemon_version.write() = Some(daemon_version);
        }
        ServerMsg::Pairing { ticket } => {
            *state.current_ticket.write() = Some(ticket);
        }
        ServerMsg::Status {
            state: cs,
            last_frame_age_ms,
        } => {
            // If the daemon transitions back to Idle (e.g. user pressed
            // Cancel on the reconnect overlay), clear the cached last
            // frame so the GUI falls back to the pairing screen instead
            // of holding up the frozen final frame under a reconnect
            // overlay forever.
            let prev = state.status.read().clone();
            if matches!(cs, ConnectionStatus::Idle) && !matches!(prev, ConnectionStatus::Idle) {
                let mut g = state.latest_frame.write();
                *g = LatestFrame::default();
            }
            *state.status.write() = cs;
            *state.last_frame_age_ms.write() = last_frame_age_ms;
        }
        ServerMsg::Frame {
            width,
            height,
            pts_ms,
            jpeg,
            clip_id: _,
        } => {
            let now = Instant::now();
            let mut g = state.latest_frame.write();
            g.jpeg = jpeg;
            g.width = width;
            g.height = height;
            g.pts_ms = pts_ms;
            g.received_at = Some(now);
        }
        ServerMsg::Detections {
            frame_pts_ms,
            dets,
            counts,
            clip_id: _,
        } => {
            let mut g = state.latest_dets.write();
            g.frame_pts_ms = frame_pts_ms;
            g.dets = dets;
            g.counts = counts;
        }
        ServerMsg::CvBanner { text, disabled } => {
            let mut g = state.latest_dets.write();
            g.cv_banner = text;
            g.cv_disabled = disabled;
        }
        ServerMsg::UploadStatus {
            blake3_hex,
            filename,
            state: upload_state,
            progress_pct,
            eta_ms,
            summary,
        } => {
            state.uploads.apply_status(
                blake3_hex,
                filename,
                upload_state,
                progress_pct,
                eta_ms,
                summary,
            );
        }
        ServerMsg::FmsAsset { request_id: _, asset: _ } => {
            // The change-bridge already pushed a FmsChange that will
            // trigger a list refresh; per-asset replies are
            // request-correlated for future per-row UX. Today the
            // Records tab refreshes everything on change, so we
            // intentionally drop this message.
        }
        ServerMsg::FmsAssetList { request_id: _, kind, assets } => {
            state.records.apply_list(kind, assets);
        }
        ServerMsg::FmsLog { .. } | ServerMsg::FmsLogList { .. } => {
            // Phase 4 ships asset CRUD only; log-list UI lands in a
            // follow-up. The daemon already emits these so the
            // surface is exercised end-to-end.
        }
        ServerMsg::FmsChange { event } => {
            // The IPC reader task can't borrow an `IpcClientHandle`
            // here (we're inside `apply_msg`, which only sees
            // `state`). Instead we set a flag and let the egui paint
            // loop notice it on the next frame and issue the
            // refresh. This keeps the reader free of UI plumbing.
            if event.entity_hint.is_none() || event.entity_hint.as_deref() == Some("asset") {
                *state.records.refresh_pending.write() = true;
            }
        }
        ServerMsg::FmsError { request_id: _, code, message } => {
            state.records.set_error(format!("{code}: {message}"));
        }
    }
}

/// Attempts to connect with retry/backoff for up to `total` time.
/// Useful at startup when the GUI auto-spawns a daemon and waits for
/// its socket to appear.
pub async fn connect_with_retry(socket_path: &Path, total: Duration) -> Result<UnixStream> {
    let start = Instant::now();
    let mut delay = Duration::from_millis(50);
    loop {
        match UnixStream::connect(socket_path).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                if start.elapsed() >= total {
                    return Err(anyhow::anyhow!(
                        "could not connect to daemon at {} after {:?}: {e}",
                        socket_path.display(),
                        total
                    ));
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_millis(500));
            }
        }
    }
}
