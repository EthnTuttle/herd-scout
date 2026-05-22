//! Wave 6 streaming task — replaces Wave 5C's broken self-subscribe.
//!
//! ## Overview
//!
//! The daemon owns a *long-lived* `Live` (`with_router().with_gossip()`)
//! and registers no broadcast itself; it is purely a subscriber.
//!
//! Pairing flow:
//!
//! 1. **Boot.** Either restore the last saved [`LiveTicket`] from
//!    [`Store`], or mint a fresh one using
//!    `LiveTicket::new(live.endpoint().addr(), &broadcast_name)` and
//!    persist it.
//! 2. **Push to GUI.** Send the ticket as `ServerMsg::Pairing` so the
//!    GUI can render the QR.
//! 3. **Phone scans the QR.** The phone's JNI `connect_impl` parses
//!    the ticket and calls `live.transport().connect(ticket.endpoint)`
//!    (Wave 6 JNI change). This establishes a moq session pointing at
//!    the daemon.
//! 4. **Phone publishes.** `Live::publish(broadcast_name, broadcast)`
//!    on the phone fans out over every existing/new session via the
//!    moq actor at `iroh-moq/src/lib.rs:482-526` — the daemon's
//!    session is in that set.
//! 5. **Daemon accepts.** This module subscribes to
//!    `live.transport().incoming_sessions()` and, for each session,
//!    calls `session.subscribe(broadcast_name)` (wrapped in a 15 s
//!    timeout per design Risk #1).
//! 6. **Decode + CV + IPC fan-out.** Wrap the `BroadcastConsumer` in a
//!    `RemoteBroadcast`, call `.media_with_decoders::<DefaultDecoders>`
//!    for `MediaTracks`, loop on `video.next_frame().await`.
//!    Every frame:
//!    - is pushed to a `watch` channel for the CV task
//!    - emits a JPEG preview over the broadcast `ServerMsg` channel,
//!      gated by `PreviewLimiter` (15 FPS cap)
//!
//! ## What is intentionally absent vs. Wave 5C
//!
//! - No "AwaitingTicket" status: the daemon mints synchronously on
//!   boot before binding the IPC socket, so the GUI always sees a
//!   ticket from its first repaint.
//! - No `live.subscribe(ticket.endpoint, …)` self-dial. We never call
//!   it; sessions arrive via `incoming_sessions`.
//! - No `existing_live: Option<Live>`. There is exactly one `Live`
//!   per daemon, owned for the daemon's lifetime.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use herd_scout_ipc::{ConnectionStatus, ServerMsg};
use iroh_live::media::audio_backend::AudioBackend;
use iroh_live::media::format::{PlaybackConfig, VideoFrame};
use iroh_live::media::subscribe::RemoteBroadcast;
use iroh_live::ticket::LiveTicket;
use iroh_live::Live;
use tokio::sync::{broadcast, watch};
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::pairing::generate_broadcast_name;
use crate::preview::{EncodedPreview, PreviewLimiter, encode_preview};
use crate::store::Store;

/// How long we'll wait for the publisher to announce the broadcast on
/// a freshly-accepted session before giving up. Per design Risk #1.
const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Top-level daemon streaming state, owned by `main.rs`.
///
/// All fields are cheap to clone (`Arc`s and senders).
#[derive(Clone, Debug)]
pub struct DaemonState {
    pub live: Live,
    pub ticket: LiveTicket,
    pub broadcast_name: String,
    /// Outbound broadcast that every connected GUI subscribes to.
    pub server_tx: broadcast::Sender<ServerMsg>,
    /// Frame fan-out channel for the CV task.
    pub frame_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    /// Latest connection status (mirrors what was last announced via
    /// `server_tx`). Useful for periodic Status pings.
    pub status_tx: watch::Sender<ConnectionStatus>,
    /// Wall-clock instant of the most recent video frame.
    pub last_frame_tx: watch::Sender<Option<Instant>>,
}

impl DaemonState {
    /// Mint a fresh ticket on `live`, persisting via `store` if
    /// available. The returned ticket points at `live.endpoint().addr()`,
    /// so the daemon must keep `live` alive for the ticket's
    /// usefulness.
    pub async fn mint(live: &Live, store: Option<&Store>) -> Result<(String, LiveTicket)> {
        let broadcast_name = generate_broadcast_name();
        let ticket = LiveTicket::new(live.endpoint().addr(), &broadcast_name);
        info!(
            endpoint = %live.endpoint().id().fmt_short(),
            broadcast = %broadcast_name,
            "minted fresh pairing ticket",
        );
        if let Some(store) = store {
            if let Err(e) = store.save_ticket(&ticket).await {
                warn!("failed to persist freshly minted ticket: {e:#}");
            }
        }
        Ok((broadcast_name, ticket))
    }
}

/// Spawn the long-lived accept loop that listens for incoming moq
/// sessions and turns each one into a decode+CV+preview pipeline.
///
/// Returns immediately. The task runs until the runtime shuts down or
/// `live`'s transport closes.
pub fn spawn_accept_loop(state: DaemonState) {
    tokio::spawn(async move {
        info!("daemon: accept loop started");
        let _ = state.status_tx.send(ConnectionStatus::Idle);
        let _ = state.server_tx.send(ServerMsg::Status {
            state: ConnectionStatus::Idle,
            last_frame_age_ms: None,
        });

        let moq = state.live.transport().clone();
        let mut incoming = moq.incoming_sessions();

        while let Some(incoming_sess) = incoming.next().await {
            let remote = incoming_sess.remote_id();
            info!(remote = %remote.fmt_short(), "daemon: incoming session");
            let session = incoming_sess.accept();

            // Spawn a per-session task — multiple phones could in theory
            // dial the same daemon. MVP only handles one at a time on
            // the data path (one frame channel) but we still gracefully
            // drop sessions that fail to subscribe.
            let st = state.clone();
            tokio::spawn(async move {
                if let Err(e) = run_session(st, session).await {
                    warn!(remote = %remote.fmt_short(), "session ended: {e:#}");
                }
            });
        }
        warn!("daemon: incoming_sessions stream ended; transport closed");
        let _ = state.status_tx.send(ConnectionStatus::Stopped);
    });
}

async fn run_session(state: DaemonState, mut session: iroh_moq::MoqSession) -> Result<()> {
    let _ = state.status_tx.send(ConnectionStatus::Connecting);
    let _ = state.server_tx.send(ServerMsg::Status {
        state: ConnectionStatus::Connecting,
        last_frame_age_ms: None,
    });

    // Subscribe with a hard deadline so a phone that dials in but
    // never publishes doesn't hang the session task forever.
    let consumer = match timeout(SUBSCRIBE_TIMEOUT, session.subscribe(&state.broadcast_name)).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            let reason = format!("subscribe failed: {e}");
            let _ = state.status_tx.send(ConnectionStatus::Reconnecting {
                reason: reason.clone(),
            });
            let _ = state.server_tx.send(ServerMsg::Status {
                state: ConnectionStatus::Reconnecting { reason },
                last_frame_age_ms: None,
            });
            return Ok(());
        }
        Err(_) => {
            let reason = format!(
                "subscribe timeout: peer never announced '{}'",
                state.broadcast_name
            );
            let _ = state.status_tx.send(ConnectionStatus::Reconnecting {
                reason: reason.clone(),
            });
            let _ = state.server_tx.send(ServerMsg::Status {
                state: ConnectionStatus::Reconnecting { reason },
                last_frame_age_ms: None,
            });
            return Ok(());
        }
    };
    info!(
        broadcast = %state.broadcast_name,
        "subscribed to publisher's broadcast",
    );

    let broadcast = RemoteBroadcast::new(state.broadcast_name.clone(), consumer)
        .await
        .map_err(|e| anyhow::anyhow!("RemoteBroadcast init failed: {e}"))?;

    // We need an AudioBackend even though herd-scout doesn't render
    // audio; MediaTracks uses it to negotiate audio renditions.
    let audio = AudioBackend::default();
    let tracks = broadcast
        .media_with_decoders::<moq_media::codec::DefaultDecoders>(&audio, PlaybackConfig::default())
        .await
        .map_err(|e| anyhow::anyhow!("media_with_decoders failed: {e}"))?;

    let mut video = tracks
        .video
        .ok_or_else(|| anyhow::anyhow!("broadcast has no video track"))?;
    drop(tracks.audio);

    info!(rendition = %video.rendition(), "video track ready");
    let _ = state.status_tx.send(ConnectionStatus::Connected);
    let _ = state.server_tx.send(ServerMsg::Status {
        state: ConnectionStatus::Connected,
        last_frame_age_ms: None,
    });

    let mut limiter = PreviewLimiter::new();
    let mut frame_count: u64 = 0;
    while let Some(frame) = video.next_frame().await {
        frame_count += 1;
        let now = Instant::now();
        let frame = Arc::new(frame);

        let _ = state.frame_tx.send(Some(frame.clone()));
        let _ = state.last_frame_tx.send(Some(now));

        if frame_count % 30 == 0 {
            debug!(
                frame_count,
                width = frame.width(),
                height = frame.height(),
                "frames decoded"
            );
        }

        if limiter.should_emit(now) {
            let frame_for_preview = frame.clone();
            let server_tx = state.server_tx.clone();
            tokio::task::spawn_blocking(move || encode_preview(&frame_for_preview))
                .await
                .ok()
                .and_then(|res| res.ok())
                .map(|p: EncodedPreview| {
                    let _ = server_tx.send(ServerMsg::Frame {
                        width: p.width,
                        height: p.height,
                        pts_ms: p.pts_ms,
                        jpeg: p.jpeg,
                    });
                });
        }
    }

    info!(frame_count, "video track closed");
    let _ = state.status_tx.send(ConnectionStatus::Reconnecting {
        reason: "publisher closed".to_string(),
    });
    let _ = state.server_tx.send(ServerMsg::Status {
        state: ConnectionStatus::Reconnecting {
            reason: "publisher closed".to_string(),
        },
        last_frame_age_ms: None,
    });
    Ok(())
}

/// Periodically (every second) publish a `Status` message so any GUI
/// connecting at any moment sees a fresh `last_frame_age_ms`. Cheap
/// keep-alive; keeps the GUI's "frame age" label monotonically
/// increasing without a side channel.
pub fn spawn_status_pinger(state: DaemonState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let last = state.last_frame_tx.borrow().to_owned();
            let age_ms = last.map(|t| t.elapsed().as_millis() as u64);
            let st = state.status_tx.borrow().clone();
            let _ = state.server_tx.send(ServerMsg::Status {
                state: st,
                last_frame_age_ms: age_ms,
            });
        }
    });
}

/// Resolve the daemon's pairing ticket on boot.
///
/// Order:
/// 1. `--ticket` on the CLI / `HERD_SCOUT_TICKET` env var (legacy
///    headless path; rare in normal use but useful for ops).
/// 2. The store's saved ticket (so the same QR works across daemon
///    restarts on the same machine).
/// 3. Mint a fresh one and persist it.
///
/// **Important:** the saved ticket is only valid if its `endpoint.id`
/// equals `live.endpoint().id()` — otherwise the QR points at a
/// different daemon's iroh node. When it doesn't match we mint fresh.
pub async fn resolve_ticket(
    live: &Live,
    store: Option<&Store>,
    cli_ticket: Option<LiveTicket>,
) -> Result<(String, LiveTicket)> {
    if let Some(t) = cli_ticket {
        info!(
            broadcast = %t.broadcast_name,
            "using ticket supplied via CLI / env",
        );
        return Ok((t.broadcast_name.clone(), t));
    }
    if let Some(store) = store {
        match store.load_last_ticket().await {
            Ok(Some(t)) if t.endpoint.id == live.endpoint().id() => {
                info!(
                    broadcast = %t.broadcast_name,
                    "restored saved ticket; endpoint id matches current daemon",
                );
                return Ok((t.broadcast_name.clone(), t));
            }
            Ok(Some(t)) => {
                info!(
                    saved = %t.endpoint.id.fmt_short(),
                    current = %live.endpoint().id().fmt_short(),
                    "saved ticket points at a different endpoint; minting fresh",
                );
            }
            Ok(None) => {
                debug!("no saved ticket; minting fresh");
            }
            Err(e) => {
                warn!("could not load saved ticket: {e:#}; minting fresh");
            }
        }
    }
    let (name, ticket) = DaemonState::mint(live, store).await?;
    Ok((name, ticket))
}

/// Translate an Anyhow context message into the daemon's
/// `ConnectionStatus::Reconnecting`. Pure helper kept here for
/// reuse from `main.rs` if/when it grows a recovery path.
#[allow(dead_code, reason = "kept for future error-mapping in main.rs")]
pub fn err_to_status(e: &anyhow::Error) -> ConnectionStatus {
    ConnectionStatus::Reconnecting {
        reason: format!("{e:#}"),
    }
}

/// Convenience wrapper: re-issue the current pairing ticket to all
/// connected GUIs (e.g. after a `RequestPairing` from the GUI).
#[allow(dead_code, reason = "kept for future republish triggers / signal handlers")]
pub fn republish_pairing(state: &DaemonState) -> Result<()> {
    let bytes = state.ticket.to_string();
    state
        .server_tx
        .send(ServerMsg::Pairing { ticket: bytes })
        .map(|_| ())
        .with_context(|| "republishing pairing ticket")?;
    Ok(())
}
