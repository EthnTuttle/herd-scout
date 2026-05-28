---
title: "Plan: Optimize the herd-scout CV sidecar — YOLO11s + embedded NMS + TRT 8.6 sm_61"
type: plan
format: roadmap
sources:
  - .wiki/wiki/concepts/drone-vision-software.md
  - ../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/farm-vision-on-gtx-1060.md
  - ../../wiki/topics/gtx-1060-headless-ai-server/raw/articles/2026-05-21-yolov8-yolo11-specs.md
  - ../../wiki/topics/gtx-1060-headless-ai-server/raw/repos/2026-05-21-supervision.md
  - ../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/pascal-driver-cuda-pinning.md
  - .wiki/output/plan-deploy-daemon-on-1060-laptop-2026-05-22.md
generated: 2026-05-26
---

# Plan: Optimize the herd-scout CV sidecar — YOLO11s + embedded NMS + TRT 8.6 sm_61

> Generated from [herd-scout](/_index.md) + [gtx-1060-headless-ai-server](../../../wiki/topics/gtx-1060-headless-ai-server/_index.md) wikis

## Executive Summary

The CV sidecar pivot just shipped: phone → iroh-moq → daemon → Python sidecar (CUDA EP YOLOv5n) → detections, end-to-end on bigdeal at `run=18 ms/frame` GPU inference. The remaining bottleneck is `post=150–180 ms/frame`, all of it numpy + `cv2.dnn.NMSBoxes` on a CPU core. Inference (18 ms) is plenty fast for the daemon's 10 FPS budget; postprocess alone consumes 1.5–1.8 frame budgets.

This plan eliminates that bottleneck in three escalating phases — each is independently shippable, and we stop as soon as we hit sustained 10 FPS:

1. **Re-export YOLO11s with NMS in the ONNX graph** so the sidecar's postprocess collapses from "all of cv2" to "decode a fixed-shape (top-k, 6) tensor". Single biggest payoff per hour of work.
2. **Adopt `supervision` (MIT) + ByteTrack** to add persistent track IDs and a clean abstraction layer, while escaping Ultralytics' AGPL on the runtime side. Same pass also adds a wire-protocol `track_id` field for downstream counting.
3. **Build a TRT 8.6.x sm_61 engine with the `EFFICIENT_NMS` plugin** if and only if Phase 1 doesn't clear the 10 FPS bar. Pure GPU-side NMS, post drops to <1 ms.

The wiki recommends YOLO11s over YOLOv5 directly — same parameter count (9.4M), higher mAP (47.0 vs 45.7 COCO), better Pascal-era ergonomics. The wiki also explicitly flags TRT 10 as Pascal-incompatible, so any TRT path is gated to 8.6.x.

## Architecture Decisions

### Decision 1: Embedded NMS over CPU postprocess

**Context**: The sidecar currently runs YOLOv5's raw `[1, 25200, 85]` head through Python: numpy multiply, argmax, mask, then a per-class `cv2.dnn.NMSBoxes` Python-list conversion loop. Measured `post=150–180 ms`.

**Options considered**:
- **Vectorize the existing numpy NMS** (drop cv2, write per-class IoU in numpy) — small win, still CPU.
- **Re-export the model with NMS in the ONNX graph** (`yolo export format=onnx nms=True`) — output becomes a fixed `[1, top_k, 6]` tensor (xyxy, conf, class). Zero Python postprocess. CUDA EP can run the embedded NMS op set on-GPU.
- **Defer NMS to a TRT plugin** — biggest payoff, biggest yak.

**Decision**: Re-export. The wiki ([farm-vision-on-gtx-1060](../../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/farm-vision-on-gtx-1060.md)) recommends YOLO11s for Pascal 6GB, and Ultralytics' export script supports `nms=True` for YOLO11. We get model-upgrade + post-collapse in one move.

**Consequences**:
- The wire format from sidecar to daemon doesn't change (`(class, conf, x1, y1, x2, y2)` per detection); only the *production* of those rows moves from Python to ORT.
- The model output dtype path needs verification — Ultralytics' `nms=True` typically forces fp32 input; if so, we lose the small fp16 input win we have today.
- Any hand-tuned thresholds (score=0.25, IoU=0.45) bake into the export and need re-export to change. Acceptable — these are stable defaults.

### Decision 2: `supervision` for the runtime, not Ultralytics

**Context**: The wiki is explicit ([supervision raw](../../../wiki/topics/gtx-1060-headless-ai-server/raw/repos/2026-05-21-supervision.md)): "for anything that might ship: `supervision` for license clarity." Ultralytics ships under AGPL-3.0; herd-scout's other code is permissively licensed.

**Options considered**:
- Keep raw numpy + cv2 NMS as today (no library) — works, but we re-implement what supervision already provides.
- Adopt Ultralytics' inference + tracking — pulls AGPL into the daemon's runtime closure even though the model is portable.
- **Adopt supervision (MIT) for tracking, zone primitives, and `Detections` abstraction** — model-agnostic, supports Ultralytics + RT-DETR + Detectron2 + SAM converters. ~38k stars, 1M+ monthly PyPI.

**Decision**: supervision. We're already using OpenCV for image ops; adding supervision is an incremental dep with strict license benefit and zero runtime cost.

**Consequences**:
- Replace `cv2.dnn.NMSBoxes` calls (now mostly redundant after Decision 1) with `sv.Detections` construction from the ONNX output.
- Add ByteTrack via `sv.ByteTrack` so each detection carries a persistent `track_id`.
- Extend wire protocol with a `u32 track_id` per detection. Daemon-side `Detection` struct + IPC `DetectionsUpdate` need a matching field. `0xFFFFFFFF` = no track yet.

### Decision 3: TRT 8.6.x as a gated escalation, not a default

**Context**: TensorRT 10 dropped Pascal sm_61. TRT 8.6.x still supports it, and its `EFFICIENT_NMS` plugin runs NMS as a CUDA kernel inside the engine — no host roundtrip. But TRT engine builds are finicky on Pascal (we already burned days on ORT-Rust deadlocks on the same box) and the build chain is heavy.

**Options considered**:
- **Default to TRT 8.6.x EFFICIENT_NMS** — maximum payoff, maximum yak.
- **Default to ORT CUDA EP with embedded-NMS ONNX, escalate to TRT only if FPS misses target**.
- Skip TRT entirely — accept ORT CUDA EP ceiling (~50 FPS YOLO11s).

**Decision**: Gate TRT behind a measurement. Phase 1 alone should clear 10 FPS sustained on YOLO11s + ORT CUDA EP (the wiki's table puts YOLO11s at 20–35 FPS PyTorch fp32 on a 1060; ONNX/ORT is at least as fast). Build the TRT path only if Phase 1's bench shows we don't make budget.

**Consequences**:
- If Phase 1 hits 10 FPS sustained: ship Phase 1+2, archive the TRT plan.
- If Phase 1 falls short: Phase 3 builds TRT 8.6.x + a `polygraphy`-driven engine + the `EFFICIENT_NMS` plugin. Document the build env in the gtx-1060 wiki.
- The user explicitly accepted the TRT-NMS plugin path in the interview, so we'll *implement* it on the milestone schedule below. The gating is on whether to *ship* it, not whether to *build* it.

### Decision 4: Don't lift the daemon FPS cap as part of this work

**Context**: `task.rs:30` caps the daemon's inference at 10 FPS. Lifting it would let us consume the 30 FPS phone publish.

**Options considered**:
- Lift cap to 30 FPS now, plan around 30 FPS sidecar throughput.
- **Keep 10 FPS cap; design for 10 FPS sustained**.

**Decision**: Keep the cap. The cap exists because (a) detection at 30 FPS gives diminishing returns for slow-moving cattle, (b) every additional inference costs IPC bandwidth + GUI paint cost, (c) the GUI overlay refreshes at egui's monitor rate regardless. Lifting the cap is its own plan.

**Consequences**: Phase 3 is *only* worth doing if it gives operational headroom (cooler GPU, lower CPU on the sidecar host) — not for FPS. The cost-benefit is real but is its own decision.

## Implementation Phases

### Phase 1 — Re-export to YOLO11s with embedded NMS (estimated: 2–3 hours)

**Goal**: Replace `/srv/aiworker/yolov5n.onnx` with a YOLO11s export that does NMS inside the graph, and trim `cv_sidecar.py`'s postprocess down to a tensor decode. Hit 10 FPS sustained.

**Tasks**:
- [ ] On the dev machine: `pip install ultralytics` in a scratch venv (don't pollute the bigdeal aiworker venv with AGPL'd train-time deps).
- [ ] `yolo export model=yolo11s.pt format=onnx nms=True opset=17 simplify=True` → produces `yolo11s.onnx` with input `[1, 3, 640, 640]` and output `[1, top_k, 6]`.
- [ ] Verify the output shape with `python -c "import onnx; m = onnx.load('yolo11s.onnx'); [print(o.name, o.type.tensor_type.shape) for o in m.graph.output]"`. Expected: a single `[1, 300, 6]`-ish tensor.
- [ ] scp the new ONNX to bigdeal: `/srv/aiworker/yolo11s.onnx`. Update `CV_SIDECAR_MODEL` in `/etc/herd-scout-cv-sidecar.env` to point at it (don't delete the old yolov5n.onnx until Phase 1 is verified — easy rollback).
- [ ] Patch `deploy/cv-sidecar/cv_sidecar.py`:
  - Drop `postprocess()` entirely.
  - Replace with: pull single `[1, top_k, 6]` tensor, slice rows where `conf > 0` (NMS pads with zeros), filter `class ∈ {17, 18, 19}` (horse/sheep/cow COCO indices), pack into the existing wire format. ~15 lines.
  - Confirm whether `nms=True` exports default to fp32; adjust `model_input_dtype()` if needed.
- [ ] Run the standalone smoke client (`deploy/cv-sidecar/smoke_client.py`) — confirm dets still come back.
- [ ] Restart `herd-scout-cv-sidecar.service`, restart `herd-scout-daemon.service`, pair the phone, watch the sidecar journal for `frame N: pre=Xms run=Yms post=Zms`. **Acceptance bar: post < 5 ms, sustained FPS ≥ 10 over a 2-min phone broadcast.**

**Dependencies**: None — this is the first move.

**Validation**:
- `journalctl -u herd-scout-cv-sidecar.service -f` shows `post < 5 ms`.
- `nvidia-smi --query-compute-apps=pid,used_memory --format=csv` shows the python sidecar still holding VRAM (model load succeeded; YOLO11s fp32 should sit ~150–200 MiB).
- Sustained FPS = `n_processed / elapsed` once the phone has been broadcasting ≥ 60 s, ≥ 10.0.

**Wiki grounding**: [farm-vision-on-gtx-1060](../../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/farm-vision-on-gtx-1060.md) recommends YOLO11s as the right model fit for Pascal 6GB at 47 mAP COCO. The same article: "TensorRT 10 dropped Pascal — use TRT 8.6.x or stay on PyTorch CUDA EP / ONNX Runtime CUDA EP" — Phase 1 stays on ORT CUDA EP, no TRT dependency.

**Rollback**: Restore the old `CV_SIDECAR_MODEL=/srv/aiworker/yolov5n.onnx` and revert the `cv_sidecar.py` patch. Single env var + single git revert.

### Phase 2 — supervision + ByteTrack tracking, wire-format track_id (estimated: 3–4 hours)

**Goal**: Bring `supervision` into the sidecar's runtime closure. Replace any remaining cv2/numpy detection bookkeeping with `sv.Detections`. Add ByteTrack so each detection carries a persistent `track_id`. Extend the IPC wire format end-to-end.

**Tasks**:
- [ ] Add `supervision>=0.20` to the aiworker venv (`/srv/aiworker/.venv`). Confirm it doesn't drag in numpy>=2 (we hard-pin numpy<2 because of pyannote 3.1.1).
- [ ] Patch `cv_sidecar.py` to:
  - Build `sv.Detections` from the YOLO11s NMS output.
  - Attach `sv.ByteTrack(frame_rate=10)` (matches daemon cap).
  - On each frame: `dets = tracker.update_with_detections(dets)` → `dets.tracker_id` is the per-track ID.
  - Pack `track_id` into the wire response next to the existing fields.
- [ ] Update wire protocol:
  - `DET_PACK = struct.Struct("<Iffffff")` → bump to `"<IIffffff"` (add `track_id u32` after `class_id`).
  - Bump a wire version constant or document the schema break — there's only one daemon version, no compat needed.
- [ ] Mirror the change in `herd-scout-daemon/src/cv/model.rs`:
  - `Detection` struct grows a `track_id: u32` field.
  - Wire-decode reads the extra u32.
- [ ] Mirror in `herd-scout-ipc::DetectionsUpdate` so the GUI can paint persistent IDs / colored boxes per track.
- [ ] Run the standalone smoke client; confirm a single static frame yields `track_id=0xFFFFFFFF` (no track) initially, then a stable ID across consecutive identical frames.
- [ ] Pair the phone, broadcast for ≥ 60 s, watch journal for stable track IDs across frames where the same animal moves.

**Dependencies**: Phase 1 must be in (the YOLO11s NMS output feeds `sv.Detections` cleanly; YOLOv5 raw output would need a parser supervision doesn't ship).

**Validation**:
- Sidecar journal: `track_ids=[0,1,2,3]` (or similar) for a frame; same IDs persist across consecutive frames where animals haven't disappeared.
- Daemon log: `Detection { class, conf, bbox, track_id }` appears in IPC traces.
- GUI (operator laptop): bbox overlays carry per-track IDs (label format up to GUI; that's a follow-up task).
- License audit: `pip-licenses --packages supervision` confirms MIT.

**Wiki grounding**: [supervision raw](../../../wiki/topics/gtx-1060-headless-ai-server/raw/repos/2026-05-21-supervision.md): "MIT (vs Ultralytics' AGPL-3.0)... LineZone + ByteTrack composition is the cleanest path to a counting deliverable." [farm-vision-on-gtx-1060](../../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/farm-vision-on-gtx-1060.md): "ByteTrack (faster) or BoT-SORT (default) — use for ground CCTV, skip for aerial stills." Phone publish is ground-CCTV-class; ByteTrack is the right choice.

**Rollback**: Drop the tracker call, revert the wire-format extra u32. The supervision dep can stay even if tracking is disabled — it's harmless.

### Phase 3 — TRT 8.6.x sm_61 engine + EFFICIENT_NMS plugin (estimated: 1–2 days, GATED)

**Goal**: Build a TRT 8.6.x engine with the `EFFICIENT_NMS_TRT` plugin embedded, drop ONNX-graph NMS in favor of a CUDA kernel. Only run this phase if Phase 1's bench fails to hit 10 FPS sustained, OR if a downstream change (lifting daemon cap, multi-camera fan-in) demands more headroom.

**Gate condition**: Phase 1 sustained FPS < 10.0 over 2 minutes of phone broadcast, OR a follow-up plan explicitly requests TRT speedup.

**Tasks**:
- [ ] On bigdeal: install TensorRT 8.6.x via the NVIDIA repo (NOT 10.x; TRT 10 dropped Pascal sm_61). Pin the package: `apt-mark hold libnvinfer8 libnvinfer-plugin8 libnvinfer-headers-dev`.
- [ ] Verify `trtexec --version` shows 8.6 and `trtexec --help` lists the EFFICIENT_NMS plugin.
- [ ] Build the engine: re-export YOLO11s without `nms=True` (raw heads), run `trtexec --onnx=yolo11s_raw.onnx --fp16 --saveEngine=yolo11s.trt` with `--plugins=EfficientNMS`. The exact incantation depends on TRT 8.6's plugin registration ABI; document the working command in `deploy/cv-sidecar/build_trt_engine.sh`.
- [ ] Switch the sidecar from ORT CUDA EP to either:
  - **Path A**: ORT TensorrtExecutionProvider (loads the TRT engine through ORT, keeps `cv_sidecar.py` mostly unchanged), OR
  - **Path B**: native pyCUDA + TRT runtime (`tensorrt.Runtime.deserialize_cuda_engine`), bypassing ORT entirely.
  - Pick A for minimum diff; B if A turns out to inherit any of ORT's static-init brittleness from the Rust era.
- [ ] Bench: target post < 1 ms, run < 10 ms, sustained FPS ≥ 30 (gives 3x headroom over the 10 FPS cap).
- [ ] Add a sidecar config flag `CV_SIDECAR_BACKEND=ort|trt` to switch back to Phase 1's ORT path without code edits if TRT misbehaves at 3am.
- [ ] Document the TRT 8.6 + EFFICIENT_NMS build for the gtx-1060 wiki — this is reusable knowledge for any future Pascal CV work.

**Dependencies**: Phase 1 (we need a YOLO11s ONNX to start from).

**Validation**:
- `trtexec --loadEngine=yolo11s.trt --fp16` self-bench reports throughput.
- Sidecar journal: `run < 10 ms, post < 1 ms`.
- `nvidia-smi --query-compute-apps`: VRAM usage may grow to 600–900 MiB (TRT engines pre-allocate workspace).
- Regression bar: every detection that came back from Phase 1 still comes back from Phase 3 within ε = 1 px / 0.01 conf (the TRT engine does the same math, the kernels are equivalent up to floating-point tolerance).

**Wiki grounding**: [pascal-driver-cuda-pinning](../../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/_index.md) — driver pin invariant (535-server, never 13.x). [farm-vision-on-gtx-1060](../../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/farm-vision-on-gtx-1060.md): "TensorRT 10 dropped Pascal — must use TensorRT 8.6.x for sm_61 INT8/FP16 export." This phase generates new wiki content if it ships.

**Rollback**: Set `CV_SIDECAR_BACKEND=ort`, restart sidecar, instantly back to Phase 1.

### Phase 4 — Bench harness + regression checkpoint (estimated: 2 hours)

**Goal**: Ship a one-shot bench script anyone can run on bigdeal to confirm the sidecar still hits its budget. Wire it into the existing aiworker bench harness if it fits cleanly; otherwise sidecar-local script.

**Tasks**:
- [ ] Write `deploy/cv-sidecar/bench.py`: spins up a synthetic 720p phone-equivalent stream against the live sidecar socket, runs for 60 s, reports `run/post/fps` percentiles.
- [ ] Land an `inventory/cv-sidecar-bench-2026-XX-XX.md` in `.wiki/inventory/` with the bench numbers from each Phase shipped — Phase 0 (current), Phase 1, Phase 2, optionally Phase 3.
- [ ] Update `.wiki/log.md` with the optimization results.

**Dependencies**: Whichever phases ship.

**Validation**: `bench.py` reports the right numbers; running it after a code change immediately catches regressions. CI integration is out of scope (no CI on bigdeal).

## Risks & Mitigations

| Risk | Source | Mitigation |
|------|--------|------------|
| `nms=True` ONNX export forces fp32 input, losing the fp16 inference path | Ultralytics export semantics, [yolov8-yolo11-specs](../../../wiki/topics/gtx-1060-headless-ai-server/raw/articles/2026-05-21-yolov8-yolo11-specs.md) | Verify dtype after export with onnx tooling; if fp32, accept it (Phase 1 still hits 10 FPS easily on a 1060 at fp32 — 20–35 FPS PyTorch fp32 per the wiki table). |
| AMP/fp16 NaN issues on Pascal during *re-training* (if we ever fine-tune) | [farm-vision-on-gtx-1060](../../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/farm-vision-on-gtx-1060.md): "Some users see AMP loss-scaling NaN issues on 1060 — fall back to FP32" | Out of scope for this plan (we're not retraining). Flagged for the future fine-tuning plan. |
| `supervision` pulls numpy>=2, conflicts with pyannote 3.1.1's numpy<2 in the shared aiworker venv | aiworker deployment notes (numpy<2 is hard-pinned) | Test in isolation first; if conflict, install supervision into a sidecar-only venv at `/srv/aiworker/.venv-cv` and adjust the systemd ExecStart. |
| TRT 8.6.x apt repo only ships for older Ubuntu/CUDA combos | NVIDIA repo policy; we're on 22.04 + CUDA 12 | Verified workable in the wiki. If repo install fails: NVIDIA still ships TRT 8.6 tarballs for Ubuntu 22.04; doc the tarball path in build_trt_engine.sh. |
| EFFICIENT_NMS plugin ABI incompatibility with the export's NMS op | TRT plugin matrix changes between TRT 8 minor versions | Build with `--plugins=...` and inspect with `trtexec --layerInfo`; if mismatch, fall back to letting TRT inline the NMS as separate ops (slower but works). |
| Future driver auto-update breaks Pascal pin | [pascal-driver-cuda-pinning](../../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/_index.md) | Already mitigated by `apt-mark hold` on `nvidia-driver-535-server` from the prior deployment. |
| Gate-on-Phase-1-bench fails because phone publish quality varies | Network jitter on LAN | Run the bench against `bench.py`'s synthetic stream, not phone publish. Phone publish is the integration test, not the perf test. |

## Open Questions

- Does YOLO11s with `nms=True` export keep the same 80-class COCO head, or does Ultralytics let us specialize at export time? If specialize-at-export is supported, exporting only the 3 classes (17/18/19) may further shrink the per-frame top-k decode. *Worth a 10-minute spike during Phase 1.*
- Does the GUI consume `track_id` cleanly today, or does Phase 2 need a paired GUI patch? *Read `herd-scout-gui` source before landing Phase 2 to know what we're committing to.*
- Is there a reason to keep the 10 FPS cap once tracking is in? Tracking is mildly more useful at 15-20 FPS for fast-moving animals. *Defer to a follow-up plan; out of scope here.*
- Should YOLO11s.pt be tracked in-tree (~20 MB) or pulled from Ultralytics releases at deploy time? *Probably the latter; today's yolov5n.onnx is 4 MB and lives in `assets/`. YOLO11s might exceed that line.*

## Sources Consulted

- [drone-vision-software](/.wiki/wiki/concepts/drone-vision-software.md) — original CV brief; confirmed COCO 17/18/19 are the right classes.
- [farm-vision-on-gtx-1060](../../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/farm-vision-on-gtx-1060.md) — the model-fit table that recommends YOLO11s, the Pascal TRT pin (8.6.x not 10), the AMP NaN warning, the supervision-for-license-clarity pointer.
- [yolov8-yolo11-specs](../../../wiki/topics/gtx-1060-headless-ai-server/raw/articles/2026-05-21-yolov8-yolo11-specs.md) — YOLO11s = 9.4M params, 47.0 mAP, 20–35 FPS on 1060 fp32.
- [supervision raw](../../../wiki/topics/gtx-1060-headless-ai-server/raw/repos/2026-05-21-supervision.md) — MIT license, ByteTrack composition, model-agnostic adapters.
- [pascal-driver-cuda-pinning](../../../wiki/topics/gtx-1060-headless-ai-server/wiki/concepts/_index.md) — driver pin rules; touched but not modified by this plan.
- [plan-deploy-daemon-on-1060-laptop](./plan-deploy-daemon-on-1060-laptop-2026-05-22.md) — predecessor deployment plan; this plan picks up where its Phase 3 left off (CV sidecar shipped, postprocess unoptimized).

## Sequence

The shape we'll execute:

1. **Phase 1** as the first push — single new ONNX, single sidecar patch. ~half-day. **Gate: if sustained FPS ≥ 10, Phase 3 stays a research task, not a build task.**
2. **Phase 2** alongside or right after Phase 1 — pure-Python addition, doesn't touch GPU. Wire-format change is the only Rust touchpoint.
3. **Phase 3 only if Phase 1 misses budget** — most likely it doesn't, so this stays a backlog item with a written gate condition. If we ever need it, the plan is here.
4. **Phase 4** rides along — bench script lands during Phase 1's verification step, then re-runs for each subsequent phase.

The whole thing is shippable in a working day if Phase 3 is gated out, which is the expected case.
