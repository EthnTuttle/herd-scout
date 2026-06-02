//! Farm-management records: Asset / Log / Quantity / Term, on a
//! smol-kv-shaped on-disk store.
//!
//! ## Why this exists
//!
//! Per [[output/plan-fms-schema-and-records-2026-06-02]] §"Phase 0 audit
//! findings", the live `iroh-smol-kv` (branch `iroh-098`) ships only an
//! in-memory `Client::local(topic, config)` with a default
//! `ExpiryConfig::horizon` of ~2 minutes — useless for an FMS today.
//! `WriteScope::put` also stamps a wallclock outer-timestamp instead of
//! an HLC, contradicting the wiki's HLC mandate.
//!
//! This crate ships a durable on-disk store whose record shape is
//! identical to smol-kv's `(scope, key, SignedValue { timestamp, value,
//! signature })` plus an HLC `(ts_ns, counter)` *embedded* in the
//! value bytes (see [`Hlc`]). Migrating to a future durable smol-kv
//! backend is a backend swap, not a crate rewrite — replay each
//! record's `(scope, key, value)` through `WriteScope::put`.
//!
//! ## On-disk layout
//!
//! Under the OS-specific user-data directory derived from
//! `directories::ProjectDirs::from("net", "herd-scout", "herd-scout")`:
//!
//! - `fms/records.jsonl` — append-only log of every write. One JSON
//!   object per line (`Record`). Replayable.
//! - `fms/snapshot.json` — periodic full snapshot of the in-memory
//!   index for fast cold-start. Optional; replaying records.jsonl is
//!   always correct.
//!
//! ## Conflict strategy
//!
//! Per [[wiki/concepts/iroh-docs-fms-schema]] §"Conflict resolution":
//! three strategies chosen per field, never one strategy for the whole
//! crate.
//!
//! - [`ReadStrategy::LastWriteWins`] — pick `max_by_key((hlc.ts_ns,
//!   hlc.counter, scope))` over the matching records. Used for benign
//!   fields like `name`, `notes`.
//! - [`ReadStrategy::AddWinsSet`] — presence-only keys; tombstones at
//!   `…/_deleted`. Used for tags, asset-refs, group membership.
//! - [`ReadStrategy::AppendOnly`] — every write is its own record;
//!   no merge. Used for logs and the quantities they carry.
//!
//! ## Public surface
//!
//! [`Fms`] is the single entry point. It wraps a [`Store`] (durable,
//! append-only on disk) and exposes the asset/log/quantity API.

#![allow(clippy::or_fun_call)]

pub mod hlc;
pub mod key;
pub mod model;
#[cfg(feature = "projection")]
pub mod projection;
pub mod store;

pub use crate::hlc::Hlc;
pub use crate::model::{
    Asset, AssetKind, BlobRef, ChangeEvent, Hash, Log, LogKind, Quantity, RecordEnvelope,
};
pub use crate::store::{ReadStrategy, Store, StoreConfig};

// `Key` is a module of free fns, intentionally re-exported as a path
// so call sites read `Key::asset_name(id)`.
pub use crate::key::Key;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::broadcast;
use tracing::{debug, info};
use ulid::Ulid;

/// Top-level FMS handle. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Fms {
    inner: Arc<FmsInner>,
}

#[derive(Debug)]
struct FmsInner {
    store: Store,
    /// Public hex tag of this device's author key. Same shape as the
    /// `store/mod.rs` device_author.bin tag — see
    /// [`Fms::open`] doc.
    author_pub_hex: String,
    /// Broadcast channel for change notifications. Subscribers (e.g.
    /// the SQLite projector in Phase 3, the IPC server in Phase 2)
    /// drain receipts; if a slow subscriber lags they get a
    /// `Lagged(n)` and re-read from the store.
    events: broadcast::Sender<ChangeEvent>,
}

impl Fms {
    /// Opens (or creates) the FMS records store under `data_dir`.
    ///
    /// The on-disk files live at `<data_dir>/fms/`. Caller picks
    /// `data_dir` via `directories::ProjectDirs` (the daemon already
    /// does this for prefs in `store/mod.rs`). `author_pub_hex` is the
    /// per-device author tag — the same one `store/mod.rs` produces
    /// for prefs. The crate doesn't generate it: the daemon owns
    /// device identity (Wave 12 `herd_scout_identity` envelope), and
    /// passes the hex-encoded public half here.
    pub async fn open(data_dir: &Path, author_pub_hex: String) -> Result<Self> {
        let store = Store::open(StoreConfig {
            root: data_dir.join("fms"),
            event_buffer: 1024,
        })
        .await
        .context("opening FMS store")?;

        let events = store.subscribe();
        info!(
            data_dir = %data_dir.display(),
            author = %short_hex(&author_pub_hex),
            "FMS opened"
        );
        Ok(Self {
            inner: Arc::new(FmsInner {
                store,
                author_pub_hex,
                events,
            }),
        })
    }

    /// Subscribes to change events. Each subscriber gets every event
    /// posted after subscription; lag yields `RecvError::Lagged(n)`,
    /// which subscribers handle by re-reading from the store.
    pub fn subscribe(&self) -> broadcast::Receiver<ChangeEvent> {
        self.inner.events.subscribe()
    }

    /// Returns this device's author tag.
    pub fn author(&self) -> &str {
        &self.inner.author_pub_hex
    }

    /// Diagnostic accessor: returns the next HLC the store would
    /// stamp on the next write. Exposed for the Phase-6 HLC drift
    /// test; not part of the stable public API.
    #[doc(hidden)]
    pub async fn debug_advance_hlc(&self) -> Hlc {
        self.inner.store.advance_hlc().await
    }

    /// Diagnostic accessor: returns every record under the `log/`
    /// prefix as `(scope, key, hlc, value)`. Used by the SQLite
    /// projector's cold-rebuild path to enumerate every log id.
    /// Not part of the stable public API.
    #[doc(hidden)]
    pub async fn debug_scan_log_records(
        &self,
    ) -> Result<Vec<(String, Vec<u8>, Hlc, Vec<u8>)>> {
        self.inner.store.scan_prefix(b"log/").await
    }

    // -----------------------------------------------------------------
    // Asset CRUD
    // -----------------------------------------------------------------

    /// Creates a new asset and returns its ULID.
    ///
    /// The ULID is timestamp-prefixed so range scans give time-ordered
    /// iteration for free, per [[wiki/concepts/iroh-docs-fms-schema]]
    /// §"Key layout".
    ///
    /// **EID hand-off (P0 follow-up).** When the inventoried
    /// `herd-scout-eid` Bluetooth-reader crate
    /// (`.wiki/inventory/features/herd-scout-eid-crate.md`) lands,
    /// the reader will create animals here (or look them up by
    /// `(country, national_id)` via a future
    /// `find_animal_by_eid` method) and then append an `Observation`
    /// log via [`Self::append_log`]. EID-reconciliation residuals
    /// (Layer 5 of the counting playbook,
    /// `.wiki/wiki/concepts/herd-counting-pipeline.md`) compare the
    /// CV count against the EID-known set; that calibration loop is
    /// the wedge.
    pub async fn create_asset(&self, kind: AssetKind, name: &str) -> Result<Ulid> {
        let id = Ulid::new();
        let hlc = self.inner.store.advance_hlc().await;
        let scope = self.inner.author_pub_hex.clone();

        let mut tx = self.inner.store.begin_transaction(scope).await;
        tx.put(Key::asset_kind(id), kind.as_str().as_bytes(), hlc, ReadStrategy::LastWriteWins);
        tx.put(Key::asset_name(id), name.as_bytes(), hlc.tick(), ReadStrategy::LastWriteWins);
        tx.put(
            Key::asset_schema(id),
            b"1",
            hlc.tick().tick(),
            ReadStrategy::LastWriteWins,
        );
        tx.commit().await?;
        debug!(id = %id, kind = ?kind, name, "asset created");
        Ok(id)
    }

    /// Reads an asset by id, returning the materialized state.
    ///
    /// Applies the per-field conflict strategy: name is LWW, tags are
    /// add-wins-set. Returns `None` if the asset has no records (never
    /// written, or all records archived in the future via tombstones).
    pub async fn read_asset(&self, id: Ulid) -> Result<Option<Asset>> {
        let prefix = key::asset_prefix(id);
        let records = self.inner.store.scan_prefix(&prefix).await?;
        Ok(model::materialize_asset(id, &records))
    }

    /// Updates a single field on an asset. The field's conflict
    /// strategy is fixed by the schema (currently all asset scalars
    /// are LWW); attempting to update an append-only or set-membership
    /// field through this method returns an error.
    pub async fn update_asset_field(
        &self,
        id: Ulid,
        field: AssetField,
        value: &[u8],
    ) -> Result<()> {
        let key = match field {
            AssetField::Name => Key::asset_name(id),
            AssetField::Notes => Key::asset_notes(id),
            AssetField::Geom => Key::asset_geom(id),
            AssetField::Parent => Key::asset_parent(id),
        };
        let hlc = self.inner.store.advance_hlc().await;
        let scope = self.inner.author_pub_hex.clone();
        let mut tx = self.inner.store.begin_transaction(scope).await;
        tx.put(key, value, hlc, ReadStrategy::LastWriteWins);
        tx.commit().await?;
        Ok(())
    }

    /// Soft-archives an asset by writing `archived=true`. Asset
    /// records remain queryable for audit; the projection layer
    /// filters archived rows out of default lists.
    pub async fn archive_asset(&self, id: Ulid) -> Result<()> {
        let hlc = self.inner.store.advance_hlc().await;
        let scope = self.inner.author_pub_hex.clone();
        let mut tx = self.inner.store.begin_transaction(scope).await;
        tx.put(
            Key::asset_archived(id),
            b"true",
            hlc,
            ReadStrategy::LastWriteWins,
        );
        tx.commit().await?;
        Ok(())
    }

    /// Returns every asset of a given kind. Ordered by ULID (i.e. by
    /// creation time).
    pub async fn list_assets_by_kind(
        &self,
        kind: AssetKind,
        include_archived: bool,
    ) -> Result<Vec<Asset>> {
        let records = self.inner.store.scan_prefix(b"asset/").await?;
        Ok(model::materialize_assets_by_kind(
            &records,
            kind,
            include_archived,
        ))
    }

    // -----------------------------------------------------------------
    // Tags / asset-refs (add-wins-set semantics)
    // -----------------------------------------------------------------

    /// Adds a tag (term reference) to an asset. Add-wins-set: a later
    /// remove via [`Self::untag_asset`] writes a tombstone; reads drop
    /// the entry if any tombstone HLC ≥ all add HLCs.
    pub async fn tag_asset(&self, asset_id: Ulid, term_id: Ulid) -> Result<()> {
        let hlc = self.inner.store.advance_hlc().await;
        let scope = self.inner.author_pub_hex.clone();
        let mut tx = self.inner.store.begin_transaction(scope).await;
        tx.put(
            Key::asset_tag(asset_id, term_id),
            b"1",
            hlc,
            ReadStrategy::AddWinsSet,
        );
        tx.commit().await?;
        Ok(())
    }

    /// Removes a tag. Writes a tombstone at `…/_deleted`. Idempotent
    /// per HLC monotonicity.
    pub async fn untag_asset(&self, asset_id: Ulid, term_id: Ulid) -> Result<()> {
        let hlc = self.inner.store.advance_hlc().await;
        let scope = self.inner.author_pub_hex.clone();
        let mut tx = self.inner.store.begin_transaction(scope).await;
        tx.put(
            Key::asset_tag_tombstone(asset_id, term_id),
            b"1",
            hlc,
            ReadStrategy::AddWinsSet,
        );
        tx.commit().await?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Logs (append-only)
    // -----------------------------------------------------------------

    /// Appends a log. Logs are immutable — see
    /// [[wiki/concepts/iroh-docs-fms-schema]] §"Conflict resolution"
    /// strategy 3. The log's `id` (a ULID) is generated here; the
    /// caller fills the rest of the [`Log`] payload.
    pub async fn append_log(&self, log: &Log) -> Result<Ulid> {
        let hlc = self.inner.store.advance_hlc().await;
        let scope = self.inner.author_pub_hex.clone();
        let mut tx = self.inner.store.begin_transaction(scope).await;

        tx.put(
            Key::log_kind(log.id),
            log.kind.as_str().as_bytes(),
            hlc,
            ReadStrategy::AppendOnly,
        );
        tx.put(
            Key::log_timestamp(log.id),
            &log.timestamp.to_bytes(),
            hlc.tick(),
            ReadStrategy::AppendOnly,
        );
        if !log.notes.is_empty() {
            tx.put(
                Key::log_notes(log.id),
                log.notes.as_bytes(),
                hlc.tick().tick(),
                ReadStrategy::AppendOnly,
            );
        }
        for asset_ref in &log.asset_refs {
            tx.put(
                Key::log_asset_ref(log.id, *asset_ref),
                b"1",
                hlc.tick(),
                ReadStrategy::AddWinsSet,
            );
        }
        for (i, q) in log.quantities.iter().enumerate() {
            let qid = i as u32;
            tx.put(
                Key::log_quantity_measure(log.id, qid),
                q.measure.as_bytes(),
                hlc.tick(),
                ReadStrategy::AppendOnly,
            );
            tx.put(
                Key::log_quantity_value(log.id, qid),
                &q.value.to_le_bytes(),
                hlc.tick(),
                ReadStrategy::AppendOnly,
            );
            tx.put(
                Key::log_quantity_unit(log.id, qid),
                q.unit.as_bytes(),
                hlc.tick(),
                ReadStrategy::AppendOnly,
            );
            if let Some(label) = &q.label {
                tx.put(
                    Key::log_quantity_label(log.id, qid),
                    label.as_bytes(),
                    hlc.tick(),
                    ReadStrategy::AppendOnly,
                );
            }
        }
        for (seq, photo) in log.photos.iter().enumerate() {
            tx.put(
                Key::log_photo(log.id, seq as u32),
                photo.hash.as_bytes(),
                hlc.tick(),
                ReadStrategy::AppendOnly,
            );
            tx.put(
                Key::log_photo_mime(log.id, seq as u32),
                photo.mime.as_bytes(),
                hlc.tick(),
                ReadStrategy::AppendOnly,
            );
            tx.put(
                Key::log_photo_size(log.id, seq as u32),
                &photo.size.to_le_bytes(),
                hlc.tick(),
                ReadStrategy::AppendOnly,
            );
        }

        tx.commit().await?;
        debug!(id = %log.id, kind = ?log.kind, "log appended");
        Ok(log.id)
    }

    /// Reads a single log by id.
    pub async fn read_log(&self, id: Ulid) -> Result<Option<Log>> {
        let prefix = key::log_prefix(id);
        let records = self.inner.store.scan_prefix(&prefix).await?;
        Ok(model::materialize_log(id, &records))
    }

    /// Lists logs that reference the given asset.
    pub async fn list_logs_for_asset(&self, asset_id: Ulid) -> Result<Vec<Log>> {
        let all = self.inner.store.scan_prefix(b"log/").await?;
        Ok(model::materialize_logs_for_asset(&all, asset_id))
    }

    /// Adds a photo to a log. The bytes are NOT stored here — caller
    /// hashes them via blake3 and uploads the bytes to iroh-blobs (or
    /// equivalent), then passes the [`Hash`] in. Mirrors
    /// [[wiki/concepts/iroh-docs-fms-schema]] §"Large value / blob
    /// references": only the hash goes in the doc.
    pub async fn add_log_photo(
        &self,
        log_id: Ulid,
        seq: u32,
        photo: BlobRef,
    ) -> Result<()> {
        let hlc = self.inner.store.advance_hlc().await;
        let scope = self.inner.author_pub_hex.clone();
        let mut tx = self.inner.store.begin_transaction(scope).await;
        tx.put(
            Key::log_photo(log_id, seq),
            photo.hash.as_bytes(),
            hlc,
            ReadStrategy::AppendOnly,
        );
        tx.put(
            Key::log_photo_mime(log_id, seq),
            photo.mime.as_bytes(),
            hlc.tick(),
            ReadStrategy::AppendOnly,
        );
        tx.put(
            Key::log_photo_size(log_id, seq),
            &photo.size.to_le_bytes(),
            hlc.tick().tick(),
            ReadStrategy::AppendOnly,
        );
        tx.commit().await?;
        Ok(())
    }
}

/// Mutable scalar fields on an asset. Used by [`Fms::update_asset_field`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetField {
    Name,
    Notes,
    Geom,
    Parent,
}

fn short_hex(s: &str) -> &str {
    &s[..s.len().min(8)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fixture_author() -> String {
        "deadbeef".repeat(8)
    }

    #[tokio::test]
    async fn create_then_read_asset() {
        let dir = tempdir().unwrap();
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let id = fms
            .create_asset(AssetKind::Animal, "Cow #1234")
            .await
            .unwrap();
        let asset = fms.read_asset(id).await.unwrap().unwrap();
        assert_eq!(asset.kind, AssetKind::Animal);
        assert_eq!(asset.name, "Cow #1234");
        assert!(!asset.archived);
    }

    #[tokio::test]
    async fn lww_keeps_newer_name() {
        let dir = tempdir().unwrap();
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let id = fms
            .create_asset(AssetKind::Animal, "old name")
            .await
            .unwrap();
        fms.update_asset_field(id, AssetField::Name, b"new name")
            .await
            .unwrap();
        let asset = fms.read_asset(id).await.unwrap().unwrap();
        assert_eq!(asset.name, "new name");
    }

    #[tokio::test]
    async fn add_wins_set_tag_then_untag() {
        let dir = tempdir().unwrap();
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let asset = fms
            .create_asset(AssetKind::Animal, "tagged cow")
            .await
            .unwrap();
        let term = Ulid::new();

        fms.tag_asset(asset, term).await.unwrap();
        let read = fms.read_asset(asset).await.unwrap().unwrap();
        assert!(read.tags.contains(&term), "tag should be present");

        fms.untag_asset(asset, term).await.unwrap();
        let read = fms.read_asset(asset).await.unwrap().unwrap();
        assert!(!read.tags.contains(&term), "tag should be tombstoned");
    }

    #[tokio::test]
    async fn append_only_log_round_trip() {
        let dir = tempdir().unwrap();
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let cow = fms
            .create_asset(AssetKind::Animal, "Cow #1234")
            .await
            .unwrap();

        let log = Log {
            id: Ulid::new(),
            kind: LogKind::Weight,
            timestamp: Hlc::new(1_700_000_000_000_000_000, 0),
            asset_refs: vec![cow],
            quantities: vec![Quantity {
                measure: "weight".into(),
                value: 425.0,
                unit: "kg".into(),
                label: Some("calf #1234".into()),
            }],
            photos: vec![],
            notes: "first weighing".into(),
        };
        let log_id = fms.append_log(&log).await.unwrap();

        let read = fms.read_log(log_id).await.unwrap().unwrap();
        assert_eq!(read.kind, LogKind::Weight);
        assert_eq!(read.notes, "first weighing");
        assert_eq!(read.asset_refs, vec![cow]);
        assert_eq!(read.quantities.len(), 1);
        assert_eq!(read.quantities[0].value, 425.0);
    }

    #[tokio::test]
    async fn restart_replays_records() {
        let dir = tempdir().unwrap();
        let id = {
            let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
            fms.create_asset(AssetKind::Animal, "persistent cow")
                .await
                .unwrap()
        };
        // Re-open: should replay records.jsonl from disk.
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let read = fms.read_asset(id).await.unwrap().unwrap();
        assert_eq!(read.name, "persistent cow");
    }

    #[tokio::test]
    async fn list_logs_for_asset_filters() {
        let dir = tempdir().unwrap();
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let cow = fms.create_asset(AssetKind::Animal, "A").await.unwrap();
        let other = fms.create_asset(AssetKind::Animal, "B").await.unwrap();

        let log1 = Log {
            id: Ulid::new(),
            kind: LogKind::Observation,
            timestamp: Hlc::new(1, 0),
            asset_refs: vec![cow],
            quantities: vec![],
            photos: vec![],
            notes: "n1".into(),
        };
        let log2 = Log {
            id: Ulid::new(),
            kind: LogKind::Observation,
            timestamp: Hlc::new(2, 0),
            asset_refs: vec![other],
            quantities: vec![],
            photos: vec![],
            notes: "n2".into(),
        };
        fms.append_log(&log1).await.unwrap();
        fms.append_log(&log2).await.unwrap();

        let logs_for_cow = fms.list_logs_for_asset(cow).await.unwrap();
        assert_eq!(logs_for_cow.len(), 1);
        assert_eq!(logs_for_cow[0].notes, "n1");
    }

    /// Phase 6 §HLC drift test — a record stamped with a remote HLC
    /// far in the future does NOT silently win against local writes
    /// stamped with a small wallclock. The in-value HLC `(ts_ns,
    /// counter)` is the comparator; the store re-stamps each commit
    /// with `advance_hlc()`, which observes the on-disk max across
    /// replays and dominates remote skew.
    ///
    /// `Fms::open(data_dir, …)` stores records under
    /// `<data_dir>/fms/records.jsonl`, so round 1 has to write at
    /// the same nested path round 2 reads from.
    #[tokio::test]
    async fn hlc_drift_local_writes_dominate_after_replay() {
        use crate::store::{ReadStrategy, Store, StoreConfig};
        let dir = tempdir().unwrap();
        let fms_root = dir.path().join("fms");

        // Round 1: write with a *huge* HLC stamp, simulating a peer
        // whose wallclock is hours ahead. We go below the public
        // `Fms` API so we can pick our own HLC.
        let store = Store::open(StoreConfig {
            root: fms_root.clone(),
            event_buffer: 16,
        })
        .await
        .unwrap();
        let alien_hlc = Hlc::new(2_000_000_000_000_000_000, 0); // year ~2033
        let scope = "remote-peer".to_string();
        let mut tx = store.begin_transaction(scope.clone()).await;
        tx.put(
            b"asset/01HM000000000000000000000K/name".to_vec(),
            b"alien".to_vec(),
            alien_hlc,
            ReadStrategy::LastWriteWins,
        );
        tx.commit().await.unwrap();
        drop(store);

        // Round 2: re-open through the public `Fms` API. The
        // generator's `observe()` happens implicitly because
        // `Store::open` walks `records.jsonl` and tracks the max HLC
        // seen; the next `advance_hlc()` returns either
        // `(alien_hlc.ts_ns, alien_hlc.counter + 1)` or
        // `(now_wallclock_ns, 0)` — whichever is larger lexicographically.
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();

        let next = fms.debug_advance_hlc().await;
        assert!(
            next > alien_hlc,
            "advance_hlc must dominate replayed remote HLC; got {next:?} vs {alien_hlc:?}",
        );
    }

    /// Phase 6 §inventory derivation — a Weight log writes a
    /// `Quantity { measure: "weight", value: 425.0, unit: "kg" }`,
    /// and re-reading the log returns it intact. The wiki's
    /// "inventory is derived" rule (`fms-data-model` §"Inventory is
    /// *derived*") is upheld because the FMS layer doesn't maintain
    /// a separate inventory table — current state falls out of
    /// re-reading the relevant log set.
    #[tokio::test]
    async fn inventory_derivation_weight_log() {
        let dir = tempdir().unwrap();
        let fms = Fms::open(dir.path(), fixture_author()).await.unwrap();
        let cow = fms
            .create_asset(AssetKind::Animal, "Cow #99")
            .await
            .unwrap();

        let log = Log {
            id: Ulid::new(),
            kind: LogKind::Weight,
            timestamp: Hlc::new(1_700_000_000_000_000_000, 0),
            asset_refs: vec![cow],
            quantities: vec![Quantity {
                measure: "weight".into(),
                value: 425.0,
                unit: "kg".into(),
                label: None,
            }],
            photos: vec![],
            notes: "first weighing".into(),
        };
        fms.append_log(&log).await.unwrap();

        // Derive the most-recent weight by walking logs for the cow.
        let logs = fms.list_logs_for_asset(cow).await.unwrap();
        let latest_weight = logs
            .iter()
            .filter(|l| l.kind == LogKind::Weight)
            .max_by_key(|l| l.timestamp)
            .and_then(|l| l.quantities.iter().find(|q| q.measure == "weight"))
            .map(|q| (q.value, q.unit.clone()));
        assert_eq!(latest_weight, Some((425.0, "kg".into())));
    }
}
