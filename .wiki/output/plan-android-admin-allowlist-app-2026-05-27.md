---
title: "Plan: Android admin app for daemon NodeId allowlist management"
type: plan
format: roadmap
generated: 2026-05-27
sources:
  # Project-local wiki
  - .wiki/wiki/concepts/iroh-sync-stack.md
  - .wiki/wiki/concepts/mobile-desktop-architecture.md
  - .wiki/wiki/concepts/herd-scout-positioning.md
  - .wiki/output/plan-iroh-bound-ssh-access-daemon-2026-05-26.md
  - .wiki/output/plan-deploy-daemon-on-1060-laptop-2026-05-22.md
  - herd-scout-daemon/docs/daemon-split-design.md
  # Code referenced
  - herd-scout-daemon/src/control.rs
  - herd-scout-daemon/src/control/config.rs
  - herd-scout-daemon/src/control/handler.rs
  - herd-scout-daemon/src/main.rs
  - herd-scout-ipc/src/lib.rs
  - herdctl/src/main.rs
  - android-jni/src/lib.rs
  - android/app/src/main/java/com/herdscout/app/HerdScoutJni.kt
  - android/app/src/main/java/com/herdscout/app/MainActivity.kt
  - deploy/README.md
---

# Plan: Android admin app for daemon NodeId allowlist management

> Generated from the herd-scout local wiki (5 articles + 2 design docs) and the existing Wave 11 control-plane implementation. Builds directly on `plan-iroh-bound-ssh-access-daemon-2026-05-26` — that plan put a NodeId allowlist behind a hand-edited `control.toml` reloaded via SIGHUP. This plan moves the read/write surface out of the filesystem and onto a fourth ALPN, then ships a separate Android APK that speaks it.

## Executive Summary

Add a fourth ALPN `herd-scout/admin/1` to the daemon's existing iroh `Router`. The handler accepts an authenticated peer (a separate `[control_plane.admins]` allowlist), reads/writes `control.toml` atomically, appends every event to a JSONL audit log, and re-uses the existing `ArcSwap<ControlConfig>` so live admin mutations take effect with the same semantics as a SIGHUP reload. Wire RPCs use the same 4-byte-BE-length-prefix + JSON framing the daemon↔GUI IPC already uses (`herd-scout-ipc`), with a small `AdminMsg` enum living in that crate. On the phone, ship a **separate APK** (`herd-scout-admin`, distinct Gradle module + applicationId) that links the existing `herd-scout-jni` with a new admin client surface — a Compose UI with three first-class concerns: a list/add/remove screen for allowlist entries, a History tab fed by both a local Room SQLite and the daemon's `TailAudit` RPC, and a Daemon-switcher in the top bar that tears the active iroh connection down and dials the next saved daemon (single active connection, never multiplexed). Identities are persisted on both daemon and phone in a versioned TOML envelope that's safe to export/import across reinstalls.

This is on-thesis: wedge #2 of `[[herd-scout-positioning]]` is "P2P / no central server"; admin should not require the operator to ssh into the laptop just to grant another phone access. `[[mobile-desktop-architecture]]` already says "desktop is just another peer" — the phone-as-admin is the same shape, accreting one more ALPN onto the single iroh Endpoint per `[[iroh-sync-stack]]`'s "one Endpoint, many ALPNs" pattern that Wave 11 ratified.

Key novelty vs Wave 11: (a) admin authorization is a **separate** allowlist (`[control_plane.admins]`) so a peer with shell access doesn't automatically get config-mutation rights, (b) writes are atomic temp-file-rename and then trigger an in-process reload (no SIGHUP needed, the daemon is the writer), (c) every mutation and SSH-bridge event is appended to a versioned JSONL audit log readable via a `TailAudit` RPC, (d) the phone keeps its own complementary local audit log so "what did I do" doesn't depend on a working network, (e) fleet mode is intentionally simple — at most one iroh `Connection` open at a time, switch = teardown + reconnect, (f) identities live in a versioned `identity.toml` envelope on both ends so a reinstall is recoverable from a single backup file.

## User Requirements (from interview + refinement)

- **Auth model**: separate admin allowlist in `control.toml` (`[control_plane.admins]`). Bootstrap is the same hand-edit-and-SIGHUP flow as today — once the first phone NodeId is in `admins`, subsequent admins can be added from that phone.
- **v1 operations**: `ListAllowed`, `AddAllowed { node_id, label }`, `RemoveAllowed { node_id }`, `Status` (daemon version, own NodeId, active sessions, last reload, identity schema version), `TailAudit { last_n, before_ts_ms? }`.
- **Storage**: same `control.toml`, atomic rewrite via temp + rename, comments dropped on RPC mutation. File becomes machine-managed; operator hand-edits still work but lose comments on the next admin write.
- **Distribution**: separate APK / Gradle build flavor (`herd-scout-admin`, applicationId `com.herdscout.admin`). The streaming app (`com.herdscout.app`) is unchanged. Both link `herd-scout-jni` and share the iroh runtime initialization, but the admin APK is the only one that calls the new admin client surface.
- **Audit log on both ends**: daemon writes JSONL at `<data_dir>/herd-scout/audit.log` covering admin RPCs, SSH bridge open/close, and config reloads. Phone keeps a local Room SQLite of every action it initiated. The two are complementary — the phone displays a "From this device" view from local SQLite and a "From daemon" view from `TailAudit`.
- **Fleet mode**: phone keeps a list of saved daemons in `SharedPreferences`. Exactly one iroh `Connection` is open at any moment; switching daemons drops the active connection and dials the new one. No multiplexing.
- **Identity backup/restore**: both daemon and phone persist their iroh secret in a versioned TOML envelope (`identity.toml`) with a `schema_version`, an embedded `node_id` for integrity-check, and human-readable metadata. The phone admin app exposes Export/Import via Android's storage-access framework so a reinstall is recoverable from a single saved file.

## Architecture Decisions

### Decision 1 — A fourth ALPN on the same iroh Router (mirror Wave 11)

**Context**: `herd-scout-daemon/src/main.rs:90-93` already builds the `Router` by hand and calls `.accept(herd_scout_ipc::CONTROL_ALPN, control_handler)`. Adding admin is the same one-liner. `[[iroh-sync-stack]]` and `[[mobile-desktop-architecture]]` both anchor on a single Endpoint per device. Spinning a second Endpoint just for admin would split the operator's mental model and double the relay chatter for no benefit, exactly as `plan-iroh-bound-ssh-access-daemon-2026-05-26` Decision 1 argued.

**Options considered**:
- **A. New ALPN on the existing Router.** Same Endpoint, same NodeId, one more `.accept(...)` call.
- **B. Reuse the SSH ALPN with an in-band "admin" subprotocol.** Tunnel admin RPCs over the same byte-pump that bridges to sshd. But the SSH handler is by Wave 11 Decision 2 a deliberate dumb byte pump — adding a parsing pre-amble re-introduces the "every parser is an attack-surface bug" risk it was designed to avoid.
- **C. Run admin over the existing daemon↔GUI IPC.** Localhost-only by design; doesn't reach the phone. Out.

**Decision**: Option A. Define `pub const ADMIN_ALPN: &[u8] = b"herd-scout/admin/1";` in `herd-scout-ipc`. Register on the daemon's `Router` next to `CONTROL_ALPN`.

**Consequences**: one more `accept(...)` call in `main.rs`. The admin handler is a separate `ProtocolHandler` impl in `herd-scout-daemon/src/admin/`. Versioned ALPN gives us `admin/2` later if we change framing. ~3 lines of wiring at the call site; ~250 lines of handler.

### Decision 2 — Wire format = the existing JSON-over-bi-stream framing

**Context**: `herd-scout-ipc/src/lib.rs` already defines a `ServerMsg`/`ClientMsg` enum framed as 4-byte big-endian length + JSON, used by daemon↔GUI on Unix sockets. The daemon-split-design.md § IPC protocol calls out that JSON is "small, debuggable, fine at 30 control msgs/sec." Admin RPCs are even slower (~1/min, human-driven).

**Decision**: define a parallel `AdminClientMsg` / `AdminServerMsg` enum in `herd-scout-ipc`, same `serde_json` + 4-byte-BE-length framing. Single bi-stream per session: client sends a `Cmd` message, server replies with a `Reply` message, both halves close. No multiplexing in v1 (admin volume doesn't justify the complexity).

**Consequences**: zero new framing code on the daemon — reuse the existing length-prefixed reader/writer helpers (`herd-scout-daemon/src/ipc/frame.rs` per the design doc). Phone side gets the same framing implemented in Rust inside `herd-scout-jni` (existing tokio runtime is already there). Kotlin never speaks the wire format — it's all behind JNI.

### Decision 3 — Admin allowlist is a *separate* set in the same `control.toml`

**Context**: User picked "Separate admin allowlist in control.toml." The Wave 11 SSH allowlist (`[control_plane].allowed_node_ids`) authorizes shell access. Mixing admin authorization into the same set means anyone who can ssh in can also rewrite the allowlist — a privilege escalation the user explicitly wanted to avoid.

**Decision**: extend `RawSection` in `herd-scout-daemon/src/control/config.rs` with:

```toml
[control_plane]
allowed_node_ids = [
  "f9ed1a5...",  # Gary's dev Mac
]
admins = [
  "9b0b4fb...",  # Gary's Pixel
]
ssh_target = "127.0.0.1:22"
```

`ControlConfig` grows an `admins: HashSet<EndpointId>` field. Default empty (fail-closed: no admins = no admin RPCs accepted). Bootstrap = hand-edit + SIGHUP, same as the SSH allowlist's bootstrap.

**Consequences**: zero migration cost (new optional field). Operators who only want SSH never see admin. The admin handler reads `cfg.admins` on each accept, lock-free via `ArcSwap` exactly like Wave 11's handler reads `cfg.allowed_node_ids`. Self-dial is rejected the same way.

### Decision 4 — Daemon owns writes; atomic temp-file rename; comments dropped

**Context**: User picked "Same control.toml, atomic rewrite, comments dropped." `control.toml` becomes machine-managed; operator hand-edits still work but the next RPC mutation will rewrite the file as plain `serde::Serialize` output without comments.

**Decision**: writes go through `fn rewrite_atomically(path, &ControlConfig) -> Result<()>`:
1. Serialize via `toml::to_string_pretty`.
2. Write to `<path>.tmp` (mode `0600`).
3. `fsync` the temp file.
4. `rename(<path>.tmp, <path>)` — atomic on POSIX.
5. After the rename, store the new `ControlConfig` into the in-process `ArcSwap`. **Skip re-reading the file** — we already have the parsed value; reloading would be redundant and races a parallel SIGHUP.

`deploy/README.md` § "Reaching a deployed daemon" gets a one-paragraph addition: "Once you have an admin device set up, prefer the admin app for further changes — `control.toml` will be rewritten without comments on the next admin mutation."

**Consequences**: hand-edits and admin RPC mutations are mutually exclusive in practice (last writer wins). No file-watcher needed; SIGHUP still works for the operator-edit path. Comment loss is a one-time event when an admin first writes; subsequent admin writes are idempotent.

### Decision 5 — Separate APK / Gradle module for the admin app

**Context**: User picked "Separate APK / build flavor." The streaming app (`com.herdscout.app`) is what runs on a phone strapped to a drone or tucked in a chute-side pocket — it's a publish-only role. Admin is a sysadmin's-tablet-at-the-kitchen-table role. Mixing them risks (a) shipping admin code in every drone install, (b) needing to gate the admin UI behind some long-press easter egg per `[[herd-scout-positioning]]`'s "native mobile ranch UX" which should be focused on the field task at hand.

**Decision**: introduce a new Gradle module `android/admin/` (sibling of `android/app/`). Both modules depend on the same `herd-scout-jni` cdylib. The admin module declares `applicationId "com.herdscout.admin"` and a different launcher icon. Shared Kotlin (e.g. NodeId display formatting, the QR scan flow) factors into a tiny `android/shared/` library module. The streaming module is **not** modified beyond moving QR-scan code to the shared module.

The new admin module ships:
- A single-screen Compose UI (`AdminActivity.kt`) with a top "Daemon status" header and a list of allowlist entries below.
- "Add" button → choose: "Scan QR" (peer's NodeId from a `herdctl whoami` QR) or "Paste NodeId."
- Long-press an entry → "Remove" confirmation.
- Connection state header showing daemon ALPN reachability (green/yellow/red) and last refresh time.

**Consequences**: two APKs to build, two Play-Store listings if we ever publish (we won't initially — both are sideloaded). Shared module avoids forking QR-scan and TopEdge code. Streaming app's APK stays small; admin app's APK stays narrow in capability.

### Decision 6 — The admin client lives in `herd-scout-jni`, exposed via Kotlin facade

**Context**: `android-jni/src/lib.rs` already brings up a tokio runtime, an iroh `Endpoint`, and the `Live` stack on the phone. Reusing that runtime for admin RPCs costs almost nothing. Writing an iroh client in Kotlin (via UniFFI or hand-rolled) duplicates work and ships a second iroh into the APK.

**Decision**: extend `herd-scout-jni` with admin-client functions:
- `nativeAdminConnect(daemonNodeId: String): Long` — opens an iroh connection, returns an opaque session handle.
- `nativeAdminListAllowed(handle: Long): String` — JSON array of `{node_id, label, added_at}`.
- `nativeAdminAddAllowed(handle: Long, nodeId: String, label: String): String` — JSON status reply.
- `nativeAdminRemoveAllowed(handle: Long, nodeId: String): String` — JSON status reply.
- `nativeAdminStatus(handle: Long): String` — daemon version, own NodeId, active sessions, last reload.
- `nativeAdminDisconnect(handle: Long)` — close.

Behind the JNI: a small `admin_client` module that opens a bi-stream on `ADMIN_ALPN`, writes the framed `AdminClientMsg`, reads the framed `AdminServerMsg`, returns the JSON. One round-trip per call; the connection is kept alive across calls so the QUIC handshake amortizes.

**Consequences**: ~120 lines of Rust in `android-jni/src/admin_client.rs`. Kotlin facade is a thin object similar to `HerdScoutJni`. Kotlin parses the JSON replies (kotlinx.serialization) — that's a deliberate boundary so the wire enum can evolve in Rust without re-generating UniFFI bindings.

### Decision 7 — Versioned `identity.toml` envelope, shared crate, used by daemon and phone

**Context**: Today the daemon's iroh secret is hidden inside `iroh-live`'s persistence and the phone has no persisted identity at all. For an admin app whose NodeId must be in `control.toml.admins` *forever*, ephemeral secrets are unworkable — and a raw 32-byte key file (the shape `herdctl/src/main.rs:62-91` uses) has no version field, no integrity check, and no way to recover from a corrupted read. `[[iroh-docs-fms-schema]]` § Schema evolution rule 3 is explicit: "schema version per entity; reader does on-the-fly upgrade in memory." Apply the same rule to the identity file.

**Decision**: introduce a tiny shared crate `herd-scout-identity` (depended on by the daemon, `herdctl`, and `herd-scout-jni`). It owns one struct, one parse function, one write function, and a `schema_version: u32` constant.

```toml
# identity.toml — schema 1
schema_version = 1

[identity]
secret_key  = "base64url(32 bytes)"   # the only secret material
node_id     = "f9ed1a539ead..."        # MUST match SecretKey::public(); integrity gate
created_at  = "2026-05-27T10:30:00Z"  # ISO-8601, informational only
label       = "Gary's Pixel"           # human-readable, optional

[origin]
device      = "android"                # one of: android | linux | macos | unknown
app_version = "0.2.0"                  # CARGO_PKG_VERSION at write time
```

The struct:

```rust
// herd-scout-identity/src/lib.rs
pub const SCHEMA_VERSION: u32 = 1;

pub struct Identity { pub secret: SecretKey, pub label: String, pub created_at: String }

pub fn load(path: &Path) -> Result<Identity, IdentityError> { ... }
pub fn save(path: &Path, id: &Identity, label: &str) -> Result<()> { ... }
pub fn parse_envelope(s: &str) -> Result<Identity, IdentityError> { ... }
pub fn render_envelope(id: &Identity, label: &str) -> String { ... }
```

`load` enforces three invariants in order: (a) `schema_version` is recognized — unknown future versions return `IdentityError::UnsupportedSchema { found, max_supported: SCHEMA_VERSION }` and the caller refuses to start; (b) `secret_key` is exactly 32 bytes after base64url-decode; (c) the `node_id` field matches `SecretKey::public().to_string()` — mismatch returns `IdentityError::IntegrityCheckFailed` and the file is treated as corrupt rather than silently producing a different NodeId than the user expects. Past schema versions get an in-memory upgrade path (none exist yet at v1; rule is "always read all known schemas, always write the latest").

The same crate exposes `pub fn import_from_user_blob(s: &str) -> Result<Identity>` for the phone's "Import" flow — accepts the file's text content from any source (Android Document Picker, paste box, scanned QR if it ever fits), runs the same validation. Symmetric `export_to_user_blob(&Identity, label) -> String` writes the envelope including the `[origin]` block stamped at export time.

Both `daemon` and `herdctl` migrate their existing key persistence to `herd-scout-identity::load_or_generate(&path)` — a one-time on-disk migration that detects the legacy 32-raw-bytes format, wraps it in the v1 envelope, writes it back atomically. The legacy reader is kept under `#[deprecated]` for one release.

**File locations**:
- Daemon: `<config_dir>/herd-scout/identity.toml` (new). Old raw key file at `<data_dir>/.../secret.key` is auto-upgraded.
- `herdctl`: `<config_dir>/herdctl/identity.toml`. Old `secret.key` auto-upgraded.
- Phone (admin app): `Context.filesDir / "identity.toml"`. Generated on first launch.
- Phone (streaming app): unchanged from current behavior — keeps generating ephemeral secrets per `Live::from_env()` boot. The streaming app does *not* read the admin app's identity (separate apps, separate sandboxed `filesDir`s by Android's UID isolation).

**Consequences**: identity is now portable. A reinstall on the phone restores admin access from a single file in Google Drive / iCloud / a USB stick. A failed read is *loud* (integrity check) rather than silent (NodeId mismatch). Future schema bumps follow `[[iroh-docs-fms-schema]]`'s "new keys, never repurpose, version per entity" pattern. Cost: ~150 lines in the new crate, plus the one-time legacy migration code.

### Decision 8 — Daemon-side audit log: append-only JSONL, versioned record, daily rotation

**Context**: The user wants an audit log on both ends. On the daemon side this needs to survive restarts, be cheap to append at SSH-bridge open/close rates, and be machine-readable so the phone's `TailAudit` RPC can stream lines without re-parsing the whole file. `[[iroh-docs-fms-schema]]` § Conflict resolution strategy 3 ("append-only with derived state") and the same article's anti-pattern note that "LWW is unacceptable for medical / movement events" frame the principle: anything that has compliance value should be append-only. Audit logs absolutely qualify.

**Decision**: append-only JSONL at `<data_dir>/herd-scout/audit.log`. One JSON object per line. Every record carries `{ schema_version: 1, ts_ms, kind, actor: { node_id, label? }, ... }` so future readers can branch on `schema_version`. Record kinds for v1:

| `kind`                | Extra fields                                                     | Trigger                                            |
| --------------------- | ---------------------------------------------------------------- | -------------------------------------------------- |
| `admin_add_allowed`   | `target_node_id`, `target_label`                                 | `AdminClientMsg::AddAllowed` succeeds              |
| `admin_remove_allowed`| `target_node_id`, `target_label_was`                             | `AdminClientMsg::RemoveAllowed` succeeds           |
| `admin_rejected`      | `reason` ("not_in_admins" \| "self_dial")                        | Admin handler drops a connection at the gate      |
| `admin_error`         | `op`, `code`, `message`                                          | An admin RPC returned `AdminServerMsg::Error`     |
| `ssh_session_open`    | `session_id`                                                     | `ControlHandler` admits a peer past the allowlist |
| `ssh_session_close`   | `session_id`, `bytes_to_sshd`, `bytes_from_sshd`, `duration_ms`  | `SessionGuard` drops                              |
| `ssh_rejected`        | `reason` ("not_in_allowed" \| "self_dial" \| "max_sessions")     | `ControlHandler` drops a connection at the gate   |
| `config_reload`       | `source` ("sighup" \| "admin_rpc"), `allowed_count`, `admins_count` | After `ArcSwap::store`                          |
| `daemon_boot`         | `version`, `own_node_id`, `allowed_count`, `admins_count`        | First record after process start                  |

Append path: `OpenOptions::new().create(true).append(true).mode(0o600).open(path)` once at boot, kept as `Arc<tokio::sync::Mutex<tokio::fs::File>>`. Each `append(record)` serializes to a single line + `\n`, writes atomically (one `write_all` on `tokio::fs::File` is atomic for short lines under POSIX `O_APPEND`), and `flush`es. We don't `fsync` per record — the cost on rotational/SD disk is too high; we accept losing the last few seconds of records on a hard kill.

Daily rotation: a background task runs at boot and every 24h, renaming `audit.log` → `audit-YYYY-MM-DD.log` if its first record's `ts_ms` is before today's UTC midnight. Old files are kept for 90 days then deleted (config-overridable). Rotation isn't strictly necessary for v1 (the file is small), but it makes `TailAudit` cheap by capping the active file size.

**Consequences**: ~80 LOC in `herd-scout-daemon/src/audit.rs`. The handlers gain an `Arc<Audit>` parameter; each one fires-and-forgets one `audit.append(record)` per relevant event. Replay tools later can re-read the JSONL trivially with `serde_json::Deserializer::from_reader(...)`. Schema bumps just add new fields (existing readers ignore unknown fields, since serde defaults to `#[serde(deny_unknown_fields)]` only when explicitly opted-in — we will *not* opt in for audit records).

### Decision 9 — Phone-side audit log: Room SQLite, complementary to the daemon's, never synchronized

**Context**: The daemon's audit log is the source of truth for what the daemon did. But the phone has its own perspective: which RPCs *this device* attempted, including failures that never reached the daemon (network down, daemon unreachable, daemon refused us). The user wanted audit on both ends — that's two complementary views, not one synchronized view. Trying to synchronize them resurrects the centralized-server problem `[[herd-scout-positioning]]` wedge #2 explicitly avoids.

**Decision**: phone uses Room (Android's official SQLite ORM) with one entity:

```kotlin
@Entity(tableName = "audit_events")
data class AuditEvent(
    @PrimaryKey(autoGenerate = true) val id: Long = 0,
    val tsMs: Long,
    val daemonNodeId: String,        // which daemon was targeted
    val kind: String,                // "rpc_attempt" | "rpc_success" | "rpc_error" | "connect" | "disconnect"
    val op: String?,                 // "add_allowed", "list_allowed", etc.
    val targetNodeId: String?,
    val targetLabel: String?,
    val errorCode: String?,
    val errorMessage: String?,
    val schemaVersion: Int = 1,      // mirrors daemon's pattern
)
```

The DAO exposes `pagingSource(daemonNodeId)` for the History tab to render with Paging 3. Records are kept indefinitely (the phone has gigabytes; admin events are tiny); a "Clear history" gesture is offered in settings.

The History UI shows two tabs:
- **From this device** — sourced from Room, always available, fast.
- **From daemon** — sourced from `TailAudit` RPC, paginated by `before_ts_ms`. If the daemon is unreachable, this tab shows a "Last fetched <relative time>" stale-cache view (the most recent `TailAudit` reply is also written into Room with `kind = "daemon_replay"`, so we have an offline-capable cache).

**Consequences**: ~200 LOC of Kotlin (Room schema + DAO + ViewModel + Compose tab). Two views diverge over time (e.g. removed entries persist in phone history), which is the correct shape — phone is auditing *itself*, daemon is auditing *itself*, and the operator sees both.

### Decision 10 — Daemon `TailAudit` RPC: read-only, paginated, capped per call

**Context**: The phone's "From daemon" tab needs to read recent records without streaming the whole file. The daemon side already serializes JSONL line-by-line; pagination via "give me the last N records before timestamp X" is the natural shape.

**Decision**: add to `AdminClientMsg`:

```rust
TailAudit { last_n: u32, before_ts_ms: Option<u64> }
```

Reply:

```rust
AuditTail { records: Vec<AuditRecord>, eof: bool }
```

The handler reads `audit.log` from the end (mmap or `BufReader::seek_to_end` + `lines().rev()`), filters to `ts_ms < before_ts_ms` if provided, takes up to `min(last_n, 500)` records, returns them in newest-first order. `eof: true` means "no older records exist for this filter." The cap is hard-coded to 500 to bound memory and wire size; a phone wanting more pages just calls again with the oldest record's `ts_ms` as the next `before_ts_ms`.

For records spanning rotated files, the handler reads the active log first, then walks rotated files in reverse-chronological-name order until it has enough records or runs out of files.

**Consequences**: ~120 LOC in the audit-tail reader. Wire-format-wise we add one variant to each enum. The `AuditRecord` struct mirrors the daemon's internal record but uses `String` for `kind` (so the phone doesn't need an exhaustive enum match — forward-compatible by construction). Phone side renders unknown `kind` as a generic gray-bullet row.

### Decision 11 — Self-retraction is rejected when it would orphan the daemon

**Context**: An admin can `RemoveAllowed(self)` on the SSH allowlist freely (it just removes shell access for that device). But `RemoveAdmin(self)` (or any operation that drops the calling admin out of `[control_plane.admins]`) needs to refuse if it would leave the daemon with `admins = []`. An empty admins set means no further mutations are possible without ssh-ing into the laptop and hand-editing `control.toml` — exactly the chicken-and-egg the admin app exists to avoid.

**Decision**: extend the admin handler's pre-write check. For any mutation whose post-write state would have `admins.len() == 0`, return `AdminServerMsg::Error { code: "would_orphan_daemon", message: "Cannot remove the last admin; add another admin device first." }`. The check runs after building the candidate `ControlConfig` snapshot but before `write_atomic`. Self-retraction off the SSH allowlist is *always* allowed (Wave 11 SSH access is independently bootstrappable).

For v1, `[control_plane.admins]` is the only admin gate, so the rule is literally "candidate `admins.len() >= 1` or reject." If we ever add per-operation roles (e.g. `[control_plane.auditors]` with read-only access), the rule generalizes to "at least one principal must retain mutate-admins capability."

**Consequences**: ~10 LOC in the admin handler. The phone UI surfaces the error in plain language ("Cannot remove the last admin device"). Recovery from a misconfigured `admins = []` (e.g. operator hand-edits the file empty) still works via the Wave 11 SSH path — the daemon doesn't *enforce* `admins.len() >= 1` at boot, only at admin-RPC-write time. That asymmetry is intentional: boot-time enforcement would brick a daemon whose operator just happened to remove all admins via SIGHUP, which is their right.

### Decision 12 — Fleet mode: at-most-one active iroh `Connection`, switch = teardown + dial-new

**Context**: User explicitly chose "fleet mode tears down iroh connections and establishes a new one. Don't try to hold many connections open." This rejects multiplexing — which is good, because holding N idle connections per fleet device costs N relay keepalives per phone (`[[iroh-sync-stack]]` notes that relay fallback is automatic but not free) and N sets of QUIC state.

**Decision**: the phone admin app holds *exactly one* `AdminSession` at a time, keyed by a `daemonNodeId: String`. Switching daemons calls `nativeAdminDisconnect(handle)` (which `connection.close().await`s, then `endpoint.close().await`s only if no other admin call is pending), waits for the close to complete, then calls `nativeAdminConnect(newDaemonNodeId)` with a fresh handle. The Endpoint *can* be reused across reconnects (binding is expensive); only the `Connection` is torn down. Implementation-wise: the JNI keeps the `Endpoint` alive for the process lifetime in a `OnceLock<Endpoint>`, and `AdminSession::connect` just opens a new `Connection` against it.

The Daemon-switcher UI is in the top app bar: a chip showing the current daemon's short label (or NodeId-short if unlabeled), tapping opens a bottom sheet with the saved daemons, the current one marked, and an "Add daemon..." entry. Switching shows a 200-300ms spinner ("Connecting to <label>...") while the connection cycles, then re-renders the list/history for the new daemon.

Saved daemons live in `SharedPreferences` as a JSON-encoded `List<DaemonEntry { nodeId, label, lastConnectedMs }>`. Up to 10 entries (forced eviction of the oldest); we don't expect real fleets larger than that for the local-ranch user, and a hard cap avoids any "ten thousand stale daemons" pathology.

**Consequences**: zero multiplexing complexity. State management is "current daemon" + "saved daemons list" + "is-connecting flag." The History tab's "From this device" view filters by the active daemon's NodeId so switching changes the displayed history alongside the live data. ~80 LOC of Kotlin for the saved-daemons store + switcher UI; ~30 LOC of Rust to make the JNI session pool single-slot.

## Implementation Phases

### Phase 0 — Extract `herd-scout-identity` crate; migrate daemon + herdctl (estimated effort: 0.5 day)

**Goal**: every iroh-secret-bearing binary in the workspace reads/writes the v1 envelope. Existing raw-bytes files auto-upgrade in place.

**Tasks**:
- [ ] Create workspace member `herd-scout-identity/` with deps `iroh = { workspace = true }`, `serde`, `toml`, `time` (RFC3339 timestamps), `thiserror`. Implement `SCHEMA_VERSION: u32 = 1`, `Identity`, `IdentityError` (`UnsupportedSchema`, `IntegrityCheckFailed`, `Io`, `Parse`, `BadKeyLength`), `parse_envelope`, `render_envelope`, `load`, `save`, `load_or_generate(&path, label)`, `import_from_user_blob`, `export_to_user_blob`.
- [ ] Atomic write helper: same temp+rename+`fsync` pattern as Decision 4's `control.toml` rewrite, with mode `0600` on Unix.
- [ ] Legacy reader: if `path.with_file_name("secret.key")` exists and the new `identity.toml` does not, read the 32 raw bytes, derive NodeId, write the envelope, *then* `std::fs::remove_file(legacy)` only after the new file is durable on disk. Log `INFO identity: migrated legacy secret.key → identity.toml`.
- [ ] Migrate `herdctl/src/main.rs` `gen_or_load_key()` to `herd_scout_identity::load_or_generate(&path, "herdctl")`. Drop the inline `write_secret` helper.
- [ ] Migrate the daemon's iroh-secret persistence path. The daemon gets `<config_dir>/herd-scout/identity.toml`. The integration point depends on how `iroh-live::Live::from_env()` resolves its key today — verify against `vendor/iroh-live/iroh-live/src/live.rs` early; we may need to bind the secret explicitly via `Endpoint::builder(presets::N0).secret_key(id.secret).bind()` and then construct `Live` from a pre-built endpoint, mirroring the path `herdctl` already uses.
- [ ] Round-trip test in `herd-scout-identity/tests/`: write → read → assert NodeId matches. Integrity check: tamper with `secret_key` → `load` returns `IntegrityCheckFailed`. Forward-compat: a `schema_version = 99` file → `UnsupportedSchema` (caller refuses to start).

**Dependencies**: none (this stays purely local; doesn't touch ALPN code).

**Validation**: `cargo test -p herd-scout-identity`. Boot the daemon on a machine with a legacy `secret.key` → daemon journal shows the one-time migration line, `identity.toml` exists, `secret.key` is gone, NodeId is unchanged.

**Wiki grounding**: `[[iroh-docs-fms-schema]]` § Schema evolution — "schema version per entity, lazy migration, no flag day"; existing `herdctl/src/main.rs:62-91` (the pattern we're generalizing).

### Phase 1 — Wire the admin ALPN with a stub handler (estimated effort: 0.25 day)

**Goal**: Daemon accepts a connection on `b"herd-scout/admin/1"`, gates on `cfg.admins`, logs a stub message, closes. No real RPCs yet.

**Tasks**:
- [ ] In `herd-scout-ipc/src/lib.rs`, add `pub const ADMIN_ALPN: &[u8] = b"herd-scout/admin/1";` next to `CONTROL_ALPN`.
- [ ] In `herd-scout-daemon/src/control/config.rs`, extend `RawSection` and `ControlConfig` with `admins: HashSet<EndpointId>` (default empty). Update `load_or_default` to parse `parsed.control_plane.admins`. Mirror the trim/skip-empty/from_str logic that's already there for `allowed_node_ids`.
- [ ] Create `herd-scout-daemon/src/admin/mod.rs` and `herd-scout-daemon/src/admin/handler.rs`. Stub `AdminHandler` implementing `ProtocolHandler`:
  ```rust
  let remote = connection.remote_id();
  let cfg = self.cfg.load();
  if remote == self.own_node_id || !cfg.admins.contains(&remote) {
      warn!(remote = %remote.fmt_short(), "admin: dropping unauthorized dial");
      return Ok(());
  }
  info!(remote = %remote.fmt_short(), "admin: authorized dial (phase 1 stub)");
  connection.close(0u32.into(), b"phase-1-stub");
  Ok(())
  ```
- [ ] In `herd-scout-daemon/src/main.rs`, mod-declare `mod admin;`, build an `AdminHandler::new(control_cfg.clone(), own_node_id)`, and add `.accept(herd_scout_ipc::ADMIN_ALPN, admin_handler)` to the `Router::builder` chain after the existing `CONTROL_ALPN` accept call.

**Dependencies**: Wave 11 already shipped (control-plane handler exists).

**Validation**:
- Add a test admin NodeId to `~/.config/herd-scout/control.toml`'s new `admins = [...]`, SIGHUP, dial from a 30-line `cargo run --example admin-stub-dial -- <daemon-node-id>` test binary in `herd-scout-daemon/examples/`. Daemon journal shows `admin: authorized dial`.
- Dial without being in `admins` → daemon journal shows `admin: dropping unauthorized dial`, dial-tester sees a closed connection.

**Wiki grounding**: `plan-iroh-bound-ssh-access-daemon-2026-05-26` Phases 1-2 are the template — same "stub the ALPN, then add the allowlist, then add the real handler" cadence.

### Phase 2 — `AdminClientMsg` / `AdminServerMsg` enums + framed bi-stream RPC (estimated effort: 0.5 day)

**Goal**: `ListAllowed` and `Status` round-trip end-to-end. Read-only path so we don't have to think about file rewrites yet.

**Tasks**:
- [ ] In `herd-scout-ipc/src/lib.rs`, add:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(tag = "type", rename_all = "snake_case")]
  pub enum AdminClientMsg {
      ListAllowed,
      AddAllowed { node_id: String, label: String },
      RemoveAllowed { node_id: String },
      Status,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct AllowedEntry {
      pub node_id: String,
      pub label: String,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct StatusReply {
      pub daemon_version: String,
      pub own_node_id: String,
      pub active_ssh_sessions: u32,
      pub admins_count: u32,
      pub allowed_count: u32,
      pub last_reload_unix_ms: u64,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(tag = "type", rename_all = "snake_case")]
  pub enum AdminServerMsg {
      Allowed { entries: Vec<AllowedEntry> },
      Status(StatusReply),
      Ok,
      Error { code: String, message: String },
  }
  ```
- [ ] Extend `ControlConfig` to carry per-entry labels: change `allowed_node_ids: HashSet<EndpointId>` to `allowed: Vec<AllowedEntry>` plus a derived `allowed_set: HashSet<EndpointId>` rebuilt on each `load_or_default` for the O(1) admit-or-drop path that Wave 11 relies on. The TOML schema becomes:
  ```toml
  [[control_plane.allowed]]
  node_id = "f9ed1a5..."
  label = "Gary's dev Mac"
  ```
  Provide a backwards-compat fallback: if `allowed_node_ids = [...]` is present (Wave 11 schema), parse it as un-labeled entries (`label = ""`). Document the migration: the next admin-RPC write rewrites in the new shape.
- [ ] Implement `herd-scout-daemon/src/admin/handler.rs` real-handler body:
  - `connection.accept_bi().await?` → `(send, recv)`.
  - Read one length-prefixed JSON message from `recv` (cap at 64 KB to avoid memory abuse).
  - Match on the variant. For `ListAllowed`: read `self.cfg.load().allowed`, send `AdminServerMsg::Allowed { entries }`. For `Status`: gather version (`env!("CARGO_PKG_VERSION")`), `own_node_id` (already on the handler), `admins_count`, `allowed_count`, and stub `active_ssh_sessions = 0` for now (Phase 4 wires the real counter).
  - `send.finish()` and return.
- [ ] Reuse `herd-scout-daemon/src/ipc/frame.rs` for the length-prefix framing if it's generic enough; otherwise inline a 30-line `read_len_prefixed_json<T>` / `write_len_prefixed_json<T>` helper in `admin/wire.rs`.

**Dependencies**: Phase 1.

**Validation**: extend the example binary `examples/admin-stub-dial.rs` to send `ListAllowed`, parse the reply, print the entries. Add a unit test that round-trips the new types through `serde_json::to_string` → `serde_json::from_str`.

**Wiki grounding**: `daemon-split-design.md` § IPC protocol (4-byte BE length + JSON), `herd-scout-ipc/src/lib.rs` (existing tag-based serde patterns).

### Phase 3 — Atomic `control.toml` rewrite + in-process reload (estimated effort: 0.5 day)

**Goal**: `AddAllowed` and `RemoveAllowed` mutate the file safely and the SSH-allowlist accept path sees the change without SIGHUP.

**Tasks**:
- [ ] In `herd-scout-daemon/src/control/config.rs`, add `pub(crate) fn write_atomic(path: &Path, cfg: &ControlConfig) -> Result<()>`:
  1. Build a `RawFile` with `RawSection { allowed: <vec of AllowedEntry>, admins: <vec of node-id strings>, ssh_target: <option<string>> }`.
  2. `let s = toml::to_string_pretty(&raw)?;`
  3. Resolve `<path>.tmp` (same parent dir to keep the rename atomic).
  4. Write file with mode `0600` via `OpenOptions::new().write(true).create_new(true).mode(0o600)`. If `<path>.tmp` already exists from a crashed prior write, `remove_file` first (best-effort).
  5. `f.sync_all()?` then `drop(f)` then `std::fs::rename(&tmp, path)?`.
- [ ] In `admin/handler.rs`, implement `AddAllowed`:
  1. Validate `node_id` parses as `EndpointId`; if not, send `AdminServerMsg::Error { code: "invalid_node_id", ... }`.
  2. Reject empty labels (force operator to label every device — discoverability).
  3. Reject duplicates: if the parsed id is already in `cfg.allowed_set`, send `Error { code: "already_present", ... }`.
  4. Take a snapshot: `let mut new = (**cfg.load()).clone();` push the new `AllowedEntry`, rebuild `allowed_set`.
  5. Call `write_atomic(&config_path(), &new)?`. On failure, send `Error { code: "io", ... }` and *do not* swap the in-process config.
  6. On success: `self.cfg.store(Arc::new(new))`, send `AdminServerMsg::Ok`.
- [ ] `RemoveAllowed`: symmetric. `Error { code: "not_found", ... }` if the id isn't in the set. Removing your own admin device is allowed (UX disabled in the phone client; daemon doesn't care — fail-closed config makes a wedge unlikely in practice).
- [ ] Race protection: serialize all *write* RPCs through a `tokio::sync::Mutex<()>` on the handler. Reads (`ListAllowed`, `Status`) bypass it (lock-free `ArcSwap` load). Two concurrent admins both trying `AddAllowed` will serialize at the daemon and the second one will see the first's write reflected in the snapshot it clones.
- [ ] **No-orphan guard** (Decision 11): inside the write mutex, after building the candidate `ControlConfig` snapshot but before `write_atomic`, reject any write whose `candidate.admins.len() == 0`. Return `AdminServerMsg::Error { code: "would_orphan_daemon", message: "Cannot remove the last admin device. Add another admin first." }`. Self-retraction with another admin still present is allowed.
- [ ] Update `spawn_sighup_reloader` to also rebuild the `allowed_set` and to log `INFO control: SIGHUP reload OK` vs `INFO control: admin-rpc reload OK` so we can disambiguate in the journal.

**Dependencies**: Phase 2.

**Validation**:
1. From the example binary, `AddAllowed` a fresh NodeId → daemon journal shows reload, `~/.config/herd-scout/control.toml` now contains the new `[[control_plane.allowed]]` block, and a *new* SSH dial from that NodeId is accepted by the Wave 11 handler immediately (no SIGHUP).
2. Crash-test: kill -9 the daemon mid-`AddAllowed` (use a `cfg(test)` sleep injected before the rename) → `control.toml` is unchanged on next boot, `<path>.tmp` may exist as a leftover but is harmless.
3. Two concurrent example binaries each call `AddAllowed` 50 times with unique NodeIds → final file has all 100 entries, no torn writes.

**Wiki grounding**: `[[iroh-docs-fms-schema]]` § Append-only with derived state — we don't go full append-only here (the file is small and human-readable matters), but the principle that "writes that are committed must be durable before the in-memory mutation is visible" is the same.

### Phase 4 — Audit log: append-only JSONL + `TailAudit` RPC + metrics (estimated effort: 1 day)

**Goal**: every admin RPC and every SSH bridge event is durable on disk. `TailAudit` returns paginated records. `Status` reflects real active sessions and last reload time.

**Tasks**:
- [ ] Create `herd-scout-daemon/src/audit.rs`. Define `pub struct Audit { file: Arc<tokio::sync::Mutex<tokio::fs::File>>, dir: PathBuf }` and `pub struct AuditRecord { schema_version: u32, ts_ms: u64, kind: AuditKind, ... }` with the enum variants from Decision 8. Implement `Audit::open(dir: &Path)`, `Audit::append(record)`, and `Audit::tail(last_n, before_ts_ms) -> Vec<AuditRecord>`.
- [ ] Tail reader: `BufReader::new(File::open(active)).lines()` collected into a `Vec<String>`, walked back-to-front, parsed lazily, filtered by `before_ts_ms`. For older records, walk `audit-YYYY-MM-DD.log` files in the dir in reverse-chronological order. Cap at 500 records per call.
- [ ] Daily-rotation task: `tokio::spawn` a loop that wakes at the next UTC midnight (`tokio::time::sleep_until`), renames the active file if its first record is from a previous UTC day, opens a fresh handle. 90-day retention sweep at the same time.
- [ ] Lift `sessions: Arc<AtomicUsize>` from `ControlHandler` into a shared `ControlMetrics`:
  ```rust
  pub(crate) struct ControlMetrics {
      pub active_ssh_sessions: AtomicUsize,
      pub last_reload_unix_ms: AtomicU64,
      pub last_reload_source: ArcSwap<&'static str>,  // "boot" | "sighup" | "admin_rpc"
  }
  ```
  Wired into both the SSH handler and the admin handler. Set `last_reload_unix_ms` + `last_reload_source` on each `cfg.store()` (boot, SIGHUP, admin RPC).
- [ ] Wire audit calls into existing handlers:
  - `main.rs` boot: `audit.append(AuditRecord::daemon_boot { version, own_node_id, allowed_count, admins_count })`.
  - `ControlHandler::accept` allowlist gate failure → `ssh_rejected { reason }`.
  - `ControlHandler::accept` admit + bridge open → `ssh_session_open { session_id }`.
  - `SessionGuard::drop` → `ssh_session_close { session_id, bytes_to_sshd, bytes_from_sshd, duration_ms }`. (`SessionGuard` gains an `Arc<Audit>` and tracks byte counters via `tokio::io::copy`'s return value — refactor the bridge from `try_join!` to two `JoinHandle`s so we can capture the byte counts on close.)
  - `AdminHandler::accept` gate failure → `admin_rejected { reason }`.
  - `AdminHandler::handle(AddAllowed | RemoveAllowed)` success → corresponding admin record. Failure → `admin_error { op, code, message }`.
  - `spawn_sighup_reloader` success → `config_reload { source: "sighup", allowed_count, admins_count }`.
  - Admin-RPC-driven config swap → `config_reload { source: "admin_rpc", allowed_count, admins_count }`.
- [ ] Extend `AdminClientMsg` with `TailAudit { last_n: u32, before_ts_ms: Option<u64> }` and `AdminServerMsg` with `AuditTail { records: Vec<AuditRecord>, eof: bool }`. Cap `last_n` at 500 server-side regardless of what the client asks for.
- [ ] Extend `StatusReply` with `last_reload_source: String`, `audit_record_count_estimate: u64` (cheap line-count of the active file, refreshed lazily), and `identity_schema_version: u32` (from `herd_scout_identity::SCHEMA_VERSION`).

**Dependencies**: Phase 3.

**Validation**:
1. Open three SSH sessions via `herdctl proxy`, run admin-stub-dial `Status` → `active_ssh_sessions == 3`.
2. From admin-stub-dial, `AddAllowed` two NodeIds, `RemoveAllowed` one, then `TailAudit { last_n: 10, before_ts_ms: None }`. Reply contains 3 admin records + at least one `config_reload`.
3. `tail -n 5 ~/.local/share/herd-scout/audit.log | jq .` parses cleanly and shows the same records.
4. Kill -9 the daemon mid-`AddAllowed` — `audit.log` doesn't have the success record (we wrote the file *first*, then audited; if the audit append fails, the rename already succeeded — accepted: dropping an audit record is preferable to failing the user-visible op).
5. Day-roll: stub the clock to advance 25h via test helper; the rotation task renames the file and opens a fresh one. Tail across both files in a follow-up `TailAudit { last_n: 100 }` reply returns records from both.

**Wiki grounding**: `[[iroh-docs-fms-schema]]` § Conflict resolution strategy 3 (append-only with derived state), § Schema evolution (versioned records). `plan-iroh-bound-ssh-access-daemon-2026-05-26` Decision 4 (`MAX_SESSIONS` + `SessionGuard`) — we extend that instrumentation rather than replacing it.

### Phase 5 — `herd-scout-jni` admin client surface + single-slot fleet model (estimated effort: 1 day)

**Goal**: a Kotlin facade `HerdScoutAdminJni` exposing the five RPCs (`List`, `Add`, `Remove`, `Status`, `TailAudit`), holding at most one active `Connection` at a time. Identity loaded from / saved to `identity.toml` via `herd-scout-identity` (Phase 0).

**Tasks**:
- [ ] In `android-jni/src/lib.rs`, add admin-client code under `cfg(target_os = "android")`. Factor the iroh-Endpoint setup helper into a small `endpoint_factory` mod that the camera path *and* the admin client share. The factory uses `herd_scout_identity::load_or_generate(&path, "herd-scout-admin")` where `path` comes from a `filesDir` argument passed through JNI (see below).
- [ ] **Single-Endpoint, single-Connection** (Decision 11): use `static ADMIN_ENDPOINT: OnceLock<Endpoint>` for the process-lifetime endpoint. Maintain `static ADMIN_SESSION: Mutex<Option<Arc<AdminSession>>>` for the at-most-one active connection. `nativeAdminConnect(daemonNodeId, filesDir)` semantics:
  1. If a session exists for a *different* daemon: `ADMIN_SESSION.lock().unwrap().take()` and call `session.close().await`. Wait for it.
  2. If a session exists for the *same* daemon: reuse and return the existing handle.
  3. Otherwise: `endpoint.connect(EndpointAddr::new(id), ADMIN_ALPN).await?`, wrap in `AdminSession`, store, return handle.
  The handle is a stable `jlong` per (daemon-node-id, session-incarnation); each `take()` increments the incarnation so stale handles error out cleanly with `AdminServerMsg::Error { code: "stale_handle" }` on use.
- [ ] Create `android-jni/src/admin_client.rs`. `AdminSession` holds the live `Connection`, a tokio `Mutex<()>` serializing RPCs (one bi-stream per RPC), and a `daemon_node_id: String` for handle identity.
- [ ] JNI exports (gated `cfg(target_os = "android")`):
  ```rust
  // All take a filesDir JString so Rust never has to reach into Context.
  Java_com_herdscout_admin_HerdScoutAdminJni_nativeAdminConnect(daemon_node_id, files_dir) -> jlong
  Java_..._nativeAdminListAllowed(handle) -> JString  // JSON Vec<AllowedEntry>
  Java_..._nativeAdminAddAllowed(handle, node_id, label) -> JString
  Java_..._nativeAdminRemoveAllowed(handle, node_id) -> JString
  Java_..._nativeAdminStatus(handle) -> JString  // JSON StatusReply
  Java_..._nativeAdminTailAudit(handle, last_n, before_ts_ms) -> JString  // JSON AuditTail
  Java_..._nativeAdminDisconnect(handle) -> jboolean  // true if we closed something
  // Identity-export/import don't need an admin connection — they read/write the local file.
  Java_..._nativeIdentityExport(files_dir, label) -> JString  // returns the TOML envelope as a UTF-8 string
  Java_..._nativeIdentityImport(files_dir, envelope) -> JString  // JSON { ok, node_id } or { error_code, message }
  Java_..._nativeIdentityWhoami(files_dir) -> JString  // returns the local NodeId
  ```
  Each call performs `connection.open_bi() → write_len_prefixed_json(cmd) → read_len_prefixed_json(reply)` and returns `serde_json::to_string(&reply)`.
- [ ] **Identity import contract**: `nativeIdentityImport` validates via `herd_scout_identity::import_from_user_blob`, then atomically writes the new envelope to `filesDir / "identity.toml"`. Crucially, it also tears down `ADMIN_SESSION` if any (the NodeId just changed; existing connections are bound to the old key). Returns the new NodeId on success; the Kotlin layer is responsible for re-prompting the user to reconnect to a daemon.
- [ ] Unit tests on host: gate behind `#[cfg(not(target_os = "android"))]` + a `#[cfg(test)]` helper that spawns an in-process daemon admin handler + endpoint pair, then round-trips `ListAllowed` and `TailAudit`. Identity round-trip test: `export → import → whoami` returns the same NodeId.

**Dependencies**: Phases 0 (identity crate), 3 (mutating RPCs), 4 (TailAudit + Status fields).

**Validation**: `cargo test -p herd-scout-jni --target $HOST_TARGET --features admin-test`. Asserts `AddAllowed` results in a file rewrite, a corresponding `ListAllowed` returns the new entry, `TailAudit` includes the matching record, switching daemons (call `nativeAdminConnect` with a different node_id) cleanly tears down the previous session.

**Wiki grounding**: `android-jni/src/lib.rs` (existing iroh-runtime + tokio scaffolding pattern), `[[mobile-desktop-architecture]]` ("Rust app core, shared on phone + desktop"), Decision 11 (single-slot connection model).

### Phase 6 — Android `herd-scout-admin` APK with fleet switcher + History + Identity backup (estimated effort: 2 days)

**Goal**: a separate Android app that manages allowlists across a saved-daemon fleet, shows a two-tab History (this device + daemon), and exports/imports identity envelopes for reinstall recovery.

**Tasks**:
- [ ] **Gradle**: copy `android/app/` to `android/admin/`. In `android/admin/build.gradle.kts`, set `applicationId = "com.herdscout.admin"`, `namespace = "com.herdscout.admin"`. Update `android/settings.gradle.kts` to `include(":app", ":shared", ":admin")`. Add `androidx.room:room-runtime`, `room-ktx`, `room-paging`, `androidx.paging:paging-compose`, `kotlinx-serialization-json` to the admin module.
- [ ] **Shared module** `android/shared/`: extract from `android/app/`:
  - QR-scan flow (`QrScanActivity.kt`, `QrViewfinderOverlay.kt`).
  - `HerdScoutJni.kt`'s loader bootstrap (`System.loadLibrary("herd_scout_jni")`).
  - `NodeIdFormat.kt` with `formatShort(node_id: String): String` returning first-8 + `...` + last-4 plus `formatRelative(tsMs: Long): String` ("23s ago", "3m ago").
  Update `android/app/`'s imports to point at the shared module.
- [ ] **Admin module — JNI facade**:
  - `HerdScoutAdminJni.kt` mirroring the JNI surface from Phase 5. Uses `kotlinx.serialization` to parse JSON replies into data classes (`AllowedEntry`, `StatusReply`, `AuditTail`, `AuditRecord`).
- [ ] **Admin module — fleet switcher** (Decision 11):
  - `DaemonRegistry.kt`: persists `List<DaemonEntry { nodeId, label, lastConnectedMs }>` to `SharedPreferences` as JSON, max 10 entries with LRU eviction. Exposes `flow: Flow<List<DaemonEntry>>` and `setActive(nodeId)`.
  - `DaemonSwitcherChip.kt`: top-app-bar Composable showing the current daemon's label, tap opens a `ModalBottomSheet` listing saved daemons (current marked with a check), an "Add daemon..." entry that opens a paste-or-scan dialog. Selecting an entry triggers `viewModel.switchDaemon(nodeId)` which:
    1. Sets a `StateFlow<UiState>` to `Switching(toLabel)`.
    2. Calls `nativeAdminConnect(newNodeId, filesDir)` on `Dispatchers.IO`. The JNI side automatically tears down any prior session (Phase 5 contract).
    3. On success: refreshes status + entries + history for the new daemon.
    4. On failure: surfaces the error and reverts to the previous daemon (still active in JNI if it didn't get torn down, or `Disconnected` if it did).
- [ ] **Admin module — main screen `AdminActivity.kt`** (Compose, three tabs):
  - **Tab 1 — Allowlist**:
    - Sticky header: daemon NodeId (short), version, "N admins / N allowed / N active SSH sessions", last-reload-relative + source ("admin RPC, 23s ago").
    - LazyColumn of `AllowedEntry` rows: NodeId-short + label, swipe-to-remove with confirmation dialog.
    - FAB: "+", opens a bottom sheet with two options: "Scan NodeId QR" or "Paste NodeId." Both prompt for a label.
  - **Tab 2 — History (split into two sub-tabs)**:
    - **From this device**: `LazyColumn` backed by `Pager(...).flow.collectAsLazyPagingItems()` from Room (Decision 9). Each row shows `tsMs`, `op`, `targetNodeId` (short), success/error chip.
    - **From daemon**: backed by `nativeAdminTailAudit(handle, 50, before_ts_ms)`. Pull-to-refresh + scroll-to-load-more (next page = oldest record's `tsMs` as `before_ts_ms`). When the daemon is unreachable, falls back to a Room cache of the last successful `TailAudit` reply (records get inserted with `kind = "daemon_replay"` and a `daemonNodeId` discriminator so this view filters correctly).
    - Both sub-tabs filter by the active `daemonNodeId`; switching daemons updates both views.
  - **Tab 3 — My Identity**:
    - Shows this device's NodeId both as text and as a QR code (for enrolling a second admin device).
    - "Export identity..." button: calls `nativeIdentityExport(filesDir, label)`, drops the returned TOML envelope into Android's `ACTION_CREATE_DOCUMENT` flow so the user picks a save location (Drive / Files / SD card). Confirms with the file size and a reminder: "Keep this file safe — anyone who has it can act as you."
    - "Import identity..." button: `ACTION_OPEN_DOCUMENT`, reads the file, calls `nativeIdentityImport`. On success: shows the new NodeId and a "You'll need to reconnect to a daemon to continue" message; tearing down the active connection happens inside the JNI. On schema-mismatch / integrity failure: shows the specific `IdentityError` code in a dialog.
- [ ] **AdminViewModel.kt**: holds `activeDaemon: StateFlow<DaemonEntry?>`, `connectionState: StateFlow<UiState>` (`Disconnected | Switching(label) | Connected(entries, status, audit)`), `auditPager: Pager<...>` (Room) + `daemonAuditFlow: Flow<List<AuditRecord>>` (Phase 5 RPC). Calls all JNI on `Dispatchers.IO`. After every successful mutation, also writes a Room row (`rpc_success`); on RPC errors, writes `rpc_error` so the user's local history shows attempted-and-failed operations even if the daemon never logged them.
- [ ] **Permissions / storage**: only `INTERNET` (the iroh client needs it). Identity file lives in `Context.filesDir / "identity.toml"` — Android-private app storage. SAF (Storage Access Framework) is permissionless on modern Android and handles export/import without `READ_EXTERNAL_STORAGE`.
- [ ] **Visual differentiation**: orange-on-charcoal launcher icon (vs streaming app's neutral icon), label "herd-scout admin." Both icons sit next to each other in the launcher; misidentification is unlikely.
- [ ] Out of scope for v1: real-time push updates (5s auto-poll on the foreground tab is fine), undo for remove, multi-select bulk operations, encrypted identity export (the file itself is the secret; users picking a Drive location are accepting that trust boundary).

**Dependencies**: Phase 5.

**Validation**: install both APKs side-by-side on a single phone. Streaming app still publishes camera as before. Open admin app → "Add daemon..." → paste laptop NodeId → see status header populate → add a second phone's NodeId via QR scan → confirm via `cat ~/.config/herd-scout/control.toml` on the laptop that the new entry is present and via `tail -3 ~/.local/share/herd-scout/audit.log | jq .` that the audit recorded it. Switch to a second saved daemon (running on a second laptop in dev) → spinner appears, connection cycles, lists/history update. From "My Identity" tab → Export → save to Drive → uninstall the app → reinstall → Import from Drive → NodeId restored, daemon still recognizes us.

**Wiki grounding**: `[[herd-scout-positioning]]` wedge #2 (P2P / no central server) — fleet mode without a central directory is the same shape; the phone holds the directory itself in `SharedPreferences`. `[[mobile-desktop-architecture]]` "two UI codebases when data model is the same" anti-pattern is *not* violated; admin and streaming are different surfaces.

### Phase 7 — Operator docs + bootstrap walkthrough + audit/backup playbook (estimated effort: 0.5 day)

**Goal**: `deploy/README.md` covers admin-bootstrap, identity backup, audit-log inspection, and fleet enrollment.

**Tasks**:
- [ ] Update `deploy/README.md` § "Reaching a deployed daemon" with new sub-sections:
  - **Granting admin rights to a phone**:
    1. On the phone, install `herd-scout-admin` APK (sideload). Open → "My Identity" → shows a QR + the NodeId text.
    2. On the laptop, ssh in (Wave 11 path). Edit `~/.config/herd-scout/control.toml`, add the phone's NodeId under `[[control_plane.admins]]`. Save. `sudo systemctl kill -s HUP herd-scout-daemon`.
    3. On the phone, "Add daemon..." → paste laptop NodeId. Status should turn green within 3-5s.
    4. From now on, add new SSH peers from the phone — no more laptop ssh required.
  - **Backing up your admin identity** (one-time, on first launch):
    1. Open admin app → "My Identity" tab → "Export identity..." → choose Drive / iCloud / a USB stick. Save the file with a descriptive name (e.g. `herd-scout-admin-identity-2026-05-27.toml`).
    2. The file's contents are sensitive — anyone who has it can act as you on the daemon. Treat it like an SSH private key.
    3. To restore on a fresh phone: install the APK → open → "My Identity" → "Import identity..." → pick the saved file. The NodeId is preserved; existing daemons recognize you immediately.
  - **Reading the audit log**:
    - From the phone: open the History tab. "From this device" is fast and works offline. "From daemon" requires a live connection but shows everything the daemon recorded — including SSH sessions and config reloads triggered by other operators.
    - From the laptop: `tail -F ~/.local/share/herd-scout/audit.log | jq .` for a live view. Rotated daily files are at `~/.local/share/herd-scout/audit-YYYY-MM-DD.log`.
    - Forensics example: `jq 'select(.kind == "ssh_session_open")' ~/.local/share/herd-scout/audit*.log` lists every shell session ever opened against this daemon.
  - **Fleet enrollment**: on the phone, "Add daemon..." accepts paste or QR. The daemon switcher chip in the top bar lists all saved daemons; tapping switches active connection (one at a time). Up to 10 saved daemons.
- [ ] Annotate `plan-iroh-bound-ssh-access-daemon-2026-05-26` Phase 5 with a forward-pointer: "Wave 12 adds an Android admin app over a fourth ALPN, atomic config rewrites, audit log, and identity backup; see `plan-android-admin-allowlist-app-2026-05-27`."
- [ ] Document that `control.toml` is now machine-managed (comments will be lost on the next admin-RPC write). Suggest operators stash any explanatory comments in `~/.config/herd-scout/NOTES.md` instead.
- [ ] Document the `identity.toml` schema in `herd-scout-identity/README.md`: when to bump `schema_version`, the rules from Decision 7, the legacy-migration semantics.

**Dependencies**: Phase 6.

**Validation**: a colleague follows the README from "fresh phone, fresh laptop" and successfully (a) grants a third device SSH access via the admin app, (b) inspects the corresponding audit record on both phone and daemon, (c) exports their identity, uninstalls/reinstalls the app, and re-imports — confirming the daemon still recognizes them.

**Wiki grounding**: `plan-iroh-bound-ssh-access-daemon-2026-05-26` Phase 5 is the template for "operator docs + one-shot bootstrap."

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Operator hand-edits `admins = []` and locks out the admin app | Wave 11 SSH path remains; ssh in and re-edit. Two allowlists are orthogonal by design. |
| Self-retraction empties `admins` | Decision 11: daemon refuses writes that would leave `admins.len()==0`. UI shows `would_orphan_daemon`. |
| Atomic rewrite drops TOML comments and reorders entries | Documented as expected (Decision 4); operators stash notes elsewhere. `toml_edit` rejected — too much code for incomplete round-trip. |
| Hand-edit ↔ admin-RPC race | Last-writer-wins; small file, only hand-edited at bootstrap. mtime-check is a future addition if real conflicts appear. |
| Crash between durable rewrite and in-process `ArcSwap::store` | Decision 4 mandates write-then-swap ordering; next boot loads the file and converges. |
| Two admins racing on `AddAllowed`/`RemoveAllowed` | Daemon write `Mutex` serializes; second writer gets a clean `already_present` / `not_found` error. |
| Malformed NodeId from the UI | `EndpointId::from_str` gates; `Error { code: "invalid_node_id" }`; UI shows verbatim. |
| Audit writes lost on hard kill (no per-record fsync) | Accepted. Phone-side Room records `rpc_attempt` before the call (Decision 9), so the union of the two views covers both directions of partial failure. |
| Audit-log size grows unboundedly | Daily rotation + 90-day retention. ~5 MB worst-case at 100 SSH opens/day for 90 days. |
| `audit.log` accidentally world-readable | Opened with mode `0600`; `data_dir` already user-private. CI asserts mode. |
| Identity export leaks secret via cloud | UI warns explicitly. No encryption in v1 — forgotten-passphrase is worse than a private Drive folder. |
| Identity import with future schema | `IdentityError::UnsupportedSchema` is loud and recoverable: keep current identity, upgrade app. |
| Fleet switch races an in-flight RPC | Single-slot JNI model serializes; QUIC close-error on old conn is treated by Kotlin as "expected during switch." |
| Iroh 0.98 API drift surprises us | Phase 1 stub is the canary, same pattern as Wave 11 Risk #2. |
| `control.toml` schema migration breaks existing Wave 11 boxes | Parser accepts both old `allowed_node_ids = [...]` and new `[[control_plane.allowed]]`; rewrites in new shape on next admin write. |
| Phone runs both apps with conflicting identities | Different `applicationId`s have separate `filesDir`s; no shared state. Streaming stays ephemeral, admin stays persistent. |
| JNI symbol bloat from admin code in the streaming app's `.so` | Accepted. Both apps link the same cdylib; admin exports are dead code at runtime in the streaming app. Split into a second cdylib only if size becomes a real issue. |
| Drone-strapped phone gets the admin APK by mistake | Distinct `applicationId`, distinct icon, distinct label. No further gating. |

## Open Questions

- **Should `Status` include the most recent SSH client NodeIds?** Useful for "who's connected right now" UI. Privacy-trivial in a single-operator deployment; might matter in a multi-tenant future. Out of scope for v1; the audit log already provides this with one query (`jq 'select(.kind == "ssh_session_open" and .ts_ms > now*1000 - 86400000)'`).
- **Pull vs. push for live updates**: v1 polls every 5s. Push requires a long-lived bi-stream and a daemon-side broadcaster. Worth doing if/when we add other live signals (CPU temp, GPU load, recent detections) — bundle them into a single `Subscribe` RPC then.
- **Admin-RPC retract self**: resolved via Decision 11. Daemon refuses any write that would leave `admins.len() == 0`. Phone UI shows the error verbatim. Self-retraction with another admin still present is allowed (with a confirmation dialog in Phase 6).
- **Tablet UI**: the current Compose layout assumes phone aspect ratio. A landscape tablet layout (master-detail with the entry list on the left and details on the right) would be a nice follow-up but doesn't affect v1.
- **Encrypted identity export**: do we add a passphrase prompt to `nativeIdentityExport` / `nativeIdentityImport`? Symmetric AEAD with Argon2id KDF is well-understood, but a forgotten passphrase is worse than an unencrypted file in a private Drive folder. Defer to v2; if we do it, follow the age (filippo.io/age) format.
- **Audit-log signing / tamper-evidence**: the daemon could chain records via `prev_hash` (each record's hash includes the previous record's hash) so deletions/edits become detectable. ~30 LOC. Worth doing if compliance ever matters; defer for now.
- **Replicated audit log via iroh-docs**: phones in a fleet could push their `rpc_attempt` records into a shared iroh-docs namespace so the operator sees a unified history across multiple admin devices. Interesting but solidly v2; the per-device + per-daemon split is fine for v1.

## Sources Consulted

| Source | What was drawn from it |
|---|---|
| `[[iroh-sync-stack]]` | One Endpoint, multiple ALPNs is the established pattern. Adding `admin/1` is the same shape Wave 11 used for `ssh/1`. |
| `[[mobile-desktop-architecture]]` | "Desktop is just another peer" — the phone-as-admin is a peer dialing the daemon's iroh node, no central server. JSON-over-bi-stream is the same framing pattern the daemon↔GUI IPC already uses. |
| `[[herd-scout-positioning]]` | Wedge #2 (P2P / no central server) justifies admin-over-iroh instead of admin-over-HTTP/REST. Native mobile UX is a wedge — separate APK keeps the field-streaming app focused. |
| `[[iroh-docs-fms-schema]]` | Append-only / HLC / atomic writes principles. Not adopted in full (allowlist is small and human-readable matters), but the durability-before-visibility ordering (Decision 4) borrows from there. |
| `.wiki/output/plan-iroh-bound-ssh-access-daemon-2026-05-26.md` | Direct precedent: ALPN registration on the existing Router, fail-closed config defaults, SIGHUP reload, NodeId-allowlist gating. We mirror its approach almost line-for-line. |
| `.wiki/output/plan-deploy-daemon-on-1060-laptop-2026-05-22.md` | Phase 1 baseline (key-only sshd, ufw) is what the SSH allowlist sits on top of; admin RPCs sit one layer above that. |
| `herd-scout-daemon/docs/daemon-split-design.md` | IPC framing: 4-byte BE length + JSON. We re-use the same framing on a new ALPN. |
| `herd-scout-daemon/src/control.rs`, `control/config.rs`, `control/handler.rs`, `main.rs` | Concrete Wave 11 code that this plan extends. `ArcSwap`, `SignalKind::hangup()`, `EndpointId::from_str`, `accept_bi`, `Router::builder.accept`. |
| `herd-scout-ipc/src/lib.rs` | Where `CONTROL_ALPN` lives, how `serde(tag = "type")` enums are shaped, where `ADMIN_ALPN` and admin enums will land. |
| `herdctl/src/main.rs` | Iroh client patterns: persistent ed25519 secret at `0600`, `Endpoint::builder(presets::N0).secret_key(...).bind()`, `endpoint.connect(EndpointAddr::new(id), ALPN)`. The phone's iroh-jni client mirrors this. |
| `android-jni/src/lib.rs`, `android/app/src/main/java/com/herdscout/app/HerdScoutJni.kt`, `MainActivity.kt` | Existing tokio runtime + iroh `Live` boot on Android, JNI surface conventions, Kotlin facade pattern, QR-scan UX. |
| `deploy/README.md` | Operator quickstart already has a Wave 11 section; Phase 7 of this plan extends it with the admin-bootstrap walkthrough. |

## Proposed Inventory Records

| ID | Type | Description |
|---|---|---|
| `q1` | open-question | Verify `iroh-live::Live` accepts a pre-bound `Endpoint` so `herd-scout-identity` can own the daemon secret. If not, file an upstream patch. |
| `t1` | task | Sideload-distribute the admin APK after Phase 6 ships (no Play Store). |
| `t2` | task | Tune audit-log retention after a month of real data. |
| `w1` | watch-item | n0-computer iroh: deprecation of `Router::accept(alpn, handler)`. |
| `w2` | watch-item | Upstream iroh identity-file format — harmonize if/when it lands. |
