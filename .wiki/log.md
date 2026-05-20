# Herd Scout Project Wiki

## [2026-05-19] init | Created local wiki for herd_scout project

## [2026-05-19] compile | Imported from external drone-vision wiki

Imported articles:
- [[drone-vision-software]] - Computer vision software research
- [[drone-hardware]] - Open source drone hardware options
- [[implementation-plan]] - Phone-based herd counter build plan

## [2026-05-20] research | "OSS mobile-to-desktop farm management + drone integrations" → 17 sources, 14 articles, 2 rounds

Round 1 — 8 parallel deep-mode agents covered: OSS FMS landscape, FMS feature taxonomy, mobile↔desktop architecture, livestock OSS, drone-FMS pipeline, Android-on-drone, ag data standards, precision-ag drone use cases. 14 sources ingested, 12 wiki articles compiled.

Round 2 — 3 focused agents on top gaps from Round 1: EID reader BLE/SPP wire protocols, iroh-smol-kv schema for FMS data, WebODM→FMS ingestion bridge. 3 sources ingested, 2 wiki articles compiled, 2 enriched. Note: agents lacked WebFetch access — content is high-confidence prior-knowledge synthesis but flagged for primary-source verification.

Output: [[output/playbook-herd-scout-2026-05-20]] — strategic playbook with phased build plan.

## [2026-05-20] plan | "mobile-to-desktop iroh app: desktop driver + phone-on-drone camera" → output/plan-mobile-to-desktop-iroh-rfc-2026-05-20.md (13 articles consulted, 7 architecture decisions, 6 phases)

Generated RFC-format implementation plan grounded in the wiki research. MVP framing: phone is a streaming camera only (no on-device ML, no MAVLink, Android only), desktop runs all inference. Critical pre-work surfaced: `vendor/iroh-live/` is referenced in `Cargo.toml` but not present on disk; `desktop/src/main.rs` is a Hello-world stub. Phase 0 of the plan resolves this before any feature work.

Key findings:
- farmOS dominates OSS FMS; livestock-specific layer is dramatically thin
- L5 (FMS structured ingest of drone outputs) is broken across all OSS FMS — concrete bridge documented
- Strap-android-on-drone: yes as ML+4G companion, no as flight controller; DroneKit-Android is dead
- This repo declares `iroh-smol-kv` not `iroh-docs` — schema patterns updated accordingly
- Defensible wedges: Rust-native, P2P/offline-first, drone-vision livestock integration, native mobile, OSS EID reader bridge