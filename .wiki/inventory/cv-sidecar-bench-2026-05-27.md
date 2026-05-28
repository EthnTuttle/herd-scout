---
title: "CV sidecar bench — Phase 0/1/2 numbers"
type: inventory
created: 2026-05-27
updated: 2026-05-27
host: bigdeal (MSI GS63VR, GTX 1060 6GB Mobile, i7-7700HQ)
plan: output/plan-optimize-cv-sidecar-trt-yolo11s-2026-05-26.md
---

# CV sidecar bench numbers

Synthetic 1280×720 BGR24 frames, 200/run, single-client over `/run/herd-scout/cv.sock`. Phone publish is 720p; 1280×720 is a fair stand-in for the real load. `dets/frame=0` because the test pattern is RNG noise — the bench measures the daemon-frame round-trip floor, not the NMS load. Real frames with detections have a *lower* postprocess cost on Phase 1+ (fewer per-row operations) and the same cost on Phase 2 (tracker scales with N detections, but N is small).

Bench harness: `deploy/cv-sidecar/bench.py`.

## Phase 0 — yolov5n.onnx + cv2.dnn.NMSBoxes (baseline, pre-optimization)

| metric | value |
|---|---|
| throughput | **5.6 FPS** |
| RTT p50 / p95 / p99 | 172 / 174.9 / 186.0 ms |
| pre / run / post | 6.4 ms / 18.0 ms / 145.8 ms |
| model | yolov5n.onnx (fp16 input) |
| postprocess | numpy mask + cv2.dnn.NMSBoxes per class |

Verdict: **fails the 10 FPS gate.** post-process dominates by 8x.

## Phase 1 — yolo11s.onnx with `nms=True` (in-graph NMS)

| metric | value |
|---|---|
| throughput | **33.5 FPS** |
| RTT p50 / p95 / p99 | 22.7 / 23.5 / 24.7 ms |
| pre / run / post | 1.8 ms / 19.0 ms / 0.2 ms |
| model | yolo11s.onnx (fp32 input, `nms=True`, [1, 300, 6] output) |
| postprocess | tensor slice + class filter + scale-back |

Verdict: **passes 10 FPS gate by 3.3×.** post-process collapsed 730×. The Phase 3 TensorRT path stays gated out per the plan's decision rule.

## Phase 2 — Phase 1 + supervision.ByteTrack tracker, wire-format `track_id`

| metric | value |
|---|---|
| throughput | **23.1 FPS** |
| RTT p50 / p95 / p99 | 52.0 / 53.6 / 54.7 ms |
| pre / run / post | 2.0 ms / ~24-50 ms / 0.3-0.5 ms |
| model | (same as Phase 1) |
| postprocess | sv.Detections + sv.ByteTrack.update_with_detections |
| wire | DET_PACK = `<IIfffff>` (28 B/det), `track_id u32` after `class_id` |

Verdict: **passes 10 FPS gate by 2.3×.** ~30% throughput regression vs. Phase 1, primarily from `run` becoming noisy (some frames at 50 ms vs. Phase 1's flat 19 ms). The bench client and the sidecar are sharing the GPU host CPU; some of the variance is just system noise. ByteTrack's CPU cost itself is sub-ms when N detections is small.

Trade-off accepted because: (a) every frame still beats the 10 FPS budget, (b) tracker IDs are useful downstream (counting, stable bbox colors), (c) supervision adoption was already on the roadmap for license clarity.

## Reproduce

```
ssh bigdeal
sudo systemctl stop herd-scout-daemon.service       # frees the sidecar's single client slot
/srv/aiworker/.venv/bin/python /srv/aiworker/herd-scout/deploy/cv-sidecar/bench.py \
    --frames 200 --width 1280 --height 720 --det-pack-size 28
sudo systemctl start herd-scout-daemon.service
```

For a Phase-0-style bench (pre-track_id wire format, on a hypothetical rollback), pass `--det-pack-size 24`. The currently-deployed sidecar is Phase 2.

## Files

- `/srv/aiworker/yolo11s.onnx` — Phase 1+ production model (38 MB).
- `/srv/aiworker/yolov5n.onnx` — Phase 0 fallback, kept on disk for emergency rollback (4 MB).
- `/etc/herd-scout-cv-sidecar.env` — toggle `CV_SIDECAR_MODEL=` between them to roll back.

## Watch items

- supervision 0.30 will remove `sv.ByteTrack`. We're on 0.28. Audit before any bump.
- Ultralytics `nms=True` exports default to fp32 input; we lost the small fp16 wins from Phase 0. Acceptable today; revisit if power consumption matters.
- The "run" variance on Phase 2 (19 → 50 ms occasionally) might be matplotlib's lazy import warming pages on first tracker call. If we ever care about consistent latency below 30 ms, eager-import in `setup_session()`.

## GPU memory cap probe (2026-05-27, post-Phase-2)

After observing `pyannote-on-fredco-audit` co-tenant on the same GPU (2.1-5.3 GiB transient), we wired `gpu_mem_limit` and `CV_SIDECAR_FORCE_CPU` toggles into the sidecar (Option A from `~/.claude/plans/ok-bigdeal-...md`). The initial cap was 768 MiB (guessed at 2.5x the then-observed 312 MiB steady-state). Empirical probe to derive a defensible value:

| cap (MiB) | result | actual VRAM (MiB) | throughput (FPS) |
|---|---|---|---|
| 64 | crash (mid-bench) | 68 | n/a |
| 128 | crash (first frame) | 68 | n/a |
| 144 | crash (first frame) | n/a | n/a |
| 160 | OK | 246 | 15.0 |
| 176 | OK | 262 | 15.4 |
| 192 | OK | 278 | 15.1 |
| 256 | OK (1000-frame sustained) | 342 | 30.5 |
| 384 | OK | 340 | 15.5 |
| 512 | OK | 340 | 14.6 |
| 768 | OK | 342 | 14.5 |

Failure mode below the floor: `BFCArena AllocateRawInternal: Available memory of 0 is smaller than requested bytes of 21504000` during the `/ArgMax` node (part of the embedded-NMS path). The session loads, the first inference call OOMs, the python process dies, the daemon's connection drops. The CUDA → CPU EP fallback in the providers list does NOT recover this — the EP is already attached, the per-call arena is what runs out.

**Empirical floor: 160 MiB. Shipped default: 256 MiB (floor + ~60% safety, power-of-two-friendly).**

`gpu_mem_limit` only constrains ORT's arena. Total VRAM held includes ~140 MiB of model weights + cuDNN workspace + CUDA stream state that ORT doesn't manage. So `cap=256 MiB → ~340 MiB total nvidia-smi VRAM`. Do not lower below 160 MiB without re-probing.

Throughput numbers above are noisy because fredco-audit was concurrently running pyannote diarization (its own VRAM swung between 2.4-5.3 GiB across the probe). The 256 MiB cap held a clean 30.5 FPS over a 1000-frame sustained bench.
