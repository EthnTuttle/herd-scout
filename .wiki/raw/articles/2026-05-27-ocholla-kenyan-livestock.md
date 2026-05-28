---
title: "Ocholla 2024 — Livestock detection in Kenyan rangelands (cattle/sheep/goats)"
source_url: https://www.mdpi.com/2072-4292/16/16/2929
type: paper
tags: [livestock, cattle, sheep, goats, rangeland, multi-species, yolo, faster-rcnn]
created: 2026-05-27
confidence: high
---

# Ocholla 2024 — Livestock detection in Kenyan rangelands

## Spec

- **Multi-species: cattle, sheep, goats together** (the only paper found covering all three of herd-scout's target species in mixed herds)
- VHR aerial imagery
- Compared 9 SOTA detectors (Faster R-CNN, YOLO variants, RetinaNet, etc.)
- Code + notebooks: github.com/Ian-ocholla/Aerial_detection_livestock

## Findings

- **YOLO variants top-ranked** across the 9-detector comparison.
- Key challenge framed by the paper: "small targets in heterogeneous environment" — directly relevant to oblique fence-line views and the small-far-end-of-FOV failure mode.
- Transfer to satellite imagery explored but lower-performing (consistent with HerdNetSat's 32% detection rate).
- Mixed-species classification confounded by goat/sheep similarity in aerial view — argues for per-class confidence thresholds AND per-species training data.

## Implications for herd-scout

1. YOLO11s is the right architecture family — confirms the existing pipeline choice.
2. Multi-species deployment will need explicit per-class threshold tuning (sheep ≠ goat in conf calibration; see `confidence-calibration`).
3. The "small target heterogeneous environment" framing implies fence/tree/rock confounders are well-known and not solvable by detector tuning alone — must be addressed at the counting layer (post-track filtering, confidence calibration, scene-aware ROI).
4. Open notebook on GitHub means we can replicate the comparison on local imagery cheaply.

## Why ingest

The only peer-reviewed paper covering herd-scout's exact species mix in pasture-realistic conditions. Provides citation-grade confirmation that YOLO is the right family and that multi-species pasture detection is hard but tractable.

## Sources

- Ocholla et al. 2024, MDPI Remote Sensing 16(16):2929
- Code: github.com/Ian-ocholla/Aerial_detection_livestock
