---
title: "Phone thermal management — Thermal API, sustained-perf mode, donor-phone choice"
summary: "Why a Pixel 6 Pro stops 4K60 outdoor recording in 3-4 min, why a Pixel 9 Pro is worse than a Pixel 7 Pro for sustained encode, and what the publisher should do about it"
tags: [phone, thermal, throttle, pixel, tensor, sustained-performance, mediacodec]
created: 2026-06-01
confidence: medium
type: concept
---

# Phone thermal management on a drone

[[android-on-drone]] claims "thermal: mostly fine in flight (slipstream cooling); ground idle is risky." Empirical 2024–2026 data **partially contradicts** this — ambient and solar load dominate over slipstream gain at typical drone speeds, especially for older Tensor chips in small chassis.

## Empirical time-to-throttle (Pixel 6 Pro, community + benchmark data)

| Workload | Time to thermal warning |
|---|---|
| 4K30 indoors | ~20–23 min |
| 4K60 indoors | ~10 min |
| **4K60 outdoors in heat** | **3–4 min** ⚠️ |
| Pixel 7 Pro 4K (vapor chamber upgrade) | >1 hour |

**The "outdoor in heat" number is the one that matters for cattle counting in a summer paddock.**

## What herd-scout actually does — 720p30, not 4K60

The wiki's "4K60 = 3–4 min outdoor" is a **pessimistic upper bound**. herd-scout's publisher captures **720p30 with hardware H.264** — much lower thermal load. Likely fine for a 20–30 min flight, especially with the mitigations below. **Verify per-airframe** rather than relying on the published 4K numbers.

## The Android Thermal API — what to wire in

[[2026-06-01-android-thermal-api|developer.android.com canonical docs]].

### `getCurrentThermalStatus` — 7 levels

`NONE → LIGHT → MODERATE → SEVERE → CRITICAL → EMERGENCY → SHUTDOWN`

`EMERGENCY` **disables modem/cellular** — meaning **LTE upload dies before camera does**. **Implication**: WiFi-tethered ground station is more thermally robust than direct LTE upload.

### `addThermalStatusListener` — the hook

Recommended ladder for the publisher:

| Level | Action |
|---|---|
| `MODERATE` | Drop bitrate 4 → 2 Mbps |
| `SEVERE` | Drop FPS 30 → 15, resolution 720p → 480p |
| `CRITICAL` | Stop upload; signal daemon "publisher backed off" |
| `EMERGENCY` | LTE already gone; camera still up; phone in survival mode |

### `getThermalHeadroom(int)` — forecast

Returns 0.0–1.0; `>0.85` light throttle imminent; `>0.95` moderate; `>1.0` severe. **Hard rate limit: do not call more than once per 10 s** (returns NaN if violated).

## Sustained Performance Mode — load-bearing

`Window.setSustainedPerformanceMode(true)` (Android 7+):

- Caps CPU/GPU at sustainable freq instead of peak.
- **Real measured trade: 18% peak lost, 2× sustained throughput** over 30 min.
- CTS guarantees `<5%` frame-rate variance.
- **Caveat**: OEMs only have to *pass* CTS; cheap MediaTek/Unisoc devices may no-op. **Pixel reference works**.

`getExclusiveCores()` reserves a CPU core for the foreground app — keep the encoder/upload thread off shared cores.

**herd-scout publisher should**:
1. Set `setSustainedPerformanceMode(true)` at startup.
2. Check `isSustainedPerformanceModeSupported()` and log when no-op.
3. Call `getExclusiveCores()` and pin the encoder thread.

## Donor-phone choice (load-bearing)

Tensor surface temps under 20-min stress:

| Phone | Temp | Notes |
|---|---|---|
| Pixel 6 Pro | 40.1 °C | Vapor chamber, larger chassis |
| Pixel 7 Pro | not tested in same set | Vapor chamber, **>1 hr 4K capable** |
| Pixel 9 Pro XL | 43.2 °C | Larger chassis newer chip |
| Pixel 9 Pro (smaller) | "considerably hotter" | Smaller chassis = worse thermals |

**Surface temps are rising across Tensor generations** despite efficiency gains.

**Recommended donor phones for herd-scout drone payloads**:
1. **Pixel 6 Pro** or **Pixel 7 Pro** — vapor chamber, large enough chassis, cheap on used market.
2. **Avoid**: Pixel 9 Pro non-XL (smaller chassis), any cheap MediaTek/Unisoc (sustained-perf-mode no-op).

## Codec choice

A/B H.264 vs HEVC on the **actual donor model** rather than assuming HEVC is more efficient. Per the codec-energy literature, **the dominant variable is hardware path vs software path** (3–4× swing) — codec selection is second-order. **Confirm MediaCodec uses the vendor codec, not OMX.google.*** SW fallback.

## Implications summary

- **Verify per-airframe at 720p30**; don't trust the 4K60 community numbers.
- Wire `addThermalStatusListener` + the 4-rung ladder above.
- Set `setSustainedPerformanceMode(true)` at publisher startup.
- **Pixel 6 Pro / 7 Pro is the donor sweet spot**; avoid small-chassis newer phones.
- Confirm hardware encode path (MediaCodec native, not SW).
- WiFi ground station > direct LTE under thermal pressure.

## Gaps not in the literature

- No drone-mounted phone heatsink build logs surfaced. DJI O4 Air Unit copper kits are the closest analogue (same problem).
- No quantified slipstream-cooling-of-phones data — community evidence suggests ambient + solar dominate.
- Mobile-encode H.264 vs HEVC vs AV1 thermal benchmarks on 2025-era Snapdragon/Tensor: gated behind paywalls.

## See also

- [[android-on-drone]]
- [[phone-on-drone-airframe]]
- [[phone-power-on-drone]]
- [[phone-publisher-android-fgs]]

## Sources

- raw: [[2026-06-01-android-thermal-api]]
- raw: [[2026-06-01-android-sustained-performance-mode]]
- raw: [[2026-06-01-pixel-thermal-throttle-empirical]]
