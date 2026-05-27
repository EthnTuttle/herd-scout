//! Wave 12 — admin-plane `ProtocolHandler`.
//!
//! Gates incoming connections on `cfg.admins`, accepts a single
//! bi-directional QUIC stream, reads one length-prefixed JSON
//! [`AdminClientMsg`], and replies with one [`AdminServerMsg`]. One RPC
//! per stream; the handler closes the stream after the reply.
//!
//! Phase 2 (read-only): `ListAllowed`, `Status`.
//! Phase 3 (mutating): `AddAllowed`, `RemoveAllowed`, with atomic
//! `control.toml` rewrite and the no-orphan guard from Decision 11.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use arc_swap::ArcSwap;
use herd_scout_ipc::{AdminClientMsg, AdminServerMsg, AllowedEntry, StatusReply};
use iroh::EndpointId;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::control::{ControlConfig, write_atomic};
use crate::ipc::frame;

#[derive(Debug, Clone)]
pub(crate) struct AdminHandler {
    cfg: Arc<ArcSwap<ControlConfig>>,
    own_node_id: EndpointId,
    config_path: PathBuf,
    /// Serializes mutating RPCs so two concurrent admins observe the
    /// same read-modify-write order. Read RPCs do not take this lock.
    write_lock: Arc<Mutex<()>>,
}

impl AdminHandler {
    pub(crate) fn new(
        cfg: Arc<ArcSwap<ControlConfig>>,
        own_node_id: EndpointId,
        config_path: PathBuf,
    ) -> Self {
        Self {
            cfg,
            own_node_id,
            config_path,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    fn status(&self) -> StatusReply {
        let cfg = self.cfg.load();
        StatusReply {
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            own_node_id: self.own_node_id.to_string(),
            active_ssh_sessions: 0, // Phase 4 wires the real counter
            admins_count: cfg.admins.len() as u32,
            allowed_count: cfg.allowed.len() as u32,
            last_reload_unix_ms: 0, // Phase 4
            last_reload_source: "boot".to_string(),
            identity_schema_version: herd_scout_identity::SCHEMA_VERSION,
        }
    }

    fn allowed(&self) -> Vec<AllowedEntry> {
        self.cfg.load().allowed.clone()
    }

    /// Apply a mutation closure to a fresh snapshot of the config,
    /// re-derive `allowed_node_ids`, run the no-orphan guard, persist
    /// atomically, then publish via `ArcSwap::store`. Returns the
    /// `AdminServerMsg` to send back.
    async fn apply_mutation(
        &self,
        mutate: impl FnOnce(&mut ControlConfig) -> Result<(), AdminServerMsg>,
    ) -> AdminServerMsg {
        let _guard = self.write_lock.lock().await;
        let mut new = (**self.cfg.load()).clone();
        if let Err(e) = mutate(&mut new) {
            return e;
        }
        // Decision 11 — refuse to orphan the daemon. After this write
        // applies, at least one admin must remain.
        if new.admins.is_empty() {
            return AdminServerMsg::Error {
                code: "would_orphan_daemon".to_string(),
                message: "Cannot leave the daemon with zero admins. Add another admin device first.".to_string(),
            };
        }
        // Re-derive `allowed_node_ids` from `allowed`.
        new.allowed_node_ids = new
            .allowed
            .iter()
            .filter_map(|e| EndpointId::from_str(&e.node_id).ok())
            .collect();

        if let Err(e) = write_atomic(&self.config_path, &new) {
            return AdminServerMsg::Error {
                code: "io".to_string(),
                message: format!("write control.toml failed: {e:#}"),
            };
        }
        self.cfg.store(Arc::new(new));
        info!("admin: control.toml rewritten via admin RPC");
        AdminServerMsg::Ok
    }

    async fn add_allowed(&self, node_id: String, label: String) -> AdminServerMsg {
        let trimmed = node_id.trim().to_string();
        if trimmed.is_empty() {
            return AdminServerMsg::Error {
                code: "invalid_node_id".to_string(),
                message: "node_id is empty".to_string(),
            };
        }
        let id = match EndpointId::from_str(&trimmed) {
            Ok(id) => id,
            Err(e) => {
                return AdminServerMsg::Error {
                    code: "invalid_node_id".to_string(),
                    message: e.to_string(),
                };
            }
        };
        let trimmed_label = label.trim().to_string();
        if trimmed_label.is_empty() {
            return AdminServerMsg::Error {
                code: "missing_label".to_string(),
                message: "label is required (helps identify devices later)".to_string(),
            };
        }

        self.apply_mutation(move |cfg| {
            if cfg.allowed_node_ids.contains(&id) {
                return Err(AdminServerMsg::Error {
                    code: "already_present".to_string(),
                    message: "this node_id is already on the allowlist".to_string(),
                });
            }
            cfg.allowed.push(AllowedEntry {
                node_id: trimmed,
                label: trimmed_label,
            });
            Ok(())
        })
        .await
    }

    async fn remove_allowed(&self, node_id: String) -> AdminServerMsg {
        let trimmed = node_id.trim().to_string();
        let id = match EndpointId::from_str(&trimmed) {
            Ok(id) => id,
            Err(e) => {
                return AdminServerMsg::Error {
                    code: "invalid_node_id".to_string(),
                    message: e.to_string(),
                };
            }
        };

        self.apply_mutation(move |cfg| {
            let before = cfg.allowed.len();
            cfg.allowed.retain(|e| {
                EndpointId::from_str(&e.node_id).map_or(true, |existing| existing != id)
            });
            if cfg.allowed.len() == before {
                return Err(AdminServerMsg::Error {
                    code: "not_found".to_string(),
                    message: "this node_id is not on the allowlist".to_string(),
                });
            }
            Ok(())
        })
        .await
    }

    async fn dispatch(&self, msg: AdminClientMsg) -> AdminServerMsg {
        match msg {
            AdminClientMsg::ListAllowed => AdminServerMsg::Allowed {
                entries: self.allowed(),
            },
            AdminClientMsg::Status => AdminServerMsg::Status(self.status()),
            AdminClientMsg::AddAllowed { node_id, label } => {
                self.add_allowed(node_id, label).await
            }
            AdminClientMsg::RemoveAllowed { node_id } => self.remove_allowed(node_id).await,
        }
    }
}

impl ProtocolHandler for AdminHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = connection.remote_id();

        // Allowlist + self-dial gate. Empty admins set = closed to all.
        {
            let cfg = self.cfg.load();
            if remote == self.own_node_id || !cfg.admins.contains(&remote) {
                warn!(
                    remote = %remote.fmt_short(),
                    "admin: dropping unauthorized dial",
                );
                return Ok(());
            }
        }

        let (mut send, mut recv) = connection.accept_bi().await.map_err(AcceptError::from_err)?;

        // One RPC per stream.
        let req_bytes = match frame::read_frame(&mut recv).await {
            Ok(Some(b)) => b,
            Ok(None) => {
                warn!(
                    remote = %remote.fmt_short(),
                    "admin: stream closed before request",
                );
                return Ok(());
            }
            Err(e) => {
                warn!(
                    remote = %remote.fmt_short(),
                    "admin: framing error: {e:#}",
                );
                return Ok(());
            }
        };

        let req: AdminClientMsg = match serde_json::from_slice(&req_bytes) {
            Ok(r) => r,
            Err(e) => {
                let reply = AdminServerMsg::Error {
                    code: "bad_request".to_string(),
                    message: format!("parse error: {e}"),
                };
                let bytes = serde_json::to_vec(&reply).unwrap_or_default();
                let _ = frame::write_frame(&mut send, &bytes).await;
                let _ = send.finish();
                return Ok(());
            }
        };

        info!(
            remote = %remote.fmt_short(),
            req = ?req,
            "admin: dispatching",
        );

        let reply = self.dispatch(req).await;
        let bytes =
            serde_json::to_vec(&reply).expect("AdminServerMsg always serializes");
        if let Err(e) = frame::write_frame(&mut send, &bytes).await {
            warn!(remote = %remote.fmt_short(), "admin: write reply failed: {e:#}");
        }
        let _ = send.finish();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn node_id_from_seed(seed: u8) -> EndpointId {
        let bytes = [seed; 32];
        iroh::SecretKey::from_bytes(&bytes).public()
    }

    /// Build a handler whose config has one admin and `extras` SSH
    /// allowlist entries. Returns the handler + the path of its
    /// `control.toml` so tests can re-read after mutations.
    fn fixture(extras: usize) -> (AdminHandler, TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("control.toml");
        let admin_id = node_id_from_seed(1);
        let mut allowed = Vec::new();
        let mut allowed_set = HashSet::new();
        for i in 0..extras {
            let id = node_id_from_seed(10 + i as u8);
            allowed.push(AllowedEntry {
                node_id: id.to_string(),
                label: format!("dev-{i}"),
            });
            allowed_set.insert(id);
        }
        let cfg = ControlConfig {
            allowed,
            allowed_node_ids: allowed_set,
            admins: [admin_id].into_iter().collect(),
            ssh_target: "127.0.0.1:22".parse().unwrap(),
        };
        write_atomic(&path, &cfg).unwrap();
        let cfg_swap = Arc::new(ArcSwap::from_pointee(cfg));
        let handler = AdminHandler::new(cfg_swap, node_id_from_seed(99), path.clone());
        (handler, tmp, path)
    }

    #[tokio::test]
    async fn add_allowed_persists_and_lists() {
        let (h, _tmp, path) = fixture(0);
        let new = node_id_from_seed(50).to_string();
        let reply = h.add_allowed(new.clone(), "phone".into()).await;
        assert!(matches!(reply, AdminServerMsg::Ok), "got {reply:?}");
        // disk reflects it
        let on_disk = crate::control::load_or_default(&path).unwrap();
        assert_eq!(on_disk.allowed.len(), 1);
        assert_eq!(on_disk.allowed[0].label, "phone");
        // ListAllowed sees it
        match h.dispatch(AdminClientMsg::ListAllowed).await {
            AdminServerMsg::Allowed { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].node_id, new);
            }
            other => panic!("expected Allowed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_rejects_duplicate() {
        let (h, _tmp, _path) = fixture(0);
        let new = node_id_from_seed(50).to_string();
        let _ = h.add_allowed(new.clone(), "phone".into()).await;
        let reply = h.add_allowed(new, "again".into()).await;
        match reply {
            AdminServerMsg::Error { code, .. } => assert_eq!(code, "already_present"),
            other => panic!("expected already_present, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_rejects_missing_label() {
        let (h, _tmp, _path) = fixture(0);
        let new = node_id_from_seed(50).to_string();
        match h.add_allowed(new, "  ".into()).await {
            AdminServerMsg::Error { code, .. } => assert_eq!(code, "missing_label"),
            other => panic!("expected missing_label, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_rejects_invalid_node_id() {
        let (h, _tmp, _path) = fixture(0);
        match h.add_allowed("not-a-node-id".into(), "x".into()).await {
            AdminServerMsg::Error { code, .. } => assert_eq!(code, "invalid_node_id"),
            other => panic!("expected invalid_node_id, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn remove_allowed_drops_entry() {
        let (h, _tmp, path) = fixture(2);
        let target = node_id_from_seed(10).to_string();
        let reply = h.remove_allowed(target.clone()).await;
        assert!(matches!(reply, AdminServerMsg::Ok));
        let on_disk = crate::control::load_or_default(&path).unwrap();
        assert_eq!(on_disk.allowed.len(), 1);
    }

    #[tokio::test]
    async fn remove_rejects_unknown() {
        let (h, _tmp, _path) = fixture(0);
        let stranger = node_id_from_seed(77).to_string();
        match h.remove_allowed(stranger).await {
            AdminServerMsg::Error { code, .. } => assert_eq!(code, "not_found"),
            other => panic!("expected not_found, got {other:?}"),
        }
    }

    /// Decision 11: removing yourself when you're the only admin must
    /// be rejected. We simulate this by mutating `cfg.admins` directly
    /// inside `apply_mutation` via a custom test path — but since the
    /// public API doesn't expose admin mutation yet (Phase 12+), we
    /// inject the empty-admins state through `apply_mutation` itself.
    #[tokio::test]
    async fn no_orphan_guard_rejects_empty_admins() {
        let (h, _tmp, _path) = fixture(0);
        let reply = h
            .apply_mutation(|cfg| {
                cfg.admins.clear();
                Ok(())
            })
            .await;
        match reply {
            AdminServerMsg::Error { code, .. } => assert_eq!(code, "would_orphan_daemon"),
            other => panic!("expected would_orphan_daemon, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn status_returns_correct_counts() {
        let (h, _tmp, _path) = fixture(3);
        match h.dispatch(AdminClientMsg::Status).await {
            AdminServerMsg::Status(s) => {
                assert_eq!(s.allowed_count, 3);
                assert_eq!(s.admins_count, 1);
                assert_eq!(
                    s.identity_schema_version,
                    herd_scout_identity::SCHEMA_VERSION
                );
            }
            other => panic!("expected Status, got {other:?}"),
        }
    }
}
