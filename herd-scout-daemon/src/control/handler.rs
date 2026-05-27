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

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arc_swap::ArcSwap;
use iroh::EndpointId;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use tracing::{debug, info, warn};

use super::config::ControlConfig;

/// Hard ceiling on simultaneous SSH bridges. Decision 4 picks a number
/// large enough to keep "one user, many tabs" usable but small enough
/// that a misbehaving peer can't exhaust the daemon's fds.
const MAX_SESSIONS: usize = 16;

#[derive(Debug, Clone)]
pub(crate) struct ControlHandler {
    cfg: Arc<ArcSwap<ControlConfig>>,
    own_node_id: EndpointId,
    sessions: Arc<AtomicUsize>,
}

impl ControlHandler {
    pub(crate) fn new(cfg: Arc<ArcSwap<ControlConfig>>, own_node_id: EndpointId) -> Self {
        Self {
            cfg,
            own_node_id,
            sessions: Arc::new(AtomicUsize::new(0)),
        }
    }
}

/// RAII guard that decrements the session counter on drop. Used so an
/// early return / panic / error path can't leak a slot.
struct SessionGuard {
    sessions: Arc<AtomicUsize>,
}

impl SessionGuard {
    fn new(sessions: Arc<AtomicUsize>) -> Self {
        Self { sessions }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.sessions.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ProtocolHandler for ControlHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = connection.remote_id();

        // Allowlist + self-dial gate. fail-closed: empty allowlist drops
        // everything.
        let cfg = self.cfg.load();
        if remote == self.own_node_id || !cfg.allowed_node_ids.contains(&remote) {
            warn!(
                remote = %remote.fmt_short(),
                "control: dropping unauthorized dial",
            );
            return Ok(());
        }

        // Concurrency cap (Decision 4). fetch_add returns the *previous*
        // value, so we admit when the post-increment count is <= MAX.
        let prev = self.sessions.fetch_add(1, Ordering::AcqRel);
        if prev >= MAX_SESSIONS {
            self.sessions.fetch_sub(1, Ordering::AcqRel);
            warn!(
                remote = %remote.fmt_short(),
                active = prev,
                "control: max sessions reached, dropping dial",
            );
            return Ok(());
        }
        let _guard = SessionGuard::new(self.sessions.clone());

        let ssh_target = cfg.ssh_target;
        drop(cfg);

        info!(
            remote = %remote.fmt_short(),
            target = %ssh_target,
            "control: bridging authorized peer to local sshd",
        );

        let (mut send, mut recv) = connection.accept_bi().await.map_err(AcceptError::from_err)?;

        let mut tcp = match tokio::net::TcpStream::connect(ssh_target).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    remote = %remote.fmt_short(),
                    target = %ssh_target,
                    "control: connect to sshd failed: {e:#}",
                );
                return Ok(());
            }
        };

        let (mut tcp_r, mut tcp_w) = tcp.split();
        let bridge = tokio::try_join!(
            tokio::io::copy(&mut recv, &mut tcp_w),
            tokio::io::copy(&mut tcp_r, &mut send),
        );
        match bridge {
            Ok((up, down)) => debug!(
                remote = %remote.fmt_short(),
                client_to_sshd = up,
                sshd_to_client = down,
                "control: bridge closed cleanly",
            ),
            Err(e) => debug!(
                remote = %remote.fmt_short(),
                "control: bridge ended with io error: {e:#}",
            ),
        }
        Ok(())
    }
}
