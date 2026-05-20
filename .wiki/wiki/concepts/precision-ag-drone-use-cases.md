---
title: "Precision-ag drone use cases — relevance matrix for herd-scout"
tags: [precision-ag, drone, use-cases, ndvi, thermal, body-condition, lost-livestock, fenceline]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# Precision-ag drone use cases

Beyond herd counting and fenceline (already in [[drone-vision-software]]), what else can drones do for farms — and which use cases are adjacent enough to herd-scout's wheelhouse to warrant roadmap attention?

## High relevance to herd-scout

| Use case | Drone HW | OSS today | Maturity | Roadmap value |
|---|---|---|---|---|
| **Lost-livestock thermal search** | Thermal (Mavic 3T, M30T) | None turn-key | Hand-rolled / research | Same users, same hardware, same flight pattern as herd counting. Clear OSS gap. |
| **Body condition score (BCS) / heat stress** | High-res RGB; thermal | Academic only | Research | Premium feature for cow-calf. **Defensible data play** — no public dataset. |
| **Pasture NDVI / forage biomass** | Multispectral or NIR-RGB | ODM + WebODM expression UI / QGIS | Production | Rangeland equivalent of crop NDVI; ranchers care for grazing rotation |
| **Post-storm fence/structure damage** | RGB | ODM ortho is production; change detection DIY | Production for capture | Adjacent to fenceline inspection |

## Medium relevance

| Use case | Drone HW | OSS today | Maturity |
|---|---|---|---|
| Predator deterrence (patrol+thermal+speaker) | Thermal + payload | None | Fantasy near-term |
| Elevation/drainage/volumetrics | RGB photogrammetry; LIDAR | ODM (DEM/DSM/contours), PDAL | Production |
| Irrigation/water stress (thermal) | Thermal | Thin (no calibration pipeline) | Research |

## Low relevance (different user)

| Use case | Why low |
|---|---|
| Weed detection (RGB/multispectral) | Datasets exist (CropAndWeed, CWD30, CoFly-WeedDB) but pasture weed control isn't aerial |
| Plant/stand counting | Same pattern as herd counting but row-crop user — OpenWeedLocator, FruitNeRF own |
| Yield estimation | Wrong user |
| Spray application planning | Commercial moat (DJI Agras, XAG); no OSS variable-rate prescription |
| Pest scouting | Research notebooks only |
| Beehive thermal inspection | Wrong user (BEEP owns) |

## Synthesis

1. ODM owns L2-L4 of [[oss-drone-fms-pipeline]] end-to-end. Don't reimplement.
2. **Biggest OSS gaps are thermal pipelines and livestock-specific models.** Both directly adjacent to herd-scout.
3. **Highest-relevance roadmap**: lost-livestock thermal search, BCS estimation, pasture NDVI/forage biomass, post-storm damage. All reuse herd-scout flight + capture + ortho infrastructure.
4. **No standout OSS BCS or thermal-livestock dataset exists.** Defensible data play.

## See also
- [[drone-vision-software]]
- [[drone-hardware]]
- [[oss-drone-fms-pipeline]]
- [[opendronemap]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-precision-ag-drone-use-cases]]
