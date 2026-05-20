---
title: "FMS feature taxonomy — must / nice / advanced"
tags: [fms, features, taxonomy, mvp, scoping]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# FMS feature taxonomy

Ten feature buckets with must / nice / advanced tiers. Use to scope MVP and roadmap.

## 1. Core records (Assets)
- **Must**: Land, animals, equipment, groups/herds, plants/crops, structures
- **Nice**: Materials, products, water, sensors, compost
- **Advanced**: Asset hierarchies; parent/child for breeding lineage
- **Reference**: [[fms-data-model]]

## 2. Operations / activities (Logs)
- **Must**: Activity, Observation, Input, Harvest, Seeding, Maintenance, Medical
- **Nice**: Lab test, Purchase, Sale, Birth, Weaning, Movement
- **Advanced**: Custom log types per module
- EU regs require on-farm treatment logs — Medical is non-optional for livestock.

## 3. Inventory
- **Must**: quantities tied to logs (not standalone counts)
- **Nice**: Seed lots, chemical lot/expiry, parts, feed, headcounts per group
- **Advanced**: Re-order alerts, multi-location, cost rollups
- **Pattern**: derive from log increments/decrements; copy farmOS

## 4. Financial
- **Must**: Per-field/per-herd cost tracking
- **Nice**: Profit per acre / per animal, labor, equipment-hour allocation
- **Advanced**: Invoicing, payroll, cash-flow, grain marketing
- **Status**: farmOS punts to QuickBooks; Ekylibre handles natively. **Real OSS gap.**

## 5. Compliance / traceability
- **Must**: Pesticide records, animal IDs, medical with withdrawal
- **Nice**: OMRI, audit reports, animal movement history, EID/RFID (USDA 840, ISO 11784/11785)
- **Advanced**: NLIS / NAIT / BCMS / EU bovine passport / export certs
- See [[livestock-eid-rfid]] and [[ag-data-standards]]

## 6. Spatial / mapping
- **Must**: Field/paddock polygons (GeoJSON), point locations, map nav
- **Nice**: Soil overlays, scouting points, GPS track lines
- **Advanced**: Prescription maps, zone management, elevation, slope/aspect for grazing models

## 7. Sensor / IoT
- **Must**: Sensor as asset; ingest endpoint (HTTP/MQTT)
- **Nice**: Weather, soil moisture, water tank, gates, GPS collars, rumen boluses
- **Advanced**: Threshold alerts, multi-source fusion, edge-device offline buffering

## 8. Drone / imagery
- **Must**: Geotagged photo attachments to logs/assets
- **Nice**: Ortho upload as map layer, NDVI tiles, time-series imagery
- **Advanced**: Auto crop-health alerts from NDVI deltas, livestock counting from CV (see [[drone-vision-software]]), volumetric measurement, DEMs
- **Status**: almost no OSS FMIS handles imagery well — **real differentiator for herd-scout**. See [[oss-drone-fms-pipeline]].

## 9. Reporting / analytics
- **Must**: Dashboard, CSV export
- **Nice**: Per-field yield, input cost per acre, weight gain over time, grazing days
- **Advanced**: Predictive yield, forage forecasting, prescription maps, ML decision support, NRCS COMET-Farm carbon

## 10. Mobile-specific
- **Must**: Offline data entry → background sync, geotagged photo, quick-form for high-frequency logs
- **Nice**: Voice notes, barcode/QR/EID scan, GPS auto-fill
- **Advanced**: Bluetooth EID readers, weigh scales, chute controllers; offline map tiles full-farm
- See [[mobile-desktop-architecture]]

## MVP for herd-scout (livestock-focused)

1. **Records**: Animal, Group, Land (paddock), Equipment
2. **Logs**: Observation, Medical, Movement, Weight, Birth
3. **Mobile-first**: offline scouting, geotag photo, EID scan lookup
4. **Mapping**: paddock polygons, animal-last-seen pins
5. **Compliance**: treatment log w/ withdrawal flag, animal-ID export

**Defer**: financial (export to QuickBooks); grain marketing; full inventory; orthos beyond single geotagged photos; IoT (start one sensor type, e.g., water tank).

## See also
- [[oss-fms-landscape]]
- [[fms-data-model]]
- [[livestock-oss-gap-analysis]]
- [[oss-drone-fms-pipeline]]
- [[mobile-desktop-architecture]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-fms-feature-taxonomy]]
- raw: [[2026-05-20-farmos]]
