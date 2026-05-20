# CV Design — herd-scout desktop

Wave 2C deliverable. Locks the technical decisions Wave 3 (CV-Integrate)
will execute against. No code in this doc; pure design.

## Decisions (locked)

1. **Model**: YOLOv5n (smallest YOLOv5 variant, ~7.5 MB ONNX) trained on
   COCO 80-class. Cow = class 19, horse = 17, sheep = 18 (verified
   against `ultralytics/yolov5/data/coco.yaml`; the wiki article's
   "15/17/18" was wrong — class 15 is `cat`).
2. **Storage**: bundled with the binary via `include_bytes!`. Model file
   committed at `desktop/assets/yolov5n.onnx` (~7.5 MB; acceptable in
   git without LFS for a single asset). First-run network fetch is
   rejected — offline-first wins. `build.rs` is *not* used to download
   on build either (closed-source builds, sandboxed CI, and field
   laptops without internet must all work).
3. **Runtime**: `ort` crate (ONNX Runtime Rust bindings),
   version `2.0.0-rc.10`, feature `download-binaries` so prebuilt
   ORT CPU shared libs are pulled at build time and statically wired —
   no system `libonnxruntime` required. CPU execution provider only
   for MVP; CoreML/CUDA can be added later behind feature flags.
4. **Frame budget**: 5–10 FPS inference cap on a 2020+ laptop CPU.
   Display loop runs at decoder rate (typically 30 FPS). Inference is
   **asynchronous and lossy**: if the inference task is busy when a
   new frame arrives, the new frame is dropped. The most recent
   detection result is overlaid on whatever frame is currently being
   rendered, even if that frame is 1–3 frames newer than the inference
   input.
5. **Integration**: Wave 2A wires `moq-media-egui`'s
   `VideoDecoderFrames` receiver into the egui app. The decoded frame
   type is `rusty_codecs::format::VideoFrame`, which exposes
   `rgba_image() -> &image::RgbaImage` (lazy cache). Wave 3 fans this
   out to a CV inference task without copying H.264 buffers.

## Model

- **Source**: ultralytics/yolov5 v7.0 release. PyTorch weights live at
  `https://github.com/ultralytics/yolov5/releases/download/v7.0/yolov5n.pt`
  (commit pin: tag `v7.0`, released 2022-11-22). Ultralytics does not
  ship `.onnx` directly in releases, so we export once locally with
  `python export.py --weights yolov5n.pt --include onnx --opset 12
  --img 640` and commit the resulting `yolov5n.onnx`. Document the
  export command in `desktop/assets/README.md`.
- **File path in repo**: `desktop/assets/yolov5n.onnx` (~7.5 MB).
- **Output shape**: `[1, 25200, 85]`. Row layout per anchor:
  `[cx, cy, w, h, obj_conf, class0_score, ..., class79_score]`.
  25,200 = (80×80 + 40×40 + 20×20) × 3 anchors. Coords are pixel
  values in the 640×640 input space (already decoded by the model's
  built-in `Detect` head).
- **COCO classes used**: 17 = horse, 18 = sheep, 19 = cow. All other
  classes are masked out before NMS.

## Dependencies to add (`desktop/Cargo.toml`)

Additive only; everything else is already declared in the workspace.

- `ort = { version = "2.0.0-rc.10", features = ["download-binaries"] }`
- `ndarray = "0.16"` — required by `ort`'s tensor APIs.
- `image = "0.25"` — already used transitively via `rusty-codecs`;
  declare explicitly so the resize helpers (`imageops::resize`) are
  in scope. `default-features = true` is fine here (no JPEG needed
  for inference path; jpeg feature would only be wanted for debug
  dumps).

Do **not** add `tract-onnx`, `candle`, `wgpu` (already pulled via
`moq-media-egui`), or `tokio` (already in `desktop/Cargo.toml`).

## Preprocessing pipeline

1. Wave 2A's display task gets a `VideoFrame` from
   `VideoDecoderFrames::recv().await`.
2. Frame is `Arc`-cloned (cheap — `bytes::Bytes` underneath) and the
   clone is sent on a `tokio::sync::mpsc::Sender<VideoFrame>` of
   capacity 1 to the inference task. `try_send` is used so a backed-up
   inference task drops the frame instead of stalling the display.
3. Inference task calls `frame.rgba_image()` to materialize an
   `image::RgbaImage` (lazy; computed once and cached).
4. Resize to 640×640 via `image::imageops::resize` with
   `FilterType::Triangle` (good speed/quality trade). Letterboxing
   (preserve aspect, pad with grey 114/114/114) is recommended;
   straight-stretch is acceptable for MVP if letterboxing complicates
   the box back-projection. **Locked: straight stretch for MVP**;
   note the aspect distortion in the doc and revisit if accuracy is
   poor.
5. Convert RGBA → RGB by dropping alpha. Convert to `f32`, divide by
   255.0. Transpose HWC → CHW. Shape: `[1, 3, 640, 640]`.

## Inference

- One `ort::Session` is created at app startup from the embedded model
  bytes (`Session::builder()?.commit_from_memory(MODEL_BYTES)?`).
- The session is wrapped in an `Arc<Mutex<Session>>` and owned by the
  inference task. ORT sessions are `Send` but not `Sync`; a single
  inference task means no contention.
- Inference runs on a `tokio::task::spawn_blocking` body, since
  `session.run()` is CPU-bound and synchronous. The task awaits frame
  receipt, spawns blocking inference, and `.await`s the join handle.
- One inference task is sufficient at MVP frame budgets (5–10 FPS, ~50
  ms per frame on a 2020+ laptop CPU). Parallel sessions are not
  required.

## Postprocessing

1. Read output tensor as `ndarray::ArrayView3<f32>` of shape
   `[1, 25200, 85]`.
2. For each of the 25,200 rows: compute final score as
   `obj_conf * max(class19, class17, class18)`. Reject if < 0.25.
3. Convert `(cx, cy, w, h)` to `(x1, y1, x2, y2)` in 640-space, then
   scale back to original frame dimensions
   (`x_scale = orig_w / 640.0`, same for y).
4. Run NMS (Non-Max Suppression) per class with IoU threshold 0.45.
   Trivial implementation in pure Rust (~50 LOC); no external NMS
   crate needed.
5. Emit `Vec<Detection>` where
   `Detection { class: CocoClass, bbox: [f32; 4], score: f32 }`
   and `CocoClass` is an enum of `Cow | Horse | Sheep`.

## Bounding-box overlay

**Recommendation: shared state, not a channel.**

Rationale: detections are stale-but-current state, not a stream of
events. The display task always wants "the latest detections", never
"every detection ever produced". A bounded mpsc channel forces the
display task to drain on every paint; a shared snapshot does not.

Use `Arc<RwLock<DetectionSnapshot>>` where
`DetectionSnapshot { detections: Vec<Detection>, frame_pts: Duration,
inferred_at: Instant }`. Inference task takes the write lock briefly
on completion; egui paint takes the read lock once per frame. egui's
paint loop is already on the UI thread; lock contention is
negligible.

Drawing: in egui's `update`, after the video frame is painted, iterate
the snapshot's detections and draw rectangles via
`Painter::rect_stroke` with per-class colour (cow = orange, horse =
cyan, sheep = magenta) plus a small label `Painter::text` above each
box. Coordinates must be transformed from frame-pixel space to the
egui rect occupied by the video texture.

## Count aggregation

- Per-class running count: just `detections.iter().filter(...).count()`
  for the latest snapshot.
- **Smoothing**: rolling 1-second window max, computed from a
  `VecDeque<(Instant, [u32; 3])>`. Display the max-over-window; this
  hides per-frame jitter (e.g. one cow briefly missed) without
  introducing the lag of an EMA. Window length is a constant; tune
  later.
- Render counts in a top-right egui panel: `Cows: 12  Horses: 0
  Sheep: 0`.

## Frame budget

- Display loop: 30 FPS, driven by decoder.
- Inference loop: capped at 10 FPS with a `tokio::time::Interval` that
  ticks every 100 ms; if the latest frame in the inbox is older than
  100 ms it is discarded and the next tick is awaited.
- Channel between display fan-out and inference: `mpsc` capacity 1
  with `try_send` — backpressure becomes drop-the-frame, never block.
- No frames are dropped on the display path. If inference falls
  behind, the overlay shows the most recent (possibly 200–500 ms old)
  detections; the video itself stays smooth.

## Build glue (build.rs vs vendored binary)

**Recommendation: vendored binary**, no `build.rs`.

The model is committed at `desktop/assets/yolov5n.onnx`. Loading is
`include_bytes!("../assets/yolov5n.onnx")`. Rationale: a `build.rs`
that fetches over HTTP breaks (a) sandboxed CI, (b) closed-network
field machines, (c) reproducible builds, and (d) the explicit
"offline-first wins" decision. 7.5 MB in git is acceptable for one
file; if the repo accumulates more model assets later, migrate them
collectively to git LFS or a `tools/fetch-models.sh` script — not
into the build pipeline. Document the export command (above) in
`desktop/assets/README.md` so the model is reproducible.

## Failure modes

- **Model file missing at compile time**: `include_bytes!` is a
  compile error, caught immediately. Cannot reach runtime in this
  state.
- **`ort` session creation fails at runtime** (corrupt bytes, ORT
  init error): log at ERROR via `tracing`; skip CV entirely; video
  still plays without overlays. The egui app must not panic.
- **Inference fails on a single frame**: log at WARN, skip that
  frame, next frame proceeds normally. Do not poison the session.
- **Output shape mismatch** (e.g. someone swaps in a different model
  version): detected on first inference, surfaced as a top-of-screen
  egui banner ("CV: model output shape unexpected"); CV disabled for
  the session.
- **Class index out of range** (won't happen with COCO80, but defensive
  check): silently skip rows with class id ≥ 80.
- **GPU frame from hardware decoder**: `VideoFrame::is_gpu()` is true.
  `rgba_image()` triggers download-and-convert; this is expensive
  (~10 ms). Acceptable at 10 FPS; revisit if profiling shows it as
  the bottleneck.

## Wave 3 concrete tasks

Ordered, atomic. Each should be a single small commit.

1. Add `ort`, `ndarray`, `image` to `desktop/Cargo.toml` per
   "Dependencies to add". Confirm `cargo build -p
   p2p-video-pipe-desktop` still passes.
2. Export `yolov5n.onnx` from the upstream `.pt` weights using the
   command in this doc; commit it at `desktop/assets/yolov5n.onnx`
   alongside `desktop/assets/README.md` documenting provenance and
   export command.
3. Create `desktop/src/cv/mod.rs` with submodules `model`,
   `preprocess`, `postprocess`, `state`. Wire as `mod cv;` in
   `main.rs`.
4. Implement `cv::model`: `Detector` struct owning the ORT session,
   constructor `Detector::new()` that does
   `Session::builder().commit_from_memory(MODEL_BYTES)`, and
   `Detector::infer(&self, frame: &VideoFrame) -> Result<Vec<Detection>>`.
5. Implement `cv::preprocess`: `VideoFrame -> ndarray Array4<f32>`
   per the pipeline above. Unit-test on a synthetic 1280×720 RGBA
   buffer.
6. Implement `cv::postprocess`: tensor output → `Vec<Detection>`
   with NMS. Unit-test with a hand-crafted 25200×85 fixture
   asserting one cow box survives and one duplicate is suppressed.
7. Implement `cv::state`: `DetectionSnapshot`, the
   `Arc<RwLock<DetectionSnapshot>>` shared handle, and the
   1-second-window count helper.
8. Wire the inference task: in `main.rs`, after the moq subscriber
   is up, spawn a `tokio::task` that owns the `Detector`, reads from
   a capacity-1 mpsc fed by the display fan-out, runs inference via
   `spawn_blocking`, and writes the snapshot. Cap at 10 FPS.
9. Render boxes + counts in egui paint, reading the snapshot. Use
   distinct colours per class. Show a "CV idle" hint when the
   snapshot is older than 2 seconds.
10. Smoke-test end-to-end with the planned pre-MVP scenario: Android
    publisher (Wave 2B) pointed at a YouTube cattle clip on a
    monitor; desktop should display non-zero cow count and tracking
    boxes within 5 seconds of stream start.
