---
title: "Automatic Block-Length Selection for the Dependent Bootstrap"
source: https://public.econ.duke.edu/~ap172/Politis_White_2004.pdf
secondary: https://math.ucsd.edu/~politis/SBblock-revER.pdf
authors: [Politis, White]
venue: Econometric Reviews 23(1), 2004
type: paper
tags: [bootstrap, block-bootstrap, autocorrelation, statistics]
ingested: 2026-06-01
quality: 5
confidence: high
---

# Block-bootstrap block-length selection — Politis & White 2004

Foundational, peer-reviewed, the canonical answer to "what block size for autocorrelated bootstrap?"

## Key findings

- For variance estimation under weak dependence, optimal block length scales as **`b* ∝ n^(1/3)`**.
- For distribution estimation, scales as **`n^(1/4)` or `n^(1/5)`** depending on smoothness.
- Plug-in automatic procedure: estimate spectral density at zero, plug into closed-form optimal-`b` expressions, pick block length adaptively.
- Unifies three block-bootstrap variants:
  - **Moving-Blocks Bootstrap (MBB)** — Künsch 1989; sample contiguous blocks
  - **Circular Block Bootstrap (CBB)** — wraps blocks at boundaries
  - **Stationary Bootstrap (SB)** — Politis-Romano 1994; geometrically distributed block lengths
- **SB preferred** when stationarity preservation matters (e.g. resampling time-series).
- **MBB preferred** when correlation is concentrated at short lags.

## Implications for herd-scout

The existing `report.rs` uses **frame-iid bootstrap** (1000 resamples, deterministic seed). For a 30 FPS pasture clip with autocorrelation timescales ~1–3 sec (typical occlusion duration / bout transitions), **frame-iid is invalid** — neighbouring frames have nearly identical detections.

Concrete starting point: 30-sec window → n=900 frames → `n^(1/3) ≈ 10` frames (~0.33 s) as MBB block length.

Better: implement the Politis-White plug-in to choose adaptively per clip. SB with mean block length ≈ 10 frames is the safer default since pasture autocorrelation length varies (grazing vs movement).

**Critical caveat from this paper that the playbook should capture**: bootstrap CIs around a biased tracker output are dangerously reassuring — the CI tightens around the wrong number. Bootstrap exposes variance, not specification error. External validation (manually counted clip per session) is irreplaceable.
