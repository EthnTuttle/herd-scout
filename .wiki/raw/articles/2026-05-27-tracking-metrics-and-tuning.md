---
title: "HOTA tracking metric + ByteTrack tuning for stationary herds"
source_url: https://arxiv.org/abs/2009.07736
type: paper
tags: [hota, mota, idf1, bytetrack, tracking-metrics, tuning, kalman]
created: 2026-05-27
confidence: high
---

# Tracking metrics & ByteTrack tuning

## HOTA (Luiten et al. 2020, IJCV / arxiv 2009.07736)

- `HOTA = sqrt(DetA × AssA)` averaged over localization thresholds α ∈ {0.05, 0.10, ..., 0.95} (so LocA is folded in implicitly).
- `DetA = |TP| / (|TP| + |FN| + |FP|)` at each α — pure detection Jaccard (same spirit as MOTA's detection terms).
- `AssA = mean over TPs of |TPA| / (|TPA| + |FNA| + |FPA|)` — association Jaccard, where TPA/FNA/FPA are *associations* between predicted and ground-truth tracks for that match.
- Geometric mean is the key property: weak detection or weak association each drive HOTA toward 0; MOTA's linear sum lets a strong detector mask poor association.
- Authors argue HOTA aligns with human visual judgment of tracker quality where MOTA does not.

## Why HOTA matters more than MOTA for *counting*

- `MOTA = 1 − (FN + FP + IDSW)/GT`. Identity errors are 1/N of detection errors in cost. A tracker that fragments every animal into 4 tracklets but detects them all loses only ~3·N/total in MOTA (maybe a 5% drop) while the count is 4× wrong.
- HOTA's AssA is computed per matched detection over the full track pair, so the same fragmentation slashes AssA roughly to 1/4 and HOTA to ~1/2.
- α-averaging exposes localization drift (cattle bboxes wandering across frames degrade IoU at high α) — MOTA's single-α=0.5 hides this.
- For stationary herds the dominant failure modes are fragmentation + ID swap between adjacent cows; both are under-charged by MOTA and correctly punished by HOTA/AssA.

For pure counting you can also report a custom **|unique_track_ids| / |gt_objects|** ratio — that's what the user actually cares about.

## ByteTrack internals (paper + source)

- arxiv 2110.06864. Reports 80.3 MOTA / 77.3 IDF1 / 63.1 HOTA on MOT17 — the gap between MOTA and HOTA is exactly why HOTA is the better predictor of count quality.
- Two-stage association: (1) high-conf detections matched IoU+score-fusion at `match_thresh`, (2) low-conf detections matched IoU-only at fixed 0.5 distance for occluded recovery.
- Pure motion model — Kalman + IoU, no appearance embedding. Strength on occlusion bridges, weakness on stationary targets where detector flicker dominates.
- Kalman state is 8-D `(cx, cy, a, h, vx, vy, va, vh)`, constant-velocity model.
- Process noise stds scale with bbox height: position `1/20·h`, velocity `1/160·h`. **Implication for stationary livestock:** small/distant cattle (small h) get tighter gates; big foreground cattle get loose gates and may swap IDs across neighbors.
- `det_thresh = track_thresh + 0.1` — hidden +0.1 buffer for new track initiation; only tunable indirectly.
- `max_time_lost = int(frame_rate / 30.0 × track_buffer)` — buffer is in frames but scales with declared frame_rate.
- For untracked states the filter zeros velocity (`mean_state[7] = 0`); a track returning from "lost" predicts in place — good for stationary herds, but covariance still inflates per frame, loosening the gate.

## Recommended ByteTrack settings for stationary livestock at 30 FPS (supervision API)

| param | recommended | default | rationale |
|---|---|---|---|
| `track_activation_threshold` | 0.30–0.40 | 0.25 | Livestock are large, persistent, well-detected. Fewer false starts > fewer recall misses. |
| `lost_track_buffer` | 45–90 frames (1.5–3 s) | 30 (1 s) | Cattle that briefly merge bboxes / are occluded by a neighbor recover under same ID. |
| `minimum_matching_threshold` | 0.85 | 0.8 | Stationary targets have near-perfect frame-to-frame IoU; tighter gate suppresses ID swaps between adjacent animals. |
| `frame_rate` | match real fps | 30 | Buffer scales with this — must be honest. |
| `minimum_consecutive_frames` | 3 | 1 | Suppresses single-frame YOLO flickers from being counted as a new animal — biggest count-error source on stationary herds. |
| (upstream) `min_box_area` | ≥32×32 px | 10 | supervision drops this filter; replicate upstream. |

## Why ingest

HOTA is the right scalar to optimize for accurate counting; MOTA actively misleads. ByteTrack's defaults are tuned for MOT17 (pedestrians, lots of motion, distinct appearance) — herd-scout's stationary, near-uniform-appearance pasture is the opposite regime, and the params above are concrete starting points.

## Sources

- Luiten et al. 2020, "HOTA: A Higher Order Metric for Evaluating Multi-Object Tracking" (arxiv 2009.07736)
- HOTA project page: https://autonomousvision.github.io/hota-metrics
- Zhang et al. 2022, "ByteTrack: Multi-Object Tracking by Associating Every Detection Box" (arxiv 2110.06864)
- ByteTrack source: `byte_tracker.py`, `kalman_filter.py` in github.com/ifzhang/ByteTrack
- supervision tracker docs (`/latest/trackers/`)
