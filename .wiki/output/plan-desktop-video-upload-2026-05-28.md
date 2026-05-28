---
title: "Plan: Desktop video upload to daemon for CV processing"
type: plan
format: roadmap
sources:
  - output/plan-mobile-to-desktop-iroh-rfc-2026-05-20.md
  - output/plan-optimize-cv-sidecar-trt-yolo11s-2026-05-26.md
  - output/playbook-accurate-herd-counting-2026-05-27.md
  - wiki/concepts/iroh-sync-stack.md
  - wiki/concepts/iroh-docs-fms-schema.md
  - wiki/concepts/herd-counting-pipeline.md
  - inventory/cv-sidecar-bench-2026-05-27.md
  - herd-scout-ipc/src/lib.rs (direct)
  - herd-scout-daemon/src/cv/task.rs (direct)
  - deploy/cv-sidecar/cv_sidecar.py (direct)
generated: 2026-05-28
---

# Plan: Desktop video upload to daemon for CV processing

> Generated from herd-scout `.wiki/` (16 concept articles, 28 raw sources, 5 prior plans). Builds on the live phone-to-daemon path from the mobile-to-desktop RFC and the count-accuracy playbook from 2026-05-27.

## Executive Summary

Add a **batch upload path** as a sibling to the live phone stream: users drag an MP4/MOV into the desktop GUI (or run `herdctl push <file>`), the bytes ride iroh-blobs to the daemon, the daemon decodes the clip frame-by-frame and feeds it to the same CV sidecar that already handles live frames, then emits **two outputs** per clip — a live overlay replay (`ServerMsg::Frame` + `Detections` to any connected GUI) AND a persistent per-clip JSON report applying the accurate-counting playbook (median-of-active-IDs + bootstrap CI + per-class breakdown). A live phone always wins; uploads queue.

**Key reuse:** the same sidecar process, the same wire protocol, the same `VideoFrame::new_cpu` construction, the same `ServerMsg::Detections` ID — a clip is simply a slow-motion phone broadcast that comes from disk. The new code is (a) a `cv2.VideoCapture` decode mode in the sidecar, (b) an upload queue + iroh-blobs ALPN on the daemon, (c) a drop-zone in the GUI, (d) a `herdctl push` subcommand, (e) a report writer that consumes the existing `track_id` stream.

**Why now:** the count-accuracy playbook (P1 tier) requires ~200 labeled frames captured at the deployment site. Today the only way to get frames into the daemon is a live phone broadcast — which is hard to reproduce, hard to label, and hard to re-run with different ByteTrack params. Upload turns "I have a 90-second clip from yesterday's flyover" into a full pipeline run with a written report.

**Cap for first cut:** 10 min / 2 GB per clip, single-clip-at-a-time queue, MP4/MOV with H.264. HEVC, multi-clip batches, and resume-across-restarts are out of scope.

## Architecture Decisions

### Decision 1: iroh-blobs as the upload transport

**Context**: The wiki (`iroh-sync-stack`) commits to iroh-blobs for "pasture/cattle photos and drone clips" — BLAKE3 content-addressed, resumable, kB→TB scale. The daemon already runs an iroh node with `CONTROL_ALPN`, admin RPC, and moq-live ALPNs.

**Options considered**:
- **iroh-blobs over the existing iroh node** — reuses the committed data plane. LAN/WAN identical. BLAKE3 hash is a natural clip ID. Resumable.
- Chunk-stream over the existing GUI Unix socket — simple but reinvents iroh-blobs and only works locally. Breaks when the GUI is on a laptop and the daemon is on bigdeal.
- Local file path + daemon reads from disk — trivial co-located, useless when the GUI is remote (which is the deployment shape per `plan-deploy-daemon-on-1060-laptop`).

**Decision**: iroh-blobs. Per the user-confirmed answer.

**Consequences**:
- A fifth ALPN is registered on the daemon's iroh `Router`: `herd-scout/upload/1`. The phone's moq broadcast and the daemon's other planes are unchanged.
- The blob hash (BLAKE3) becomes the canonical clip ID — used as the directory name under `<data_dir>/uploads/<blake3>/`, the report's `clip_id` field, and the wire identifier in `ServerMsg::UploadStatus`.
- The blob store on the daemon is the source of truth for the bytes. The GUI doesn't keep a copy after upload; if the user wants the clip back, the daemon serves it via the same iroh-blobs path.
- iroh-blobs is already a workspace dep (per `Cargo.toml` audited during the mobile-to-desktop RFC).

### Decision 2: Reuse the existing CV sidecar; extend its wire protocol with a file-decode mode

**Context**: The sidecar today reads BGR24 frames over a Unix socket and emits `[1, 300, 6]` decoded detections + ByteTrack `track_id` (per `cv-sidecar-bench-2026-05-27`: 23 FPS Phase 2 sustained, single-client, 256 MiB GPU mem cap). Adding a separate decode service would duplicate the OpenCV + ORT runtime.

**Options considered**:
- **Extend the sidecar's wire protocol with a `MODE_FILE` opcode** so the sidecar opens the file via `cv2.VideoCapture`, iterates frames, and emits the same per-frame detection response. The daemon's role becomes "tell the sidecar the file path; relay the per-frame responses to the GUI."
- Decode in Rust (rusty-codecs / ffmpeg-next) and feed BGR24 frames over the existing wire — keeps the sidecar interface unchanged, but adds a new Rust dep with cross-platform build pain.
- Run `ffmpeg` as a subprocess from the daemon, pipe rawvideo to the sidecar — works but adds a third process per clip and complicates lifecycle.

**Decision**: Extend the sidecar protocol. A new `request_kind: u32` field is prefixed to each request:
- `0x00` (today's behavior, but explicit) — `frame_id, w, h, payload_len, BGR24...`
- `0x01` (new) — `clip_id (16 bytes BLAKE3), path_len, utf8 path` — sidecar opens the file, iterates, and emits one response per decoded frame, **using the existing response framing** with `frame_id = decode-order index` per clip.

A terminator response (`n_dets = 0xFFFFFFFF`) marks end-of-clip.

**Consequences**:
- Sidecar holds the file path; daemon must hand it a path the sidecar process can `open()`. They share a host today (per `plan-deploy-daemon-on-1060-laptop`); the staged blob path is `<data_dir>/uploads/<blake3>/clip.<ext>` on the daemon host, readable by the sidecar service user.
- **The sidecar remains single-client.** Live phone and upload share the same sidecar. The decision to queue (Decision 4) makes that workable.
- The daemon's bench (Phase 2: 23 FPS, p50 RTT 52 ms) is the upper bound. A 10-min 30 FPS clip = 18 000 frames; at 23 FPS that's ~13 minutes wall-clock per clip — same order as live. Acceptable.
- HEVC files won't decode out of the box (cv2 builds vary). MP4/H.264 is the committed first cut. Document HEVC as a watch item.

### Decision 3: BLAKE3 hash is the clip ID

**Context**: iroh-blobs is BLAKE3-content-addressed. The hash is a free 32-byte (64-hex) collision-resistant ID we don't have to mint or coordinate.

**Options considered**:
- **BLAKE3 hash from iroh-blobs** — free, deterministic, dedupable (re-uploading the same file is a no-op).
- UUID minted by the daemon — needs a registry; doesn't dedupe.
- Filename-derived — collisions, user-controlled, ugly.

**Decision**: BLAKE3. The wire identifier is the full 32-byte hash; UI surfaces the first 8 hex chars + filename.

**Consequences**:
- Re-upload is automatically a no-op at the byte level (iroh-blobs dedupes). The daemon should still queue a fresh **process** request — the user may want to re-run with new ByteTrack params.
- `report.json` includes the clip_id so artifacts are self-describing.

### Decision 4: Queue uploads behind any active live session

**Context**: The sidecar is single-client. The phone-to-daemon live path is the primary product loop and farmers in the field are watching it. GPU contention from a parallel sidecar instance was modeled in `cv-sidecar-bench-2026-05-27` (the pyannote co-tenant case) and the conclusion was a 256 MiB cap — comfortable but not enough to spawn a second YOLO11s.

**Options considered**:
- **Queue uploads; live wins** — simplest. Live continues uninterrupted; uploads start when the live session ends. Per the user-confirmed answer.
- Preempt live — risky; farmer in the field would see live cut out.
- Reject uploads while live is active — hard error, feels broken.
- Run a second sidecar instance — most engineering cost; needs disjoint GPU mem caps; not justified for first cut.

**Decision**: Queue. The daemon's upload queue is processed only when `ConnectionStatus != Connecting | Connected` (i.e. no active phone session).

**Consequences**:
- Pending uploads visibly wait. The GUI shows "Queued (live session active)" until the live session ends.
- No GPU contention with live; the bench's 23 FPS Phase 2 number applies unchanged.
- Future: a second sidecar slot is a straightforward extension if/when the workload demands it.

### Decision 5: Two outputs per clip — live replay + persistent JSON report

**Context**: Per the user-confirmed answer.

- **Live overlay replay** = the sidecar's per-frame `Detections` response is forwarded to any connected GUI as `ServerMsg::Frame` + `ServerMsg::Detections` exactly as it does for the phone stream. The GUI sees boxes overlaid on the clip's frames; same UI as live.
- **Persistent JSON report** = a structured artifact written to `<data_dir>/uploads/<blake3>/report.json` after the clip finishes. Applies the accurate-counting playbook from `playbook-accurate-herd-counting-2026-05-27`: median-of-active-IDs over a 30-frame sliding window, bootstrap 95% CI, per-class breakdown, per-track detection probability, frame-rate metadata. Format defined in the Spec section.

The report is the deliverable that survives. The replay is the immediate UX.

**Annotated MP4 export was not selected** in the interview; deferred.

### Decision 6: Storage in `<data_dir>/uploads/<blake3>/`

**Context**: Daemon already has a data dir (`directories::ProjectDirs`-resolved path holding `iroh_secret`, the iroh-blobs store, control.toml, audit log). Per the user-confirmed answer.

**Decision**:
```
<data_dir>/uploads/<blake3-hex>/
├── clip.<ext>          # symlink or hardlink into the iroh-blobs store
├── report.json         # the persistent count + tracks + CI artifact
└── meta.json           # filename, mtime, upload_ts, processing_ts, source_node_id
```

Hardlinking from the iroh-blobs store avoids duplicating bytes; symlinking is the fallback if the store and uploads dir are on different filesystems.

### Decision 7: Cap at 10 min / 2 GB; single-clip-at-a-time queue

Per the user-confirmed answer. Rationale:
- 10 min covers a single drone flyover or pasture-cam window — the realistic field unit.
- 2 GB at 1080p H.264 is ~4 hours of headroom; the cap is structural, not bandwidth-driven.
- Single-clip queue keeps state minimal and matches the single-client sidecar.

Out of scope: multi-clip batch upload, hour-long full sessions, resume-across-daemon-restart.

## Implementation Phases

### Phase 0 — Wire-protocol additions (estimated: half a day)

**Goal**: define the new types and constants without behavior changes.

**Tasks**:
- [ ] Add `UPLOAD_ALPN: &[u8] = b"herd-scout/upload/1"` to `herd-scout-ipc/src/lib.rs` next to `CONTROL_ALPN` / `ADMIN_ALPN`.
- [ ] Add `UploadClientMsg` (length-prefixed JSON over the new ALPN's bidi stream):
  - `Push { filename: String, size_bytes: u64, blake3_hex: String }` — client-side hash of the bytes; daemon verifies against iroh-blobs' computed hash before processing.
  - `ListQueue` — return current queue snapshot.
  - `CancelQueued { blake3_hex: String }` — drop a pending entry.
- [ ] Add `UploadServerMsg`:
  - `Accepted { blake3_hex }` — bytes received; queued.
  - `RejectedTooBig { actual_bytes, max_bytes }` — caps enforced server-side.
  - `RejectedHashMismatch { reported, computed }` — integrity guard.
  - `QueueSnapshot { entries: Vec<UploadEntry> }`.
  - `Error { code: String, message: String }` (mirrors `AdminServerMsg::Error`).
- [ ] Extend `ServerMsg` (the GUI's existing socket) with two non-breaking variants:
  - `UploadStatus { blake3_hex, state: UploadState, progress_pct: u8, eta_ms: Option<u64> }` where `UploadState` ∈ {`Queued`, `Decoding`, `Done`, `Failed { reason }`}.
  - `Frame` and `Detections` are reused unchanged for the per-frame replay; add an optional `clip_id: Option<String>` field on both so the GUI can disambiguate live vs upload-replay frames. Default-`None` keeps existing GUI clients working.
- [ ] Define `UploadEntry { blake3_hex, filename, size_bytes, state, queued_ts_ms, started_ts_ms: Option<_>, finished_ts_ms: Option<_> }`.
- [ ] Round-trip JSON tests for every new variant in `lib.rs::tests`.

**Validation**: `cargo test -p herd-scout-ipc` green.

**Wiki grounding**: Wire-protocol shape mirrors `AdminClientMsg` / `AdminServerMsg` from Wave 12 (`plan-android-admin-allowlist-app-2026-05-27`), which is the established pattern for new ALPNs on the daemon's iroh router.

### Phase 1 — Sidecar file-decode mode (estimated: 1 day)

**Goal**: extend `deploy/cv-sidecar/cv_sidecar.py` to accept a file path and emit per-frame detections via `cv2.VideoCapture`.

**Tasks**:
- [ ] Prefix every request frame with `request_kind: u32` (LE). Keep `0x00` as today's BGR24 frame request (unchanged response framing). Add `0x01` for file mode.
- [ ] `0x01` payload: `clip_id_blake3: [u8; 32]`, `path_len: u32`, `utf8 path: [u8; path_len]`.
- [ ] Sidecar opens `cv2.VideoCapture(path)`; iterates frames; for each frame:
  - Build the same `sv.Detections` + ByteTrack pipeline as live.
  - Write the existing response (`frame_id, n_dets, [det…]`) where `frame_id = decode-order index` (0, 1, 2, …).
  - Add a `pts_ms: u64` field to the response (it's already useful for live too — derived from `cap.get(cv2.CAP_PROP_POS_MSEC)`). Append after the existing fixed prefix to stay backward-compatible with the live path.
- [ ] On end-of-file: write a terminator response with `frame_id = decode-order count`, `n_dets = 0xFFFFFFFF` (sentinel).
- [ ] On decode failure mid-clip: write a terminator with `n_dets = 0xFFFFFFFE` plus a length-prefixed UTF-8 reason; daemon records this in `report.json`.
- [ ] **Reset the ByteTrack instance at clip boundaries** (`sv.ByteTrack(...)` re-instantiated). Track IDs are per-clip; live and upload do not share track-id space.
- [ ] Apply the playbook-recommended ByteTrack params from `herd-counting-pipeline` for upload clips: `track_activation_threshold=0.35`, `lost_track_buffer=60` (= 2 s at 30 FPS — derive from clip's actual fps), `minimum_matching_threshold=0.85`, `minimum_consecutive_frames=3`. **Do not change the live path's params in this phase** — that's a separate Wave.
- [ ] Update `deploy/cv-sidecar/bench.py` with a `--file` mode that exercises the new path on a checked-in 10-second sample clip. Ship the sample under `deploy/cv-sidecar/samples/sample.mp4` (small, license-clean — record one with the test pasture cam).

**Dependencies**: Phase 0 (the constants + types are referenced by the Rust side, but the sidecar protocol is pure Python).

**Validation**:
- `python deploy/cv-sidecar/cv_sidecar.py` started manually, then a Python smoke client sends a `0x01` request with `samples/sample.mp4` → all frames return with detections + `track_id`s, terminator arrives, no leaked GPU memory.
- Live path still works (smoke_client.py unchanged smoke test).

**Wiki grounding**: `playbook-accurate-herd-counting-2026-05-27` § P0 #1 specifies the ByteTrack params for stationary/slow herds; uploads are ~always slow-pan or stationary (drone flyovers, pasture cams), so the playbook params apply directly. Live keeps its current params for now.

### Phase 2 — Daemon upload ALPN + queue (estimated: 2 days)

**Goal**: daemon accepts iroh-blobs uploads on `UPLOAD_ALPN`, stages them under `<data_dir>/uploads/<blake3>/`, queues processing, and processes one at a time when no live session is active.

**Tasks**:
- [ ] Register `UPLOAD_ALPN` on the existing iroh `Router` (the same router that already carries moq-live, gossip, `CONTROL_ALPN`, `ADMIN_ALPN`). Allowlist gating: reuse the `[control_plane.admins]` set from Wave 12 — uploaders must be admins. Audit-log every accepted/rejected upload via the existing append-only JSONL audit infra (`herd-scout-daemon/src/audit.rs` is already present in the working tree).
- [ ] Per-stream handler: read `UploadClientMsg::Push`, enforce `size_bytes ≤ 2 GiB` upfront, accept the iroh-blobs transfer (the bytes ride iroh-blobs natively — the JSON message is the metadata wrapper). On completion, verify the iroh-computed BLAKE3 matches the reported one; reject with `Error { code: "hash_mismatch" }` on disagreement.
- [ ] Stage: hardlink (fallback symlink) the blob into `<data_dir>/uploads/<blake3-hex>/clip.<ext>`. Write `meta.json`. Append an `UploadEntry { state: Queued }` to the in-memory queue (and persist the queue to `<data_dir>/uploads/queue.json` so it survives restarts).
- [ ] Build a `UploadProcessor` actor that:
  - Subscribes to `ConnectionStatus` (existing daemon state).
  - When status is `Idle | Stopped` and the queue is non-empty, pop the head, set state `Decoding`, emit `ServerMsg::UploadStatus`, send `0x01 (clip_id, path)` to the sidecar, **fan the sidecar responses out to the GUI socket exactly like live frames** — i.e. produce `ServerMsg::Frame` (re-encode JPEG preview from the BGR24 the daemon already has from the sidecar's frame echo... see Open Question 1) + `ServerMsg::Detections { clip_id: Some(...) }` per frame.
  - On terminator: write `report.json` (Phase 3), set state `Done`, emit `UploadStatus { Done }`, advance the queue.
- [ ] Live preemption guard: when `ConnectionStatus` transitions to `Connecting | Connected` mid-process, do **not** kill the in-flight upload — let the current frame complete, then either (a) suspend the upload and resume after live ends, or (b) finish the current clip first if it's > 90% done. Pick (b) for simplicity in v1; document (a) as a follow-up. Live phone broadcast still works because the **sidecar does not interleave clients** — the daemon's existing CV task and the upload processor must coordinate which one is the active sidecar caller via a `tokio::sync::Mutex<SidecarHandle>`.
- [ ] Cap enforcement (server-side): `payload_len > 2 GiB` → `RejectedTooBig` before bytes are accepted; clip duration > 10 min → `Failed { reason: "duration_cap_exceeded" }` after probing with `cv2.VideoCapture.get(cv2.CAP_PROP_FRAME_COUNT) / fps` (the sidecar reports this in a "probe" sub-call before committing).
- [ ] `ListQueue` and `CancelQueued` RPCs.

**Dependencies**: Phase 0, Phase 1.

**Validation**:
- `herdctl` has a temporary `upload-test` subcommand (full UX is Phase 4) that sends a 10-second sample over `UPLOAD_ALPN` from the dev box → daemon stages it, queues it, processes it (no live session active), writes `report.json`, finishes.
- Pair a phone live; queue an upload; verify the upload is `Queued` until live ends; verify it then progresses.
- Audit log shows `upload_accepted`, `upload_processed`, `upload_done` records.

**Wiki grounding**: Reuses the audit + admin-allowlist + ALPN-on-router patterns from `plan-android-admin-allowlist-app-2026-05-27`. iroh-blobs as the byte transport is `iroh-sync-stack` § iroh-blobs.

### Phase 3 — Report writer applying the accurate-counting playbook (estimated: 1.5 days)

**Goal**: per clip, write a `report.json` that applies the accurate-counting pipeline to the stream of `(frame_pts_ms, [DetWire])` from the sidecar.

The report schema (committed):

```json
{
  "schema_version": 1,
  "clip_id": "9c2f...",
  "filename": "drone-flyover-2026-05-28.mp4",
  "duration_ms": 87520,
  "fps": 29.97,
  "frame_count": 2624,
  "processing_ms": 38120,
  "bytrack_params": {
    "track_activation_threshold": 0.35,
    "lost_track_buffer": 60,
    "minimum_matching_threshold": 0.85,
    "minimum_consecutive_frames": 3
  },
  "summary": {
    "median_active_count_total": 47,
    "median_active_count_per_class": { "horse": 0, "sheep": 0, "cow": 47 },
    "bootstrap_ci_95_total": [44, 51],
    "max_simultaneous_total": 53,
    "unique_track_ids_total": 62,
    "unique_track_ids_eligible": 49
  },
  "tracks": [
    {
      "track_id": 17,
      "class": "cow",
      "first_frame": 14,
      "last_frame": 2611,
      "frame_count": 2491,
      "eligible": true,
      "mean_confidence": 0.81,
      "centroid_track_len_px": 2840
    }
  ],
  "frames_per_window": [
    { "window_center_frame": 15, "active_ids": 38 },
    { "window_center_frame": 45, "active_ids": 41 }
  ],
  "warnings": [
    "closure_uncertain: 4 tracks span the entire clip (animals may have entered/exited)"
  ]
}
```

**Tasks**:
- [ ] In the daemon, accumulate per-frame `(pts_ms, Vec<DetWire>)` during the upload's processing.
- [ ] At terminator, run the playbook layer-2.5 + layer-3 logic (in pure Rust, no Python re-call):
  - **Cumulative-frame filter**: a `tracker_id` is `eligible` iff seen in ≥ 15 cumulative frames.
  - **Centroid-jump sanity**: drop frames where Δcentroid > 150 px/frame for an ID; record dropped count.
  - **Active-count-per-frame**: `len(unique eligible tracker_ids in this frame)`.
  - **Median over 30-frame windows**: emit `frames_per_window` as overlapping windows centered every ~1 s.
  - **Bootstrap CI**: 1000-resample over the per-frame active counts → 2.5th / 97.5th percentile.
  - **Per-class summary**: same logic restricted to one class at a time.
  - **Closure warnings**: any track whose `[first_frame, last_frame]` spans `[0, frame_count-1]` triggers `closure_uncertain`.
- [ ] Write `report.json` atomically (`tempfile + rename`) into the upload dir.
- [ ] Emit a `ServerMsg::UploadStatus { state: Done }` with the report's headline numbers (median, CI) inlined for the GUI's queue panel.
- [ ] Unit tests on the layer-2.5/3 logic with synthetic detection streams.

**Dependencies**: Phase 2.

**Validation**:
- A 60-second test clip with known cattle count (e.g. 3 cattle in frame the entire time) reports `median_active_count_total = 3 ± small_CI`.
- A test clip with one cow walking in and out triggers `closure_uncertain`.
- Re-running the same clip produces an identical `report.json` (modulo `processing_ms`).

**Wiki grounding**: `playbook-accurate-herd-counting-2026-05-27` § P0 #1-3 + § P1 #8 (bootstrap CI). The schema is the canonical artifact for "what was the count of clip X" — enables the validation Tier 2 (conformal) and Tier 1 (EID reconciliation, future) workflows from `count-validation-conformal`.

### Phase 4 — herdctl push CLI (estimated: 1 day)

**Goal**: `herdctl push <file>` uploads a clip to the daemon and tails its progress.

**Tasks**:
- [ ] New subcommand in `herdctl/`: `push [--daemon <node-id>] <path>`.
- [ ] Compute BLAKE3 locally (already a transitive dep via iroh-blobs).
- [ ] Open a stream on `UPLOAD_ALPN` to the active daemon (selected via the existing `herdctl` daemon-selection mechanism from Wave 12), send `Push { filename, size_bytes, blake3_hex }`, then push the bytes via iroh-blobs.
- [ ] After `Accepted`, transition into a tail mode that subscribes to `UploadServerMsg` updates (the same connection or a new `ListQueue`-poll loop — pick whichever fits the existing herdctl style) and prints progress: `[######    ] 60% queued -> decoding -> done`.
- [ ] On `Done`, fetch the report from the daemon (a new `UploadClientMsg::FetchReport { blake3_hex }` is the cleanest path; alternative is an admin RPC `cat <data_dir>/uploads/<blake3>/report.json`) and print headline `summary` numbers.
- [ ] `herdctl uploads list` and `herdctl uploads cancel <prefix>` for queue management.

**Dependencies**: Phase 2, Phase 3.

**Validation**:
- `herdctl push samples/sample.mp4` succeeds; prints progress; ends with `cow: 3 (CI [3, 3])`.
- `herdctl uploads list` shows recent processed clips.

**Wiki grounding**: Wave 12 established `herdctl` as the multi-purpose admin client; this is a natural extension.

### Phase 5 — Desktop GUI drag-drop (estimated: 1.5 days)

**Goal**: drop an MP4 onto the egui desktop window → upload progress appears in a side panel → finished clips show the report's headline numbers.

**Tasks**:
- [ ] In `herd-scout-gui/` (egui), enable file-drop on the main window via `egui::Context::input(|i| i.raw.dropped_files)`.
- [ ] On drop, validate locally (size ≤ 2 GiB, extension in {mp4, mov, m4v}). Reject obviously bad files in the GUI without contacting the daemon.
- [ ] Compute BLAKE3 in a background thread (`tokio::task::spawn_blocking`); show a progress spinner.
- [ ] Open a fresh `UPLOAD_ALPN` stream from the GUI process (the GUI is local but talks iroh too — re-use the iroh node the GUI already uses? Or pipe via the Unix socket to the daemon's iroh node? **Decision**: GUI sends an `UploadHandoff { path, blake3_hex, size }` over the existing GUI Unix socket, and the daemon (which already has the iroh node) accepts the bytes via local file-stream — i.e. the GUI doesn't need its own iroh node. This simplifies the GUI and matches the existing GUI-socket-only pattern.). Update Phase 0 wire types accordingly: add `ClientMsg::UploadHandoff { path, blake3_hex, size }` and `ClientMsg::UploadCancel { blake3_hex }`.
- [ ] New "Uploads" side panel in egui: list of pending + recent clips, each row shows filename, state (`Queued | Decoding | Done | Failed`), progress %, and on `Done`, the headline count + CI.
- [ ] When an upload is `Decoding` and the user clicks its row: switch the main video pane from live-camera mode to "playing back this clip with overlays" — same `ServerMsg::Frame + Detections` rendering path, but filtered to that `clip_id`.
- [ ] Keyboard shortcut: `Ctrl/Cmd+O` opens a native file picker.
- [ ] Persist the uploads-panel state across GUI restarts by re-fetching `ListQueue` on connect.

**Dependencies**: Phase 0 (extended to add `UploadHandoff` to `ClientMsg`), Phase 2.

**Validation**:
- Drop an MP4 onto the GUI; see it queue (or process immediately if no live session); see live overlay frames during processing; see headline numbers when done.
- Clicking on a previously-processed clip replays its overlay (decoded fresh through the sidecar — we don't cache frames).
- File rejection: drop a 4 GB file → instant GUI error, no daemon round-trip.

**Wiki grounding**: matches the egui "drop video file" pattern referenced in `drone-vision-software` § video input; preserves the desktop-as-driver mental model from the mobile-to-desktop RFC.

### Phase 6 — Acceptance + docs (estimated: half a day)

**Tasks**:
- [ ] Update `deploy/README.md` with the upload pipeline, including the `UPLOAD_ALPN`, the data-dir layout, and how to inspect a `report.json`.
- [ ] Add an `inventory/upload-clip-bench-YYYY-MM-DD.md` capturing throughput on a real clip (target: ≥ 23 FPS sustained matching the live bench's Phase 2 number; if less, identify whether the bottleneck is decode or sidecar).
- [ ] Update `wiki/_index.md` and `output/_index.md` with this plan.
- [ ] Append to `log.md`.

## Risks & Mitigations

| Risk | Source | Mitigation |
|---|---|---|
| **Sidecar contention with live phone broadcast** | `cv-sidecar-bench-2026-05-27` (single-client sidecar) | Queue uploads behind active live (Decision 4). Mutex on the sidecar handle. Surface "Queued (live active)" in GUI. |
| **HEVC files won't decode on default cv2 builds** | OpenCV build config varies; not in our control | First cut: MP4/H.264 only. Document HEVC as a watch item. Surface "unsupported codec" error if probe fails. |
| **10-min cap missed by a clip that's 10:30 long** | Soft cap UX expectation | Probe `frame_count / fps` server-side before committing; reject with explicit `duration_cap_exceeded`. The cap is structural, not a soft guideline. |
| **iroh-blobs alpn collision on the router** | New ALPN registered with existing router | Use distinct prefix `herd-scout/upload/1`. Versioned the same way `CONTROL_ALPN` and `ADMIN_ALPN` are. |
| **Hash mismatch between client and daemon** | Network corruption, GUI bug | Compute on both ends (BLAKE3 is cheap); reject with `RejectedHashMismatch` and force re-upload. iroh-blobs already verifies on ingest. |
| **Decoding an upload while live session arrives** | Live preemption case (Decision 4) | v1: finish the current clip if > 90% done; otherwise suspend after the next frame and resume when live ends. The Mutex on `SidecarHandle` arbitrates. |
| **Disk fill from accumulated uploads** | No retention policy | Each clip is ~150 MB - 2 GB. v1: no auto-cleanup; user runs `herdctl uploads gc` (Phase 6 follow-up if asked). Document the data-dir path so users can hand-prune. |
| **`cv2.VideoCapture` is rotation-aware on some platforms but not others** | OpenCV behavior; mobile-recorded MP4s often have a 90° rotation tag | Probe orientation tag during the sidecar's probe call; pass an explicit `cv2.ROTATE_*` to the read loop. Test with iPhone-recorded test clips. |
| **Daemon restarts mid-process** | Power loss, intentional restart | Queue persisted to `queue.json`; on restart the in-flight clip resets to `Queued` and is re-decoded from byte zero. Idempotent (BLAKE3 dedupe). v1 acceptable; resume-from-frame is out of scope. |
| **Sidecar wire-protocol bump breaks live path** | Phase 1 prefixes a `request_kind` to the existing protocol | Default the daemon's live path to send `0x00`; default the sidecar to treat absent prefix as `0x00` for one release; add a CI smoke that runs the live smoke client against the new sidecar. |
| **ByteTrack params for upload differ from live** | Phase 1 sets playbook-recommended params per-clip | Reset tracker per clip (already a clean boundary). Document in `meta.json` which params were used so reports stay reproducible. |

## Open Questions

1. **Frame echo from sidecar to daemon for `ServerMsg::Frame`**: the live path produces `Frame` JPEGs by JPEG-encoding the daemon's pre-sidecar BGR24 frame. For uploads, the daemon doesn't have those bytes — the sidecar holds them. Two options: (a) sidecar echoes the BGR24 of every Nth frame to the daemon for JPEG-encoding (bandwidth: ~1.5 MB/frame at 720p, eats the wire), (b) sidecar JPEG-encodes preview frames itself and includes them in the response (adds OpenCV `imencode` cost on the sidecar host but it's <2 ms/frame). **Tentatively pick (b);** verify in Phase 1 prototyping before committing.
2. **Should `herdctl push` accept a directory or glob for batch upload?** v1: no. Single file only. Easy follow-up.
3. **Per-clip ByteTrack-param overrides** — the report locks in the params used. A future version could let `herdctl push --params high-recall` switch presets for the same clip. Not v1.
4. **Should the GUI offer a "process again with different params" button?** Useful for the playbook P1 work (capturing 200 labeled frames and tuning F1-peaks). Out of scope for first cut; the bytes are still in iroh-blobs so re-processing is a metadata-only retrigger.

## Sources Consulted

| Source | Contribution |
|---|---|
| [[plan-mobile-to-desktop-iroh-rfc-2026-05-20]] | The live phone-to-daemon path; upload is its batch sibling. Confirms iroh as committed data plane. |
| [[plan-optimize-cv-sidecar-trt-yolo11s-2026-05-26]] | Sidecar wire protocol; ByteTrack integration; YOLO11s + embedded NMS as the model contract. |
| [[playbook-accurate-herd-counting-2026-05-27]] | Layer 2.5/3 algorithms (cumulative-frame filter, median-of-active, bootstrap CI); ByteTrack params for stationary herds; the report schema's design. |
| [[iroh-sync-stack]] | iroh-blobs as the right transport (BLAKE3, resumable, kB→TB). |
| [[plan-android-admin-allowlist-app-2026-05-27]] | ALPN-on-router pattern; admin allowlist gating; audit log infra; herdctl daemon-selection. |
| [[plan-deploy-daemon-on-1060-laptop-2026-05-22]] | Deployment shape (daemon on bigdeal, GUI on laptop) — drives the iroh-blobs decision. |
| `cv-sidecar-bench-2026-05-27` | 23 FPS Phase 2 throughput as the realistic upload-processing rate. |
| `herd-scout-ipc/src/lib.rs` (direct) | `ServerMsg`/`ClientMsg`/`AdminClientMsg` enum patterns; ALPN constant placement; framing convention. |
| `herd-scout-daemon/src/cv/task.rs` (direct) | `VideoFrame` construction; the watch-channel boundary the upload processor must replicate. |
| `deploy/cv-sidecar/cv_sidecar.py` (direct) | Existing wire protocol; `cv2.VideoCapture` is already a transitive import. |
| [[herd-counting-pipeline]] | The 5-layer pipeline that the report makes concrete. |
