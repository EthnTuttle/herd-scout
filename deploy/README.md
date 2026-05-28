# herd-scout-daemon — deploy notes

This dir holds the systemd units, env files, and CV sidecar wrapper for
deploying `herd-scout-daemon` on a headless Linux box. The hardware
baseline (BIOS, NVIDIA driver, ufw/fail2ban, throttled, power cap) is in
the hub wiki at `~/wiki/topics/gtx-1060-headless-ai-server/`. The
herd-scout-specific deploy roadmap lives at
`.wiki/output/plan-deploy-daemon-on-1060-laptop-2026-05-22.md`.

## Reaching a deployed daemon (Wave 11 — iroh-bound SSH)

The daemon registers a third ALPN (`herd-scout/ssh/1`) on its existing
iroh endpoint. Authorized peers reach the laptop's local sshd over iroh —
no DNS, no port forwarding, no Tailscale. See
`.wiki/output/plan-iroh-bound-ssh-access-daemon-2026-05-26.md` for the
full architecture; this section is the operator quickstart.

### One-time setup

**1. Daemon side — install the control config.**

```sh
mkdir -p ~/.config/herd-scout
cat > ~/.config/herd-scout/control.toml <<'EOF'
[control_plane]
# One entry per device that should reach this daemon.
# Run `herdctl whoami` on each operator machine to get its NodeId.
allowed_node_ids = [
  # "f9ed1a539ead29859b0b4fbe8c91d3418206e5068b4668388698fc18c9dc409e",  # Gary's dev Mac
]
ssh_target = "127.0.0.1:22"
EOF
```

If the file is missing or the allowlist is empty, the control plane is
**closed to all peers** (fail-closed) — you'll see
`WARN no control.toml at <path> — control plane closed to all peers` in
the journal at startup.

**2. Daemon side — print the laptop's NodeId once.**

The daemon persists its iroh `SecretKey` at
`~/.local/share/herd-scout/iroh_secret` (mode `0600`) on first launch
and reuses it across restarts. The NodeId is stable for the life of
that file — write it into peers' `~/.ssh/config` and `control.toml`
once and forget about it.

```sh
journalctl -u herd-scout-daemon | grep "iroh endpoint bound" | tail -1
# → "iroh endpoint bound, control plane up id=k51qz... allowed=N"
```

The journal logs only the short id. For the full 64-hex form (what
`~/.ssh/config HostName` and peer allowlists need), grab the persisted
secret's public half:

```sh
# On the laptop, as the daemon's user:
xxd -p -c 64 ~/.local/share/herd-scout/iroh_secret  # this is the SECRET; do not share
# Easier: pair once and capture the ticket. The daemon prints
# `herd-scout-daemon ticket: iroh-live:<base64>...` on first interactive
# launch, and the first 32 bytes of the base64 payload are the NodeId.
# In practice: run `herdctl ping <short-id>` from each operator
# machine — the daemon will accept or reject based on the allowlist,
# and the journal will log the full remote NodeId either way.
```

Operators reach out from their machines using `herdctl ping <id>`
(see step 3) — that exchanges enough information for the operator
journal/config to lock in. The short form printed at startup is also
unambiguous as long as you have a unique-prefix match.

**Rotating the daemon's identity** (e.g. after a key compromise): stop
the daemon, `rm ~/.local/share/herd-scout/iroh_secret`, restart. A
fresh secret is generated and persisted. Every peer that knew the old
NodeId must update its config — this is the trade-off for stable
identity.

**3. Operator side — install `herdctl` and register your identity.**

```sh
cd ~/repos/herd-scout
cargo install --path herdctl
herdctl whoami
# → f9ed1a539ead29859b0b4fbe8c91d3418206e5068b4668388698fc18c9dc409e
```

The first run generates a persisted ed25519 key at
`$XDG_CONFIG_HOME/herdctl/secret.key` (mode `0600`). Subsequent runs
reuse it — that's the NodeId the daemon allowlists.

Paste that NodeId into the laptop's `~/.config/herd-scout/control.toml`,
then send the daemon a SIGHUP so it reloads without restart:

```sh
# On the laptop:
sudo systemctl kill --signal=HUP herd-scout-daemon
# Or, if running interactively:
kill -HUP $(pgrep herd-scout-daemon)
```

The daemon journal will show
`INFO control: reload OK allowed=N`.

**4. Operator side — wire `~/.ssh/config`.**

```
Host herd-scout-laptop
  HostName <LAPTOP_NODE_ID>          # 64-hex iroh EndpointId, NOT a hostname
  User herdscout
  ProxyCommand herdctl proxy %h
  ServerAliveInterval 30
```

Then everything works:

```sh
ssh herd-scout-laptop                                                                  # interactive shell
scp file.bin herd-scout-laptop:~/                                                      # copy a binary
ssh -L /tmp/herd-scout.sock:/home/herdscout/.local/share/herd-scout/daemon.sock \      # forward the daemon UDS
    herd-scout-laptop -fN
HERD_SCOUT_SOCKET=/tmp/herd-scout.sock cargo run -p herd-scout-gui                      # GUI sees the remote daemon
```

OpenSSH 8.0+ on both ends is required for UDS forwarding. The daemon's
own auth is the iroh allowlist; sshd's `~herdscout/.ssh/authorized_keys`
handles user auth on top, exactly as it would on a normal laptop.

### Health checks

```sh
herdctl ping <NODE_ID>
# → "ok" on success (allowlist accepted us)
# → non-zero exit on dial failure
```

`ping` opens a control-plane bi-stream and immediately closes. If the
daemon's allowlist rejects us, `open_bi()` fails — that's the canary.
Cleaner than ssh-banner sniffing, no sshd dependency.

### Adding / removing peers

Two paths:

1. **Hand-edit + SIGHUP** (Wave 11). Edit `~/.config/herd-scout/control.toml`,
   send `SIGHUP`. No restart needed. In-flight sessions stay alive until
   they disconnect; new dials see the updated allowlist.
2. **Android admin app** (Wave 12). Mutate the allowlist remotely from
   a phone. See the *Wave 12* section below for the bootstrap flow,
   identity backup procedure, and audit-log inspection.

### Threat model (one paragraph)

The control plane has two gates: (a) the iroh `Endpoint` will only
respond to peers holding our NodeId, (b) the `ProtocolHandler` drops any
QUIC connection from a NodeId not in the allowlist *before* opening any
bi-stream. Unauthorized peers never see a single sshd byte; they see a
QUIC connection that closes with no `accept_bi()`. Authorized peers then
go through sshd's normal user-auth path. The daemon does **not** parse
SSH protocol — it's a byte pump.

If the laptop is on a LAN you can still ssh in directly to
`127.0.0.1:22` via that LAN; that's your fallback if you mis-configure
the allowlist and lock yourself out of the iroh path. Keep ufw set to
allow SSH on the LAN side per
`.wiki/output/plan-deploy-daemon-on-1060-laptop-2026-05-22` Phase 1.

### Concurrent session cap

The handler caps simultaneous bridges at 16
(`herd-scout-daemon/src/control/handler.rs:27`). Beyond that, additional
dials log
`WARN control: max sessions reached, dropping dial` and are rejected.
Plenty for "one user, many tabs"; not enough for a runaway client. sshd's
own `MaxStartups` is a second line.

## Wave 12 — Android admin app

The `herd-scout-admin` APK manages the SSH allowlist over a fourth
ALPN (`herd-scout/admin/1`) on the daemon's existing iroh Router. The
streaming app (`com.herdscout.app`) is unchanged; admin is a separate
APK with its own `applicationId` and launcher icon.

Architecture decisions and the full plan live at
`.wiki/output/plan-android-admin-allowlist-app-2026-05-27.md`. The two
allowlists are intentionally orthogonal:

- `[control_plane.allowed]` — peers that may dial `CONTROL_ALPN` and
  reach local sshd (Wave 11).
- `[control_plane.admins]` — peers that may dial `ADMIN_ALPN` and
  mutate the allowlist (Wave 12).

A peer with shell access does **not** automatically gain config-mutation
rights. The daemon refuses any admin RPC that would leave `admins.len() == 0`
(error `would_orphan_daemon`) so self-retraction can't lock you out.

### Granting admin rights to the first phone

1. On the phone, sideload `herd-scout-admin.apk` (built via
   `./gradlew :admin:assembleDebug`). Open it; the **My Identity** tab
   shows a QR + the local NodeId in plain text.
2. On the laptop, `ssh herd-scout-laptop` (Wave 11 path). Edit
   `~/.config/herd-scout/control.toml` and add the phone's NodeId
   under a new `admins` array:

   ```toml
   [control_plane]
   admins = ["<PHONE_NODE_ID>"]

   [[control_plane.allowed]]
   node_id = "<DEV_MAC_NODE_ID>"
   label = "dev mac"
   ```

3. `sudo systemctl kill --signal=HUP herd-scout-daemon` — the journal
   shows `INFO control: reload OK allowed=N admins=1`.
4. On the phone, tap the daemon-switcher chip in the top bar →
   *Add daemon* → paste the laptop's NodeId. The Status header should
   turn green within 3-5 s.

From here on, you can add new SSH peers from the phone — no more
laptop ssh required.

### Backing up the admin identity

Identities live in a versioned TOML envelope (`identity.toml`,
`schema_version = 1`) with an integrity-checked `node_id` field. The
phone admin app exposes Export / Import via Android's Storage Access
Framework — Drive, Files, USB, all work.

1. **My Identity** tab → **Export identity…** → choose a save
   location. The file is plain text — anyone with it can act as you.
2. To restore on a fresh phone: install the APK → **Import
   identity…** → pick the saved file. The NodeId is preserved; existing
   daemons recognize you immediately.

The same envelope format is used by the daemon
(`<config_dir>/herd-scout/identity.toml`) and `herdctl`
(`<config_dir>/herdctl/identity.toml`). Legacy raw-32-byte and 64-hex
files are auto-migrated in place on the first run.

### Reading the audit log

The daemon writes append-only JSONL at
`<data_dir>/herd-scout/audit.log` (mode 0600). Records cover SSH bridge
open / close, admin RPCs, gate rejections, config reloads, and daemon
boot. Daily rotation produces `audit-YYYY-MM-DD.log`; 90-day retention
sweep on the same task.

```sh
# Live tail with pretty-printed JSON:
tail -F ~/.local/share/herd-scout/audit.log | jq .

# Forensic queries:
jq 'select(.kind == "ssh_session_open")' \
   ~/.local/share/herd-scout/audit*.log
jq 'select(.kind == "admin_add_allowed")' \
   ~/.local/share/herd-scout/audit*.log
```

The phone's **History** tab has two sub-tabs:

- **From this device** — Room SQLite of every RPC this phone
  attempted, including failures that never reached the daemon. Always
  available, even offline.
- **From daemon** — paginated `TailAudit` RPC. Shows what the daemon
  recorded, including SSH sessions and config reloads triggered by
  *other* operators. Falls back to a Room-cached replay when the
  daemon is unreachable.

The two views diverge on purpose; the union covers both directions of
partial failure.

### Fleet mode

Up to 10 saved daemons in the phone's `SharedPreferences`. Exactly one
iroh `Connection` is open at a time — switching daemons closes the
prior session and dials the new one. No multiplexing.

## systemd units

- `systemd/herd-scout-daemon.service` — the daemon itself.
- `systemd/herd-scout-cv-sidecar.service` — the Python YOLO sidecar
  (Wave 6.5 pivot — see `.wiki/output/plan-optimize-cv-sidecar-trt-yolo11s-2026-05-26.md`).
- `herd-scout-daemon.env`, `herd-scout-cv-sidecar.env` — environment
  files referenced by the unit files.

Install:

```sh
sudo ln -s "$PWD/systemd/herd-scout-daemon.service" /etc/systemd/system/
sudo ln -s "$PWD/systemd/herd-scout-cv-sidecar.service" /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now herd-scout-cv-sidecar herd-scout-daemon
```
