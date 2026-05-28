---
title: "CSD-YOLOv8s — Dense sheep small-target detection in oblique pasture view"
source_url: https://www.sciopen.com/article/10.12133/j.smartag.SA202401004
type: paper
tags: [sheep, yolov8, oblique, small-target, dense, occlusion, pasture, sppfcspc, cbam]
created: 2026-05-27
confidence: high
---

# CSD-YOLOv8s — Dense sheep small-target detection

## Spec

- Yu et al. — Smart Agricultural Technology
- **Sheep, UAV-on-ground (oblique) view in natural grassland pastures** — the closest published spec to herd-scout's pasture-mounted-cam case
- Architecture: YOLOv8s + SPPFCSPC + CBAM + C2f_DS modules

## Numbers

- Precision: 93.0–95.2%
- mAP: 91.2–93.1%
- Speed: **87 FPS** (model not specified for HW; presumably commodity GPU)

## Targeted failure modes

The paper's stated targets exactly match herd-scout's pain points:
- Dense clustering (sheep mobs)
- Occlusion (overlapping animals, fence/tree partial occlusion)
- Small targets (animals at far end of FOV)
- Varied lighting (dawn/dusk, dappled shade)

## Architecture tweaks worth porting to YOLO11s

1. **SPPFCSPC** — Spatial Pyramid Pooling Fast + Cross Stage Partial Connections. Multi-scale feature fusion that helps small-target recall without exploding parameter count.
2. **CBAM** — Convolutional Block Attention Module. Channel + spatial attention. Standard ~1% mAP gain on small targets at modest cost.
3. **C2f_DS** — Custom block with depthwise-separable convolutions for efficiency.

YOLO11s already has improvements from YOLOv8 in this direction (C2PSA attention, etc.) but explicit CBAM in the neck and an SPPF-CSPC variant are well-validated drop-ins if benchmarks show small-target recall is the bottleneck.

## Implications for herd-scout

- Realistic precision target: 93–95% precision at 91–93% mAP is achievable on dense pasture sheep with a tuned YOLO + attention setup. Today's YOLO11s baseline (no fine-tune) will fall well below this — the gap is the fine-tuning / dataset effort.
- **Closest published architecture to herd-scout's spec.** When/if herd-scout fine-tunes YOLO11s on local data, this paper's tweaks are the first thing to try if small-target recall is weak.
- 87 FPS reported speed implies the architecture is not a bottleneck; data + tuning are.

## Why ingest

Direct architectural reference for a small-target dense-pasture case. Confirms the target metric range (~93% precision / ~92% mAP) is plausible at our spec, and provides three specific architectural tweaks ranked by ROI for future fine-tuning work.

## Sources

- Yu et al., "CSD-YOLOv8s: Dense sheep small-target detection in natural grassland pastures" (Smart Agricultural Technology, 2024)
- Open access via sciopen.com (DOI 10.12133/j.smartag.SA202401004)
