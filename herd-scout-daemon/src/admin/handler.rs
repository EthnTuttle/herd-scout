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
use std::sync::atomic::Ordering;

use arc_swap::ArcSwap;
use herd_scout_ipc::{AdminClientMsg, AdminServerMsg, AllowedEntry, StatusReply};
use iroh::EndpointId;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use serde_json::json;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::audit::{Audit, ControlMetrics};
use crate::control::{ControlConfig, write_atomic};
use crate::ipc::frame;

/// Build the `AccessLimit` predicate for the admin RPC ALPN.
///
/// Wave 14 refactor: the admins-set membership + self-dial gate moved
/// out of `AdminHandler::accept` into iroh's router-layer
/// `AccessLimit::new`. Rejection audit lines (`admin_rejected` with
/// `reason: "self_dial" | "not_in_admins"`) are now fire-and-forget
/// via `tokio::spawn` — accepted regression: a runtime-shutdown race
/// may drop the audit-log future. The per-RPC `would_orphan_daemon`
/// guard inside `apply_mutation` stays untouched.
pub fn admins_predicate(
    cfg: Arc<ArcSwap<ControlConfig>>,
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
                "admin: dropping unauthorized dial",
            );
            let audit = audit.clone();
            let reason = if is_self { "self_dial" } else { "not_in_admins" };
            tokio::spawn(async move {
                audit
                    .log(
                        "admin_rejected",
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

#[derive(Debug, Clone)]
pub struct AdminHandler {
    cfg: Arc<ArcSwap<ControlConfig>>,
    own_node_id: EndpointId,
    config_path: PathBuf,
    audit: Audit,
    metrics: Arc<ControlMetrics>,
    /// Serializes mutating RPCs so two concurrent admins observe the
    /// same read-modify-write order. Read RPCs do not take this lock.
    write_lock: Arc<Mutex<()>>,
}

impl AdminHandler {
    pub fn new(
        cfg: Arc<ArcSwap<ControlConfig>>,
        own_node_id: EndpointId,
        config_path: PathBuf,
        audit: Audit,
        metrics: Arc<ControlMetrics>,
    ) -> Self {
        Self {
            cfg,
            own_node_id,
            config_path,
            audit,
            metrics,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    fn status(&self) -> StatusReply {
        let cfg = self.cfg.load();
        StatusReply {
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            own_node_id: self.own_node_id.to_string(),
            active_ssh_sessions: self
                .metrics
                .active_ssh_sessions
                .load(Ordering::Acquire) as u32,
            admins_count: cfg.admins.len() as u32,
            allowed_count: cfg.allowed.len() as u32,
            last_reload_unix_ms: self
                .metrics
                .last_reload_unix_ms
                .load(Ordering::Acquire),
            last_reload_source: (**self.metrics.last_reload_source.load()).to_string(),
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

        let allowed_count = new.allowed.len();
        let admins_count = new.admins.len();

        if let Err(e) = write_atomic(&self.config_path, &new) {
            return AdminServerMsg::Error {
                code: "io".to_string(),
                message: format!("write control.toml failed: {e:#}"),
            };
        }
        self.cfg.store(Arc::new(new));
        self.metrics.record_reload("admin_rpc");
        self.audit
            .log(
                "config_reload",
                None,
                None,
                json!({
                    "source": "admin_rpc",
                    "allowed_count": allowed_count,
                    "admins_count": admins_count,
                }),
            )
            .await;
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
            AdminClientMsg::TailAudit {
                last_n,
                before_ts_ms,
            } => {
                let (records, eof) = self.audit.tail(last_n, before_ts_ms).await;
                AdminServerMsg::AuditTail { records, eof }
            }
        }
    }
}

impl AdminHandler {
    /// Audit the per-RPC outcome, conditioned on the request and the
    /// daemon's reply. Read-only RPCs (List, Status, TailAudit) are
    /// not audited — they don't mutate state and can be inferred from
    /// QUIC connection logs if forensics ever cares.
    async fn audit_rpc(&self, actor: EndpointId, req: &AdminClientMsg, reply: &AdminServerMsg) {
        let actor_id = Some(actor.to_string());
        match (req, reply) {
            (AdminClientMsg::AddAllowed { node_id, label }, AdminServerMsg::Ok) => {
                self.audit
                    .log(
                        "admin_add_allowed",
                        actor_id,
                        None,
                        json!({
                            "target_node_id": node_id,
                            "target_label": label,
                        }),
                    )
                    .await;
            }
            (AdminClientMsg::RemoveAllowed { node_id }, AdminServerMsg::Ok) => {
                self.audit
                    .log(
                        "admin_remove_allowed",
                        actor_id,
                        None,
                        json!({ "target_node_id": node_id }),
                    )
                    .await;
            }
            (
                AdminClientMsg::AddAllowed { .. } | AdminClientMsg::RemoveAllowed { .. },
                AdminServerMsg::Error { code, message },
            ) => {
                self.audit
                    .log(
                        "admin_error",
                        actor_id,
                        None,
                        json!({
                            "op": match req {
                                AdminClientMsg::AddAllowed { .. } => "add_allowed",
                                AdminClientMsg::RemoveAllowed { .. } => "remove_allowed",
                                _ => "unknown",
                            },
                            "code": code,
                            "message": message,
                        }),
                    )
                    .await;
            }
            _ => {}
        }
    }
}

impl ProtocolHandler for AdminHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = connection.remote_id();

        // Allowlist + self-dial gate runs in `admins_predicate` via
        // `AccessLimit` (registered in `main.rs`). When this method is
        // entered, the connection has already been authorized.

        // Loop on bi-streams until the peer drops the connection. Each
        // bi-stream carries one request → one reply. The phone reuses
        // the connection across foreground polls (Decision 12) so it
        // doesn't pay the QUIC handshake cost on every Status refresh.
        loop {
            let (mut send, mut recv) = match connection.accept_bi().await {
                Ok(streams) => streams,
                Err(_) => {
                    // Peer closed the connection (or transport-level
                    // error). Either way, we're done with this dial.
                    return Ok(());
                }
            };

            let req_bytes = match frame::read_frame(&mut recv).await {
                Ok(Some(b)) => b,
                Ok(None) => continue, // empty stream; skip
                Err(e) => {
                    warn!(
                        remote = %remote.fmt_short(),
                        "admin: framing error: {e:#}",
                    );
                    continue;
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
                    continue;
                }
            };

            info!(
                remote = %remote.fmt_short(),
                req = ?req,
                "admin: dispatching",
            );

            let reply = self.dispatch(req.clone()).await;
            self.audit_rpc(remote, &req, &reply).await;
            let bytes =
                serde_json::to_vec(&reply).expect("AdminServerMsg always serializes");
            if let Err(e) = frame::write_frame(&mut send, &bytes).await {
                warn!(remote = %remote.fmt_short(), "admin: write reply failed: {e:#}");
            }
            let _ = send.finish();
        }
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
    async fn fixture(extras: usize) -> (AdminHandler, TempDir, PathBuf) {
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
        let audit_dir = tmp.path().join("audit");
        let audit = Audit::open(audit_dir).await.unwrap();
        let metrics = ControlMetrics::new();
        let handler = AdminHandler::new(
            cfg_swap,
            node_id_from_seed(99),
            path.clone(),
            audit,
            metrics,
        );
        (handler, tmp, path)
    }

    #[tokio::test]
    async fn add_allowed_persists_and_lists() {
        let (h, _tmp, path) = fixture(0).await;
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
        let (h, _tmp, _path) = fixture(0).await;
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
        let (h, _tmp, _path) = fixture(0).await;
        let new = node_id_from_seed(50).to_string();
        match h.add_allowed(new, "  ".into()).await {
            AdminServerMsg::Error { code, .. } => assert_eq!(code, "missing_label"),
            other => panic!("expected missing_label, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_rejects_invalid_node_id() {
        let (h, _tmp, _path) = fixture(0).await;
        match h.add_allowed("not-a-node-id".into(), "x".into()).await {
            AdminServerMsg::Error { code, .. } => assert_eq!(code, "invalid_node_id"),
            other => panic!("expected invalid_node_id, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn remove_allowed_drops_entry() {
        let (h, _tmp, path) = fixture(2).await;
        let target = node_id_from_seed(10).to_string();
        let reply = h.remove_allowed(target.clone()).await;
        assert!(matches!(reply, AdminServerMsg::Ok));
        let on_disk = crate::control::load_or_default(&path).unwrap();
        assert_eq!(on_disk.allowed.len(), 1);
    }

    #[tokio::test]
    async fn remove_rejects_unknown() {
        let (h, _tmp, _path) = fixture(0).await;
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
        let (h, _tmp, _path) = fixture(0).await;
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
        let (h, _tmp, _path) = fixture(3).await;
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

    #[tokio::test]
    async fn add_then_tail_audit_returns_record() {
        let (h, _tmp, _path) = fixture(0).await;
        let new = node_id_from_seed(50).to_string();
        h.add_allowed(new.clone(), "phone".into()).await;
        // The dispatch path is what writes the per-RPC audit. Drive it
        // through dispatch to mirror real behavior.
        h.audit_rpc(
            node_id_from_seed(1),
            &AdminClientMsg::AddAllowed {
                node_id: new.clone(),
                label: "phone".into(),
            },
            &AdminServerMsg::Ok,
        )
        .await;
        match h
            .dispatch(AdminClientMsg::TailAudit {
                last_n: 10,
                before_ts_ms: None,
            })
            .await
        {
            AdminServerMsg::AuditTail { records, .. } => {
                // The mutation path writes a `config_reload`, the
                // explicit `audit_rpc` call writes `admin_add_allowed`.
                let kinds: Vec<_> = records.iter().map(|r| r.kind.as_str()).collect();
                assert!(kinds.contains(&"admin_add_allowed"), "kinds = {kinds:?}");
                assert!(kinds.contains(&"config_reload"), "kinds = {kinds:?}");
            }
            other => panic!("expected AuditTail, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn status_reflects_admin_rpc_reload_source() {
        let (h, _tmp, _path) = fixture(0).await;
        let new = node_id_from_seed(50).to_string();
        let _ = h.add_allowed(new, "phone".into()).await;
        match h.dispatch(AdminClientMsg::Status).await {
            AdminServerMsg::Status(s) => {
                assert_eq!(s.last_reload_source, "admin_rpc");
                assert!(s.last_reload_unix_ms > 0);
            }
            other => panic!("expected Status, got {other:?}"),
        }
    }
}
