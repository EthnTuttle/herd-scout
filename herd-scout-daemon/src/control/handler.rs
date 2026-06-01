//! Wave 11 — control-plane `ProtocolHandler`.
//!
//! Accepts an iroh QUIC connection on the `herd-scout/ssh/1` ALPN, gates
//! the remote `EndpointId` (a.k.a. `NodeId`) against the allowlist from
//! `control.toml`, then byte-pumps the bidirectional stream into local
//! sshd at `cfg.ssh_target`. Fail-closed and unauthenticated dials are
//! dropped silently after a `WARN` log.
//!
//! Decisions 1-4 of the plan: one Endpoint, three ALPNs; byte-pump only
//! (no SSH parsing); allowlist gating with self-dial rejection; cap at
//! `MAX_SESSIONS` concurrent bridges.
//!
//! Wave 12: every gate rejection and every session open/close emits one
//! line to the daemon's append-only audit log (see `audit.rs`). The
//! `active_ssh_sessions` counter is shared with the admin handler via
//! `ControlMetrics` so `Status` can report it accurately.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use arc_swap::ArcSwap;
use iroh::EndpointId;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use serde_json::json;
use tracing::{debug, info, warn};

use super::config::ControlConfig;
use crate::audit::{Audit, ControlMetrics};

/// Build the `AccessLimit` predicate for the SSH/control ALPN.
///
/// Wave 14 refactor: the allowlist + self-dial gate moved out of
/// `ControlHandler::accept` so iroh can short-circuit unauthorized
/// dials at the router layer (`AccessLimit::new`). The predicate is
/// `Fn(EndpointId) -> bool` (not `FnMut`), so it must capture by
/// `Arc`/`Clone`. Rejection audit lines (`ssh_rejected` with
/// `reason: "self_dial" | "not_in_allowed"`) are now fire-and-forget
/// via `tokio::spawn` — accepted regression: a runtime-shutdown race
/// drops the future quietly.
///
/// The `MAX_SESSIONS` cap stays inside the handler (it's stateful and
/// can't be expressed as a stateless predicate); its `ssh_rejected`
/// audit line with `reason: "max_sessions"` is unchanged.
pub fn allowlist_predicate(
    cfg: Arc<ArcSwap<ControlConfig>>,
    own_node_id: EndpointId,
    audit: Audit,
) -> impl Fn(EndpointId) -> bool + Send + Sync + 'static {
    move |remote: EndpointId| {
        let snapshot = cfg.load();
        let is_self = remote == own_node_id;
        let in_allowed = snapshot.allowed_node_ids.contains(&remote);
        if is_self || !in_allowed {
            warn!(
                remote = %remote.fmt_short(),
                "control: dropping unauthorized dial",
            );
            let audit = audit.clone();
            let reason = if is_self { "self_dial" } else { "not_in_allowed" };
            tokio::spawn(async move {
                audit
                    .log(
                        "ssh_rejected",
                        Some(remote.to_string()),
                        None,
                        json!({ "reason": reason }),
                    )
                    .await;
            });
            return false;
        }
        true
    }
}

/// Hard ceiling on simultaneous SSH bridges. Decision 4 picks a number
/// large enough to keep "one user, many tabs" usable but small enough
/// that a misbehaving peer can't exhaust the daemon's fds.
const MAX_SESSIONS: usize = 16;

#[derive(Debug, Clone)]
pub struct ControlHandler {
    cfg: Arc<ArcSwap<ControlConfig>>,
    metrics: Arc<ControlMetrics>,
    audit: Audit,
}

impl ControlHandler {
    pub fn new(
        cfg: Arc<ArcSwap<ControlConfig>>,
        metrics: Arc<ControlMetrics>,
        audit: Audit,
    ) -> Self {
        Self {
            cfg,
            metrics,
            audit,
        }
    }
}

/// RAII guard that decrements the shared `active_ssh_sessions` counter
/// on drop. Used so an early return / panic / error path can't leak a
/// slot.
struct SessionGuard {
    metrics: Arc<ControlMetrics>,
}

impl SessionGuard {
    fn new(metrics: Arc<ControlMetrics>) -> Self {
        Self { metrics }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.metrics
            .active_ssh_sessions
            .fetch_sub(1, Ordering::AcqRel);
    }
}

impl ProtocolHandler for ControlHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = connection.remote_id();

        // Allowlist + self-dial gate runs in `allowlist_predicate` via
        // `AccessLimit` (registered in `main.rs`). When this method is
        // entered, the connection has already been authorized.
        let cfg = self.cfg.load();

        // Concurrency cap (Decision 4). fetch_add returns the *previous*
        // value, so we admit when the post-increment count is <= MAX.
        let prev = self
            .metrics
            .active_ssh_sessions
            .fetch_add(1, Ordering::AcqRel);
        if prev >= MAX_SESSIONS {
            self.metrics
                .active_ssh_sessions
                .fetch_sub(1, Ordering::AcqRel);
            warn!(
                remote = %remote.fmt_short(),
                active = prev,
                "control: max sessions reached, dropping dial",
            );
            self.audit
                .log(
                    "ssh_rejected",
                    Some(remote.to_string()),
                    None,
                    json!({
                        "reason": "max_sessions",
                        "active": prev,
                    }),
                )
                .await;
            return Ok(());
        }
        let _guard = SessionGuard::new(self.metrics.clone());

        let ssh_target = cfg.ssh_target;
        drop(cfg);

        info!(
            remote = %remote.fmt_short(),
            target = %ssh_target,
            "control: bridging authorized peer to local sshd",
        );
        self.audit
            .log(
                "ssh_session_open",
                Some(remote.to_string()),
                None,
                json!({ "target": ssh_target.to_string() }),
            )
            .await;
        let started = Instant::now();

        let (mut send, mut recv) = connection.accept_bi().await.map_err(AcceptError::from_err)?;

        let mut tcp = match tokio::net::TcpStream::connect(ssh_target).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    remote = %remote.fmt_short(),
                    target = %ssh_target,
                    "control: connect to sshd failed: {e:#}",
                );
                self.audit
                    .log(
                        "ssh_session_close",
                        Some(remote.to_string()),
                        None,
                        json!({
                            "reason": "sshd_connect_failed",
                            "duration_ms": started.elapsed().as_millis() as u64,
                            "bytes_to_sshd": 0,
                            "bytes_from_sshd": 0,
                        }),
                    )
                    .await;
                return Ok(());
            }
        };

        let (mut tcp_r, mut tcp_w) = tcp.split();
        let bridge = tokio::try_join!(
            tokio::io::copy(&mut recv, &mut tcp_w),
            tokio::io::copy(&mut tcp_r, &mut send),
        );
        let duration_ms = started.elapsed().as_millis() as u64;
        match bridge {
            Ok((up, down)) => {
                debug!(
                    remote = %remote.fmt_short(),
                    client_to_sshd = up,
                    sshd_to_client = down,
                    "control: bridge closed cleanly",
                );
                self.audit
                    .log(
                        "ssh_session_close",
                        Some(remote.to_string()),
                        None,
                        json!({
                            "reason": "clean",
                            "duration_ms": duration_ms,
                            "bytes_to_sshd": up,
                            "bytes_from_sshd": down,
                        }),
                    )
                    .await;
            }
            Err(e) => {
                debug!(
                    remote = %remote.fmt_short(),
                    "control: bridge ended with io error: {e:#}",
                );
                self.audit
                    .log(
                        "ssh_session_close",
                        Some(remote.to_string()),
                        None,
                        json!({
                            "reason": "io_error",
                            "error": e.to_string(),
                            "duration_ms": duration_ms,
                        }),
                    )
                    .await;
            }
        }
        Ok(())
    }
}
