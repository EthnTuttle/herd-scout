---
title: "Livestock CV accuracy — realistic numbers from the published literature"
summary: "What detection / counting accuracy is achievable for cattle, sheep, goats from drone or fixed-cam video"
tags: [livestock, accuracy, cattle, sheep, drone, uav, gsd, altitude, mae, precision]
created: 2026-05-27
confidence: high
type: synthesis
---

# Livestock CV accuracy from the literature

What the agricultural and animal-science peer-reviewed literature says about counting accuracy for cattle, sheep, and goats from drone or fixed-cam video. Anchors realistic targets for [[herd-counting-pipeline]] and frames the gap between "out of the box" and "fine-tuned on local data."

## Realistic accuracy at herd-scout's spec

For RGB UAV at 30–100 m AGL on cattle/sheep in pasture, modern YOLO-family detectors land in:

- **mAP**: 0.85–0.95 under good conditions
- **Precision**: 0.90–0.95
- **Recall**: 0.85–0.92
- **Counting MAE on pasture-sized herds**: ±5–10% credible target; ±15–25% bad-case (poor light, dense clumping)

Anchored by:
- [[2026-05-20-shao-cattle-uav-dataset|Shao 2020]] — canonical at 50 m AGL nadir
- [[2026-05-27-csd-yolov8s-sheep]] — 95.2% precision, 93.1% mAP (closest spec to herd-scout)
- [[2026-05-27-ocholla-kenyan-livestock]] — multi-species (cattle/sheep/goats), YOLO top-ranked

## Recommended altitude band

**30–60 m AGL is the sweet spot** in the cattle literature (Shao 50 m is the canonical anchor). The trade-off curve:

- **Above ~80 m**: GSD coarsens enough that AP collapses. Wüthrich 2025 thesis confirms systematic AP decline with altitude — AP is the most sensitive metric, even more than precision/recall.
- **Below 30 m**: drone disturbance dominates. Herds either bunch (counting blur, IoU collisions) or scatter (tracker ID switches). The behavioral effect can be larger than any model improvement.

For oblique fixed-cam, the equivalent spec is "**animals occupy ≥30 px on the long side**" — match focal length to that. Below ~30 px the small-target regime kicks in (see CSD-YOLOv8s).

## RGB vs thermal

**Stay RGB** for detection. Mask-R-CNN + thermal-RGB drylot cattle paper:
- RGB F1 = 0.89
- Thermal-only F1 = 0.64

Thermal complements RGB only for heat-stress / nighttime — it is not a counting upgrade.

## Top three actionable lessons

1. **Stay RGB, target ~50 m equivalent GSD, and annotate occlusion explicitly.** Shao's quality labels (Normal / Truncated / Blurred / Occluded) are the right schema. herd-scout should carry the same per-detection quality flag through to the tracker so we can downweight or drop ambiguous tracks before counting.

2. **Pre-train on the open sheep+cattle UAV corpus before paddock fine-tune.** Stack Aerial Sheep (4,133 imgs, public domain) + Shao cattle (670 imgs, CC BY-NC-ND) + Ocholla Kenya repo (cattle/sheep/goats) as a base; then fine-tune on a few hundred local-paddock frames. This avoids the typical 70% precision floor (Rančić et al. 2023) seen on cold-start models.

3. **Have a density-map fallback for dense/clumped scenes.** When ByteTrack ID-switch rate spikes (the literature shows this happens when herds bunch under drone disturbance or when animals lie down at midday), switch the counter to a density regressor on the same frame. Detection-then-track is the right default; density regression is the right safety net. See [[density-counting-fidtm-p2pnet]].

## Open datasets worth pretraining on

| dataset | size | species | license |
|---|---|---|---|
| Aerial Sheep (Roboflow Universe / HuggingFace) | 4,133 imgs | sheep | Public Domain |
| Shao cattle (Aso, Japan) | 670 imgs / 1,949 anns | cattle | CC BY-NC-ND 4.0 |
| Ocholla Kenya | open notebook | cattle, sheep, goats | check repo |
| SheepCounter (Roboflow Universe) | 1,743 imgs | sheep | (paired with Fraunhofer YOLOv5/6/7/8 paper) |

Public Domain Aerial Sheep is the easiest to start with; license-clean.

## Architectural tweaks worth porting if accuracy gaps remain

From [[2026-05-27-csd-yolov8s-sheep|CSD-YOLOv8s]]:
1. **SPPFCSPC** — spatial pyramid pooling + cross-stage partial connections (small-target recall)
2. **CBAM** — convolutional block attention (channel + spatial, ~1% mAP gain on small targets)
3. **C2f_DS** — depthwise-separable conv block (efficiency)

YOLO11s already has improvements in this direction, but explicit CBAM in the neck and an SPPF-CSPC variant are the first knobs to turn if small-target recall is the bottleneck after fine-tuning.

## See also

- [[herd-counting-pipeline]] — where these numbers anchor the realistic-target section
- [[drone-vision-software]] — base CV stack
- [[drone-hardware]] — drone choice
- [[livestock-oss-gap-analysis]] — broader OSS livestock landscape
- [[herdnet-livestock-cv]] — rejected aerial detector (HerdNet)
- Sources: [[2026-05-27-shao-cattle-uav-dataset]], [[2026-05-27-ocholla-kenyan-livestock]], [[2026-05-27-csd-yolov8s-sheep]]
