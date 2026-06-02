---
title: "Feature: herd-scout-eid Rust crate (Bluetooth ISO 11784/11785)"
type: feature-candidate
priority: p0
created: 2026-06-02
source: assess-herd-scout-2026-06-02
status: open
estimate: weekend MVP for ~70-80% reader coverage
wiki_evidence:
  - concepts/livestock-eid-rfid
  - concepts/livestock-oss-gap-analysis
  - concepts/herd-scout-positioning
---

# Feature: herd-scout-eid Rust crate

## Why P0

GitHub search for "ISO 11784 RFID livestock" returns **zero repositories** ([[../../wiki/concepts/livestock-eid-rfid]]). No OSS library reads Bluetooth/serial output from common stick readers (Allflex, Tru-Test, Datamars, Gallagher) and parses ISO 11784/11785 frames. This is the strongest unique wedge per the positioning article: *genuinely novel, no OSS competition, tractable scope, high leverage* — every commercial-tag rancher could use it. USDA APHIS 840 EID rule has been in force since 2024-11-05 for cattle/bison ages 18 mo+ crossing state lines, creating real paying-customer demand.

## Scope (weekend MVP per the wiki)

`Reader` trait with three transports:

- `serialport` — USB CDC (Allflex RS420, Agrident APR/AWR, Tru-Test SRS2)
- `bluer` — BlueZ SPP on Linux
- `btleplug` — cross-platform BLE Nordic-UART

Line-buffered parser, optional FDX-B/HDX prefix, optional terminators:

```regex
^[A_H]?\s?(\d{3})\s?(\d{12})(?:[,\s].*)?$
```

Output type:

```rust
EidTag {
    country: u16,
    national_id: u64,
    protocol: FdxB | Hdx | Unknown,
    raw: String,
}
```

Demo CLI: pair, print tags, JSON-line output.

## Coverage estimate

~70-80% of deployed sticks: Allflex/Agrident SPP, Tru-Test/Datamars XRS2/SRS2, most Nordic-UART BLE readers. Multi-week / hardware-required work (Tru-Test XR5000/ID5000 binary, Datamars BLE GATT, bidirectional commands, ICAR certification) is explicitly out of scope.

## Hardware to acquire

1. Used **Allflex RS420** ($150-300)
2. **Datamars XRS2** ($800) for BLE testing
3. Optional: **Agrident APR500** for cleanest documented protocol

## Action items before code

1. Request Agrident "ASCII Protocol" PDF from sales@agrident.com (often distributed without NDA).
2. Pair Android with "Serial Bluetooth Terminal", scan known FDX-B and HDX tags, capture exact bytes. Commit captures as fixtures.
3. Pull ICAR device list at icar.org for vendor/model matrix.
4. Forum mining: Arduino forums, r/cattle, ranchersnet for reverse-engineered detail.

## Integration shape

Once the crate exists, wire it into the daemon's iroh-smol-kv schema (see [[iroh-smol-kv-fms-schema]]) so EID scans:
1. Create or lookup an `Animal` asset by `(country, national_id)`.
2. Append an `Observation` log with optional weight (if scale connected).
3. Become marks for layer-5 EID reconciliation in [[../../wiki/concepts/herd-counting-pipeline]] — the herd-scout-unique wedge.

## See also
- [[../../wiki/concepts/livestock-eid-rfid]]
- [[../../wiki/concepts/livestock-oss-gap-analysis]]
- [[../../wiki/concepts/herd-scout-positioning]]
- [[../../output/assess-herd-scout-2026-06-02]] §Opportunities
