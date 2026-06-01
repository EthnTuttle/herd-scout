---
title: "MultiCamCows2024 — large-scale Holstein re-ID dataset with self-supervised pipeline"
source: https://arxiv.org/abs/2410.12695
dataset: https://tinyurl.com/MultiCamCows2024
authors: [Yu, Burghardt, Dowsey, Campbell]
year: 2024 (v3 Jun 2025)
type: paper
tags: [reid, holstein, cattle, dataset, self-supervised, multi-camera]
ingested: 2026-06-01
quality: 5
confidence: high
---

# MultiCamCows2024 — Holstein cattle re-ID at scale

Direct answer to the herd-counting-pipeline wiki's "ResNet50 future work" stub.

## Dataset

- **101,329 images, 90 Holstein cows, 3 ceiling cameras, 7 days** on a working dairy farm.
- Largest open cow-reID dataset usable for fine-tuning.
- License: **CC BY-NC-SA 4.0** — non-commercial. Important for herd-scout's licensing posture (commercial deployments need their own labeled data).

## Method + results

- **>96% single-image identification accuracy** using a **self-supervised tracklet-based pipeline**.
- No per-cow human labelling required — tracklets across cameras supply positive pairs automatically.
- Works on **coat-pattern features** (not face/muzzle) — directly applicable to herd-scout's existing top-down/oblique camera angles.
- Companion 2026 work (arXiv:2602.15962, same group, Feb 2026) reaches **94.82% reID in dense crowds** using SAM + unsupervised contrastive embeddings — current SOTA for Holstein-cattle dense scenes.

## Implications for herd-scout

- **Self-supervised ReID is feasible without manual ID labels.** The existing tracker output (ByteTrack tracklets) supplies positive pairs.
- Coat-pattern embeddings — not the Holstein-specific dairy face/muzzle work — are the right model class for pasture/race deployments where animals aren't standing at a parlour.
- Implementation path: train a lightweight (ResNet50 or smaller) coat-pattern embedding on the herd-scout sidecar's recorded tracklets; run only at track-creation/re-entry events to keep compute manageable on Pascal.
- Replaces the speculative ResNet50 ReID note in [[herd-counting-pipeline]] with a concrete, benchmarked recipe.
