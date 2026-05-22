//! Async streaming task: connect to an iroh-live publisher, decode video,
//! and fan frames out to UI consumers (and the Wave 3 CV inference task).
//!
//! ## Wave 5C: auto-mint
//!
//! When [`spawn`] is called with `ticket: None`, the task itself spins up
//! an iroh endpoint, mints a fresh [`LiveTicket`] (with a random
//! `herd-scout-<8-hex>` broadcast name), publishes it through the
//! `current_ticket` watch channel so the UI can render it as a QR
//! immediately, and (best-effort) persists it via [`Store`]. Then it
//! enters the same connect-subscribe-decode loop as the
//! "ticket supplied" path.
//!
//! When a ticket *is* supplied (env var, CLI, or saved-from-store), the
//! task skips the mint step and proceeds straight to subscribe — same
//! external behaviour as before Wave 5C.
//!
//! The decode loop intentionally writes every frame into a shared
//! [`tokio::sync::watch`] channel. The UI clones a receiver to render the
//! latest frame; the CV inference task clones *another* receiver and runs
//! YOLO on the same frame stream — see [`StreamHandle::frame_rx`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh_live::media::audio_backend::AudioBackend;
use iroh_live::media::format::{PlaybackConfig, VideoFrame};
use iroh_live::{Live, ticket::LiveTicket};
use rand::TryRngCore;
use tokio::sync::{Mutex, watch};
use tracing::{debug, error, info, warn};

use crate::store::Store;

/// Connection state surfaced to the UI for the status indicator.
///
/// Wave 5A uses the same state to decide when to render the
/// reconnect overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// No ticket has been minted/supplied yet (transient — only
    /// observable while the auto-mint endpoint binds, ~1-2s).
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

    /// The ticket the streaming task is currently bound to. `None` while
    /// the task is still binding its iroh endpoint on the auto-mint path
    /// (tens of milliseconds to a couple seconds), then `Some(_)` for the
    /// task's lifetime.
    ///
    /// Wave 5C: the UI polls this each repaint and re-renders the QR
    /// when the value transitions from `None` to `Some(_)`.
    current_ticket_rx: watch::Receiver<Option<LiveTicket>>,
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

    /// Wave 5C: the ticket the streaming task is currently using.
    ///
    /// Returns `None` only during the brief window between [`spawn`] and
    /// the auto-mint path's endpoint-bind completing. When a ticket was
    /// passed into `spawn` directly, this is `Some(_)` from the very
    /// first call (the channel is seeded synchronously with the ticket
    /// before the task is spawned).
    pub fn current_ticket(&self) -> Option<LiveTicket> {
        self.current_ticket_rx.borrow().clone()
    }
}

/// Spawn the streaming task on the current tokio runtime.
///
/// Must be called from inside `Runtime::enter()` (or the eframe app's
/// constructor while the runtime guard is held). The returned handle can
/// be polled cheaply from the egui update loop.
///
/// ## Parameters
///
/// - `ticket`: if `Some`, the task subscribes to this ticket's broadcast
///   name and dials its endpoint. If `None`, the task auto-mints a fresh
///   ticket on its own endpoint (Wave 5C) and surfaces it through
///   [`StreamHandle::current_ticket`].
/// - `store`: optional persistence handle. When the task auto-mints,
///   it will best-effort save the freshly-minted ticket so the next
///   launch reuses the same broadcast name. Saves on the auto-mint
///   path are non-fatal; failures only `warn!`.
/// - `egui_ctx`: the egui context used to wake the paint loop on every
///   frame / status / ticket transition.
pub fn spawn(
    ticket: Option<LiveTicket>,
    store: Option<Arc<Store>>,
    egui_ctx: egui::Context,
) -> StreamHandle {
    let (frame_tx, frame_rx) = watch::channel(None);
    // Initial status:
    //   - ticket supplied → "Connecting" (matches pre-5C behaviour;
    //     the UI's status chip shows "connecting" while we dial)
    //   - no ticket → "AwaitingTicket" (transient; flips to
    //     "Connecting" as soon as the auto-mint binds the endpoint)
    let (status_tx, status_rx) = watch::channel(if ticket.is_some() {
        ConnectionStatus::Connecting
    } else {
        ConnectionStatus::AwaitingTicket
    });
    let (last_frame_tx, last_frame_rx) = watch::channel(None);
    // Seed `current_ticket` with whatever the caller already has so the
    // UI can render the QR on the very first repaint when launching
    // from env / CLI / saved-store.
    let (current_ticket_tx, current_ticket_rx) = watch::channel(ticket.clone());

    let frame_tx = Arc::new(frame_tx);
    let status_tx = Arc::new(status_tx);
    let last_frame_tx = Arc::new(last_frame_tx);
    let current_ticket_tx = Arc::new(current_ticket_tx);
    let ctx_for_task = egui_ctx.clone();

    tokio::spawn(async move {
        // === Wave 5C: ensure we have a Live + ticket to work with ===
        //
        // We always need a Live to call `subscribe` on, even when the
        // caller supplied a ticket — but in the supplied-ticket path
        // the historical behaviour was to lazily build a fresh Live on
        // every reconnect attempt, so `run_subscription` keeps doing
        // that when we have no auto-minted Live.
        //
        // On the auto-mint path we *must* hold the Live across the
        // whole task lifetime: the ticket points at its
        // `endpoint().addr()`, so destroying and re-binding the Live
        // would invalidate the QR the user just scanned.
        let (live_for_subscribe, ticket) = if let Some(t) = ticket {
            (None, t)
        } else {
            match auto_mint(&current_ticket_tx, store.as_deref(), &ctx_for_task).await {
                Ok((live, t)) => (Some(live), t),
                Err(e) => {
                    error!("auto-mint failed: {e}; staying in AwaitingTicket");
                    let _ = status_tx.send(ConnectionStatus::Reconnecting {
                        reason: format!("auto-mint failed: {e}"),
                    });
                    ctx_for_task.request_repaint();
                    return;
                }
            }
        };

        // Shared AudioBackend across reconnect attempts. Wrapped in a
        // Mutex<Option<...>> so we can lazily create it on first
        // connect and survive across retries.
        let audio_ctx: Arc<Mutex<Option<AudioBackend>>> = Arc::new(Mutex::new(None));

        loop {
            let _ = status_tx.send(ConnectionStatus::Connecting);
            ctx_for_task.request_repaint();

            match run_subscription(
                live_for_subscribe.clone(),
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
                    let _ = status_tx.send(ConnectionStatus::Reconnecting { reason: err });
                }
            }

            ctx_for_task.request_repaint();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    StreamHandle {
        frame_rx,
        status_rx,
        last_frame_rx,
        current_ticket_rx,
    }
}

/// Bind a fresh iroh endpoint, mint a [`LiveTicket`] for it under a
/// randomly-generated broadcast name, push the ticket onto the
/// `current_ticket` watch channel so the UI can render the QR, and
/// best-effort persist it via [`Store`].
///
/// Returns the live `Live` instance (kept alive for the lifetime of the
/// streaming task — the ticket's endpoint addr stays valid only while
/// this `Live` is held) and the freshly-minted ticket.
async fn auto_mint(
    current_ticket_tx: &watch::Sender<Option<LiveTicket>>,
    store: Option<&Store>,
    egui_ctx: &egui::Context,
) -> Result<(Live, LiveTicket), String> {
    let broadcast_name = generate_broadcast_name();
    info!(broadcast = %broadcast_name, "auto-minting ticket: binding endpoint");

    // `with_router()` so peers can dial in; `with_gossip()` mirrors the
    // phone's `connect_impl` so both sides participate in the same
    // gossip topic. (The phone is the publisher; the desktop is the
    // subscriber. The endpoint addr in the ticket is the desktop's,
    // which the phone parses but only uses `broadcast_name` from —
    // see android-jni `connect_impl`.)
    let live = Live::from_env()
        .await
        .map_err(|e| format!("Live::from_env failed: {e}"))?
        .with_router()
        .with_gossip()
        .spawn();

    let ticket = LiveTicket::new(live.endpoint().addr(), &broadcast_name);
    info!(
        endpoint = %live.endpoint().id().fmt_short(),
        broadcast = %broadcast_name,
        "auto-minted ticket; awaiting publisher",
    );

    // Surface the ticket to the UI immediately so the QR can render
    // without waiting for any peers.
    let _ = current_ticket_tx.send(Some(ticket.clone()));
    egui_ctx.request_repaint();

    // Best-effort persistence. A failure here is logged and otherwise
    // ignored — we never want a flaky disk to block streaming.
    if let Some(store) = store {
        if let Err(e) = store.save_ticket(&ticket).await {
            warn!("failed to persist auto-minted ticket: {e:#}");
        }
    }

    Ok((live, ticket))
}

/// Generate a fresh broadcast name of the form `herd-scout-<8 hex chars>`.
///
/// 32 random bits is plenty of namespace for the MVP — collisions are
/// astronomically unlikely on a single device's lifetime, and the name
/// is only used as a string key inside the iroh-moq broadcast registry.
fn generate_broadcast_name() -> String {
    let mut buf = [0u8; 4];
    // `try_fill_bytes` returns an error only if the OS RNG is wedged;
    // in that case we fall back to a wall-clock-derived name so the
    // app still boots. This matches the store-module pattern where
    // RNG failure is treated as recoverable.
    if rand::rng().try_fill_bytes(&mut buf).is_err() {
        warn!("OS RNG unavailable; falling back to time-derived broadcast name");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u32)
            .unwrap_or(0);
        buf.copy_from_slice(&now.to_be_bytes());
    }
    format!(
        "herd-scout-{:02x}{:02x}{:02x}{:02x}",
        buf[0], buf[1], buf[2], buf[3]
    )
}

/// Single connect-subscribe-decode cycle. Runs until the publisher
/// disconnects or an error occurs; the outer loop in [`spawn`] handles
/// reconnect.
///
/// When `existing_live` is `Some`, that Live is reused (Wave 5C's
/// auto-mint path; the ticket points at its endpoint, so it must
/// outlive the ticket). When `None`, a fresh Live is bound for this
/// attempt — the historical pre-5C behaviour for env/CLI/saved
/// tickets.
async fn run_subscription(
    existing_live: Option<Live>,
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

    let live = match existing_live {
        Some(l) => l,
        None => Live::from_env()
            .await
            .map_err(|e| format!("Live::from_env failed: {e}"))?
            .spawn(),
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_name_has_expected_shape() {
        let name = generate_broadcast_name();
        assert!(
            name.starts_with("herd-scout-"),
            "expected herd-scout- prefix, got {name}"
        );
        let suffix = &name["herd-scout-".len()..];
        assert_eq!(suffix.len(), 8, "expected 8-char hex suffix in {name}");
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "expected all-hex suffix in {name}"
        );
    }

    #[test]
    fn broadcast_names_are_unique_in_practice() {
        // 32 bits of entropy: 100 draws should never collide. This is a
        // sanity check on the RNG path — the fallback path is
        // deliberately deterministic and not exercised here.
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let n = generate_broadcast_name();
            assert!(seen.insert(n), "broadcast name collision in 100 draws");
        }
    }
}
