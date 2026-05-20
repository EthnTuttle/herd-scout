---
title: "Livestock OSS — gap analysis"
tags: [livestock, oss, gaps, eid, rfid, herd, dairy, cattle, pasture]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# Livestock OSS — gap analysis

The general OSS FMS landscape has a few mature platforms (see [[oss-fms-landscape]]). The **livestock-specific** layer is dramatically thinner. This is herd-scout's home turf.

## What exists

### General with livestock support
- **farmOS** — has Animal asset type, Group container, medical/observation/movement logs. Handles livestock as one of many entity types but not deeply.

### Species-specific OSS
- **BovHEAT** (MIT, Python, 20 stars) — heat/estrus detection from SCR Heatime accelerometer XLSX. Niche but production-quality. Single sensor brand. See [[2026-05-20-bovheat]]
- **NSIP example** (zircote/nsip-example) — sheep breeding records as GitHub Issues. Quirky.
- **Mastitis ML, dairy nutrition tools, Pashu-Aahar** — mostly research/coursework

### Computer vision
- **[[herdnet-livestock-cv]]** (MIT, ~57 stars) — aerial livestock counting, 73-83% F1. Counts only; doesn't manage records.
- **AVAT** — livestock video annotation, 14 stars, toy scale

### Pasture / rotational grazing
- **Piquetear** (React Native) — rotational grazing planner, 0 stars, abandoned Jan 2023
- **No mature OSS PastureMap alternative.** Closed competitors (PastureMap, MaiaGrazing, AgriWebb) own this.

## Pain points

1. **EID/RFID is unsolved in OSS.** GitHub search "ISO 11784 RFID livestock" returned **zero repositories**. No OSS readers/handlers for:
   - ISO 11784/11785 HDX/FDX-B half-duplex protocol
   - USDA 840, NLIS, CCIA tag formats
   - Allflex / Tru-Test / Datamars / Gallagher reader protocols (Bluetooth SPP / serial)

   Every rancher with an Allflex stick reader is stuck in vendor apps. See [[livestock-eid-rfid]].

2. **No offline-capable mobile EID reader app in OSS.** farmOS Field Kit doesn't speak EID readers.

3. **No OSS reconciliation layer**: match drone-counted animals to EID inventory to flag missing/sick animals.

4. **No OSS exports for compliance** (USDA 840 reporting, NLIS uploads, BVD/TB schemes).

5. **Project starvation pattern**: high interest (lots of student projects on GitHub) but low completion rate — real ranchers don't pay or contribute code.

## Underserved species

Dairy gets most academic attention. Particularly underserved:
- **Beef cow-calf** (extensive grazing operations)
- **Sheep / goats** (small-flock smallholders)
- **Aquaculture** — essentially zero meaningful OSS

## Gaps where herd-scout adds asymmetric value

1. **Drone herd counting → herd inventory reconciliation.** [[herdnet-livestock-cv]] counts; nothing closes the loop to "compare to EID inventory, flag missing." This is the obvious novel angle.
2. **OSS EID reader bridge.** A small Rust library reading ISO 11784/11785 from common Bluetooth stick readers (Allflex, Tru-Test, Datamars, Gallagher), with farmOS sync, would have outsized impact. See [[livestock-eid-rfid]].
3. **Offline-first native mobile** for chute-side and pasture use; farmOS is web-heavy, Field Kit is a PWA.
4. **Pasture rotation planner** — Piquetear is dead, PastureMap closed, farmOS doesn't really cover it.
5. **Compliance exports** (USDA 840, NLIS, CCIA, EU EID registry).

## See also
- [[oss-fms-landscape]]
- [[livestock-eid-rfid]]
- [[herdnet-livestock-cv]]
- [[oss-drone-fms-pipeline]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-bovheat]]
- raw: [[2026-05-20-herdnet]]
- raw: [[2026-05-20-fms-feature-taxonomy]]
