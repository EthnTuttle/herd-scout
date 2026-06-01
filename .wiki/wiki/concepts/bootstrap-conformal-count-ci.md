---
title: "Bootstrap + conformal CIs on counts — block bootstrap, BCa, J+aB"
summary: "Why frame-iid bootstrap is wrong for autocorrelated MOT output, what to use instead, and how to fuse conformal prediction onto the same compute"
tags: [bootstrap, conformal, block-bootstrap, jackknife-after-bootstrap, statistics, ci]
created: 2026-06-01
confidence: high
type: concept
---

# Bootstrap + conformal CIs on counts

The current `report.rs` implementation in herd-scout uses **frame-iid bootstrap** with 1000 resamples and a deterministic seed from the clip hash. The 5-layer [[herd-counting-pipeline]] mentions split conformal in layer 5 as a separate primitive. Both choices have known issues; both can be upgraded without changing the ML pipeline.

## Why frame-iid bootstrap is wrong here

For 30 FPS pasture video, neighboring frames are **almost identically detected** — the same cows are in roughly the same pixel positions for several seconds at a time. Frame-iid resampling treats them as independent draws, so the resulting "CI" massively understates the true uncertainty.

The fix is **block bootstrap** — resample contiguous *blocks* of frames instead of individual frames, preserving the local autocorrelation structure.

## Politis & White 2004 — block-length selection

[[2026-06-01-politis-white-2004-block-bootstrap|Politis & White 2004, Econometric Reviews]].

- For variance estimation under weak dependence, optimal block length scales as **`b* ∝ n^(1/3)`**.
- Three variants:
  - **Moving-Blocks Bootstrap (MBB)** — Künsch 1989; sample contiguous blocks
  - **Circular Block Bootstrap (CBB)** — wraps blocks at boundaries
  - **Stationary Bootstrap (SB)** — Politis-Romano 1994; geometrically distributed block lengths
- **SB preferred** when stationarity preservation matters (pasture video transitions between grazing / movement bouts).
- Plug-in procedure: estimate spectral density at zero, compute optimal `b` adaptively.

**Concrete starting point for herd-scout**:
- 30-sec window → n=900 frames
- `n^(1/3) ≈ 10` frames (~0.33 s) baseline block length
- Better: implement Politis-White plug-in to choose adaptively per clip.
- Default to **SB with mean block length ≈ 10 frames** since pasture autocorrelation length varies (grazing vs. movement bouts).

## Resample count

- Efron's classical guidance: **B = 1000 is sufficient for percentile/BCa CIs**; >10000 only matters for tail quantiles.
- herd-scout's existing 1000 resamples is fine.

## CI variant choice: percentile vs BCa vs t

- **Percentile** is OK only if the bootstrap distribution is symmetric and unbiased.
- **Counts are bounded below by 0** with a long upper tail from re-ID errors → **distribution is rarely symmetric**.
- **BCa (bias-corrected accelerated)** is the gold standard for skewed estimators. Adds two scalars (bias correction, acceleration) — ~30 LOC.
- Bootstrap-t requires a variance estimate of the statistic, awkward for `Σ p_i` (soft-counting).

**Recommended default: SB block bootstrap + BCa percentile.**

## Critical caveat — bootstrap doesn't expose bias

Bootstrap reports variance, not specification error. If the tracker systematically loses one cow per minute, **the bootstrap CI tightens around the wrong number**. This is the failure mode the existing `report.rs` is implicitly assuming away.

**Mitigation** (load-bearing): an externally-counted clip per session. EID reconciliation per [[herd-counting-pipeline]] layer 5 already provides this when EID hardware is present — the conformal/bootstrap layer is for when it isn't.

## Hybrid: jackknife+-after-bootstrap (J+aB)

[[2026-06-01-jackknife-plus-after-bootstrap-kim-2020|Kim, Xu, Barber NeurIPS 2020]].

- Reuses the **same bootstrap ensemble** to produce a conformal-style **predictive interval**.
- **Finite-sample marginal miscoverage at most `2α`** — formal guarantee.
- **No additional model fits required** — "free" relative to split-conformal which needs a held-out calibration set.
- Coverage holds **without distributional assumptions** on the data or the aggregation rule.

**Implementation cost**: ~30 LOC on top of the existing bootstrap. Smallest possible upgrade for the largest validation-layer gain.

## Recommended combined upgrade

1. **Replace frame-iid with stationary block bootstrap** (Politis-Romano 1994; mean block length ≈ 10 frames for 30 FPS).
2. **Switch percentile → BCa** for the CI variant.
3. **Add J+aB conformal interval** on top of the same ensemble — ship both intervals (variance vs predictive) in the report.
4. **Per-class non-conformity scores**: don't pool across cow/horse/sheep — separate calibration per class to avoid the conditional-coverage failure mode in [[2026-06-01-angelopoulos-bates-conformal-2022]].

## See also

- [[herd-counting-pipeline]] — layer 4 (aggregation) and layer 5 (validation)
- [[count-validation-conformal]] (raw) — split conformal alternative
- [[track-recovery-busca-hit]] — recovery layer that reduces the systematic-bias problem this CI can't catch

## Sources

- raw: [[2026-06-01-politis-white-2004-block-bootstrap]]
- raw: [[2026-06-01-jackknife-plus-after-bootstrap-kim-2020]]
- raw: [[2026-06-01-angelopoulos-bates-conformal-2022]]
