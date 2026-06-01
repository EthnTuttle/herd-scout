---
title: "Pixel 6 Pro time-to-throttle and Tensor temperature trends"
sources:
  - https://www.reddit.com/r/GooglePixel/comments/r3vtky/the_pixel_6_pro_stops_recording_at_23_minutes_due/
  - https://xdaforums.com/t/pixel-6-overheats-after-10-minutes-of-4k60-shooting-30-camera-sample.4378765/
  - https://www.androidauthority.com/tensor-temps-tested-3489447/
type: article
tags: [pixel, tensor, thermal, throttle, empirical, drone]
ingested: 2026-06-01
quality: 4
confidence: medium
---

# Empirical Pixel time-to-throttle data

Community + benchmark data that calibrates the herd-scout flight-duration plan.

## 4K time-to-throttle (Pixel 6 Pro)

- **4K30 indoors: ~20–23 min** before thermal warning
- **4K60 indoors: ~10 min** to throttle
- **4K60 outdoors in heat: 3–4 min** ⚠️
- Pixel 7 Pro: **>1 hour 4K** (vapor chamber upgrade)
- HEVC ↔ H.264 swap extends runtime — hardware encoder paths differ in thermal load by SoC revision.

## Tensor surface temps (3DMark Wild Life 20-min stress)

- Pixel 6 Pro: **40.1 °C**
- Pixel 9 Pro XL: **43.2 °C**
- Pixel 9 Pro (smaller chassis): "considerably hotter"

Trend: surface temps **rising** across Tensor generations (G1→G4) despite 48% battery efficiency gain. **Smaller chassis = worse thermals.**

## Key implication: contradicts wiki claim

The existing [[android-on-drone]] article says "Thermal: Mostly fine in flight (slipstream cooling); ground idle is risky." The Pixel 6 Pro **4K60 outdoor 3–4 min** number suggests **ambient + solar load dominate over slipstream gain** at typical drone speeds.

**Recommended downgrade**: "in flight" claim depends on ambient temp and altitude; verify per-airframe at 720p30 (lower thermal load than 4K60).

## Donor-phone strategy

For a sacrificial drone payload, **bigger chassis beats faster chip**. Pixel 6/7 Pro (vapor chamber) outperforms Pixel 9 Pro (smaller) on sustained encode despite older silicon.

## Codec choice (community evidence)

H.264 vs HEVC swap matters per-SoC. **Test both** on the actual donor model rather than assuming HEVC is always more efficient (true for compute, ambiguous for thermal due to encoder fixed-function block design).

## Implications for herd-scout

- **Target 720p30, not 4K60**, for the publisher — already the case per the existing repo.
- 720p30 thermal envelope is materially better than 4K60; community numbers above are pessimistic upper bounds for what herd-scout actually does.
- **Pixel 6 Pro / 7 Pro** is the right donor-phone sweet spot (vapor chamber, large enough chassis).
- Avoid Pixel 9 Pro non-XL (smaller chassis = worse sustained thermals).
- A/B H.264 vs HEVC on the actual donor model.
