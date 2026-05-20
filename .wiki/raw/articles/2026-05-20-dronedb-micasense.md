---
title: "DroneDB and micasense/imageprocessing — drone data hosting + multispectral processing"
source_url: https://github.com/dronedb/dronedb
secondary_urls:
  - https://github.com/micasense/imageprocessing
type: project
tags: [dronedb, micasense, multispectral, ndvi, drone-hosting, cog, oss]
created: 2026-05-20
confidence: medium
---

# DroneDB

- License: MPL-2.0
- Status: active
- Role: modern hosting layer for drone outputs (COG, COPC, geotagged files, point clouds) with TMS/COG/EPT sharing endpoints
- Has WebODM plugin (`coreplugins/dronedb`)
- The closest thing to an OSS "DroneDeploy data room"

## micasense/imageprocessing

- Stack: Python notebooks
- License: open (community-supported, not vendor-supported)
- ~229 commits, updated for v2 notebooks (RedEdge-P / Altum-PT, 2023)
- Handles: radiometric calibration, panel reflectance detection, band alignment / co-registration, dual-camera 10-band stacks, NDVI/NDRE
- Output: reflectance TIFFs ready for ODM stitching

## Risk

micasense/imageprocessing is community-maintained — vendor is MicaSense (commercial), not Anthropic-of-MicaSense. Long-term maintenance is uncertain.

## Relevance to herd-scout

Two niche but real layers in the OSS drone pipeline. DroneDB is the natural "store the orthos" layer. micasense scripts handle the painful radiometric calibration step before ODM. Both are consumed, not reimplemented.
