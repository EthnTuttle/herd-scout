//! SQLite read-projection (Plan-FMS Phase 3).
//!
//! ## What this is
//!
//! The on-disk smol-kv-shaped log in [`crate::store`] is the source of
//! truth. This module mirrors the materialized state into a local
//! SQLite database so callers can answer queries the prefix-scan
//! index doesn't serve well — most importantly **full-text search
//! across log notes** via SQLite's FTS5 virtual table.
//!
//! Per [[wiki/concepts/iroh-docs-fms-schema]] §"Indexing — SQLite
//! projection": *iroh-smol-kv = source of truth (immutable system of
//! record); local SQLite = projection (rebuildable at any time).*
//! Drop the SQLite file at any time and the next [`Projection::open`]
//! re-derives it from `records.jsonl`. There is no migration story
//! because there is no truth in this file.
//!
//! ## How it stays in sync
//!
//! `open` does a one-shot rebuild from the current store state, then
//! spawns a tokio task that subscribes to the [`Fms`](crate::Fms)
//! change stream and applies one `apply_change` per `ChangeEvent`.
//! The projector never writes to the store; it only reads.
//!
//! ## Restart correctness
//!
//! On restart we wipe the SQLite file and rebuild from the (replayed)
//! in-memory index. This is cheap at v1 dataset size — the wiki gate
//! is "≤1k animals + ≤10k logs over a year." If/when that gate
//! lifts we'll add a checkpoint file recording the highest HLC
//! applied so warm restart skips the rebuild; today the cost isn't
//! worth the code.
//!
//! ## Off by default at the workspace level
//!
//! The `projection` feature is on by default *for the FMS crate*,
//! but the daemon hasn't opted in yet — the existing in-memory
//! `BTreeMap` index serves all current RPCs. The Phase 3 daemon
//! plumbing flips the switch.

#![cfg(feature = "projection")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use ulid::Ulid;

use crate::model::{AssetKind, ChangeEvent, ChangeStrategy, LogKind};
use crate::Fms;

/// Search hit returned by [`Projection::search_logs`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSearchHit {
    pub log_id: Ulid,
    pub kind: LogKind,
    pub ts_ns: u64,
    pub notes: String,
    /// FTS5 `bm25` rank (lower = better match). Cosmetic.
    pub rank: f64,
}

/// Read-projection handle. Cheap to clone.
#[derive(Clone)]
pub struct Projection {
    inner: Arc<ProjectionInner>,
}

impl std::fmt::Debug for Projection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Projection")
            .field("path", &self.inner.path)
            .finish()
    }
}

struct ProjectionInner {
    path: PathBuf,
    /// All access is serialized through one connection. SQLite
    /// itself is concurrent-readers-with-WAL, but the projector's
    /// only writer is its own change-loop, so the cost of a Mutex
    /// here is one syscall per query — fine at dataset scale.
    conn: Mutex<Connection>,
}

impl Projection {
    /// Opens (or creates+rebuilds) the projection at `<root>/projection.sqlite`.
    /// `root` is the same `<data_dir>` the [`Fms`] handle was opened
    /// against — projection sits next to `fms/records.jsonl` so a
    /// `rm -rf <root>/projection.sqlite` is the canonical reset.
    ///
    /// On open we always wipe + rebuild from the FMS in-memory state.
    /// See module docs for the rationale.
    pub async fn open(root: &Path, fms: &Fms) -> Result<Self> {
        let path = root.join("projection.sqlite");
        if path.exists() {
            // Wipe and rebuild — the projection is a pure projection,
            // never the source of truth. Easier to drop than to
            // version.
            std::fs::remove_file(&path)
                .with_context(|| format!("removing stale projection at {}", path.display()))?;
        }

        let conn = Connection::open(&path)
            .with_context(|| format!("opening projection sqlite at {}", path.display()))?;
        init_schema(&conn).context("initializing projection schema")?;

        let proj = Self {
            inner: Arc::new(ProjectionInner {
                path: path.clone(),
                conn: Mutex::new(conn),
            }),
        };

        proj.rebuild_from_store(fms).await?;
        info!(path = %path.display(), "FMS projection (re)built");
        Ok(proj)
    }

    /// Spawns the change-stream subscriber. The returned task handle
    /// is detached — drop it to let it run as long as the `Fms` is
    /// alive. The task exits when the change-stream channel closes.
    pub fn spawn_subscriber(&self, fms: &Fms) {
        let mut rx = fms.subscribe();
        let proj = self.clone();
        let fms_for_apply = fms.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if let Err(e) = proj.apply_change(&event, &fms_for_apply).await {
                            warn!("projection: apply_change failed: {e:#}");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("projection: lagged by {n}; rebuilding from store");
                        if let Err(e) = proj.rebuild_from_store(&fms_for_apply).await {
                            warn!("projection: rebuild after lag failed: {e:#}");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        debug!("projection: change-stream closed; exiting");
                        return;
                    }
                }
            }
        });
    }

    /// Full-text search across `log.notes`. Returns up to `limit`
    /// hits, ordered by FTS5's `bm25` rank.
    pub async fn search_logs(&self, query: &str, limit: u32) -> Result<Vec<LogSearchHit>> {
        let q = query.trim().to_string();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.inner.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT l.id, l.kind, l.ts_ns, l.notes, bm25(log_fts) AS rank \
                 FROM log_fts \
                 JOIN log AS l ON l.id = log_fts.log_id \
                 WHERE log_fts MATCH ?1 \
                 ORDER BY rank \
                 LIMIT ?2",
            )
            .context("preparing FTS query")?;
        let rows = stmt
            .query_map(params![q, limit as i64], |row| {
                let id_str: String = row.get(0)?;
                let kind_str: String = row.get(1)?;
                let ts_ns: i64 = row.get(2)?;
                let notes: String = row.get(3)?;
                let rank: f64 = row.get(4)?;
                Ok((id_str, kind_str, ts_ns, notes, rank))
            })
            .context("FTS query")?;

        let mut hits = Vec::new();
        for r in rows {
            let (id_str, kind_str, ts_ns, notes, rank) = r.context("decoding FTS row")?;
            let Ok(id) = id_str.parse::<Ulid>() else {
                continue;
            };
            let Some(kind) = LogKind::parse(&kind_str) else {
                continue;
            };
            hits.push(LogSearchHit {
                log_id: id,
                kind,
                ts_ns: ts_ns as u64,
                notes,
                rank,
            });
        }
        Ok(hits)
    }

    /// Returns the count of `asset` rows in the projection. Cheap;
    /// useful for tests and operator diagnostics.
    pub async fn asset_count(&self) -> Result<u64> {
        let conn = self.inner.conn.lock().await;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM asset", [], |r| r.get(0))
            .context("asset count")?;
        Ok(n as u64)
    }

    /// Returns the count of `log` rows in the projection.
    pub async fn log_count(&self) -> Result<u64> {
        let conn = self.inner.conn.lock().await;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM log", [], |r| r.get(0))
            .context("log count")?;
        Ok(n as u64)
    }

    // -------------------------------------------------------------
    // Internals
    // -------------------------------------------------------------

    async fn rebuild_from_store(&self, fms: &Fms) -> Result<()> {
        let conn = self.inner.conn.lock().await;
        // Wipe content but keep schema.
        conn.execute_batch(
            "DELETE FROM log_asset_ref; \
             DELETE FROM quantity; \
             DELETE FROM log_fts; \
             DELETE FROM log; \
             DELETE FROM asset_tag; \
             DELETE FROM asset;",
        )
        .context("clearing projection")?;
        drop(conn);

        // Walk every kind through the store API. The store is
        // already in memory; this is a pure scan + materialize.
        for kind in [
            AssetKind::Animal,
            AssetKind::Group,
            AssetKind::Land,
            AssetKind::Equipment,
        ] {
            let assets = fms.list_assets_by_kind(kind, true).await?;
            for asset in assets {
                self.upsert_asset_owned(asset).await?;
            }
        }

        // Logs: we don't have a "list all logs" API yet, so walk the
        // raw store. Cheap because the inner `BTreeMap` already has
        // everything in memory.
        let log_records = fms.debug_scan_log_records().await?;
        let log_ids: std::collections::BTreeSet<Ulid> = log_records
            .iter()
            .filter_map(|(_, key, _, _)| crate::model::parse_log_id_pub(key))
            .collect();
        for id in log_ids {
            if let Some(log) = fms.read_log(id).await? {
                self.upsert_log_owned(log).await?;
            }
        }
        // Strategy is informational only at the apply layer; kept
        // imported so the apply_change match is symmetric.
        let _ = ChangeStrategy::AppendOnly;
        Ok(())
    }

    async fn apply_change(&self, ev: &ChangeEvent, fms: &Fms) -> Result<()> {
        let key = match std::str::from_utf8(&ev.key) {
            Ok(s) => s,
            Err(_) => {
                debug!("projection: skipping non-utf8 key");
                return Ok(());
            }
        };

        // Cheap dispatch: which entity does this key target? Re-read
        // the materialized form from the store and upsert. This costs
        // one extra prefix-scan per change vs maintaining per-key
        // partial state — but partial-state correctness for add-wins
        // sets is tricky, and the rebuild is fast.
        if let Some(asset_id) = parse_asset_id_from_key(key) {
            if let Some(asset) = fms.read_asset(asset_id).await? {
                self.upsert_asset_owned(asset).await?;
            } else {
                self.delete_asset(asset_id).await?;
            }
            return Ok(());
        }
        if let Some(log_id) = parse_log_id_from_key(key) {
            // Logs are append-only; first write registers the log.
            // We always upsert (cheap, idempotent) regardless of
            // strategy.
            if let Some(log) = fms.read_log(log_id).await? {
                self.upsert_log_owned(log).await?;
            }
            // Strategy is informational only here — kept so the API
            // is symmetric for future per-strategy behaviour.
            let _ = ev.strategy;
            let _ = ChangeStrategy::AppendOnly;
            return Ok(());
        }
        Ok(())
    }

    async fn upsert_asset_owned(&self, asset: crate::model::Asset) -> Result<()> {
        let conn = self.inner.conn.lock().await;
        let id_str = asset.id.to_string();
        let parent_str = asset.parent.as_ref().map(|p| p.to_string());
        conn.execute(
            "INSERT INTO asset (id, kind, name, notes, parent, archived) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(id) DO UPDATE SET \
                kind = excluded.kind, \
                name = excluded.name, \
                notes = excluded.notes, \
                parent = excluded.parent, \
                archived = excluded.archived",
            params![
                &id_str,
                asset.kind.as_str(),
                &asset.name,
                &asset.notes,
                parent_str,
                asset.archived as i64,
            ],
        )
        .context("upsert asset")?;
        // Tags: replace the row's tag set wholesale. Cheap because
        // the materialization already resolved add-wins-set to a
        // simple Vec.
        conn.execute("DELETE FROM asset_tag WHERE asset_id = ?1", params![&id_str])
            .context("clear asset_tag")?;
        for term in &asset.tags {
            conn.execute(
                "INSERT INTO asset_tag (asset_id, term_id) VALUES (?1, ?2)",
                params![&id_str, term.to_string()],
            )
            .context("insert asset_tag")?;
        }
        Ok(())
    }

    async fn delete_asset(&self, id: Ulid) -> Result<()> {
        let conn = self.inner.conn.lock().await;
        let id_str = id.to_string();
        conn.execute("DELETE FROM asset_tag WHERE asset_id = ?1", params![&id_str])
            .ok();
        conn.execute("DELETE FROM asset WHERE id = ?1", params![&id_str])
            .ok();
        Ok(())
    }

    async fn upsert_log_owned(&self, log: crate::model::Log) -> Result<()> {
        let conn = self.inner.conn.lock().await;
        let id_str = log.id.to_string();
        // log table.
        conn.execute(
            "INSERT INTO log (id, kind, ts_ns, notes) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET \
                kind = excluded.kind, \
                ts_ns = excluded.ts_ns, \
                notes = excluded.notes",
            params![
                &id_str,
                log.kind.as_str(),
                log.timestamp.ts_ns as i64,
                &log.notes,
            ],
        )
        .context("upsert log")?;

        // Update the FTS index. Standalone FTS5 (no `content=…`) is
        // append-only-by-rowid; emulate update via delete+insert
        // keyed on the `log_id` column.
        conn.execute("DELETE FROM log_fts WHERE log_id = ?1", params![&id_str])
            .ok();
        conn.execute(
            "INSERT INTO log_fts(log_id, notes) VALUES (?1, ?2)",
            params![&id_str, &log.notes],
        )
        .context("upsert log_fts")?;

        // Refs.
        conn.execute("DELETE FROM log_asset_ref WHERE log_id = ?1", params![&id_str])
            .context("clear log_asset_ref")?;
        for asset_ref in &log.asset_refs {
            conn.execute(
                "INSERT INTO log_asset_ref (log_id, asset_id) VALUES (?1, ?2)",
                params![&id_str, asset_ref.to_string()],
            )
            .context("insert log_asset_ref")?;
        }

        // Quantities.
        conn.execute("DELETE FROM quantity WHERE log_id = ?1", params![&id_str])
            .context("clear quantity")?;
        for (i, q) in log.quantities.iter().enumerate() {
            conn.execute(
                "INSERT INTO quantity (log_id, ord, measure, value, unit, label) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    &id_str,
                    i as i64,
                    &q.measure,
                    q.value,
                    &q.unit,
                    q.label.as_deref(),
                ],
            )
            .context("insert quantity")?;
        }
        Ok(())
    }
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; \
         PRAGMA synchronous = NORMAL; \
         CREATE TABLE IF NOT EXISTS asset ( \
            id        TEXT PRIMARY KEY, \
            kind      TEXT NOT NULL, \
            name      TEXT NOT NULL, \
            notes     TEXT NOT NULL DEFAULT '', \
            parent    TEXT, \
            archived  INTEGER NOT NULL DEFAULT 0 \
         ); \
         CREATE INDEX IF NOT EXISTS idx_asset_kind ON asset(kind, archived); \
         CREATE TABLE IF NOT EXISTS asset_tag ( \
            asset_id  TEXT NOT NULL, \
            term_id   TEXT NOT NULL, \
            PRIMARY KEY (asset_id, term_id), \
            FOREIGN KEY (asset_id) REFERENCES asset(id) ON DELETE CASCADE \
         ); \
         CREATE INDEX IF NOT EXISTS idx_asset_tag_term ON asset_tag(term_id); \
         CREATE TABLE IF NOT EXISTS log ( \
            id     TEXT PRIMARY KEY, \
            kind   TEXT NOT NULL, \
            ts_ns  INTEGER NOT NULL, \
            notes  TEXT NOT NULL DEFAULT '' \
         ); \
         CREATE INDEX IF NOT EXISTS idx_log_kind_ts ON log(kind, ts_ns); \
         CREATE TABLE IF NOT EXISTS log_asset_ref ( \
            log_id    TEXT NOT NULL, \
            asset_id  TEXT NOT NULL, \
            PRIMARY KEY (log_id, asset_id), \
            FOREIGN KEY (log_id)   REFERENCES log(id) ON DELETE CASCADE, \
            FOREIGN KEY (asset_id) REFERENCES asset(id) ON DELETE CASCADE \
         ); \
         CREATE INDEX IF NOT EXISTS idx_log_asset_ref_asset ON log_asset_ref(asset_id); \
         CREATE TABLE IF NOT EXISTS quantity ( \
            log_id    TEXT NOT NULL, \
            ord       INTEGER NOT NULL, \
            measure   TEXT NOT NULL, \
            value     REAL NOT NULL, \
            unit      TEXT NOT NULL, \
            label     TEXT, \
            PRIMARY KEY (log_id, ord), \
            FOREIGN KEY (log_id) REFERENCES log(id) ON DELETE CASCADE \
         ); \
         CREATE VIRTUAL TABLE IF NOT EXISTS log_fts USING fts5( \
            log_id UNINDEXED, \
            notes, \
            tokenize = 'porter unicode61' \
         );",
    )
    .context("creating projection schema")?;
    Ok(())
}

fn parse_asset_id_from_key(key: &str) -> Option<Ulid> {
    let rest = key.strip_prefix("asset/")?;
    let id_str = rest.split('/').next()?;
    id_str.parse().ok()
}

fn parse_log_id_from_key(key: &str) -> Option<Ulid> {
    let rest = key.strip_prefix("log/")?;
    let id_str = rest.split('/').next()?;
    id_str.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetKind, Hlc, Log, LogKind, Quantity};
    use tempfile::tempdir;

    fn fixture_author() -> String {
        "deadbeef".repeat(8)
    }

    #[tokio::test]
    async fn fts_round_trip() {
        let dir = tempdir().unwrap();
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let cow = fms
            .create_asset(AssetKind::Animal, "Cow #1")
            .await
            .unwrap();

        // Append two logs with distinct notes.
        let l1 = Log {
            id: Ulid::new(),
            kind: LogKind::Observation,
            timestamp: Hlc::new(1, 0),
            asset_refs: vec![cow],
            quantities: vec![],
            photos: vec![],
            notes: "limping on left foreleg, possibly thorn".into(),
        };
        let l2 = Log {
            id: Ulid::new(),
            kind: LogKind::Medical,
            timestamp: Hlc::new(2, 0),
            asset_refs: vec![cow],
            quantities: vec![],
            photos: vec![],
            notes: "treated for thorn abscess; 14-day withdrawal".into(),
        };
        fms.append_log(&l1).await.unwrap();
        fms.append_log(&l2).await.unwrap();

        let proj = Projection::open(dir.path(), &fms).await.unwrap();
        assert_eq!(proj.asset_count().await.unwrap(), 1);
        assert_eq!(proj.log_count().await.unwrap(), 2);

        let hits = proj.search_logs("thorn", 10).await.unwrap();
        assert_eq!(hits.len(), 2);

        let hits = proj.search_logs("withdrawal", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, LogKind::Medical);

        let hits = proj.search_logs("nonexistent", 10).await.unwrap();
        assert!(hits.is_empty());

        let hits = proj.search_logs("", 10).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn projection_stays_in_sync_with_changes() {
        let dir = tempdir().unwrap();
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let proj = Projection::open(dir.path(), &fms).await.unwrap();
        proj.spawn_subscriber(&fms);

        // Initial state: empty.
        assert_eq!(proj.asset_count().await.unwrap(), 0);

        // Create an asset; the change-bridge should update the
        // projection. We give the subscriber a bounded retry to
        // settle (broadcast → tokio task → sqlite write).
        let _id = fms
            .create_asset(AssetKind::Animal, "live cow")
            .await
            .unwrap();
        for _ in 0..50 {
            if proj.asset_count().await.unwrap() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(proj.asset_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn rebuild_after_wipe_recovers_state() {
        let dir = tempdir().unwrap();
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let _ = fms
            .create_asset(AssetKind::Animal, "persistent")
            .await
            .unwrap();
        let log = Log {
            id: Ulid::new(),
            kind: LogKind::Observation,
            timestamp: Hlc::new(1, 0),
            asset_refs: vec![],
            quantities: vec![Quantity {
                measure: "weight".into(),
                value: 100.0,
                unit: "kg".into(),
                label: None,
            }],
            photos: vec![],
            notes: "searchable".into(),
        };
        fms.append_log(&log).await.unwrap();

        let proj = Projection::open(dir.path(), &fms).await.unwrap();
        assert_eq!(proj.asset_count().await.unwrap(), 1);
        assert_eq!(proj.log_count().await.unwrap(), 1);
        assert_eq!(proj.search_logs("searchable", 5).await.unwrap().len(), 1);

        // Drop projection, wipe the file, re-open. Should rebuild.
        drop(proj);
        let proj = Projection::open(dir.path(), &fms).await.unwrap();
        assert_eq!(proj.asset_count().await.unwrap(), 1);
        assert_eq!(proj.log_count().await.unwrap(), 1);
        assert_eq!(proj.search_logs("searchable", 5).await.unwrap().len(), 1);

    }
}
