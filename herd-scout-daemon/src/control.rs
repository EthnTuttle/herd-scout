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
use serde_json::json;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{error, info, warn};

use crate::audit::{Audit, ControlMetrics};

/// Spawn the SIGHUP-reload task. Drops `Arc` clones on shutdown.
///
/// Wave 12: each successful reload also stamps `ControlMetrics` and
/// emits a `config_reload` audit record so `Status` and the History
/// tab can surface "operator hand-edited at HH:MM."
pub(crate) fn spawn_sighup_reloader(
    cfg: Arc<ArcSwap<ControlConfig>>,
    metrics: Arc<ControlMetrics>,
    audit: Audit,
) {
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
                    let allowed = new.allowed.len();
                    let admins = new.admins.len();
                    info!(
                        allowed,
                        admins,
                        "control: reload OK",
                    );
                    cfg.store(Arc::new(new));
                    metrics.record_reload("sighup");
                    audit
                        .log(
                            "config_reload",
                            None,
                            None,
                            json!({
                                "source": "sighup",
                                "allowed_count": allowed,
                                "admins_count": admins,
                            }),
                        )
                        .await;
                }
                Err(e) => warn!("control: reload failed (keeping previous): {e:#}"),
            }
        }
    });
}
