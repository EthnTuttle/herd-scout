---
title: "Ag data standards — what to adopt, what to skip"
tags: [standards, isobus, isoxml, adapt, geojson, geotiff, eppo, modus, eid, gs1]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# Ag data standards

Pragmatic guide for an OSS livestock-focused FMS. Theory vs practice.

## The pragmatic shortlist for herd-scout

**Must support**:

- **GeoJSON** (RFC 7946) — paddock/field boundaries, primary
- **Shapefile** (read-only) — NRCS exports legacy
- **GeoTIFF / COG** — drone imagery; cloud-optimized for HTTP range requests
- **EPPO codes** — pest/forage taxonomy (free, open data, 98,500+ species)
- **ISO 11784/11785** — livestock RFID/EID (see [[livestock-eid-rfid]])

**Should support**:

- **ISOXML** (ISO 11783 task data) — if any tractor integration is in scope
- **Modus** (AgGateway) — lab soil/tissue test results in JSON; labs increasingly emit it
- **STAC** — imagery catalog, paired with COG
- **AGROVOC** (FAO multilingual SKOS, CC-BY-4.0) — i18n tagging
- **USDA 840** export — US livestock compliance

**Skip until forced**:

- **EPCIS / GS1** — buyer-driven traceability (Walmart, FSMA 204); skip until a buyer demands
- **EU INSPIRE / GML** — EU CAP mandate; skip if non-EU
- **AgriRouter** — EU multi-vendor message bus

**Watch but don't bet**:

- **AgStack Asset Registry** (Linux Foundation, 2021+) — open field-boundary IDs
- **TIM** (Tractor Implement Management, ISOBUS extension)
- **AgGateway TraceabilityAPI**

## Key standards summary

### ISO 11783 / ISOBUS
- CAN-bus (SAE J1939) on tractors. 14-part standard.
- Governed by ISO; **conformance run by AEF (Agricultural Industry Electronics Foundation)** with PlugFest events
- **ISOXML** = file form (TASKDATA.XML on USB stick)
- **DDIs** (Data Dictionary Identifiers) — free registry at isobus.net
- **TIM** — newer extension allowing implements to command tractors
- Full spec paywalled (~$200/part × 14); AEF + isoxml.dev community has reverse-engineered docs

### AgGateway ADAPT
- EPL-1.0, .NET (C#); v3.1.0 March 2024
- Translates between proprietary OEM formats (JD, Climate, AGCO, Trimble, CNH, Raven) and a neutral object model
- Plugins in separate repos
- Real adoption: ag retailers and FMIS vendors, not farmers directly

### Modus (AgGateway)
- Lab soil/tissue/plant test results, JSON, **actively updated 2026**
- Labs increasingly emit Modus JSON
- High value for any agronomy feature

### EPPO codes
- Free, open data license, 98,500+ species (plants, pests, microorganisms)
- Stable across taxonomy revisions
- Use as canonical pest/crop identifier

### Geospatial
- **GeoJSON** (RFC 7946) — universal modern field-boundary format
- **Shapefile** — legacy; 2GB limit, terrible encoding, but USDA/NRCS still emit it
- **GeoPackage** (OGC SQLite-based) — replaces shapefile
- **KML** — Google Earth legacy; common in drone/scout apps
- **FlatGeobuf** — binary, streamable

### Imagery
- **GeoTIFF** — baseline
- **COG (Cloud-Optimized GeoTIFF)** — TIFF superset organized for HTTP range requests; USGS, NASA, Google, DigitalGlobe all use it. Default for cloud-native imagery
- **STAC** — catalog spec, paired with COG

### Livestock identification
- See [[livestock-eid-rfid]] for ISO 11784/11785 + country systems

### APIs / message buses
- **No accepted REST/GraphQL standard** for FMS APIs exists. Each vendor rolls own (JD Operations Center, Climate FieldView, Trimble, AGCO, CNH).
- ADAPT is *file/object*, not API.
- AgriRouter (EU) is closest neutral message bus.
- OGC API - Features is closest neutral geospatial API.

## Real adoption — theory vs practice

| Standard | Theory | Practice for small farmers |
|---|---|---|
| ISOBUS / ISOXML | Mandatory | Yes via USB on tractor — real |
| ADAPT | "Universal translator" | Used by FMIS vendors — invisible to farmers |
| GeoJSON / Shapefile | Open | **Yes — every app needs both** |
| COG + STAC | Cloud-native imagery | **Yes — adopt for any imagery feature** |
| GS1 / EPCIS | End-to-end traceability | **Only when buyer demands**; skip until forced |
| ISO 11784/85 RFID | Livestock | Real where regulated; otherwise eartag visual |
| EPPO / AGROVOC | Taxonomy | Cheap to adopt, high value, **do it** |
| Modus (lab) | Soil/tissue | Real; labs emit Modus JSON. **Adopt** |
| AgriRouter | Neutral bus | EU only |
| AgStack Asset Registry | Open field IDs | Watch but don't bet yet |
| INSPIRE / GML | EU mandate | Only matters for EU CAP tools |
| Climate FieldView / JD APIs | "Open" | Partner agreements, gated |

## Dead / zombie

- Pure shapefile-only workflows (still need to read)
- ISO XML pre-version-4 dialects
- MIMOSA OSA-EAI in ag (was hyped, near-dormant)

## See also
- [[fms-data-model]]
- [[livestock-eid-rfid]]
- [[oss-drone-fms-pipeline]]
- [[opendronemap]]

## Sources
- raw: [[2026-05-20-aggateway-adapt]]
