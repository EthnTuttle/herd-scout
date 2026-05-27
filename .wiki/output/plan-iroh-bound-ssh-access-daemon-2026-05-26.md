---
title: "Plan: Iroh-bound SSH access for herd-scout-daemon (no DNS, NAT-traversed)"
type: plan
format: roadmap
generated: 2026-05-26
sources:
  # Project-local wiki
  - .wiki/wiki/concepts/iroh-sync-stack.md
  - .wiki/wiki/concepts/mobile-desktop-architecture.md
  - .wiki/wiki/concepts/herd-scout-positioning.md
  - .wiki/output/plan-deploy-daemon-on-1060-laptop-2026-05-22.md
  - herd-scout-daemon/docs/daemon-split-design.md
  # Code referenced
  - vendor/iroh-live/iroh-live/src/live.rs
  - herd-scout-daemon/src/ipc/server.rs
  # Gap research
  - https://github.com/n0-computer/dumbpipe
  - https://docs.rs/iroh/latest/iroh/protocol/index.html
---

# Plan: Iroh-bound SSH access for herd-scout-daemon

> Generated from the project-local wiki (5 articles + 2 design docs) plus targeted research on n0-computer/dumbpipe and the iroh `Router::accept` extension point. Builds on `plan-deploy-daemon-on-1060-laptop-2026-05-22` Decision 6, which currently uses raw OpenSSH UDS forwarding to reach the daemon and assumes the operator has a working hostname/route to the GS63VR. This plan removes that assumption.

## Executive Summary

Add a third ALPN to the daemon's existing `Live` router that bridges incoming bi-directional QUIC streams straight into the laptop's local OpenSSH (`127.0.0.1:22`). Gate the handler on a NodeId allowlist read from the daemon config — anything not in the list is dropped at the iroh layer before sshd ever sees a byte. Ship a small companion binary `herdctl proxy <node-id>` that copies stdin/stdout to a fresh QUIC stream on that ALPN, designed to be wired into `~/.ssh/config` as a `ProxyCommand`. From that point forward, every existing tool — `ssh`, `scp`, `sftp`, `ssh -L /tmp/herd-scout.sock:.../daemon.sock`, `ssh-agent` forwarding — works against `Host herd-scout-laptop` with no DNS, no port forwarding, and no Tailscale, and the GUI's `HERD_SCOUT_SOCKET=/tmp/herd-scout.sock cargo run -p herd-scout-gui` flow from Decision 6 keeps working unchanged.

This is on-thesis: wedge #2 of `[[herd-scout-positioning]]` is "P2P / offline-first with no central server," and `[[mobile-desktop-architecture]]` already declares "desktop is just another peer of the same iroh namespace — not a server." We are extending iroh's role from data-plane-only to data-plane + control-plane on the same `Live` instance, sharing a single iroh Endpoint.

Key novelty vs `plan-deploy-daemon-on-1060-laptop`: (a) we register a new ALPN `herd-scout/ssh/1` on the daemon's existing Router instead of standing up a second iroh node, (b) auth is "NodeId allowlist + delegate to sshd" — daemon never sees a password, sshd handles user auth as today, (c) we deliberately reject building on dumbpipe directly because it has no peer authorization (any holder of the NodeId can dial), but we mirror its CLI shape for the proxy client.

## User Requirements (from interview)

- **Use case**: GUI reaches daemon UDS without DNS / port-forwarding / Tailscale (replaces Decision 6 of the deploy plan). Interactive shell, scp, and `-L` UDS forward all fall out of using OpenSSH on top.
- **Auth**: allowlisted iroh NodeIds at the daemon. No new credentials, no passwords, no captive shell.
- **Implementation**: SSH `ProxyCommand` over iroh. Reuse OpenSSH end-to-end; do **not** embed `russh` or write a Rust SSH server.
- **Discovery**: static config — daemon prints its NodeId once, user pastes into `~/.ssh/config` and `~/.config/herdctl/config.toml`. No gossip-based discovery in this plan.

## Architecture Decisions

### Decision 1 — One iroh Endpoint, three ALPNs (don't spin up a second node)

**Context**: `vendor/iroh-live/iroh-live/src/live.rs:168-170` shows `Live::with_router()` registers exactly two ALPNs on the `Router`: `iroh_moq::ALPN` (data plane) and `iroh_gossip::ALPN` (presence). The `Router` builder API supports additional `accept(alpn, handler)` calls, and the daemon already holds the live `Live` for its lifetime (per `daemon-split-design.md` § Pairing flow step 1). `[[iroh-sync-stack]]` is explicit that one iroh node "provides the data plane that makes phone↔desktop offline-first work" — running a second node for control plane would double the relay traffic, double the persistent state, and split the operator's mental model.

**Options considered**:
- **A. Add an ALPN to the existing `Live` router.** Same Endpoint, same NodeId, same relay address. Daemon prints one NodeId, ops uses it for both pairing and SSH.
- **B. Spawn a second `iroh::Endpoint` dedicated to control plane.** Cleaner separation; lets us use a different relay if we want. But: two NodeIds to manage, twice the relay/discovery overhead, and requires either a second QR or a second config field on the client.
- **C. Use dumbpipe in a sidecar process.** Reuse n0's existing tool; daemon doesn't grow new code. But dumbpipe has **no NodeId allowlist** (confirmed via gap research) — anyone holding the NodeId can dial. Doesn't satisfy the chosen auth model.

**Decision**: Option A. Register a new ALPN `b"herd-scout/ssh/1"` on `Live`'s router via a fork or wrapper of `Live::with_router()`, or via a small upstream patch to `vendor/iroh-live` that exposes additional `.accept(alpn, handler)` calls during builder construction.

**Consequences**: We need a way to register additional ALPNs without re-implementing `with_router`. Either (a) extend `iroh-live`'s builder with `.with_extra_protocol(alpn, handler)`, or (b) bypass `with_router` entirely in the daemon and build the `Router` ourselves (`Router::builder(endpoint).accept(moq_alpn, ...).accept(gossip_alpn, ...).accept(ssh_alpn, ...).spawn()`). Option (a) is the upstream-friendly path; option (b) lands faster but duplicates the matching logic from `iroh-live`. Pick (b) for Wave 11; offer (a) upstream once the shape proves out.

### Decision 2 — Bridge the QUIC stream to localhost sshd; do not embed an SSH server

**Context**: The user explicitly chose "ssh ProxyCommand over iroh" over "Native PTY-over-iroh (russh-based)." Rationale: every line of SSH-server code we write is a line of auth-critical code we have to maintain. OpenSSH on the GS63VR is already configured per the deploy plan's Phase 1 (key-only, ufw, fail2ban). PTY semantics, agent forwarding, env propagation, scp/sftp subsystems, `MaxStartups`, `LoginGraceTime`, `Match` blocks — sshd already implements all of it correctly.

**Decision**: the ALPN handler is a dumb byte pump. On accept: `connection.accept_bi() → (send, recv)`, open `tokio::net::TcpStream::connect("127.0.0.1:22")`, then `tokio::io::copy_bidirectional`. Done. **No PTY, no auth code, no shell parsing inside the daemon.**

**Consequences**: sshd must be running on the laptop (it already is per Phase 1 of the deploy plan). The laptop's `/etc/ssh/sshd_config` is the source of truth for SSH-side policy — the daemon doesn't replicate it. If the operator wants to disable password auth or restrict shells, they edit sshd_config the same way they would for any Linux box.

### Decision 3 — NodeId allowlist gates incoming connections at the iroh layer

**Context**: User picked "Allowlisted iroh node IDs" with the preview:

```toml
[control_plane]
allowed_node_ids = [
  "abc123…",  # Gary's dev Mac
  "def456…",  # Gary's phone (ops)
]
```

The iroh `Connection` struct exposes `remote_node_id()` (or equivalent on the 0.98 branch — verify against `vendor/iroh-live`'s pinned iroh version). Rejection happens before any SSH bytes are read.

**Decision**: at daemon boot, load `~/.config/herd-scout-daemon/control.toml` (or `$HERD_SCOUT_CONFIG_DIR/control.toml`). Parse `[control_plane].allowed_node_ids`. Pass an `Arc<HashSet<NodeId>>` into the ALPN handler. On `accept(connection)`: read `connection.remote_node_id()`, if not in the set, log `WARN dropping unauthorized control-plane dial from {node_id}` and close the connection immediately (do **not** open the TCP bridge). Reload on `SIGHUP` so adding a new device doesn't require a restart.

**Consequences**: dropping a stranger is O(1) — just a hashset lookup. The daemon does not speak SSH protocol to unauthorized peers, so they cannot probe sshd through us. We pay one config file's worth of operational overhead; the file is small, hand-edited, and lives under `~/.config/`. Misconfiguration mode = an empty allowlist = nobody can connect = the operator gets a clear log line and an obvious fix. Self-dial (NodeId == own NodeId) is also rejected, mirroring the iroh "Connecting to ourself is not supported" guardrail noted in `daemon-split-design.md` § Why.

### Decision 4 — `herdctl proxy` is a dedicated bin crate; ships alongside daemon and GUI

**Context**: `~/.ssh/config`'s `ProxyCommand` invokes a binary that copies stdin/stdout. dumbpipe demonstrates the shape (`dumbpipe connect <ticket>` does this), but we need NodeId-allowlist-aware behavior on the *daemon* side, not the client side; the client just dials. We could call `dumbpipe connect` directly, but: (a) it speaks dumbpipe's ALPN, not ours, and (b) we want a stable name and a stable ALPN string under our control.

**Decision**: add a fourth workspace member `herdctl` (`herdctl/Cargo.toml`, `herdctl/src/main.rs`). Subcommands:

- `herdctl proxy <node-id>` — open a QUIC bi-stream on `b"herd-scout/ssh/1"` to the given NodeId, copy stdin↔send, recv↔stdout. Exit when either side closes.
- `herdctl forward <node-id> <local-uds-path>` — *future*; bind a local UDS at `<local-uds-path>`, forward each accept to a fresh control-plane stream that goes to the daemon's IPC UDS path on the remote. (Replaces `ssh -L` for the GUI flow without needing sshd at all. **Out of scope for Wave 11**, captured here so we don't paint ourselves into a corner.)
- `herdctl ping <node-id>` — connect-and-disconnect health check; exit 0 if the allowlist accepts us, non-zero otherwise.

**Consequences**: `herdctl` depends on iroh + tokio but **not** on iroh-live, iroh-moq, ort, or any of the daemon's heavy deps. Should compile in <2 minutes and produce a small binary. Distributed by `cargo install --path herdctl` on each operator machine; no system service.

### Decision 5 — Reuse the daemon's existing iroh persistence; no separate identity

**Context**: The daemon already persists its iroh secret key (the NodeId is derived from it) under `directories::ProjectDirs` per `daemon-split-design.md` § IPC protocol. Generating a separate identity for the control plane would mean two NodeIds the user has to track, which contradicts Decision 1.

**Decision**: the control-plane ALPN runs on the **same** iroh Endpoint as the data plane. The daemon prints one NodeId at boot to stdout/journald: `herd-scout-daemon ready: NodeId=k51qz...`. That string goes into both the operator's `~/.ssh/config` (as `HostName`) and the operator's pairing flow (as part of the `LiveTicket`).

**Consequences**: rotating the daemon's identity rotates everything at once. Acceptable — these all live on the same physical box, and the operator's `~/.ssh/config` and any client-side allowlists need to be updated together anyway.

### Decision 6 — `herd-scout/ssh/1` ALPN versioning

**Context**: ALPNs are forever. If we ever want to change the framing (e.g. multiplex multiple TCP destinations on one stream, or add a length-prefixed handshake), we need a versioned ALPN.

**Decision**: ALPN is the literal byte string `b"herd-scout/ssh/1"`. Version 1 = "open one bi-stream, byte-pipe to `127.0.0.1:22`." Future `ssh/2` could carry a target-port handshake (`{port: 22}` on first frame) so the same ALPN handles arbitrary local ports. Not building that now; the bytes spent on `/1` keep the door open.

**Consequences**: clients pin the ALPN string; daemon only accepts that exact string. A future v2 daemon supports both ALPNs in parallel for one release before retiring v1.

## Implementation Phases

### Phase 1 — Wire a third ALPN onto the daemon's Router (estimated effort: 0.5 day)

**Goal**: Daemon accepts a connection on `b"herd-scout/ssh/1"` and immediately closes it after logging the remote NodeId. No bridge yet, no allowlist yet — just prove the ALPN routes correctly.

**Tasks**:
- [ ] Read `vendor/iroh-live/iroh-live/src/live.rs:160-200` to confirm whether `Live::with_router()` consumes the `Router::builder(...)` mid-construction or returns a builder we can extend. If it consumes, file a tiny upstream patch adding `Live::with_router_extra(impl FnOnce(RouterBuilder) -> RouterBuilder)` or land Decision 1 option (b) — build the Router by hand in the daemon and skip `with_router()`.
- [ ] Create `herd-scout-daemon/src/control/mod.rs` and `herd-scout-daemon/src/control/handler.rs`. Define `pub const ALPN: &[u8] = b"herd-scout/ssh/1";` and a `ControlHandler` struct implementing `iroh::protocol::ProtocolHandler`.
- [ ] Stub `ProtocolHandler::accept(&self, connection)`:
  ```rust
  let node_id = connection.remote_node_id().ok();
  tracing::info!(?node_id, "control-plane dial received");
  connection.close(0u32.into(), b"phase-1-stub");
  Ok(())
  ```
- [ ] Wire the handler into the daemon's startup path next to the existing `Live::from_env(...).with_router().with_gossip().spawn()` call.
- [ ] On the dev Mac: `cargo install --path tools/dial-tester` (or one-off `cargo run`) — a 30-line binary that dials a NodeId on the new ALPN and prints the close reason. Confirm the daemon logs `control-plane dial received`.

**Dependencies**: daemon builds and runs (Wave 6 baseline); iroh 0.98 API documented enough to know whether `RouterBuilder` is exposed.

**Validation**: dial-tester exits cleanly, daemon journal shows the `INFO` line with the dialer's NodeId.

**Wiki grounding**: `vendor/iroh-live/iroh-live/src/live.rs:168-170` (Router accept pattern), `https://docs.rs/iroh/latest/iroh/protocol/index.html` (ProtocolHandler shape from gap research).

### Phase 2 — NodeId allowlist with hot reload (estimated effort: 0.5 day)

**Goal**: Unauthorized dials are dropped before any TCP bridge exists. Authorized dials reach a stub that just logs "would bridge."

**Tasks**:
- [ ] Define `herd-scout-daemon/src/control/config.rs` with serde:
  ```rust
  #[derive(Deserialize)]
  pub struct ControlConfig {
      pub allowed_node_ids: HashSet<NodeId>,
      #[serde(default = "default_target")] pub ssh_target: SocketAddr, // 127.0.0.1:22
  }
  ```
- [ ] Resolve config path: `$HERD_SCOUT_CONFIG_DIR/control.toml` if set, else `directories::ProjectDirs::from("net", "herd-scout", "herd-scout").config_dir().join("control.toml")`. Match the layout `daemon-split-design.md` § IPC protocol established for the IPC socket.
- [ ] Boot-time load: if file is missing, log `WARN no control.toml found at <path> — control plane open to nobody` and proceed with empty allowlist (fail-closed by default).
- [ ] Wrap config in `Arc<ArcSwap<ControlConfig>>` so the handler reads it lock-free per-connection.
- [ ] Install a `SIGHUP` handler (`tokio::signal::unix::signal(SignalKind::hangup())`) that re-reads the file and atomically swaps. On parse error: keep old config, log `ERROR`.
- [ ] In `ControlHandler::accept`: read `connection.remote_node_id()`; if `None` or not in `cfg.allowed_node_ids`, log `WARN dropping unauthorized control-plane dial`, return immediately. Otherwise log `INFO authorized dial from {node_id}` and close with `b"phase-2-stub"`.

**Dependencies**: Phase 1 complete.

**Validation**:
1. Empty allowlist + dial-tester from a known NodeId → daemon logs WARN, dial-tester sees a close. ✓
2. Add the dial-tester's NodeId to `control.toml`, send `kill -HUP $(pgrep herd-scout-daemon)`, dial again → daemon logs `authorized dial`. ✓
3. Boot daemon with malformed TOML → daemon stays up, logs `ERROR config parse failed, keeping previous`. ✓

**Wiki grounding**: `[[mobile-desktop-architecture]]` § anti-patterns ("LWW with device wallclock — field devices have unsynced clocks") inspires the fail-closed-on-empty default; we don't trust the absence of config to mean "allow everyone."

### Phase 3 — Bridge accepted streams to `127.0.0.1:22` (estimated effort: 0.5 day)

**Goal**: Real SSH bytes flow. From a dev Mac with `ssh -o ProxyCommand=...` we get an OpenSSH login prompt on the GS63VR.

**Tasks**:
- [ ] In `ControlHandler::accept`, after the allowlist check:
  ```rust
  let (mut send, mut recv) = connection.accept_bi().await?;
  let mut tcp = tokio::net::TcpStream::connect(cfg.ssh_target).await
      .map_err(|e| { tracing::error!(?e, "sshd connect failed"); e })?;
  let (mut tcp_r, mut tcp_w) = tcp.split();
  let to_sshd = tokio::io::copy(&mut recv, &mut tcp_w);
  let from_sshd = tokio::io::copy(&mut tcp_r, &mut send);
  let _ = tokio::try_join!(to_sshd, from_sshd);
  ```
- [ ] Wrap the whole accept in a `tokio::spawn` so a slow sshd doesn't block other dials.
- [ ] Cap concurrent control-plane sessions at `MAX_CONTROL_SESSIONS = 16` (tracked via an `AtomicUsize`); over-cap dials are rejected with `b"too-many-sessions"`. sshd has its own `MaxStartups`, but ours is the first line.
- [ ] Add structured tracing spans: `control_session_id`, `remote_node_id`, `bytes_to_sshd`, `bytes_from_sshd`. On disconnect, log session duration + byte counts at DEBUG.
- [ ] Smoke test from the dev Mac:
  ```bash
  # Dial directly without ssh, just byte-pipe a stub
  cargo run -p herdctl -- proxy <NODE_ID> < /dev/null | head -c 32 | xxd
  # Should show "SSH-2.0-OpenSSH_..." banner
  ```

**Dependencies**: Phase 2 complete; sshd running on port 22 of the daemon host.

**Validation**: from the dev Mac, `cargo run -p herdctl -- proxy <NODE_ID>` followed by stdin keystrokes that look like SSH client bytes elicits the SSH banner in stdout. Real `ssh -o ProxyCommand='cargo run -p herdctl -q -- proxy %h' herdscout@<NODE_ID>` lands at the password/key prompt. After accepting our `~/.ssh/id_ed25519.pub` (already in `~herdscout/.ssh/authorized_keys` per Phase 1 of the deploy plan), we get a shell. `journalctl -u herd-scout-daemon -f` shows session-start and session-end log lines. `whoami` returns `herdscout`. `nvidia-smi` works as expected.

**Wiki grounding**: `plan-deploy-daemon-on-1060-laptop-2026-05-22` Decision 6 (existing UDS-forward flow that this replaces); the gap-research finding that dumbpipe's `connect-tcp` does the same byte-pump pattern and is field-tested.

### Phase 4 — Build `herdctl proxy` as a workspace bin (estimated effort: 0.5 day)

**Goal**: `cargo install --path herdctl` produces a small binary; `~/.ssh/config` works copy-paste.

**Tasks**:
- [ ] Create workspace member `herdctl/`. `Cargo.toml`: deps = `iroh = "0.98"` (or whatever the daemon pins), `tokio = { version = "1", features = ["rt-multi-thread", "macros", "io-std", "io-util"] }`, `clap = { version = "4", features = ["derive"] }`, `tracing-subscriber`. **No iroh-live, no iroh-moq, no ort.**
- [ ] `herdctl/src/main.rs` skeleton:
  ```rust
  #[derive(Parser)] struct Cli { #[command(subcommand)] cmd: Cmd }
  #[derive(Subcommand)] enum Cmd {
      Proxy { node_id: NodeId },
      Ping  { node_id: NodeId },
      // Forward { node_id: NodeId, local_uds: PathBuf } — TODO Wave 12
  }
  ```
- [ ] `proxy` subcommand: build an `Endpoint` (no relay restrictions, default config), `endpoint.connect(node_addr, b"herd-scout/ssh/1")`, `connection.open_bi()`, `tokio::try_join!(copy(stdin → send), copy(recv → stdout))`. Exit 0 on clean close, non-zero on error.
- [ ] `ping` subcommand: same connect, immediately call `connection.close()`, exit 0 if open succeeded. Useful for health checks and `ssh -o ConnectTimeout` retries.
- [ ] Unit-test framing: `herdctl proxy <node-id> < known.bin > out.bin` against a daemon configured to echo bytes back (test-only ALPN handler `b"herd-scout/ssh-echo/1"` gated under `cfg(test)`).
- [ ] Build size check: `cargo build --release -p herdctl` should produce a stripped binary < 30 MB. iroh has historically been heavy; if it's bigger, that's fine but note it.

**Dependencies**: Phase 3 complete.

**Validation**: drop the snippet below into `~/.ssh/config` on the dev Mac (substituting the real NodeId) and run `ssh herd-scout-laptop`:
```
Host herd-scout-laptop
  HostName k51qz...                       # iroh NodeId, not a hostname
  User herdscout
  ProxyCommand /Users/garykrause/.cargo/bin/herdctl proxy %h
  ServerAliveInterval 30
```

Both `scp file.bin herd-scout-laptop:~/` and `ssh -L /tmp/herd-scout.sock:/home/herdscout/.local/share/herd-scout/daemon.sock herd-scout-laptop` work, restoring exactly the GUI flow from `plan-deploy-daemon-on-1060-laptop-2026-05-22` Decision 6 — but with no Wi-Fi network, no DNS, no port forwarding required.

**Wiki grounding**: dumbpipe's `connect`/`listen` CLI shape (gap research) — we adopt the proxy pattern but bind it to our ALPN and ship it in-tree.

### Phase 5 — Operator docs, defaults, and a one-shot installer (estimated effort: 0.5 day)

**Goal**: `deploy/README.md` shows a copy-paste flow that takes a fresh laptop from "daemon installed" to "GUI running on dev Mac" without any IP-layer config.

**Tasks**:
- [ ] Update `deploy/README.md`:
  - Replace the "SSH UDS forward" section from Decision 6 of the deploy plan with the iroh-bound flow.
  - Document the `control.toml` schema, where it lives, and how to find your dev-Mac NodeId (`herdctl whoami` — adds a fourth subcommand that just prints the local Endpoint NodeId, ~10 lines).
  - Document `kill -HUP` for hot reload.
- [ ] `deploy/install-control-plane.sh` (idempotent):
  ```bash
  set -euo pipefail
  CFG=${HERD_SCOUT_CONFIG_DIR:-$HOME/.config/herd-scout}
  mkdir -p "$CFG"
  [ -f "$CFG/control.toml" ] || cat > "$CFG/control.toml" <<'EOF'
  # Add one entry per device that should reach this daemon.
  [control_plane]
  allowed_node_ids = []
  ssh_target = "127.0.0.1:22"
  EOF
  ```
- [ ] Add a `[Wave 11 — control plane]` section to `herd-scout-daemon/docs/daemon-split-design.md`'s "Daemon lifecycle" subsection, recording the ALPN, the config path, and the SIGHUP semantics.
- [ ] Update `plan-deploy-daemon-on-1060-laptop-2026-05-22` Decision 6 with a forward-pointer: "Wave 11 replaces this with iroh-bound SSH; see `plan-iroh-bound-ssh-access-daemon-2026-05-26`." (Don't rewrite the historical decision — annotate it.)

**Dependencies**: Phase 4 complete.

**Validation**: a colleague (or you, on a fresh machine) follows `deploy/README.md` end-to-end and gets a working `ssh herd-scout-laptop` with no help.

**Wiki grounding**: `[[herd-scout-positioning]]` wedge #2 (P2P / no central server) — this phase is where the wedge becomes operationally real.

### Phase 6 — (optional) `herdctl forward` for direct UDS-over-iroh, no SSH involved (estimated effort: 1 day)

**Goal**: GUI can reach the daemon's IPC UDS without sshd in the loop at all. Removes the OpenSSH-8.0-on-both-ends requirement from `plan-deploy-daemon-on-1060-laptop-2026-05-22` Risk #SSH-UDS-forwarding-fails.

**Tasks**:
- [ ] Define a second daemon ALPN `b"herd-scout/uds/1"`. Handler: on `accept_bi()`, connect to the daemon's existing IPC UDS path (the one already at `~/.local/share/herd-scout/daemon.sock`), `copy_bidirectional`. Reuse the same NodeId allowlist from Decision 3.
- [ ] `herdctl forward <node-id> <local-uds-path>` subcommand: bind a Unix listener at `<local-uds-path>` (delete-then-bind to recover stale), and on each accept open a fresh QUIC bi-stream on `b"herd-scout/uds/1"` and `copy_bidirectional` between them.
- [ ] GUI flow becomes:
  ```bash
  herdctl forward <NODE_ID> /tmp/herd-scout.sock &
  HERD_SCOUT_SOCKET=/tmp/herd-scout.sock cargo run -p herd-scout-gui
  ```
- [ ] Document the tradeoff: this path skips OpenSSH entirely, so you no longer get scp/sftp/agent-forwarding/PTY for free — you only get IPC UDS access. SSH ProxyCommand from Phase 4 stays the answer when you need a shell.

**Dependencies**: Phases 1-5 complete; some real usage to validate the SSH path before adding a second one.

**Validation**: kill sshd (`sudo systemctl stop ssh`), confirm `herdctl forward` still works, GUI still connects.

**Wiki grounding**: `daemon-split-design.md` § IPC protocol (UDS path layout); Decision 1 (one Endpoint, multiple ALPNs) generalizes naturally.

## Risks & Mitigations

| Risk | Source | Mitigation |
|---|---|---|
| **`vendor/iroh-live`'s `Live::with_router()` doesn't expose extra-ALPN registration** | `vendor/iroh-live/iroh-live/src/live.rs:160-170` builder hides the `RouterBuilder` | Phase 1 builds the `Router` by hand in the daemon (Decision 1 option B) and skips `with_router()`. Upstream patch is a follow-up, not a blocker. |
| **iroh API drift: 0.98 may rename `remote_node_id()` or `accept_bi()`** | gap research note: "API surface is similar but not identical" between iroh-docs and iroh-smol-kv | Phase 1's dial-tester is the canary — it's <50 lines and exercises every API we depend on. If a method renames, we find out before writing the bridge. |
| **Allowlist misconfiguration locks the operator out** | Decision 3 fail-closed default | Daemon also accepts SSH on the LAN at `127.0.0.1:22`; if the operator can ssh into the box another way (during initial setup, on the same Wi-Fi), they can edit `control.toml` and `kill -HUP`. The deploy plan's Phase 1 baseline (ufw, key-only sshd) keeps that fallback usable. **Document this explicitly** in Phase 5. |
| **Iroh relay latency on rural 4G translates into SSH session lag** | `[[android-on-drone]]` § 4G latency (cited in deploy plan Risk #2) | Inherited risk; SSH is interactive but tolerates 200-300 ms RTT acceptably. `ssh -o ServerAliveInterval=30` keeps half-open sessions from dying silently. We are not making this worse than the existing data-plane path. |
| **dumbpipe-style "anyone with the NodeId can dial" if allowlist parsing silently fails** | gap research: dumbpipe has no allowlist | Phase 2 fail-closed-on-empty + log loud on parse error. Add a startup self-test: daemon prints `control plane: N allowed peers` at INFO so misconfigs are visible. |
| **Bigger attack surface: a third ALPN means a third potential parsing bug** | general | Decision 2 — handler is a byte pump, no parsing. The only daemon-side parser we add is for `control.toml` (serde, hand-checked schema). sshd is the actual SSH protocol implementation; we don't reimplement it. |
| **Rotating the daemon NodeId rotates pairing too** | Decision 5 (single Endpoint) | Acceptable — these are the same physical box's identity. If we ever need to rotate one without the other, that's a justifying use case for splitting Endpoints later. |
| **`MAX_CONTROL_SESSIONS = 16` cap can be reached by a misbehaving client opening sessions in a loop** | Phase 3 | The cap is on accepted-and-bridged sessions, not pre-allowlist dials. Pre-allowlist dials are O(1) hashset rejects. A friendly DOS from an *allowed* peer is in scope for the operator's own monitoring; bad actor in the allowlist is already privileged. |
| **`herdctl` binary size is too big to distribute easily** | iroh's deps | Acceptable for now (tens of MB). If it becomes friction, build a `--no-default-features` profile that strips relay support; but document that without relay, both peers must be on the same network. |
| **Future v2 ALPN migration breaks old clients** | Decision 6 | Daemon supports both `ssh/1` and `ssh/2` for one release. herdctl pins the version it speaks. Same lifecycle pattern that iroh itself uses internally. |

## Open Questions

- **Does iroh 0.98 (the iroh-smol-kv branch) expose `connection.remote_node_id()` directly, or is it via a session-info accessor?** Phase 1's dial-tester surfaces this in the first hour of work. If the API is different, the allowlist check moves to wherever the NodeId actually lives — but the design doesn't change.
- **Is there a generic `RouterBuilder` we can fork into during `with_router()` construction?** If yes, the upstream-friendly patch is small. If no, we hand-build the Router. Confirm during Phase 1.
- **`SIGHUP` reload ordering vs in-flight sessions**: a peer that was allowed at connection time but is removed mid-session — kept until disconnect, or forcibly closed? **Default**: keep until natural disconnect. Add a `herdctl-admin kick <node-id>` later if needed (out of scope).
- **Should `control.toml` live in `/etc/herd-scout/control.toml` for systemd-managed installs**, instead of the user's `~/.config`? The deploy plan creates a dedicated `herdscout` user, so `~/.config/herd-scout/control.toml` for that user is fine. Phase 5's installer points at `$HOME/.config/herd-scout/` which resolves to that user's home. Confirm during Phase 5.
- **Multi-laptop fleets**: if we deploy the daemon to two laptops (failover or load-split), the operator needs two `Host` entries in `~/.ssh/config`. That's the static-config UX they chose. If fleets grow past ~5 boxes, revisit "Discover via iroh-gossip topic" from the discovery interview question.
- **Phone as a control client**: the operator's Pixel is in the data-plane allowlist (it's a herd-scout publisher). Do we want it to also be a control-plane client? Probably not for shell access (no good keyboard), but `herdctl ping` from a phone-based ops app is a nice "is the laptop reachable?" check. Out of MVP scope; the architecture supports it.

## Sources Consulted

| Source | What was drawn from it |
|---|---|
| `[[iroh-sync-stack]]` | One iroh node provides the data plane today; extending to control plane on the same Endpoint is consistent with that single-node architecture. Note: iroh-smol-kv is a leaner fork; verify exact API names. |
| `[[mobile-desktop-architecture]]` | "Desktop is just another peer" — the daemon already speaks iroh, adding a control ALPN is the same shape, not a new pattern. Anti-pattern: don't trust device wallclocks → fail-closed on missing/bad config. |
| `[[herd-scout-positioning]]` | Wedge #2 (P2P / no central server) is the strategic justification: if our pitch is "no central server," our ops shouldn't depend on DNS or port-forwarding. |
| `.wiki/output/plan-deploy-daemon-on-1060-laptop-2026-05-22.md` | Decision 6 (SSH UDS forward) is what this plan replaces. Phase 1 baseline (key-only sshd, ufw, dedicated `herdscout` user) is what we lean on. |
| `herd-scout-daemon/docs/daemon-split-design.md` | Daemon already holds `Live` for its lifetime; persistent identity via `directories::ProjectDirs`; IPC over UDS pattern at `~/.local/share/herd-scout/daemon.sock`. The control plane mirrors that design. |
| `vendor/iroh-live/iroh-live/src/live.rs:160-200` | Confirms `Router::accept(alpn, handler)` is the extension point and that today only moq + gossip ALPNs are registered. |
| `https://github.com/n0-computer/dumbpipe` (gap research) | Reference implementation of QUIC-byte-pump-over-iroh; has `connect-tcp`/`listen-tcp`/`connect-unix`/`listen-unix`. Confirmed it lacks NodeId allowlist — that gap is exactly what justifies building this in-tree instead of depending on dumbpipe. |
| `https://docs.rs/iroh/latest/iroh/protocol/` (gap research) | `ProtocolHandler::accept(connection)` shape, `Router::builder(endpoint).accept(alpn, handler).spawn()` registration pattern. |

## Proposed Inventory Records (suggested, not auto-created)

If this plan kicks off a durable work queue, the following inventory items would be tracked. Sample (3 rows) before creating:

| ID | Type | Description | Status |
|---|---|---|---|
| `q1` | open-question | Verify `iroh = "0.98"` (iroh-smol-kv branch) exposes `connection.remote_node_id()` and a `RouterBuilder` we can extend; locks Phase 1 option (a) vs (b). | open |
| `t1` | task | After Phase 4 ships, annotate `plan-deploy-daemon-on-1060-laptop-2026-05-22` Decision 6 with a forward-pointer to this plan. | blocked-on Phase 5 |
| `w1` | watch-item | n0-computer dumbpipe issue tracker for "peer authorization" / "node-id allowlist" features — if upstream lands one, revisit the in-tree handler vs. depending on dumbpipe. | watching |

Tell me if you want these created.
