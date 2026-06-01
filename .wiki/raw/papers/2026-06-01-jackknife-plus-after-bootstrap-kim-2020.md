---
title: "Predictive Inference Is Free with the Jackknife+-after-Bootstrap"
source: https://arxiv.org/abs/2002.09025
authors: [Kim, Xu, Barber]
venue: NeurIPS 2020
type: paper
tags: [conformal, bootstrap, jackknife, predictive-inference]
ingested: 2026-06-01
quality: 5
confidence: high
---

# Jackknife+-after-Bootstrap (J+aB)

Direct hybrid of bootstrap variance estimation and conformal predictive intervals — exact thing the gap-list called out as missing.

## Key findings

- Reuses the **same bootstrap ensemble** already trained for variance estimation to produce a conformal-style predictive interval.
- **Finite-sample marginal miscoverage at most `2α`** — formal guarantee.
- **No additional model fits required** — "free" relative to split-conformal which needs a held-out calibration set.
- Coverage holds **without distributional assumptions** on the data or the aggregation rule (mean / median / trimmed-mean of the ensemble).

## Implications for herd-scout

- herd-scout already runs 1000 bootstrap resamples for the count CI. **J+aB lets us emit a conformal interval on the same compute** — no additional inference.
- Pairs with [[politis-white-2004-block-bootstrap|block bootstrap]]: use SB with adaptive block length (variance estimate) AND derive a conformal predictive interval from the same ensemble.
- The marginal-coverage guarantee is more honest than "1000-resample percentile" because it's distribution-free — and it doesn't require the held-out calibration set that split conformal demands.
- Implementation cost: ~30 LOC on top of existing bootstrap. Smallest possible upgrade for the largest theoretical gain in the validation layer.
