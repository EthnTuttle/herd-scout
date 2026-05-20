---
title: "herd-scout positioning — wedges and what to build"
tags: [herd-scout, positioning, strategy, wedges, mvp, oss, livestock]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# herd-scout positioning

Synthesis across the round-1 research — what makes herd-scout differentiated, what to build, what to consume, and what to skip.

## Defensible wedges (where OSS is actually thin)

1. **Rust-native FMS**. The whole OSS FMS space is PHP/Ruby/JS/Go (see [[oss-fms-landscape]]). No Rust competitor exists. Edge-deployment / performance / single-binary distribution = real difference.
2. **P2P / offline-first with no central server.** farmOS, LiteFarm, Ekylibre all assume a central server. iroh-docs + iroh-blobs + iroh-gossip is a novel architecture for ranch use ([[iroh-sync-stack]], [[mobile-desktop-architecture]]).
3. **Drone-vision integrated livestock workflows.** [[herdnet-livestock-cv]] counts; nothing closes the loop to herd records. No OSS combines drone capture + on-farm vision inference + herd records in a single product. See [[oss-drone-fms-pipeline]] § L5.
4. **Native mobile ranch UX.** Every major OSS FMS punts to PWA. A genuine offline-first native (or Tauri/Rust) mobile app for chute-side livestock work is differentiated.
5. **OSS EID reader bridge.** Zero OSS repos parse ISO 11784/11785 or talk to Allflex/Tru-Test/Datamars/Gallagher Bluetooth readers. Tractable scope, high leverage. See [[livestock-eid-rfid]].

## What herd-scout should consume, not build

- **farmOS data model** ([[fms-data-model]]) — adopt Asset+Log+Quantity+Term+Plan; stay JSON:API compatible so users can import/export
- **OpenDroneMap** ([[opendronemap]]) — orthos, DEM, multispectral. Don't reimplement.
- **micasense/imageprocessing** — radiometric calibration before ODM
- **DroneDB** — drone output hosting (COG/COPC/point clouds)
- **HerdNet** ([[herdnet-livestock-cv]]) — aerial livestock detection model
- **iroh / iroh-docs / iroh-blobs / iroh-gossip** ([[iroh-sync-stack]]) — sync data plane
- **Tauri 2** ([[mobile-desktop-architecture]]) — single Rust process across phone+desktop
- **MAVLink wire protocol + `io.dronefleet.mavlink`** — talk to autopilot if doing phone-on-drone (see [[android-on-drone]])

## What herd-scout should skip

- Closed SaaS competition (don't fight DroneDeploy / AgriWebb / PastureMap directly)
- Crop-centric features (row-crop user is wrong) — [[fms-feature-taxonomy]]
- Spray planning, weed detection on row crops, plant counting
- Beehive, aquaculture (different users, BEEP owns beekeeping)
- DroneKit-Android (dead; fork or roll own MAVLink)
- ElectricSQL local-first sync (pivoted away in 2024)

## MVP slice (informed by [[fms-feature-taxonomy]])

1. **Records**: Animal, Group (herd/mob), Land (paddock), Equipment
2. **Logs**: Observation, Medical (with withdrawal flag), Movement, Weight, Birth
3. **Mobile-first**: offline scouting, geotagged photo, EID scan lookup
4. **Mapping**: paddock polygons (GeoJSON), animal-last-seen pins
5. **Compliance**: treatment log + animal-ID export
6. **Sync**: iroh-docs namespace per farm; phone+desktop both peers
7. **Drone hook**: photo upload from existing P2P video pipe (iroh-live) → attach to log

**Defer**: financial rollups (export to QuickBooks initially), grain marketing, full inventory, ortho ingestion (single geotagged image is enough for MVP), full IoT (start one sensor type, water tank).

## Drone-livestock killer features (post-MVP)

In rough priority for ranch users:
1. **Drone count → reconcile against EID inventory → flag missing/sick animals.** No OSS does this.
2. **Lost-livestock thermal search** (see [[precision-ag-drone-use-cases]])
3. **Body condition scoring** (defensible data play — no public dataset)
4. **Pasture NDVI / forage biomass for grazing rotation** (eats AgriWebb Foragecaster's lunch)
5. **Post-storm fence/structure damage assessment**

## Andoid-on-drone path (see [[android-on-drone]])

If pursued:
- Phone on a Pixhawk/PX4 quad as **(a) onboard ML/CV camera + (b) 4G BVLOS bridge**, talking MAVLink to the FC
- iroh-live (already in repo) handles the phone↔desktop video stream
- HerdNet (or fine-tuned variant) runs inference on phone or desktop
- Counts written as Observation logs to iroh-docs; sync to all peers
- Easier path: Pi 5 + Pi Camera + 4G hat (well-documented, no battery management) — but loses the consumer-phone economics

## See also
- [[oss-fms-landscape]]
- [[fms-data-model]]
- [[fms-feature-taxonomy]]
- [[livestock-oss-gap-analysis]]
- [[livestock-eid-rfid]]
- [[oss-drone-fms-pipeline]]
- [[precision-ag-drone-use-cases]]
- [[mobile-desktop-architecture]]
- [[iroh-sync-stack]]
- [[android-on-drone]]
- [[ag-data-standards]]
- [[implementation-plan]]
- [[drone-hardware]]
- [[drone-vision-software]]

## Sources
Synthesizes findings from all 14 raw sources ingested 2026-05-20.
