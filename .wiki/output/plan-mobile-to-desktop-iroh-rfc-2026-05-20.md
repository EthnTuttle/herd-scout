---
title: "RFC: Mobile-to-desktop app on iroh — desktop driver, phone-on-drone camera"
type: plan
format: rfc
status: proposed
generated: 2026-05-20
session: 2026-05-20-132147
author: herd-scout team
sources:
  - concepts/mobile-desktop-architecture.md
  - concepts/iroh-sync-stack.md
  - concepts/iroh-docs-fms-schema.md
  - concepts/android-on-drone.md
  - concepts/drone-hardware.md
  - concepts/drone-vision-software.md
  - concepts/implementation-plan.md
  - concepts/herd-scout-positioning.md
  - concepts/fms-data-model.md
  - concepts/oss-drone-fms-pipeline.md
  - concepts/livestock-eid-rfid.md
---

# RFC: Mobile-to-desktop app on iroh

> Desktop driver + Android phone-on-drone camera (MVP). Generated from herd-scout `.wiki/` (14 concept articles, 17 raw sources, 2 research rounds completed 2026-05-20).

## Executive summary

Build an Android-phone-as-streaming-camera + Rust desktop processor as the MVP, leveraging the iroh stack already declared in this workspace. **The phone is a dumb camera that streams video over iroh-live to one or more desktop "driver" peers** — no on-device inference, no MAVLink, no drone integration. All ML/CV runs on desktop. The MVP is **drone-agnostic by design**: the phone is the camera, whatever's holding it (drone, vehicle, hand) is outside the software architecture. This avoids edge-compute complexity, lets users repurpose existing low-end Android phones, keeps the data plane uniform (video and future records ride the same iroh transport), and ships *today* on whatever drone the user already owns.

The single biggest known unknown is that the workspace's `Cargo.toml` references `vendor/iroh-live/...` paths that do not exist on disk. Phase 0 adds that as a git submodule before any feature work.

## Context & scope

### Problem

The user wants a "mobile-to-desktop" application leveraging iroh, where the desktop is the main driver and the phone is a camera with some native processing/connectivity. The concrete MVP framing is **phone-as-camera, drone-agnostic**: an Android phone (with whatever drone, vehicle, or human is carrying it) streams video to the desktop, which runs herd counting / fenceline inspection. Processing happens on desktop so that low-end phones suffice. The drone — whatever it is — is outside the software boundary; programming it from the app is a future phase, not the MVP.

### What the wiki says

- **iroh transport + iroh-blobs + iroh-gossip + iroh-smol-kv is the right data plane** — already in repo, KV CRDT with range-based set reconciliation handles arbitrary offline durations, BLAKE3 content addressing handles photos/clips ([[iroh-sync-stack]], [[mobile-desktop-architecture]]).
- **Tauri 2** is the only mature path to share a single Rust core across phone + desktop ([[mobile-desktop-architecture]]). Mobile is alpha-tier; desktop is solid. Compose Multiplatform UI calling Rust via UniFFI is the documented fallback.
- **`vendor/iroh-live/moq-media-android`** is declared in workspace deps — Android side of moq-based video pipe is presumably the streaming primitive ([[Cargo.toml]]).
- **Phone-on-drone verdict** ([[android-on-drone]]): phone-as-FC is a no, phone-as-(camera + 4G bridge) is good, phone-as-companion-with-MAVLink is a stretch goal. Used flagship Android NPUs are competitive with Jetson Nano for MobileNet workloads — relevant for *post-MVP* edge inference, not the first cut.
- **Drone hardware budget** ($530 phone-on-drone build) is documented in [[implementation-plan]].
- **Vision pipeline** ([[drone-vision-software]]): YOLOv5/v7/HerdNet over RTSP/RTMP/UDP is the standard pattern. We replace RTSP-over-LAN with iroh-live-over-anywhere.

### What the wiki does NOT say

- The exact state of `vendor/iroh-live/` (the repo references paths that don't exist locally — see Open Questions)
- Pairing/onboarding flow specifics (QR-code? iroh-tickets?)
- Live video bandwidth numbers on rural 4G

## Goals

1. **Single Rust core** running on desktop and phone (Android first), with `iroh` and `iroh-live`/moq for the video plane.
2. **Phone is a streaming camera only.** No on-device inference. No MAVLink. No flight control. No drone-specific code.
3. **Desktop is the driver.** Receives stream(s), runs CV inference (YOLO/HerdNet), displays results, owns persistence.
4. **Drone-agnostic.** The MVP works whether the phone is bolted to a DJI Mavic, an ArduPilot quad, a vehicle dash, or held in a hand. The drone is the user's existing equipment; the app neither sees nor cares which one it is.
5. **Pairing works regardless of network shape.** Iroh's transport handles peer-direct on LAN, hole-punch + relay fallback across the internet, transparently. The app does not need to know which path is in use.
6. **Reliability over latency.** Single-digit-second latency is acceptable. Surviving flaky connectivity beats fast-on-LAN-only.
7. **Foundations for later layers.** The data plane chosen for video must coexist with future iroh-smol-kv records (Asset/Log/Quantity) without re-architecting.

## Non-goals

- On-device ML on the phone (deferred — phone is the bottleneck-prevention principle drives this)
- **Drone-specific anything.** No MAVLink, no DJI SDK, no flight control, no telemetry read, no waypoint upload, no airframe assumptions. The MVP is **drone-agnostic**: the phone is the camera; what carries it is the user's problem. Programming the drone from desktop/mobile is a future phase.
- Phone as flight controller (explicitly rejected — see [[android-on-drone]])
- iOS support (deferred — Android-first; Tauri 2 mobile alpha pain is real, doubling platforms doubles it)
- Full FMS records / logs / compliance (deferred — Phase 3+)
- Photogrammetry / WebODM ingestion (deferred — Phase 4)
- BLE EID reader bridge (deferred — separate `herd-scout-eid` crate per [[livestock-eid-rfid]])
- Live NDVI from a flying drone (impossible per [[oss-drone-fms-pipeline]] § L6 — radiometric calibration requires post-flight processing)

## Design

### High-level architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Some platform that holds the phone — out of scope          │
│  (a drone flown by its own GCS, a vehicle, a hand, etc.)    │
│                                                             │
│  Android phone (the only thing the MVP sees)                │
│  └─ Android app                                             │
│     ├─ CameraX → YUV frames                                 │
│     ├─ rusty-codecs (workspace dep) → H.264 encode          │
│     ├─ moq-media-android (workspace dep) → MoQ tracks       │
│     └─ iroh node → publish track to namespace               │
└─────────────────────────────────────────────────────────────┘
                              │  (iroh transport: peer-direct / hole-punch / relay — transparent)
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  Desktop "driver" (Linux/macOS/Windows)                     │
│  ├─ iroh node (joined to same namespace via ticket)         │
│  ├─ moq-media-egui (workspace dep) → subscribe & decode     │
│  ├─ Frame fan-out:                                          │
│  │    ├─ → eframe/egui display                              │
│  │    └─ → CV inference task (YOLOv5/v8/HerdNet)            │
│  ├─ Detections → overlay on display + append to ring buffer │
│  └─ (future) write Observation logs to iroh-smol-kv         │
└─────────────────────────────────────────────────────────────┘
```

The "platform that holds the phone" box is intentionally drawn as out of scope. Whether it's a drone, a truck dash, or a chest harness, nothing in the architecture changes. The phone is the camera; everything upstream of CameraX is the user's problem.

**One namespace per "session".** Initial pairing produces an iroh ticket the desktop emits as a QR code; phone scans → joins namespace → starts publishing the camera track. Multiple desktops can subscribe to the same namespace (trainer + apprentice watching together).

### Component decisions

| Component | Choice | Rationale (wiki citation) |
|---|---|---|
| **UI / app shell — desktop** | egui (`eframe`) | Already in `desktop/Cargo.toml`. Pure Rust, ships on all desktop OSes, no webview. Sufficient for "video + overlay + sidebar". |
| **UI / app shell — phone (MVP)** | Native Android (Kotlin) thin shell wrapping Rust core via JNI/`uniffi` | Tauri 2 mobile is alpha-tier ([[mobile-desktop-architecture]]); the workspace already targets Android via `moq-media-android`. Skip the cross-platform UI abstraction for the MVP. |
| **Video transport** | iroh + moq (via `vendor/iroh-live`) | Already in workspace. MoQ-over-iroh gives sub-second latency with NAT traversal and relay fallback "for free" (per [[iroh-sync-stack]]: the LAN path is peer-direct, the WAN path falls back to relay). |
| **Encoding (phone)** | Hardware H.264 via `rusty-capture` / `rusty-codecs` (workspace deps) | Already in workspace. Hardware encoders on every modern Android phone; CPU-only codec on the phone would be the bottleneck the user wants to avoid. |
| **CV runtime (desktop)** | ONNX Runtime or `ort` crate, with YOLOv5n/YOLOv8n weights as default | [[drone-vision-software]] benchmarks; pure Rust binding via `ort` keeps the Rust core unified. CUDA optional. |
| **Vision model (default)** | YOLOv5n / v8n (COCO classes incl. cow/horse/sheep) | [[drone-vision-software]]: pre-trained on COCO; cattle/horse/sheep classes available; works fine for MVP demo. HerdNet ([[herdnet]]) is the upgrade path once we collect aerial-perspective training data. |
| **Records data plane (Phase 3+)** | iroh-smol-kv per [[iroh-docs-fms-schema]] | Same iroh node already running for video. Per-device author keys; per-farm namespaces; one scalar per key; HLC timestamps; SQLite projection. |
| **Pairing** | iroh ticket as QR code on desktop, scanned by phone | Per [[iroh-docs-fms-schema]] § "Namespace key distribution is your problem"; iroh-tickets crate already in workspace deps (`iroh-tickets = "0.5.0"`). |

### Pairing flow (MVP)

```
Desktop                                    Phone (Android)
───────                                    ──────────────
1. App start
2. Generate iroh node + namespace
3. Mint iroh ticket (read+write capability)
4. Render ticket as QR code on screen
                          ─── QR ────────► 5. Open app, "Pair with desktop"
                                            6. Camera scan → ticket bytes
                                            7. Connect to namespace via ticket
                                            8. Open camera, start MoQ publish
9. Subscribe to camera track ◄─── stream ── 9. (continuous)
10. Decode → display + inference
```

No internet required for LAN pairing. Cross-internet pairing uses iroh's hole-punch + relay fallback transparently.

### Data plane summary

For the MVP, the only data flowing over iroh is **live video tracks**. Records (Asset/Log/Quantity per [[iroh-docs-fms-schema]]) come in Phase 3. By choosing iroh as the transport now, the same node and pairing flow extends to records later — no second sync system, no parallel namespaces.

Storage on desktop:
- **Video**: optionally archive incoming frames as MP4 chunks on local disk (rolling buffer); future Phase pushes interesting clips to `iroh-blobs` for cross-peer access.
- **Detections**: in-memory ring buffer for MVP; SQLite persistence in Phase 3.
- **Settings**: local config file; no sync needed for MVP.

### What runs where (MVP)

| Concern | Phone (Android) | Desktop |
|---|---|---|
| Camera capture | ✓ | — |
| Encode (H.264) | ✓ (hardware) | — |
| Stream publish (MoQ over iroh) | ✓ | — |
| Stream subscribe + decode | — | ✓ |
| Display | (preview only) | ✓ |
| ML inference | **no** | ✓ |
| Detection overlay | — | ✓ |
| Persistence | — | ✓ (rolling buffer + settings) |
| Pairing UI | scan QR | show QR |

## Alternatives considered

### Alternative A — Tauri 2 single binary across phone + desktop
**Pros**: maximum code reuse; single Rust process; future-proof for shared UI.
**Cons**: Tauri 2 mobile is alpha-tier with TLS/OpenSSL cross-compile pain and Xcode device-deploy quirks ([[mobile-desktop-architecture]]). The MVP doesn't need shared UI — phone is a single-screen "show preview + status" app. Adopting Tauri 2 mobile would absorb its rough edges before any user-facing value.
**Verdict**: Rejected for MVP. Reconsider in Phase 3 when records UI surface grows.

### Alternative B — RTSP over LAN
**Pros**: dirt simple; well-trodden path documented in [[drone-vision-software]] and [[implementation-plan]]; off-the-shelf phone apps (IP Webcam) already ship.
**Cons**: LAN-only; no NAT traversal; no relay fallback; doesn't extend to a multi-peer architecture; doesn't compose with future record sync. Defeats the iroh-as-data-plane bet.
**Verdict**: Rejected. RTSP would force re-architecture for cross-internet and multi-device.

### Alternative C — Phone runs ML, sends only detection events
**Pros**: tiny bandwidth; latency is irrelevant; multiple phones scale to one desktop trivially.
**Cons**: phone *is* the bottleneck (user explicitly rejected this); thermal load on a sealed phone in a drone case is real; MVP can't iterate on detection algorithms without phone OTA; loses the "low-end phone" demographic ([[android-on-drone]] § thermal).
**Verdict**: Rejected for MVP. Reconsider as an optional mode in Phase 4 once desktop pipeline is stable.

### Alternative D — DJI/closed drone SDK as the video source
**Pros**: best image quality, gimbal stabilization, telemetry.
**Cons**: closed SDK; tied to one OEM; defeats the "open source or bust" principle ([[herd-scout-positioning]]).
**Verdict**: Rejected.

### Alternative E — Compose Multiplatform UI calling Rust via UniFFI
**Pros**: stable mobile UI; same Kotlin compiler chain as Android-only; later iOS path available.
**Cons**: more upfront ceremony; UniFFI bindings to maintain; for the MVP's single-screen Android app, plain Kotlin + JNI is enough.
**Verdict**: Rejected for MVP. **Recommended fallback if Tauri 2 ever becomes the path forward** ([[mobile-desktop-architecture]]).

## Risks & mitigations

| Risk | Source | Mitigation |
|---|---|---|
| **`vendor/iroh-live/` missing on disk** | direct repo inspection (`Cargo.toml` references `vendor/iroh-live/iroh-live` etc.; `vendor/` does not exist) | Phase 0: add iroh-live as a git submodule per user direction. If the chosen upstream's API drifts, fall back to the `Frando/moq` git deps already declared. Build must go green before any feature work. |
| **Tauri 2 mobile alpha rough edges** | [[mobile-desktop-architecture]] | Avoid Tauri mobile in MVP; use native Android shell. Adopt Tauri 2 only when desktop stabilizes and shared UI becomes valuable. |
| **Live video on rural 4G** | [[android-on-drone]] § 4G latency 80-250 ms | iroh's hole-punch + relay fallback handles WAN paths transparently. Reliability is the goal, not low latency — buffer and reconnect aggressively. Phone-and-desktop colocation is NOT assumed. |
| **Phone thermal in drone case** | [[android-on-drone]] § thermal | Slipstream cooling helps in flight; ground-idle is the risk window. Vented case + screen off + headless service. Keep phone CPU load low (encode only, no inference). |
| **Phone vibration → IMU/OIS** | [[android-on-drone]] § vibration | Soft-mount foam/gel pads; disable EIS/OIS in CameraX config to avoid jelly artifacts compounding with prop vibration. |
| **iroh-smol-kv API differs from iroh-docs** | [[iroh-docs-fms-schema]] § gotchas | Phase 3 (records) verifies API against live `iroh-smol-kv` source on `iroh-098` branch before code commits. |
| **Wallclock LWW corruption (when records arrive)** | [[mobile-desktop-architecture]] § anti-patterns | HLC timestamps mandated from Phase 3 onward. |
| **DroneKit-Android dead** | [[android-on-drone]] | Not a Phase 0-2 risk (no MAVLink). When Phase 5+ adds telemetry, fork or use `io.dronefleet.mavlink`. |
| **Single-bottleneck desktop** | implicit (one desktop, N phones) | MVP assumes 1 desktop + 1 phone. Phase 4 can add fan-out (multiple desktops or a relay node). |
| **GPS on phone is consumer-grade** | [[drone-hardware]] | Don't use phone GPS for nav. Drone autopilot owns flight. Phone GPS is fine for geotagging captured frames as metadata. |

## Cross-cutting concerns

### Security
- Iroh ticket = capability. Treat it like an SSH key. Generate fresh per session for MVP; no long-lived re-pair until Phase 3.
- No cloud account; no user identity; no PII left on a remote server.
- Local archive on desktop is AES-encryptable via standard Rust crates if the user wants it (out of scope for MVP).

### Performance & reliability
- **Reliability prioritized over latency.** Per user direction (2026-05-20), the goal is "the stream gets through and frames are usable" rather than a tight latency budget. Buffer aggressively; backpressure cleanly; reconnect transparently.
- **Phone**: hardware H.264 at 1080p30; CPU should idle <30%. If not, drop to 720p30.
- **Desktop**: YOLOv5n@COCO at 30 FPS on a 2020+ laptop CPU; CUDA optional. Inference rate caps at ~5–10 FPS; display rate runs full.
- **No LAN-vs-WAN distinction.** Iroh's transport handles peer-direct on LAN, hole-punch + relay fallback across the internet, transparently. The app does not need to know which path is in use; if a path exists, the stream flows.
- **Loose latency budget**: low single-digit seconds is acceptable. Anything that stays glued together over a flaky 4G link beats anything brittle on a fast LAN.

### Backwards compatibility
- N/A for MVP (greenfield).
- Architecture choices (iroh namespace per session, Asset/Log primitives planned for Phase 3) deliberately avoid painting into corners — see [[iroh-docs-fms-schema]] § schema evolution rules.

### Observability
- `tracing` crate already in `desktop/Cargo.toml`. Add structured spans on the iroh subscribe path, the decode path, and the inference path. Log frame timestamps end-to-end for latency triage.

## Implementation phases (overview)

(Detailed per-phase tasks at end of this RFC.)

| Phase | Title | Outcome | Effort estimate |
|---|---|---|---|
| **0** | Workspace builds | `vendor/iroh-live` submodule added; `cargo build --workspace` green; CI guards it | 0.5–2 days |
| **1** | Desktop receives a moq stream | egui window shows decoded frames from a hardcoded test track | 1–2 days |
| **2** | Android app publishes camera | Phone scans QR ticket → publishes camera as a MoQ track over iroh | 3–7 days |
| **3** | Desktop CV inference | Frames fan out to `ort` + YOLOv5n; bounding boxes overlaid on display | 2–4 days |
| **4** | Field test (hardware-adaptive) | Whichever of three branches matches the user's actual drone/phone setup; validate reliability not latency | 3–7 days |
| **5** | Pairing + multi-session UX | QR display, ticket regen, multiple desktops can subscribe; reconnect UX | 3–5 days |
| **post-MVP** | Records (Phase 3 of [[herd-scout-positioning]]) | iroh-smol-kv schema; Asset/Log persistence; SQLite projection | weeks |

Total MVP (Phases 0-5): ~2-4 weeks of focused work given existing hardware (no Holybro acquisition).

## Detailed phases

### Phase 0 — Workspace builds (CRITICAL PRE-WORK)

**Goal**: `cargo build --workspace` succeeds.

**Context**: `Cargo.toml` declares `vendor/iroh-live/iroh-live`, `vendor/iroh-live/moq-media-android`, etc., but `vendor/` does not exist on disk. `desktop/src/main.rs` is a "Hello world" stub. Nothing currently compiles end-to-end.

**Tasks**:
- [ ] Identify upstream URL for iroh-live (likely n0-computer's iroh-live experimental repo or a personal fork — confirm before committing).
- [ ] `git submodule add <url> vendor/iroh-live` and commit `.gitmodules`.
- [ ] Pin to a known-good commit; document in README ("clone with `--recursive` or run `git submodule update --init` after clone").
- [ ] If the submodule's API has drifted from what `Cargo.toml`'s `[workspace.dependencies]` table expects, prefer pinning the submodule to an older compatible commit over editing path-deps. The git deps for `Frando/moq` (`hang`, `moq-lite`, `moq-mux`, `moq-relay`, `moq-native`) remain as the documented fallback if the submodule path becomes unworkable.
- [ ] Verify `cargo build --workspace` succeeds on Linux/macOS host.
- [ ] Confirm Android target builds: `cargo build --target aarch64-linux-android -p moq-media-android`.
- [ ] Add a CI workflow that runs `cargo build --workspace` (Linux + macOS) so the build state never silently rots again.

**Validation**: green build locally; CI passes; `git clone --recursive` on a fresh checkout produces a buildable tree.

**Wiki grounding**: This phase is necessary because the wiki ([[mobile-desktop-architecture]], [[iroh-sync-stack]]) presumes the iroh-live workspace as the streaming primitive but does not document its source-of-truth. The repo state (verified during this RFC) shows the path doesn't exist locally.

### Phase 1 — Desktop receives a MoQ stream

**Goal**: egui window on desktop shows decoded video from a test MoQ track over iroh.

**Tasks**:
- [ ] Replace `desktop/src/main.rs` Hello-world with an `eframe` app that initializes an iroh node.
- [ ] Hardcode connection to a test track (use a local moq-relay or a second desktop instance publishing a test pattern).
- [ ] Subscribe to the track via `moq-media-egui`.
- [ ] Render decoded frames in egui central panel.
- [ ] Add latency overlay (frame timestamp - now).

**Dependencies**: Phase 0.

**Validation**: launch `cargo run -p p2p-video-pipe-desktop`; on a second machine (or second instance) publishing test frames, see them in the egui window with sub-second latency on LAN.

**Wiki grounding**: [[iroh-sync-stack]] (iroh transport + relay fallback), [[drone-vision-software]] (live stream → display pattern).

### Phase 2 — Android app publishes camera

**Goal**: Android app on a real phone publishes its rear camera as a MoQ track that Desktop subscribes to.

**Tasks**:
- [ ] Create `android/` Gradle project with native Rust core via cargo-ndk + UniFFI (or JNI directly if simpler for one screen).
- [ ] Single-screen UI: "Scan ticket" button → CameraX QR scanner → ticket bytes.
- [ ] Wire `moq-media-android` (workspace dep) to CameraX → H.264 hardware encoder → MoQ track publisher.
- [ ] Add `iroh-android` foreground service so streaming survives screen-off (drone flight = locked phone).
- [ ] Permissions: CAMERA, INTERNET, FOREGROUND_SERVICE_CAMERA, ACCESS_FINE_LOCATION (for geotag in metadata).
- [ ] APK output via `./gradlew assembleDebug`; install on test phone via `adb install`.

**Dependencies**: Phase 1 (so there's a desktop subscriber to validate against).

**Validation**: paste/scan a desktop-generated ticket on phone; phone publishes; desktop sees the live camera feed.

**Wiki grounding**: [[android-on-drone]] (phone-as-camera role), [[mobile-desktop-architecture]] (Android-native shell choice).

**Risks**:
- iroh-android packaging: TLS/AWS-LC cross-compile per `iroh = { features = ["tls-aws-lc-rs"] }` in `Cargo.toml` — this is the exact risk [[mobile-desktop-architecture]] flagged for Tauri-mobile and applies here too. Budget extra day for resolution.
- Foreground-service-camera permission requires Android 14+ specific manifest declarations.

### Phase 3 — Desktop CV inference

**Goal**: bounding boxes for cow/horse/sheep classes appear on the live feed.

**Tasks**:
- [ ] Add `ort` crate for ONNX Runtime; bundle a YOLOv5n COCO ONNX model.
- [ ] Frame fan-out: subscribe path produces decoded RGBA → channel → (a) egui display, (b) inference task.
- [ ] Inference task: resize to 640×640 → ONNX inference → NMS → bounding boxes.
- [ ] Render boxes + labels + confidence on egui frame.
- [ ] Cap inference rate (e.g., 5–10 FPS) to keep CPU budget reasonable; display rate is full 30 FPS.
- [ ] Detection ring buffer (last 1000 detections in memory) for sidebar listing.

**Dependencies**: Phase 1 (frames flowing).

**Validation**: point phone at livestock photos / video on a screen → boxes appear correctly. Latency: end-to-end (camera-to-detection-overlay) under 1 second on LAN.

**Wiki grounding**: [[drone-vision-software]] (YOLO model selection, COCO classes 15/17/18 for cattle/horse/sheep), [[oss-drone-fms-pipeline]] § L6 (live inference).

### Phase 4 — Field test (drone-agnostic)

**Goal**: prove the pipeline works on real aerial-class video, regardless of what's holding the phone.

**Drone-agnostic principle**: the phone is the camera. Whatever holds the phone — a DJI airframe with the phone strapped to it, an ArduPilot quad, a vehicle dash mount, a chest harness, a hiking pole, or a hand — is irrelevant to the MVP. The drone is *not* part of the software architecture. Programming the drone from desktop/mobile is a future phase, not Phase 4.

This means: **no drone purchase, no drone-specific integration, no MAVLink, no airframe assumptions** for the MVP. If you've got a DJI Mavic, strap the phone to it, fly the DJI on its own controller, and use the *phone's* camera (not the DJI's). If you've got nothing, walk a pasture with the phone in your hand.

**Tasks**:
- [ ] Mount the phone to *whatever's available*. If a drone, foam/gel vibration dampening; phone facing down. If a vehicle, dashboard or window mount.
- [ ] Disable EIS/OIS in CameraX config to avoid jelly artifacts compounding with vibration (motion-stabilized cameras misbehave on a moving platform).
- [ ] Run the phone app from Phase 2; pair to desktop (Phase 5 if available, or ticket-paste fallback).
- [ ] Capture: a moving outdoor session with livestock or livestock-like targets in frame.
- [ ] Iterate on: frame drops, reconnection behavior over the platform's connectivity (drone WiFi, hotspot, 4G — whatever's available), thermal behavior under sustained streaming, GPS metadata correctness.
- [ ] Archive captured footage for HerdNet fine-tuning later.

**Dependencies**: Phase 3 (to demonstrate counting on the live feed).

**Validation** (reliability-focused per user direction):
- Stream survives a 5-minute session without an unrecoverable disconnect
- Bounding boxes correctly identify livestock (real or on-screen test footage)
- Reconnect after a deliberate WiFi-off / WiFi-on cycle works without user intervention
- No phone overheating shutdown
- Pipeline works the same on three different "platforms" (e.g., handheld + vehicle + whatever drone) — the drone is interchangeable

**Explicitly out of scope for Phase 4**:
- Programming the drone from the app (no MAVLink, no waypoint upload, no RTL trigger)
- Reading drone telemetry (no MAVLink subscribe either)
- Drone-specific UI (no DJI SDK, no PX4 integration)
- Replacing the user's existing GCS (QGroundControl, DJI Fly, etc. — they fly the drone, we just stream from a phone bolted to it)

**Wiki grounding**: [[android-on-drone]] (vibration/thermal mitigations — apply whenever the phone moves regardless of platform). [[implementation-plan]]'s $530 Holybro build is a *future aspirational* path, not a Phase 4 prerequisite.

### Phase 5 — Pairing + multi-session UX

**Goal**: a usable pair-and-fly flow without manual ticket-pasting.

**Tasks**:
- [ ] Desktop renders iroh ticket as QR code on app launch; auto-rotates per session.
- [ ] Phone "Pair with desktop" button opens camera in QR-scan mode.
- [ ] On scan, phone connects + starts publishing automatically.
- [ ] Allow the desktop to mint multiple tickets (one read-only for an apprentice machine, one read+write for the phone).
- [ ] Status UI on both sides: connected peers, current track stats (bitrate, latency, frame count).

**Dependencies**: Phase 2 + Phase 1.

**Validation**: a third-party can pair phone to desktop in under 60 seconds without reading docs.

**Wiki grounding**: [[iroh-docs-fms-schema]] § "Namespace key distribution is your problem" — iroh-tickets handles invite codes; QR-on-desktop-scanned-by-phone is the documented pattern.

### Post-MVP — Records layer

**Goal**: extend iroh-smol-kv to carry Asset / Log / Quantity entries, write Observation logs from desktop detections, project to SQLite for queries.

This is "Phase 1 MVP" of the higher-level [[herd-scout-positioning]] document and out of scope for this RFC except as a forward-compatibility target. Key constraint: **the iroh node and namespace from MVP keep running**; records ride the same data plane.

## Open questions (resolved 2026-05-20)

User direction received during plan review:

1. ~~Where is `vendor/iroh-live/`?~~ → **Add as git submodule.** Phase 0 task list updated.
2. ~~Is `Frando/moq` sufficient as fallback?~~ → **Yes, retain as documented fallback** if submodule API drifts.
3. ~~Latency target?~~ → **Reliability over latency.** "Doesn't have to be low — we just need reliability." Performance section + risks updated; Phase 4 validation switched from latency-based to reliability-based criteria.
4. ~~Hardware on hand for Phase 4?~~ → **Phone + a drone, no Holybro budget.** Phase 4 reworked into three branches (DJI/closed → hand-held fallback; ArduPilot strappable → drone mount; vehicle/handheld → drive-by). [[implementation-plan]]'s $530 build moved to *future aspirational* status.
5. ~~LAN vs WAN colocation?~~ → **"With iroh, it shouldn't matter."** Correct — iroh's transport (peer-direct + hole-punch + relay fallback) makes this transparent. Cross-cutting performance section updated to remove LAN/WAN distinction.

## Remaining open questions

- ~~Which drone, specifically?~~ → **Drone-agnostic; doesn't matter.** Phase 4 strapped to whatever's available; programming the drone is a future phase.
- **Submodule upstream URL** — the exact URL for the iroh-live source tree. Probably an n0-computer experimental repo or a personal fork; needs confirmation before `git submodule add`.
- **CV model storage**: bundle YOLOv5n weights with the desktop binary (~7 MB) or fetch on first run? Bundling simplifies offline-first; fetch trims binary size.
- **Reconnection semantics**: when the phone briefly loses connectivity, should the desktop continue showing the last frame with a "reconnecting" overlay, or go to a black/disconnected screen? UX call.

## Sources consulted

| Article | What it contributed |
|---|---|
| [[mobile-desktop-architecture]] | UI stack picks; Tauri 2 alpha caveat; sync-engine tradeoffs |
| [[iroh-sync-stack]] | iroh transport + smol-kv + blobs + gossip roles; LAN-direct + relay-fallback model |
| [[iroh-docs-fms-schema]] | Namespace + author key strategy; pairing via iroh-tickets / QR; (forward) records schema |
| [[android-on-drone]] | Phone-as-camera-not-FC verdict; thermal/vibration mitigations; 4G latency numbers |
| [[drone-hardware]] | Autopilot + companion + phone-as-camera options |
| [[drone-vision-software]] | YOLO model selection; COCO livestock classes; live-inference pattern |
| [[implementation-plan]] | $530 phone-on-drone build plan for Phase 4 |
| [[herd-scout-positioning]] | Strategic wedges; phased build plan |
| [[fms-data-model]] | Forward-compatibility constraint for Phase 3+ records |
| [[oss-drone-fms-pipeline]] | Why live NDVI is impossible (radiometric calibration); inference-on-RTSP precedent |
| [[livestock-eid-rfid]] | Phone's other future role (BLE EID reader bridge) — informs Phase 6+ scope |
| `Cargo.toml` (direct) | Workspace deps; `iroh-live`, `moq-media-android`, `iroh-tickets`, `iroh-blobs`, `iroh-smol-kv` |
| `desktop/src/main.rs` (direct) | Confirmed Hello-world stub state — drives Phase 0 pre-work |
