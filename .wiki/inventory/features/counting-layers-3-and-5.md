---
title: "Feature: counting layers 3 and 5 — deployment policy + conformal + confidence chip"
type: feature-candidate
priority: p1
created: 2026-06-02
source: assess-herd-scout-2026-06-02
status: open
wiki_evidence:
  - concepts/herd-counting-pipeline
  - concepts/bootstrap-conformal-count-ci
  - output/playbook-accurate-herd-counting-2026-05-27
---

# Feature: counting layers 3 + 5

## Why P1

The CV pipeline today implements layers 1 (detection, YOLO11s + embedded NMS) and 2 (tracking, supervision.ByteTrack). Layers 3 (deployment-aware policy), 4 (multi-pass aggregation), and 5 (validation + confidence chip) are designs only. Without them, the upload-mode `report.json` is just a number — not the auditable count-with-confidence the wiki specifies.

Bumping layers 3+5 to P1 (vs layer 4 at P2) because:

- Layer 3 is a policy switch — small change, large behavior diff per deployment.
- Layer 5 conformal alone is ~30 LOC for a real 95% CI per site.
- Once `herd-scout-eid` ships, layer-5 EID-reconciliation residual becomes the primary wedge.

## Layer 3 — deployment-aware counting policy

| deployment | policy | implementation |
|---|---|---|
| Drone single flyover, no GPS | Max distinct IDs across frames | `set(tracker_id)` after layer-2 filters |
| Drone single flyover, with GPS | World-coord dedup | Project bbox center to lat/lon via altitude+FOV; cluster within ~1 animal radius |
| Drone multi-pass | (defer to layer 4 — P2) | Lincoln-Petersen / N-mixture |
| Static pasture cam, herd stationary | Median(simultaneous active count) over 30-frame window | per-frame `len(unique active tracker_ids)`, median |
| Static pasture cam, gate/race/funnel | LineZone with debounce | `LineZone(minimum_crossing_threshold=2..3)` |
| Multi-camera overlapping | POOL hand-off | IoU > 0.2 in overlap region, homography-aligned, shared global ID |

Default: "median of simultaneous active count over a stable window" for stationary herds.

## Layer 5 — split conformal + 🟢/🟡/🔴 chip

Three-tier validation:

1. **EID reconciliation** (when `herd-scout-eid` is present): `Δ = N_cv − N_eid`; Lincoln-Petersen with EID-known animals as marks → site-specific detection probability `p̂`.
2. **Conformal prediction** (no EID): split conformal on a 200-frame per-site calibration set → distribution-free interval. Wraps any black-box detector.
3. **Active learning loop**: random spot-checks (catch FNs) + low-confidence flags (catch FPs). Push to admin RPC plane.

Confidence chip:
- 🟢 EID-reconciled
- 🟡 Conformal-only
- 🔴 Closure violated (animals entered/exited mid-count) or `Δ` excessive

## Open questions

- Where do per-site calibration frames live in iroh-blobs/iroh-smol-kv?
- Layer-5 active learning needs a UI that doesn't yet exist (see [[../../wiki/concepts/herd-counting-pipeline]] §Layer 5 Huang ICCV-2023 region-and-range UX).
- Live mode vs upload mode aggregation differences (closure assumption is *valid* in upload mode) — see research-gap entry on offline-file counting.

## See also
- [[../../wiki/concepts/herd-counting-pipeline]]
- [[../../wiki/concepts/bootstrap-conformal-count-ci]]
- [[../../output/playbook-accurate-herd-counting-2026-05-27]]
- [[../../output/assess-herd-scout-2026-06-02]] §Opportunities
- [[herd-scout-eid-crate]]
