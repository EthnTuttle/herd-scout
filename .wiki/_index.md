# Herd Scout Wiki

- topic: herd-scout
- created: 2026-05-19
- updated: 2026-05-20
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
- 17 ingested sources — see [[raw/_index]]

## Articles

### Concepts (17)

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

**Drone**
- [[concepts/drone-vision-software]] — YOLO, OpenDataCam, video streaming
- [[concepts/drone-hardware]] — ArduPilot, phone as camera, Jetson
- [[concepts/oss-drone-fms-pipeline]] — 6 layers, the broken L5 handoff
- [[concepts/webodm-fms-bridge]] — concrete L5 bridge pipeline + MVP
- [[concepts/precision-ag-drone-use-cases]] — relevance matrix
- [[concepts/android-on-drone]] — verdict + architecture
- [[concepts/implementation-plan]] — original ~$530 build plan

## Status
- Round 1 complete (8 parallel agents, 14 sources, 12 new articles)
- Round 2 complete (3 focused agents on top gaps, 3 sources, 2 new articles + 2 articles enriched)
- Total: 17 sources, 14 concept articles + 3 pre-existing articles cross-linked

## Outputs

- [[output/playbook-herd-scout-2026-05-20]] — strategic playbook
- [[output/plan-mobile-to-desktop-iroh-rfc-2026-05-20]] — RFC for the mobile-to-desktop iroh app MVP
- [[output/plan-deploy-daemon-on-1060-laptop-2026-05-22]] — roadmap for deploying the daemon on the GTX 1060 GS63VR laptop (headless Ubuntu, ort+CUDA, systemd)
