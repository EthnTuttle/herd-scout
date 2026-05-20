---
title: "OSS drone→FMS pipeline — layers, gaps, the broken handoff"
tags: [drone, fms, opendronemap, ndvi, orthomosaic, pipeline, integration]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# OSS drone→FMS pipeline

How drone footage becomes useful structured data inside an FMS, with no closed-source links in the chain.

## The 6 layers

```
L1 Capture (flight planning)
   └─ L2 Stitching/photogrammetry
       └─ L3 Multispectral/NDVI
           └─ L4 Storage/GIS
               └─ L5 FMS ingestion       ← THE BROKEN LINK
                   └─ L6 Real-time inference (research-grade)
```

### L1 — Capture (mature)

- **QGroundControl** (Qt, BSD/Apache, MAVLink) — Pattern Tool generates survey grids over polygons; configurable angle/altitude/overlap; works with PX4 and ArduPilot.
- **Mission Planner / APM Planner** — auto-generates survey/grid missions from drawn polygons; KML import via add-ons.
- **PaparazziUAV / MAVProxy** — niche.
- Output: flight log + geotagged JPGs/TIFFs.

### L2 — Stitching/photogrammetry (mature)

- **[[opendronemap]]** is the unambiguous OSS center of gravity. ODM CLI + NodeODM REST + ClusterODM load balancer + WebODM UI.
- Accuracy with GCPs is competitive with Pix4D for ortho/DSM. Tree canopy/dense veg edges still favor commercial.
- **Don't reimplement — consume ODM outputs.**

### L3 — Multispectral/NDVI (mature, two paths)

**Path A — through ODM (preferred):** ODM officially supports MicaSense RedEdge-MX/Altum, Sentera 6X, DJI P4M, DJI Mavic 3M. `--radiometric-calibration camera+sun` produces multi-band GeoTIFF orthomosaic. NDVI/NDRE/VARI/GLI/EXG calculated downstream in WebODM expression UI or QGIS Raster Calculator. **Indices are NOT baked into ODM core.**

**Path B — vendor scripts:** **micasense/imageprocessing** (community Python notebooks) — handles radiometric calibration, panel reflectance, band alignment. Outputs reflectance TIFFs ready for ODM.

For RGB-only drones: VARI/GLI from RGB give a poor-man's vegetation proxy.

### L4 — Storage/GIS (mature)

- **GeoTIFF / COG (Cloud-Optimized GeoTIFF)** — universal raster interchange
- **GeoPackage** — SQLite-based, replaces shapefile
- **PostGIS** on PostgreSQL — spatial DB
- **QGIS** — desktop GIS; `Vegetation Indices` plugin computes 22+ indices
- **GeoServer / MapServer** — serve WMS/WMTS/WFS
- **DroneDB** (MPL-2.0) — modern hosting for drone outputs (COG, COPC, geotagged files, point clouds); has WebODM plugin. Closest OSS "DroneDeploy data room"

### L5 — FMS ingestion (THE BROKEN LINK)

This is where OSS pipelines break:

- **farmOS-map** supports WMS/XYZ/Google/ArcGIS tiles, GeoJSON, WKT vectors. **Does NOT natively ingest GeoTIFF/COG.** No mainline farmOS plugin for ODM/DroneDB.
- Long-closed issue (#221, OGC WFS for areas) → low priority.
- Practical workflow today: ODM → GeoTIFF → serve via GeoServer (or DroneDB COG) → add as WMS/XYZ layer URL inside farmOS. **Manual, per-flight, file-based, lossy.**
- Vegetation index data does not become a queryable per-field attribute. Remains a backdrop layer.
- No automatic per-area aggregation ("mean NDVI for field 7 on 2026-05-20").
- **Tania, LiteFarm**: also no drone/raster ingestion modules.
- **Field Mapper** (newer 2024/2025 GitHub) advertises OSS NDVI + prescription maps — early-stage, watch.

### L6 — Real-time / live streaming (research-grade)

- **MAVLink Camera Protocol** + **GStreamer** RTSP/RTMP — works on Pixhawk + companion. QGC can preview RTSP from the air.
- **ROS2 + PX4** + Aerial Autonomy Stack — research / academic.
- **No OSS tool currently does live NDVI from a flying drone** because radiometric calibration usually requires a panel image at takeoff and pose-corrected stitching post-flight.
- Live-inference for object detection (e.g., herd counting on RTSP) is doable today — see [[drone-vision-software]].

## Gaps

1. **L5 ingestion is the killer gap.** Orthos/NDVI rasters do not become structured per-field attributes in any mainstream OSS FMS without custom glue (Python + PostGIS zonal stats + farmOS REST API).
2. **No prescription-map round-trip** — generating variable-rate-application shapefiles from NDVI for a sprayer is manual QGIS.
3. **WebODM plugin ecosystem is small** — mostly first-party plugins.
4. **Multispectral camera lock-in** — community Python tools are not vendor-supported.
5. **No OSS managed ClusterODM offering** at scale (webodm.net is semi-commercial).
6. **Time-series / change detection** — no OSS FMS module compares a field's NDVI across flights automatically.

## Implication for herd-scout

The L1→L4 chain is fully viable in OSS today. **The L5 (FMS structured ingest) and L6 (real-time inference) layers are where OSS pipelines break**. A glue tool between WebODM/DroneDB and an FMS — or a herd-scout-native ingestion path that reads ODM outputs and updates Asset+Log records (e.g., "this orthomosaic counted 247 cattle in paddock A on 2026-05-20") — has outsized impact.

## See also
- [[opendronemap]]
- [[drone-vision-software]]
- [[drone-hardware]]
- [[implementation-plan]]
- [[precision-ag-drone-use-cases]]
- [[fms-data-model]]
- [[ag-data-standards]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-opendronemap]]
- raw: [[2026-05-20-dronedb-micasense]]
- raw: [[2026-05-20-precision-ag-drone-use-cases]]
