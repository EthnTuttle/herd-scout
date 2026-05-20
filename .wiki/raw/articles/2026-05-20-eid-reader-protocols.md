---
title: "Livestock EID stick reader wire protocols (BLE/SPP/serial)"
source_url: https://www.icar.org/index.php/certifications-icar-certifications/devices-with-icar-certificate
type: synthesis
tags: [eid, rfid, allflex, agrident, tru-test, gallagher, datamars, bluetooth, spp, ble, iso-11784]
created: 2026-05-20
confidence: medium
caveats: |
  Round 2 agent operated without WebFetch — content is prior-knowledge
  synthesis to be verified against vendor docs and live captures before
  committing to crate design. Treat as hypothesis, not citable evidence.
---

# EID stick reader wire protocols

## ISO 11784 / 11785 (high confidence)

The 64-bit air-interface frame is decoded by the reader and presented to host as a **15-digit decimal string**:

- **3 digits** = country code (ISO 3166 numeric) OR manufacturer code
- **12 digits** = national ID (max ~2³⁸)

Air-frame layout: 64 bits = 1 animal-flag + 14 reserved + 1 data-block flag + 10-bit country/manufacturer + 38-bit national ID.

Two air protocols (ISO 11785):
- **FDX-B** (Full Duplex, 134.2 kHz, Manchester-coded) — common for cattle eartags, pet microchips
- **HDX** (Half Duplex, FSK, transponder transmits during reader off-period) — harsh/metal environments; TI RI-TRP series, Allflex HDX cattle tags

## What readers emit (medium confidence — verify per vendor)

Mostly **just the 15-digit decimal**, sometimes with:
- 1-char prefix for FDX-B vs HDX (`A` vs `H`, or `_` for unknown)
- Optional CR / LF / CRLF terminator
- Occasional comma-separated trailing flag (animal/data-block bits)
- Optional 2-char hex BCC checksum (Agrident family)

## Per-vendor matrix (medium confidence)

| Vendor / Model | Connection | Frame format | Example | Openness |
|---|---|---|---|---|
| **Allflex RS420 / LPR / AWR300** | Bluetooth SPP + USB CDC; some newer BLE | ASCII line-based, CR/LF | `A 982 000123456789\r\n` | Vendor "Communication Protocol" PDF distributed to integrators on request; reverse-engineered. Mostly **rebadged Agrident** post MSD/Merck acquisition. |
| **Tru-Test / Datamars XRS2 / SRS2** | Bluetooth SPP + USB serial (FTDI) | ASCII line-based | `0982000123456789\r\n` (some firmware emits CSV w/ timestamp) | "Data Link" SDK / serial command guide partly available |
| **Tru-Test XR5000 / ID5000** indicator | RS-232 + Bluetooth SPP | Proprietary STX/ETX framed + LRC; CSV export | Binary live link; CSV session export | Mostly closed |
| **Datamars Z-Tags / GES3S / SmartReader** | BLE (GATT) on newer; SPP on older | ASCII over SPP; Nordic-UART-style GATT | 15-digit ISO output, sometimes prefixed with reader ID | Closed; some BLE UUIDs reverse-engineered via nRF Connect |
| **Gallagher HR5 / TSi / TWR-5** | Bluetooth SPP + USB | ASCII line; "APS" protocol for indicators | `LA 982 000123456789` style | Gallagher publishes integrator protocol on request (NDA varies) |
| **Shearwell SDL440** | Bluetooth SPP | ASCII line | `0982000123456789\r\n` | Closed; minimal info |
| **Agrident APR/AWR (ICAR-certified, OEM for many)** | BLE + SPP + USB | ASCII; **public protocol description PDF** | `A982000123456789\r` plus diagnostic commands `SD?\r` | **Best-documented vendor** — protocol PDF freely circulated |

## Common patterns (high value — what the crate exploits)

- **Most stick readers emit 15-digit ASCII over Bluetooth SPP at 9600 8N1** (some 38400) terminated by `\r`, `\n`, or `\r\n`. Single line-buffered parser handles 70-80% of field hardware.
- BLE (newer) typically uses Nordic-UART-style GATT (TX char + RX char) with the same ASCII payload — line parser reusable, only transport differs.
- USB connection is CDC virtual serial (FTDI / Silicon Labs CP210x) — same ASCII stream.
- Regex `^[A_H]?\s?(\d{3})\s?(\d{12})(?:[,\s].*)?$` plus optional checksum captures Allflex/Agrident/Tru-Test/Gallagher line output.

## Public OSS / reverse-engineering (LOW confidence — verify each)

- **`pyallflex`** — Python script circulating on GitHub gist/repo for Allflex/Agrident SPP parsing. Existence plausible, not confirmed
- **OpenScales / livestock-weigh-scale Arduino projects** — hobbyist HX711+EID-reader builds; protocol confirmation via `readline()` on SPP socket
- **ICAR test certificates** at icar.org — RFID device certifications listing supported protocols per device; useful matrix
- **rfidler / Proxmark3** — FDX-B/HDX air-side decoders in C; reusable conceptually but a stick reader never exposes the air frame
- **Tracesoft, FarmIT 3000, Stockbook, Cattlemax** — commercial integrators; sometimes leak protocol detail in support docs

**No OSS Rust crates known to exist for this.**

## Verdict — feasibility of `herd-scout-eid` Rust crate

**Weekend-shippable MVP** (high confidence):
- `Reader` trait with three transports: `serialport` (USB CDC), `bluer` (Linux BlueZ SPP), `btleplug` (cross-platform BLE)
- Line-buffered parser for 15-digit ASCII, optional FDX-B/HDX prefix, optional terminators
- `EidTag { country: u16, national_id: u64, protocol: FdxB | Hdx | Unknown, raw: String }`
- Unit tests with synthesized lines per vendor
- Demo CLI: pair, print tags, JSON-line output

Covers Allflex/Agrident/Tru-Test SPP and most Nordic-UART-style BLE — ~70-80% of deployed sticks.

**Multi-week / hardware-required**:
- Tru-Test XR5000/ID5000 binary STX/ETX/LRC protocol — needs real device + capture
- Bidirectional commands (battery, mode, session log) — every vendor differs
- Datamars BLE GATT — needs nRF Connect captures from real reader
- ICAR certification as "data collector" — months, formal process

## Concrete next steps

1. **Acquire hardware** (cheapest first): used **Allflex RS420** ($150-300 used). Then **Datamars XRS2** ($800 new) for BLE testing. Optional: **Agrident APR500** for cleanest documented protocol.
2. **Get Agrident "ASCII Protocol" PDF** from sales@agrident.com — distributed without NDA in many cases. Likely covers most deployed hardware.
3. **Capture, don't trust docs** — pair Android phone with "Serial Bluetooth Terminal" to each reader, scan known FDX-B and HDX tags, capture exact byte sequences. Commit captures as test fixtures.
4. **Chase ICAR device list** at icar.org — comprehensive vendor/model matrix.
5. **Forum mining** — `site:forum.arduino.cc`, `site:reddit.com/r/cattle`, `site:ranchersnet.com` for "Allflex bluetooth", "EID reader serial".
6. **Verify in Round 3** with WebFetch to confirm OSS landscape and pin down GitHub repos before crate design.
