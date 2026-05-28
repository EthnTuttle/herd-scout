---
title: "Count validation: conformal prediction, EID reconciliation, active learning UX"
source_url: https://openaccess.thecvf.com/content/ICCV2023/papers/Huang_Interactive_Class-Agnostic_Object_Counting_ICCV_2023_paper.pdf
type: paper
tags: [conformal-prediction, validation, eid, rfid, active-learning, ux, count-uncertainty, herd-scout-specific]
created: 2026-05-27
confidence: high
---

# Validating CV-derived herd counts without dense ground truth

## Three-tier strategy

### Tier 1 — EID present (authoritative count known)

Treat the EID-register count `N_eid` (from ISO 11784/11785 ear-tag scans — see `livestock-eid-rfid`) as ground truth.

- Print CV count `N_cv` plus residual `Δ = N_cv − N_eid` as a **Verified** badge ("CV agreed with tag scan").
- When `|Δ| > 0`, fall back to Lincoln-Petersen / Chapman: of the `N_eid` known animals, how many did the CV pipeline track this session? That detection rate `p̂` produces a 95% CI on what the count would be if EID were absent — calibrating the CV stack site-by-site without any frame labeling.
- Log `p̂` over time as a model-health metric. Drift = retrain trigger.

### Tier 2 — No EID (CV-only)

**Conformal prediction** — split conformal:
- Need only a small per-site calibration set (~200 labeled frames once at install).
- Distribution-free, finite-sample coverage: "the true label is in the predicted set with probability ≥ 1−α."
- Per-frame nonconformity scores aggregated over a tracking window → count interval over time, not just per-frame.
- Calibration set is portable: re-run when site conditions drift.

**Repeated-count / N-mixture** (when herd is approximately stationary across consecutive sweeps):
- Joint-estimate N and detection probability without any labels (see `multipass-aggregation`).

**Active learning loop** (Roboflow pattern):
- Combine random spot-checks (catch false negatives — dominant error for occluded cattle) with low-confidence-frame flagging (catch false positives).
- Confidence-based sampling alone CANNOT catch false negatives — missed animals have no confidence score. Random spot-check is non-negotiable.
- Push flagged frames to herd-scout's admin RPC plane (already present in Wave 11/12) for farmer review.

### Tier 3 — UX surfacing uncertainty

- Display count as `247 ± 8` with a colored confidence chip:
  - **Green**: EID-reconciled (Tier 1)
  - **Amber**: conformal-only (Tier 2)
  - **Red**: high disagreement / animals entered/exited mid-count (closure violated)
- **"Tap to verify" mode** adopting the Huang ICCV-2023 region-and-range UX:
  - Paddock view tiles into IPSE-style regions (Iterative Peak Selection and Expansion).
  - Farmer taps a region they think is wrong, picks a count *range* — no per-animal clicking. Leverages human "subitizing" (~1–4 at-a-glance count).
  - Refinement module updates locally; correction propagates to subsequent frames.
  - Reported 30–40% MAE reduction with minimal user input across benchmarks.
- "Verified vs estimated" persistent state per session — once farmer confirms, the count's confidence flips to green and the correction is stored as a calibration sample for future conformal recalibration.

## Why conformal prediction over Bayesian / softmax-CI

- Conformal: distribution-free, finite-sample, methodology-agnostic. Wraps any black-box detector. Requires only a small calibration set.
- Bayesian uncertainty: requires retraining or a Bayesian-CV framework (e.g., MC Dropout, deep ensembles). Heavyweight.
- Raw softmax / detector confidence: poorly calibrated out of the box (see `confidence-calibration`); doesn't translate to a count interval at all.

For herd-scout's "wrap a YOLO black-box and emit `247 ± 8`" requirement, conformal is the only realistic option.

## Key gap not in the literature

**No published work specifically reconciles ISO 11784/11785 EID-reader streams with live CV counts.** herd-scout has an opportunity to publish that reconciliation algorithm itself as a differentiator, both technically (specific Lincoln-Petersen-with-EID-as-marks formulation) and product-positioning-wise (commercial vendors like CattleEye, OneCup, Folio3 do not publish count uncertainty; printing a real CI would be differentiating).

## Why ingest

Without dense ground truth, this is the only set of techniques that produces an honest confidence interval on a herd count. Tier 1 (EID reconciliation) is uniquely available to herd-scout because of the existing RFID reader integration — turns a hardware feature into a CV calibration mechanism with no extra effort from the farmer.

## Sources

- Huang et al. ICCV 2023, "Interactive Class-Agnostic Object Counting" — region+range UX
- Roboflow blog — "What is Active Learning?"
- MathWorks — "Quantify Uncertainty in Object Detection Using Split Conformal Prediction"
- Royle 2004 N-mixture / Lincoln-Petersen (also covered in `multipass-aggregation`)
- arXiv 2507.14855 — Uncertainty-aware DETR (localization uncertainty input)
- Crossing reference: `livestock-eid-rfid` for EID protocol; `multipass-aggregation` for the statistical methods
