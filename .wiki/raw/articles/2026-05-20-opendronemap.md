---
title: "OpenDroneMap (ODM, NodeODM, ClusterODM, WebODM) — OSS photogrammetry pipeline"
source_url: https://docs.opendronemap.org
secondary_urls:
  - https://github.com/OpenDroneMap/ODM
  - https://github.com/OpenDroneMap/WebODM
  - https://docs.opendronemap.org/multispectral/
type: project
tags: [opendronemap, odm, webodm, photogrammetry, ndvi, multispectral, orthomosaic, geotiff, oss]
created: 2026-05-20
confidence: high
---

# OpenDroneMap (ODM)

- License: BSD
- Stack: Python + C++ (OpenSfM, OpenMVS, GDAL, PDAL, MVS-Texturing)
- Status: production, dominant in OSS photogrammetry

## Layered architecture

- **ODM** — core CLI pipeline
- **NodeODM** — REST API wrapper around a single processing node
- **ClusterODM** — load balancer across many NodeODMs (autoscaling on AWS/Hetzner/Scaleway)
- **WebODM** — Django + React UI, Docker, talks to NodeODM/ClusterODM. Plugin system in `coreplugins/`: align-service, cesiumion, contours, dronedb, lightning, measure, **objdetect**.

## Inputs / outputs

- **Inputs**: JPEG, TIFF, DNG, plus video (.mp4/.mov/.lrv/.ts) with GPS subtitles in recent ODM versions
- **Outputs**: GeoTIFF orthomosaic, DSM/DTM GeoTIFF, LAZ point cloud, OBJ/PLY 3D mesh, textured model

## Multispectral support

Officially supports MicaSense RedEdge-MX, MicaSense Altum, Sentera 6X, DJI P4 Multispectral, DJI Mavic 3 Multispectral. Use `--radiometric-calibration camera+sun` for radiometric normalization. Output is multi-band GeoTIFF orthomosaic. **NDVI/NDRE/VARI/GLI/EXG indices are NOT baked into ODM core** — calculated downstream in WebODM expression UI or QGIS Raster Calculator.

## Hardware needs

- CPU-bound; GPU (NVIDIA GTX 9xx+) gives ~2x SIFT speedup
- 500-image RGB dataset: ~1-3 hours on 16-core/32GB
- 5-band multispectral: 2-4x longer
- Docker is the recommended deploy

## Quality

With Ground Control Points (GCPs), accuracy competitive with Pix4D for orthomosaic/DSM. Tree canopy / dense vegetation edges still favor commercial.

## Relevance to herd-scout

ODM owns the photogrammetry layer end-to-end. **Don't reimplement — consume ODM outputs.** WebODM coreplugin/objdetect even shows a precedent for object detection on orthos. The break in the OSS pipeline is the *handoff from ODM/WebODM to FMS data model* — an ingestion bridge there is high-value.
