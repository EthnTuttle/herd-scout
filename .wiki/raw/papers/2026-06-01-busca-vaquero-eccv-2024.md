---
title: "BUSCA: Lost and Found — online tracker recovery via Q&A transformer"
source: https://arxiv.org/abs/2407.10151
repo: https://github.com/lorenzovaquero/BUSCA
authors: [Vaquero, Xu, Alameda-Pineda, Brea, Mucientes]
venue: ECCV 2024
type: paper
tags: [tracking, online, id-switch, recovery, transformer, eccv]
ingested: 2026-06-01
quality: 5
confidence: high
---

# BUSCA — online recovery for missed detections

Plug-in module that recovers detector-missed objects **fully online** — directly addresses the "track went silent for N frames" failure mode that makes ByteTrack drop tracks before the herd-scout eligibility window of ≥15 cumulative frames.

## Key findings

- Generates candidate proposals from neighbouring tracks + motion priors.
- A **decision Transformer** answers a Q&A-style question: "does this proposal extend track k?" — combining visual + spatiotemporal features.
- Demonstrated compatibility across **5 base trackers** including ByteTrack-class methods.
- SOTA on 3 benchmarks at submission.
- **Fully online** — no future-frame access. The right primitive for herd-scout's short-window counting (median over 30 frames, eligibility at ≥15 cumulative frames).

## Implications for herd-scout

This is the closest fit to "online ID-switch detection" the assess gap analysis flagged. It operationalises the heuristic as a learned model rather than a hand-tuned rule (centroid-jump filter, cumulative-frame eligibility).

Pairs naturally with the existing pipeline: BUSCA recovers tracks that ByteTrack drops, *before* they fragment into a phantom new ID — reducing the fragmentation error that the playbook calls out as the "largest single error source."

Compute cost: transformer inference per candidate. Need to verify it fits the 23 FPS budget on Pascal — likely too heavy for live, possibly fine for the upload-batch path where latency is unconstrained.
