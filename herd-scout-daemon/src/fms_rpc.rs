//! FMS RPC handlers (Phase 2 of plan-fms-schema-and-records-2026-06-02).
//!
//! Translates each FMS-flavored [`ClientMsg`] variant into a call on
//! [`herd_scout_fms::Fms`] and pushes a corresponding `ServerMsg` onto
//! the daemon's broadcast channel. Errors come back as
//! [`ServerMsg::FmsError`] so the GUI can show a per-request toast
//! without needing to model a separate error channel.
//!
//! This module is the *only* place that does `Ulid::from_str`,
//! `LogKind` ↔ `LogKindWire` mapping, etc. — keep the wire types and
//! the domain types syntactically separate.

use std::str::FromStr;

use herd_scout_fms::{
    Asset, AssetField as DomainAssetField, AssetKind, BlobRef, Fms, Log, LogKind, Quantity,
};
use herd_scout_fms::projection::Projection;
use herd_scout_ipc::{
    AssetFieldWire, AssetKindWire, AssetWire, AuditRecord, FmsChangeWire, LogKindWire, LogWire,
    QuantityWire, ServerMsg,
};
use tokio::sync::broadcast;
use tracing::{debug, warn};
use ulid::Ulid;

use crate::audit::{Audit, AUDIT_SCHEMA_VERSION};

/// Dispatches a single FMS ClientMsg variant. The daemon's main loop
/// calls one of these for each variant; the function spawns the
/// async work and returns immediately so the control loop is never
/// blocked on FMS I/O.
pub fn handle_create_asset(
    fms: &Fms,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    kind: AssetKindWire,
    name: String,
) {
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        match fms.create_asset(asset_kind_in(kind), &name).await {
            Ok(id) => match fms.read_asset(id).await {
                Ok(Some(asset)) => {
                    let _ = server_tx.send(ServerMsg::FmsAsset {
                        request_id,
                        asset: Some(asset_wire_out(&asset)),
                    });
                }
                Ok(None) => {
                    let _ = server_tx.send(fms_err(
                        request_id,
                        "read_after_create_empty",
                        "asset disappeared right after create",
                    ));
                }
                Err(e) => {
                    let _ = server_tx.send(fms_err(request_id, "read_failed", &e.to_string()));
                }
            },
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "create_failed", &e.to_string()));
            }
        }
    });
}

pub fn handle_read_asset(
    fms: &Fms,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    id: String,
) {
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        let Some(uid) = parse_ulid(&id) else {
            let _ = server_tx.send(fms_err(request_id, "bad_ulid", &id));
            return;
        };
        match fms.read_asset(uid).await {
            Ok(asset) => {
                let _ = server_tx.send(ServerMsg::FmsAsset {
                    request_id,
                    asset: asset.as_ref().map(asset_wire_out),
                });
            }
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "read_failed", &e.to_string()));
            }
        }
    });
}

pub fn handle_update_asset_field(
    fms: &Fms,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    id: String,
    field: AssetFieldWire,
    value: Vec<u8>,
) {
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        let Some(uid) = parse_ulid(&id) else {
            let _ = server_tx.send(fms_err(request_id, "bad_ulid", &id));
            return;
        };
        let domain_field = match field {
            AssetFieldWire::Name => DomainAssetField::Name,
            AssetFieldWire::Notes => DomainAssetField::Notes,
            AssetFieldWire::Geom => DomainAssetField::Geom,
            AssetFieldWire::Parent => DomainAssetField::Parent,
        };
        if let Err(e) = fms.update_asset_field(uid, domain_field, &value).await {
            let _ = server_tx.send(fms_err(request_id, "update_failed", &e.to_string()));
            return;
        }
        match fms.read_asset(uid).await {
            Ok(asset) => {
                let _ = server_tx.send(ServerMsg::FmsAsset {
                    request_id,
                    asset: asset.as_ref().map(asset_wire_out),
                });
            }
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "read_failed", &e.to_string()));
            }
        }
    });
}

pub fn handle_archive_asset(
    fms: &Fms,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    id: String,
) {
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        let Some(uid) = parse_ulid(&id) else {
            let _ = server_tx.send(fms_err(request_id, "bad_ulid", &id));
            return;
        };
        if let Err(e) = fms.archive_asset(uid).await {
            let _ = server_tx.send(fms_err(request_id, "archive_failed", &e.to_string()));
            return;
        }
        match fms.read_asset(uid).await {
            Ok(asset) => {
                let _ = server_tx.send(ServerMsg::FmsAsset {
                    request_id,
                    asset: asset.as_ref().map(asset_wire_out),
                });
            }
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "read_failed", &e.to_string()));
            }
        }
    });
}

pub fn handle_list_assets(
    fms: &Fms,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    kind: AssetKindWire,
    include_archived: bool,
) {
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        match fms.list_assets_by_kind(asset_kind_in(kind), include_archived).await {
            Ok(assets) => {
                let _ = server_tx.send(ServerMsg::FmsAssetList {
                    request_id,
                    kind,
                    assets: assets.iter().map(asset_wire_out).collect(),
                });
            }
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "list_failed", &e.to_string()));
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn handle_append_log(
    fms: &Fms,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    id: String,
    kind: LogKindWire,
    ts_ns: u64,
    asset_refs: Vec<String>,
    quantities: Vec<QuantityWire>,
    notes: String,
) {
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        let Some(uid) = parse_ulid(&id) else {
            let _ = server_tx.send(fms_err(request_id, "bad_ulid", &id));
            return;
        };
        let mut refs = Vec::with_capacity(asset_refs.len());
        for r in &asset_refs {
            match parse_ulid(r) {
                Some(u) => refs.push(u),
                None => {
                    let _ = server_tx.send(fms_err(request_id, "bad_asset_ref", r));
                    return;
                }
            }
        }
        let log = Log {
            id: uid,
            kind: log_kind_in(kind),
            timestamp: herd_scout_fms::Hlc::new(ts_ns, 0),
            asset_refs: refs,
            quantities: quantities.into_iter().map(quantity_in).collect(),
            photos: Vec::<BlobRef>::new(),
            notes,
        };
        match fms.append_log(&log).await {
            Ok(_) => match fms.read_log(uid).await {
                Ok(read) => {
                    let _ = server_tx.send(ServerMsg::FmsLog {
                        request_id,
                        log: read.as_ref().map(log_wire_out),
                    });
                }
                Err(e) => {
                    let _ = server_tx
                        .send(fms_err(request_id, "read_after_append_failed", &e.to_string()));
                }
            },
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "append_failed", &e.to_string()));
            }
        }
    });
}

pub fn handle_read_log(
    fms: &Fms,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    id: String,
) {
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        let Some(uid) = parse_ulid(&id) else {
            let _ = server_tx.send(fms_err(request_id, "bad_ulid", &id));
            return;
        };
        match fms.read_log(uid).await {
            Ok(log) => {
                let _ = server_tx.send(ServerMsg::FmsLog {
                    request_id,
                    log: log.as_ref().map(log_wire_out),
                });
            }
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "read_log_failed", &e.to_string()));
            }
        }
    });
}

pub fn handle_list_logs_for_asset(
    fms: &Fms,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    asset_id: String,
) {
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        let Some(uid) = parse_ulid(&asset_id) else {
            let _ = server_tx.send(fms_err(request_id, "bad_ulid", &asset_id));
            return;
        };
        match fms.list_logs_for_asset(uid).await {
            Ok(logs) => {
                let _ = server_tx.send(ServerMsg::FmsLogList {
                    request_id,
                    asset_id,
                    logs: logs.iter().map(log_wire_out).collect(),
                });
            }
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "list_logs_failed", &e.to_string()));
            }
        }
    });
}

/// Plan-FMS Phase 3b: full-text search across log notes via the
/// projection layer. The reply reuses `FmsLogList` so the GUI's
/// existing log-rendering path can render hits without learning a
/// new variant; `asset_id` is an empty string to flag "this list
/// isn't scoped to one asset."
pub fn handle_search_logs(
    fms: &Fms,
    projection: Option<&Projection>,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    query: String,
    limit: u32,
) {
    let Some(projection) = projection.cloned() else {
        let _ = server_tx.send(fms_err(
            request_id,
            "projection_unavailable",
            "FMS SQLite projection failed to open at boot",
        ));
        return;
    };
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        match projection.search_logs(&query, limit).await {
            Ok(hits) => {
                let mut logs = Vec::with_capacity(hits.len());
                for hit in hits {
                    match fms.read_log(hit.log_id).await {
                        Ok(Some(log)) => logs.push(log_wire_out(&log)),
                        Ok(None) => {} // log was tombstoned between hit and read
                        Err(e) => {
                            warn!("projection: hit read failed: {e:#}");
                        }
                    }
                }
                let _ = server_tx.send(ServerMsg::FmsLogList {
                    request_id,
                    asset_id: String::new(),
                    logs,
                });
            }
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "search_failed", &e.to_string()));
            }
        }
    });
}

pub fn handle_tag_asset(
    fms: &Fms,
    server_tx: &broadcast::Sender<ServerMsg>,
    request_id: u64,
    asset_id: String,
    term_id: String,
    present: bool,
) {
    let fms = fms.clone();
    let server_tx = server_tx.clone();
    tokio::spawn(async move {
        let Some(asset_uid) = parse_ulid(&asset_id) else {
            let _ = server_tx.send(fms_err(request_id, "bad_ulid", &asset_id));
            return;
        };
        let Some(term_uid) = parse_ulid(&term_id) else {
            let _ = server_tx.send(fms_err(request_id, "bad_ulid", &term_id));
            return;
        };
        let res = if present {
            fms.tag_asset(asset_uid, term_uid).await
        } else {
            fms.untag_asset(asset_uid, term_uid).await
        };
        if let Err(e) = res {
            let _ = server_tx.send(fms_err(request_id, "tag_failed", &e.to_string()));
            return;
        }
        match fms.read_asset(asset_uid).await {
            Ok(asset) => {
                let _ = server_tx.send(ServerMsg::FmsAsset {
                    request_id,
                    asset: asset.as_ref().map(asset_wire_out),
                });
            }
            Err(e) => {
                let _ = server_tx.send(fms_err(request_id, "read_failed", &e.to_string()));
            }
        }
    });
}

/// Spawns a task that bridges every [`herd_scout_fms::ChangeEvent`] to
/// a [`ServerMsg::FmsChange`] broadcast and an audit-log record. The
/// IPC server already fans broadcasts out to every connected GUI; we
/// just translate the shape and emit. The audit log is best-effort —
/// `Audit::append` swallows write errors per its module header.
///
/// `audit` is `Some` whenever the daemon's main loop opened an audit
/// writer; when it's `None` we skip auditing but still emit the
/// change broadcast (the daemon's audit fallback under `temp_dir()`
/// makes this branch essentially never taken in practice).
pub fn spawn_change_bridge(
    fms: &Fms,
    server_tx: broadcast::Sender<ServerMsg>,
    audit: Option<Audit>,
) {
    let mut rx = fms.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let key = match std::str::from_utf8(&ev.key) {
                        Ok(s) => s.to_string(),
                        Err(_) => {
                            warn!("fms change has non-utf8 key; skipping");
                            continue;
                        }
                    };
                    let entity_hint = entity_hint(&key);
                    let strategy = match ev.strategy {
                        herd_scout_fms::model::ChangeStrategy::LastWriteWins => "lww",
                        herd_scout_fms::model::ChangeStrategy::AddWinsSet => "add_wins_set",
                        herd_scout_fms::model::ChangeStrategy::AppendOnly => "append_only",
                    };
                    let _ = server_tx.send(ServerMsg::FmsChange {
                        event: FmsChangeWire {
                            scope: ev.scope.clone(),
                            key: key.clone(),
                            ts_ns: ev.hlc.ts_ns,
                            strategy: strategy.to_string(),
                            entity_hint: entity_hint.clone(),
                        },
                    });

                    if let Some(audit) = audit.as_ref() {
                        let kind = match entity_hint.as_deref() {
                            Some("log") => "fms_log_append",
                            Some(_) => "fms_asset_write",
                            None => "fms_other_write",
                        };
                        let details = serde_json::json!({
                            "key": key,
                            "scope": ev.scope,
                            "strategy": strategy,
                            "hlc_ts_ns": ev.hlc.ts_ns,
                            "hlc_counter": ev.hlc.counter,
                        });
                        let record = AuditRecord {
                            schema_version: AUDIT_SCHEMA_VERSION,
                            ts_ms: now_unix_ms(),
                            kind: kind.to_string(),
                            actor_node_id: None,
                            actor_label: None,
                            details,
                        };
                        audit.append(record).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("fms change bridge lagged by {n}; continuing");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    debug!("fms change bridge: channel closed; exiting");
                    return;
                }
            }
        }
    });
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------

fn parse_ulid(s: &str) -> Option<Ulid> {
    Ulid::from_str(s).ok()
}

fn asset_kind_in(k: AssetKindWire) -> AssetKind {
    match k {
        AssetKindWire::Animal => AssetKind::Animal,
        AssetKindWire::Group => AssetKind::Group,
        AssetKindWire::Land => AssetKind::Land,
        AssetKindWire::Equipment => AssetKind::Equipment,
    }
}

fn asset_kind_out(k: AssetKind) -> AssetKindWire {
    match k {
        AssetKind::Animal => AssetKindWire::Animal,
        AssetKind::Group => AssetKindWire::Group,
        AssetKind::Land => AssetKindWire::Land,
        AssetKind::Equipment => AssetKindWire::Equipment,
    }
}

fn log_kind_in(k: LogKindWire) -> LogKind {
    match k {
        LogKindWire::Observation => LogKind::Observation,
        LogKindWire::Medical => LogKind::Medical,
        LogKindWire::Movement => LogKind::Movement,
        LogKindWire::Weight => LogKind::Weight,
        LogKindWire::Birth => LogKind::Birth,
    }
}

fn log_kind_out(k: LogKind) -> LogKindWire {
    match k {
        LogKind::Observation => LogKindWire::Observation,
        LogKind::Medical => LogKindWire::Medical,
        LogKind::Movement => LogKindWire::Movement,
        LogKind::Weight => LogKindWire::Weight,
        LogKind::Birth => LogKindWire::Birth,
    }
}

fn quantity_in(q: QuantityWire) -> Quantity {
    Quantity {
        measure: q.measure,
        value: q.value,
        unit: q.unit,
        label: q.label,
    }
}

fn quantity_out(q: &Quantity) -> QuantityWire {
    QuantityWire {
        measure: q.measure.clone(),
        value: q.value,
        unit: q.unit.clone(),
        label: q.label.clone(),
    }
}

fn asset_wire_out(a: &Asset) -> AssetWire {
    AssetWire {
        id: a.id.to_string(),
        kind: asset_kind_out(a.kind),
        name: a.name.clone(),
        notes: a.notes.clone(),
        parent: a.parent.map(|p| p.to_string()),
        archived: a.archived,
        tags: a.tags.iter().map(|t| t.to_string()).collect(),
    }
}

fn log_wire_out(l: &Log) -> LogWire {
    LogWire {
        id: l.id.to_string(),
        kind: log_kind_out(l.kind),
        ts_ns: l.timestamp.ts_ns,
        asset_refs: l.asset_refs.iter().map(|a| a.to_string()).collect(),
        quantities: l.quantities.iter().map(quantity_out).collect(),
        notes: l.notes.clone(),
    }
}

fn fms_err(request_id: u64, code: &str, message: &str) -> ServerMsg {
    ServerMsg::FmsError {
        request_id,
        code: code.to_string(),
        message: message.to_string(),
    }
}

/// Cheap entity hint for a UTF-8 key. Returns `Some("animal" | "group"
/// | "land" | "equipment")` when the prefix names an asset and the
/// key carries a `kind` field; falls back to `Some("asset")` /
/// `Some("log")` otherwise so the GUI can still dispatch to the right
/// list view.
fn entity_hint(key: &str) -> Option<String> {
    if let Some(rest) = key.strip_prefix("asset/") {
        let mut parts = rest.split('/');
        let _id = parts.next()?;
        match parts.next()? {
            "kind" => None, // value carries the kind; consumer reads it
            _ => Some("asset".into()),
        }
    } else if key.starts_with("log/") {
        Some("log".into())
    } else {
        None
    }
}
