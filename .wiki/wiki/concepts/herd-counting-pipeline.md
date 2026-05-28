---
title: "Herd counting pipeline — from per-frame detections to a verified herd count"
summary: "How herd-scout converts YOLO11s + ByteTrack output into an accurate, calibrated, confidence-bounded herd count"
tags: [counting, herd-scout, bytetrack, yolo11s, supervision, conformal, calibration]
created: 2026-05-27
confidence: high
type: synthesis
---

# Herd counting pipeline

Per-frame object detections (YOLO11s, embedded NMS) plus a tracker (`supervision.ByteTrack`) do **not** by themselves produce an accurate herd count. A naive `len(set(tracker_id))` over a session is **biased upward** — fragmentation, false positives on static clutter, and re-entry events all push the count high — and offers no confidence interval. This article is the architecture for fixing that, organized as five layers stacked on top of the existing CV sidecar.

Live spec (per [[cv-sidecar-bench-2026-05-27|cv-sidecar-bench]]): YOLO11s with `nms=True`, supervision.ByteTrack producing `track_id`, GTX 1060, 1280×720 @ 30 FPS, 23 FPS sustained with tracker.

## The five layers

```
[1] Detection      → YOLO11s, per-class confidence, NMS variant
[2] Tracking       → ByteTrack tuning + min-track-length + jump-distance sanity
[3] Counting       → policy: max-distinct-IDs vs zone-based vs density-fallback
[4] Aggregation    → multi-pass / multi-cam combination → one number + interval
[5] Validation     → conformal + EID reconciliation → confidence chip
```

Most of the count error today happens in layer 3 because layers 1, 2, 4, 5 don't exist in our pipeline yet — we have a per-frame detector and a tracker, and the rest is implicit / absent.

## Layer 1: Detection — calibrated, per-class, count-aware

See [[confidence-calibration]] for full detail.

- **Per-class confidence threshold** instead of one global threshold. Cold-start defaults: `cow=0.30, horse=0.30, sheep=0.20`. Tune on ~200 labeled frames using `yolo val` F1-confidence curves.
- **Two-gate confidence pattern** from supervision examples: filter at the YOLO call (`conf=0.35`) AND again on `detections.confidence > 0.5` for *counting*. Display all detections, count only confident ones.
- **NMS choice**: keep embedded `nms=True` (current) for now. If validation shows crowded under-count, two options in priority order:
  1. Re-export with `iou=0.75` (more permissive — keeps overlapping boxes)
  2. Re-export without `nms=True`, run **soft-NMS** (Bodla et al. 2017) in Python with σ=0.5
- **Calibration**: NetCal `LogisticCalibration(detection=True)` per-class scalar fit on ~1k matched detections. Makes `E[count] = Σ p_i_calibrated` unbiased. One-time work per site.
- **Soft counting**: skip thresholding entirely; sum calibrated probabilities over surviving detections. Unbiased in expectation if calibration is good.

## Layer 2: Tracking — tuned for stationary livestock

See [[tracking-metrics-and-tuning]] for full detail.

The defaults of `supervision.ByteTrack` come from MOT17 (pedestrians, lots of motion, distinct appearance) — exactly the opposite of pasture livestock. Recommended changes for stationary herds at 30 FPS:

| supervision param | default | herd-scout | rationale |
|---|---|---|---|
| `track_activation_threshold` | 0.25 | 0.35 | Livestock are large and well-detected; fewer false starts > more recall |
| `lost_track_buffer` | 30 (1 s) | 60 (2 s) | Bridges the typical occlusion-by-neighbor case |
| `minimum_matching_threshold` | 0.8 | 0.85 | Tighter gate suppresses ID swaps between adjacent stationary cows |
| `minimum_consecutive_frames` | 1 | 3 | Suppresses single-frame YOLO flickers from getting `tracker_id` exposed |
| `frame_rate` | 30 | match real fps | `lost_track_buffer` scales with this |
| (upstream) `min_box_area` | 10 | 32×32 px | supervision drops this; replicate by filtering detections upstream |

Plus **two layer-2.5 filters** before counting (none of these are in supervision; herd-scout-specific wrapper):

1. **Cumulative-frame-count filter**: a `tracker_id` is eligible for counting only after it has been seen in ≥15 cumulative frames (0.5 s at 30 FPS). Stricter than supervision's `minimum_consecutive_frames=3` because real occlusion is non-consecutive.
2. **Centroid-jump sanity check**: drop frames where Δcentroid > V_max·Δt for a given `tracker_id` (the NVIDIA NvDCF "teleport" failure mode). Default V_max ≈ 150 px/frame at 30 FPS for pasture livestock.

**Optimize HOTA, not MOTA.** MOTA under-charges fragmentation by a factor of ~N where N is the number of fragments per real animal. HOTA's AssA term punishes fragmentation correctly. For pure counting also report a custom `|unique_track_ids| / |gt_animals|` ratio.

## Layer 3: Counting policy — match the deployment

See [[supervision-counting-api]] for the API surface and [[counting-failure-modes]] for what each policy gets wrong.

**Decision matrix:**

| deployment | policy | implementation |
|---|---|---|
| Drone single flyover, no GPS | Max distinct IDs across frames | `set(tracker_id)` after layer-2 filters |
| Drone single flyover, with GPS | World-coord dedup | Project bbox center to lat/lon via altitude+FOV; cluster within ~1 animal radius |
| Drone multi-pass | N-mixture if no ReID; Chapman if ReID | Royle 2004 (`unmarked` R) / Chapman estimator |
| Static pasture cam, herd stationary | Median(simultaneous active count) over 30-frame window | per-frame `len(unique active tracker_ids)`, median |
| Static pasture cam, gate/race/funnel | LineZone with debounce | `LineZone(minimum_crossing_threshold=2..3, triggering_anchors=[BOTTOM_CENTER])` |
| Multi-camera overlapping | POOL hand-off | IoU > 0.2 in overlap region, homography-aligned, shared global ID |

**Default to "median of simultaneous active count over a stable window"** for stationary herds. Cumulative max-distinct-IDs over a long window over-counts every ID switch as a phantom animal.

## Layer 4: Aggregation — one number with a confidence interval

See [[multipass-aggregation]] for the full decision tree and formulas.

**Lincoln-Petersen / Chapman** (when individuals can be re-identified across two passes — coat patterns, ear tags, ResNet ReID):
- `N̂_C = ((n+1)·(K+1) / (k+1)) − 1`, then truncate
- 95% CI by bootstrap or normal approximation

**Royle 2004 N-mixture** (multiple flyovers, no ReID):
- `N_i ~ Poisson(λ)`, `y_it ~ Binomial(N_i, p)`
- Joint MLE of (λ, p) gives **detection probability** `p` as a free byproduct — a model-health metric

**Bootstrap CI for the count itself**: resample frames within window 1000× → 95% CI. Cheap and honest, works regardless of which counting policy.

## Layer 5: Validation — confidence chip + active learning

See [[count-validation-conformal]] for full detail.

**Three-tier validation:**

1. **EID reconciliation** (when ISO 11784/11785 readers are present — see [[livestock-eid-rfid]]):
   - `N_eid` = ground truth from ear-tag scans
   - `Δ = N_cv − N_eid` is the audit residual
   - Use Lincoln-Petersen with EID-known animals as "marks" to derive site-specific detection probability `p̂` — calibrates the CV stack with no frame labeling
2. **Conformal prediction** (no EID): split conformal on a 200-frame per-site calibration set → distribution-free `247 ± 8 (95%)` interval. Wraps any black-box detector.
3. **Active learning loop**: random spot-checks (catch FNs) + low-confidence flags (catch FPs). Push to admin RPC plane. The Huang ICCV-2023 region-and-range UX (tap a region, pick a count *range*) is the right correction interaction.

**Confidence chip:**
- 🟢 EID-reconciled
- 🟡 Conformal-only
- 🔴 Closure violated (animals entered/exited mid-count) or `Δ` excessive

## Why this beats the naive count

- **Layer 1** removes systematic per-class bias and crowded under-count.
- **Layer 2** removes flicker fragmentation (the dominant over-count source) and ID-switch teleports.
- **Layer 3** chooses the policy that matches the actual deployment instead of forcing one onto all of them.
- **Layer 4** turns multi-pass ambiguity into a real confidence interval.
- **Layer 5** turns the existing EID reader hardware into a CV calibration source, and gives the farmer a way to verify and correct without per-animal clicking.

## When detection itself fails (dense regime)

When per-instance detection breaks down on dense clusters (sheep flocks at feeders, cattle bunched under drone disturbance — body sizes <10–15 px, density >50 / megapixel), no amount of layer 2–5 work helps. The fix is a **density-regression branch** alongside the YOLO branch.

See [[density-counting-fidtm-p2pnet]] for full alternatives. Recommendation: **YOLO11s (sparse) + FIDTM-on-dense-ROI (MIT license, point output, MIT pretrained weights)** as a future optional branch. Trigger: when local box density exceeds a threshold, crop the dense region and run the FIDTM head; replace the YOLO count for that region with the point count. Keeps the 30 FPS budget on sparse frames; pays the density cost only where detection actually fails.

## What still won't be perfect

- **Re-entry double-count** across `lost_track_buffer` is unavoidable with a pure motion-model tracker. Fix is appearance-based ReID (ResNet50 embeddings) — a separate Wave.
- **Camera motion / drone yaw** breaks the constant-velocity Kalman model. Mitigation: pre-warp frames by a homography from drone IMU before passing to the tracker. Out of scope today.
- **Closure violations** (animals enter/exit mid-count) silently invalidate Lincoln-Petersen and N-mixture estimates. Detect by checking for any `tracker_id` whose lifetime spans the entire window — flag the count as 🔴.

## Realistic accuracy targets

Per the published livestock-CV literature (see [[livestock-cv-accuracy]] for sources):

- Achievable detection precision/recall: **0.90–0.95 / 0.85–0.92** under good conditions.
- Achievable counting MAE: **±5–10%** on pasture-sized herds.
- Bad-case (poor light, dense clumping): **±15–25%** without the density-branch fallback.

These are the numbers to commit to publicly. Anything tighter requires fine-tuning on local imagery or the density branch.

## See also

- [[drone-vision-software]] — original CV stack picks (YOLO, OpenDataCam, RTSP)
- [[supervision-counting-api]] — API and recipe details
- [[tracking-metrics-and-tuning]] — HOTA + ByteTrack tuning
- [[confidence-calibration]] — soft-NMS, per-class, NetCal
- [[counting-failure-modes]] — what goes wrong and severity ranking
- [[multipass-aggregation]] — Lincoln-Petersen, N-mixture, GPS dedup
- [[count-validation-conformal]] — conformal + EID reconciliation
- [[density-counting-fidtm-p2pnet]] — when detection breaks down
- [[livestock-cv-accuracy]] — literature accuracy bounds
- [[livestock-eid-rfid]] — EID hardware that grounds layer 5
- [[cv-sidecar-bench-2026-05-27]] — current sidecar performance baseline
- [[herdnet-livestock-cv]] — alternative aerial detector (rejected — see deep-dive)
- [[herd-scout-positioning]]
