---
title: "iroh-docs / iroh-smol-kv schema patterns for FMS data"
source_url: https://github.com/n0-computer/iroh-smol-kv
secondary_urls:
  - https://docs.iroh.computer
  - https://github.com/n0-computer/iroh
type: synthesis
tags: [iroh, iroh-smol-kv, schema, kv-crdt, hlc, blake3, ulid, sqlite-projection]
created: 2026-05-20
confidence: medium
caveats: |
  Repo's actual dep is iroh-smol-kv (n0-computer/iroh-smol-kv branch iroh-098),
  not the older iroh-docs crate. Method names below approximate — verify
  against live iroh-smol-kv source before locking API.
---

# iroh-docs / iroh-smol-kv schema patterns for FMS data

## Important correction

The herd-scout `Cargo.toml` declares:

```toml
iroh-smol-kv = { git = "https://github.com/n0-computer/iroh-smol-kv", branch = "iroh-098", default-features = false }
```

**Not** the older `iroh-docs` crate. iroh-smol-kv is a leaner fork/rewrite implementing the same Meyer 2022 range-based set reconciliation primitive. API surface is similar but **not identical** — verify exact method names against the live README before locking code.

## Recommended key layout (entity-attribute, one scalar per key)

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
log/<ulid>/quantity/<qid>/value   → f64 LE bytes (or stringified decimal)
log/<ulid>/quantity/<qid>/unit    → "kg"
log/<ulid>/quantity/<qid>/label   → "calf #1234"
log/<ulid>/photo/<seq>            → BLAKE3 hash (32 bytes)
log/<ulid>/photo/<seq>/mime       → "image/jpeg"
log/<ulid>/photo/<seq>/size       → u64

term/<ulid>/parent                → "<term-ulid>"
term/<ulid>/label/<lang>          → "Holstein"
term/<ulid>/vocab                 → "animal_type"
term/<ulid>/code/<scheme>         → "EPPO:..."

plan/<ulid>/kind                  → "grazing_rotation"
plan/<ulid>/name                  → "Spring 2026"
plan/<ulid>/step/<seq>/scheduled  → HLC
plan/<ulid>/step/<seq>/log_ref    → "<log-ulid>" (filled when realized)

user/<user-ulid>/device/<author-pubkey>  → device_label  (audit trail)
```

**Why one-scalar-per-key**: per-field LWW means two authors editing different fields don't conflict. A blob-per-entity (JSON-as-value) collapses every edit into one merge cell. Set-membership keys (`/tag/<id> → 1`) give many-to-many as add-wins sets, no contended JSON arrays.

## Namespace design

**One namespace per farm/org. Not per user. Not per device.**

- *Per device*: fragments data, breaks reconciliation across devices
- *Per user*: doesn't model collaboration (two field hands on one ranch)
- *Per farm*: right granularity. Plan for ≥2 namespaces per device (farm + personal prefs)
- *Sub-farm partitioning*: if farm grows huge (>10⁶ entries) split by year (`farm-2026`, `farm-2027`)

## Author key strategy

**Per device, not per user.** Each install generates its own ed25519 author key.

- Reconciliation is keyed on `(namespace, author, key)` — two devices sharing one author would race for the same `(author,key)` cell, defeating per-device dimension and creating artificial conflicts
- Hardware loss/rotation is cleaner: revoke device key, issue new one
- "Who edited X?" → maintain `user/<user-ulid>/device/<author-pubkey>` registry in the same namespace

## Conflict patterns

Range reconciliation is **not** a merge function — it tells two peers which `(namespace, author, key, timestamp, hash)` records each is missing. After sync, both hold the union. Reading `key=K` returns potentially many entries (one per author who ever wrote it).

Three resolution strategies, choose per field:

1. **LWW on (HLC timestamp, author_id) tiebreak** — fine for benign last-write fields (`asset/<id>/name`, `asset/<id>/notes`). **Hybrid Logical Clock** mandatory; never use device wallclock.
2. **Add-wins set** — presence-only keys. Tombstones for removal: `asset/<id>/tag/<term-id>/_deleted=<hlc>`; on read, drop tag if any deletion timestamp ≥ all add timestamps.
3. **Append-only with derived state** — quantities, movement events. `log/<ulid>` is never mutated after creation; current state is a fold. **This is the farmOS pattern; LWW conflicts simply don't apply.**

LWW is **unacceptable** for: quantities (silent stock loss), medical withdrawal periods (safety), movement events (animal in two paddocks). Make these append-only logs.

## Large value / blob references

Pattern: **iroh-docs stores BLAKE3 hashes; iroh-blobs stores the bytes.**

Photo lifecycle:
1. `blobs.add_bytes(photo_bytes).await` → `Hash` (32 bytes BLAKE3)
2. Write `log/<log-ulid>/photo/0001 → <hash>` plus `…/mime` and `…/size`
3. Sync: doc replicates 32-byte hash on cheap connections; blob downloads lazily/opportunistically (resumable)
4. Five photos → `…/photo/0001` … `…/0005` (sortable, sparse, additive)
5. Delete: write `…/photo/0001/_deleted=<hlc>`. GC blob locally only after all peers observed tombstone (or never — BLAKE3 dedup is cheap)

Never embed bytes in the doc — pessimizes range reconciliation and loses blob-level resumable transfer.

## Schema evolution

Three rules:
1. **New fields = new keys**. Adding `asset/<id>/breed` is a no-op for old replicas.
2. **Never repurpose a key**. New semantics → new key. Migrate.
3. **Schema version per entity**: `asset/<id>/_schema=2`. Reader does on-the-fly upgrade in memory; writer writes back at `_schema=N` only when user touches the record. Lazy migration, no flag day.

Renaming/removing requires write-side compatibility shims for ~one release.

## Indexing for queries

iroh-docs is prefix-scan only. Keep iroh-docs as source-of-truth, project into local SQLite for queries (FTS5 free-text, R-Tree for geom, ordinary indexes for time/asset filters).

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

On startup, replay from last checkpoint. If SQLite lost or schema changes, drop and replay — projection is pure derivation. UI writes go to iroh-docs; projector updates SQLite; UI reactively re-reads.

## Gotchas

- **In-memory `Docs` in production drops everything on restart** — use persistent backend
- **Wallclock LWW silently corrupts data** — devices drift hours in field. HLC mandatory.
- **Tombstones are forever** — no GC across replicas without coordination protocol; budget storage (typically tiny)
- **Author key compromise = full namespace write capability** — treat like SSH keys
- **Range reconciliation cost is per-entry** — 10M small keys heavier than 100k medium. Don't decompose so finely you create millions of trivia keys.
- **iroh-smol-kv API differs from iroh-docs** — verify method names before code commits
- **No transactions across keys** — two related writes are observed independently. Projector must tolerate partial entities; treat creation as "all required fields present" gate.
- **Namespace key distribution is your problem** — iroh-tickets handles invite codes; design onboarding (QR code on desktop scanned by phone) up front
- **Gossip ≠ delivery guarantee** — best-effort live notification; reconciliation is truth. Don't write logic assuming gossip arrives.

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
