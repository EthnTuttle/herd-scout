//! Wave 11 — control-plane ALPN over the daemon's iroh Endpoint.
//!
//! Registers a third ALPN (`herd_scout_ipc::CONTROL_ALPN`) on the same
//! Router that already serves moq + gossip. Incoming bi-streams are
//! gated on a NodeId allowlist read from `control.toml` (fail-closed,
//! SIGHUP reloads). Authorized streams are byte-pumped into local
//! sshd at `127.0.0.1:22`. See
//! `.wiki/output/plan-iroh-bound-ssh-access-daemon-2026-05-26.md`.

mod config;
mod handler;

pub(crate) use config::{ControlConfig, config_path, load_or_default, write_atomic};
pub(crate) use handler::ControlHandler;

use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{error, info, warn};

/// Spawn the SIGHUP-reload task. Drops `Arc` clones on shutdown.
pub(crate) fn spawn_sighup_reloader(cfg: Arc<ArcSwap<ControlConfig>>) {
    tokio::spawn(async move {
        let mut sighup = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                error!("control: cannot install SIGHUP handler: {e:#}");
                return;
            }
        };
        let path = config_path();
        while sighup.recv().await.is_some() {
            match load_or_default(&path) {
                Ok(new) => {
                    info!(
                        allowed = new.allowed_node_ids.len(),
                        admins = new.admins.len(),
                        "control: reload OK",
                    );
                    cfg.store(Arc::new(new));
                }
                Err(e) => warn!("control: reload failed (keeping previous): {e:#}"),
            }
        }
    });
}
