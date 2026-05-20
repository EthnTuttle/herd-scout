---
title: "FMS data model — Asset / Log / Quantity / Term / Plan"
tags: [fms, data-model, farmos, schema, architecture]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# FMS data model — five primitives

farmOS pioneered a five-primitive model for farm record-keeping that maps cleanly to nearly every FMS feature. Adopt this design directly — or stay API-compatible — to inherit a decade of learning and ease data portability.

## The five primitives

### 1. Asset
A thing on the farm. Asset types in farmOS:

- **Land** — fields, paddocks, beds, ranges
- **Animal** — individual or grouped livestock
- **Plant** — crops, trees, perennials
- **Equipment** — tractors, implements, vehicles
- **Group** — container holding other assets (a herd is a Group of Animals)
- **Sensor** — IoT devices
- **Structure** — barns, silos, fences, gates
- **Material** — generic input/output (feed, fertilizer)
- **Product** — saleable output
- **Water** — water bodies, tanks
- **Compost** — compost piles

Assets have geometry (point, line, polygon). Hierarchy via parent/child references — a paddock is a child of a field, a calf is a child of a cow.

### 2. Log
A time-stamped event. Log types:

- **Activity** — generic
- **Observation** — what was seen/measured
- **Input** — what was applied (with lot, method, source)
- **Harvest** — what was taken
- **Lab Test** — soil, tissue, milk results
- **Maintenance** — equipment service
- **Medical** — vet treatment with withdrawal periods
- **Seeding / Transplanting**
- **Birth, Weaning, Movement, Purchase, Sale**

Logs reference assets (which asset(s) this log concerns) and carry quantities.

### 3. Quantity
Numeric measurement attached to a log: `{measure, value, unit, label}`. Examples: `{weight, 425, kg, "calf #1234"}`, `{volume, 200, liters, "feed bin"}`.

**Inventory is *derived*** — sum of increments minus decrements per `(asset, measure, unit)` since the last reset log. **Don't build a separate inventory table.**

### 4. Term
Taxonomy vocabulary. Crop varieties, animal types, units, log categories, materials. Hierarchical, multilingual, mappable to standards (EPPO codes, AGROVOC) — see [[ag-data-standards]].

### 5. Plan
A sequence of expected/scheduled activities. Crop plan, breeding plan, grazing rotation. Plans generate logs.

## Why this model wins

- **Extensible without schema changes**: a new feature usually maps to a new asset type, log type, or quantity — not a new table.
- **Inventory is free**: derive it; never desync.
- **Time-series is free**: every log is timestamped; queries fall out naturally.
- **Multi-tenancy is free**: assets and logs scope to a farm/owner.
- **Compliance is free**: medical logs with withdrawal flags, input application logs with rate/applicator/weather — all just logs.

## Cross-platform / sync implications

For a [[mobile-desktop-architecture]]-style offline-first app on iroh-docs:

- One iroh-docs namespace per farm
- Each asset = one set of keys (e.g., `asset/<id>/name`, `asset/<id>/geom`)
- Each log = one set of keys (e.g., `log/<id>/timestamp`, `log/<id>/type`, `log/<id>/asset_refs`, `log/<id>/quantities`)
- BLAKE3 hashes (in iroh-blobs) for media stored in `log/<id>/photos`
- Range-based reconciliation handles arbitrary offline durations — no log replay needed

## Recommendation for herd-scout

Adopt the Asset+Log+Quantity+Term+Plan model directly. Stay JSON:API compatible with farmOS so users can import/export. Native Rust types map cleanly:

```rust
struct Asset { id, kind, name, geom, parent, ... }
struct Log { id, kind, timestamp, asset_refs: Vec<AssetRef>, quantities: Vec<Quantity>, ... }
struct Quantity { measure, value, unit, label }
```

## See also
- [[oss-fms-landscape]]
- [[fms-feature-taxonomy]]
- [[mobile-desktop-architecture]]
- [[ag-data-standards]]

## Sources
- raw: [[2026-05-20-farmos]]
- raw: [[2026-05-20-fms-feature-taxonomy]]
