---
title: "Precision-ag drone use cases beyond herd counting"
source_url: https://en.wikipedia.org/wiki/Precision_agriculture
secondary_urls:
  - https://docs.opendronemap.org/multispectral/
  - https://github.com/geezacoleman/OpenWeedLocator
type: synthesis
tags: [precision-ag, drone, use-cases, ndvi, thermal, weed-detection, body-condition-score]
created: 2026-05-20
confidence: high
---

# Precision-ag drone use case matrix (OSS-relevance lens for herd-scout)

## High relevance to herd-scout

| Use case | Drone HW | OSS today | Maturity | Why it matters |
|---|---|---|---|---|
| **Lost-livestock thermal search** | Thermal (Mavic 3T, M30T) | None turn-key; YOLO retrained on thermal would work | research / hand-rolled | Same users, same hardware, same flight pattern as herd counting. Clear OSS gap. |
| **Body condition score (BCS) / heat stress** | High-res RGB; thermal | Academic only; no released models or public datasets | research | Premium feature for cow-calf; **product moat opportunity** (defensible data play) |
| **Pasture NDVI / forage biomass** | Multispectral or NIR-converted RGB | ODM + WebODM expression UI / QGIS | production | Rangeland equivalent of crop NDVI; ranchers care about grazing rotation planning |
| **Post-storm fence/structure damage** | RGB | ODM ortho is production; change detection DIY | production for capture | Adjacent to fenceline inspection; same flight pattern |

## Medium relevance

| Use case | Drone HW | OSS today | Maturity |
|---|---|---|---|
| **Predator deterrence (patrol+thermal+speaker)** | Thermal + payload | None | fantasy near-term |
| **Elevation/drainage/volumetrics** | RGB photogrammetry; LIDAR | ODM (DEM/DSM/contours), PDAL | production |
| **Irrigation/water stress (thermal)** | Thermal | Thin — ODM accepts thermal TIFFs but no calibration pipeline | research |

## Low relevance (different user / different problem)

| Use case | Why low |
|---|---|
| Weed detection (RGB/multispectral) | Datasets exist (CropAndWeed, CWD30, CoFly-WeedDB) but pasture weed control isn't aerial; ranchers spot-spray ground |
| Plant/stand counting | Same object-detection pattern as herd counting, but row crop user — OpenWeedLocator (461 stars) and FruitNeRF (327 stars) own this niche |
| Yield estimation (row crops) | Wrong user |
| Spray application planning | Commercial moat (DJI Agras, XAG); no OSS variable-rate prescription generation |
| Pest scouting | Research notebooks only |
| Beehive thermal inspection | Wrong user (BEEP owns beekeeping) |

## Big-picture takeaways

1. **ODM owns the photogrammetry layer end-to-end.** Don't reimplement; consume.
2. **Biggest OSS gaps are thermal pipelines and livestock-specific models.** Both directly adjacent to herd-scout.
3. **Highest-relevance roadmap**: lost-livestock thermal search, BCS estimation, pasture NDVI/forage biomass, post-storm damage. All reuse existing herd-scout flight + capture + ortho infrastructure.
4. **No standout OSS BCS or thermal-livestock dataset exists** — defensible data play.
