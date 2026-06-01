---
title: "Tracker choice — when to switch from ByteTrack to BoT-SORT or OC-SORT"
summary: "2025-2026 tracker selection: ByteTrack baseline vs BoT-SORT (default in Ultralytics, with optional ReID) vs OC-SORT (best for non-linear bunching motion)"
tags: [tracking, bytetrack, botsort, ocsort, deep-ocsort, occluboost, sam2, boxmot]
created: 2026-06-01
confidence: high
type: concept
---

# Tracker choice — ByteTrack vs BoT-SORT vs OC-SORT in 2025-2026

The herd-scout sidecar today uses **`supervision.ByteTrack`** with custom tuning (`track_activation_threshold=0.35`, `lost_track_buffer=60`, `minimum_matching_threshold=0.85`, `minimum_consecutive_frames=3`) plus the cumulative-frame eligibility filter (≥15) and centroid-jump sanity check from [[herd-counting-pipeline]]. As of June 2026, **Ultralytics ships BoT-SORT as the default tracker** over ByteTrack — but that doesn't automatically mean herd-scout should switch.

## The decision matrix

| Scenario | Best tracker | Why |
|---|---|---|
| Stationary chute / fixed pasture cam | **ByteTrack (current) or OC-SORT** | GMC wasted on fixed cameras; OC-SORT only adds value if bunching is the failure mode |
| Cattle bunching at gates / feed bunks | **OC-SORT or Deep OC-SORT** | Non-linear motion + occlusion is exactly OC-SORT's design target |
| Phone-on-drone (moving camera) | **BoT-SORT with GMC** | GMC is load-bearing for moving cameras |
| Mixed deployment, willing to pay ReID cost | **BoT-SORT with `with_reid=True`** | Appearance ReID handles long occlusions; +compute |
| Best published HOTA on MOT17 (offline, 2026) | **OccluBoost** (via BoxMOT) | 70.47 HOTA top of leaderboard |

## What each tracker actually adds

### BoT-SORT (Aharon et al. 2022)

Three contributions over ByteTrack:
1. **Global Motion Compensation (GMC)** — `orb` / `sift` / `ecc` / `sparseOptFlow`. **Only matters for moving cameras.** Wasted on fixed pasture cams.
2. **Improved Kalman state** — 8-dim with width/height instead of aspect-ratio/scale. Camera-agnostic gain. ~always net positive.
3. **IoU + ReID fusion** — `with_reid=True` (off by default). Appearance embedding handles long occlusions. Compute cost: meaningful on Pascal.

If herd-scout switches to BoT-SORT for a fixed cam: only the **Kalman state vector** survives as a real win (modest). The BoT-SORT default isn't strictly better than ByteTrack here.

### OC-SORT / Deep OC-SORT (Cao et al. 2022, 2023)

**Pure motion-model tracker, no ReID required.** Core idea: **Observation-centric Re-Update (ORU)** — when a track is recovered after occlusion, retroactively re-update the Kalman filter using observations during the gap, preventing the error accumulation that breaks ByteTrack on non-linear paths.

- **DanceTrack** (the canonical non-linear-motion benchmark): **89.4 MOTA**.
- MOT17/20 HOTA: 63.2 / 62.4 — competitive with BoT-SORT.
- Compute cost: **~700 FPS association on CPU**. Drop-in replacement, near-zero overhead.

Cattle bunching at gates / feed bunks is a non-linear-motion regime — exactly OC-SORT's design target. **Strongest theoretical fit for herd-scout's hardest failure case** and the cheapest A/B test.

**Deep OC-SORT** adds Adaptive CMC + Dynamic Appearance. CMC is the moving-camera feature again (irrelevant for fixed); **Dynamic Appearance is the part worth A/B testing on static herds** if OC-SORT alone isn't enough.

### SAM 2 (Meta, 2024)

Per-session memory module tracks objects through temporary disappearance. A100 numbers: tiny @ 91 FPS, large @ 40 FPS.

**Verdict for 6 GB Pascal sidecar**: not feasible for live inference. A100 numbers don't translate (~5–10× slowdown on Pascal); no FlashAttention on sm_61; tiny variant exceeds real-time at 6 GB once YOLO11s is co-resident.

**Practical only for offline re-scoring** of failed clips — compatible with herd-scout's upload-batch path, **not** the live broadcast path.

## Recommended A/B path for herd-scout

1. **Keep ByteTrack as the live-broadcast default** until OC-SORT is validated.
2. **A/B test OC-SORT** on captured bunching/gate clips via [[boxmot-multi-tracker-zoo|BoxMOT]] — drop-in, no compute risk on Pascal.
3. **If OC-SORT wins on bunching**: deploy as the new default for the live path.
4. **Only switch to BoT-SORT** if/when the moving-camera (drone) deployment lands — GMC becomes load-bearing then.
5. **Skip SAM 2** for the live sidecar; revisit only for offline upload-batch re-scoring on a future sidecar SKU with more VRAM.

## Compatibility caveat for YOLO26

[[yolo26-and-tracker-compat]]: YOLO26's NMS-free **end-to-end head** outputs `(N, 300, 6)` directly. ByteTrack expects raw pre-NMS scores for its low-confidence "BT-Low" association pass — **the new head breaks that**. Keep YOLO26's **one-to-many head** (legacy mode via `end2end=False` at export) for ByteTrack compatibility until empirical validation says otherwise.

## See also

- [[herd-counting-pipeline]] — the 5-layer pipeline this slots into at layer 2
- [[boxmot-multi-tracker-zoo]] — experimentation harness
- [[yolo26-and-tracker-compat]] — head-choice constraint
- [[track-recovery-busca-hit]] — orthogonal track-recovery layer
- [[counting-failure-modes]] (raw) — failure mode taxonomy these trackers address

## Sources

- raw: [[2026-06-01-ultralytics-tracking-defaults]]
- raw: [[2026-06-01-oc-sort-cao-2022]]
- raw: [[2026-06-01-boxmot-sam2-tooling]]
