//! Async streaming task: connect to an iroh-live publisher, decode video,
//! and fan frames out to UI consumers (and eventually the Wave 3 CV
//! inference task).
//!
//! The decode loop intentionally writes every frame into a shared
//! [`tokio::sync::watch`] channel. The UI clones a receiver to render the
//! latest frame; Wave 3 will clone *another* receiver and run YOLO
//! inference on the same frame stream — see [`StreamHandle::frame_rx`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh_live::media::audio_backend::AudioBackend;
use iroh_live::media::format::{PlaybackConfig, VideoFrame};
use iroh_live::{Live, ticket::LiveTicket};
use tokio::sync::{Mutex, watch};
use tracing::{debug, error, info, warn};

/// Connection state surfaced to the UI for the status indicator.
///
/// Wave 5A will use the same state to decide when to render the
/// reconnect overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// No ticket was provided yet (awaiting QR scan in Wave 5A, or env var today).
    AwaitingTicket,
    /// The async task is dialing the publisher.
    Connecting,
    /// Subscribed and decoding frames.
    Connected,
    /// The previous subscription failed; the loop is sleeping before retrying.
    Reconnecting { reason: String },
    /// The async task has stopped permanently (only happens on shutdown).
    #[allow(dead_code, reason = "reserved for Wave 5A graceful-shutdown UX")]
    Stopped,
}

impl ConnectionStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::AwaitingTicket => "awaiting ticket",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting { .. } => "reconnecting",
            Self::Stopped => "stopped",
        }
    }
}

/// Shared, observable state owned by the streaming task.
///
/// All fields are watched via [`watch::Sender`] so the UI can react in its
/// `update()` loop without polling tokio mutexes.
#[derive(Debug)]
pub struct StreamHandle {
    /// Latest decoded frame (None until the first frame arrives).
    ///
    /// **Fan-out point for Wave 3 CV inference.** The Wave 3 agent should
    /// call `frame_rx()` to obtain its own receiver — multiple consumers
    /// can read the most recent frame independently without rewriting the
    /// decode loop.
    frame_rx: watch::Receiver<Option<Arc<VideoFrame>>>,

    /// Connection status for the UI status indicator.
    status_rx: watch::Receiver<ConnectionStatus>,

    /// Wall-clock instant of the most recently received frame (None until
    /// the first frame arrives). Used by the UI to compute "frame age" and
    /// by Wave 5A to decide when to draw the reconnect overlay.
    last_frame_rx: watch::Receiver<Option<Instant>>,
}

impl StreamHandle {
    /// Returns the latest decoded frame, if one has arrived.
    pub fn current_frame(&self) -> Option<Arc<VideoFrame>> {
        self.frame_rx.borrow().clone()
    }

    /// Subscribes a fresh receiver to the frame channel.
    ///
    /// **Wave 3 entry point.** Call this from the inference task; each
    /// subscriber observes the latest frame independently of UI polling.
    #[allow(dead_code, reason = "Wave 3 CV inference task will call this")]
    pub fn frame_rx(&self) -> watch::Receiver<Option<Arc<VideoFrame>>> {
        self.frame_rx.clone()
    }

    pub fn status(&self) -> ConnectionStatus {
        self.status_rx.borrow().clone()
    }

    /// Returns the wall-clock instant of the most recently received frame.
    /// `None` if no frame has been received since the task started.
    pub fn last_frame_at(&self) -> Option<Instant> {
        *self.last_frame_rx.borrow()
    }

    /// Returns the age of the most recent frame, or `None` if none yet.
    pub fn frame_age(&self) -> Option<Duration> {
        self.last_frame_at().map(|t| t.elapsed())
    }
}

/// Spawn the streaming task on the current tokio runtime.
///
/// Must be called from inside `Runtime::enter()` (or the eframe app's
/// constructor while the runtime guard is held). The returned handle can
/// be polled cheaply from the egui update loop.
pub fn spawn(ticket: Option<LiveTicket>, egui_ctx: egui::Context) -> StreamHandle {
    let (frame_tx, frame_rx) = watch::channel(None);
    let (status_tx, status_rx) = watch::channel(if ticket.is_some() {
        ConnectionStatus::Connecting
    } else {
        ConnectionStatus::AwaitingTicket
    });
    let (last_frame_tx, last_frame_rx) = watch::channel(None);

    if let Some(ticket) = ticket {
        let frame_tx = Arc::new(frame_tx);
        let status_tx = Arc::new(status_tx);
        let last_frame_tx = Arc::new(last_frame_tx);
        let ctx_for_task = egui_ctx.clone();

        tokio::spawn(async move {
            // Shared AudioBackend across reconnect attempts. Wrapped in a
            // Mutex<Option<...>> so we can lazily create it on first
            // connect and survive across retries.
            let audio_ctx: Arc<Mutex<Option<AudioBackend>>> = Arc::new(Mutex::new(None));

            loop {
                let _ = status_tx.send(ConnectionStatus::Connecting);
                ctx_for_task.request_repaint();

                match run_subscription(
                    ticket.clone(),
                    audio_ctx.clone(),
                    frame_tx.clone(),
                    status_tx.clone(),
                    last_frame_tx.clone(),
                    ctx_for_task.clone(),
                )
                .await
                {
                    Ok(()) => {
                        warn!("subscription ended cleanly; reconnecting in 1s");
                        let _ = status_tx.send(ConnectionStatus::Reconnecting {
                            reason: "publisher closed".to_string(),
                        });
                    }
                    Err(err) => {
                        error!("subscription failed: {err}; reconnecting in 1s");
                        let _ = status_tx.send(ConnectionStatus::Reconnecting {
                            reason: err,
                        });
                    }
                }

                ctx_for_task.request_repaint();
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }

    StreamHandle {
        frame_rx,
        status_rx,
        last_frame_rx,
    }
}

/// Single connect-subscribe-decode cycle. Runs until the publisher
/// disconnects or an error occurs; the outer loop in [`spawn`] handles
/// reconnect.
async fn run_subscription(
    ticket: LiveTicket,
    audio_ctx: Arc<Mutex<Option<AudioBackend>>>,
    frame_tx: Arc<watch::Sender<Option<Arc<VideoFrame>>>>,
    status_tx: Arc<watch::Sender<ConnectionStatus>>,
    last_frame_tx: Arc<watch::Sender<Option<Instant>>>,
    egui_ctx: egui::Context,
) -> Result<(), String> {
    info!(
        broadcast = %ticket.broadcast_name,
        endpoint = %ticket.endpoint.id.fmt_short(),
        "connecting"
    );

    let live = Live::from_env()
        .await
        .map_err(|e| format!("Live::from_env failed: {e}"))?
        .spawn();

    let sub = live
        .subscribe(ticket.endpoint.clone(), &ticket.broadcast_name)
        .await
        .map_err(|e| format!("subscribe failed: {e}"))?;
    info!("session connected");

    // Build (or reuse) the audio backend. Even though we don't render audio
    // for the herd-scout MVP, MediaTracks::new requires an AudioStreamFactory
    // to attempt audio rendition selection. The returned audio track is
    // dropped immediately.
    let audio = {
        let mut guard = audio_ctx.lock().await;
        if guard.is_none() {
            *guard = Some(AudioBackend::default());
        }
        guard.as_ref().expect("audio backend just inserted").clone()
    };

    let tracks = sub
        .media(&audio, PlaybackConfig::default())
        .await
        .map_err(|e| format!("media subscribe failed: {e}"))?;

    let mut video = tracks
        .video
        .ok_or_else(|| "broadcast has no video track".to_string())?;

    // Audio is intentionally dropped — we only care about video for herd-scout.
    drop(tracks.audio);

    info!(rendition = %video.rendition(), "subscribed to video track");
    let _ = status_tx.send(ConnectionStatus::Connected);
    egui_ctx.request_repaint();

    let mut frame_count: u64 = 0;
    while let Some(frame) = video.next_frame().await {
        frame_count += 1;
        let now = Instant::now();
        let frame = Arc::new(frame);

        // === Fan-out point for Wave 3 CV inference ===
        // Send into the watch channel; UI and (future) inference task each
        // hold receivers and observe the latest frame independently.
        let _ = frame_tx.send(Some(frame.clone()));
        let _ = last_frame_tx.send(Some(now));

        if frame_count % 30 == 0 {
            debug!(
                frame_count,
                width = frame.width(),
                height = frame.height(),
                "frames decoded"
            );
        }

        // Wake egui so it picks up the new frame promptly.
        egui_ctx.request_repaint();
    }

    info!(frame_count, "video track closed");
    Ok(())
}
