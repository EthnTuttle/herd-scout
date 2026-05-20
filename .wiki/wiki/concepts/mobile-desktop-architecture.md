---
title: "Mobile↔Desktop offline-first architecture for farm apps"
tags: [architecture, offline-first, sync, crdt, tauri, iroh, rust, mobile-desktop]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# Mobile↔Desktop offline-first architecture

The user's "mobile to desktop" requirement = single app/data model that works on phone and desktop, offline in the field, syncs when peers meet. The proven path is local-first + CRDT + content-addressed media.

## Recommended stack for herd-scout

```
┌──────────────────────────────────────────────────────────┐
│  UI (Tauri 2 webview, or Compose Multiplatform fallback) │
├──────────────────────────────────────────────────────────┤
│  Rust app core (shared on phone + desktop)               │
│  ├─ Domain model (Asset / Log / Quantity / Term / Plan)  │
│  ├─ Local SQLite (rusqlite or sqlx) — indexable queries  │
│  ├─ iroh node                                            │
│  ├─ iroh-docs (KV CRDT, range reconciliation)            │
│  ├─ iroh-blobs (BLAKE3 photos / drone clips)             │
│  └─ iroh-gossip (live presence on farm Wi-Fi)            │
└──────────────────────────────────────────────────────────┘
```

Desktop is just another peer of the same iroh namespace — not a server.

## UI layer choices

| Stack | Mobile | Desktop | Rust-native | Verdict for herd-scout |
|---|---|---|---|---|
| **Tauri 2** | iOS+Android (alpha-tier rough edges) | Stable | **Yes** | **Primary pick** — keeps single Rust process |
| **Compose Multiplatform** | iOS+Android (stable; Wrike, Physics Wallah ship it) | Stable | No (Kotlin) | **Fallback** if Tauri 2 mobile blocks; call into Rust core via UniFFI |
| Flutter | Mature | Production-capable | No (Dart) | Doable, abandons Rust unification |
| Dioxus | Roughly Tauri-mobile tier | Solid | Yes | Smaller production footprint than Tauri |
| RN+Electron / Capacitor+Electron | — | — | No | Two UI codebases — rejected |

## Sync engines compared

| Engine | Model | Field-readiness | Fit for herd-scout |
|---|---|---|---|
| **iroh-docs** | KV CRDT, range-based reconciliation | Excellent (Paycode tolls in production) | **Pick** — already in repo (`vendor/iroh-live`) |
| Automerge | JSON CRDT, full doc semantics | Excellent (purpose-built for offline) | Strong but heavier; pick if rich-text or doc-shaped data |
| PowerSync | Server-mediated SQLite↔Postgres | Good for intermittent; **server is merge oracle** | Wrong shape — no central server in herd-scout vision |
| ElectricSQL | **Pivoted away from local-first in 2024** | Not the product anymore | **Avoid in 2026** — old tutorials are stale |
| RxDB / PouchDB+CouchDB | JS-first replication | Mature | Awkward in Rust/Tauri |
| Turso libSQL embedded replica | SQLite read-replica | Read-mostly offline | Writes route to primary — wrong shape for offline writes |

## The pattern (write order)

1. **Write to local first, always.** SQLite + iroh-docs. Never gate UI on network.
2. **Background sync task** observes the local replica and reconciles when peers reachable. Exponential backoff, resumable transfers.
3. **Append-only event log + materialized views.** Capture observations as immutable events (`animal_id`, `event`, `wallclock`, `device_id`); derive current state. Sidesteps most LWW conflicts.
4. **Content-addressed media via iroh-blobs.** Photos → BLAKE3; only the hash goes in the CRDT. Free dedup.
5. **iroh-gossip for live presence.** Two devices on the same farm Wi-Fi sync immediately, no polling.
6. **Desktop is a peer.** Same Rust binary, same iroh node, same namespace — no privileged data path.

## Anti-patterns

- "Online-only with offline cache" — if writes need a network round-trip, you don't have offline-first
- LWW with device wallclock — field devices have unsynced clocks; LWW silently picks wrong write. Use Lamport/HLC or CRDT causality
- Server-authoritative conflict resolution for a P2P-shaped problem — PowerSync/ElectricSQL assume a Postgres oracle
- Storing media inside the CRDT — bloats the doc, breaks range reconciliation efficiency
- Picking ElectricSQL based on stale 2023 articles — pivoted to AI-agent infra
- Two UI codebases when data model is the same (RN+Electron split)
- Picking Tauri 2 mobile without budgeting for alpha rough edges (TLS/OpenSSL cross-compile, Xcode device deploy)
- Skipping persistent storage — `Docs::persistent(path)` for production; in-memory is dev-only

## See also
- [[iroh-sync-stack]]
- [[fms-data-model]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-iroh-docs-blobs]]
- raw: [[2026-05-20-tauri-2-mobile]]
