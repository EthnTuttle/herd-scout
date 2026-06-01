---
title: "A Gentle Introduction to Conformal Prediction and Distribution-Free Uncertainty Quantification"
source: https://arxiv.org/abs/2107.07511
authors: [Angelopoulos, Bates]
year: 2022
type: paper
tags: [conformal, uncertainty, tutorial, calibration]
ingested: 2026-06-01
quality: 5
confidence: high
---

# Conformal prediction tutorial — Angelopoulos & Bates

The standard tutorial for conformal prediction and distribution-free UQ. Covers split conformal, conformalized quantile regression, **time-series CP**, and abstention.

## Key findings relevant to herd-scout

- **Recipe applicable to soft-counts** (`Σ p_i_calibrated`):
  1. Treat soft count as a regression target.
  2. Use a held-out calibration clip to compute non-conformity scores `|Ĉ - C_true|`.
  3. Emit interval `Ĉ ± q̂_{1-α}` with marginal coverage guarantee.
- **Conditional coverage failure modes**: intervals can be marginally valid but bad per-class. Important because herd-scout reports per-class breakdowns (cow / horse / sheep).
- Time-series CP: covers the case where iid assumption fails — directly relevant since pasture video frames are autocorrelated.

## Implications for herd-scout

- The existing 200-frame split-conformal calibration mentioned in [[count-validation-conformal]] follows this recipe. Ingesting the canonical reference makes the wiki citation honest.
- Per-class conditional coverage is a known weakness — herd-scout should compute per-class non-conformity scores separately (one calibration set per class) rather than pooling.
- Time-series CP variant matters when the calibration clip and the deployment clip differ in autocorrelation structure.
