---
title: "Playbook: Robust herd counting from MOT outputs + phone-on-drone airframe"
type: playbook
created: 2026-06-01
question: "What 2025-2026 advances close the remaining gaps in the 5-layer counting pipeline, and what concrete airframe spec do we need to ship a phone-on-drone payload?"
sources:
  - 2026-06-01-ultralytics-tracking-defaults
  - 2026-06-01-oc-sort-cao-2022
  - 2026-06-01-boxmot-sam2-tooling
  - 2026-06-01-busca-vaquero-eccv-2024
  - 2026-06-01-multicamcows2024-yu
  - 2026-06-01-politis-white-2004-block-bootstrap
  - 2026-06-01-jackknife-plus-after-bootstrap-kim-2020
  - 2026-06-01-angelopoulos-bates-conformal-2022
  - 2026-06-01-yolo26-ultralytics
  - 2026-06-01-grounding-dino-livestock
  - 2026-06-01-cotracker3-karaev-2024
  - 2026-06-01-ardupilot-vibration-damping
  - 2026-06-01-fpv-camera-mounting-tpu
  - 2026-06-01-wildair-drone-vibration-frequencies
  - 2026-06-01-android-thermal-api
  - 2026-06-01-android-sustained-performance-mode
  - 2026-06-01-pixel-thermal-throttle-empirical
  - 2026-06-01-android-foreground-service-types
  - 2026-06-01-phone-power-from-drone
  - 2026-06-01-thingiverse-phone-drone-mounts
---

# Playbook — robust counting + phone-on-drone airframe

Two-half follow-up to the 2026-06-01 assess. The counting half is a layered upgrade on top of the existing [[herd-counting-pipeline]] (Round 3 work). The airframe half is the buildable spec missing from [[android-on-drone]].

## Half 1 — Counting upgrades (priority order)

### P0 — costs nothing, ships now

1. **YOLO26 retrain with the legacy one-to-many head.** Drop-in replacement for YOLO11s; +43% CPU speedup; ProgLoss/STAL improves small-object recall. Keep `end2end=False` until ByteTrack BT-Low compatibility is empirically validated. See [[yolo26-and-tracker-compat]].

2. **Switch `report.rs` from frame-iid bootstrap to stationary block bootstrap (SB) with mean block length ≈ 10 frames at 30 FPS, BCa CIs.** Frame-iid is invalid for autocorrelated MOT output — neighbouring frames are nearly identical. SB + BCa is ~50 LOC. See [[bootstrap-conformal-count-ci]].

3. **Add the J+aB conformal interval on top of the same bootstrap ensemble.** ~30 LOC. Distribution-free predictive interval with finite-sample marginal coverage at most `2α`. Ship both intervals (variance and predictive) in the per-clip JSON report.

### P1 — A/B experiments, then deploy winner

4. **OC-SORT vs ByteTrack A/B on bunching/gate clips** via [[boxmot-multi-tracker-zoo|BoxMOT]]. Compute cost is near-zero (~700 FPS association on CPU). If OC-SORT wins on HOTA/AssA and the `|unique_ids| / |gt|` ratio: deploy as live-broadcast default.

5. **HIT post-hoc tracklet stitching on the upload-batch path.** Appearance-free, IoU-only, runs after the clip is fully ingested before [[herd-counting-pipeline|layer 4]] aggregation. Cheapest possible recovery upgrade. See [[track-recovery-busca-hit]].

### P2 — ReID (kills re-entry double-counting)

6. **Self-supervised cattle re-ID via tracklet contrastive learning.** Train a coat-pattern embedding on recorded tracklets — no per-cow labels needed. Mode A: trigger only at track creation + re-entry events, not per-frame. ~96% accuracy precedent on MultiCamCows2024. See [[cattle-reid-self-supervised]].

### P3 — research-grade, validate before shipping

7. **BUSCA online recovery on the upload-batch path.** Transformer Q&A "does this proposal extend track k?" Likely too heavy for live; fits the upload path where latency is unconstrained. Validate compute fit on Pascal first.

8. **BoT-SORT with GMC for the phone-on-drone (moving camera) deployment** when that lands. GMC is wasted on fixed cams but load-bearing for moving cameras.

### Skip

- **SAM 2 live**: not feasible on 6 GB Pascal alongside YOLO11s/26.
- **Open-vocab detectors (Grounding DINO, YOLO-World) as YOLO replacement**: zero-shot mAP 76.8% < fine-tuned YOLO 90%+. Server-side audit/labeling tool only.
- **CoTracker3 for lameness**: no published livestock benchmark. T-LEAP + BLSTM is the validated path (85% accuracy from 1s of video) if lameness is in scope.

## Half 2 — Phone-on-drone airframe (buildable spec)

### Mounting

- **Suspended (elastic-hanging) topology**, not corner or sandwich.
- **95A or 98A TPU printed tray**.
- **4× M3×8 mm silicone grommets, 50A durometer, 20–30% compression**.
- Optional 1/2"–3/4" silicone O-rings (outperform Buna-N).
- **Printed cage around the phone** (not just a clamp) — protects rear glass.
- 5" quad minimum, 7–10" comfortable.
- See [[phone-on-drone-airframe]].

### Vibration target

- 100–300 Hz Z-axis dominant.
- **Balance props first** — >300% improvement before any damper.
- Replace damping material every **6–12 months**; print TPU spares (50–100 hard landings life).

### Avoid

- Moon Gel (>100 °F failure).
- Hard plastic clamps as the only mount.
- Drone-mounted smartphone gimbals (Hohem, Feiyu) — 200–400 g + power burden, no community traction.
- Over-tight straps.

### Power

- Hardware encode confirmed (MediaCodec native, NOT OMX.google.* SW fallback). 3–4× power swing.
- **Pixel 6/7 internal battery covers a single 20–35 min flight.**
- Multi-flight days: drone-LiPo → buck (~25 W) → USB-C PD source IC (FUSB302) → **9 V / 3 A = 27 W** to phone.
- Charge limit 80% (Pixel Adaptive Charging / Samsung Protect Battery).
- Replace donor phones every 200–400 cycles (annual budget).
- See [[phone-power-on-drone]].

### Thermal

- **Donor phone choice**: Pixel 6 Pro or Pixel 7 Pro (vapor chamber, large chassis). **Avoid Pixel 9 Pro non-XL** (smaller chassis = worse thermals).
- Wire `addThermalStatusListener`:
  - `MODERATE` → bitrate 4 → 2 Mbps
  - `SEVERE` → FPS 30 → 15, resolution 720p → 480p
  - `CRITICAL` → stop upload
- `setSustainedPerformanceMode(true)` at startup. Real measured trade: 18% peak / 2× sustained.
- WiFi ground station > direct LTE under thermal pressure (LTE dies at `EMERGENCY`).
- 720p30 hardware H.264 is **much** lower thermal load than the 4K60 community numbers; verify per-airframe.
- See [[phone-thermal-management]].

### Foreground service (Android 14/15/16)

- Manifest: `foregroundServiceType="camera|connectedDevice"` — **NOT `dataSync`** (6-hour cap on API 35+).
- Permissions: `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CAMERA`, `FOREGROUND_SERVICE_CONNECTED_DEVICE`, `CAMERA`, `CHANGE_WIFI_STATE`.
- Start from a foreground/visible activity, not `BOOT_COMPLETED`.
- Grant runtime perms **before** `startForeground()` or SecurityException.
- Test on the actual donor phone model — Xiaomi/OnePlus/OPPO OEMs often kill FGS regardless of spec.
- See [[phone-publisher-android-fgs]].

## Open gaps surfaced by this round

These are durable follow-ups worth their own future research/inventory items:

1. **No academic UAV paper on smartphone payload mounting** — herd-scout could publish the first.
2. **Block-length adaptation for the Politis-Romano stationary bootstrap** — implementing the spectral-density plug-in is the right next step beyond the `n^(1/3)` heuristic.
3. **Compute fit of BUSCA on Pascal** — needs benchmarking; if it fits, it's a major fragmentation-error reduction.
4. **MultiCamCows2024 license is CC BY-NC-SA 4.0** — for any commercial deployment, a separate clean-licensed cattle re-ID dataset is needed, OR site-specific self-supervised training (the recommended path).
5. **Mobile encode H.264 vs HEVC vs AV1 thermal benchmarks** on 2025-era Snapdragon/Tensor — gated behind paywalls; do this measurement on the actual donor phone.
6. **OEM kill-FGS behavior empirical map** — cross-reference dontkillmyapp.com per donor phone model.
7. **T-LEAP + BLSTM for cattle lameness** — orthogonal to counting but a known-validated path if lameness becomes a product surface.

## Suggested theses (future `/wiki:research --mode thesis`)

1. **"OC-SORT outperforms tuned ByteTrack on cattle bunching scenes by >5 HOTA points with no compute increase."** Testable via BoxMOT on captured gate-bunching clips.
2. **"Stationary block bootstrap with mean block length ≈ 10 frames produces wider but better-calibrated 95% CIs than frame-iid bootstrap on autocorrelated MOT output."** Testable on labeled bench footage.
3. **"Self-supervised tracklet-contrastive cattle re-ID at track-creation events alone (Mode A) reduces herd-scout's re-entry double-count rate by >50% with <10% compute overhead on Pascal."** Testable as soon as the tracklet pipeline is wired.

## See also

- [[herd-counting-pipeline]] — the 5-layer pipeline this extends
- [[livestock-cv-accuracy]] — accuracy bounds the upgrades push against
- [[android-on-drone]] — verdict layer the airframe half builds on
- [[../output/playbook-accurate-herd-counting-2026-05-27|playbook-accurate-herd-counting-2026-05-27]] — Round-3 predecessor
