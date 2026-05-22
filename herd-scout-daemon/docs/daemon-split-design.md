# Wave 6 — Daemon Split Design

## Why

Two intertwined problems force this split.

**Pairing is broken in Wave 5C.** `desktop/src/stream.rs:200-214,269-311` auto-mints a `LiveTicket` containing the desktop's *own* `EndpointAddr`, then `run_subscription` (line 370-373) calls `live.subscribe(ticket.endpoint, ...)` — i.e. dials the desktop's own endpoint. iroh refuses this with "Connecting to ourself is not supported" and the reconnect loop (line 248-249) floods stderr.

**The deeper bug is direction-of-flow vs. iroh-live's `Live::subscribe` signature.** `vendor/iroh-live/iroh-live/src/live.rs:228-241` requires the subscriber to know the publisher's `EndpointAddr` up front. UX wants the phone to scan the *desktop's* QR — at QR-mint time the desktop has no idea what `EndpointAddr` the phone will bind. We must drop down to `iroh-moq`'s session-level API, where either side can dial and the publisher's broadcasts auto-flow over the resulting session via `Moq::publish`'s "fan out to all sessions" actor logic (`vendor/iroh-live/iroh-moq/src/lib.rs:482-526`).

Splitting into a daemon also unlocks: persistent iroh node across GUI restarts, headless operation on a Pi/NUC, multi-GUI viewports, and a clean integration point for future recording / WebODM ingestion.

## Locked decisions

- Two new crates `herd-scout-daemon` and `herd-scout-gui` at workspace root, siblings of `android-jni` and `vendor/iroh-live/*`
- `desktop/` is **deleted**, not renamed (its package is `p2p-video-pipe-desktop`; we want a clean break)
- Phone scans desktop's QR. Daemon owns mint and ticket persistence
- Pairing drops below `iroh-live::Live::subscribe`. Daemon dials *the publisher when discovered*, not the QR-encoded addr
- CV stays daemon-side. GUI is a viewport
- JNI on phone: ~5 line change to dial the desktop's endpoint and retain the session, so `Live::publish` fans out to it
- Wave 5B `Store` migrates to daemon as-is. Wave 3 CV migrates to daemon as-is
- macOS Swift rpath shim moves to `herd-scout-daemon/build.rs` only. GUI links neither iroh nor screencapturekit
- MVP is single-client: one GUI per daemon

## Crate structure

```
/Users/garykrause/repos/herd-scout/
├── Cargo.toml            (workspace; add daemon + gui members; drop desktop)
├── herd-scout-daemon/    NEW — bin crate "herd-scout-daemon"
│   ├── Cargo.toml        iroh-live, iroh-moq, ort, ndarray, image,
│   │                     parking_lot, anyhow, iroh-smol-kv, directories,
│   │                     serde, serde_json, rand, tokio, tracing, tracing-subscriber,
│   │                     interprocess
│   ├── build.rs          (Swift rpath shim — moved from desktop/build.rs)
│   ├── assets/yolov5n.onnx (moved from desktop/assets/)
│   └── src/
│       ├── main.rs       (entry, IPC server bind, lifecycle)
│       ├── stream.rs     (rewritten — incoming_sessions + per-session subscribe)
│       ├── store/        (moved verbatim from desktop/src/store/)
│       ├── cv/           (moved verbatim from desktop/src/cv/)
│       ├── ipc/          NEW — IPC server, framing, message types
│       │   ├── mod.rs
│       │   ├── proto.rs  (message enum, serde derive)
│       │   ├── server.rs (UDS listener + per-conn task)
│       │   └── frame.rs  (length-prefixed framing)
│       ├── pairing.rs    (mint helper — keeps generate_broadcast_name)
│       └── preview.rs    (RGBA → JPEG re-encoder for IPC frames)
├── herd-scout-gui/       NEW — bin crate "herd-scout-gui"
│   ├── Cargo.toml        egui, eframe, qrcode, tokio, tracing, tracing-subscriber,
│   │                     serde, serde_json, parking_lot, interprocess, image
│   │                     — NO iroh, NO ort, NO moq-media-egui
│   └── src/
│       ├── main.rs       (entry; auto-spawn daemon if absent)
│       ├── ui.rs         (App; ported from desktop/src/ui.rs)
│       ├── ipc/
│       │   ├── mod.rs
│       │   ├── client.rs (UDS dial + reconnect)
│       │   └── proto.rs  (shared types — copy of daemon's, or via shared crate)
│       ├── pairing.rs    (QR rendering — render_qr_image moved here)
│       ├── frame_view.rs (JPEG bytes → egui texture; replaces moq-media-egui)
│       └── overlay.rs    (CV box drawing — pure egui, no model types)
└── android-jni/          unchanged except 5-line connect_impl change
```

Optionally factor IPC types into a third tiny crate `herd-scout-ipc` (only `serde`); recommended.

## IPC protocol

**Transport: Unix domain socket on Unix, named pipe on Windows.** Use [`interprocess`](https://crates.io/crates/interprocess) (cross-platform UDS / named-pipe with tokio adapters). Rationale:

- TCP requires a port + firewall prompts on macOS; UDS is a filesystem path
- iroh-as-IPC is overkill (don't spin up a second iroh node just for localhost)
- gRPC is heavy for single-client RPC

Socket path (per `directories::ProjectDirs` triple Wave 5B uses):
- macOS: `~/Library/Application Support/net.herd-scout.herd-scout/daemon.sock`
- Linux: `$XDG_RUNTIME_DIR/herd-scout/daemon.sock` (fallback to data dir)
- Windows: `\\.\pipe\herd-scout-daemon`

**Framing: 4-byte big-endian length prefix + payload.** `serde_json` for control messages (small, debuggable). JSON over UDS at 30 control msgs/sec is fine.

**Frame transport: JPEG-encoded preview frames.** Daemon receives via moq, decodes (CV needs RGBA), runs YOLO on full-res, then **JPEG re-encodes a 720p downscaled preview at quality 80** (~50–200 KB at 720p, ~6 MB/s on the wire). `image` crate's `JpegEncoder`. CV runs on full-res before the JPEG step; the GUI never sees full-res. Future upgrade: shared-memory ring buffer.

**Message enum sketch:**

```
// daemon → gui
ServerMsg::Hello { daemon_version, capabilities }
ServerMsg::Pairing { ticket: String }
ServerMsg::Status { state: ConnectionStatus, last_frame_age_ms: Option<u64> }
ServerMsg::Frame { width: u16, height: u16, pts_ms: u64, jpeg: Vec<u8> }
ServerMsg::Detections { frame_pts_ms: u64, dets: Vec<DetWire>, counts: ClassCountsWire }
ServerMsg::CvBanner { text: Option<String>, disabled: bool }

// gui → daemon
ClientMsg::Hello { gui_version }
ClientMsg::RequestPairing       // ask daemon to (re-)mint
ClientMsg::ConnectTicket { ticket: String }
ClientMsg::ClearSavedTicket
```

`ConnectionStatus` and `ClassCountsWire` mirror `desktop/src/stream.rs:39-65` and `desktop/src/cv/state.rs:51-56`. `DetWire { class: u8, bbox: [f32;4], score: f32 }`.

## Pairing flow (concrete)

Key fact from `vendor/iroh-live/iroh-moq/src/lib.rs`: **`Moq::publish` registers a broadcast that gets pushed to every existing session and every future session** (lines 482-526, `handle_session` and `handle_publish_broadcast`). And `MoqSession::subscribe(name)` *waits* for the remote to announce that name (line 324-344). So we don't need separate discovery — moq already announces broadcasts over existing sessions.

We just need **a session between phone and desktop**.

**Step by step:**

1. **Daemon boot.** `Live::from_env().await?.with_router().with_gossip().spawn()`. Hold the `Live` for daemon lifetime.
2. **Mint rendezvous.** `let broadcast_name = generate_broadcast_name();` then `let ticket = LiveTicket::new(live.endpoint().addr(), &broadcast_name);`. Persist via `Store::save_ticket`. Send to GUI as `ServerMsg::Pairing { ticket: ticket.to_string() }`.
3. **GUI renders QR** of the ticket string (existing `pairing::render_qr_image`).
4. **Phone scans QR**, calls `nativeConnectWithTicket` (`android-jni/src/lib.rs:175`). **JNI change**: in `connect_impl`, after `Live::from_env(...).spawn()`, call `let _session = live.transport().connect(ticket.endpoint).await?;` and stash it in `SessionHandle` so it stays alive. (~5 lines.)
5. **Phone calls `nativeStartStreaming`.** Today's `live.publish(broadcast_name, broadcast)` works unchanged — the actor (`iroh-moq/src/lib.rs:512-517`) iterates `self.sessions` and fans the broadcast out, which now includes the session to the daemon.
6. **Daemon's accept loop.** New: spawn a task that listens on `live.transport().incoming_sessions().next().await` (or whatever the moq actor exposes — `Moq::incoming_sessions` per iroh-moq/src/lib.rs:149-153). For each session, spawn:

   ```text
   match tokio::time::timeout(15s, session.subscribe(&broadcast_name)).await:
       Ok(Ok(consumer)) => promote to RemoteBroadcast + media decode (existing
                           run_subscription body)
       _ => log and drop session
   ```

7. **Decode + CV + IPC fan-out.** Once `session.subscribe(name)` returns a `BroadcastConsumer`, wrap in `RemoteBroadcast` (same as `iroh-live/src/live.rs:239`), call `.media_with_decoders::<DefaultDecoders>(...)` to get `MediaTracks`. Loop on `video.next_frame().await` exactly as `stream.rs:404-427` does today. For each `VideoFrame`:
   - Push to a `watch::Sender<Option<Arc<VideoFrame>>>` so the CV task picks it up unchanged
   - JPEG-encode 720p preview + send `ServerMsg::Frame` to GUI

8. **CV results to GUI.** CV task gains an `mpsc::Sender<ServerMsg>` arg; on each snapshot update it also sends a `ServerMsg::Detections`.

## Daemon lifecycle

**Hybrid.** GUI on launch:

1. Try connect to existing daemon socket. Success → use it.
2. `ConnectionRefused` / `NotFound` → fork-spawn `herd-scout-daemon` as a child process (`tokio::process::Command`, stdout/stderr → `<data_dir>/daemon.log`). Poll for socket up to 5s. Still absent → fatal-error screen.
3. GUI exits → daemon survives. User stops via `pkill` or "Quit daemon" menu (`ClientMsg::Shutdown`).

Headless: `./herd-scout-daemon` directly. Pairs via QR — daemon writes ticket string to stdout on first launch + on re-mint so operators can scan from logs.

Future: `launchd`/`systemd`. Out of scope for Wave 6.

## What moves where (file mapping)

| Current `desktop/` path | New location | Notes |
|---|---|---|
| `Cargo.toml` | DELETED (replaced by two new) | `desktop` removed from workspace members |
| `build.rs` | `herd-scout-daemon/build.rs` | verbatim |
| `assets/yolov5n.onnx` | `herd-scout-daemon/assets/yolov5n.onnx` | verbatim |
| `src/main.rs` | split: ticket-load → `daemon/src/main.rs`; eframe boot → `gui/src/main.rs` | rewritten |
| `src/stream.rs` | `herd-scout-daemon/src/stream.rs` | rewritten — see Pairing flow |
| `src/cv/` | `herd-scout-daemon/src/cv/` | verbatim, plus `mpsc::Sender<ServerMsg>` side-channel |
| `src/store/` | `herd-scout-daemon/src/store/` | verbatim |
| `src/pairing/mod.rs` | split: parse helpers → `gui/src/pairing.rs`; QR render → `gui/src/pairing.rs`; `generate_broadcast_name` → `daemon/src/pairing.rs` | |
| `src/ui.rs` | `herd-scout-gui/src/ui.rs` | rewritten — replace StreamHandle calls with IPC client; replace FrameView with JPEG-decode-to-egui |
| (new) | `herd-scout-daemon/src/ipc/` | |
| (new) | `herd-scout-gui/src/ipc/` | |

## What gets thrown away from Wave 5C

- The auto-mint-then-self-subscribe loop (`stream.rs:200-214` + `stream.rs:269-311`). Mint logic survives on daemon, but self-subscribe gets replaced by `incoming_sessions().next().await` + per-session `session.subscribe(name)`.
- `ConnectionStatus::AwaitingTicket` UI state: daemon mints synchronously on boot before accepting GUI connections; GUI never observes "no ticket yet."
- `StreamHandle::current_ticket` watch channel. Replaced by `ServerMsg::Pairing` push.
- `stream::spawn(Some(ticket), Some(store), ctx)` from the paste box. Replaced by `ClientMsg::ConnectTicket`.

## Risks

1. **`MoqSession::subscribe` waits forever for an announcement** (line 332-344). Wrap in `tokio::time::timeout(15s, ...)`, drop session on timeout.
2. **Phone's `live.transport().connect(ticket.endpoint)` may fail without prior gossip rendezvous.** It should work — both sides have `with_gossip()` and `with_router()`. If it fails, fall back to gossip-based addr exchange via `iroh_smol_kv::Client::local(topic, config)`.
3. **JPEG re-encode at 30 FPS could pin a CPU core on Pi5.** Cap preview rate at 15 FPS or downscale to 540p.
4. **`interprocess` on Windows**: verify in smoke test before committing. Fallback: `tokio::net::UnixStream` directly on Unix and skip Windows for MVP.
5. **Daemon and GUI version drift.** `ServerMsg::Hello { daemon_version }` lets GUI refuse incompatible daemons. Bake `env!("CARGO_PKG_VERSION")`.
6. **Process supervision absent.** Daemon segfault → GUI sees closed socket, shows reconnect overlay. MVP: log + reconnect-attempt banner; manual restart.
7. **CV refactor for IPC fan-out.** Keep `SharedSnapshot` (future "headless dump to disk") *and* add `mpsc::Sender<ServerMsg>`. Two-writer, trivially safe.

## Wave 6 implementation tasks (atomic, ordered)

1. Workspace plumbing: add `herd-scout-daemon` + `herd-scout-gui` to root `Cargo.toml` `[workspace.members]`. Remove `"desktop"`. Delete `desktop/`.
2. Bootstrap `herd-scout-daemon/Cargo.toml`: carry deps from `desktop/Cargo.toml` minus egui/eframe/qrcode/moq-media-egui. Add `interprocess`. `[[bin]] name = "herd-scout-daemon"`.
3. Bootstrap `herd-scout-gui/Cargo.toml`: egui, eframe, qrcode (no defaults), tokio, tracing, tracing-subscriber, serde, serde_json, parking_lot, interprocess, image. **No iroh, no ort, no moq-media-egui.**
4. Move CV verbatim. Update `include_bytes!` path in `cv/model.rs` if needed.
5. Move Store verbatim.
6. Move Swift rpath shim verbatim.
7. Define IPC proto (`ipc/proto.rs`): `ServerMsg` / `ClientMsg` enums (serde, `#[serde(tag = "type")]`). Mirror `ConnectionStatus`, `DetWire`, `ClassCountsWire`. Optionally extract to `herd-scout-ipc` crate.
8. Implement IPC framing (`ipc/frame.rs`): 4-byte BE length + JSON body. Tokio codec adapter. <80 LOC.
9. Implement IPC server (`ipc/server.rs`): bind UDS at data-dir path (delete-then-bind to recover from stale), accept loop, per-conn task with two halves.
10. Rewrite `daemon/src/stream.rs`: spawn Live once at boot. Mint LiveTicket once (Store-load first, mint fresh only if absent). Push via `ServerMsg::Pairing`. Spawn `incoming_sessions` accept loop. For each session, `tokio::time::timeout(15s, session.subscribe(broadcast_name))`. On success run existing decode loop body. On failure/timeout drop and keep accepting.
11. Wire CV: same `watch::Sender<Option<Arc<VideoFrame>>>` fan-out. `cv::spawn_cv_task` gains `mpsc::Sender<ServerMsg>`; on snapshot update also sends `ServerMsg::Detections`.
12. JPEG preview encoder: `daemon/src/preview.rs`. Downscale to 720p, JPEG q80, return `Vec<u8>`. `tokio::task::spawn_blocking`. Cap 15 FPS.
13. Bootstrap GUI main: parse `--ticket` (legacy headless → `ClientMsg::ConnectTicket`), try-connect to daemon socket, on failure spawn daemon child process and poll. Tokio runtime, IPC client task, eframe.
14. Port `ui.rs` to IPC: `IpcClient` with watch channels for status / latest-frame-bytes / detections / pairing-ticket. egui paint reads watches synchronously. Replace `FrameView` with `gui/src/frame_view.rs` (JPEG → `image::load_from_memory` → `egui::TextureHandle`).
15. Port pairing screen: `current_ticket` from `ServerMsg::Pairing`. Paste-box "Connect" sends `ClientMsg::ConnectTicket`.
16. Port CV overlay: `draw_cv_overlay` body verbatim with wire-format types.
17. Port reconnect overlay: `draw_reconnect_overlay` verbatim.
18. Daemon auto-spawn: `Command::new(std::env::current_exe()?.with_file_name("herd-scout-daemon"))`. Redirect stderr to `<data_dir>/daemon.log`.
19. Smoke test: `cargo test --workspace`. CV / store / pairing tests come along verbatim. Add IPC roundtrip test (in-process) gated `cfg(unix)`.
20. Manual e2e: daemon up, GUI up, phone scans QR, frames flow, CV boxes paint. Document launch order in workspace README.

## Critical Files for Implementation

- `/Users/garykrause/repos/herd-scout/desktop/src/stream.rs` (rewritten subscribe-via-incoming-session replaces this entire file)
- `/Users/garykrause/repos/herd-scout/vendor/iroh-live/iroh-moq/src/lib.rs` (`Moq::incoming_sessions`, `MoqSession::subscribe`, actor publish/session fan-out)
- `/Users/garykrause/repos/herd-scout/android-jni/src/lib.rs` (5-line add in `connect_impl` to call `live.transport().connect(ticket.endpoint).await?` and retain the session)
- `/Users/garykrause/repos/herd-scout/desktop/src/ui.rs` (GUI port template — patterns survive, only data sources change)
- `/Users/garykrause/repos/herd-scout/Cargo.toml` (workspace members swap)
