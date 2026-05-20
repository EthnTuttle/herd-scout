---
title: "herd-scout playbook — OSS livestock-focused FMS with drone vision"
type: playbook
created: 2026-05-20
updated: 2026-05-20
session: 2026-05-20-132147
session_duration: ~28min
rounds: 2
sources: 17
articles_created: 14
---

# herd-scout playbook

Synthesis of two research rounds answering: *"mobile-to-desktop OSS farm management — what features? what exists? any drone integrations or can we strap an android onto a drone?"*

## TL;DR

- **What exists**: farmOS dominates OSS farm management; LiteFarm + Ekylibre fill out the big-three. Livestock-specific OSS is shockingly thin; drone integration into FMS is broken at the ingestion layer; no Rust competitor exists; offline-first native mobile is unfilled.
- **Features needed**: 10-bucket taxonomy (records / logs / inventory / financial / compliance / spatial / sensor / drone / reporting / mobile). farmOS's Asset+Log+Quantity+Term+Plan model is the right design — adopt or stay JSON:API compatible.
- **Drone integrations**: OSS pipeline L1-L4 is mature (QGC + ODM + GeoTIFF). L5 (FMS structured ingest) is broken — orthos and NDVI never become queryable per-field attributes. Concrete bridge pattern documented in [[webodm-fms-bridge]].
- **Strap an Android onto a drone**: don't make it the flight controller. Do make it the (a) onboard ML/CV camera + (b) 4G BVLOS bridge talking MAVLink to a real Pixhawk. DroneKit-Android is dead — fork it or use `io.dronefleet.mavlink`.

## What exists — OSS farm management

### Big three (active)

| Project | License | Stack | Mobile | Strength |
|---|---|---|---|---|
| **farmOS** | GPL-2.0 | PHP/Drupal 10 | Field Kit PWA | Dominant; canonical [[fms-data-model]]; 1.3k stars |
| **LiteFarm** | GPL-3.0 | Node + React PWA | Mobile-first PWA | Sustainability/cert focus; modern stack |
| **Ekylibre** | AGPL-3.0 | Ruby on Rails | Responsive web | Heavyweight ERP+accounting (EU) |

### The long tail (mostly faltering)

Tania (abandoned 2019), OpenFarm (archived 2025), OpenATK (dormant), Livestock Tracker / AVAT (toy scale). BEEP dominates beekeeping; OpenWeedLocator and FruitNeRF exist for crop-side CV but aren't FMS.

### Livestock-specific (thin)

- **farmOS** Animal asset type
- **BovHEAT** — dairy estrus from SCR Heatime accelerometer XLSX (single sensor, ~20 stars)
- **HerdNet** — aerial livestock counting model (MIT, 73-83% F1, ~57 stars)
- **AVAT** — livestock video annotation (toy)
- **Piquetear** — rotational grazing (abandoned 2023)

**Critical gap**: zero OSS repos parse ISO 11784/11785 EID tags or talk to common stick readers (Allflex/Tru-Test/Datamars/Gallagher).

## Features — what an FMS needs

Ten buckets, with must / nice / advanced tiers. Full table in [[fms-feature-taxonomy]].

**MVP slice for herd-scout** (livestock-focused):
1. **Records**: Animal, Group (herd/mob), Land (paddock), Equipment
2. **Logs**: Observation, Medical (with withdrawal flag), Movement, Weight, Birth
3. **Mobile-first**: offline scouting, geotagged photo, EID scan lookup
4. **Mapping**: paddock polygons (GeoJSON), animal-last-seen pins
5. **Compliance**: treatment log + animal-ID export

**Defer**: financial rollups (export to QuickBooks initially), grain marketing, full inventory, ortho ingestion, full IoT.

## Mobile↔desktop architecture

### Recommended stack

```
┌──────────────────────────────────────────────────────────┐
│  UI: Tauri 2 webview (or Compose Multiplatform fallback) │
├──────────────────────────────────────────────────────────┤
│  Rust app core (shared phone + desktop)                  │
│  ├─ Domain model (Asset / Log / Quantity / Term / Plan)  │
│  ├─ Local SQLite (rusqlite/sqlx) — indexable queries     │
│  ├─ iroh node                                            │
│  ├─ iroh-smol-kv (KV CRDT, range reconciliation)         │
│  ├─ iroh-blobs (BLAKE3 photos / drone clips)             │
│  └─ iroh-gossip (live presence on farm Wi-Fi)            │
└──────────────────────────────────────────────────────────┘
```

Desktop is just another peer of the same iroh namespace — not a server.

### Sync engine — pick iroh-smol-kv

The repo's `Cargo.toml` already declares `iroh-smol-kv = { git = "...", branch = "iroh-098" }`. It's a leaner implementation of the same Meyer 2022 range-based set reconciliation as iroh-docs. Trade-off vs Automerge: KV CRDT (not JSON CRDT) — simpler reasoning, free P2P transport.

**Avoid**: ElectricSQL (pivoted away from local-first in 2024), PowerSync (assumes Postgres oracle, wrong shape), LWW with device wallclocks.

### Schema (concrete)

See [[iroh-docs-fms-schema]]. Highlights:

- One namespace per farm (not per device, not per user)
- Per-device ed25519 author keys (so reconciliation doesn't race)
- One scalar per key: `asset/<ulid>/name`, `asset/<ulid>/geom`, `log/<ulid>/quantity/<qid>/value`
- HLC timestamps; never wallclock
- Append-only logs for safety-critical fields (quantities, medical, movement)
- BLAKE3 hash in iroh-smol-kv, bytes in iroh-blobs
- SQLite projection for queries (FTS5, R-Tree, time/asset indexes)

## Drone integrations

### What works in OSS (L1-L4)

- **L1 Capture**: QGroundControl, Mission Planner — survey grid, KML export, MAVLink
- **L2 Stitching**: ODM/WebODM/NodeODM/ClusterODM — orthomosaic, DSM, point cloud (BSD)
- **L3 Multispectral**: ODM with `--radiometric-calibration camera+sun` + micasense/imageprocessing
- **L4 Storage/GIS**: GeoTIFF/COG, GeoPackage, PostGIS, QGIS, GeoServer, **DroneDB** (MPL-2.0)

### What's broken — L5 ingestion

Orthos and NDVI rasters do not become structured per-field attributes inside any mainstream OSS FMS. farmOS-map supports WMS/XYZ/GeoJSON but NOT GeoTIFF/COG natively. Manual workflow today: ODM → GeoTIFF → GeoServer → WMS layer in farmOS. Lossy and tedious.

**Concrete fix** ([[webodm-fms-bridge]]): NodeODM webhook on task complete → `gdal_calc` for NDVI → `rasterstats` for per-paddock zonal stats (mean, std, p10/p90, pct_stressed) → write farmOS Observation log with quantities. ~80 LOC Python; same pattern in Rust with `gdal` crate + custom zonal stats over `gdal::Dataset`.

### What's research-grade — L6 real-time

Live RTSP/RTMP from drone with on-board YOLO works (drone-vision-software), but no OSS does live NDVI in flight (radiometric calibration needs panel image at takeoff and pose-corrected stitching).

### Adjacent killer features

| Use case | Drone HW | Maturity | Priority |
|---|---|---|---|
| Drone count → reconcile against EID inventory | RGB | Greenfield | **Top** — no OSS does this |
| Lost-livestock thermal search | Thermal | Hand-rolled | High — same users, same hardware |
| Body condition scoring | RGB high-res | Research | High — no public dataset (defensible) |
| Pasture NDVI / forage biomass | Multispectral | Production toolchain | High — eats AgriWebb's lunch |
| Post-storm fence/structure damage | RGB | Production for capture | Medium |

## Strap an Android onto a drone

### Verdict

| Role | Verdict |
|---|---|
| Phone as **flight controller** (PID @ 400 Hz–1 kHz) | **NO** — Android scheduling jitter too high |
| Phone as standalone mission computer driving "dumb" drone | Mediocre — builds on dead libraries |
| Phone as **(a) onboard ML/CV camera + (b) 4G BVLOS bridge** talking MAVLink to a real Pixhawk/PX4 | **GOOD** — FlytBase architecture, DIY at ~$200 |

### Phone vs Pi 5 vs Jetson

- Pixel 4 CPU-only TFLite: MobileNet v1 224 quantized = 5 ms (~200 FPS theoretical)
- MobileNet SSD on Snapdragon 8xx with NNAPI/GPU: 15-40 ms (25-60 FPS) — comparable to or faster than Jetson Nano
- Pi 5 + 4G hat (~$150) better-trodden software path (MAVSDK/ROS2); slower NPU
- Used Pixel 6a/7a: cheaper for the compute; includes battery + camera + GPS + LTE + display; fragmented software stack

### Connection options

| Connection | Use |
|---|---|
| USB-OTG → Pixhawk | High bandwidth MAVLink + telemetry |
| BLE/SPP → HC-05 → autopilot UART | Low-rate (1-2 Hz) |
| **WiFi → ESP8266/ESP32 MAVLink bridge** | **Most popular DIY**; UDP MAVLink |
| Built-in 4G LTE | BVLOS supervisory backhaul |

### Critical software finding

**DroneKit-Android is dead** (last release Oct 2016). Use `io.dronefleet.mavlink` (Java, more actively maintained), MAVSDK Android, or roll-your-own (MAVLink wire protocol is documented; few hundred lines for focused use case).

### Why this fits herd-scout

- iroh-live (already in repo) handles phone↔desktop video stream
- HerdNet or fine-tuned variant runs inference on phone (low latency) or desktop (richer batch passes)
- Counts written as Observation logs to iroh-smol-kv; sync to all peers
- BVLOS via phone 4G enables ranches larger than visual line-of-sight without enterprise drone hardware

## Defensible wedges (where OSS is thin)

1. **Rust-native FMS** — entire OSS FMS space is PHP/Ruby/JS/Go
2. **P2P / offline-first / no central server** — farmOS, LiteFarm, Ekylibre all assume a server; iroh-docs is novel
3. **Drone-vision integrated livestock workflows** — HerdNet counts; nothing closes the loop to herd records
4. **Native mobile ranch UX** — every major OSS FMS punts to PWA
5. **OSS EID reader bridge** — zero OSS repos exist; concrete weekend MVP scoped in [[livestock-eid-rfid]]

## What to build first (3 phases)

### Phase 0 — The crate that ships in a weekend

`herd-scout-eid` — Rust library reading 15-digit ISO 11784 codes from Bluetooth SPP / BLE / USB-CDC stick readers. Three transports (`serialport` / `bluer` / `btleplug`); line-buffered ASCII parser; `EidTag { country, national_id, protocol, raw }`; demo CLI; unit tests with synthesized vendor lines. Covers ~70-80% of deployed sticks. **Independently useful**: any farmOS user with an Allflex stick reader becomes a potential adopter.

### Phase 1 — MVP herd-scout

- Tauri 2 + Rust core (single binary mobile + desktop)
- iroh-smol-kv schema per [[iroh-docs-fms-schema]]
- SQLite projection for queries
- Animal / Group / Land / Equipment assets
- Observation / Medical / Movement / Weight / Birth logs
- Offline-first scouting: geotagged photo + EID scan lookup
- farmOS JSON:API import/export so users can migrate
- Phone↔desktop live presence via iroh-gossip

### Phase 2 — Drone hook

- Reuse existing `vendor/iroh-live` for phone-on-drone video stream
- HerdNet (or fine-tuned) inference on phone or desktop
- Counts → Observation logs
- WebODM webhook → zonal-stats bridge per [[webodm-fms-bridge]]; Observation logs with mean/stddev NDVI per paddock
- Lost-livestock thermal flight pattern with thermal drone integration as research roadmap

## What to consume, not build

- farmOS data model + JSON:API ([[fms-data-model]])
- ODM / WebODM / NodeODM ([[opendronemap]])
- micasense/imageprocessing — radiometric calibration
- DroneDB — drone output hosting
- HerdNet — aerial livestock detection model ([[herdnet]])
- iroh / iroh-smol-kv / iroh-blobs / iroh-gossip ([[iroh-sync-stack]])
- Tauri 2 — single Rust process across phone+desktop ([[mobile-desktop-architecture]])
- MAVLink wire protocol + `io.dronefleet.mavlink` (if doing phone-on-drone)

## What to skip

- DroneKit-Android (dead)
- ElectricSQL local-first (pivoted 2024)
- Closed SaaS competition (DroneDeploy / AgriWebb / PastureMap)
- Crop-centric features (wrong user)
- Spray planning, weed detection on row crops
- Beekeeping / aquaculture (BEEP owns / different users)
- EPCIS, INSPIRE, AgriRouter (skip until forced)

## Suggested follow-up theses

After this research, these specific claims are testable with `--mode thesis`:

1. *"A Rust+Tauri+iroh-smol-kv stack can ship a usable offline-first livestock app in <2k LOC of new domain code"* (verdict pending implementation)
2. *"~70-80% of deployed Bluetooth EID stick readers can be parsed by a single line-buffered ASCII regex"* (high prior probability per [[livestock-eid-rfid]])
3. *"WebODM webhook → rasterstats → farmOS log is the dominant OSS pattern that will emerge for L5 drone-FMS ingestion"* (medium probability)
4. *"Phone-as-companion-computer beats Pi+camera on $/inference for ML-on-drone in 2026"* (high probability for specific MobileNet workloads, lower for YOLO)

## Sources

17 raw sources ingested at `.wiki/raw/articles/`:

Round 1: farmOS, LiteFarm, Ekylibre, OpenDroneMap, iroh sync stack, Tauri 2, AgGateway/ADAPT standards, ArduPilot companion computers, DroneKit-Android status, HerdNet, BovHEAT, DroneDB+micasense, precision-ag drone use cases, FMS feature taxonomy.

Round 2: EID reader protocols, iroh-docs FMS schema, WebODM-FMS bridge.

## Wiki entry points

- [[herd-scout-positioning]] — strategic synthesis
- [[oss-fms-landscape]] — what exists
- [[fms-feature-taxonomy]] — what features are needed
- [[oss-drone-fms-pipeline]] — drone integration layers
- [[webodm-fms-bridge]] — concrete L5 ingest
- [[android-on-drone]] — phone-on-drone verdict
- [[livestock-eid-rfid]] — EID reader gap + crate scope
- [[iroh-docs-fms-schema]] — concrete iroh-smol-kv schema
- [[mobile-desktop-architecture]] — stack picks
