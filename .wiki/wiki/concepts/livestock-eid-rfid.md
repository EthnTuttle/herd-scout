---
title: "Livestock EID / RFID — standards, hardware, OSS gap"
tags: [eid, rfid, iso-11784, iso-11785, allflex, tru-test, livestock, identification]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# Livestock EID / RFID

## Standards

- **ISO 11784** — code structure (15 digits: country code + national ID)
- **ISO 11785** — RFID transmission protocol, **134.2 kHz LF**, **FDX-B** and **HDX** modes
- Implanted/eartag transponders. Universally adopted internationally for cattle and pets.

### Country systems built on ISO 11784

| System | Country | Status |
|---|---|---|
| **USDA 840** | US | Voluntary; becoming mandatory for interstate cattle 2024-2025 rule |
| **NLIS** | Australia | Mandatory cattle/sheep |
| **CCIA** | Canada | Mandatory cattle |
| **EID** (UK/EU sheep) | UK / EU | Mandatory sheep |

## Hardware (closed-source, vendor-owned)

- **Allflex** — stick readers, panel readers; Bluetooth SPP / serial output
- **Tru-Test / Datamars** — XR/ID-series indicators with Bluetooth + weigh-scale integration
- **Gallagher** — readers + weigh scales
- **SCR Heatime** — accelerometer collars (BovHEAT works because SCR exports XLSX)

All ship proprietary closed apps for tag management. Some emit Bluetooth SPP / serial at the wire level.

## ICAR

**ICAR (International Committee for Animal Recording)** — guidelines for milk recording, performance data; ADE (Animal Data Exchange) format.

## OSS gap

GitHub search "ISO 11784 RFID livestock" returns **zero repositories**. There is **no OSS library that**:

- Parses 15-digit ISO 11784 EIDs
- Reads Bluetooth SPP / serial output from common stick readers
- Submits to NLIS / CCIA / USDA 840 reporting endpoints
- Bridges reader output → farmOS log

farmOS users currently coerce reader CSV exports into farmOS via custom Drupal modules. None ship out of the box.

## Why this matters for herd-scout

A Bluetooth-EID-reader library in Rust + Tauri Mobile, with offline buffering and sync via [[iroh-sync-stack]], would be:

- Genuinely novel (no OSS competition)
- Aligned with herd-scout's livestock focus
- Tractable (the wire protocol is documented; ISO codes are 15 digits of ASCII)
- High leverage (every commercial-tag rancher could use it)

## Implementation sketch

```
Phone (Tauri 2 + Rust core)
  └─ Bluetooth (BLE / RFCOMM SPP) to Allflex/Tru-Test stick reader
     └─ Parse ISO 11784/11785 frames
        └─ Lookup or create Animal asset (iroh-smol-kv)
           └─ Append Observation log with weight (if scale connected)
              └─ Sync to other peers via iroh
```

## Wire-protocol details (medium confidence — verify before crate design)

### What readers actually emit

Most stick readers emit the **15-digit ISO 11784 decimal as ASCII over Bluetooth SPP at 9600 8N1** (some 38400) terminated by `\r`, `\n`, or `\r\n`. Optional 1-char prefix for FDX-B vs HDX (`A` vs `H`, or `_`). Optional comma-separated trailing flag bytes. Some Agrident firmware adds a 2-char hex BCC checksum.

Single line-buffered parser handles 70-80% of field hardware:

```
^[A_H]?\s?(\d{3})\s?(\d{12})(?:[,\s].*)?$
```

### Per-vendor matrix

| Vendor / Model | Connection | Frame | Openness |
|---|---|---|---|
| **Agrident APR/AWR** | BLE + SPP + USB | ASCII; **public protocol PDF**; OEM for many rebadged readers | Best documented |
| **Allflex RS420 / LPR / AWR300** | SPP + USB CDC; some BLE | ASCII line `A 982 000123456789\r\n` | Mostly **rebadged Agrident** post MSD/Merck; integrator PDF on request |
| **Tru-Test / Datamars XRS2 / SRS2** | SPP + USB serial (FTDI) | ASCII; some firmware emits CSV w/ timestamp | "Data Link" SDK partly available |
| **Tru-Test XR5000 / ID5000 indicator** | RS-232 + SPP | Binary STX/ETX/LRC + CSV export | Mostly closed |
| **Datamars Z-Tags / GES3S** | SPP (older) / BLE Nordic-UART (newer) | ASCII over both | Closed; some BLE UUIDs reverse-engineered |
| **Gallagher HR5 / TSi / TWR-5** | SPP + USB | ASCII line `LA 982 000123456789` | Integrator protocol on request (NDA varies) |

### ISO 11784/11785 air-frame summary

64 bits = 1 animal-flag + 14 reserved + 1 data-block flag + 10-bit country/manufacturer + 38-bit national ID. Two air protocols:
- **FDX-B** (134.2 kHz Manchester, full-duplex, continuous) — cattle eartags, pet microchips
- **HDX** (FSK, transponder transmits during reader off-period) — harsh/metal environments

Reader presents to host as 15-digit decimal: 3 digits country/manufacturer + 12 digits national ID.

## Verdict — feasibility of `herd-scout-eid` Rust crate

**Weekend MVP** (high confidence):
- `Reader` trait with three transports: `serialport` (USB CDC), `bluer` (BlueZ SPP), `btleplug` (cross-platform BLE)
- Line-buffered parser, optional FDX-B/HDX prefix, optional terminators
- `EidTag { country: u16, national_id: u64, protocol: FdxB | Hdx | Unknown, raw: String }`
- Demo CLI: pair, print tags, JSON-line output

Covers Allflex/Agrident/Tru-Test SPP and most Nordic-UART BLE readers — ~70-80% of deployed sticks.

**Multi-week / hardware-required**:
- Tru-Test XR5000/ID5000 binary protocol — needs real device + capture
- Bidirectional commands (battery, mode, session log) — every vendor differs
- Datamars BLE GATT — needs nRF Connect captures
- ICAR certification as "data collector" — months, formal

## Concrete next steps

1. **Acquire**: used **Allflex RS420** ($150-300), then **Datamars XRS2** ($800) for BLE testing. Optional: **Agrident APR500** for cleanest documented protocol.
2. **Get Agrident "ASCII Protocol" PDF** — request from sales@agrident.com. Often distributed without NDA.
3. **Capture, don't trust docs** — pair Android with "Serial Bluetooth Terminal", scan known FDX-B and HDX tags, capture exact bytes. Commit captures as fixtures.
4. **Chase ICAR device list** at icar.org — comprehensive vendor/model matrix.
5. **Forum mining** for reverse-engineered detail — Arduino forums, r/cattle, ranchersnet.

## See also
- [[livestock-oss-gap-analysis]]
- [[iroh-sync-stack]]
- [[ag-data-standards]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-aggateway-adapt]]
- raw: [[2026-05-20-fms-feature-taxonomy]]
- raw: [[2026-05-20-eid-reader-protocols]]
