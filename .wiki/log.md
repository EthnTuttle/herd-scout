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

## [2026-06-01] research | "Robust herd counting from MOT outputs + phone-on-drone airframes" --local --deep → 20 sources, 9 concept + 1 reference + 1 playbook

Round 4 — 8 parallel deep agents split across two halves: (A) MOT counting upgrades (BoT-SORT/OC-SORT vs ByteTrack, ID-switch correction, bootstrap CIs, foundation-model alternatives) and (B) phone-on-drone airframe (mounts/vibration, power, thermal, foreground-service constraints).

Key findings:

A. Counting upgrades:
- (a) Ultralytics now ships **BoT-SORT as default**, but its Global Motion Compensation is wasted on fixed cameras — only ReID + improved Kalman state matter; switch decision is scenario-specific (see [[concepts/tracker-choice-bot-oc-byte]]).
- (b) **OC-SORT (Cao 2022)** is the strongest fit for cattle bunching at gates — observation-centric re-update handles non-linear motion, ~700 FPS association on CPU, near-zero compute overhead vs ByteTrack. Cheapest high-value A/B test via [[references/boxmot-multi-tracker-zoo|BoxMOT]].
- (c) **YOLO26 retrain caveat**: NMS-free end-to-end head breaks ByteTrack's BT-Low score-based association pass. Keep `end2end=False` (legacy one-to-many head) until empirically validated. See [[concepts/yolo26-and-tracker-compat]].
- (d) Frame-iid bootstrap in current `report.rs` is statistically wrong for autocorrelated MOT output. Replace with **stationary block bootstrap** (Politis-Romano 1994; mean block length ≈ 10 frames at 30 FPS) + **BCa**. Add **J+aB** (Kim/Xu/Barber NeurIPS 2020) for a free conformal predictive interval on the same ensemble. See [[concepts/bootstrap-conformal-count-ci]].
- (e) **MultiCamCows2024** (Yu et al., 101k images / 90 Holsteins / 7 days) gives the concrete recipe to replace the speculative ResNet50 ReID stub: self-supervised tracklet-contrastive coat-pattern embedding, no per-cow labels needed, ~96% accuracy. See [[concepts/cattle-reid-self-supervised]].
- (f) **BUSCA** (Vaquero ECCV 2024) and **HIT** (2024) are the modern primitives for online vs offline tracklet recovery. See [[concepts/track-recovery-busca-hit]].
- (g) **SAM 2** infeasible on 6 GB Pascal alongside YOLO11s/26 — offline only. **Open-vocab detectors (Grounding DINO)** zero-shot ceilings at 76.8% mAP on cattle (well below fine-tuned YOLO) — server-side audit/labeling tool only. **CoTracker3** unproven for livestock; T-LEAP+BLSTM is the validated lameness path.

B. Phone-on-drone airframe:
- (a) Concrete BOM: **95A or 98A TPU printed tray + 4× M3×8 mm 50A silicone grommets + suspended (elastic-hanging) topology** (not corner/sandwich). Sorbothane 30A backup at 15-20% compression. Phone is mass-favorable for off-the-shelf dampers (closer to design mass than typical FCs). Avoid Moon Gel (>100 °F failure) and drone-mounted smartphone gimbals (no community traction). See [[concepts/phone-on-drone-airframe]].
- (b) Frequency target: **100-300 Hz Z-axis dominant**. Balance props first (>300% improvement before damping). Replace damping material every 6-12 months; print TPU spares (50-100 hard-landing life).
- (c) Power: hardware encode (MediaCodec native, NOT OMX.google.* SW fallback) is load-bearing — 3-4× power swing. Pixel 6/7 internal battery covers a single 20-35 min flight. Multi-flight: drone-LiPo → buck → USB-C PD source IC (FUSB302) → **9 V / 3 A = 27 W**. Charge limit 80%; replace donor phones every 200-400 cycles. See [[concepts/phone-power-on-drone]].
- (d) Thermal: empirical Pixel 6 Pro 4K60 outdoor throttle at **3-4 min** contradicts wiki "mostly fine in flight" claim — but herd-scout's 720p30 has materially lower load; verify per-airframe. **Donor sweet spot: Pixel 6 Pro / 7 Pro** (vapor chamber, large chassis); avoid Pixel 9 Pro non-XL (smaller chassis = worse thermals despite newer chip). Wire `addThermalStatusListener` with 4-rung ladder (MODERATE→bitrate, SEVERE→FPS+resolution, CRITICAL→stop, EMERGENCY→LTE already gone). `setSustainedPerformanceMode(true)` real measured trade: 18% peak / 2× sustained. WiFi ground station > direct LTE under thermal pressure. See [[concepts/phone-thermal-management]].
- (e) Foreground service: **`camera|connectedDevice` only**, NOT `dataSync` (6-hour cap on Android 15+). `camera` cannot be created from background or `BOOT_COMPLETED`; runtime perms must be granted before `startForeground()`. Forward-compatible through Android 16. OEM kill behavior (Xiaomi/Samsung One UI/OPPO/OnePlus) bites regardless of spec — test on actual donor phone. See [[concepts/phone-publisher-android-fgs]].

Output: [[output/playbook-mot-airframe-2026-06-01]] — P0/P1/P2 counting upgrades + buildable phone-on-drone airframe spec, with 3 suggested theses for future `/wiki:research --mode thesis`.

Open gaps surfaced (durable follow-ups): (1) no academic UAV paper on smartphone payload mounting (publishable opportunity), (2) compute fit of BUSCA on Pascal needs benchmarking, (3) MultiCamCows2024 license (CC BY-NC-SA 4.0) is non-commercial — site-specific self-supervised training is the canonical path, (4) mobile encode H.264 vs HEVC vs AV1 thermal benchmarks gated behind paywalls.

## [2026-06-02] assess --retardmax | herd-scout repo (`/Users/garykrause/repos/herd-scout`) → 14 alignments, 10 research gaps, 19 build opportunities, 17 market gaps

Compared the local repo against the local `.wiki/` (25 concept articles + 1 reference + 9 outputs across 48 sources) and the broader market via 5 parallel web-research agents (competitors, best practices, emerging trends, adjacent fields, failures).

Headline findings:

- **The repo has built the harder half first.** 5 iroh ALPNs (MoQ, gossip, SSH bridge, admin RPC, blob upload), versioned identity envelope, append-only audit log + Sigstore Rekor mirror, Wave-14 AccessLimit predicate gate, Android phone publisher (CameraX + MediaCodec + Wave-14 FGS manifest), egui desktop, herdctl iroh-bound CLI, Python YOLO sidecar over UDS — Waves 1–14 all shipped.
- **The FMS half — the actual differentiator per the wiki — is not yet in code.** No Animal/Group/Land/Log records, no iroh-smol-kv KV CRDT wired (despite being the declared data plane), no EID reader bridge, no paddock map, no farmOS-compat JSON:API, no compliance reporting. Desktop is egui, not Tauri 2 (wiki/repo mismatch).
- **CV pipeline implements layers 1–2 of the wiki's 5-layer counting stack;** layers 3 (deployment-aware policy), 4 (Lincoln-Petersen / N-mixture), 5 (conformal + EID reconciliation) are designs only.
- **Drone integration is research-only** — buildable BOM and post-hoc playbooks live in `output/`, but no MAVLink or ODM ingest exists in `src/`.
- **Market gap is real and getting more crowded.** Closed incumbents (AgriWebb 17k producers, Halter satellite SKU 2026, Vence, 701x, Datamars, Allflex SenseHub, CattleEye, Optifarm) own ranch UX with monthly subscriptions; OSS competitors (farmOS, LiteFarm, Ekylibre [SaaS sunsetting 2026]) are stable but web-PWA-first with no livestock CV. Nobody — open or closed — currently ships `(Rust + P2P + drone-CV + EID-reconciliation + native-mobile)`.

Recommendations cluster as:
- **P0**: iroh-smol-kv FMS schema crate; Animal/Group/Land/Equipment record types + minimal CRUD; `herd-scout-eid` Bluetooth-reader Rust crate (weekend MVP per [[concepts/livestock-eid-rfid]]).
- **P1**: Counting layers 3+5 (deployment-policy + conformal + 🟢/🟡/🔴 chip); farmOS JSON:API consumer/producer; treatment log + withdrawal flag; documented accuracy commitments; paddock polygons + animal-last-seen pin.
- **P2**: YOLO26 + OccluBoost / RF-DETR-M; DINOv3 + WildFusion ReID; Lincoln-Petersen / N-mixture; WebODM/DroneDB ingest (L5); `mavlink` crate + 4G bridge; Tauri 2 frontend (iOS); compliance exports (USDA 840, NLIS, CCIA, EU passport); ICAR ADE / OADA `formats` / AgGateway TraceabilityAPI adapters; on-device LLM ("ask the herd").

Anti-patterns from the OSS livestock graveyard (OpenFarm, Tania, DroneKit-Android, BeefChain, Cainthus → Ever.Ag, PastureMap, Vence → Merck, HerdNet weights CC-BY-NC-SA poison pill, Tauri 2 mobile churn) captured as a section in the report.

Output: [[output/assess-herd-scout-2026-06-02]] — full gap analysis with research commands, build queue, competitive landscape, emerging trends, anti-patterns, confidence notes.


## [2026-06-02] plan | "FMS schema and other P0" → output/plan-fms-schema-and-records-2026-06-02.md (9 wiki articles + 2 inventory items + 1 assess output consulted, 7 architecture decisions, 7 phases)

Roadmap for the assess P0 record-layer work. User-locked scope: iroh-smol-kv schema + Animal/Group/Land/Equipment records + Observation/Medical/Movement/Weight/Birth log CRUD only — defers `herd-scout-eid` Bluetooth crate and farmOS JSON:API consumer/producer to separate plans. Frontend extends existing egui (no Tauri 2 pivot — wiki-flagged churn risk). iroh stays pinned at 0.98.0 / iroh-blobs 0.102.0 (blocked on watch [[inventory/watch/iroh-blobs-233-poisoned-store]]).

Key architecture decisions:
- Entity-attribute key layout, one scalar per key (per [[wiki/concepts/iroh-docs-fms-schema]])
- ULID + HLC mandatory; never wallclock LWW
- Per-field conflict strategy: LWW (name/notes), add-wins-set (tags, asset_refs), append-only (logs, quantities)
- Co-location-aware SQLite projection — GUI probes IPC socket at startup; reachable → IPC reader, no local SQLite; unreachable → own iroh peer + own SQLite. Configurable override via `HERD_SCOUT_GUI_MODE`.
- LiveTicket-style farm-namespace QR invites — reuses Wave-2 ML Kit scanner + daemon ticket-mint pipeline.

Phases: Phase 0 iroh-smol-kv API audit (1w) → Phase 1 `herd-scout-fms` crate (2w) → Phase 2 daemon integration + IPC RPCs (1w) → Phase 3 SQLite projection + GUI co-location (1w) → Phase 4 egui Records + Logs UX (2-3w) → Phase 5 QR farm-namespace onboarding (1w) → Phase 6 validation + README + audit-log integration (1w). ~9-11 weeks total.

Open questions surfaced: iroh-smol-kv crate publication state, Medical-withdrawal units (suggest `i32 days`), Term-taxonomy bootstrap (suggest tiny default set), Group-membership model (locked: `asset/<animal-id>/parent → <group-id>`, derive Group members), iroh 1.0 GA migration as separate wave, EID-to-FMS hand-off via IPC `CreateAsset` (no in-plan wiring), multi-farm out of scope (one namespace per daemon for v1).


## [2026-06-02] phase 3 shipped | Plan-FMS Phase 3 — daemon SQLite projection + FTS5 search

Lands the daemon-side half of the originally-scoped Phase 3.
`herd-scout-fms` gained a `projection` feature (default-on) wrapping
rusqlite 0.32 (bundled SQLite + FTS5). Daemon opens the projection
next to `<data_dir>/fms/`, wipes-and-rebuilds on every boot, and
spawns a tokio subscriber that applies every `ChangeEvent` as an
upsert. New `ClientMsg::FmsSearchLogs` RPC + daemon handler
(`fms_rpc::handle_search_logs`) run FTS5 BM25-ranked queries through
the projection and reply via the existing `FmsLogList` variant.
egui Records tab gained a search box wired to the same RPC. The
"remote-mode GUI runs its own iroh peer" surface
(`HERD_SCOUT_GUI_MODE`) stays deferred under Phase 5 — no
cross-device records to mirror until durable smol-kv lands or the
daemon owns records-exchange.

Workspace test count: 158 (3 new projection tests). See plan §"Phase
3 shipped" for the full deviation note.
