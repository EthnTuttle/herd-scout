---
title: "Phone power on a drone payload — battery life, USB-PD, degradation"
summary: "How to power a streaming Android payload from drone LiPo, what charging targets actually hit, and how plugged-in heat degrades phones over time"
tags: [drone, phone, power, usb-pd, battery, lipo, degradation, chargie]
created: 2026-06-01
confidence: medium
type: concept
---

# Phone power on a drone payload

[[android-on-drone]] called out 4G LTE as a transport but didn't dimension the power side. This article covers it.

## Battery life under the herd-scout publisher load

Streaming load = CameraX H.264 encode + 4–8 Mbps network out (iroh over WiFi or LTE).

| Encoder path | Power | Pixel 7 (3.5 Wh) endurance |
|---|---|---|
| **Hardware H.264/HEVC** (MediaCodec native) | **0.8–1.3 W** | **2.5–3.5 hr** ✓ |
| Software encode (OMX.google.* fallback) | 3.2–4.5 W | <1 hr; thermal throttle in 90–120 s on budget chips ✗ |

**Implication**: confirm MediaCodec is on hardware path, not SW fallback. The 3–4× power swing is load-bearing.

A typical 20–35 min drone flight fits comfortably on a Pixel 6/7's internal battery on the hardware path. **Multi-flight days need external power** from the drone.

## USB-C PD from drone LiPo

**No mainstream off-the-shelf "3S/4S/6S → USB-C PD" board surfaced.** The closest open-hardware reference is **LiPow** (Hackaday). Practical builds use:

1. **Buck converter** from drone LiPo (Pololu D24V50F5 or similar mUBEC, ~25 W).
2. **USB-C PD source IC** (IP2726, FUSB302) on the buck output.

### Watt budget

| PD spec | Replenishes? |
|---|---|
| **5 V / 3 A = 15 W** | **Borderline** — phone draws 4–6 W streaming + 5–10 W charging. Sustains, doesn't replenish in flight. |
| **9 V / 2 A = 18 W** | OK for sustained; partial replenishment |
| **9 V / 3 A = 27 W** | **Recommended** — full replenishment during flight |

## Battery removal — don't

Pixel/Samsung batteries are glued. Removal is impractical and triggers firmware checks. Treat the phone battery as a **UPS**, not a target for removal.

## Charging hygiene

[[2026-06-01-phone-power-from-drone|chargie.org synthesis of NREL/Frontiers]]:

- **200 cycles @ 25 °C → 3.3% capacity loss**
- **Same 200 cycles @ 45 °C → 6.7% loss (>2×)**
- **Capping max charge to 80% extends lifespan up to 4×**
- High C-rates compound thermal damage at elevated temps.

**Operator playbook**:

- Enable **Pixel Adaptive Charging** / **Samsung Protect Battery** — limits to 80%.
- Don't plug a drone payload phone into a fully-charged-at-45°C state.
- Replace donor phones every **200–400 cycles** of payload service (≈1 year of daily flights).

## Sustained-performance mode trade

(See [[phone-thermal-management]] for full detail — relevant here because peak vs sustained is the central trade.)

- `Window.setSustainedPerformanceMode(true)` (Android 7+): **18% peak performance lost, 2× sustained throughput gained** over 30 min. Real, not theoretical.

## Recommended payload power architecture

```
Drone LiPo (3S/4S/6S)
    │
    ▼
Buck converter (Pololu D24V50F5 or mUBEC, ~25 W out)
    │
    ▼
USB-C PD source IC (FUSB302), advertise 9V/3A (27 W)
    │
    ▼
Phone (USB-C)
    │
    ├─ Charge limit 80% in OS settings
    ├─ Sustained performance mode ON
    └─ Hardware encode confirmed (MediaCodec native)
```

## See also

- [[android-on-drone]] — high-level architecture
- [[phone-on-drone-airframe]] — physical mounting
- [[phone-thermal-management]] — thermal side
- [[phone-publisher-android-fgs]] — software side

## Sources

- raw: [[2026-06-01-phone-power-from-drone]]
- raw: [[2026-06-01-android-sustained-performance-mode]] (cross-reference, sustained-perf trade)
