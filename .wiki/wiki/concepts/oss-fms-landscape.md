---
title: "OSS Farm Management Systems — landscape"
tags: [fms, farmos, litefarm, ekylibre, oss, landscape]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# OSS Farm Management Systems — landscape

The open-source farm management space is a small constellation around three active platforms, with a long tail of niche tools and a graveyard of abandoned projects.

## The big three

| Project | License | Stack | Mobile | Stars | Status | Strength |
|---|---|---|---|---|---|---|
| **farmOS** | GPL-2.0 | PHP / Drupal 10 | Field Kit PWA (Vue) | 1.3k | Active 2026 | Dominant; canonical [[fms-data-model]] |
| **LiteFarm** | GPL-3.0 | Node + Postgres + React | Mobile-first PWA | 217+ | Active 2026 | Sustainability/cert focus |
| **Ekylibre** | AGPL-3.0 | Ruby on Rails | Responsive web | 476 | Active daily | Heavyweight ERP+accounting (EU) |

Of these, farmOS is the gravity well — biggest user base, mature [[fms-data-model]], JSON:API, real ecosystem (Field Kit PWA, modules). LiteFarm is the modern-stack #2 with NA/LatAm deployments. Ekylibre owns the heavyweight commercial-farm/cooperative segment in Europe.

## Auxiliary active projects

- **Field Kit** (farmOS PWA companion) — offline-first Vue app; sync to farmOS server
- **BEEP** (beekeeping, AGPL) — dominant OSS in beekeeping; LoRaWAN/IoT integration; 5k+ users in 9 languages
- **Hive Pal** (beekeeping, mobile-first TypeScript) — newer 2026 entrant
- **Farm-Data-Relay-System (FDRS)** — sensor mesh networking on ESP32 (ESP-NOW + LoRa + MQTT); 607 stars; the plumbing many DIY farms use
- **Fields2Cover** — coverage path planning library for autonomous tractors/sprayers (BSD, C++)

## Computer-vision-for-ag tools (not FMS, but adjacent)

- **Microsoft FarmVibes.AI** — multi-modal geospatial ML (satellite, drone, weather)
- **OpenWeedLocator** — Pi-based real-time weed detection (461 stars)
- **AgML** — standardized ag datasets + pretrained models (280 stars)
- **FruitNeRF** — NeRF-based fruit counting for orchards (327 stars)
- **[[herdnet-livestock-cv]]** — aerial livestock counting, MIT, 57 stars

None of these are integrated into an FMS. The bridge is the gap.

## Long tail (mostly faltering)

- **Tania** (Go, 815 stars) — last release Dec 2019, abandoned
- **OpenFarm** — archived April 2025
- **OpenATK** — dormant since 2020-2022
- **AgroSense** — freemium hybrid, OSS core unclear
- **Livestock Tracker** (Angular+.NET, 11 stars) — toy scale
- **AVAT** (livestock video annotation, 14 stars) — closest analog to herd-scout but tiny
- Lots of student/demo "livestock management system" repos — see [[livestock-oss-gap-analysis]]

## What this means for herd-scout

farmOS exists. Don't rebuild it. Either:
- Build on top (modules, JSON:API consumer, complementary mobile client), or
- Stay data-compatible (same Asset/Log/Quantity/Term/Plan model), or
- Address what farmOS doesn't do — see [[herd-scout-positioning]]

## See also
- [[fms-data-model]]
- [[fms-feature-taxonomy]]
- [[livestock-oss-gap-analysis]]
- [[oss-drone-fms-pipeline]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-farmos]]
- raw: [[2026-05-20-litefarm]]
- raw: [[2026-05-20-ekylibre]]
