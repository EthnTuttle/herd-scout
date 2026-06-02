---
title: "Feature: iroh-smol-kv FMS schema crate"
type: feature-candidate
priority: p0
created: 2026-06-02
source: assess-herd-scout-2026-06-02
status: open
wiki_evidence:
  - concepts/iroh-docs-fms-schema
  - concepts/fms-data-model
  - concepts/iroh-sync-stack
---

# Feature: iroh-smol-kv FMS schema crate

## Why P0

The repo declares `iroh-smol-kv` as the planned data plane (per `Cargo.toml` and the playbook), but no records are actually being written to it. Every other wiki wedge — Animal records, EID reconciliation, paddock polygons, farmOS-compat export, treatment logs — needs this layer first. Without it, the existing video / CV / audit infrastructure has nowhere to write structured data.

## Scope (~2-4 weeks)

Implement the concrete schema from [[concepts/iroh-docs-fms-schema]]:

- One namespace per farm/org (not per user, not per device)
- Per-device author keys (so reconciliation doesn't race for the same `(author,key)` cell)
- One scalar per key (`asset/<id>/name`, `asset/<id>/geom`) — per-field LWW vs collapsing to JSON-blob
- HLC timestamps, never device wallclock
- Append-only logs for things where LWW is unsafe (quantities, medical, movement)
- iroh-blobs hash-refs for photos
- SQLite projection for queries (FTS5, R-Tree); iroh-docs is source-of-truth

## Open questions

- Is iroh-smol-kv API stable enough at iroh 1.0-rc.1? (Wiki notes API "similar but not identical" to iroh-docs — verify against live source)
- Where does the projection live — in the daemon, in the GUI, or both?
- Auth: which (namespace, author, key) cells does the admin app's NodeId have write access to?

## See also
- [[../../wiki/concepts/iroh-docs-fms-schema]]
- [[../../wiki/concepts/fms-data-model]]
- [[../../wiki/concepts/iroh-sync-stack]]
- [[../../output/assess-herd-scout-2026-06-02]] §Opportunities
