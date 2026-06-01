---
title: "YOLO26 — Ultralytics, January 2026 release"
source: https://docs.ultralytics.com/models/yolo26/
type: article
tags: [yolo, yolo26, nms-free, edge, livestock]
ingested: 2026-06-01
quality: 5
confidence: high
---

# YOLO26 — production successor to YOLO11

Released **Jan 14, 2026**.

## Key findings

- **Dual-head architecture**:
  - **NMS-free one-to-one head** outputs `(N, 300, 6)` — end-to-end detection, no post-processing
  - Legacy **one-to-many head** still available via `end2end` flag at export
- **ProgLoss + STAL** target small-object recall — directly relevant to far-field cattle (drone 30-60 m AGL).
- **+43% CPU inference speedup** over predecessors.
- CPU ONNX 640px latency: n=38.9 ms, s=87.2 ms, m=220 ms.
- COCO mAP: 40.9 (n) → 57.5 (x).
- All export targets supported (ONNX, TensorRT, CoreML, TFLite, OpenVINO).

## Critical caveat for herd-scout's tracker integration

**Tracker integration is undocumented.** The 300-detection cap on the one-to-one head is well above any pen-scale herd, but **ByteTrack expects raw pre-NMS scores** for its low-confidence "BT-Low" association pass.

Switching to the end-to-end (NMS-free) head **breaks the score distribution ByteTrack uses** for its second association pass.

**Recommendation:** keep the **one-to-many head** for ByteTrack compatibility until empirical validation shows the end-to-end head gives equivalent or better tracking.

## Implications for herd-scout

- Drop-in retrain: same dataset, same export target, same sidecar wiring.
- Highest-ROI single change identified in the assess 2026-06-01 report.
- ProgLoss/STAL specifically helps the failure mode flagged in [[livestock-cv-accuracy]] — small-object recall on far cattle.
- **Use one-to-many head**, not the new end-to-end head, until tracker compatibility is validated.
- Pascal-specific: ONNX Runtime CUDA EP at FP32 (not FP16) — same as YOLO11s today.
