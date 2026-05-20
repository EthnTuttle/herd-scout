---
title: "iroh sync stack — iroh-docs + iroh-blobs + iroh-gossip"
tags: [iroh, p2p, crdt, blake3, sync, rust, n0-computer]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# iroh sync stack

iroh is already in this repo via `vendor/iroh-live` (workspace deps in `Cargo.toml`). For herd-scout it provides the data plane that makes phone↔desktop offline-first work without a central server.

> **Important correction**: this repo declares `iroh-smol-kv` (n0-computer/iroh-smol-kv branch `iroh-098`), not the older `iroh-docs` crate. iroh-smol-kv is a leaner fork/rewrite implementing the same Meyer 2022 range-based set reconciliation primitive. API surface is similar but **not identical** — verify exact method names against the live iroh-smol-kv source before locking code.

## Components

### iroh (transport)
- Dial-by-public-key
- Hole-punching with automatic relay fallback
- Phone↔desktop on farm Wi-Fi → peer-direct
- Phone↔desktop across internet → relay fallback

### iroh-docs (KV CRDT)
- Range-based set reconciliation (Meyer 2022 — recursive partition + fingerprint comparison)
- Keyed by `(namespace, author, key)` triplets
- Eventual consistency, multi-author writes
- `Docs::persistent(path)` for production; in-memory mode is dev-only
- **Production case**: Paycode highway toll terminals
- **Why field-ready**: range reconciliation handles arbitrary offline durations because peers diff sets when they meet — no log replay required for catching up

### iroh-blobs
- BLAKE3 content-addressed storage
- Resumable, kB→TB scale
- Ideal for pasture/cattle photos and drone clips
- **Don't put media in the CRDT** — store the BLAKE3 hash in iroh-docs, the bytes in iroh-blobs

### iroh-gossip
- Live pub/sub overlay
- Peers learn of new entries without polling
- Useful when two devices are on the same farm Wi-Fi

### iroh-ffi
- FFI bindings for non-Rust callers
- Useful if/when Compose Multiplatform or Flutter wraps the Rust core

## Why this combination

It's essentially the n0-computer answer to "Automerge + sync server + blob store" — but with no server. Tradeoff vs Automerge: KV CRDT (not JSON CRDT). Simpler reasoning; you give up rich-document semantics for a free P2P transport.

## How herd-scout would use it

For the concrete schema design (key layout, namespace strategy, conflict resolution, blob refs, schema evolution), see [[iroh-docs-fms-schema]]. Quick summary:

- **One namespace per farm/org** (not per user, not per device)
- **Per-device author keys** (so reconciliation doesn't race for the same `(author,key)` cell)
- **One scalar per key** (`asset/<id>/name`, `asset/<id>/geom`) — per-field LWW vs collapsing to JSON-blob
- **HLC timestamps**, never device wallclock
- **Append-only logs** for things where LWW is unsafe (quantities, medical, movement)
- **iroh-blobs for photos** — store BLAKE3 hash in iroh-docs, bytes in iroh-blobs
- **SQLite projection** for queries (FTS5, R-Tree); iroh-docs is source-of-truth
- iroh-gossip for "your buddy just added a calving log" notifications on farm Wi-Fi

## See also
- [[iroh-docs-fms-schema]] — concrete key layout + code sketch
- [[mobile-desktop-architecture]]
- [[fms-data-model]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-iroh-docs-blobs]]
- raw: [[2026-05-20-iroh-docs-fms-schema]]
