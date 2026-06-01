---
title: "Multi-Object Tracking with Ultralytics YOLO"
source: https://docs.ultralytics.com/modes/track/
type: article
tags: [tracking, bytetrack, botsort, ultralytics, reid, gmc]
ingested: 2026-06-01
quality: 5
confidence: high
---

# Ultralytics tracking defaults — BoT-SORT vs ByteTrack

Canonical Ultralytics docs as of June 2026.

## Key findings

- **BoT-SORT is the default tracker** in Ultralytics, replacing ByteTrack as the recommended choice.
- BoT-SORT contributes three things ByteTrack lacks:
  - **Global Motion Compensation (GMC)** — `orb` / `sift` / `ecc` / `sparseOptFlow` modes
  - **ReID** (`with_reid`) — appearance embedding for re-identification
  - Proximity + appearance thresholds
- **ReID is OFF by default** "to minimize performance overhead" — must be opted in.
- Doc explicitly notes GMC is for **moving cameras** (vehicles, drones, PTZ). For **fixed cameras** "either tracker works," with BoT-SORT's ReID helping in occlusion-heavy crowded scenes.
- Tracker config files (`bytetrack.yaml`, `botsort.yaml`) expose all hyperparameters; switching is one CLI flag.

## Implications for herd-scout

For the **stationary chute / pasture-cam** scenario, GMC adds nothing — drop the BoT-SORT default's headline feature. The only reason to switch from ByteTrack to BoT-SORT is **optional ReID for bunching/cluster occlusions**, which is exactly herd-scout's hardest case but adds compute cost on Pascal.

For **drone-mounted (moving camera)** scenarios, GMC becomes load-bearing — phone-on-drone aerial counting is the case where BoT-SORT actually wins on architecture, not just feature count.
