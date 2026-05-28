---
title: "Playbook: Accurate herd counting from CV detections"
type: playbook
created: 2026-05-27
question: "We're capturing video and there is object detection, how can we ensure we get an accurate count of the herd from the object detection results?"
---

# Playbook: accurate herd counting

## The question

> We're capturing video and there is object detection, how can we ensure we get an accurate count of the herd from the object detection results?

## TL;DR

A naive `len(set(tracker_id))` over a session is **biased upward** (fragmentation + FPs + re-entry all push the count high) and gives no confidence interval. Fix it in five layers:

1. **Detection** — per-class confidence thresholds, optional soft-NMS, per-class temperature calibration
2. **Tracking** — ByteTrack params tuned for stationary livestock, plus a min-track-length filter and centroid-jump sanity check (NOT in supervision today)
3. **Counting** — pick the policy that matches the deployment (GPS dedup / Chapman / N-mixture / line-cross / median-of-active-IDs)
4. **Aggregation** — combine multi-pass / multi-cam observations into one number with a real CI
5. **Validation** — conformal prediction for a CI; reconcile against the EID/RFID register when present (uniquely available to herd-scout)

Realistic accuracy target: **±5–10% MAE** on pasture-sized herds under good conditions.

## Findings per sub-question

### 1. What systematically causes inaccurate counts?

[[counting-failure-modes]] catalogs the failure modes; the count error is **biased upward** under a naive unique-IDs counter because the three highest-impact mechanisms all push the same direction:

1. **Track fragmentation (flicker)** — same animal becomes two `track_id`s when detector drops it for >`track_buffer` frames. **Largest single error source.**
2. **False positives on hay bales / shadows / static clutter** — gets a track, gets counted.
3. **Re-entry double-count** — animal leaves frame, returns after `track_buffer` expires, gets new ID.
4. **Occlusion in tight groups** — back-row animal not detected at all (under-count, partly recovered if brief).
5. **Centroid teleport on ID reassignment** — NVIDIA NvDCF case; centroid jumps 300–600 px in one frame. Fix: drop frames where Δcentroid > V_max·Δt.
6. **ID switches without teleport** — near-zero impact on a unique-IDs counter (two animals just swap labels).

### 2. Track-to-count algorithms

[[supervision-counting-api]] details what supervision provides; the policy choice should match the deployment:

| deployment | counting policy | why |
|---|---|---|
| Drone single flyover, no GPS | max-distinct-IDs over session | simplest; biased high |
| Drone single flyover with GPS | world-coord dedup | cheapest correct answer; we have phone GPS |
| Drone multi-pass, ReID-capable | Chapman | gives a real CI |
| Drone multi-pass, no ReID | Royle 2004 N-mixture | also gives detection probability |
| Static cam, stationary herd | median(simultaneous active) over 30-frame window | bounds ID-switch inflation |
| Static cam, gate/race | LineZone with `minimum_crossing_threshold=2..3` | natural event |
| Multi-camera overlapping | POOL hand-off (IoU>0.2 in overlap region) | lighter than ReID embeddings |

**Default for stationary herds**: `median(len(unique active tracker_ids))` over a 30-frame window where each ID has ≥15 cumulative frames. Cumulative max-distinct-IDs over a long window over-counts.

### 3. Tracking metrics — what to optimize

[[tracking-metrics-and-tuning]]: target **HOTA** (specifically AssA), not MOTA. MOTA charges 1 per ID switch but rewards each correct frame-detection — a tracker that fragments every animal into 4 tracklets but detects them all loses only ~5% MOTA while the count is 4× wrong. HOTA's geometric-mean structure punishes that correctly.

For ByteTrack (supervision API) on stationary livestock at 30 FPS:

| param | default | herd-scout |
|---|---|---|
| `track_activation_threshold` | 0.25 | **0.35** |
| `lost_track_buffer` | 30 (1 s) | **60 (2 s)** |
| `minimum_matching_threshold` | 0.8 | **0.85** |
| `minimum_consecutive_frames` | 1 | **3** |

Plus two herd-scout-specific filters before counting:
- **Cumulative-frame-count filter**: ID eligible only after ≥15 cumulative frames
- **Centroid-jump sanity check**: drop frames with Δcentroid > V_max·Δt (V_max ≈ 150 px/frame)

### 4. When detection breaks down (dense regime)

[[density-counting-fidtm-p2pnet]]: crossover ~10–15 px per-instance size or >50 instances per megapixel. For dense sheep flocks at feeders, detection-only counts collapse.

Recommended fallback: **YOLO11s (sparse) + FIDTM-on-dense-ROI** hybrid. FIDTM has MIT code AND open weights (unlike HerdNet, P2PNet); plain conv+HRNet graph exports cleanly to ONNX; output is points (not blobs) so it composes naturally with ByteTrack downstream.

### 5. Multi-pass aggregation

[[multipass-aggregation]] decision tree:

- **Single drone flyover with GPS** → world-coord dedup (project bbox center via altitude+FOV, cluster by Euclidean distance)
- **Multi-pass with ReID** → Chapman: `N̂_C = ((n+1)(K+1)/(k+1)) − 1`
- **Multi-pass without ReID** → Royle 2004 N-mixture: jointly estimate λ and detection probability p
- **Bootstrap CI** for any policy: resample frames within window 1000× → 95% CI

Quantified evidence (MegaDetector wildlife pipeline): permissive IoU + larger track buffer cut tracks-per-animal from ~4 to ~1.2.

### 6. Livestock-specific accuracy

[[livestock-cv-accuracy]]: realistic numbers from the literature.

- **Achievable**: precision 0.90–0.95, recall 0.85–0.92, mAP 0.85–0.95, counting MAE ±5–10%
- **Sweet-spot altitude**: 30–60 m AGL (50 m is the canonical anchor; above 80 m AP collapses; below 30 m drone disturbance dominates)
- **Stay RGB**: thermal F1 = 0.64 vs RGB F1 = 0.89 on cattle
- **Pretrain stack**: Aerial Sheep (4,133 imgs, public domain) + Shao cattle (670 imgs) + Ocholla Kenya (cattle/sheep/goats)
- **Closest published architecture**: CSD-YOLOv8s (95.2% precision, 93.1% mAP, 87 FPS on dense oblique sheep)

### 7. Validation without dense ground truth

[[count-validation-conformal]] three-tier strategy:

**Tier 1 — EID present** (uniquely available to herd-scout via existing ISO 11784/11785 readers):
- `N_eid` = ground truth from ear-tag scans
- Audit residual `Δ = N_cv − N_eid`
- Use Lincoln-Petersen with EID-known animals as "marks" → derive site-specific detection probability `p̂` with NO frame labeling
- **No published work specifically reconciles EID streams with live CV counts. Publish-worthy differentiator.**

**Tier 2 — Conformal prediction**:
- Split conformal on a 200-frame per-site calibration set
- Distribution-free `247 ± 8 (95%)` interval
- Wraps any black-box detector

**Tier 3 — UX**:
- Confidence chip: 🟢 EID-reconciled / 🟡 conformal / 🔴 closure violated
- Tap-to-verify mode (Huang ICCV 2023): tap a region, pick a count *range*. Leverages human subitizing. 30–40% MAE reduction with minimal user input.

### 8. Confidence calibration

[[confidence-calibration]]:

- **Per-class thresholds** are the highest-leverage cheap fix. F1-peak from 200 labeled frames per class. Cold-start: cow=0.30, horse=0.30, sheep=0.20.
- **Soft-NMS** (Bodla et al.) is worth re-exporting without `nms=True` IF validation shows crowded under-count. Cheaper interim: try raising embedded NMS IoU to 0.75 first.
- **Per-class temperature scaling** via NetCal makes E[count] unbiased without changing the model. One scalar per class.
- **Soft counting**: `count = Σ p_i_calibrated` over surviving (soft-)NMS detections — unbiased in expectation if calibration is good.

## Actionable steps for herd-scout (priority order)

These are sized to land in ~1–2 sprints each. They build on the existing CV sidecar (Phase 2 from [[cv-sidecar-bench-2026-05-27]]).

### P0 — wins this week, no model retrain

1. **Tune supervision.ByteTrack params**: set `track_activation_threshold=0.35`, `lost_track_buffer=60`, `minimum_matching_threshold=0.85`, `minimum_consecutive_frames=3`. One-line config in the sidecar.
2. **Add min-track-length and centroid-jump filters** in the daemon's count post-processor (new ~30 LOC). A `tracker_id` is eligible only after ≥15 cumulative frames AND no Δcentroid > 150 px/frame in its history.
3. **Switch the reported count from `len(set(tracker_id))` to `median(active_ids_per_frame)` over the last 30 frames.** Bounds ID-switch inflation.
4. **Add per-class confidence thresholds** (cow=0.30, horse=0.30, sheep=0.20). Plumb into the sidecar export of the YOLO config.

### P1 — validation week, ~200 labeled frames

5. **Capture and label 200 frames** at the deployment site (admin app already has the recording infra per Wave 12). Annotate with Shao schema (Normal/Truncated/Blurred/Occluded).
6. **Run `yolo val`** on the labeled set; pick F1-peak per class as the production threshold. Replace P0 #4 cold-start values with measured ones.
7. **Fit per-class temperature scaling** via NetCal `LogisticCalibration(detection=True)`. Ship as a scalar lookup applied post-NMS.
8. **Bootstrap a 95% CI** on the count (~30 LOC). Resample frames within window 1000×.

### P2 — EID reconciliation (the differentiator)

9. **Wire the ISO 11784/11785 reader stream** (per [[livestock-eid-rfid]]) to the count post-processor. When `N_eid` is available, compute residual and confidence chip color.
10. **Publish the EID-CV reconciliation algorithm** as an open OSS spec — herd-scout's positioning angle.
11. **Implement Lincoln-Petersen with EID-known animals as marks** to derive site-specific `p̂`. Log over time as model-health metric.

### P3 — multi-pass / multi-cam (when deployments demand it)

12. **GPS-anchored world-coord dedup** for drone single-flyover. Project bbox center to lat/lon via altitude + FOV intrinsics.
13. **Royle N-mixture** for multi-pass without ReID. Use `unmarked` R or port to Python.
14. **POOL hand-off** for multi-cam overlapping deployments. Homography-aligned overlap regions, IoU > 0.2.

### P4 — dense-regime fallback

15. **Density-branch prototype**: trigger when local box density exceeds threshold T; crop the dense ROI; run a FIDTM-style point head; replace YOLO count for that region with point count. MIT license, open weights, ONNX-exportable.

### P5 — long-term

16. **Fine-tune YOLO11s on local pasture data** (per the recommendations in [[livestock-cv-accuracy]]).
17. **Appearance-based ReID** (ResNet50 embeddings) — kills re-entry double-counting, enables Chapman across longer sessions.
18. **Tap-to-verify UX** with Huang region-and-range correction.

## Examples

**Naive, biased upward** (today):
```python
# herd_count = number of distinct tracker_ids ever seen
all_ids = set()
for frame in stream:
    detections = sidecar.infer(frame)
    all_ids.update(detections.tracker_id)
return len(all_ids)
```
Fragmentation → +N% per spurious split. FPs on bales → +1 per static FP. Re-entry → +1 per re-entry.

**Layered, calibrated, with CI** (target):
```python
# 1. Per-class threshold + calibration applied in sidecar
# 2. ByteTrack tuned (track_activation_threshold=0.35, etc.)
# 3. Daemon-side filters
eligible_ids = {}  # tracker_id -> cumulative_frames
counts_per_frame = []  # rolling window
for frame in stream:
    detections = sidecar.infer(frame)  # already calibrated, per-class thresholded
    detections = drop_centroid_jumps(detections)  # > 150 px/frame
    for det in detections:
        eligible_ids[det.id] = eligible_ids.get(det.id, 0) + 1
    active = [d.id for d in detections if eligible_ids[d.id] >= 15]
    counts_per_frame.append(len(set(active)))

# 4. Reported count = median over rolling window
median_count = numpy.median(counts_per_frame[-30:])

# 5. Bootstrap CI
ci_low, ci_high = bootstrap(counts_per_frame[-30:], 1000, [2.5, 97.5])

# 6. EID reconciliation (when available)
if eid_count is not None:
    delta = median_count - eid_count
    chip_color = "green" if abs(delta) <= 1 else "amber"

return Count(value=median_count, ci=(ci_low, ci_high), chip=chip_color)
```

## Sources

Round 3, 2026-05-27 (this question):

- [[2026-05-27-supervision-counting-api]] — LineZone / PolygonZone / ByteTrack wrapper
- [[2026-05-27-tracking-metrics-and-tuning]] — HOTA + ByteTrack tuning
- [[2026-05-27-confidence-calibration]] — Soft-NMS, per-class, NetCal
- [[2026-05-27-counting-failure-modes]] — taxonomy + severity
- [[2026-05-27-shao-cattle-uav-dataset]] — canonical 50m AGL benchmark
- [[2026-05-27-ocholla-kenyan-livestock]] — multi-species pasture
- [[2026-05-27-csd-yolov8s-sheep]] — closest architecture to herd-scout's spec
- [[2026-05-27-multicam-cattle-tracking]] — POOL hand-off, density-adaptive IoU
- [[2026-05-27-density-counting-fidtm-p2pnet]] — when detection breaks down
- [[2026-05-27-multipass-aggregation]] — Lincoln-Petersen, N-mixture, GPS dedup
- [[2026-05-27-count-validation-conformal]] — conformal + EID reconciliation

Synthesis articles:

- [[herd-counting-pipeline]] — 5-layer pipeline (the concept article version of this playbook)
- [[livestock-cv-accuracy]] — realistic numbers from the literature

Existing infrastructure context:

- [[cv-sidecar-bench-2026-05-27]] — current Phase 2 baseline (23 FPS w/ ByteTrack)
- [[livestock-eid-rfid]] — ISO 11784/11785 readers (the Tier 1 ground truth)
- [[drone-vision-software]] — base CV stack
- [[2026-05-21-herdnet-deep-dive]] — HerdNet rejection (license + post-flight design)

## Suggested theses (verifiable claims for `--mode thesis`)

From the findings, three testable claims worth investigating later:

1. **"Switching the reported count from `len(set(tracker_id))` to `median(active_ids_per_frame)` over a 30-frame window reduces MAE on pasture cattle by >30% with no model retraining."** — testable on captured + labeled bench footage.
2. **"For a stationary herd at 30 FPS, ByteTrack with `track_activation_threshold=0.35`, `lost_track_buffer=60`, `minimum_matching_threshold=0.85`, `minimum_consecutive_frames=3` outperforms supervision defaults on HOTA by >10 points."** — testable with the labeled-frames captured for P1.
3. **"Reconciling live CV counts against the ISO 11784/11785 EID register via Lincoln-Petersen produces a tighter 95% confidence interval than split conformal prediction on the same calibration set."** — testable as soon as EID reader plumbing is in place.
