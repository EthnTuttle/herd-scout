---
title: "Multi-pass aggregation: Lincoln-Petersen, N-mixture, GPS dedup, multi-cam hand-off"
source_url: https://en.wikipedia.org/wiki/Mark_and_recapture
type: synthesis
tags: [lincoln-petersen, chapman, n-mixture, royle, gps-dedup, multi-camera, distance-sampling, aggregation]
created: 2026-05-27
confidence: high
---

# Multi-pass aggregation methods

Methods for combining counts from multiple flyovers, sliding-time-window observations, or multi-camera deployments into one canonical count with a confidence interval.

## Lincoln-Petersen / Chapman estimator

For two passes when individuals can be re-identified across passes (coat patterns, ResNet ReID, ear tags).

- Lincoln-Petersen: `N̂ = nK/k` where n = marked on visit 1, K = caught on visit 2, k = recaptures.
- **Chapman correction (preferred — less biased on small samples):**
  `N̂_C = ((n+1)·(K+1) / (k+1)) − 1`, then truncate.
- Assumptions: closed population, equal catchability, no mark loss, accurate ID.
- CI: normal approximation with continuity correction; bootstrap recommended for small n.

## Royle 2004 N-mixture model

When you have multiple flyovers but **cannot re-ID** individuals — exactly herd-scout's YOLO+tracker output across consecutive passes.

- Process: `N_i ~ Poisson(λ)` for site i.
- Observation: `y_it ~ Binomial(N_i, p)` for replicate t.
- Estimates true abundance N **and** detection probability p **jointly** from repeated noisy counts at the same sites — no individual marking required.
- Implementation: R package `unmarked`. UAV-tailored extension exists (Wiley 2041-210X.14054).

## Distance sampling (line-transect)

For wide-FoV drone passes where detection drops with distance from flight line.

- Half-normal detection: `g(y) = exp(−y² / 2σ²)`
- Density: `D̂ = n / (2·L·ESW)` where ESW (effective strip width) = `∫₀^w g(x) dx`
- Line-transect framework integrates flight-line geometry with detection-probability decay — the "right" framework for a single drone pass over a large area.

## GPS-anchored dedup (single flyover, overlapping frames)

ScienceDirect S0168169921003719 — graph-based deduplication using GPS-tagged image overlap to avoid double-counting cattle across adjacent drone images.

- Project each bbox center to a world coordinate via altitude + FOV intrinsics.
- Cluster world coords across overlapping frames by Euclidean distance threshold (~1 animal-radius).
- Cheapest "right answer" for a single drone flyover with overlapping frames — herd-scout already has phone GPS.

## Multi-camera POOL hand-off

See `2026-05-27-multicam-cattle-tracking` for full detail. Summary:
- Each camera defines an overlap region (POOL).
- IoU > 0.2 between adjacent cameras' bboxes (homography-aligned to ground plane) → assign same global ID.
- Lighter than ReID-embedding-based multi-cam matching; works without an appearance model.

## Wildlife tracking pipeline (MegaDetector + ByteTrack + ResNet50 ReID)

Real-world deployment (`juanmmaidana.github.io/posts/wildlife-tracking-part1/`):
- Pipeline: detect → ByteTrack → ResNet50 ReID
- Aggregation: **majority-vote species per track, then count unique tracks**
- Key trick: **ultra-permissive IoU thresholds dropped tracks-per-animal from ~4 to ~1.2** — quantified evidence that fragmentation is the dominant bias
- 95.7% test accuracy, 17× faster than manual

## Decision tree

```
Single drone flyover (drone passes paddock once):
  IF overlapping frames have GPS:
    → GPS-anchored world-coord dedup. Cluster by ~1 animal radius.
  ELSE:
    → Tracker-based "count distinct IDs" with:
       (a) min-track-length filter (≥ K hits, K ≈ 5–10),
       (b) permissive IoU to avoid fragmentation,
       (c) report as point estimate.
       Biased high (fragmentation) AND low (occlusion); accept it as v1.

Multiple flyovers (out-and-back, repeat passes):
  IF can Re-ID individuals (coat patterns, ear tags, ResNet ReID):
    → Chapman: N̂ = ((n1+1)(n2+1)/(m+1)) − 1
       where n1, n2 are counts on each pass, m = matched individuals.
       CI by bootstrap or normal approx.
  ELSE (just counts y_1..y_T per pass):
    → N-mixture (Royle 2004): MLE of (λ, p) from
       y_t ~ Binomial(N, p), N ~ Poisson(λ).
       Use R `unmarked` package.
       Bonus: also gives detection probability estimate p.

Static pasture cam, hours-long window:
  → Sliding-window MAX of distinct ACTIVE track IDs
     (active = currently visible, not all-time cumulative).
     Track IDs that survive ≥ K frames AND ≥ T seconds.
  → If a gate/funnel exists: line-crossing counter (supervision LineZone).
  → If herd is mostly stationary: max simultaneous count over a stable
     window (per-frame count smoothed by median-of-N).

Multi-camera (overlapping pasture cams):
  → Spatial hand-off via overlap regions (POOL pattern).
     IoU > 0.2 between cams' overlap-region bboxes → share global ID.
  → If non-overlapping: ResNet/ReID embeddings + Hungarian matching
     across simultaneous frames.
  → Final count = unique global IDs surviving track-length filter.
```

## Cheapest upgrades to "max distinct IDs" (priority order)

1. **Min-track-length / hit-streak filter** — drop track IDs with < K confirmed hits (K ≈ 5–10). Kills spurious flicker tracks. ByteTrack already exposes this.
2. **Permissive IoU + larger track buffer** — cuts fragmentation 4× → 1.2× (quantified by MegaDetector wildlife pipeline).
3. **MAX of simultaneous active count over a stable window** rather than cumulative ID count — bounds the ID-switch inflation. Smooth with a 5-second median.
4. **GPS world-frame dedup** when phone GPS is available (drone case).
5. **Bootstrap interval for the count** — resample frames within window 1000× → 95% CI. Cheap and honest.
6. **Opportunistic Chapman** when user does an out-and-back with ReID-capable input.

## Why ingest

Decision tree maps every herd-scout deployment scenario (single flyover, multi-flyover, fixed cam, multi-cam) to a specific aggregation method with a published statistical foundation. Replaces ad-hoc "max distinct IDs" with the right method per case, plus a confidence interval.

## Sources

- Mark & Recapture, Wikipedia + ecology stats canon
- Royle 2004, Biometrics, "N-mixture models"
- Distance Sampling primer: NPS + numberanalytics.com
- ScienceDirect S0168169921003719 — Cattle counting in the wild with geolocated aerial images
- juanmmaidana.github.io/posts/wildlife-tracking-part1/ — MegaDetector wildlife tracking
- TDS — Object Counting in Videos (line-crossing)
- PMC11861714 — POOL multi-cam hand-off (also see `multicam-cattle-tracking`)
