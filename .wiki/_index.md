# Herd Scout Wiki

- topic: herd-scout
- created: 2026-05-19
- updated: 2026-06-02
- status: active
- type: local
- summary: Open source livestock-focused farm management — Rust + iroh P2P + drone vision + native mobile

## Research Focus
- OSS farm management systems landscape and gaps
- Mobile↔desktop offline-first architecture (Tauri + iroh)
- Drone integrations (capture → photogrammetry → FMS)
- Android phone as onboard companion / 4G bridge
- Livestock-specific tooling (EID, herd records, pasture)
- Computer vision for livestock counting + fenceline + thermal

## Sources
- 48 ingested sources — see [[raw/_index]]

## Articles

### Concepts (19)

**Strategy / synthesis**
- [[concepts/herd-scout-positioning]] — defensible wedges and what to build

**OSS FMS landscape**
- [[concepts/oss-fms-landscape]] — farmOS, LiteFarm, Ekylibre + the long tail
- [[concepts/fms-data-model]] — Asset / Log / Quantity / Term / Plan
- [[concepts/fms-feature-taxonomy]] — must / nice / advanced
- [[concepts/livestock-oss-gap-analysis]] — where OSS livestock is thin
- [[concepts/livestock-eid-rfid]] — ISO 11784/11785, Allflex, the OSS gap
- [[concepts/ag-data-standards]] — adopt / skip pragmatic guide

**Mobile-desktop architecture**
- [[concepts/mobile-desktop-architecture]] — offline-first stack picks
- [[concepts/iroh-sync-stack]] — iroh-docs + iroh-blobs + iroh-gossip
- [[concepts/iroh-docs-fms-schema]] — concrete key layout + code sketch

**Computer vision / counting**
- [[concepts/herd-counting-pipeline]] — 5-layer pipeline: detection → tracking → counting → aggregation → validation
- [[concepts/livestock-cv-accuracy]] — realistic precision / recall / MAE numbers + altitude band
- [[concepts/tracker-choice-bot-oc-byte]] — ByteTrack vs BoT-SORT vs OC-SORT — when to switch
- [[concepts/track-recovery-busca-hit]] — online + offline tracklet recovery
- [[concepts/cattle-reid-self-supervised]] — concrete recipe replacing the ResNet50 stub
- [[concepts/bootstrap-conformal-count-ci]] — block bootstrap, BCa, J+aB conformal hybrid
- [[concepts/yolo26-and-tracker-compat]] — YOLO26 retrain + tracker head-choice constraint

**Drone**
- [[concepts/drone-vision-software]] — YOLO, OpenDataCam, video streaming
- [[concepts/drone-hardware]] — ArduPilot, phone as camera, Jetson
- [[concepts/oss-drone-fms-pipeline]] — 6 layers, the broken L5 handoff
- [[concepts/webodm-fms-bridge]] — concrete L5 bridge pipeline + MVP
- [[concepts/precision-ag-drone-use-cases]] — relevance matrix
- [[concepts/android-on-drone]] — verdict + architecture
- [[concepts/phone-on-drone-airframe]] — buildable BOM, vibration, mounting, lifetime
- [[concepts/phone-power-on-drone]] — battery life, USB-PD, degradation
- [[concepts/phone-thermal-management]] — Thermal API, sustained-perf, donor-phone choice
- [[concepts/phone-publisher-android-fgs]] — Android 14/15/16 foreground-service constraints
- [[concepts/implementation-plan]] — original ~$530 build plan

## Status
- Round 1 complete (8 parallel agents, 14 sources, 12 new articles)
- Round 2 complete (3 focused agents on top gaps, 3 sources, 2 new articles + 2 articles enriched)
- Round 3 complete (8 parallel agents on accurate-herd-counting question, 11 sources, 2 new concept articles + cross-links)
- Round 4 complete (8 parallel agents on MOT upgrades + phone-on-drone airframe, 20 sources, 9 new concept + 1 reference + 1 playbook)
- Total: 48 sources, 25 concept articles + 1 reference

## Outputs

- [[output/playbook-herd-scout-2026-05-20]] — strategic playbook
- [[output/plan-mobile-to-desktop-iroh-rfc-2026-05-20]] — RFC for the mobile-to-desktop iroh app MVP
- [[output/plan-deploy-daemon-on-1060-laptop-2026-05-22]] — roadmap for deploying the daemon on the GTX 1060 GS63VR laptop (headless Ubuntu, ort+CUDA, systemd)
- [[output/plan-optimize-cv-sidecar-trt-yolo11s-2026-05-26]] — roadmap for optimizing the CV sidecar (YOLO11s + embedded NMS, supervision/ByteTrack, TRT 8.6 EFFICIENT_NMS gated)
- [[output/plan-iroh-bound-ssh-access-daemon-2026-05-26]] — roadmap for iroh-bound SSH access to the daemon (third ALPN on the existing Live router, NodeId allowlist, `herdctl proxy` as ssh ProxyCommand — replaces the SSH UDS forward from the deploy plan)
- [[output/plan-android-admin-allowlist-app-2026-05-27]] — roadmap for an Android admin APK that manages the daemon's permitted NodeIDs over a fourth ALPN `herd-scout/admin/1` (separate `[control_plane.admins]` allowlist, atomic `control.toml` rewrites, append-only audit log on both ends + `TailAudit` RPC, single-slot fleet switcher, versioned `identity.toml` envelope shared by daemon/herdctl/phone for backup/restore, dedicated `com.herdscout.admin` build flavor)
- [[output/playbook-accurate-herd-counting-2026-05-27]] — playbook for accurate herd counting from CV detections (5-layer pipeline: detection → tracking → counting → aggregation → validation; EID reconciliation as the differentiator)
- [[output/plan-desktop-video-upload-2026-05-28]] — roadmap for desktop video upload to daemon (fifth ALPN `herd-scout/upload/1` over iroh-blobs, sidecar file-decode mode, single-clip queue behind live phone, per-clip JSON report applying the accurate-counting playbook, GUI drag-drop + `herdctl push`)
- [[output/playbook-mot-airframe-2026-06-01]] — Round-4 playbook: P0/P1/P2 counting upgrades (YOLO26 retrain, block-bootstrap+BCa+J+aB, OC-SORT A/B, HIT post-hoc, self-supervised cattle re-ID) + buildable phone-on-drone airframe spec (95A TPU + 50A grommets + suspended topology, USB-PD power, thermal listener ladder, `camera|connectedDevice` FGS manifest)
- [[output/assess-herd-scout-2026-06-02]] — Repo vs wiki vs market gap analysis (--retardmax): 14 alignments, 10 research gaps, 19 build opportunities, 17 market gaps; immediate `/wiki:research` queue, P0/P1/P2 build queue, competitive landscape, emerging trends, anti-patterns from the OSS livestock graveyard, confidence notes
- [[output/plan-fms-schema-and-records-2026-06-02]] — Roadmap for the assess P0: iroh-smol-kv FMS schema + Animal/Group/Land/Equipment + 5-log CRUD; 7 architecture decisions, 7 phases, egui frontend (Tauri 2 deferred), co-location-aware SQLite projection, QR farm-namespace onboarding, iroh 0.98.0 pin (defers EID crate + JSON:API to separate plans)
