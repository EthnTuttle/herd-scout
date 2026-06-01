---
title: "CoTracker3: Simpler and Better Point Tracking by Pseudo-Labelling Real Videos"
source: https://arxiv.org/abs/2410.11831
authors: [Karaev et al., Meta]
year: 2024
type: paper
tags: [point-tracking, cotracker, gait, lameness, video]
ingested: 2026-06-01
quality: 4
confidence: medium
---

# CoTracker3 — dense point tracking

Meta FAIR, October 2024.

## Key findings

- Online + offline variants.
- Handles occluded points.
- "1000× less data" than predecessors — simplified architecture.
- **No published livestock or gait benchmarks surfaced** as of June 2026.
- Closest livestock work: Russello et al. (arXiv:2508.10643, Aug 2025) uses **T-LEAP pose estimation + BLSTM** for cattle lameness, hitting **85% accuracy from 1s of video**. **Not** CoTracker3.

## Implications for herd-scout

- CoTracker3 is **unproven for livestock** as of mid-2026.
- For lameness/gait analytics, the validated path is **T-LEAP + BLSTM**, not CoTracker3.
- Skip CoTracker3 for live deployment; revisit if a livestock-specific benchmark publishes.
- Worth keeping on the watch list — point-tracking + occlusion handling could eventually replace ByteTrack for fine-grained motion if a livestock paper validates it.
