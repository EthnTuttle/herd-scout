---
title: "WebODM → FMS ingestion bridge — concrete pipeline"
source_url: https://docs.opendronemap.org
secondary_urls:
  - https://github.com/perrygeo/python-rasterstats
  - https://farmos.org/development/api/
type: synthesis
tags: [webodm, odm, ndvi, zonal-statistics, rasterstats, postgis, farmos, json-api, ingest]
created: 2026-05-20
confidence: medium
caveats: |
  Round 2 agent could not WebFetch live API docs. Pipeline shape and code
  patterns are correct in principle; verify exact NodeODM webhook field
  names and current farmOS JSON:API quantity payload shape against live
  docs before implementation.
---

# WebODM → FMS ingestion bridge

## Pipeline diagram

```
[ Mission plan ] ──> [ Flight (geotagged JPG/TIFF + panel shot) ]
                           │
                           ▼
                  [ Reflectance prep ]              (multispectral only)
                  micasense/imageprocessing
                           │  reflectance TIFFs
                           ▼
                  [ ODM / NodeODM ]   POST /task/new ──> task_uuid
                  --radiometric-calibration camera+sun
                           │  multi-band ortho.tif, dsm.tif, point.laz
                           ▼
                  [ Index calc ]   gdal_calc / WebODM expression
                  NDVI = (NIR-RED)/(NIR+RED)  -> ndvi.tif (COG)
                           │
                           ▼
                  [ Zonal stats ]   rasterstats  OR  PostGIS ST_SummaryStatsAgg
                  inputs: ndvi.tif  +  paddock polygons
                  outputs: per-paddock {mean,min,max,std,p10,p90,count,
                           pct_below_0.3, area_stressed_m2}
                           │
                           ▼
                  [ FMS writer ]   farmOS JSON:API POST /api/log/observation
                  - asset.land = paddock UUID
                  - timestamp = flight time
                  - quantities = [{measure:ratio, value:0.62, unit:NDVI, label:"mean"}, …]
                  - file attachment = ndvi COG (or external URL via DroneDB/GeoServer)
                           │
                           ▼
                  [ Map UI ]   farmOS-map shows COG via XYZ/WMS layer +
                               paddock polygon coloured by latest NDVI quantity
```

## Tools per step

| Step | Tool | URL |
|---|---|---|
| Reflectance prep | `micasense/imageprocessing` | github.com/micasense/imageprocessing |
| Photogrammetry | ODM / NodeODM / WebODM | github.com/OpenDroneMap |
| Index calc | `gdal_calc.py`, `rio-calc` | gdal.org, rasterio.readthedocs.io |
| COG conversion | `rio cogeo` | github.com/cogeotiff/rio-cogeo |
| Zonal stats | **`rasterstats`** | github.com/perrygeo/python-rasterstats |
| Spatial DB alt | PostGIS Raster `ST_SummaryStatsAgg`, `ST_Clip` | postgis.net/docs |
| FMS write | farmOS JSON:API | farmos.org/development/api |
| Hosting orthos | DroneDB or GeoServer | docs.dronedb.app, geoserver.org |
| Polygon UI | farmOS-map | github.com/farmOS/farmOS-map |

## Zonal stats — pick `rasterstats`

- **`rasterstats`** (Python, MIT, rasterio+fiona). One call: `zonal_stats('paddocks.geojson','ndvi.tif', stats=['mean','min','max','std','count','percentile_10','percentile_90'])` returns list of dicts. Pip-installable, zero infra. Throughput fine for ranch scale (≤100 paddocks × ≤1GB ortho). Supports `categorical` and `add_stats` (custom lambdas for "pct above threshold").
- **PostGIS Raster** (`ST_Clip` + `ST_SummaryStatsAgg`) — powerful with existing Postgres+PostGIS, but raster tile import (`raster2pgsql`) and tuning is real ops burden. Overkill for herd-scout.
- **QGIS Zonal Statistics** — manual GUI, useful for one-off validation.
- **GDAL alone** — DIY rasterstats; no reason to rebuild.
- **Google Earth Engine** — closed, cloud-only, GEE account required. Wrong for OSS / offline.

## NodeODM REST + webhooks

- `POST /task/new` with images
- `GET /task/<uuid>/info` — poll status; `status.code`: 10 queued, 20 running, 40 completed, 30/50 failed
- `GET /task/<uuid>/download/<asset>` — `orthophoto.tif`, `dsm.tif`, `dtm.tif`, `all.zip`
- Auth via token
- **Webhook**: `POST /task/new` accepts `webhook` form field (URL). NodeODM POSTs task JSON to that URL on completion. **Right primitive for herd-scout.**
- WebODM Django adds project/task/permission layers; same NodeODM mechanics underneath. WebODM `coreplugins/dronedb` = precedent for "on task complete, push outputs elsewhere".
- **Binding flight → paddock**: not done by ODM. Client-side: when uploading task, herd-scout knows paddock UUID; stores `(task_uuid → asset_uuid)` in its own DB. On webhook callback, look up asset, run zonal stats clipped to that asset's geometry, write log.

## Metrics ranchers actually use

For pasture / rangeland (not row crop):

- **Mean NDVI per paddock** — primary signal; drives grazing rotation order
- **Stddev NDVI** — uniformity proxy; high stddev = patchy
- **Percentile 10 / 90** — robust to shadows/water outliers
- **Pixel count above threshold** (e.g., NDVI > 0.5) — converted to area_m2 → "ha of healthy forage"
- **Area of stressed zones** (NDVI < 0.3) — bare/overgrazed acres
- **Histogram (10 bins)** — distribution shape, change detection
- **Delta vs previous flight** — most actionable signal

Don't bother with: NDRE (only useful for fertilization on row crops), fancy SAVI/MSAVI variants (marginal gain on rangeland), per-pixel temporal series (collapse to paddock).

## Time-series storage (farmOS-style)

**One Observation log per (flight, paddock).** No separate Flight entity needed.

```jsonc
// Log: type=observation, timestamp=2026-05-20T11:23:00Z
{
  "asset": [{ "type":"asset--land", "id":"<paddock-7-uuid>" }],
  "name": "Drone NDVI flight 2026-05-20",
  "quantity": [
    {"measure":"ratio","value":0.62,"units":"NDVI","label":"mean"},
    {"measure":"ratio","value":0.11,"units":"NDVI","label":"stddev"},
    {"measure":"area","value":3.4,"units":"ha","label":"stressed_area"},
    {"measure":"count","value":284571,"units":"px","label":"pixel_count"}
  ],
  "file": [/* COG attachment or DroneDB URL */],
  "data": "{\"flight_id\":\"...\",\"odm_task_uuid\":\"...\",\"sensor\":\"P4M\"}"
}
```

Time-series falls out of querying logs by `asset.id` ordered by `timestamp`. Optionally an `Equipment`-type asset for the drone enables flight-level grouping. For multi-paddock flights: shared `data.flight_id` UUID across the per-paddock logs.

## Prior art

- **WebODM `coreplugins/dronedb`** — closest existing "ODM-output to external system" plugin. Pushes COGs to DroneDB, not to FMS log model.
- **WebODM `coreplugins/objdetect`** — runs object detection on orthos; precedent for post-processing pipelines but no FMS write.
- **Field Mapper** (newer 2024-2025) — advertises OSS NDVI + prescription maps. Round 1 flagged early-stage; needs deeper look.
- **AgGateway ADAPT** — standardizes prescription maps and as-applied data; schema layer, not pipeline.
- **farmOS forum threads** — recurring requests for GeoTIFF/COG ingest; no merged module. Confirms gap.
- **No shipped OSS module ties (ODM webhook → zonal stats → FMS log).** Genuinely open slot.

## MVP — minimum viable bridge (~80 LOC Python)

```python
def on_task_complete(payload):
    task_uuid = payload['uuid']
    asset_id  = lookup_asset_for_task(task_uuid)
    ortho     = nodeodm.download(task_uuid, 'orthophoto.tif')

    # 2. Compute NDVI (assuming bands ordered B,G,R,NIR,RE)
    subprocess.run(['gdal_calc.py',
        '-A', ortho, '--A_band=4',  # NIR
        '-B', ortho, '--B_band=3',  # RED
        '--calc=(A-B)/(A+B+1e-6)', '--outfile=ndvi.tif'])

    # 3. Zonal stats clipped to paddock geometry
    paddock_geom = farmos.get_asset_geom(asset_id)
    stats = rasterstats.zonal_stats(paddock_geom, 'ndvi.tif',
        stats=['mean','min','max','std','count'],
        add_stats={'pct_stressed': lambda a: float((a < 0.3).sum())/a.count()})

    # 4. Write farmOS observation log
    farmos.post_log({
        'type': 'log--observation',
        'attributes': {
            'name': f'Drone NDVI {payload["date"]}',
            'timestamp': payload['date'],
            'quantity': [
                {'measure':'ratio','value':stats[0]['mean'],'units':'NDVI','label':'mean'},
                {'measure':'ratio','value':stats[0]['std'], 'units':'NDVI','label':'stddev'},
                {'measure':'ratio','value':stats[0]['pct_stressed'],'units':'fraction','label':'stressed'},
            ]
        },
        'relationships': {'asset':{'data':[{'type':'asset--land','id':asset_id}]}}
    })
```

In Rust (herd-scout's stack): substitute `gdal` crate + `geo`/`geo-types` for polygon clipping; zonal stats hand-rollable in <100 LOC over `gdal::Dataset` (rasterize polygon to mask, masked-mean over band).

## Edge cases

- **Paddock crosses tile boundary** — rasterstats handles automatically (window read via rasterio); pre-tiled COG even cheaper.
- **Cloud occlusion** — no clean OSS solution. Mitigations: drop high-blue pixels (cloud proxy); flag log with `confidence` quantity if cloud-mask coverage > 5%; require multispectral + shadow detection in micasense scripts.
- **Missed bands / panel image missing** — ODM still produces ortho but radiometric calibration invalid. Detect via "panel not found" in ODM log; refuse to compute NDVI; write log with note instead of quantities.
- **Different flight angle / sun angle** — BRDF effects break cross-time NDVI comparison. Standardize flight time (within 2h of solar noon); record solar elevation in log `data`.
- **Inconsistent CRS** — paddock usually EPSG:4326, ortho EPSG:3857 / local UTM. **Always reproject paddock to ortho CRS** before zonal stats. Bugs occur when CRS metadata is missing.
- **Resolution mismatch** — 5cm/px ortho × ranch-scale paddock = millions of pixels. Acceptable; for huge fields, downsample (`gdalwarp -tr 0.5 0.5`) — 100x speedup, no signal loss for mean/stddev.
- **Geo-registration drift** between flights — paddock polygon static but ortho georeg shifts ~1m without GCPs. OK for paddock stats; breaks sub-paddock change detection.
