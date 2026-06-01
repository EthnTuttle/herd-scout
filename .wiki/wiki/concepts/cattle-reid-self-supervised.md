---
title: "Cattle re-ID — MultiCamCows2024, self-supervised tracklet pipelines"
summary: "Concrete recipe and dataset for adding appearance-based ReID on top of ByteTrack — replaces the speculative ResNet50 stub in herd-counting-pipeline"
tags: [reid, holstein, cattle, self-supervised, dataset, multicamcows2024, contrastive]
created: 2026-06-01
confidence: high
type: concept
---

# Cattle re-ID — self-supervised, no per-cow labels needed

[[herd-counting-pipeline]] flagged ResNet50-based ReID as "future work — kills re-entry double-counting, enables Chapman across longer sessions" without specifics. The 2024–2026 literature gives a concrete recipe.

## The dataset that changed the math: MultiCamCows2024

[[2026-06-01-multicamcows2024-yu|Yu, Burghardt, Dowsey, Campbell 2024]] (v3 Jun 2025).

- **101,329 images, 90 Holstein cows, 3 ceiling cameras, 7 days** on a working dairy farm.
- **Largest open cow-reID dataset** usable for fine-tuning.
- License: **CC BY-NC-SA 4.0** — non-commercial. Commercial deployments need their own labeled data, but the **method** is unencumbered.

## The method: self-supervised tracklets

**>96% single-image identification accuracy** with **no per-cow human labelling**.

- The existing tracker output (ByteTrack tracklets) supplies **positive pairs** automatically — frames within the same tracklet are by construction the same animal.
- Multi-camera positive pairs come from cross-camera tracklets in overlapping FOVs.
- Train a contrastive embedding (SimCLR / MoCo / SwAV style) on **coat-pattern features**.
- 2026 follow-up (arXiv:2602.15962, same group): **94.82% reID in dense crowds** using SAM + unsupervised contrastive embeddings — current SOTA for Holstein dense scenes.

## Why coat-pattern and not muzzle/face

- Muzzle/face/ear-tag pipelines (e.g. Bovid Holstein-Friesian dairy parlour systems) require the animal to stand still at known angles.
- Coat-pattern features work from **top-down or oblique views** at distance — exactly herd-scout's deployment geometry (pasture cam, drone nadir).
- For mixed-color or solid-color breeds (Angus, Hereford), coat-pattern is weaker — fall back to body-shape + gait embeddings or accept ReID is hard for that breed.

## How it slots into herd-scout

Two integration modes:

### Mode A: triggered ReID (cheap)

- Run the embedding **only at track creation** (new `track_id` from tracker) and **at re-entry events** (track lost and reappeared).
- Compare against a small bank of recent track embeddings; if cosine similarity > τ, merge `track_id`s.
- Compute amortizes well; doesn't touch the per-frame budget.

### Mode B: continuous ReID (Deep OC-SORT / BoT-SORT-with-ReID)

- Embedding on every detection.
- Higher quality matching, higher compute cost.
- Likely **not feasible on Pascal** alongside YOLO11s/YOLO26 + tracker — see [[boxmot-multi-tracker-zoo]] note on Deep OC-SORT GPU footprint.

**Mode A is the right starting point** for herd-scout.

## Implementation path

1. Capture 7 days of recorded tracklets from a deployment site (admin app already records — Wave 12 infra).
2. Self-supervised contrastive train on those tracklets — no labels needed.
3. Export to ONNX, deploy as a small head in the sidecar (or in the daemon, called only at track creation/re-entry).
4. Validate on captured + manually verified clips.

Avoid pre-training on MultiCamCows2024 weights for commercial deployments (CC BY-NC-SA 4.0); use the **method** with herd-scout's own data.

## Cross-references

- [[herd-counting-pipeline]] § "Re-entry double-count" — ReID closes that gap
- [[track-recovery-busca-hit]] — orthogonal layer: recovery without ReID
- [[tracker-choice-bot-oc-byte]] — Mode B is implicit in Deep OC-SORT / BoT-SORT-with-ReID

## Sources

- raw: [[2026-06-01-multicamcows2024-yu]]
