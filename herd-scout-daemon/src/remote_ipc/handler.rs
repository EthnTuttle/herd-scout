//! `ProtocolHandler` for `REMOTE_IPC_ALPN`.
//!
//! Each authorized peer gets:
//!
//! 1. One bi-directional QUIC stream.
//! 2. A send-half task that subscribes to `to_clients_tx` (broadcast
//!    of `ServerMsg`) and writes every message as a length-prefixed
//!    JSON frame.
//! 3. A recv-half task that reads frames, decodes them as
//!    `ClientMsg`, and forwards on `from_clients_tx`.
//!
//! Drop semantics: when the recv-half exits the connection is
//! closed; the send-half observes the broadcast Lagged/Closed and
//! exits too. Multiple concurrent remote GUIs are supported because
//! the daemon's broadcast channel is multi-subscriber.
//!
//! No Hello/handshake here — same wire as the UDS. The first frame
//! the peer sees is the daemon's `ServerMsg::Hello`, and the GUI
//! sends `ClientMsg::Hello` immediately. We rely on the daemon's
//! existing UDS server to send the Hello on accept... but UDS-side
//! Hello is sent inside `serve_connection` for each UDS conn, not
//! over the broadcast. We replicate that same per-conn Hello here.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Context;
use herd_scout_ipc::{ClientMsg, ServerMsg};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::EndpointId;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};

use crate::audit::Audit;
use crate::ipc::frame;

/// Build the `AccessLimit` predicate for the remote-IPC ALPN.
/// Mirrors `admin::admins_predicate` — same semantics, different
/// audit kind.
pub fn ipc_predicate(
    cfg: Arc<arc_swap::ArcSwap<crate::control::ControlConfig>>,
    own_node_id: EndpointId,
    audit: Audit,
) -> impl Fn(EndpointId) -> bool + Send + Sync + 'static {
    move |remote: EndpointId| {
        let snapshot = cfg.load();
        let is_self = remote == own_node_id;
        let in_admins = snapshot.admins.contains(&remote);
        if is_self || !in_admins {
            warn!(
                remote = %remote.fmt_short(),
                "remote_ipc: dropping unauthorized dial",
            );
            let audit = audit.clone();
            let reason = if is_self { "self_dial" } else { "not_in_admins" };
            tokio::spawn(async move {
                audit
                    .log(
                        "remote_ipc_rejected",
                        Some(remote.to_string()),
                        None,
                        serde_json::json!({ "reason": reason }),
                    )
                    .await;
            });
            return false;
        }
        true
    }
}

/// Handler that ferries between the QUIC bi-stream and the daemon's
/// IPC channels. Cheap to clone (Arc internally).
#[derive(Debug, Clone)]
pub struct RemoteIpcHandler {
    inner: Arc<HandlerInner>,
}

#[derive(Debug)]
struct HandlerInner {
    /// `from_clients_tx`: every `ClientMsg` we read from a remote
    /// GUI gets sent here. The daemon's main loop already drains it
    /// the same way it drains UDS-GUI requests.
    from_clients_tx: mpsc::Sender<ClientMsg>,
    /// `to_clients_tx`: every `ServerMsg` the daemon broadcasts
    /// reaches every remote GUI subscriber attached to this
    /// channel.
    to_clients_tx: broadcast::Sender<ServerMsg>,
    /// Audit log handle. We append `remote_ipc_session_open` and
    /// `remote_ipc_session_close` so operators can correlate
    /// remote-GUI activity with admin RPCs.
    audit: Audit,
    /// Active session counter. Surfaced through the existing
    /// metrics so a future `Status` reply can report it.
    active_sessions: AtomicUsize,
}

impl RemoteIpcHandler {
    pub fn new(
        from_clients_tx: mpsc::Sender<ClientMsg>,
        to_clients_tx: broadcast::Sender<ServerMsg>,
        audit: Audit,
    ) -> Self {
        Self {
            inner: Arc::new(HandlerInner {
                from_clients_tx,
                to_clients_tx,
                audit,
                active_sessions: AtomicUsize::new(0),
            }),
        }
    }

    /// Returns the current count of active remote-IPC sessions.
    /// Diagnostic only.
    pub fn active_sessions(&self) -> usize {
        self.inner.active_sessions.load(Ordering::Acquire)
    }
}

impl ProtocolHandler for RemoteIpcHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = connection.remote_id();
        let inner = self.inner.clone();
        let session_id = inner.active_sessions.fetch_add(1, Ordering::AcqRel) + 1;
        info!(
            remote = %remote.fmt_short(),
            session_id,
            "remote_ipc: session opened",
        );
        inner
            .audit
            .log(
                "remote_ipc_session_open",
                Some(remote.to_string()),
                None,
                serde_json::json!({ "session_id": session_id }),
            )
            .await;

        // Run the bridge. We only support one bi-stream per
        // connection — same shape as the UDS, where one connection =
        // one GUI. Multiple bi-streams would let one peer multiplex
        // multiple GUIs over one QUIC handshake; we don't need that
        // today.
        let result = serve_remote(connection, &inner).await;

        let active = inner.active_sessions.fetch_sub(1, Ordering::AcqRel) - 1;
        info!(
            remote = %remote.fmt_short(),
            session_id,
            active_after = active,
            "remote_ipc: session closed",
        );
        inner
            .audit
            .log(
                "remote_ipc_session_close",
                Some(remote.to_string()),
                None,
                serde_json::json!({
                    "session_id": session_id,
                    "ok": result.is_ok(),
                }),
            )
            .await;

        Ok(())
    }
}

async fn serve_remote(
    connection: Connection,
    inner: &Arc<HandlerInner>,
) -> anyhow::Result<()> {
    // Accept the GUI's first (and only) bi-stream.
    let (mut send, mut recv) = connection
        .accept_bi()
        .await
        .context("accepting remote-ipc bi-stream")?;

    // Send Hello immediately. Mirrors what the UDS path does in
    // `serve_connection`.
    let hello = ServerMsg::Hello {
        daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: vec![
            "jpeg-preview".to_string(),
            "cv".to_string(),
            "fms".to_string(),
            "remote".to_string(),
        ],
    };
    let bytes = serde_json::to_vec(&hello).context("serializing Hello")?;
    frame::write_frame(&mut send, &bytes)
        .await
        .context("writing Hello to remote GUI")?;

    let mut to_client_rx = inner.to_clients_tx.subscribe();
    let from_clients_tx = inner.from_clients_tx.clone();

    let send_task = tokio::spawn(async move {
        loop {
            match to_client_rx.recv().await {
                Ok(msg) => {
                    let bytes = match serde_json::to_vec(&msg) {
                        Ok(b) => b,
                        Err(e) => {
                            warn!("remote_ipc: serialize ServerMsg failed: {e}");
                            continue;
                        }
                    };
                    if let Err(e) = frame::write_frame(&mut send, &bytes).await {
                        debug!("remote_ipc: write to remote failed (closed?): {e}");
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("remote_ipc: remote fell behind by {n} ServerMsgs; continuing");
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    let recv_task = tokio::spawn(async move {
        loop {
            match frame::read_frame(&mut recv).await {
                Ok(Some(bytes)) => match serde_json::from_slice::<ClientMsg>(&bytes) {
                    Ok(msg) => {
                        if from_clients_tx.send(msg).await.is_err() {
                            // Daemon's main loop dropped its receiver
                            // — likely shutdown. Bail.
                            return;
                        }
                    }
                    Err(e) => {
                        warn!("remote_ipc: ignoring undecodable ClientMsg: {e}");
                    }
                },
                Ok(None) => return,
                Err(e) => {
                    debug!("remote_ipc: read from remote failed: {e}");
                    return;
                }
            }
        }
    });

    // Either half exiting means the session is over. We don't
    // explicitly cancel the other half — both observe the dropped
    // QUIC stream.
    let _ = tokio::join!(send_task, recv_task);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn node_from_seed(seed: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[seed; 32]).public()
    }

    #[tokio::test]
    async fn predicate_admits_admin_peer() {
        let admin = node_from_seed(1);
        let other = node_from_seed(2);
        let cfg = crate::control::ControlConfig {
            allowed: vec![],
            allowed_node_ids: HashSet::new(),
            admins: [admin].into_iter().collect(),
            ssh_target: "127.0.0.1:22".parse().unwrap(),
        };
        let cfg_swap = Arc::new(arc_swap::ArcSwap::from_pointee(cfg));
        let tmp = tempfile::TempDir::new().unwrap();
        let audit = crate::audit::Audit::open(tmp.path().to_path_buf())
            .await
            .unwrap();

        let pred = ipc_predicate(cfg_swap, node_from_seed(99), audit);
        assert!(pred(admin), "admin peer should be admitted");
        assert!(!pred(other), "non-admin peer should be rejected");
    }

    #[tokio::test]
    async fn predicate_rejects_self_dial() {
        let own = node_from_seed(99);
        let cfg = crate::control::ControlConfig {
            allowed: vec![],
            allowed_node_ids: HashSet::new(),
            // Even if the daemon's own id sneaks into the admins
            // set, self-dial must be refused: a daemon cannot be
            // its own GUI client.
            admins: [own].into_iter().collect(),
            ssh_target: "127.0.0.1:22".parse().unwrap(),
        };
        let cfg_swap = Arc::new(arc_swap::ArcSwap::from_pointee(cfg));
        let tmp = tempfile::TempDir::new().unwrap();
        let audit = crate::audit::Audit::open(tmp.path().to_path_buf())
            .await
            .unwrap();

        let pred = ipc_predicate(cfg_swap, own, audit);
        assert!(!pred(own), "self-dial must be rejected even when in admins");
    }
}
