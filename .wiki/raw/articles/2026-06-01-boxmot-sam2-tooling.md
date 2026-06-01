---
title: "BoxMOT multi-tracker zoo + SAM 2 feasibility on Pascal"
source: https://github.com/mikel-brostrom/boxmot
secondary: https://github.com/facebookresearch/sam2
type: article
tags: [boxmot, tracking, sam2, pascal, sidecar, occluboost]
ingested: 2026-06-01
quality: 4
confidence: high
---

# BoxMOT + SAM 2 — tooling and feasibility

## BoxMOT

Reference multi-tracker zoo (mikel-brostrom). 8.2k stars, **v20.0.0 May 2026**, 145 releases.

Trackers supported:
- BoT-SORT, ByteTrack, OC-SORT, Deep OC-SORT, StrongSORT, HybridSORT, SFSort, **BoostTrack**, **OccluBoost**

**OccluBoost leads MOT17 at 70.47 HOTA**, BoT-SORT second.

Plug-in API:
```python
Boxmot(detector="yolov8n", reid="osnet_x0_25_msmt17", tracker="botsort")
```

Detector + ReID + tracker are independently swappable — directly usable as the experimentation harness for swapping ByteTrack → OC-SORT / Deep OC-SORT / OccluBoost without rewriting the herd-scout sidecar.

## SAM 2 — Pascal feasibility

| variant | params | A100 FPS |
|---|---|---|
| tiny | 38.9M | 91 |
| small | 46M | 85 |
| base+ | 80.8M | 64 |
| large | 224.4M | 40 |

(torch 2.5.1, CUDA 12.4, A100)

Per-session **memory module tracks objects through temporary disappearance** — theoretically a great ID-switch fix.

**Verdict for 6 GB Pascal (GTX 1060) sidecar:** not feasible for live inference.
- A100 numbers don't translate (~5–10× slowdown on Pascal)
- No FlashAttention on sm_61
- fp16 limited (Pascal silent demote, see [[ctranslate2-quantization-on-pascal]] in gtx-1060 wiki)
- Tiny variant exceeds real-time at 6 GB once YOLO11s detector is co-resident

Practical only for **offline re-scoring of failed clips** — not the live broadcast path.

## Implications for herd-scout

- BoxMOT is the right harness for the OC-SORT A/B experiment proposed in [[oc-sort-cao-2022]].
- SAM 2 is **explicitly out of scope** for live sidecar use on existing GTX 1060. Worth testing on the eventual cloud-side audit pipeline only.
