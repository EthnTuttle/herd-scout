//! herd-scout daemon (Wave 6 split).
//!
//! Headless background process that:
//! - owns one long-lived iroh / iroh-live `Live` instance,
//! - mints a `LiveTicket` rendezvous on boot,
//! - listens on `Moq::incoming_sessions` for phones that scanned the
//!   QR and dialed in,
//! - per-session: subscribes to the broadcast, decodes video, runs CV,
//!   emits JPEG previews,
//! - exposes everything over a Unix-domain-socket IPC server consumed
//!   by `herd-scout-gui` (or any other client speaking the wire
//!   protocol declared in the `herd-scout-ipc` crate).
//!
//! Launch:
//!
//! ```sh
//! cargo run -p herd-scout-daemon
//! cargo run -p herd-scout-daemon -- --ticket "iroh-live:..."
//! HERD_SCOUT_TICKET="iroh-live:..." cargo run -p herd-scout-daemon
//! ```

mod cv;
mod ipc;
mod pairing;
mod preview;
mod store;
mod stream;

use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use herd_scout_ipc::{ClientMsg, ConnectionStatus, ServerMsg};
use iroh_live::ticket::LiveTicket;
use iroh_live::Live;
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::stream::{DaemonState, resolve_ticket, spawn_accept_loop, spawn_status_pinger};

const TICKET_ENV: &str = "HERD_SCOUT_TICKET";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli_ticket = parse_ticket_arg();

    info!("herd-scout-daemon v{} starting", env!("CARGO_PKG_VERSION"));

    // Open the prefs store; non-fatal if it fails.
    let store = match store::Store::open().await {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            warn!("could not open prefs store: {e:#}");
            None
        }
    };

    // Bring up the Live endpoint. `with_router()` so phones can dial in;
    // `with_gossip()` for parity with the JNI publisher and to enable
    // future room-based discovery.
    let live = Live::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("Live::from_env failed: {e}"))?
        .with_router()
        .with_gossip()
        .spawn();
    info!(id = %live.endpoint().id().fmt_short(), "iroh endpoint bound");

    let (broadcast_name, ticket) =
        resolve_ticket(&live, store.as_deref(), cli_ticket).await?;
    info!(broadcast = %broadcast_name, "ticket ready: {ticket}");
    println!("herd-scout-daemon ticket: {ticket}");

    // Internal channels.
    let (server_tx, _) = broadcast::channel::<ServerMsg>(256);
    let (frame_tx, frame_rx) = watch::channel(None);
    let (status_tx, _status_rx) = watch::channel(ConnectionStatus::Idle);
    let (last_frame_tx, _last_frame_rx) = watch::channel::<Option<Instant>>(None);

    // CV → IPC mpsc; the CV task pushes Detections / CvBanner here, and
    // a small forwarder fan-outs onto the broadcast.
    let (cv_tx, mut cv_rx) = mpsc::channel::<ServerMsg>(64);
    {
        let server_tx = server_tx.clone();
        tokio::spawn(async move {
            while let Some(m) = cv_rx.recv().await {
                let _ = server_tx.send(m);
            }
        });
    }

    // CV inference task (Wave 3, ported).
    let snapshot = cv::state::new_shared_snapshot();
    cv::spawn_cv_task(frame_rx.clone(), snapshot.clone(), cv_tx);

    let state = DaemonState {
        live: live.clone(),
        ticket: ticket.clone(),
        broadcast_name: broadcast_name.clone(),
        server_tx: server_tx.clone(),
        frame_tx,
        status_tx,
        last_frame_tx,
    };

    // Republish the pairing ticket so any GUI that connects later sees
    // it via its initial backlog (the broadcast channel buffers per
    // subscriber, so the *next* GUI's subscribe-on-connect catches it).
    let _ = state.server_tx.send(ServerMsg::Pairing {
        ticket: ticket.to_string(),
    });

    // Listen for incoming moq sessions and per-session start the
    // decode / CV / preview pipeline.
    spawn_accept_loop(state.clone());
    spawn_status_pinger(state.clone());

    // IPC: bind and run the UDS server.
    let socket_path = ipc::socket_path()?;
    info!(path = %socket_path.display(), "binding daemon IPC socket");
    let listener = ipc::server::bind(&socket_path)
        .with_context(|| format!("binding daemon socket at {}", socket_path.display()))?;
    let (client_tx, mut client_rx) = mpsc::channel::<ClientMsg>(32);
    {
        let listener = listener;
        let server_tx = server_tx.clone();
        let client_tx = client_tx.clone();
        tokio::spawn(async move {
            ipc::server::run(listener, client_tx, server_tx).await;
        });
    }

    // Control loop: handle GUI requests.
    let live_for_ctrl = live.clone();
    let store_for_ctrl = store.clone();
    let server_tx_ctrl = server_tx.clone();
    let mut state = state;
    while let Some(msg) = client_rx.recv().await {
        match msg {
            ClientMsg::Hello { gui_version } => {
                info!(gui_version, "GUI hello");
                // Re-publish the current pairing on Hello so the
                // freshly-connected GUI sees the QR ticket.
                let _ = server_tx_ctrl.send(ServerMsg::Pairing {
                    ticket: state.ticket.to_string(),
                });
            }
            ClientMsg::RequestPairing => {
                debug!("RequestPairing from GUI");
                let _ = server_tx_ctrl.send(ServerMsg::Pairing {
                    ticket: state.ticket.to_string(),
                });
            }
            ClientMsg::ConnectTicket { ticket: raw } => {
                match LiveTicket::from_str(raw.trim()) {
                    Ok(t) => {
                        // The "Connect with pasted ticket" path is the
                        // legacy debug affordance; in Wave 6 the daemon
                        // is the rendezvous host so a pasted ticket
                        // would only make sense if the user wants to
                        // dial out to *another* daemon. We do that
                        // best-effort here by issuing an outbound moq
                        // connect.
                        let live_clone = live_for_ctrl.clone();
                        let server_tx = server_tx_ctrl.clone();
                        tokio::spawn(async move {
                            match live_clone.transport().connect(t.endpoint.clone()).await {
                                Ok(_session) => {
                                    info!(
                                        broadcast = %t.broadcast_name,
                                        "ConnectTicket: outbound moq session established",
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        broadcast = %t.broadcast_name,
                                        "ConnectTicket: dial failed: {e:#}",
                                    );
                                    let _ = server_tx.send(ServerMsg::Status {
                                        state: ConnectionStatus::Reconnecting {
                                            reason: format!("dial failed: {e}"),
                                        },
                                        last_frame_age_ms: None,
                                    });
                                }
                            }
                        });
                    }
                    Err(e) => {
                        warn!("ConnectTicket: parse failed: {e}");
                    }
                }
            }
            ClientMsg::CancelStream => {
                // User pressed Cancel on the reconnect overlay. We can't
                // forcibly abort the per-session task without touching
                // stream.rs (Wave 7 keeps that file stable), but we can
                // (a) flip the daemon-reported status back to Idle so
                // the GUI clears its last-rendered frame and falls back
                // to the pairing screen, and (b) re-publish the current
                // pairing ticket so the QR repaints. Any orphan
                // `run_session` task left over from a phone that's now
                // gone will exit on its own when the publisher's video
                // track closes (it transitions to
                // `Reconnecting{publisher closed}` on its way out, but
                // since we just announced Idle the GUI ignores that).
                info!("CancelStream from GUI; returning to Idle and republishing pairing");
                let _ = state.status_tx.send(ConnectionStatus::Idle);
                let _ = server_tx_ctrl.send(ServerMsg::Status {
                    state: ConnectionStatus::Idle,
                    last_frame_age_ms: None,
                });
                let _ = server_tx_ctrl.send(ServerMsg::Pairing {
                    ticket: state.ticket.to_string(),
                });
            }
            ClientMsg::ClearSavedTicket => {
                if let Some(s) = store_for_ctrl.as_deref() {
                    info!("ClearSavedTicket from GUI; re-minting fresh");
                    match DaemonState::mint(&live_for_ctrl, Some(s)).await {
                        Ok((name, t)) => {
                            state.broadcast_name = name;
                            state.ticket = t.clone();
                            let _ = server_tx_ctrl
                                .send(ServerMsg::Pairing { ticket: t.to_string() });
                        }
                        Err(e) => {
                            error!("re-mint failed: {e:#}");
                        }
                    }
                }
            }
            ClientMsg::Shutdown => {
                info!("Shutdown requested by GUI; exiting daemon");
                break;
            }
        }
    }

    live.shutdown().await;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,herd_scout_daemon=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Reads a ticket from `--ticket <value>` if present, otherwise from the
/// `HERD_SCOUT_TICKET` environment variable.
fn parse_ticket_arg() -> Option<LiveTicket> {
    let raw = cli_ticket().or_else(|| std::env::var(TICKET_ENV).ok());
    let raw = raw?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    match LiveTicket::from_str(&raw) {
        Ok(t) => Some(t),
        Err(e) => {
            warn!("failed to parse ticket: {e}");
            None
        }
    }
}

fn cli_ticket() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(eq) = arg.strip_prefix("--ticket=") {
            return Some(eq.to_string());
        }
        if arg == "--ticket" {
            return args.next();
        }
    }
    None
}
