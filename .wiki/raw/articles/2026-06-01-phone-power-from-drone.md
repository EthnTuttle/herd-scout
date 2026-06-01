---
title: "Phone power management on a drone — USB-PD, charging, battery degradation"
sources:
  - https://chargie.org/battery-degradation-impact-of-temperature-and-charging-rates-on-lithium-ion-cell/
  - https://developer.android.com/games/optimize/adpf/thermal
type: article
tags: [phone, drone, power, usb-pd, battery, degradation]
ingested: 2026-06-01
quality: 3
confidence: medium
---

# Phone power on a drone payload

## Battery life under sustained encode + 4-8 Mbps upload

- **Hardware H.264/H.265 encode**: ~0.8–1.3 W on modern SoCs
- **Software encode**: 3.2–4.5 W (3–4× higher; thermal throttle in 90–120 s on budget chips)
- Pixel 7 internal battery (~3.5 Wh) at hardware-encoded streaming → **~2.5–3.5 hr**, comfortably one drone flight (20–35 min).
- Multi-flight days need external power.

## USB-C PD from drone battery

- **No mainstream off-the-shelf "3S/4S/6S → USB-C PD" board surfaced**. LiPow (Hackaday) is the closest open-hardware project.
- Practical path:
  1. Buck converter (Pololu D24V50F5 or similar mUBEC, ~25 W) from drone LiPo
  2. → USB-C PD source IC (e.g. **IP2726, FUSB302**)
- **15 W (5V/3A) is borderline** — phone draws 4–6 W streaming + 5–10 W charging.
  - 15 W only sustains, doesn't replenish.
  - **Aim for 18–27 W PD** to charge during flight.

## Battery removal

- Pixel/Samsung batteries are glued. Removal impractical, triggers firmware checks.
- **Better**: enable **charge-limit-to-80%** ("Adaptive Charging" Pixel / "Protect Battery" Samsung).
- Treat phone battery as a UPS — no replenishment in flight, recharge between flights.

## Battery degradation (chargie.org synthesis of NREL/Frontiers)

- **200 cycles @ 25 °C → 3.3% capacity loss**
- **Same 200 cycles @ 45 °C → 6.7% loss (>2×)**
- High C-rates compound thermal damage at elevated temperatures.
- **Capping max charge at 80% extends lifespan up to 4×.**

## Sustained-performance mode trade

(Cross-ref [[android-sustained-performance-mode]])

- 18% peak performance lost
- 2× sustained throughput gained
- Real, not theoretical (per dev.to LLM-on-Android measurements)

## Implications for herd-scout

- Hardware encode is **load-bearing** — verify MediaCodec is on hardware path (not OMX.google.* SW fallback). 3–4× power swing.
- For >1-flight days: **18–27 W USB-PD source from drone** OR battery swap workflow.
- Charge limit 80%; never plug in a drone payload phone at 100% in heat.
- Operator playbook line: **swap drone payload phones every 200–400 cycles** at minimum (1× battery degradation budget per year of daily flights).
