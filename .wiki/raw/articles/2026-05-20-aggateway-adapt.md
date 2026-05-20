---
title: "AgGateway ADAPT — open-source ag data interop framework"
source_url: https://aggateway.org
secondary_urls:
  - https://github.com/AgGateway/ADAPT
  - https://github.com/AgGateway
type: project
tags: [aggateway, adapt, isobus, isoxml, modus, standards, interop, agricultural-data]
created: 2026-05-20
confidence: high
---

# AgGateway ADAPT and ag data standards

- ADAPT license: EPL-1.0
- Stack: .NET (C#)
- Status: v3.1.0 March 2024, 84 stars, 30 open issues — moderate but active

## What ADAPT is

A neutral object model translating between proprietary OEM formats (John Deere, Climate FieldView, AGCO, Trimble, CNH, Raven) and an interoperable representation. Plugins exist in separate repos.

## Key standards covered (or related)

| Standard | Status | Adopt? |
|---|---|---|
| **ISOBUS / ISO 11783** | Universal on tractors. Paywalled spec; AEF runs conformance. **ISOXML** is the file-based form (TASKDATA.XML on USB). | Yes — must read/write |
| **DDIs (Data Dictionary Identifiers)** | Free registry at isobus.net | Yes |
| **GeoJSON** | RFC 7946, universal | Yes — primary boundary format |
| **Shapefile** | Legacy ESRI; still dominant in USDA/NRCS exports | Read-only |
| **GeoPackage (GPKG)** | OGC SQLite-based, replaces shapefile | Yes |
| **GeoTIFF / COG** | Cloud-optimized GeoTIFF — Google/USGS/NASA serve data this way | Yes — primary imagery format |
| **STAC (SpatioTemporal Asset Catalog)** | Open spec for cataloging imagery + COG | Yes if doing imagery |
| **EPPO codes** | Free, open data, 98,500+ species | Yes — canonical pest/crop ID |
| **AGROVOC** | FAO multilingual SKOS, CC-BY-4.0 | Yes for tagging/i18n |
| **Modus (AgGateway)** | Lab soil/tissue test results, JSON, active 2026 | Yes for agronomy features |
| **ISO 11784/11785 RFID** | LF 134.2 kHz HDX/FDX-B for livestock | Yes for livestock |
| **USDA 840 / NLIS / CCIA / EID** | National livestock ID systems atop ISO 11784 | Yes per market |
| **GS1 / EPCIS** | Downstream traceability — buyer-driven | Skip until forced |
| **EU INSPIRE / GML** | EU CAP mandate | Skip if non-EU |
| **AgriRouter** | EU multi-vendor message bus | EU only |
| **AgStack Asset Registry** | Linux Foundation, 2021+; open field-boundary IDs | Watch but don't bet yet |

## APIs / message buses

**No accepted REST/GraphQL standard exists.** Each vendor rolls its own (JD Operations Center, Climate FieldView, Trimble Ag, AGCO Fuse, CNHi). ADAPT is a *file/object* standard, not API. AgriRouter is the closest neutral message bus (EU). OGC API - Features is the closest geospatial neutral.

## Practical takeaway for herd-scout (OSS, livestock-focused, US/global)

**Must support**: GeoJSON (paddocks), Shapefile read (NRCS imports), GeoTIFF/COG (drone imagery), EPPO codes (pest/forage), ISO 11784/11785 (EID tags).

**Should support**: ISOXML import/export (if any tractor integration), Modus JSON (lab results), STAC (imagery catalog), USDA 840 export.

**Skip until forced**: EPCIS, INSPIRE GML, full GS1 produce traceability, AgriRouter (US scope).
