---
title: "Android Sustained Performance Mode — Window.setSustainedPerformanceMode"
source: https://source.android.com/docs/core/power/performance
type: article
tags: [android, sustained-performance, throttle, cts, api]
ingested: 2026-06-01
quality: 5
confidence: high
---

# Sustained Performance Mode — AOSP HAL docs

`Window.setSustainedPerformanceMode(true)` (Android 7+).

## What it does

- Caps CPU/GPU at **highest sustainable frequency** rather than letting them boost-then-throttle.
- Trades **~18% peak performance** for **~2× sustained throughput** over 30 min (per dev.to thermal-throttling-LLM measurements).
- CTS guarantees:
  - Frame-rate variation **<5%** over time
  - Post-30-min sustained FPS must exceed un-flagged 30-min FPS

## Caveats

- OEMs only have to *pass* CTS — implementation quality varies.
- Pixel reference implementation works; cheap MediaTek/Unisoc devices may no-op.
- Apps **must check `isSustainedPerformanceModeSupported()`** before relying.

## Bonus: getExclusiveCores

Reserves a CPU core for the foreground app — directly relevant to keeping the encoder/upload thread off shared cores.

## Implications for herd-scout

- Set the flag at publisher startup; check support and log when no-op.
- Pair with thermal listener — sustained mode trades peak for endurance, which is exactly the drone-flight-duration tradeoff.
- **2× sustained throughput** is a real number (not theoretical) — load-bearing for the difference between "phone shuts down at 8 min" and "phone streams the whole flight."
