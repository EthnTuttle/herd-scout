---
title: "Android Thermal API — getThermalHeadroom + ThermalStatusListener"
source: https://developer.android.com/games/optimize/adpf/thermal
secondary: https://source.android.com/docs/core/power/thermal-mitigation
type: article
tags: [android, thermal, throttle, api, sustained-performance]
ingested: 2026-06-01
quality: 5
confidence: high
---

# Android Thermal API — canonical Google docs

## Severity levels (`getCurrentThermalStatus`)

7 levels:
1. `THERMAL_STATUS_NONE`
2. `THERMAL_STATUS_LIGHT`
3. `THERMAL_STATUS_MODERATE`
4. `THERMAL_STATUS_SEVERE`
5. `THERMAL_STATUS_CRITICAL`
6. `THERMAL_STATUS_EMERGENCY` — **disables modem/cellular** (LTE upload dies before camera does)
7. `THERMAL_STATUS_SHUTDOWN`

## getThermalHeadroom (API 30+)

- Returns 0.0–1.0 forecast
- `> 0.85` light throttle, `> 0.95` moderate, `> 1.0` severe
- **Hard rate limit**: do not call more than once per 10 s (returns NaN if violated).

## Listener API

- `addThermalStatusListener(...)` fires callbacks on transitions.
- Drop-in for herd-scout to **dynamically lower bitrate / FPS / drop network upload** on transitions.

## Recommended app-side mitigations (per Google)

- Lower frame rate
- Lower resolution
- Defer network work
- Shift to small cores

## Implications for herd-scout

- Wire `addThermalStatusListener` into the publisher's encoder pipeline.
- **Ladder behavior**:
  - `MODERATE` → drop bitrate 4 → 2 Mbps
  - `SEVERE` → drop FPS 30 → 15 + resolution 720p → 480p
  - `CRITICAL` → stop upload, signal daemon "publisher backed off"
  - `EMERGENCY` already dropped LTE; phone is in survival mode
- Implication for transport choice: **WiFi ground-station beats LTE direct upload** under thermal pressure (LTE dies first).
