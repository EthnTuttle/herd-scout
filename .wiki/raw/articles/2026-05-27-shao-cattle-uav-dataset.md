---
title: "Shao 2020 — Cattle detection and counting in UAV images (Aso, Japan)"
source_url: https://datasetninja.com/cattle-detection-and-counting-in-uav-images
type: paper
tags: [cattle, uav, drone, dataset, shao, japan, 50m, occlusion-labels]
created: 2026-05-27
confidence: high
---

# Shao 2020 — Cattle detection and counting in UAV images

## Spec

- Cattle, DJI Phantom 4
- **~50 m above ground level (AGL), near-nadir**
- Frame size: 4000×3000 px
- 670 images / 1,949 cattle annotations across two pastures (Aso, Japan)
- License: CC BY-NC-ND 4.0 (research only — same blocker as HerdNet for commercial herd-scout shipping)

## Annotation schema (the actually-useful part)

Each cattle annotation carries a **quality label**:
- Normal
- Truncated (cut off by frame edge)
- Blurred (motion blur)
- Occluded (partially hidden by another animal, structure, vegetation)

This schema directly maps to herd-scout's confounders. **Recommendation:** carry the same per-detection quality flag through to the tracker so we can downweight or drop ambiguous tracks before counting.

## Status as a benchmark

De-facto reference for "cattle UAV counting." Cited by every subsequent UAV-cattle paper. The 50 m AGL altitude has become the de-facto canonical anchor — it is the sweet spot where:
- Cattle occupy enough pixels to be reliably detected (>30 px on long side)
- GSD is fine enough that AP doesn't collapse (Wüthrich 2025 thesis confirms AP drops above ~80 m)
- Drone disturbance is moderate (animals neither bunch panicked nor scatter)

## Why ingest

Provides (a) the canonical altitude/AGL parameter for herd-scout drone capture, (b) an annotation schema with quality labels we should adopt verbatim, (c) a citation backbone for counting-accuracy claims in the playbook. License blocks weight reuse, but a license-clean re-annotation by farm staff on local imagery is straightforward (Shao's images are CC BY-NC-ND, so derivative weights are tainted; but the schema is uncopyrightable).

## Sources

- Shao et al. 2020, Int. J. Remote Sensing — "Cattle detection and counting in UAV images based on convolutional neural networks"
- Dataset card: datasetninja.com/cattle-detection-and-counting-in-uav-images
