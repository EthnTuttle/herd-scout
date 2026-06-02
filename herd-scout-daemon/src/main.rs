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

// Wave 13: many formerly-bin-private modules now live under
// `lib.rs` so the upload pipeline can share them with the daemon
// binary. Bin-only modules (everything that doesn't need to be
// reachable from `upload`) stay declared here.
use herd_scout_daemon::{admin, audit, control, cv, fms_rpc, ipc, upload};
#[cfg(feature = "rekor-mirror")]
use herd_scout_daemon::audit_mirror;
mod daemon_secret;
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

    // Persist the iroh secret so the daemon's NodeId is stable across
    // restarts. Operators paste it into `~/.ssh/config` HostName and
    // peers' control.toml allowlists; rotating it on every restart
    // would break those references silently.
    if let Err(e) = daemon_secret::ensure_iroh_secret_persisted() {
        warn!("could not persist iroh secret (NodeId may rotate on restart): {e:#}");
    }

    // Open the prefs store; non-fatal if it fails.
    let store = match store::Store::open().await {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            warn!("could not open prefs store: {e:#}");
            None
        }
    };

    // Plan-FMS Phase 2: open the records store next to prefs. The two
    // share `directories::ProjectDirs` so they land in the same
    // OS-conventional data dir; the FMS files live under a `fms/`
    // subdir owned by the new crate.
    //
    // Author tag: cribbed from the prefs Store. If prefs failed to
    // open we still bring up FMS using a temp data dir + ephemeral
    // author so the IPC surface is non-fatal — record persistence is
    // best-effort until the operator fixes the data dir.
    let fms = open_fms_records().await;

    // Bring up the Live endpoint. We hand-build the iroh `Router`
    // ourselves (NOT `with_router()`) so we can mount the Wave 11
    // control-plane ALPN alongside moq + gossip on a single Endpoint.
    let live = Live::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("Live::from_env failed: {e}"))?
        .with_gossip()
        .spawn();
    let endpoint = live.endpoint().clone();
    let own_node_id = endpoint.id();

    // Wave 11 control plane: load `control.toml` (fail-closed), spawn a
    // SIGHUP-triggered reloader, build the handler. Wave 12 adds the
    // shared `ControlMetrics` and `Audit` so SSH bridge events and
    // admin RPCs land in the same on-disk log.
    let control_cfg = std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(
        control::load_or_default(&control::config_path()).unwrap_or_else(|e| {
            warn!("control: bad config at startup, closing control plane: {e:#}");
            control::ControlConfig::default()
        }),
    ));
    let metrics = audit::ControlMetrics::new();
    let audit_log = match audit::audit_dir().and_then(|d| Ok(d)) {
        Ok(dir) => match audit::Audit::open(dir).await {
            Ok(a) => {
                audit::spawn_rotation_task(a.clone());
                a
            }
            Err(e) => {
                error!("audit: open failed (audit log disabled): {e:#}");
                // Last-ditch: open under /tmp so the daemon still works.
                audit::Audit::open(std::env::temp_dir().join("herd-scout-audit"))
                    .await
                    .expect("temp_dir audit fallback")
            }
        },
        Err(e) => {
            error!("audit: cannot resolve audit dir: {e:#}");
            audit::Audit::open(std::env::temp_dir().join("herd-scout-audit"))
                .await
                .expect("temp_dir audit fallback")
        }
    };
    // Wave 14 prototype (feature-gated): wire the Rekor-mirror task to
    // the audit log so periodic Merkle-root commitments reach the
    // public Sigstore log. Off by default; enable via
    // `--features rekor-mirror` AND `[audit_mirror].enabled = true` in
    // control.toml.
    #[cfg(feature = "rekor-mirror")]
    {
        let mirror_cfg =
            audit_mirror::load_from_control_toml(&control::config_path());
        if mirror_cfg.enabled {
            // Re-derive the daemon SecretKey from the IROH_SECRET env
            // var that `daemon_secret::ensure_iroh_secret_persisted`
            // just set. Doing it this way avoids changing the public
            // API of `daemon_secret`.
            match std::env::var("IROH_SECRET")
                .ok()
                .and_then(|s| decode_hex_secret(&s).ok())
            {
                Some(secret) => {
                    if let Some(tx) =
                        audit_mirror::spawn(mirror_cfg, secret, audit_log.clone())
                    {
                        audit_log.set_mirror_tx(tx);
                        info!("audit_mirror: wired to audit log");
                    }
                }
                None => warn!(
                    "audit_mirror: enabled but no IROH_SECRET available; mirror NOT started",
                ),
            }
        }
    }

    metrics.record_reload("boot");
    audit_log
        .log(
            "daemon_boot",
            Some(own_node_id.to_string()),
            None,
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "allowed_count": control_cfg.load().allowed.len(),
                "admins_count": control_cfg.load().admins.len(),
            }),
        )
        .await;

    control::spawn_sighup_reloader(control_cfg.clone(), metrics.clone(), audit_log.clone());
    let control_handler = control::ControlHandler::new(
        control_cfg.clone(),
        metrics.clone(),
        audit_log.clone(),
    );
    let admin_handler = admin::AdminHandler::new(
        control_cfg.clone(),
        own_node_id,
        control::config_path(),
        audit_log.clone(),
        metrics.clone(),
    );

    // Wave 13: the upload subsystem.
    let uploads_dir = match upload::store::resolve_uploads_dir() {
        Ok(d) => d,
        Err(e) => {
            error!("upload: cannot resolve uploads dir, disabling upload pipeline: {e:#}");
            std::env::temp_dir().join("herd-scout-uploads-fallback")
        }
    };
    let upload_queue = match upload::queue::Queue::load_or_init(&uploads_dir).await {
        Ok(q) => q,
        Err(e) => {
            error!("upload: queue load failed: {e:#}");
            // Continue with an empty queue; persistence will retry on
            // the next mutation.
            upload::queue::Queue::load_or_init(&std::env::temp_dir().join("herd-scout-empty-queue"))
                .await
                .expect("temp queue fallback")
        }
    };

    // Server broadcast channel must exist before we build the upload
    // handler (which clones the sender for fan-out). Establish it
    // here, ahead of the rest of the channel wiring further down.
    let (server_tx, _) = broadcast::channel::<ServerMsg>(256);

    let upload_handler = upload::handler::UploadHandler::new(
        audit_log.clone(),
        upload_queue.clone(),
        uploads_dir.clone(),
        server_tx.clone(),
    );

    // Mount moq + gossip via Live::register_protocols, then add our
    // SSH-bridge ALPN (Wave 11), the admin RPC ALPN (Wave 12), and the
    // upload ALPN (Wave 13). The Router is kept alive for the lifetime
    // of `main` — dropping it aborts the accept loop.
    //
    // Wave 14: each ALPN's allowlist gate runs at the router layer via
    // `iroh::protocol::AccessLimit` so unauthorized dials are closed
    // with `(0, b"not allowed")` before our handler is invoked.
    // Rejection audit lines (`ssh_rejected`, `admin_rejected`,
    // `upload_rejected`) are emitted fire-and-forget from inside the
    // sync predicate via `tokio::spawn` — the accepted regression is
    // that a runtime-shutdown race may drop the audit-log future.
    let ssh_gate = control::ssh_allowlist_predicate(
        control_cfg.clone(),
        own_node_id,
        audit_log.clone(),
    );
    let admin_gate = admin::admins_predicate(
        control_cfg.clone(),
        own_node_id,
        audit_log.clone(),
    );
    let upload_gate = upload::handler::admins_predicate(
        control_cfg.clone(),
        audit_log.clone(),
    );
    let router = live
        .register_protocols(iroh::protocol::Router::builder(endpoint))
        .accept(
            herd_scout_ipc::CONTROL_ALPN,
            iroh::protocol::AccessLimit::new(control_handler, ssh_gate),
        )
        .accept(
            herd_scout_ipc::ADMIN_ALPN,
            iroh::protocol::AccessLimit::new(admin_handler, admin_gate),
        )
        .accept(
            herd_scout_ipc::UPLOAD_ALPN,
            iroh::protocol::AccessLimit::new(upload_handler.clone(), upload_gate),
        )
        .spawn();
    info!(
        id = %live.endpoint().id().fmt_short(),
        allowed = control_cfg.load().allowed_node_ids.len(),
        admins = control_cfg.load().admins.len(),
        audit_dir = %audit_log.dir().display(),
        "iroh endpoint bound, control plane up",
    );

    let (broadcast_name, ticket) =
        resolve_ticket(&live, store.as_deref(), cli_ticket).await?;
    info!(broadcast = %broadcast_name, "ticket ready: {ticket}");
    println!("herd-scout-daemon ticket: {ticket}");

    // Internal channels. `server_tx` was created earlier so the upload
    // handler could capture a clone before the router builder ran.
    let (frame_tx, frame_rx) = watch::channel(None);
    let (status_tx, status_rx) = watch::channel(ConnectionStatus::Idle);
    let (last_frame_tx, _last_frame_rx) = watch::channel::<Option<Instant>>(None);

    // Wave 13: a watch channel the CV task publishes the sidecar
    // handle into, so the upload processor can grab it once `Detector::new`
    // succeeds. `None` until the detector is ready.
    let (sidecar_tx, sidecar_rx) =
        watch::channel::<Option<cv::model::SidecarHandle>>(None);

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
    cv::spawn_cv_task(frame_rx.clone(), snapshot.clone(), cv_tx, Some(sidecar_tx));

    // Wave 13: spawn the upload processor. It waits on the sidecar
    // handle becoming available, then loops on the queue.
    upload::processor::spawn_processor(
        upload_queue.clone(),
        uploads_dir.clone(),
        sidecar_rx,
        status_rx,
        server_tx.clone(),
        audit_log.clone(),
    );

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

    // Plan-FMS Phase 2: bridge FMS change events to the GUI broadcast
    // and the audit log.
    if let Some(fms_handle) = fms.as_ref() {
        fms_rpc::spawn_change_bridge(
            fms_handle,
            server_tx.clone(),
            Some(audit_log.clone()),
        );
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
            ClientMsg::UploadHandoff {
                path,
                blake3_hex,
                size_bytes,
            } => {
                let h = upload_handler.clone();
                tokio::spawn(async move {
                    upload::handler::handle_local_handoff(&h, &path, &blake3_hex, size_bytes)
                        .await;
                });
            }
            ClientMsg::UploadCancel { blake3_hex } => {
                let h = upload_handler.clone();
                tokio::spawn(async move {
                    upload::handler::handle_local_cancel(&h, &blake3_hex).await;
                });
            }

            // === FMS records (Phase 2) ===
            ClientMsg::FmsCreateAsset { request_id, kind, name } => {
                if let Some(f) = fms.as_ref() {
                    fms_rpc::handle_create_asset(f, &server_tx_ctrl, request_id, kind, name);
                } else {
                    let _ = server_tx_ctrl.send(ServerMsg::FmsError {
                        request_id,
                        code: "fms_unavailable".into(),
                        message: "FMS records store failed to open at boot".into(),
                    });
                }
            }
            ClientMsg::FmsReadAsset { request_id, id } => {
                if let Some(f) = fms.as_ref() {
                    fms_rpc::handle_read_asset(f, &server_tx_ctrl, request_id, id);
                }
            }
            ClientMsg::FmsUpdateAssetField {
                request_id,
                id,
                field,
                value,
            } => {
                if let Some(f) = fms.as_ref() {
                    fms_rpc::handle_update_asset_field(
                        f,
                        &server_tx_ctrl,
                        request_id,
                        id,
                        field,
                        value,
                    );
                }
            }
            ClientMsg::FmsArchiveAsset { request_id, id } => {
                if let Some(f) = fms.as_ref() {
                    fms_rpc::handle_archive_asset(f, &server_tx_ctrl, request_id, id);
                }
            }
            ClientMsg::FmsListAssets {
                request_id,
                kind,
                include_archived,
            } => {
                if let Some(f) = fms.as_ref() {
                    fms_rpc::handle_list_assets(
                        f,
                        &server_tx_ctrl,
                        request_id,
                        kind,
                        include_archived,
                    );
                }
            }
            ClientMsg::FmsAppendLog {
                request_id,
                id,
                kind,
                ts_ns,
                asset_refs,
                quantities,
                notes,
            } => {
                if let Some(f) = fms.as_ref() {
                    fms_rpc::handle_append_log(
                        f,
                        &server_tx_ctrl,
                        request_id,
                        id,
                        kind,
                        ts_ns,
                        asset_refs,
                        quantities,
                        notes,
                    );
                }
            }
            ClientMsg::FmsReadLog { request_id, id } => {
                if let Some(f) = fms.as_ref() {
                    fms_rpc::handle_read_log(f, &server_tx_ctrl, request_id, id);
                }
            }
            ClientMsg::FmsListLogsForAsset {
                request_id,
                asset_id,
            } => {
                if let Some(f) = fms.as_ref() {
                    fms_rpc::handle_list_logs_for_asset(
                        f,
                        &server_tx_ctrl,
                        request_id,
                        asset_id,
                    );
                }
            }
            ClientMsg::FmsTagAsset {
                request_id,
                asset_id,
                term_id,
                present,
            } => {
                if let Some(f) = fms.as_ref() {
                    fms_rpc::handle_tag_asset(
                        f,
                        &server_tx_ctrl,
                        request_id,
                        asset_id,
                        term_id,
                        present,
                    );
                }
            }
        }
    }

    // Wave 11: shut the hand-built router (and its accept loop) before
    // closing the endpoint via `live.shutdown()`. Shutting the router
    // down first lets in-flight control-plane bridges drain.
    if let Err(err) = router.shutdown().await {
        warn!("error while shutting down iroh router: {err:#}");
    }
    live.shutdown().await;
    Ok(())
}

/// Opens the Plan-FMS records store under the same data dir
/// `audit::audit_dir()` resolves (typically
/// `$XDG_DATA_HOME/herd-scout/`). Best-effort: returns `None` and
/// logs on failure so the daemon's IPC surface stays up.
async fn open_fms_records() -> Option<herd_scout_fms::Fms> {
    // The audit module's data-dir resolver gives us a stable
    // OS-conventional path. The fms crate puts its files under a
    // `fms/` subdir inside whatever path we hand it.
    let data_dir = match audit::audit_dir() {
        Ok(p) => {
            // audit_dir is `<data_dir>/herd-scout`; FMS lives next
            // to it, not nested under it.
            p.parent().map(|p| p.to_path_buf()).unwrap_or(p)
        }
        Err(e) => {
            warn!("fms: cannot resolve data dir, using temp_dir: {e:#}");
            std::env::temp_dir().join("herd-scout")
        }
    };

    // Author tag: derive a stable per-device hex string. We don't
    // need the actual ed25519 secret here — the smol-kv-shaped
    // sidecar uses the tag only as the LWW tiebreaker. The Wave 12
    // identity envelope owns key material; this just borrows the
    // canonical NodeId hex form.
    let author_pub_hex = match std::env::var("IROH_SECRET") {
        Ok(s) if s.len() == 64 => s,
        _ => {
            // Fallback: a well-known string so two records from this
            // daemon are LWW-comparable. Real device-author work
            // already lives in `store/mod.rs`; we'll unify in Phase 5.
            "herd-scout-daemon-default-author".to_string()
        }
    };

    match herd_scout_fms::Fms::open(&data_dir, author_pub_hex).await {
        Ok(f) => Some(f),
        Err(e) => {
            warn!("fms: open failed (records disabled): {e:#}");
            None
        }
    }
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

#[cfg(feature = "rekor-mirror")]
fn decode_hex_secret(s: &str) -> anyhow::Result<iroh::SecretKey> {
    let s = s.trim().to_ascii_lowercase();
    if s.len() != 64 {
        anyhow::bail!("IROH_SECRET must be 64 hex chars");
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .map_err(|e| anyhow::anyhow!("hex parse: {e}"))?;
    }
    Ok(iroh::SecretKey::from_bytes(&out))
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
