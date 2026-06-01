# Concepts Index

Compiled wiki articles. Synthesis from raw sources with cross-references and confidence levels.

## Strategy / synthesis

- [[herd-scout-positioning]] — wedges, MVP slice, killer features

## OSS Farm Management

- [[oss-fms-landscape]] — landscape catalog
- [[fms-data-model]] — five primitives (Asset/Log/Quantity/Term/Plan)
- [[fms-feature-taxonomy]] — must/nice/advanced features
- [[livestock-oss-gap-analysis]] — livestock-specific gaps
- [[livestock-eid-rfid]] — RFID standards + OSS gap
- [[ag-data-standards]] — what to adopt, what to skip

## Mobile-desktop architecture

- [[mobile-desktop-architecture]] — stack picks + sync engines
- [[iroh-sync-stack]] — iroh-docs + iroh-blobs + iroh-gossip
- [[iroh-docs-fms-schema]] — concrete key layout + code sketch

## Drone

- [[drone-vision-software]] — YOLO + OpenDataCam + RTSP
- [[drone-hardware]] — ArduPilot + companion + phone-as-camera
- [[oss-drone-fms-pipeline]] — 6 layers; L5 ingestion is the broken link
- [[webodm-fms-bridge]] — concrete pipeline + 80 LOC MVP
- [[precision-ag-drone-use-cases]] — relevance matrix for herd-scout
- [[android-on-drone]] — phone as companion + 4G bridge
- [[implementation-plan]] — ~$530 phone-on-drone build plan

## Phone-on-drone airframe (Round 4, 2026-06-01)

- [[phone-on-drone-airframe]] — buildable BOM, vibration, mounting, lifetime
- [[phone-power-on-drone]] — battery life, USB-PD spec, degradation
- [[phone-thermal-management]] — Thermal API, sustained-perf mode, donor-phone choice
- [[phone-publisher-android-fgs]] — Android 14/15/16 foreground-service constraints

## Computer vision / counting (Round 4 follow-ups, 2026-06-01)

- [[tracker-choice-bot-oc-byte]] — ByteTrack vs BoT-SORT vs OC-SORT — when to switch
- [[track-recovery-busca-hit]] — online + offline tracklet recovery
- [[cattle-reid-self-supervised]] — concrete recipe replacing the ResNet50 stub
- [[bootstrap-conformal-count-ci]] — block bootstrap, BCa, J+aB
- [[yolo26-and-tracker-compat]] — YOLO26 retrain + tracker head-choice constraint
