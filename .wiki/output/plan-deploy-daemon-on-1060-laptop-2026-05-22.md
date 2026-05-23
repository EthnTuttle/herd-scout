---
title: "Plan: Deploy herd-scout-daemon on the GTX 1060 laptop"
type: plan
format: roadmap
generated: 2026-05-22
sources:
  # Project-local wiki
  - .wiki/output/plan-mobile-to-desktop-iroh-rfc-2026-05-20.md
  - herd-scout-daemon/docs/cv-design.md
  - herd-scout-daemon/docs/daemon-split-design.md
  - .wiki/wiki/concepts/iroh-sync-stack.md
  - .wiki/wiki/concepts/mobile-desktop-architecture.md
  # Hub wiki: gtx-1060-headless-ai-server
  - HUB/topics/gtx-1060-headless-ai-server/wiki/topics/gtx-1060-headless-ai-server-synthesis.md
  - HUB/topics/gtx-1060-headless-ai-server/wiki/concepts/pascal-driver-cuda-pinning.md
  - HUB/topics/gtx-1060-headless-ai-server/wiki/concepts/headless-ubuntu-laptop-baseline.md
  - HUB/topics/gtx-1060-headless-ai-server/wiki/concepts/farm-vision-on-gtx-1060.md
  - HUB/topics/gtx-1060-headless-ai-server/wiki/concepts/gpu-thermals-and-ops.md
  - HUB/topics/gtx-1060-headless-ai-server/wiki/concepts/gpu-bench-and-smoke-tests.md
  - HUB/topics/gtx-1060-headless-ai-server/output/plan-gs63vr-headless-server-2026-05-21.md
---

# Plan: Deploy herd-scout-daemon on the GTX 1060 laptop

> Generated from the project-local wiki (5 articles) + the `gtx-1060-headless-ai-server` hub wiki (7 articles). The hub wiki already encodes the hardware-forced decisions (proprietary NVIDIA driver 535-server, CUDA 12.x ceiling, msi-ec unimplemented, battery-removal advisory). This plan layers the herd-scout-specific deployment on top.

## Executive Summary

Stand up the MSI GS63VR (Pascal GTX 1060 6GB mobile, i7-7700HQ, 16 GB RAM) as a **headless Ubuntu 22.04 LTS server** that runs `herd-scout-daemon` as a systemd service. The daemon binds an iroh endpoint, accepts moq sessions from a phone in the field over iroh's relay-fallback path, decodes 720p H.264 video, runs **YOLOv5n via ONNX Runtime CUDA EP** (GPU-accelerated — ~3-10× faster than the current CPU-only path), and serves preview frames + detections over a Unix domain socket to any GUI client that opens an SSH tunnel into the box. The hub wiki's existing GS63VR plan covers transcription + a separate MediaMTX/SRT ingest path; this plan replaces all of that with the herd-scout daemon, reusing the hardware/OS/driver baseline.

Key novelty vs the hub wiki's existing plan: (a) we use **iroh's transport directly** for the video plane, no MediaMTX or SRT, (b) `ort` 2.0.0-rc.12 with the CUDA execution provider replaces the Python+PyTorch CV stack, (c) the daemon binary is a single static-ish Rust executable plus a YOLO ONNX blob — no Python venv, no HF cache.

## User Requirements (from interview)

- **OS**: Ubuntu 22.04 LTS server (per hub wiki — confirmed match)
- **Daemon role**: headless service; **GUI runs on a different machine**
- **GPU**: wire `ort`'s CUDA EP behind a feature flag — recommended for the 1060
- **Network**: iroh's relay fallback (laptop and phone may not be on the same LAN)

## Architecture Decisions

### Decision 1 — Reuse the hub-wiki GS63VR baseline; do NOT re-invent OS/driver setup

**Context**: `[[gtx-1060-headless-ai-server-synthesis]]` already locks: Ubuntu Server 22.04 LTS (NOT desktop), BIOS IGFX + AHCI + WoL, ethernet only, SSH key-only + ufw + fail2ban, `nvidia-driver-535-server` (proprietary, NOT `-open` — Pascal can't use open kernel modules), CUDA 12.x ceiling (CUDA 13 dropped Pascal sm_61), battery removal for 24/7 deployment, `nvidia-smi -pl 65` power cap, throttled for CPU PL1/PL2.

**Decision**: follow the hub wiki's [[headless-ubuntu-laptop-baseline]] + [[pascal-driver-cuda-pinning]] + [[gpu-thermals-and-ops]] verbatim. This plan only differs in what runs *on top* of that baseline.

**Consequences**: zero net-new ops research; one shared truth source for the hardware. If the hub wiki's plan changes (e.g. driver bump from 535 → 570), the herd-scout deployment just inherits it.

### Decision 2 — Build `herd-scout-daemon` natively on Linux; cross-compile rejected

**Context**: Our `Cargo.toml` workspace already builds on macOS via the Frando/moq + iroh stack (`cargo build --workspace` exits 0 on the dev Mac). The daemon's only platform-sensitive code is the `build.rs` Swift rpath shim (macOS-only) and the IPC layer (`cfg(unix)`-gated, works on both).

**Options considered**:
- **A. Cross-compile from macOS to Linux x86_64** (`--target x86_64-unknown-linux-gnu`): would require `cargo zigbuild` or a Linux sysroot; iroh's TLS stack (`tls-aws-lc-rs` feature) cross-compiles cleanly on most setups but adds friction.
- **B. Build natively on the GS63VR**: rust-up + `cargo build` directly on the Linux box. Slower first build (~10-15 min on the i7-7700HQ; iroh + moq + ort = a lot of dependencies) but reproducible, no cross-toolchain headaches.
- **C. GitHub Actions release build**: build CI artifacts on `ubuntu-22.04` runners, scp the binary to the laptop. Cleanest long-term but needs CI setup work.

**Decision**: **Option B for Wave 10** (initial deployment), upgrade to **Option C for Wave 11+** once we ship multiple times. Build on the box first to confirm the toolchain works; add CI later.

**Consequences**: Phase 2 below includes installing the Rust toolchain and the build deps on the GS63VR. The first build is slow; subsequent rebuilds use the cargo cache.

### Decision 3 — Wire the `ort` CUDA execution provider behind a feature flag

**Context**: `herd-scout-daemon/Cargo.toml:29` declares `ort = { version = "2.0.0-rc.12", features = ["download-binaries"] }`. The `download-binaries` feature ships **CPU-only prebuilt ORT binaries**. CUDA EP requires either the `cuda` feature flag (which downloads CUDA-enabled binaries) or `load-dynamic` (which expects a system-installed `libonnxruntime.so` with CUDA support). [[farm-vision-on-gtx-1060]] documents that "TensorRT 10 dropped Pascal — must use TensorRT 8.6.x for sm_61 INT8/FP16 export, or stay on PyTorch CUDA EP / ONNX Runtime CUDA EP".

**Options considered**:
- **A. `ort` feature `cuda`** (downloaded CUDA-enabled ORT): simplest. Pinned ORT version 1.20+ ships CUDA 12.x binaries that should work on Pascal CC 6.1 (the kernels are sm_61-compiled). ~500 MB binary + cuDNN.
- **B. `load-dynamic`** with system `libonnxruntime.so`: more flexible (can pick a Pascal-friendly older ORT), heavier ops burden (manual install).
- **C. Stay CPU-only**: simplest to deploy, throws away the GPU. Hub wiki's [[farm-vision-on-gtx-1060]] benchmarks YOLO11n at 40-60 FPS on the 1060 vs ~10 FPS on the CPU path — meaningful gap.

**Decision**: **Option A**. Add a Cargo feature `gpu` to `herd-scout-daemon/Cargo.toml` that enables `ort/cuda`. Default-disabled so the macOS dev build keeps working. Linux deploy build runs `cargo build --release -p herd-scout-daemon --features gpu`.

**Consequences**: the CV inference path needs a small code change in `herd-scout-daemon/src/cv/model.rs` to register the CUDA EP when the feature is on, with **runtime fallback to CPU if CUDA init fails** (per the user's "belt + suspenders" preference if surfaced in interview — defaulted here for resilience). Increases binary size to ~600 MB on Linux, acceptable for a headless server.

### Decision 4 — Run as systemd service `herd-scout-daemon.service`

**Context**: `[[daemon-split-design]]` § Daemon Lifecycle: "launchd/systemd units. Out of scope for Wave 6." Hub wiki's [[gpu-thermals-and-ops]] documents the systemd patterns already used (nvidia-smi power-cap oneshot, etc.). The `gtx-1060-headless-ai-server` plan already has a systemd unit pattern for its faster-whisper service.

**Decision**: ship a `deploy/systemd/herd-scout-daemon.service` unit file in the repo. Type=simple, Restart=on-failure, restart limit 5/min, journald logging, nvidia-smi power-cap dependency (`After=nvidia-power-cap.service`).

**Consequences**: Phase 4 below covers the unit file + the install script.

### Decision 5 — Iroh's relay path; no MediaMTX/SRT/Tailscale

**Context**: The hub wiki's existing GS63VR plan uses MediaMTX + SRT + Larix Broadcaster on the phone, with ffmpeg+NVDEC pulling frames into a YOLO worker. That's a completely different stack and not what herd-scout uses. Our `[[mobile-desktop-architecture]]` and `[[iroh-sync-stack]]` make iroh the data plane; iroh's relay fallback handles NAT traversal "for free" (the phone in the field and the laptop on home Wi-Fi don't need to be on the same LAN — iroh relays frames through n0-computer's relay infrastructure).

**Decision**: keep the existing iroh-moq path. The laptop's daemon binds an iroh endpoint, mints a `LiveTicket`, displays the QR via the GUI (running on a different machine that ssh-tunnels into the daemon's UDS), the phone scans, dials the laptop's endpoint via relay fallback if needed, publishes. **No MediaMTX, no SRT, no Tailscale.**

**Consequences**: the laptop's only exposed surface is SSH (port 22 via ufw, key-only). The iroh transport handles its own auth (the LiveTicket is the capability). No firewall rules to open for video.

### Decision 6 — GUI connects via SSH tunnel to the daemon's UDS

**Context**: The user picked "daemon-only, headless service — GUI runs on a different machine." The daemon's IPC is a **Unix domain socket** at `~/.local/share/herd-scout/daemon.sock` (per `[[daemon-split-design]]` § IPC protocol). To reach it from a remote GUI:

- SSH `LocalForward` of the UDS: `ssh -L /tmp/herd-scout.sock:/home/<user>/.local/share/herd-scout/daemon.sock <laptop>` (OpenSSH 8.0+ supports UDS forwarding directly)
- Then run the GUI locally: `HERD_SCOUT_SOCKET=/tmp/herd-scout.sock cargo run -p herd-scout-gui` (note: requires a small `herd-scout-gui` change to read the socket path from an env var)

**Decision**: implement the env-var override for the socket path in `herd-scout-gui` (Phase 5 below). Document the SSH tunnel command in the deploy README.

**Consequences**: zero new network surface; SSH already covers the auth. GUI's auto-spawn-daemon-as-child fallback (Wave 6) is bypassed when `HERD_SCOUT_SOCKET` is set.

## Implementation Phases

### Phase 1 — Hardware + OS baseline (estimated effort: 1-2 days)

**Goal**: GS63VR runs Ubuntu Server 22.04 LTS, accessible over SSH, with a working NVIDIA driver and CUDA stack.

**Tasks**:
- [ ] Follow the hub wiki's `output/plan-gs63vr-headless-server-2026-05-21.md` Phases 1-4 verbatim (BIOS, OS install, driver pinning, thermals). Specifically:
  - [ ] BIOS: Secure Boot off, Primary Display = IGFX, AHCI, WoL on
  - [ ] Ubuntu Server 22.04 LTS install, ethernet only, SSH server enabled at install time
  - [ ] `sudo ubuntu-drivers install --gpgpu` → `nvidia-driver-535-server`
  - [ ] Pin via `/etc/apt/preferences.d/nvidia` (block 13.x + driver upgrades)
  - [ ] `nvidia-persistenced` enabled, `nvidia-smi -pm 1`, `nvidia-smi -pl 65` (systemd oneshot)
  - [ ] Remove battery, on cooling pad, lid open, UPS attached
  - [ ] `throttled` for CPU undervolt + PL1/PL2 cap
  - [ ] schedutil CPU governor
  - [ ] ufw + fail2ban + key-only SSH
- [ ] Verify GPU is alive: `nvidia-smi` shows the 1060, `nvcc --version` shows CUDA 12.x

**Dependencies**: GS63VR hardware on hand, ethernet jack + cable.

**Validation**: SSH into the box from your dev Mac; `nvidia-smi` shows GPU at idle 0% util, ~30°C; `lsb_release -a` confirms 22.04.

**Wiki grounding**: `[[gtx-1060-headless-ai-server-synthesis]]`, `[[headless-ubuntu-laptop-baseline]]`, `[[pascal-driver-cuda-pinning]]`, `[[gpu-thermals-and-ops]]`.

### Phase 2 — Build toolchain + first daemon build (estimated effort: 0.5-1 day)

**Goal**: `cargo build --release -p herd-scout-daemon` succeeds on the GS63VR.

**Tasks**:
- [ ] Install Rust toolchain: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable`. Add `~/.cargo/bin` to `PATH`.
- [ ] Install build deps: `sudo apt install -y build-essential cmake pkg-config libssl-dev clang protobuf-compiler git`. (iroh-relay's deps; some may not be strictly needed but they cover the iroh + moq workspace.)
- [ ] Clone the herd-scout repo (or rsync from the dev Mac): `git clone --recursive git@github.com:EthnTuttle/herd-scout.git ~/herd-scout`
- [ ] First build (CPU-only, no GPU feature yet): `cd ~/herd-scout && cargo build --release -p herd-scout-daemon`. Expect ~10-15 min on the 7700HQ.
- [ ] Smoke run (without phone): `./target/release/herd-scout-daemon` — should print the iroh-live ticket and bind the IPC socket.
- [ ] Confirm the IPC socket exists at `~/.local/share/herd-scout/daemon.sock` and the daemon stays alive.

**Dependencies**: Phase 1 complete.

**Validation**: SSH `ls -l ~/.local/share/herd-scout/daemon.sock` shows the socket file; `ps aux | grep herd-scout` shows the daemon running.

**Wiki grounding**: `herd-scout-daemon/Cargo.toml` deps, `[[daemon-split-design]]` § IPC.

### Phase 3 — Wire `ort` CUDA EP behind a `gpu` feature flag (estimated effort: 0.5 day)

**Goal**: `cargo build --release -p herd-scout-daemon --features gpu` succeeds and inference runs on the GTX 1060 (verified via `nvidia-smi` showing GPU util > 0% during inference).

**Tasks**:
- [ ] Edit `herd-scout-daemon/Cargo.toml`: add a `[features]` section with `gpu = ["ort/cuda"]` (verify exact ort feature name in `cargo doc`; may be `cuda` or `cuda-binaries`).
- [ ] Edit `herd-scout-daemon/src/cv/model.rs`: when `cfg(feature = "gpu")` is on, register the CUDA EP via `Session::builder().with_execution_providers([CUDAExecutionProvider::default()])`. Wrap in a fallback: if CUDA init returns `Err`, log WARN and proceed with the default CPU EP.
- [ ] Build: `cargo build --release -p herd-scout-daemon --features gpu`. First build re-fetches ORT with CUDA bindings (~500 MB).
- [ ] Run with a test stream (use a YouTube cattle clip via Android emulator or a saved .mp4 fed through a local moq publisher): observe `nvidia-smi -l 1` shows GPU compute util when frames arrive.
- [ ] Bench: log inference time per frame at INFO level (already partially done in `cv/task.rs`). Compare CPU baseline (~50 ms/frame) to GPU (target: ~5-15 ms/frame on YOLOv5n on a 1060).

**Dependencies**: Phase 2 complete; CUDA 12.x toolkit installed (Phase 1 already covers this via `nvidia-driver-535-server` + cuDNN 9 from pip wheels per hub wiki).

**Validation**: a 30-second test stream at 30 FPS produces detections at the full inference rate (currently capped at 10 FPS — bump to 25-30 FPS in the same edit, since the GPU can sustain it). `nvidia-smi -l 1` shows compute util spiking on each inference.

**Wiki grounding**: `herd-scout-daemon/docs/cv-design.md` § "CoreML/CUDA can be added later behind feature flags", `[[farm-vision-on-gtx-1060]]` for realistic FPS targets.

### Phase 4 — systemd service unit (estimated effort: 0.5 day)

**Goal**: daemon runs as a managed systemd service, restarts on crash, logs to journald.

**Tasks**:
- [ ] Create `deploy/systemd/herd-scout-daemon.service` in the repo:
  ```ini
  [Unit]
  Description=herd-scout daemon — moq subscriber + CV inference
  After=network-online.target nvidia-persistenced.service
  Wants=network-online.target

  [Service]
  Type=simple
  User=herdscout
  Group=herdscout
  WorkingDirectory=/home/herdscout
  ExecStart=/home/herdscout/herd-scout/target/release/herd-scout-daemon
  Restart=on-failure
  RestartSec=5s
  StartLimitBurst=5
  StartLimitIntervalSec=60s
  Environment=RUST_LOG=info,herd_scout_daemon=debug
  StandardOutput=journal
  StandardError=journal

  [Install]
  WantedBy=multi-user.target
  ```
- [ ] Create dedicated user: `sudo useradd -m -s /bin/bash herdscout`. Move the build into `/home/herdscout/herd-scout/`.
- [ ] Symlink: `sudo ln -s ~/herd-scout/deploy/systemd/herd-scout-daemon.service /etc/systemd/system/`. `sudo systemctl daemon-reload && sudo systemctl enable --now herd-scout-daemon`.
- [ ] Verify: `systemctl status herd-scout-daemon` is active; `journalctl -u herd-scout-daemon -f` shows the iroh ticket and "ready, listening on UDS" lines.
- [ ] Reboot the laptop; confirm the daemon comes back automatically.

**Dependencies**: Phase 3 complete (we want the GPU-enabled build managed).

**Validation**: `sudo reboot` and after 60 s, `systemctl is-active herd-scout-daemon` returns `active`.

**Wiki grounding**: hub wiki `output/plan-gs63vr-headless-server-2026-05-21.md` Phase 6 (systemd patterns), `[[gpu-thermals-and-ops]]`.

### Phase 5 — GUI connects to remote daemon over SSH UDS forward (estimated effort: 0.5-1 day)

**Goal**: GUI on the dev Mac displays the live feed + CV detections from the GS63VR daemon, via an SSH tunnel.

**Tasks**:
- [ ] Edit `herd-scout-gui/src/ipc/client.rs`: read `HERD_SCOUT_SOCKET` env var as a socket path override. When set, skip the auto-spawn-daemon path and connect directly. Fallback to existing behavior when unset.
- [ ] Test the SSH UDS forward locally: `ssh -L /tmp/herd-scout.sock:/home/herdscout/.local/share/herd-scout/daemon.sock herdscout@<laptop-ip>` (requires OpenSSH 8.0+ on both ends — confirm with `ssh -V`). Background with `-fN`.
- [ ] Run the GUI: `HERD_SCOUT_SOCKET=/tmp/herd-scout.sock cargo run -p herd-scout-gui`. Confirm: pairing screen renders the QR, status bar shows "Connecting" / "Idle" reflecting the *daemon's* state.
- [ ] Document the workflow in `deploy/README.md` with a copy-paste section for the SSH tunnel command.
- [ ] Optional: a small shell helper at `deploy/connect-gui.sh` that opens the tunnel + launches the GUI.

**Dependencies**: Phase 4 complete (daemon is reliably running on the laptop).

**Validation**: from the dev Mac, GUI connects, shows the QR, you can scan it from your phone, frames flow back to the GUI through: phone → iroh-relay → laptop daemon → SSH-tunneled UDS → GUI. End-to-end e2e.

**Wiki grounding**: `[[daemon-split-design]]` § IPC + Daemon Lifecycle, `[[mobile-desktop-architecture]]` § "Desktop is just another peer."

### Phase 6 — Smoke tests + monitoring (estimated effort: 0.5 day)

**Goal**: confidence the deployment is durable for unattended field use.

**Tasks**:
- [ ] Run `gpu-burn` for 10 min (per hub wiki's [[gpu-bench-and-smoke-tests]]) to confirm thermals hold under load. **Build with `make COMPUTE=6.1`** (Pascal sm_61).
- [ ] Bench inference throughput: feed a 2-min cattle clip into a local moq publisher, measure frames-decoded and frames-inferred. Target: 30 FPS decode, 25-30 FPS inference (vs CPU baseline of ~10 FPS).
- [ ] Monitor GPU temp over a 30-min sustained stream: `nvidia-smi --query-gpu=temperature.gpu,power.draw --format=csv -l 5`. Expected: ≤ 80°C with the power cap at 65W. If higher, drop the cap to 55W.
- [ ] Configure `journald` retention: edit `/etc/systemd/journald.conf` → `SystemMaxUse=500M`, `MaxRetentionSec=4week` (per hub wiki synthesis).
- [ ] Optional: install DCGM-exporter for Prometheus-style GPU monitoring; out of MVP scope but well-documented in the hub wiki.

**Dependencies**: Phase 5 complete.

**Validation**: 30-min sustained stream at full FPS without dropouts, GPU temp stays under 80°C, journal logs show no unexpected restart events.

**Wiki grounding**: hub wiki `[[gpu-bench-and-smoke-tests]]`, `[[gpu-thermals-and-ops]]`.

### Phase 7 (optional, post-MVP) — GitHub Actions release build (estimated effort: 1 day)

**Goal**: `git push` produces a deploy artifact; pull it onto the GS63VR with a single `wget`.

**Tasks**:
- [ ] `.github/workflows/release-daemon.yml` — runs on `ubuntu-22.04`, installs Rust + CUDA 12.x toolchain, builds `--release --features gpu`, uploads the binary as a release asset.
- [ ] On the GS63VR: a small `deploy/update-daemon.sh` that downloads the latest release binary, verifies sha256, restarts the systemd service.
- [ ] Tagged releases drive the workflow; rolling builds optional.

**Dependencies**: Phases 1-6 complete; project has stable enough deploy cadence to warrant CI.

**Validation**: a fresh `git tag v0.2.0 && git push --tags` results in an artifact downloadable via `wget` on the GS63VR; `update-daemon.sh` deploys it; service comes back up cleanly.

**Wiki grounding**: hub wiki's existing CI patterns (if any) — check `iroh-transport-stratum-v2` topic for prior art.

## Risks & Mitigations

| Risk | Source | Mitigation |
|---|---|---|
| **`ort` CUDA feature has runtime issues on Pascal sm_61** | [[farm-vision-on-gtx-1060]] flags TRT 10 dropped Pascal; ORT bundled CUDA may have similar issues | Phase 3 includes a runtime fallback to CPU on CUDA init failure. If GPU just doesn't work, deploy stays viable on CPU at lower FPS. |
| **iroh's relay path adds 80-250 ms latency on rural 4G** | `[[android-on-drone]]` § 4G latency | The user explicitly chose reliability over latency. Buffer aggressively (already done in `stream.rs`); document expectation. |
| **GS63VR thermals under sustained CV load** | [[gpu-thermals-and-ops]] | Power-cap at 65W (or drop to 55W); cooling pad; lid open; remove battery for 24/7. Monitor temp in Phase 6. |
| **CUDA toolkit version mismatch (driver 535 vs ORT bundled)** | [[pascal-driver-cuda-pinning]] | Pin both via apt preferences (driver) + `Cargo.lock` (ort). Test on the actual hardware before declaring Phase 3 done. |
| **Daemon segfault under load → systemd restart loop** | `[[daemon-split-design]]` § Daemon Lifecycle Risks | systemd unit has `StartLimitBurst=5` `StartLimitIntervalSec=60s` — fail fast, surface the issue rather than burn cycles in a loop. journalctl shows the panic. |
| **First Rust build OOMs on the 16 GB box** | iroh + moq + ort = many crates, debug builds use ~10 GB | Build `--release` only (smaller intermediate state); add 8 GB swap at Phase 1; build serially (no `-j` override). |
| **`cargo build` runs out of disk** | Linux dev Mac has plenty; the GS63VR's SSD might not | Phase 2 confirms ≥ 30 GB free in `/home/herdscout` before first build. iroh-live submodule alone is multi-GB once compiled. |
| **SSH UDS forwarding fails (older OpenSSH on either end)** | OpenSSH 8.0+ required for UDS forwarding | Phase 5 verifies `ssh -V` on both ends. Fallback: forward a TCP port (`-L 8765:localhost:8765`) and add a TCP listener mode to the daemon (small extra IPC server task; out of MVP scope). |
| **YOLOv5n COCO accuracy degrades on aerial cattle** | [[2026-05-21-herdnet-deep-dive]] (just-completed research) | Capture training data during field tests; fine-tune YOLOv8n on in-house aerial in a future wave. The 1060 can train YOLO11s overnight per [[farm-vision-on-gtx-1060]]. |
| **No mux switch on the GS63VR, IGFX boot doesn't expose the 1060 to display** | [[headless-ubuntu-laptop-baseline]] | Headless server only; we never need video output from the 1060. Acceptable. |

## Open Questions

- **Which build of CUDA-enabled ORT does `ort = "2.0.0-rc.12" + features = ["cuda"]` actually pull?** Need to verify the ORT version it grabs supports sm_61. Phase 3 will surface this; if the bundled ORT is too new, fall back to `load-dynamic` + a manually-installed older `libonnxruntime.so` per [[farm-vision-on-gtx-1060]]'s "stay on PyTorch CUDA EP / ONNX Runtime CUDA EP" note.
- **Should the daemon expose a `--bind-public` mode** that listens on a public iroh-relay-routable endpoint, vs the current "any incoming session is accepted"? Not needed for MVP but relevant when there are multiple phones in the field.
- **Multi-phone fan-in**: today's daemon handles one phone at a time on the data path. The 1060 + 16 GB RAM could plausibly handle 2-3 simultaneous phones at 1280×720 / 10 FPS inference. Worth benchmarking once we have multiple phones; out of this plan's scope.
- **GUI on iPad / phone**: post-MVP. The hub wiki's plan specifically mentions "Android v1 / iOS v2" — same constraint here, since the GUI is currently `cfg(unix)` desktop-only.

## Sources Consulted

| Source | What was drawn from it |
|---|---|
| `.wiki/output/plan-mobile-to-desktop-iroh-rfc-2026-05-20.md` | Performance baselines, network reach decisions, original CV runtime decisions |
| `herd-scout-daemon/docs/cv-design.md` | YOLOv5n model details, the explicit "CoreML/CUDA can be added later behind feature flags" handoff |
| `herd-scout-daemon/docs/daemon-split-design.md` | IPC over UDS architecture; explicit "launchd/systemd is out of scope for Wave 6" → this plan picks it up |
| `[[iroh-sync-stack]]` | iroh transport layer; relay-fallback for non-LAN reach |
| `[[mobile-desktop-architecture]]` | Desktop = peer, daemon role, headless-deployable shape |
| HUB `[[gtx-1060-headless-ai-server-synthesis]]` | Hardware verdict (Pascal forces proprietary driver, CUDA 12.x ceiling, msi-ec unimplemented, battery removal) |
| HUB `[[pascal-driver-cuda-pinning]]` | `nvidia-driver-535-server` + apt pinning, no CUDA 13 |
| HUB `[[headless-ubuntu-laptop-baseline]]` | BIOS settings, Server (not Desktop), SSH key-only + ufw + fail2ban |
| HUB `[[farm-vision-on-gtx-1060]]` | Realistic YOLO FPS targets on the 1060; Pascal-specific inference notes; TRT 10 dropped Pascal warning |
| HUB `[[gpu-thermals-and-ops]]` | nvidia-smi power cap, throttled, nbfc-linux, systemd patterns |
| HUB `[[gpu-bench-and-smoke-tests]]` | gpu-burn `make COMPUTE=6.1`, smoke-test methodology |
| HUB `output/plan-gs63vr-headless-server-2026-05-21.md` | Reference plan (this is the parallel-track plan; we replace the content but reuse the phasing structure) |

## Proposed Inventory Records (suggested, not auto-created)

If this plan kicks off a durable work queue, the following inventory items would be tracked. Sample (3 rows) before creating:

| ID | Type | Description | Status |
|---|---|---|---|
| `q1` | open-question | Verify `ort=2.0.0-rc.12 + features=["cuda"]` ships sm_61-compatible kernels | open |
| `t1` | task | Capture aerial cattle training data during Wave 4 field test for future YOLOv8n fine-tune | blocked-on Wave 4 |
| `w1` | watch-item | When `ort` releases stable 2.0; revisit feature-flag wiring | watching |

Tell me if you want these created.
