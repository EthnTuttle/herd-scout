---
title: "YOLO26 — drop-in upgrade with one tracker compatibility caveat"
summary: "How to retrain on YOLO26 (Jan 2026) without breaking the ByteTrack BT-Low score-based association pass"
tags: [yolo, yolo26, nms-free, tracker, bytetrack, retrain]
created: 2026-06-01
confidence: high
type: concept
---

# YOLO26 — what it changes for herd-scout

YOLO26 (Ultralytics, **Jan 14, 2026**) is the production successor to YOLO11. The assess 2026-06-01 report flagged "retrain on YOLO26" as the **single highest-ROI action** for herd-scout. This article covers the one non-trivial caveat.

## What YOLO26 actually adds

- **Dual-head architecture**:
  - **NMS-free one-to-one head** outputs `(N, 300, 6)` directly — end-to-end detection, no post-processing.
  - Legacy **one-to-many head** still available via the `end2end=False` flag at export.
- **ProgLoss + STAL** target small-object recall — directly relevant to far-field cattle (drone 30–60 m AGL per [[livestock-cv-accuracy]]).
- **+43% CPU inference speedup** over predecessors.
- All export targets (ONNX, TensorRT, CoreML, TFLite, OpenVINO) — same as YOLO11, no toolchain change.
- COCO mAP: 40.9 (n) → 57.5 (x).

## The tracker compatibility caveat

ByteTrack's two-pass association uses **raw detection scores**:

1. **Pass 1 ("BT-High")**: high-confidence detections (above threshold) match to existing tracks.
2. **Pass 2 ("BT-Low")**: lower-confidence detections — those that fell *below* the activation threshold but are still detections — are matched to *unmatched* tracks from pass 1, recovering tracks across brief detection drops.

This second pass is the reason ByteTrack outperforms simpler trackers — it uses the **score distribution** to recover almost-detections.

YOLO26's **NMS-free end-to-one head** outputs a fixed `(N, 300, 6)` of *kept* detections — the score distribution above and below threshold that ByteTrack relies on **does not exist** in this output.

## Recommendation

**Export YOLO26 with the legacy one-to-many head** (`end2end=False`) until empirical validation says otherwise.

```python
# Pseudocode for the sidecar export script
model = YOLO("yolo26s.pt")
model.export(format="onnx", end2end=False)  # keep legacy head
```

This preserves:
- ByteTrack's BT-Low pass
- Per-class confidence thresholding (cow=0.30, horse=0.30, sheep=0.20 from [[herd-counting-pipeline]])
- Calibration via NetCal `LogisticCalibration(detection=True)`
- Soft-counting (`Σ p_i_calibrated`)

## Validation path before switching to end-to-end head

1. Train YOLO26s on the same dataset as current YOLO11s — apples-to-apples.
2. **Two exports**: legacy (`end2end=False`) and end-to-end (`end2end=True`).
3. A/B both heads through the existing sidecar + ByteTrack pipeline.
4. Compare HOTA + AssA + the herd-scout-specific `|unique_track_ids| / |gt_animals|` ratio (per [[herd-counting-pipeline]]).
5. If end-to-end head matches or beats legacy: switch.

This is the cheapest possible "do we trust the new head?" experiment.

## What about retrain data requirements?

The same dataset that fine-tuned YOLO11s. No new labeling needed.

For pasture-cam / drone deployments, the [[livestock-cv-accuracy]] pretrain stack (Aerial Sheep + Shao cattle + Ocholla Kenya) plus a few hundred site-specific frames is the canonical recipe — unchanged for YOLO26.

## Pascal-specific notes

- ONNX Runtime CUDA EP at FP32 (not FP16) — same as YOLO11s today (Pascal silent fp16 demote per [[ctranslate2-quantization-on-pascal]] in the gtx-1060-headless-ai-server hub wiki).
- The 43% CPU speedup is irrelevant on the Pascal sidecar (we're GPU-bound); the **mAP and small-object recall improvements** are what we care about.

## See also

- [[herd-counting-pipeline]] — where YOLO26 slots in at layer 1
- [[livestock-cv-accuracy]] — the accuracy bounds that retrain is trying to push
- [[tracker-choice-bot-oc-byte]] — how the tracker decision interacts with this head choice

## Sources

- raw: [[2026-06-01-yolo26-ultralytics]]
- raw: [[2026-06-01-grounding-dino-livestock]] — open-vocab is **not** an alternative to YOLO26 here
- raw: [[2026-06-01-cotracker3-karaev-2024]] — CoTracker3 is **not** a replacement, point-tracking is orthogonal
