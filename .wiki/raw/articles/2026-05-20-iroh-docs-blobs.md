---
title: "iroh-docs + iroh-blobs + iroh-gossip — Rust-native P2P sync stack"
source_url: https://docs.iroh.computer
secondary_urls:
  - https://github.com/n0-computer/iroh
  - https://docs.iroh.computer/protocols/kv-crdts
type: project
tags: [iroh, p2p, crdt, blake3, sync, offline-first, rust, oss]
created: 2026-05-20
confidence: high
---

# iroh stack for offline-first farm app sync

- License: Apache-2.0 / MIT
- Stack: Rust
- Maintainer: n0-computer
- Already in use in this repo (vendor/iroh-live workspace)

## Components

### iroh (transport)
- Dial-by-public-key
- Hole-punching with automatic relay fallback
- Phone↔desktop on farm Wi-Fi → peer-direct
- Phone↔desktop across internet → relay fallback

### iroh-docs (KV CRDT)
- Range-based set reconciliation (Meyer 2022)
- Keyed by `(namespace, author, key)` triplets
- Recursive partition + fingerprint comparison
- Eventual consistency, multi-author
- Persistent storage via `Docs::persistent(path)`
- **Production case**: Paycode highway toll terminals — handles arbitrary offline durations because peers diff sets when they meet (no log replay required for catching up)

### iroh-blobs
- BLAKE3 content-addressed storage
- Resumable, kB→TB scale
- Ideal for photos/videos/large attachments
- Avoids bloating CRDT — store only the hash

### iroh-gossip
- Live pub/sub overlay
- Peers learn of new entries without polling
- Useful for live presence on farm Wi-Fi

### iroh-ffi
- FFI bindings for non-Rust callers
- Useful if Compose Multiplatform / Flutter UI is added later

## Why this fits offline-first farm work

- 30-day offline tolerance (range reconciliation, no log replay)
- Multi-author writes (multiple field hands editing concurrently)
- Built-in transport (no separate server needed)
- Content-addressed media (free dedup of pasture photos / drone clips)
- "Desktop is just another peer" — same Rust binary, same iroh node, same namespace

## Anti-patterns to avoid

- Storing media inside the CRDT (use iroh-blobs)
- LWW timestamps from unsynced device clocks (use Lamport/HLC or CRDT causality)
- In-memory `Docs` for production (use `Docs::persistent(path)`)
- Server-authoritative conflict resolution (defeats the P2P shape)

## Relevance to herd-scout

iroh-docs + iroh-blobs + iroh-gossip is essentially the n0-computer answer to "Automerge + a sync server + a blob store." It is a KV CRDT (not JSON CRDT like Automerge) — simpler reasoning, free P2P transport. Already in the repo via `vendor/iroh-live` — natural foundation for a herd-scout offline-first FMS.
