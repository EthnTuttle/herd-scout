---
title: "iroh-smol-kv schema design for FMS data"
tags: [iroh, iroh-smol-kv, schema, kv-crdt, hlc, blake3, ulid, sqlite, projection]
created: 2026-05-20
updated: 2026-05-20
confidence: medium
type: concept
caveats: |
  iroh-smol-kv API surface is similar to but distinct from iroh-docs.
  Method names below approximate — verify against live iroh-smol-kv
  source on branch iroh-098 before locking code.
---

# iroh-smol-kv schema for FMS data

Concrete schema patterns for storing the [[fms-data-model]] (Asset / Log / Quantity / Term / Plan) on top of iroh-smol-kv (the KV CRDT this repo actually uses — see [[iroh-sync-stack]]).

## Key layout — entity-attribute, one scalar per key

Use ULIDs (sortable) so range scans give time-ordered iteration for free.

```
asset/<ulid>/kind                 → "animal" | "land" | "equipment" | ...
asset/<ulid>/name                 → "Cow #1234"
asset/<ulid>/geom                 → GeoJSON bytes (CBOR)
asset/<ulid>/parent               → "<parent-ulid>" or empty
asset/<ulid>/created_at           → HLC (u64 + node id)
asset/<ulid>/archived             → bool (soft delete)
asset/<ulid>/tag/<term-ulid>      → 1   (set membership; presence-only)

log/<ulid>/kind                   → "observation" | "medical" | ...
log/<ulid>/timestamp              → HLC
log/<ulid>/notes                  → text
log/<ulid>/asset_ref/<asset-ulid> → 1   (set membership — many-to-many)
log/<ulid>/quantity/<qid>/measure → "weight"
log/<ulid>/quantity/<qid>/value   → f64 LE bytes
log/<ulid>/quantity/<qid>/unit    → "kg"
log/<ulid>/quantity/<qid>/label   → "calf #1234"
log/<ulid>/photo/<seq>            → BLAKE3 hash (32 bytes)
log/<ulid>/photo/<seq>/mime       → "image/jpeg"

term/<ulid>/parent                → "<term-ulid>"
term/<ulid>/label/<lang>          → "Holstein"
term/<ulid>/vocab                 → "animal_type"
term/<ulid>/code/<scheme>         → "EPPO:..."

plan/<ulid>/kind                  → "grazing_rotation"
plan/<ulid>/step/<seq>/scheduled  → HLC
plan/<ulid>/step/<seq>/log_ref    → "<log-ulid>" (filled when realized)

user/<user-ulid>/device/<author-pubkey>  → device_label  (audit trail)
```

## Why one-scalar-per-key

- Per-field LWW means two authors editing different fields don't conflict
- A blob-per-entity (JSON-as-value) collapses every edit into one merge cell — much worse
- Set-membership (`/tag/<id> → 1`) gives many-to-many as add-wins sets, no contended JSON arrays

## Namespace design

**One namespace per farm/org. Not per user. Not per device.**

| Choice | Verdict |
|---|---|
| Per device | Fragments data; breaks reconciliation across devices |
| Per user | Doesn't model collaboration |
| **Per farm** | Right granularity. Plan ≥2 namespaces per device (farm + personal prefs) |
| Sub-farm by year | If farm grows >10⁶ entries: `farm-2026`, `farm-2027` |

## Author key strategy

**Per device, not per user.** Each install generates its own ed25519 author key.

- Reconciliation is keyed on `(namespace, author, key)` — sharing one author across two devices races for the same `(author,key)` cell, defeating per-device dimension and creating artificial conflicts
- Hardware loss/rotation cleaner: revoke device key, issue new one; old data still attributable
- "Who edited X?" → maintain `user/<user-ulid>/device/<author-pubkey>` registry in same namespace

## Conflict resolution — three strategies, choose per field

Range reconciliation is **not** a merge function. After sync, both peers hold the union of all `(namespace, author, key, ts, hash)` records. Reading `key=K` returns potentially many entries.

1. **LWW on (HLC ts, author_id)** — fine for benign last-write fields (`name`, `notes`). HLC mandatory; never wallclock (devices drift hours in field).
2. **Add-wins set** — presence-only keys. Tombstones for removal: `…/_deleted=<hlc>`; on read, drop if any deletion timestamp ≥ all add timestamps.
3. **Append-only with derived state** — quantities, movements. `log/<ulid>` is never mutated after creation. Current state is a fold over logs. **farmOS pattern; LWW conflicts simply don't apply.**

LWW is **unacceptable** for: quantities (silent stock loss), medical withdrawal periods (safety), movement events (animal in two paddocks). Make these append-only.

## Large value / blob references

**iroh-smol-kv stores BLAKE3 hashes; iroh-blobs stores bytes.**

Photo lifecycle:
1. `blobs.add_bytes(photo_bytes)` → `Hash` (32 bytes)
2. Write `log/<id>/photo/0001 → <hash>` plus `…/mime` and `…/size`
3. Sync: doc replicates 32-byte hash on cheap connections; blob downloads lazily/opportunistically
4. Five photos → `…/photo/0001` … `…/0005` (sortable, sparse, additive)
5. Delete: write `…/photo/0001/_deleted=<hlc>`; GC blob locally only after all peers observed tombstone (or never — BLAKE3 dedup is cheap)

**Never embed bytes in the doc** — pessimizes range reconciliation; loses blob-level resumable transfer.

## Schema evolution

Three rules:
1. **New fields = new keys** (no-op for old replicas)
2. **Never repurpose a key** — new semantics → new key, migrate
3. **Schema version per entity**: `asset/<id>/_schema=2`. Reader does on-the-fly upgrade in memory; writer writes back at `_schema=N` only when user touches the record. Lazy migration, no flag day.

## Indexing — SQLite projection

iroh-smol-kv is prefix-scan only. Pattern:

- **iroh-smol-kv = source of truth** (immutable system of record)
- **Local SQLite = projection** (rebuildable at any time)

Sync via doc subscription:

```rust
let mut events = doc.subscribe().await?;
while let Some(ev) = events.next().await {
    match ev {
        Event::InsertRemote { entry, .. } | Event::InsertLocal { entry, .. } => {
            apply_to_sqlite(&entry).await?;
            checkpoint.save(entry.timestamp()).await?;
        }
        Event::ContentReady { hash } => mark_blob_available(hash).await?,
        _ => {}
    }
}
```

On startup, replay from last checkpoint. SQLite lost or schema changes → drop and replay (projection is pure derivation). UI writes go to iroh-smol-kv, projector updates SQLite, UI reactively re-reads.

SQLite indexes: FTS5 for free-text on log notes, R-Tree for `asset.geom`, ordinary indexes for time/asset filters.

## Gotchas

- **In-memory `Docs` in production drops everything on restart** — use persistent backend
- **Wallclock LWW silently corrupts data** — HLC mandatory
- **Tombstones are forever** — no GC across replicas without coordination protocol; budget storage (typically tiny)
- **Author key compromise = full namespace write capability** — treat like SSH keys
- **Range reconciliation cost is per-entry** — 10M small keys heavier than 100k medium. Don't decompose so finely you create millions of trivia keys.
- **iroh-smol-kv API differs from iroh-docs** — verify method names before code commits
- **No transactions across keys** — two related writes observed independently. Projector tolerates partial entities; treat creation as "all required fields present" gate.
- **Namespace key distribution is your problem** — iroh-tickets for invite codes; design onboarding (QR code on desktop scanned by phone) up front
- **Gossip ≠ delivery guarantee** — best-effort live notification; reconciliation is truth

## Code sketch (illustrative — verify API names)

```rust
use iroh_smol_kv::{Doc, AuthorId};
use iroh_blobs::Hash;
use ulid::Ulid;

struct Fms { doc: Doc, author: AuthorId, blobs: BlobsClient }

impl Fms {
    async fn create_asset(&self, kind: &str, name: &str) -> anyhow::Result<Ulid> {
        let id = Ulid::new();
        let prefix = format!("asset/{id}");
        self.doc.set_bytes(self.author, format!("{prefix}/kind"), kind.into()).await?;
        self.doc.set_bytes(self.author, format!("{prefix}/name"), name.into()).await?;
        self.doc.set_bytes(self.author, format!("{prefix}/_schema"), b"1".into()).await?;
        Ok(id)
    }

    async fn add_log_photo(&self, log_id: Ulid, seq: u32, bytes: Vec<u8>) -> anyhow::Result<()> {
        let hash: Hash = self.blobs.add_bytes(bytes).await?.hash;
        let key = format!("log/{log_id}/photo/{seq:04}");
        self.doc.set_bytes(self.author, key, hash.as_bytes().to_vec()).await?;
        Ok(())
    }

    async fn get_asset_name(&self, id: Ulid) -> anyhow::Result<Option<String>> {
        let key = format!("asset/{id}/name");
        let entries = self.doc.get_many(prefix_eq(&key)).await?;
        Ok(entries.into_iter()
            .max_by_key(|e| (e.timestamp(), e.author()))
            .map(|e| String::from_utf8_lossy(e.content()).into()))
    }
}
```

## See also
- [[iroh-sync-stack]]
- [[fms-data-model]]
- [[mobile-desktop-architecture]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-iroh-docs-fms-schema]]
- raw: [[2026-05-20-iroh-docs-blobs]]
