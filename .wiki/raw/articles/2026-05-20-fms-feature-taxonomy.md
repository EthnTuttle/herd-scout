---
title: "FMS feature taxonomy — what a real farm management system needs"
source_url: https://farmos.org/model/
secondary_urls:
  - https://en.wikipedia.org/wiki/Precision_agriculture
  - https://en.wikipedia.org/wiki/Animal_identification
  - https://www.agriwebb.com/features
type: synthesis
tags: [fms, features, taxonomy, mvp, requirements, livestock, scoping]
created: 2026-05-20
confidence: high
---

# FMS feature taxonomy

## 1. Core records (Assets)

- **Must**: Land/fields, animals, equipment, groups/herds, plants/crops, structures
- **Nice**: Materials, products, water, sensors, compost
- **Advanced**: Asset hierarchies, parent/child for breeding lineage
- **Reference**: farmOS asset types — copy this design

## 2. Operations / activities (Logs)

- **Must**: Activity, Observation, Input application (lot#, method), Harvest, Seeding, Maintenance, Medical/vet (with vet field)
- **Nice**: Lab test, Purchase, Sale, Birth, Weaning, Movement
- **Advanced**: Custom log types per module
- **Note**: EU regs require on-farm treatment logs — Medical is non-optional for livestock

## 3. Inventory

- **Must**: quantities tied to logs (not standalone counts)
- **Nice**: Seed lots, chemical lot/expiry, parts, feed, headcounts per group
- **Advanced**: Re-order alerts, multi-location, cost rollups
- **Pattern**: derive inventory from log increments/decrements (farmOS does this — copy)

## 4. Financial

- **Must**: Per-field/per-herd cost tracking
- **Nice**: Profit per acre / per animal, labor costs, equipment-hour allocation
- **Advanced**: Invoicing, payroll, cash-flow, grain marketing
- **Status**: farmOS punts to QuickBooks; Ekylibre handles natively. Real OSS gap.

## 5. Compliance / traceability

- **Must**: Pesticide records (date/product/rate/applicator/weather/REI/PHI), animal IDs, medical with withdrawal
- **Nice**: OMRI tracking, audit reports, animal movement history, EID/RFID (USDA 840, ISO 11784/11785)
- **Advanced**: NLIS / NAIT / BCMS / EU bovine passport / export certs
- **Reference**: farmOS handles core; advanced national integrations are a gap

## 6. Spatial / mapping

- **Must**: Field/paddock polygons (GeoJSON), point locations, map nav
- **Nice**: Soil overlays, scouting points, GPS track lines
- **Advanced**: Prescription maps, zone management, elevation/contour, slope/aspect for grazing models

## 7. Sensor / IoT

- **Must**: Sensor as asset type; ingest endpoint (HTTP/MQTT)
- **Nice**: Weather, soil moisture, water tank, gates, GPS collars, rumen boluses
- **Advanced**: Threshold alerts, multi-source fusion, edge-device offline buffering

## 8. Drone / imagery

- **Must**: Photo attachments to logs/assets with geotag preserved
- **Nice**: Ortho upload as map layer, NDVI tiles, time-series imagery
- **Advanced**: Auto crop-health alerts from NDVI deltas, livestock counting from CV, volumetric measurement (silage), DEMs
- **Status**: almost no OSS FMIS handles imagery well — **real gap, credible herd-scout differentiator**

## 9. Reporting / analytics

- **Must**: Dashboard, CSV export
- **Nice**: Per-field yield, input cost per acre, weight gain over time, grazing days
- **Advanced**: Predictive yield, forage forecasting, prescription maps, ML decision support, NRCS COMET-Farm carbon

## 10. Mobile-specific

- **Must**: Offline data entry → background sync, geotagged photo capture, quick-form for high-frequency logs
- **Nice**: Voice notes, barcode/QR/EID scan, GPS auto-fill for scouting
- **Advanced**: Bluetooth EID readers, weigh scales, chute controllers; offline map tiles full-farm; background location

## MVP for herd-scout

1. **Records**: Animal, Group, Land (paddock), Equipment
2. **Logs**: Observation, Medical, Movement, Weight, Birth
3. **Mobile-first**: offline scouting, geotag photo, EID scan lookup
4. **Mapping**: paddock polygons, animal-last-seen pins
5. **Compliance**: treatment log w/ withdrawal flag, animal-ID export

**Defer**: financial (export to QuickBooks); grain marketing; full inventory; orthos (single geotagged image is enough); IoT (start one sensor type, e.g., water tank).

## Architectural lesson from farmOS

The Asset + Log + Quantity + Term + Plan five-primitive model is extensible enough that nearly every feature above maps to it without schema changes. **A new Rust/Tauri FMIS should adopt this same data model (or stay JSON:API compatible with farmOS) so existing data ports cleanly.**
