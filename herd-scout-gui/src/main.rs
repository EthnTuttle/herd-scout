//! herd-scout-gui (Wave 6 split): the egui frontend that talks to a
//! local `herd-scout-daemon` over a Unix domain socket.
//!
//! On launch:
//! 1. Locate the daemon socket path (`directories::ProjectDirs` —
//!    same path used by the daemon).
//! 2. Try to connect.
//! 3. If the connection is refused / the file is missing: spawn
//!    `herd-scout-daemon` as a child process (stderr → daemon.log)
//!    and poll the socket for up to 5 s.
//! 4. Run the egui App.

#![cfg(unix)]

mod frame_view;
mod ipc;
mod overlay;
mod pairing;
mod records;
mod ui;
mod uploads;

use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use herd_scout_ipc::ClientMsg;
use tokio::process::Command;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const TICKET_ENV: &str = "HERD_SCOUT_TICKET";
/// Plan: remote-IPC bridge — env var that selects a remote daemon
/// instead of the local UDS. CLI `--daemon <NodeId>` takes precedence.
const DAEMON_NODE_ENV: &str = "HERD_SCOUT_DAEMON";

fn main() -> eframe::Result<()> {
    init_tracing();

    // Build a multi-thread tokio runtime up front; eframe is sync.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let _guard = rt.enter();

    let cli_ticket_str = parse_ticket_arg();

    // Plan: remote-IPC bridge — `--daemon <NodeId>` (or
    // `HERD_SCOUT_DAEMON=…`) skips the local UDS path and dials the
    // daemon's `herd-scout/ipc/1` ALPN over iroh.
    let remote_node = parse_daemon_node_arg();

    let handle = rt.block_on(async {
        let state = ipc::client::SharedClientState::arc();
        let ctx_for_repaint = std::sync::Mutex::new(None::<egui::Context>);
        let repaint_handle: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>> =
            std::sync::Arc::new(ctx_for_repaint);
        let rh = repaint_handle.clone();
        let cb = move || {
            if let Some(ctx) = rh.lock().ok().and_then(|g| g.clone()) {
                ctx.request_repaint();
            }
        };

        let handle = if let Some(node_id) = remote_node {
            info!(daemon = %node_id.fmt_short(), "GUI: remote mode");
            match load_or_create_gui_identity().await {
                Ok(secret) => {
                    let me = secret.public();
                    info!(
                        gui_node = %me.fmt_short(),
                        "GUI: local NodeId (must be in daemon's [control_plane.admins])",
                    );
                    ipc::client::run_remote_client(node_id, secret, state, cb).await
                }
                Err(e) => Err(anyhow!("identity load failed: {e:#}")),
            }
        } else {
            // Local UDS path — auto-spawn daemon if the socket is
            // unreachable.
            let socket_path = match daemon_socket_path() {
                Ok(p) => p,
                Err(e) => {
                    return (
                        Err::<ipc::client::IpcClientHandle, anyhow::Error>(anyhow!(
                            "could not resolve daemon socket path: {e:#}"
                        )),
                        repaint_handle,
                    );
                }
            };
            info!(path = %socket_path.display(), "GUI: local UDS mode");

            if ipc::client::connect_with_retry(&socket_path, Duration::from_millis(200))
                .await
                .is_err()
            {
                info!("daemon socket missing; spawning daemon child");
                if let Err(e) = spawn_daemon_child().await {
                    tracing::error!("could not spawn daemon: {e:#}");
                }
                if let Err(e) =
                    ipc::client::connect_with_retry(&socket_path, Duration::from_secs(5))
                        .await
                {
                    tracing::error!("daemon never came up: {e:#}");
                }
            }
            ipc::client::run_client(&socket_path, state, cb).await
        };

        (handle, repaint_handle)
    });
    let (handle_res, repaint_handle) = handle;
    let handle = match handle_res {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("could not connect to daemon: {e:#}");
            return Err(eframe::Error::AppCreation(format!("{e:#}").into()));
        }
    };

    // If we received a ticket on the CLI / env, forward it to the
    // daemon as ConnectTicket — preserves the legacy headless path.
    if let Some(t) = cli_ticket_str {
        handle.try_send(ClientMsg::ConnectTicket { ticket: t });
    }

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("herd-scout"),
        ..Default::default()
    };

    eframe::run_native(
        "herd-scout",
        native_options,
        Box::new(move |cc| {
            // Wire the egui context into the repaint bridge so the
            // IPC reader task can wake the paint loop on each frame.
            if let Ok(mut g) = repaint_handle.lock() {
                *g = Some(cc.egui_ctx.clone());
            }
            Ok(Box::new(ui::App::new(cc, handle)))
        }),
    )
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,herd_scout_gui=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn daemon_socket_path() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let mut p = PathBuf::from(xdg);
        p.push("herd-scout");
        return Ok(p.join("daemon.sock"));
    }
    let dirs = directories::ProjectDirs::from("net", "herd-scout", "herd-scout")
        .ok_or_else(|| anyhow!("no user-data directory available"))?;
    Ok(dirs.data_dir().join("daemon.sock"))
}

fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("net", "herd-scout", "herd-scout")
        .map(|d| d.data_dir().to_path_buf())
}

async fn spawn_daemon_child() -> Result<()> {
    // Find the daemon binary alongside this GUI binary.
    let me = std::env::current_exe().context("std::env::current_exe failed")?;
    let daemon = me.with_file_name("herd-scout-daemon");
    let log_path = data_dir().map(|d| {
        let _ = std::fs::create_dir_all(&d);
        d.join("daemon.log")
    });
    let stderr = match log_path.as_ref() {
        Some(p) => match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
        {
            Ok(f) => Stdio::from(f),
            Err(e) => {
                warn!("could not open {} for daemon stderr: {e}", p.display());
                Stdio::inherit()
            }
        },
        None => Stdio::inherit(),
    };

    info!(daemon = %daemon.display(), "spawning daemon child");
    let mut cmd = Command::new(&daemon);
    cmd.stdout(Stdio::null()).stderr(stderr);
    let _child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn {}", daemon.display()))?;
    Ok(())
}

/// Parses `--daemon <NodeId>` from argv or `$HERD_SCOUT_DAEMON`.
/// Returns `None` for "no remote-mode flag set" — the caller falls
/// back to the local UDS path.
fn parse_daemon_node_arg() -> Option<iroh::EndpointId> {
    let raw = cli_daemon().or_else(|| std::env::var(DAEMON_NODE_ENV).ok())?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    match iroh::EndpointId::from_str(raw) {
        Ok(id) => Some(id),
        Err(e) => {
            warn!("--daemon NodeId parse failed: {e}; falling back to local mode");
            None
        }
    }
}

fn cli_daemon() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(eq) = arg.strip_prefix("--daemon=") {
            return Some(eq.to_string());
        }
        if arg == "--daemon" {
            return args.next();
        }
    }
    None
}

/// Loads (or creates) this GUI's iroh identity envelope. Same shape
/// as `herdctl`'s identity (`herd_scout_identity::load_or_generate`)
/// so an operator can reuse the same NodeId across CLI and GUI.
async fn load_or_create_gui_identity() -> Result<iroh::SecretKey> {
    let dirs = directories::ProjectDirs::from("net", "herd-scout", "herd-scout-gui")
        .ok_or_else(|| anyhow!("no config dir available"))?;
    let path = dirs.config_dir().join("identity.toml");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let id = herd_scout_identity::load_or_generate(&path, "herd-scout-gui")
        .with_context(|| format!("load or create identity at {}", path.display()))?;
    Ok(id.secret)
}

fn parse_ticket_arg() -> Option<String> {
    let raw = cli_ticket().or_else(|| std::env::var(TICKET_ENV).ok());
    let raw = raw?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    Some(raw)
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
