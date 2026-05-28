---
title: "Counting failure modes in detect-then-track pipelines"
source_url: https://forums.developer.nvidia.com/t/nvdcf-tracker-id-reassignment-causes-false-line-crossing-counts-in-deepstream-7/
type: forum
tags: [failure-modes, id-switch, fragmentation, teleport, occlusion, bytetrack, deepstream, livestock]
created: 2026-05-27
confidence: high
---

# Counting failure modes in detect-then-track pipelines

## Catalog

| Failure mode | Bias direction | Severity | Trigger |
|---|---|---|---|
| Track fragmentation (flicker) | over-count | systematic, scales with detector recall × herd size | detector drops box for >`track_buffer` frames; new ID issued on next hit |
| ID switch (no teleport) | ~zero on unique-IDs counter | low | two animals swap IDs at crossing |
| Centroid teleport on ID reassignment | ±1 per event, frequent | high | ReID disabled + uniform appearance + close proximity |
| Occlusion in tight groups | under-count | systematic, scales with density | back-row animal not detected at all |
| Small-object failure at FOV edge | under-count | systematic, perspective-biased | animal below NMS minimum or below conf threshold |
| False positives on hay/shadow/rocks | over-count | environment-biased | static clutter persisted across frames, gets a track |
| Re-entry double-count | over-count | +1 per re-entry | animal leaves frame, returns after `track_buffer` expires |
| Camera motion / drone yaw | multiplicative | global | breaks Kalman constant-velocity assumption |
| Homogeneous appearance (cattle look alike) | compounding | n/a | breaks any ReID-based fix |

## Severity ranking for a naive `len(set(tracker_ids))` counter

1. Fragmentation/flicker → over-count (largest expected error)
2. False positives on static clutter → over-count
3. Re-entry across `track_buffer` → over-count
4. Occlusion in tight groups → under-count (partly recovered if brief)
5. ID switches without teleport → near zero on unique-IDs counter
6. Teleport line-crossings → only matter if using a line/zone counter, not a unique-IDs counter

**Key insight:** count error is biased *upward* under the naive approach because three of the top mechanisms (fragmentation, FPs, re-entry) all push the same direction.

## Concrete observed cases

**NVIDIA NvDCF teleport** (DeepStream 7 forum):
- Tracker reassigns existing `object_id` to a different detection; centroid jumps 300–600+ px in one frame.
- Triggers: visually identical objects, ReID disabled (motion-only), close proximity, low track confidence.
- Legitimate motion is 20–50 px/frame; teleports exceed 400 px → distinguishable.
- Workaround: max-jump-distance filter (>150 px) discards spurious crossings. Doesn't fix the tracker; fixes the counter.

**ByteTrack ID jitter on football** (supervision PR #1080):
- "no.14 → no.26 in less than 500 ms" during occlusions.
- Maintainer fix: tune `track_buffer` (frames to keep a lost track alive). Too low → fragments tracks; too high → ghost tracks.

**ByteTrack documented weak spots** (Roboflow deep-dive):
- Heavy occlusion, sudden appearance/motion change, exiting/re-entering frame.
- Small objects yield "limited pixel representation" → unstable boxes → ID instability.

## Highest-leverage mitigations

1. **Tune `lost_track_buffer` long enough to bridge typical occlusions** (1.5–3 s for stationary livestock).
2. **Centroid-jump sanity check** in the counter (drop frames where Δcentroid > V_max·Δt for an ID).
3. **Min-track-length / hit-count filter** before counting (≥15 cumulative frames at 30 FPS = 0.5 s) — kills flicker FPs.
4. **Decouple "counted" from "drawn"**: the unique-IDs set populated only by confirmed tracks; visual overlay shows everything.
5. **Keep line/zone logic separate from unique-IDs**: the failure modes for the two counters differ.

## Why ingest

Concrete, quantified failure-mode catalog with directional severity. The combined effect of fragmentation + FP + re-entry biases counts UPWARD on a naive unique-IDs counter, which contradicts the typical worry ("we'll under-count occluded animals"). Severity ordering directly drives the mitigation priority list in `concepts/herd-counting-pipeline`.

## Sources

- NVIDIA DevTalk forum (NvDCF tracker teleport, DeepStream 7)
- supervision PR #1080 (ByteTrack ID jitter)
- Roboflow ByteTrack deep-dive (`blog.roboflow.com/what-is-bytetrack-computer-vision`)
- supervision tracking docs (`/latest/how_to/track_objects/`)
- arXiv 2307.14591 — IDS detection & rectification in MOT (abstract only)
