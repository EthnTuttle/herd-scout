---
title: "supervision counting API: LineZone, PolygonZone, ByteTrack wrapper"
source_url: https://supervision.roboflow.com/latest/detection/tools/line_zone/
type: docs
tags: [supervision, roboflow, bytetrack, line-zone, polygon-zone, counting, api]
created: 2026-05-27
confidence: high
---

# supervision counting API

## LineZone

- `LineZone(start, end, triggering_anchors=(TOP_LEFT,TOP_RIGHT,BOTTOM_LEFT,BOTTOM_RIGHT), minimum_crossing_threshold=1)`
- Maintains `in_count`, `out_count`, `in_count_per_class`, `out_count_per_class`. Cross-product of anchors against the line vector.
- `minimum_crossing_threshold` is a debounce knob: detection must remain on the opposite side for N frames before a crossing is committed. Authors note: "useful when dealing with unstable bounding boxes or when detections may linger on the line."
- Hard requirement: `tracker_id` must be present, else counting is silently skipped with a warning.
- Source-level (`line_zone.py`):
  - `crossing_history_length = max(2, minimum_crossing_threshold + 1)`; per-tracker `deque(maxlen=...)` of "left-side" booleans.
  - Crossing fires only when deque is full AND oldest state is unique (`history.count(oldest_state) == 1`) — one clean side-flip surrounded by N stable frames.
  - Region-of-interest band perpendicular to line endpoints: detections outside that perpendicular band ignored, so a finite line segment doesn't count things passing its extension.
  - If both left and right anchors of one bbox straddle the line, that frame is skipped — prevents double-trigger on objects sitting on the line.

## PolygonZone

- `PolygonZone(polygon, triggering_anchors=(BOTTOM_CENTER,))` — default anchor is bottom-center (good for ground-plane animals).
- Sets `current_count = int(np.sum(is_in_zone))` per `trigger()` call. **Per-frame, not cumulative** — no built-in unique-ID accumulation.
- Precomputed boolean mask sized to polygon bbox; lookup is `mask[y, x]` per anchor — O(N) per detection.
- No internal track-state, no debounce — application wires that itself by collecting `detections.tracker_id[is_in_zone]` into a Python `set`.

## supervision.ByteTrack wrapper

Defaults (note: differ from upstream paper):

| supervision name | upstream name | default | role |
|---|---|---|---|
| `track_activation_threshold` | `track_thresh` | 0.25 | min conf to start a new track |
| `lost_track_buffer` | `track_buffer` | 30 frames | grace frames before lost track is dropped |
| `minimum_matching_threshold` | `match_thresh` | 0.8 (IoU distance) | tighter = harder to match |
| `frame_rate` | `frame_rate` | 30 | scales `lost_track_buffer` |
| `minimum_consecutive_frames` | n/a (supervision-only) | 1 | tentative tracks suppressed until confirmed |
| `min_box_area` | upstream-only | n/a (10) | NOT exposed by supervision; filter upstream |

- `minimum_consecutive_frames` is the supervision-side track-confirmation gate: tentative tracks don't get a stable `tracker_id` exposed until they survive N frames. Designed to "prevent accidental tracks from false detection or double detection."
- Legacy `sv.ByteTrack` is deprecated in v0.28 (removal v0.30) in favor of an external `trackers` package's `ByteTrackTracker`.

## Canonical counting recipes (from `examples/`)

**Traffic analysis** (`examples/traffic_analysis/ultralytics_example.py`):
- `tracker_id_to_zone_id: dict[int,int]` records first-seen zone for each tracker.
- `counts: dict[zone_out, dict[zone_in, set[int]]]`.
- Final count = `len(set_of_tracker_ids)` per (in-zone, out-zone) pair — **canonical "max-distinct-IDs" idiom**.
- Uses `triggering_anchors=[Position.CENTER]`, default `sv.ByteTrack()`.

**count_people_in_zone** (`examples/count_people_in_zone/`):
- `confidence_threshold=0.5` filter applied **twice**: once via `model(conf=...)`, once via `detections.confidence > threshold`. Defensive double-gate.
- `imgsz=1280` for higher-resolution detection.
- **Does not use a tracker** — purely `current_count = zone.trigger()` per frame. The simplest viable path when "right now" count suffices.

## Why ingest

Authoritative API surface and the actual integration target for herd-scout's CV sidecar. The two reference patterns (max-distinct-IDs across zones; per-frame `current_count` without tracker) bracket the design space. `minimum_crossing_threshold` and `minimum_consecutive_frames` are the debounce knobs we should be using but aren't.

## Sources

- supervision LineZone docs + `line_zone.py` source
- supervision PolygonZone docs + `polygon_zone.py` source
- supervision tracker docs (`/latest/trackers/`)
- `examples/traffic_analysis/ultralytics_example.py`
- `examples/count_people_in_zone/ultralytics_example.py`
