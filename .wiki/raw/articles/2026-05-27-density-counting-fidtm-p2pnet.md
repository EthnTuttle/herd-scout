---
title: "Density-based counting: FIDTM, P2PNet, DecideNet — when detection breaks down"
source_url: https://arxiv.org/abs/2102.07925
type: paper
tags: [density-estimation, fidtm, p2pnet, csrnet, decidenet, dm-count, hybrid, crowd-counting]
created: 2026-05-27
confidence: high
---

# Density-based counting alternatives + hybrid architectures

## When density beats detection

Crossover regime, consistent across crowd-counting and animal-counting literature:

- Per-instance head/body size **< ~10–15 px**, OR
- Local density **> ~50 instances per 1 megapixel**

(Rule of thumb derived from ShanghaiTech A vs B and TRANCOS benchmarks.)

Below crossover → YOLO + ByteTrack outperforms (you get IDs for free).
Above crossover → NMS suppresses real animals; detection-only count collapses; density methods scale.

For herd-scout: scattered paddock cattle = sparse regime. Sheep flock at a feeder, cattle bunched under drone disturbance = dense regime.

## Method comparison

### FIDTM — Focal Inverse Distance Transform Maps (Liang et al. 2022, IEEE TMM, arxiv 2102.07925)

| property | value |
|---|---|
| input | RGB image |
| output | FIDT map + Local-Maxima Detection Strategy (LMDS) → discrete points |
| backbone | HRNet variant |
| license | **MIT** + open weights for ShanghaiA/B, UCF-QNRF, JHU, NWPU |
| F1 (ShanghaiA / ShanghaiB / UCF-QNRF / JHU++ / NWPU) | 77.6 / 83.5 / 82.2 / 62.4 / 78.9 |
| ONNX | not shipped, but plain conv → exportable |

Crucially: solves the "blurry blob overlap" failure mode of pure density maps in dense regions. Output is points, not blobs — composes naturally with ByteTrack downstream.

### P2PNet — Point-to-Point Network (Song et al. ICCV 2021 oral, arxiv 2107.12746)

| property | value |
|---|---|
| input | RGB image |
| output | set of (x, y, conf) point proposals via Hungarian one-to-one matching |
| backbone | VGG-16 + small head |
| license | Tencent — must verify before commercial use |
| MAE / MSE on ShanghaiA | 52.74 / 85.06 (SOTA at release) |
| nAP localization metric | 64.4% on SHA |

End-to-end point set; no NMS, no LMDS post-processing. Architecturally cleanest fit for an "alternative to YOLO+ByteTrack" pipeline. Tencent license is the sole blocker.

### DecideNet (Liu et al. CVPR 2018, arxiv 1712.06679)

| property | value |
|---|---|
| approach | detection density map + regression density map fused by attention module that picks per-pixel which branch to trust |
| Mall MAE | 1.52 |
| ShanghaiTech B MAE | 9.23 |
| official code | none in upstream search |

Exactly the "detect when sparse, regress when dense" hybrid herd-scout would build. Use as a design reference, not a drop-in.

### CSRNet (Li et al. CVPR 2018, arxiv 1802.10062)

- VGG-16 + dilated conv, ~16M params
- Output: continuous density heatmap (count = sum)
- ShanghaiTech A MAE 68.2, B MAE 10.6, TRANCOS MAE 3.56
- License: arXiv "nonexclusive-distrib"; pytorch repo has NO LICENSE file → effectively all-rights-reserved → risky for commercial. Skip on legal grounds.

### DM-Count (Wang et al. NeurIPS 2020, arxiv 2009.13077)

- Density map trained with **Optimal Transport + Total Variation loss** (no Gaussian smoothing of GT).
- ~16% error reduction over previous SOTA on standard benchmarks.
- License: MIT in upstream microsoft/DM-Count.
- Best-in-class loss for density map training; pair with HRNet/CSRNet backbone for a permissively-licensed density head.

## HerdNet position

Already covered in `raw/articles/2026-05-21-herdnet-deep-dive`. Summary: code MIT but **weights CC BY-NC-SA-4.0 (research only)** — blocks commercial reuse. Out-of-distribution on pasture livestock (trained on African wildlife). Skip.

## Recommended hybrid for herd-scout

Two-branch DecideNet-style fusion is overkill for a 30 FPS daemon. Pragmatic recipe:

1. Run YOLO11s + ByteTrack as today, producing per-frame detections and tracks.
2. Compute coarse local-density proxy from box centroids (boxes per HxW cell, OR NMS-suppression rate).
3. **When a region exceeds threshold T**, crop it and run a point-regression head (FIDTM-style) only on that ROI. Replace the YOLO count for that cell with the point count; seed any unmatched points as new track candidates.
4. Outside dense regions, trust YOLO + ByteTrack unchanged.

Keeps the 30 FPS budget, reuses ByteTrack IDs in the sparse majority, only pays the density cost where detection actually fails.

## Cleanest license + ONNX path

**FIDTM is the winner**: MIT code, open pretrained weights, point output (not blob density), plain conv+HRNet graph that exports to ONNX without custom ops. P2PNet is architecturally a closer fit but its Tencent license + Hungarian-matching layer add legal AND export risk.

## Why ingest

The detection+ByteTrack pipeline has a known break-point in dense scenes. This article catalogs the alternatives, scores them on license + ONNX path + accuracy, and recommends a license-clean hybrid (YOLO11s sparse / FIDTM-on-dense-ROI) that preserves the existing daemon architecture.

## Sources

- Liang et al. 2022, "FIDTM" (arxiv 2102.07925), MIT, github.com/dk-liang/FIDTM
- Song et al. 2021, "P2PNet" (arxiv 2107.12746), ICCV oral, Tencent
- Liu et al. 2018, "DecideNet" (arxiv 1712.06679), CVPR
- Li et al. 2018, "CSRNet" (arxiv 1802.10062), CVPR
- Wang et al. 2020, "DM-Count" (arxiv 2009.13077), NeurIPS, MIT
- Crossing reference: `2026-05-21-herdnet-deep-dive` for HerdNet status
