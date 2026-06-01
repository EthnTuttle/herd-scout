---
title: "BoxMOT — multi-tracker experimentation harness"
summary: "Reference for the boxmot pip package used to A/B trackers (BoT-SORT, OC-SORT, Deep OC-SORT, OccluBoost) without rewriting the herd-scout sidecar"
tags: [boxmot, tracking, reference, tooling]
created: 2026-06-01
confidence: high
type: reference
---

# BoxMOT — reference

Repo: https://github.com/mikel-brostrom/boxmot

Maintainer: mikel-brostrom. **8.2k stars, v20.0.0 May 2026, 145 releases.** Active.

## Trackers supported

- BoT-SORT
- ByteTrack
- OC-SORT
- Deep OC-SORT
- StrongSORT
- HybridSORT
- SFSort
- BoostTrack
- **OccluBoost** — leads MOT17 at 70.47 HOTA

## Plug-in API

```python
from boxmot import Boxmot

tracker = Boxmot(
    detector="yolov8n",
    reid="osnet_x0_25_msmt17",
    tracker="botsort",  # or "bytetrack", "ocsort", "deepocsort", "occluboost"
)
```

Detector + ReID + tracker are independently swappable.

## Usage in herd-scout's experiment workflow

For the OC-SORT vs ByteTrack A/B proposed in [[tracker-choice-bot-oc-byte]]:

1. Capture/keep a labeled bunching/gate clip (the failure-mode case).
2. Run YOLO11s detection through BoxMOT with `tracker="bytetrack"` and `tracker="ocsort"`.
3. Compare HOTA, AssA, and the herd-scout-specific `|unique_track_ids| / |gt_animals|` ratio per [[herd-counting-pipeline]].
4. If OC-SORT wins: re-implement the same logic against the production sidecar's tracker hook (or wrap BoxMOT directly in the sidecar).

## Why this isn't the production tracker

The herd-scout production sidecar uses **`supervision.ByteTrack`** directly — minimal Python deps, predictable on Pascal. BoxMOT is a heavier dep tree intended for **research / experimentation**, not as the live tracker.

After A/B identifies a winner, port the chosen algorithm — don't ship BoxMOT itself unless its dependency footprint is acceptable.

## See also

- [[tracker-choice-bot-oc-byte]] — the experiment design
- [[track-recovery-busca-hit]] — orthogonal recovery layer

## Sources

- raw: [[2026-06-01-boxmot-sam2-tooling]]
