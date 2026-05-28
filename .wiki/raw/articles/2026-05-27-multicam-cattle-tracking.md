---
title: "Multi-cam black cattle tracking — POOL hand-off + density-adaptive IoU (PMC11861714)"
source_url: https://pmc.ncbi.nlm.nih.gov/articles/PMC11861714/
type: paper
tags: [multi-camera, tracking, cattle, pool-region, ioU-adaptive, mot, hand-off, re-id]
created: 2026-05-27
confidence: high
---

# Multi-camera cattle tracking — POOL hand-off & density-adaptive IoU

## Spec

- Nature Scientific Reports, 2025
- 4 corner cameras observing a 23.3 × 20 m pen with ~55 black cattle
- Custom Tracking Algorithm (CTA): Manhattan-distance + IoU score, **30-frame consistency check** before issuing a new ID
- Beats general-purpose trackers by a wide margin on this case

## Numbers

| tracker | MOTA |
|---|---|
| **CTA (this paper)** | **95.61%** |
| ByteTrack | 73.34% |
| BoT-SORT | 73.29% |
| OC-SORT | 71.79% |

22-point MOTA gap on cattle vs. on humans. Demonstrates how much domain-specific tuning is worth.

## POOL hand-off (the technique)

- Each camera defines an "overlap region" (POOL) where its FoV intersects an adjacent camera's FoV.
- When a cattle bbox is inside its camera's POOL, the system checks the same region in the adjacent camera.
- **IoU > 0.2** between the two cameras' bboxes (after homography to a common ground plane) → assign the same global ID.
- Multi-process shared lock for global ID issuance.

For a static pasture-cam deployment with overlapping fields of view, this is the right pattern. Lighter than ReID-embedding-based multi-cam matching and works without an appearance model.

## Density-adaptive IoU threshold

- `τ(D) = τ_min + (τ_max − τ_min) · min(1, αD)` where `D = N/A` (animals per area)
- When density is high, IoU threshold tightens (less likely to merge two distinct cattle into one track).
- When density is low, IoU loosens (more lenient matching, fewer fragmentations).

For herd-scout, this is a small code change — local density of detections in a frame region drives a per-region IoU threshold. Not currently in supervision.ByteTrack but worth a wrapper.

## 30-frame consistency check

- A new candidate detection must produce consistent IoU + Manhattan agreement across **30 frames** before being issued a track ID.
- 1 second at 30 FPS — long enough to filter noise without delaying counts.
- This is a stronger version of supervision's `minimum_consecutive_frames=1` default.

## Implications for herd-scout

- For multi-cam pasture deployments: POOL pattern with homography-aligned overlap regions and IoU > 0.2 cross-cam matching is the right baseline.
- For single-cam deployments: density-adaptive IoU + 30-frame consistency check are both worth implementing as a wrapper around supervision.ByteTrack.
- The 22-point MOTA gap shows that domain-specific tuning on cattle outperforms generic SOTA trackers — herd-scout should not assume "ByteTrack default" is good enough.

## Why ingest

Best peer-reviewed reference for **multi-camera livestock counting**, plus two single-cam tricks (density-adaptive IoU, 30-frame consistency) that don't require multi-cam to be valuable.

## Sources

- "Optimizing black cattle tracking through multi-camera collaboration"
- Nature Scientific Reports / PMC11861714
- Compared trackers: ByteTrack, BoT-SORT, OC-SORT
