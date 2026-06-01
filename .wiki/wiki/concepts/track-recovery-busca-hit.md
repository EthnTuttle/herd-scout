---
title: "Track recovery — BUSCA, HIT, and tracklet stitching for short-window counting"
summary: "Online + offline ways to recover tracks before they fragment, recovering counting accuracy without retraining the detector"
tags: [tracking, busca, hit, tracklet-stitching, online, id-switch, fragmentation]
created: 2026-06-01
confidence: high
type: concept
---

# Track recovery — fix fragments before they become phantom counts

The largest single error source in herd-scout's count pipeline is **track fragmentation** (per [[herd-counting-pipeline]] and the playbook): one animal becomes two `track_id`s when the detector drops it for >`lost_track_buffer` frames. The cumulative-frame eligibility filter (≥15) and the median-of-active-IDs window mitigate this, but they don't *recover* the lost track — they suppress it.

Two 2024 publications give the modern algorithmic primitives for actual recovery.

## BUSCA — online recovery via Q&A transformer (ECCV 2024)

[[2026-06-01-busca-vaquero-eccv-2024|Vaquero et al. ECCV 2024]].

- **Plug-in module** that recovers detector-missed objects **fully online** (no future-frame access).
- Generates candidate proposals from neighbouring tracks + motion priors.
- A **decision Transformer** answers a Q&A-style question: "does this proposal extend track k?" — combining visual + spatiotemporal features.
- Compatible with **5 base trackers** including ByteTrack-class methods.
- SOTA on 3 benchmarks at submission.

**Fits the live broadcast path** because it's online. Compute cost: transformer inference per candidate. Need to verify it fits the 23 FPS budget on Pascal — likely too heavy for live, possibly fine for the upload-batch path where latency is unconstrained.

## HIT — hierarchical IoU tracking with offline refinement (2024)

Du, Zhao, Su (arXiv:2406.13271).

- **Hybrid online + offline** framework.
- Online tracker runs as today.
- Offline refinement pass uses **tracklet intervals** as priors and merges fragments via interpolation + global linking (Hungarian-style).
- **Appearance-free** (IoU only) — deployable on the existing Rust/CV-sidecar without adding ResNet50.
- Integrated into 7 different base trackers.

**Cheapest possible upgrade path.** Appearance-free, post-hoc, works on any tracker output.

**Fits the upload-batch path**: HIT runs after the clip is fully ingested, before [[herd-counting-pipeline|layer 4]] aggregation.

## Recommended deployment

| Path | Tracker | Recovery layer |
|---|---|---|
| Live broadcast | ByteTrack (or OC-SORT after A/B) | None initially → BUSCA if compute budget allows |
| Upload batch | Same tracker | **HIT post-hoc tracklet stitching** before counting |

HIT is the higher-leverage place to start because:
1. It runs offline → no latency budget concerns.
2. It's appearance-free → no model training, no Pascal compute risk.
3. It runs on the upload path, which is where herd-scout's most accurate count estimates live anyway.

## What this leaves for ReID

Even with BUSCA + HIT, **re-entry double-count across `lost_track_buffer` boundaries is unavoidable** with a pure motion-model tracker. The fix is appearance-based ReID — see [[cattle-reid-self-supervised]] for the dataset/method that closes this gap.

## See also

- [[herd-counting-pipeline]] — fragmentation is the dominant error source at layer 2
- [[tracker-choice-bot-oc-byte]] — base-tracker selection
- [[cattle-reid-self-supervised]] — appearance ReID for re-entry case
- [[bootstrap-conformal-count-ci]] — CIs that won't hide a fragmentation bias

## Sources

- raw: [[2026-06-01-busca-vaquero-eccv-2024]]
