---
title: "WebODM → FMS ingestion bridge — concrete pipeline"
tags: [webodm, odm, ndvi, zonal-statistics, rasterstats, postgis, farmos, ingest, l5-bridge]
created: 2026-05-20
updated: 2026-05-20
confidence: medium
type: concept
---

# WebODM → FMS ingestion bridge

The killer gap in [[oss-drone-fms-pipeline]] is layer 5 — orthos and NDVI rasters never become structured per-field metrics inside an FMS without manual glue. This article spells out the bridge concretely.

## Pipeline

```
[ Mission plan ] ──> [ Flight (geotagged JPG/TIFF + panel shot) ]
                           │
                           ▼
                  [ Reflectance prep ]              (multispectral)
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
                           │  per-paddock {mean,std,p10,p90,pct_stressed,…}
                           ▼
                  [ FMS writer ]   farmOS JSON:API POST /api/log/observation
                           │       (or write directly to iroh-smol-kv per
                           │        [[iroh-docs-fms-schema]])
                           ▼
                  [ Map UI ]   COG via XYZ/WMS layer + paddock polygon
                               coloured by latest NDVI quantity
```

## Tools per step

| Step | Tool |
|---|---|
| Reflectance prep | `micasense/imageprocessing` (Python) |
| Photogrammetry | ODM / NodeODM / WebODM ([[opendronemap]]) |
| Index calc | `gdal_calc.py` / `rio-calc` |
| COG conversion | `rio cogeo` |
| Zonal stats | **`rasterstats`** (Python, MIT, MIT) |
| Spatial DB alt | PostGIS Raster |
| FMS write | farmOS JSON:API or [[iroh-docs-fms-schema]] |
| Hosting orthos | DroneDB or GeoServer |
| Polygon UI | farmOS-map |

## Zonal stats — pick `rasterstats`

- **`rasterstats`** — pip-installable, zero infra, supports `add_stats` for custom lambdas. Fine for ranch scale (≤100 paddocks × ≤1GB ortho)
- **PostGIS Raster** — overkill ops burden unless you already have Postgres
- **GEE / cloud-only** — wrong for OSS/offline

## NodeODM webhooks

`POST /task/new` accepts a `webhook` form field. NodeODM POSTs task JSON to that URL on completion. **This is the right primitive** — point it at a herd-scout endpoint that triggers the zonal-stats job.

Binding flight → paddock is **client-side**: when uploading the task, herd-scout knows the paddock UUID; stores `(task_uuid → asset_uuid)` locally. On callback, look up the asset and clip stats to its geometry.

## Metrics ranchers actually use (rangeland)

- **Mean NDVI per paddock** — primary signal; drives grazing rotation
- **Stddev NDVI** — patchiness proxy
- **Percentile 10/90** — robust to shadows/water outliers
- **Pixel count > 0.5 NDVI** → "ha of healthy forage"
- **Area below 0.3 NDVI** → bare/overgrazed acres
- **Histogram (10 bins)** — distribution shape, change detection
- **Δ vs previous flight** — most actionable signal

Don't bother with NDRE (row-crop fertilization), SAVI variants (marginal), per-pixel temporal series (collapse to paddock).

## Time-series storage (farmOS-style)

**One Observation log per (flight, paddock).** No separate "Flight" entity needed — but optionally an `Equipment`-type asset for the drone enables flight-level grouping.

```jsonc
{
  "type": "log--observation",
  "asset": [{ "type":"asset--land", "id":"<paddock-7-uuid>" }],
  "name": "Drone NDVI flight 2026-05-20",
  "quantity": [
    {"measure":"ratio","value":0.62,"units":"NDVI","label":"mean"},
    {"measure":"ratio","value":0.11,"units":"NDVI","label":"stddev"},
    {"measure":"area","value":3.4,"units":"ha","label":"stressed_area"}
  ],
  "file": [/* COG attachment or DroneDB URL */],
  "data": "{\"flight_id\":\"...\",\"odm_task_uuid\":\"...\",\"sensor\":\"P4M\"}"
}
```

For multi-paddock flights: shared `data.flight_id` UUID across the per-paddock logs; group in query layer.

## MVP — ~80 LOC

```python
def on_task_complete(payload):
    task_uuid = payload['uuid']
    asset_id  = lookup_asset_for_task(task_uuid)
    ortho     = nodeodm.download(task_uuid, 'orthophoto.tif')

    subprocess.run(['gdal_calc.py',
        '-A', ortho, '--A_band=4',
        '-B', ortho, '--B_band=3',
        '--calc=(A-B)/(A+B+1e-6)', '--outfile=ndvi.tif'])

    paddock_geom = farmos.get_asset_geom(asset_id)
    stats = rasterstats.zonal_stats(paddock_geom, 'ndvi.tif',
        stats=['mean','min','max','std','count'],
        add_stats={'pct_stressed': lambda a: float((a < 0.3).sum())/a.count()})

    farmos.post_log({
        'type': 'log--observation',
        'attributes': {
            'name': f'Drone NDVI {payload["date"]}',
            'timestamp': payload['date'],
            'quantity': [
                {'measure':'ratio','value':stats[0]['mean'],   'units':'NDVI','label':'mean'},
                {'measure':'ratio','value':stats[0]['std'],    'units':'NDVI','label':'stddev'},
                {'measure':'ratio','value':stats[0]['pct_stressed'],
                                                                'units':'fraction','label':'stressed'},
            ]
        },
        'relationships': {'asset':{'data':[{'type':'asset--land','id':asset_id}]}}
    })
```

In Rust: substitute `gdal` crate + `geo`/`geo-types`; hand-rolled zonal stats <100 LOC over `gdal::Dataset` (rasterize polygon to mask, masked-mean over band).

## Edge cases

- **Paddock crosses tile boundary** — rasterstats handles via window read
- **Cloud occlusion** — drop high-blue pixels; flag log with `confidence` quantity if cloud-mask coverage > 5%
- **Missed bands / panel image missing** — detect "panel not found" in ODM log; refuse NDVI; write log with note
- **Flight angle / sun angle** — BRDF effects; standardize within 2h of solar noon; record solar elevation in `data`
- **Inconsistent CRS** — paddock usually EPSG:4326, ortho EPSG:3857/UTM. **Always reproject paddock to ortho CRS** before zonal stats
- **Resolution mismatch** — for huge fields, downsample to 50cm/px (`gdalwarp -tr 0.5 0.5`); 100x speedup, no signal loss for mean/std
- **Geo-registration drift** — paddock polygon static but ortho georef shifts ~1m without GCPs. OK for paddock-scale; breaks sub-paddock change detection

## Prior art

- WebODM `coreplugins/dronedb` — closest existing "ODM-output to external system" plugin
- WebODM `coreplugins/objdetect` — post-processing precedent, no FMS write
- Field Mapper (newer) — advertises NDVI + prescription maps; early-stage, watch
- AgGateway ADAPT — schema layer for prescription/as-applied data, not pipeline
- farmOS forum threads — recurring requests for GeoTIFF/COG ingest; no merged module

**No shipped OSS module ties (ODM webhook → zonal stats → FMS log). Genuinely open slot.**

## See also
- [[oss-drone-fms-pipeline]]
- [[opendronemap]]
- [[fms-data-model]]
- [[iroh-docs-fms-schema]]
- [[ag-data-standards]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-webodm-fms-bridge]]
- raw: [[2026-05-20-opendronemap]]
- raw: [[2026-05-20-dronedb-micasense]]
