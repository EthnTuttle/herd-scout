---
title: "Soft-NMS, per-class confidence, and detection calibration for unbiased counts"
source_url: https://arxiv.org/abs/1704.04503
type: paper
tags: [soft-nms, nms, calibration, netcal, confidence-threshold, per-class, counting-bias]
created: 2026-05-27
confidence: high
---

# Soft-NMS, per-class thresholds, and calibration

## Soft-NMS (Bodla et al. 2017, arxiv 1704.04503)

- Replaces hard suppression with score decay:
  - Linear: `s_i ← s_i · (1 − IoU)` if `IoU > Nt`
  - Gaussian: `s_i ← s_i · exp(−IoU² / σ)`, σ ≈ 0.5
- Reports +1.1–1.7% mAP on PASCAL VOC / COCO; biggest gains on overlapping/crowded scenes — the exact failure mode for grazing herds where two real animals at IoU > 0.5 lose one to hard NMS, causing systematic under-count.
- Same complexity as standard NMS — drop-in replacement.
- Community-default hyperparameters: σ=0.5 (Gaussian) or Nt=0.3 (linear).

## NMS variants in supervision

- `Detections.with_nms()` and `Detections.with_nmm()`. NMM = Non-Max Merge — averages overlapping boxes instead of suppressing them; similar in spirit to soft-NMS's "preserve rather than discard" philosophy.
- `class_agnostic` flag toggles cross-class behavior.
- Default IoU = 0.5; raise to 0.7–0.8 to keep more overlapping boxes (counter-intuitive: higher IoU threshold means *more permissive* — only suppresses when boxes overlap a lot).
- supervision has no native soft-NMS; closest is `with_nmm`.

## NetCal (Küppers et al.)

GitHub: `EFS-OpenSource/calibration-framework`. The only mature OSS library for **detection-specific** calibration.

- Methods: `HistogramBinning`, `LogisticCalibration` (Platt), `TemperatureScaling`, `BetaCalibration` — all with `detection=True` flag.
- "Dependent" variants (`LogisticCalibrationDependent`, `BetaCalibrationDependent`) condition calibration on bbox position/size; docs explicitly recommend these over scalar variants for detection (distant sheep are systematically under-confident, etc.).
- Standard workflow:
  1. Match detections to ground truth (IoU > 0.5 → TP=1, else FP=0)
  2. Stack `(confidence, relative_x, relative_y, w, h)` features
  3. Fit per-class calibrator
  4. Measure D-ECE (Detection Expected Calibration Error)
- Conventional sample size: ≥1k matched detections per class.

## Per-class confidence thresholds (recipe)

Procedure (Ultralytics + Roboflow standard):
1. Capture ~200 labeled frames covering target classes.
2. `yolo val data=... save_json=True` → outputs F1-confidence curve per class.
3. Pick each class's F1-peak as that class's production threshold.

Defensible cold-start defaults for COCO YOLO11s baseline (no fine-tune) on pasture livestock:

| class | start conf | rationale |
|---|---|---|
| cow | 0.30 | Large, well-represented in COCO, strong AP. |
| horse | 0.30 | Similar — large, distinctive. |
| sheep | 0.20 | Smaller, more uniform/clustered, weaker baseline AP — lower threshold to fight under-count. |

These bias slightly toward recall: counting prefers minor over-count + dedup over hard miss.

## Soft counting (calibrated probability sum)

For unbiased E[count]: skip thresholding entirely and compute
`count = Σ p_i_calibrated` over surviving (soft-)NMS detections.
Unbiased in expectation if calibration is good — even when individual scores are noisy.

## Bottom line

- **Per-class thresholds** are the highest-leverage cheap fix. Picking F1-peak per class on 200 validation frames is half a day of work.
- **Soft-NMS** is worth re-exporting YOLO11s without `nms=True` if and only if validation shows crowded under-count (sheep flocks, cattle at feeders). Cheaper interim: try raising the embedded NMS IoU to 0.75 first.
- **Per-class temperature scaling** via NetCal is a one-parameter-per-class fix that makes E[count] unbiased without changing the model. Escalate to dependent calibration if distance/size bias remains.

## Why ingest

Three concrete, code-level levers — per-class thresholds, soft-NMS, NetCal calibration — that directly attack systematic count bias without requiring model retraining or architecture changes.

## Sources

- Bodla et al. 2017, "Soft-NMS — Improving Object Detection With One Line of Code" (arxiv 1704.04503)
- Küppers et al. NetCal: github.com/EFS-OpenSource/calibration-framework
- Ultralytics validation docs (`docs.ultralytics.com/modes/val/`)
- supervision NMS/NMM API (`supervision.roboflow.com`)
- Roboflow blog: "What is mAP" / threshold tuning
