---
title: "FPV Action Camera Mounting Guide — Vibration Isolation and Tilt Adjustment"
source: https://blog.uavmodel.com/fpv-action-camera-mounting-guide-vibration-isolation-and-tilt-adjustment/
type: article
tags: [drone, mounting, tpu, grommets, fpv, hardware]
ingested: 2026-06-01
quality: 4
confidence: high
---

# FPV camera mounting — TPU + grommet specifics

Current FPV practice; very specific BOM numbers.

## TPU shore hardness map

| Hardness | Use |
|---|---|
| **85A** | Best damping (light cams) |
| **95A** | All-around |
| **98A** | Heavier cams |
| **D60** | Crash-first, less damping |

For a **150–200 g phone, 95A or 98A is the sweet spot**.

## Mounting hardware spec

- **M3 silicone grommets**
- **40–50 Shore A**
- **6 mm or 8 mm**
- **20–30% compression**
- **4 per mount**

Directly transferable as the herd-scout phone tray BOM starting point.

## Failure modes

- Hard contact points defeating isolation
- **TPU degrades after 50–100 crashes** (printed parts)
- Over-tight straps re-couple vibration to the frame

## Camera-specific tuning

- **ND filters (ND8/16/32)** reduce jello by lengthening exposure smoothing.
- **Phone CameraX equivalent**: cap shutter to ~1/(2×fps) — manual exposure mode.
- Tilt table maps 5–50° angles by mission type. **Nadir cattle counting at 30–60 m AGL** fits the "cinewhoop indoor" 5–10° flat regime.

## Frame-size guidance

Camera-weight-vs-frame table extrapolates: **150–200 g phone needs a 5" quad minimum**, comfortably 7–10".

## Implications for herd-scout

Concrete BOM:
- **95A or 98A TPU printed tray**
- **4× M3×8 mm silicone grommets, 50A durometer**
- **20–30% compression**
- 7–10" quad minimum for stable lift
- Cap CameraX shutter to ~1/60 s at 30 FPS for jello reduction
