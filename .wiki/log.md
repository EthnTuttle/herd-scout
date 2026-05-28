# Herd Scout Project Wiki

## [2026-05-19] init | Created local wiki for herd_scout project

## [2026-05-19] compile | Imported from external drone-vision wiki

Imported articles:
- [[drone-vision-software]] - Computer vision software research
- [[drone-hardware]] - Open source drone hardware options
- [[implementation-plan]] - Phone-based herd counter build plan

## [2026-05-20] research | "OSS mobile-to-desktop farm management + drone integrations" → 17 sources, 14 articles, 2 rounds

Round 1 — 8 parallel deep-mode agents covered: OSS FMS landscape, FMS feature taxonomy, mobile↔desktop architecture, livestock OSS, drone-FMS pipeline, Android-on-drone, ag data standards, precision-ag drone use cases. 14 sources ingested, 12 wiki articles compiled.

Round 2 — 3 focused agents on top gaps from Round 1: EID reader BLE/SPP wire protocols, iroh-smol-kv schema for FMS data, WebODM→FMS ingestion bridge. 3 sources ingested, 2 wiki articles compiled, 2 enriched. Note: agents lacked WebFetch access — content is high-confidence prior-knowledge synthesis but flagged for primary-source verification.

Output: [[output/playbook-herd-scout-2026-05-20]] — strategic playbook with phased build plan.

## [2026-05-20] plan | "mobile-to-desktop iroh app: desktop driver + phone-on-drone camera" → output/plan-mobile-to-desktop-iroh-rfc-2026-05-20.md (13 articles consulted, 7 architecture decisions, 6 phases)

Generated RFC-format implementation plan grounded in the wiki research. MVP framing: phone is a streaming camera only (no on-device ML, no MAVLink, Android only), desktop runs all inference. Critical pre-work surfaced: `vendor/iroh-live/` is referenced in `Cargo.toml` but not present on disk; `desktop/src/main.rs` is a Hello-world stub. Phase 0 of the plan resolves this before any feature work.

## [2026-05-22] plan | "deploying the daemon on the 1060 laptop setup" → output/plan-deploy-daemon-on-1060-laptop-2026-05-22.md (12 articles consulted: 5 project-local + 7 from gtx-1060-headless-ai-server hub wiki, 6 architecture decisions, 6 phases + 1 optional)

Roadmap for deploying herd-scout-daemon as a headless systemd service on an MSI GS63VR (GTX 1060 6GB mobile) running Ubuntu Server 22.04 LTS. Key decisions: reuse the hub wiki's GS63VR baseline verbatim (Pascal forces proprietary nvidia-driver-535-server + CUDA 12.x ceiling); wire ort 2.0.0-rc.12 CUDA EP behind a `gpu` Cargo feature with runtime CPU fallback; build natively on the box for now; iroh's relay path replaces MediaMTX/SRT (different stack from the hub wiki's existing GS63VR plan); GUI on a different machine connects via SSH UDS forward. End-to-end e2e: phone → iroh-relay → laptop daemon → SSH UDS → remote GUI.

Key findings:
- farmOS dominates OSS FMS; livestock-specific layer is dramatically thin
- L5 (FMS structured ingest of drone outputs) is broken across all OSS FMS — concrete bridge documented
- Strap-android-on-drone: yes as ML+4G companion, no as flight controller; DroneKit-Android is dead
- This repo declares `iroh-smol-kv` not `iroh-docs` — schema patterns updated accordingly
- Defensible wedges: Rust-native, P2P/offline-first, drone-vision livestock integration, native mobile, OSS EID reader bridge

## [2026-05-26] plan | "optimize the herd-scout CV sidecar (post-process bottleneck)" → output/plan-optimize-cv-sidecar-trt-yolo11s-2026-05-26.md (6 articles consulted: 1 project-local + 5 from gtx-1060-headless-ai-server hub wiki, 4 architecture decisions, 4 phases with Phase 3 gated)

Roadmap for collapsing the CV sidecar's 150-180 ms/frame numpy+cv2 NMS postprocess. Wiki recommended path: re-export YOLO11s with `nms=True` (Phase 1) so NMS runs in the ONNX graph and postprocess shrinks to a tensor decode. Add supervision (MIT) + ByteTrack for tracking + license clarity (Phase 2); extend wire protocol with `track_id`. Phase 3 — TRT 8.6.x sm_61 + EFFICIENT_NMS plugin — built only if Phase 1 misses the 10 FPS sustained gate. Daemon's 10 FPS cap stays in place; lifting it is its own plan.

Predecessor: [[output/plan-deploy-daemon-on-1060-laptop-2026-05-22]] (Phase 3 of which just shipped — sidecar live, GPU verified at 18 ms/frame inference, 128 MiB VRAM held by Python sidecar PID).

## [2026-05-26] plan | "iroh-bound SSH session to bind herd-scout-daemon (no DNS / NAT-traversed)" → output/plan-iroh-bound-ssh-access-daemon-2026-05-26.md (5 wiki articles + 2 design docs consulted, 2 web sources for gap research, 6 architecture decisions, 6 phases — Phase 6 optional)

Roadmap for an iroh-tunneled SSH path to the daemon. Locked decisions (after interview): use case = GUI reaching daemon UDS without DNS/Tailscale (replaces Decision 6 of the deploy plan); auth = NodeId allowlist on the daemon, sshd handles user auth as today; implementation = SSH ProxyCommand wrapping a small `herdctl proxy <node-id>` binary that pipes a QUIC bi-stream to stdin/stdout; discovery = static `~/.ssh/config` HostName = NodeId.

Architecture: daemon registers a third ALPN `herd-scout/ssh/1` on its existing `Live` router (one Endpoint, one NodeId), gates incoming dials on a `control.toml` allowlist (fail-closed on missing/empty, `kill -HUP` for hot reload), then `tokio::io::copy_bidirectional` between the QUIC stream and `127.0.0.1:22`. New workspace member `herdctl/` ships `proxy`, `ping`, `whoami` subcommands; future `forward` subcommand provides direct UDS-over-iroh in Phase 6.

Gap research findings: (a) n0-computer/dumbpipe is a working reference for QUIC-byte-pump CLIs but has no peer-authorization layer — confirmed why we build in-tree, (b) `iroh::protocol::Router::accept(alpn, handler)` + `ProtocolHandler::accept(connection)` is the documented extension point and `vendor/iroh-live/iroh-live/src/live.rs:168-170` already uses the same pattern for moq + gossip — we hang a third ALPN onto it. Open questions tracked: iroh-smol-kv 0.98 surface for `connection.remote_node_id()` / `RouterBuilder` reuse; addressed by Phase 1's dial-tester canary.

## [2026-05-27] ship | Wave 11 implementation: iroh-bound SSH access landed → herd-scout-ipc + herd-scout-daemon/src/control + herdctl + deploy/README.md

Built per [[output/plan-iroh-bound-ssh-access-daemon-2026-05-26]]. Three-wave swarm execution per `/Users/garykrause/.claude/plans/we-should-be-using-vivid-quokka.md`:

- **Wave A** (in-line, ~5 min): added `pub const CONTROL_ALPN: &[u8] = b"herd-scout/ssh/1"` to `herd-scout-ipc/src/lib.rs`. `ControlConfig` deliberately stayed inside the daemon (would have pulled iroh into the GUI's compile graph otherwise — `herd-scout-ipc` is shared with `herd-scout-gui`).
- **Wave B1** (parallel agent): daemon control plane. New `herd-scout-daemon/src/control.rs` + `control/{config.rs, handler.rs}`. Hand-built Router via `Live::register_protocols(...).accept(CONTROL_ALPN, ControlHandler).spawn()` instead of `with_router()`. ArcSwap + SIGHUP hot reload. AtomicUsize cap 16 with RAII guard. `cargo build --release -p herd-scout-daemon` clean.
- **Wave B2** (parallel agent): `herdctl` workspace member with `proxy` / `ping` / `whoami`. Persisted ed25519 identity at `$XDG_CONFIG_HOME/herdctl/secret.key` (mode 0600). No iroh-live, no moq, no ort — small fast-compiling client. `cargo tree -p herdctl --depth 1` clean.
- **Wave C** (in-line): `deploy/README.md` documents the full operator quickstart (control.toml schema, `herdctl whoami`/`ping`, `~/.ssh/config` snippet, threat model). Decision 6 of the deploy plan annotated with a forward-pointer.

iroh 0.98 API surprises caught at implementation time: `NodeId` → `EndpointId`, `NodeAddr` → `EndpointAddr`, `Connection::remote_id()` is infallible at handler time (not `remote_node_id() -> Result<...>`), `ProtocolHandler` uses native AFIT (no `#[async_trait]`), `SecretKey::generate()` is no-arg. Wiki concept `[[iroh-sync-stack]]` already flagged the smol-kv API drift; same applies to the bare iroh crate at 0.98.

Iroh 1.0-rc bump deferred per user decision: rc.1 shipped 2026-05-27 (today), three satellite git-branch deps (`n0-computer/iroh-smol-kv@iroh-098`, `Frando/moq@deps/iroh-098`, `moq-dev/web-transport@deps/iroh-098`) have not migrated yet. Tracked as a watch-item; ships as its own PR stream once Frando lands `deps/iroh-100` upstream.
## [2026-05-27] plan | "Android app for managing permitted nodeIDs that can connect to a daemon" → output/plan-android-admin-allowlist-app-2026-05-27.md (5 articles + 2 design docs consulted, 7 architecture decisions, 7 phases)

Roadmap for a separate Android APK (`com.herdscout.admin`) that manages the daemon's SSH-allowlist NodeIds over a fourth iroh ALPN `herd-scout/admin/1`. Adds a separate `[control_plane.admins]` set in `control.toml` (orthogonal to the SSH-access allowlist Wave 11 introduced) so privilege escalation is blocked. Daemon owns atomic temp-file-rename writes; in-process `ArcSwap` reload skips SIGHUP. Phone-side admin client lives in `herd-scout-jni` and reuses the existing tokio + iroh runtime; Kotlin Compose UI is a fresh Gradle module sharing a small library with the streaming app. Bootstraps via the existing Wave 11 SSH path (hand-edit + SIGHUP once). Builds directly on `plan-iroh-bound-ssh-access-daemon-2026-05-26`.

## [2026-05-27] ship | Wave 11 follow-up: persist daemon iroh secret across restarts → herd-scout-daemon/src/daemon_secret.rs

Surfaced during Wave 11 verification: bigdeal's daemon was minting a new ed25519 secret on every restart (no IROH_SECRET in /etc/herd-scout-daemon.env, iroh-live's `secret_key_from_env` falls back to `SecretKey::generate()` and only logs reuse instructions). That rotated the daemon's NodeId on every reboot, breaking every operator's `~/.ssh/config` HostName and every peer's `control.toml` allowlist. Fix: new `daemon_secret` module that resolves the data dir via `directories::ProjectDirs` (matches `Store::open` triple), reads or generates a 64-hex secret at `<data_dir>/iroh_secret` (mode 0600), and `unsafe { set_var("IROH_SECRET", hex) }` *before* `Live::from_env()` runs. Migration: if `IROH_SECRET` is already in the env (from systemd), it wins and is also persisted to disk so the env becomes optional next boot. No vendor patch — iroh-live still drives the secret resolution, we just guarantee the env var is populated. Verified end-to-end: two consecutive `systemctl restart` cycles loaded the same persisted secret, NodeId stayed pinned at `f7e32cbffb…`, herdctl ping `ok` on both, ssh ProxyCommand re-authenticated cleanly.

## [2026-05-27] plan-refine | "Android admin app for daemon NodeIDs" → output/plan-android-admin-allowlist-app-2026-05-27.md (refined: 7→8 phases, 7→11 architecture decisions)

Incorporated user refinements after initial plan: (a) audit log on both ends — daemon writes append-only JSONL with versioned records and 90d rotation, exposed via a paginated `TailAudit` RPC; phone keeps a complementary Room SQLite that records `rpc_attempt` *before* the call so partial failures are captured. (b) Fleet mode = at-most-one active iroh `Connection`; switching daemons tears down and reconnects (no multiplexing). Single-Endpoint, single-Connection JNI model; up to 10 saved daemons in `SharedPreferences`. (c) Versioned `identity.toml` envelope in a new shared crate `herd-scout-identity`, used by daemon + `herdctl` + phone (with one-time legacy `secret.key` migration). Schema-version + integrity-check (`node_id` field must derive from `secret_key`). Phone admin app exposes Export/Import via Android SAF for reinstall recovery.

## [2026-05-27] research | "accurate herd counting from CV detections" → 11 sources, 2 concept articles, 1 playbook

Question-mode research with 8 parallel deep agents on the question "how can we ensure we get an accurate count of the herd from the object detection results?". Decomposed into 8 sub-questions: failure modes, counting algorithms (LineZone/PolygonZone), tracking metrics (HOTA vs MOTA), density estimation alternatives, multi-pass aggregation, livestock-specific accuracy, validation without ground truth, confidence calibration. Compiled into a 5-layer pipeline (detection → tracking → counting → aggregation → validation) with prioritized P0–P5 actions. Key findings: (a) naive `len(set(tracker_id))` is biased UPWARD (fragmentation + FPs + re-entry all push high), not downward as commonly assumed; (b) target HOTA/AssA, not MOTA — MOTA under-charges fragmentation by ~N×; (c) supervision.ByteTrack defaults are tuned for MOT17 pedestrians, not stationary livestock — `track_activation_threshold=0.35`, `lost_track_buffer=60`, `minimum_consecutive_frames=3` recommended; (d) reconciling live CV counts against the existing ISO 11784/11785 EID register via Lincoln-Petersen is publish-worthy — no prior published work; (e) FIDTM (MIT license, MIT pretrained weights, points-not-blobs) is the cleanest density-fallback path for dense regimes; (f) realistic counting MAE target is ±5–10% on pasture-sized herds at 30–60 m AGL drone altitude.

## [2026-05-28] plan | "desktop uploads video to daemon for processing" → output/plan-desktop-video-upload-2026-05-28.md (10 articles consulted, 7 decisions, 7 phases)

Roadmap for a batch upload pipeline as a sibling to the live phone-to-daemon path. Bytes ride iroh-blobs over a new `herd-scout/upload/1` ALPN registered on the existing iroh router (admin-allowlist-gated, audit-logged, mirrors the Wave 11/12 ALPN pattern). The sidecar's wire protocol gains a `request_kind` prefix; `0x01` is a file-decode mode that opens an MP4 via `cv2.VideoCapture` and emits the same per-frame detection responses the live path produces. Two outputs per clip: live overlay replay (`ServerMsg::Frame`/`Detections` with a new `clip_id` field, replayed to any connected GUI) and a persistent `report.json` applying the accurate-counting playbook (median-of-active-IDs + bootstrap CI + per-class summary + per-track stats; written atomically into `<data_dir>/uploads/<blake3>/`). Single-clip queue behind any active live phone session (sidecar is single-client; live wins). 10 min / 2 GB cap, MP4/H.264 first cut. UX: GUI drag-drop AND `herdctl push <file>`. Builds cleanly on plan-android-admin-allowlist-app (allowlist + audit infra) and playbook-accurate-herd-counting (count algorithm + ByteTrack params).
