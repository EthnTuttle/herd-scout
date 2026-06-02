---
title: "Plan: iroh-smol-kv FMS schema + record CRUD (P0)"
type: plan
format: roadmap
sources:
  - wiki/concepts/iroh-docs-fms-schema
  - wiki/concepts/fms-data-model
  - wiki/concepts/mobile-desktop-architecture
  - wiki/concepts/iroh-sync-stack
  - wiki/concepts/livestock-oss-gap-analysis
  - wiki/concepts/herd-scout-positioning
  - wiki/concepts/fms-feature-taxonomy
  - inventory/features/iroh-smol-kv-fms-schema
  - inventory/watch/iroh-blobs-233-poisoned-store
  - output/assess-herd-scout-2026-06-02
generated: 2026-06-02
---

# Plan: iroh-smol-kv FMS schema + record CRUD (P0)

> Generated from the herd-scout local wiki (9 articles + 2 inventory items + 1 assess output).

## Executive Summary

Wire iroh-smol-kv as the durable data plane and ship the minimal record layer the assess identified as P0: Animal / Group / Land / Equipment assets and Observation / Medical / Movement / Weight / Birth logs. Schema follows the wiki's locked design (entity-attribute, one-scalar-per-key, ULID + HLC, per-field conflict strategy, BLAKE3 photo refs, SQLite projection). Frontend extends the existing **egui** GUI — no Tauri 2 pivot in this plan. Projection is **co-location-aware**: when GUI and daemon share a host, the GUI is a thin IPC reader against the daemon's SQLite; when the GUI is remote, it joins as its own iroh peer with its own projection. Onboarding reuses the existing LiveTicket QR primitive for farm-namespace invites. iroh stays pinned at 0.98.0 / iroh-blobs 0.102.0 (blocked on watch item #233). farmOS JSON:API and the EID Bluetooth crate are out of scope — they get their own plans.

## Architecture Decisions

### Decision 1: Entity-attribute key layout, one scalar per key

**Context**: [[../wiki/concepts/iroh-docs-fms-schema]] §"Why one-scalar-per-key" documents that per-field LWW means two authors editing different fields don't conflict; a JSON-blob-per-entity collapses every edit into one merge cell.

**Options considered**:
- **A. JSON-blob-per-entity** (`asset/<id> → {name, geom, ...}`) — simpler reads, contended writes
- **B. Entity-attribute, one scalar per key** (`asset/<id>/name`, `asset/<id>/geom`) — per-field LWW, no false conflicts (per [[../wiki/concepts/iroh-docs-fms-schema]])

**Decision**: **B**. Locks in the wiki's documented design verbatim.

**Consequences**: Reads do prefix scans (`asset/<id>/`) and gather scalars; writes touch only the changed scalar. Wiki gotcha: "10M small keys heavier than 100k medium" — don't decompose finer than one scalar per field.

### Decision 2: ULID for ids; HLC mandatory for timestamps

**Context**: [[../wiki/concepts/iroh-docs-fms-schema]] §Gotchas: "Wallclock LWW silently corrupts data — HLC mandatory." [[../wiki/concepts/mobile-desktop-architecture]] §Anti-patterns: "LWW with device wallclock — field devices have unsynced clocks; LWW silently picks wrong write."

**Options considered**:
- UUIDv4 + wallclock — opaque ids, broken LWW
- ULID + wallclock — sortable ids, broken LWW
- **ULID + HLC** — sortable ids, correct LWW

**Decision**: **ULID + HLC**. Use the `ulid` crate and `uhlc::HLC`.

**Consequences**: HLC clock initialized at daemon boot, persisted to disk, advanced on every write. Phone and GUI receive HLC tick from daemon over IPC for their own writes (or run their own HLC if remote).

### Decision 3: Per-field conflict strategy

**Context**: [[../wiki/concepts/iroh-docs-fms-schema]] §"Conflict resolution" defines three strategies. LWW is **unacceptable** for: quantities (silent stock loss), medical withdrawal periods (safety), movement events (animal in two paddocks).

**Decision**: Apply the wiki's three strategies field-by-field:

| Field class | Strategy | Implementation |
|---|---|---|
| `asset/<id>/name`, `notes` | LWW on `(HLC ts, author_id)` | Read picks max (ts, author) |
| `asset/<id>/tag/<term-id>`, `log/<id>/asset_ref/<asset-id>` | Add-wins set | `…/_deleted=<hlc>` tombstones; drop on read if any deletion ≥ all adds |
| `log/<id>/*` (the entire log entity) | Append-only | Logs immutable after creation; current state derived |
| `asset/<id>/quantity/*` | Append-only via logs | Quantities live on logs, not assets |

**Consequences**: Medical withdrawal periods, movement events, and quantities are *never* LWW-overwritten. Inventory is derived per [[../wiki/concepts/fms-data-model]] §"Inventory is derived" — no separate inventory table.

### Decision 4: Frontend stays on egui; defer Tauri 2

**Context**: [[../wiki/concepts/mobile-desktop-architecture]] recommends Tauri 2 as primary; the assess flagged Tauri 2 mobile churn as the single biggest *technical* risk for a future mobile pivot. User chose to extend the existing egui GUI.

**Decision**: Extend `herd-scout-gui` (egui). Add forms + list views for Animal/Group/Land/Equipment + log-entry forms.

**Consequences**: iOS and a Tauri 2 mobile path are explicitly out of scope. The Android admin app stays as-is (it doesn't need record CRUD in this plan). Migrating to Tauri 2 later remains possible but is a separate wave; per the assess anti-pattern, treat any future port as a *port*, not a rewrite-and-throw-out.

### Decision 5: Co-location-aware SQLite projection

**Context**: User question during interview — "is there a way to know if the daemon is running locally so we don't duplicate databases?" Daemon already owns a Unix-domain IPC socket at `$XDG_RUNTIME_DIR/herd-scout/ipc.sock` (or `/run/herd-scout/ipc.sock` for system-mode); GUI auto-spawns the daemon today.

**Options considered**:
- Daemon-owned only — GUI does all reads via IPC (works locally, breaks for remote GUI)
- GUI-local always — every GUI gets its own SQLite (storage cost, drift risk)
- **Co-location-aware** — GUI probes the IPC socket at startup; reachable → IPC reader, no local SQLite. Unreachable → GUI joins as its own iroh peer with its own projection

**Decision**: **Co-location-aware**. Same binary, different mode by environment.

**Consequences**:
- Local case (the common one today): one SQLite projection on disk; GUI is a thin IPC reader.
- Remote case (e.g., GUI on operator's laptop, daemon on the GTX 1060 box per [[plan-deploy-daemon-on-1060-laptop-2026-05-22]]): GUI runs its own iroh-smol-kv replica + projection. Storage cost paid only when needed.
- Mode is detected at GUI startup; configurable override for testing.

### Decision 6: Onboarding via LiveTicket-style farm-namespace invites

**Context**: [[../wiki/concepts/iroh-docs-fms-schema]] §Gotchas: "Namespace key distribution is your problem — iroh-tickets for invite codes; design onboarding (QR code on desktop scanned by phone) up front." Daemon already mints LiveTickets and the phone scans QR for video pairing.

**Decision**: Mint farm-namespace invite tickets that look and feel exactly like LiveTickets. Phone or remote GUI scans the QR (or pastes the base64) to join.

**Consequences**: Reuses ML Kit QR scanner already in the phone app; reuses LiveTicket-minting code path on the daemon. Multi-device sync of records works the moment the second device redeems the ticket. Ticket scope: write-grant for the farm namespace + author-key registration in `user/<user-ulid>/device/<author-pubkey>`.

### Decision 7: Pin iroh 0.98.0 / iroh-blobs 0.102.0; defer JSON:API

**Context**: [[../inventory/watch/iroh-blobs-233-poisoned-store]] is open; iroh 1.0-rc.1 published 2026-05-27 but GA timing is unconfirmed. Assess marks farmOS JSON:API as P1 with its own inventory candidate.

**Decision**: Build on the same versions Wave 14 ships. Schedule the iroh 1.0 migration as its own wave once #233 closes. Do not bolt JSON:API into this plan.

**Consequences**: One less moving part this quarter; preserves momentum. The watch item already documents the upgrade gate.

## Implementation Phases

### Phase 0 — Schema lock-in + iroh-smol-kv API audit (1 week)

**Goal**: Verify the wiki's API sketch against the live iroh-smol-kv (branch `iroh-098`) source; freeze the exact key layout for the five asset types and five log types.

**Tasks**:
- [ ] Audit iroh-smol-kv method names against live source ([[../wiki/concepts/iroh-sync-stack]] caveat: API "similar but not identical" to iroh-docs).
- [ ] Confirm the crate's persistence backend at `Docs::persistent(path)` (or current equivalent name).
- [ ] Confirm subscribe-stream API and event variants for projection.
- [ ] Lock the key layout in `docs/fms-schema.md` inside the daemon crate (one source of truth, copy from [[../wiki/concepts/iroh-docs-fms-schema]] §"Key layout"). Define the exact term-set IDs for `kind` enum values.
- [ ] Write `_schema=1` markers per entity type and document the migration rules ([[../wiki/concepts/iroh-docs-fms-schema]] §"Schema evolution" — new fields = new keys; never repurpose; lazy on-the-fly upgrade).

**Dependencies**: None.

**Validation**: Daemon compiles against the audited iroh-smol-kv API; a unit test creates a persistent doc, writes one scalar, restarts, reads it back.

**Wiki grounding**: [[../wiki/concepts/iroh-docs-fms-schema]] §Gotchas warns "iroh-smol-kv API differs from iroh-docs — verify method names before code commits." This phase exists explicitly to honor that.

### Phase 1 — `herd-scout-fms` crate (2 weeks)

**Goal**: A workspace crate that owns the FMS data model, the iroh-smol-kv read/write surface, and HLC + ULID glue.

**Tasks**:
- [ ] New crate `herd-scout-fms` in the workspace.
- [ ] Native Rust types per [[../wiki/concepts/fms-data-model]] §Recommendation:
  ```rust
  struct Asset { id: Ulid, kind: AssetKind, name: String, geom: Option<GeoJson>, parent: Option<Ulid>, archived: bool, schema: u32 }
  enum AssetKind { Animal, Group, Land, Equipment }
  struct Log { id: Ulid, kind: LogKind, timestamp: Hlc, asset_refs: Vec<Ulid>, quantities: Vec<Quantity>, photos: Vec<BlobRef>, notes: String }
  enum LogKind { Observation, Medical, Movement, Weight, Birth }
  struct Quantity { measure: String, value: f64, unit: String, label: Option<String> }
  struct BlobRef { hash: blake3::Hash, mime: String, size: u64 }
  ```
- [ ] `Fms` struct holding `Doc`, `AuthorId`, `BlobsClient`, persistent `HLC`. Methods: `create_asset`, `read_asset`, `update_asset_field`, `archive_asset`, `append_log`, `read_log`, `list_assets_by_kind`, `list_logs_for_asset`, `add_log_photo`. Match the wiki code sketch ([[../wiki/concepts/iroh-docs-fms-schema]] §"Code sketch") for naming.
- [ ] Conflict-resolution helpers: LWW reader (`max_by_key((ts, author))`), add-wins-set reader (drop entries with `…/_deleted` ≥ all adds), append-only-log reader (no merge — concatenation).
- [ ] HLC persistence: clock state at `<data_dir>/herd-scout/hlc.bin`; load on boot, advance on every write, persist on graceful shutdown.
- [ ] Photo helper: `add_log_photo(log_id, seq, bytes)` — `blobs.add_bytes` then write `log/<id>/photo/<seq:04>` + `…/mime` + `…/size` ([[../wiki/concepts/iroh-docs-fms-schema]] §"Large value / blob references"). **Never embed bytes in the doc.**
- [ ] Crate-level unit tests covering each conflict strategy with synthetic two-author concurrent writes.

**Dependencies**: Phase 0.

**Validation**: Round-trip every asset/log type; concurrent-write tests for each conflict strategy pass; restart-and-reload preserves both records and HLC state.

**Wiki grounding**: [[../wiki/concepts/iroh-docs-fms-schema]] §"Code sketch" + [[../wiki/concepts/fms-data-model]] §"Recommendation for herd-scout".

### Phase 2 — Daemon integration + IPC RPCs (1 week)

**Goal**: Daemon opens the farm namespace on boot, exposes record CRUD over the existing IPC channel.

**Tasks**:
- [ ] Daemon boot sequence: open or create the farm namespace at `<data_dir>/herd-scout/fms.docdb`; lazy-create empty namespace on first run.
- [ ] Extend `herd-scout-ipc::ClientMsg` with: `CreateAsset`, `ReadAsset`, `UpdateAssetField`, `ArchiveAsset`, `ListAssetsByKind`, `AppendLog`, `ReadLog`, `ListLogsForAsset`, `AddLogPhoto`.
- [ ] Extend `herd-scout-ipc::ServerMsg` with the matching responses + a `RecordEvent` push variant for live updates.
- [ ] Daemon subscribes to its own iroh-smol-kv doc and emits `RecordEvent` to connected IPC clients (drives reactive GUI lists).
- [ ] Wire FMS write events into the existing audit log (`audit.rs`) — record schema-version, entity, key, author, ts. Reuse Wave-12 append-only JSONL infrastructure; record names match the existing event vocabulary (`fms_asset_create`, `fms_log_append`, etc.).

**Dependencies**: Phase 1.

**Validation**: `herdctl` (with a tiny new `fms` subcommand or via a debug interactive shell) creates an asset on the daemon, GUI receives the live `RecordEvent` push, list query reflects it.

**Wiki grounding**: [[../wiki/concepts/mobile-desktop-architecture]] §"The pattern" — write to local first, always; daemon is a peer, not a server.

### Phase 3 — SQLite projection + co-location-aware GUI (1 week)

**Goal**: Daemon-owned SQLite read-projection; GUI co-location detection.

**Tasks**:
- [ ] Daemon: SQLite at `<data_dir>/herd-scout/fms.sqlite`. Schema:
  - `asset(id, kind, name, geom, parent, archived, hlc_ts, schema)`
  - `log(id, kind, hlc_ts, notes, schema)`
  - `log_asset_ref(log_id, asset_id)`
  - `quantity(log_id, measure, value, unit, label)`
  - `log_photo(log_id, seq, blake3, mime, size)`
- [ ] Indexes: FTS5 virtual table on `log.notes`; R-Tree on `asset.geom`; ordinary indexes on `(asset.kind, archived)`, `(log.kind, hlc_ts)`, `(log_asset_ref.asset_id)`.
- [ ] Doc-event subscriber that calls `apply_to_sqlite` per [[../wiki/concepts/iroh-docs-fms-schema]] §"Indexing — SQLite projection". Persist last-applied checkpoint at `<data_dir>/herd-scout/projection.checkpoint`.
- [ ] Replay-from-scratch path: drop SQLite, replay from doc head — projection is pure derivation.
- [ ] GUI startup: probe `$XDG_RUNTIME_DIR/herd-scout/ipc.sock` (or `/run/herd-scout/ipc.sock` for system mode). Reachable → IPC mode (no local SQLite). Unreachable → remote mode: spin up own iroh peer + own SQLite + own projection.
- [ ] Configurable override `HERD_SCOUT_GUI_MODE=ipc|remote` for testing.

**Dependencies**: Phase 2.

**Validation**: Co-located: GUI shows zero SQLite files of its own. Remote: GUI starts cold against an unreachable daemon, runs its own iroh peer, syncs the namespace, and renders identical lists.

**Wiki grounding**: [[../wiki/concepts/iroh-docs-fms-schema]] §"Indexing — SQLite projection" — "iroh-smol-kv = source of truth (immutable system of record); local SQLite = projection (rebuildable at any time)."

### Phase 4 — egui CRUD UX (2-3 weeks)

**Goal**: Add Animal/Group/Land/Equipment forms + list views, plus the five log-entry forms, to `herd-scout-gui`.

**Tasks**:
- [ ] Side-panel navigation: live preview (existing) + "Records" tab.
- [ ] Records tab — four asset-kind sub-tabs (Animal / Group / Land / Equipment). Each sub-tab: list view (sortable by name, kind-filter, archived toggle) + create-form modal + per-row edit-form modal + archive button.
- [ ] Animal-specific form fields: `name`, optional `parent` (Group picker), optional `geom` (point — last-seen pin), tag-set picker (Term picker).
- [ ] Land form: `name`, `geom` (polygon — defer; simple WKT text-area for v1).
- [ ] Equipment form: `name`, `geom` (point optional).
- [ ] Group form: `name`, parent (Group), member-list (Animal multi-picker).
- [ ] "Logs" tab — five log-kind sub-tabs (Observation / Medical / Movement / Weight / Birth). Each: list view filtered by date + asset-ref + kind, create-form modal:
  - Observation: notes + asset_refs + photos
  - Medical: notes + asset_refs + withdrawal-period field (number of days, mandatory)
  - Movement: from-Land + to-Land + asset_refs (mandatory)
  - Weight: asset_refs + Quantity{measure="weight", value, unit (kg|lb)}
  - Birth: dam (Animal picker, required) + sire (Animal picker, optional) + offspring asset_refs
- [ ] Reactive updates: subscribe to `RecordEvent` IPC push; refresh affected list view on change.
- [ ] Empty-state and error-state UX for both co-located and remote modes.

**Dependencies**: Phase 3.

**Validation**: Click-through of each create / read / update / archive path renders the right SQLite write, the right doc entry, and the right audit-log line. Withdrawal-period field on Medical is non-optional.

**Wiki grounding**: [[../wiki/concepts/fms-feature-taxonomy]] §"MVP for herd-scout" — Animal/Group/Land/Equipment + Observation/Medical/Movement/Weight/Birth + treatment-log-with-withdrawal-flag is the documented MVP.

### Phase 5 — QR farm-namespace onboarding (1 week)

**Goal**: Operators can mint a farm-invite QR on the daemon; phone or remote GUI scans/pastes to join.

**Tasks**:
- [ ] Daemon: `mint_farm_invite() -> FarmTicket` — base64 envelope carrying namespace id, primary-peer NodeId, write-grant token, and a label. Reuse the LiveTicket envelope shape and minting pipeline as much as possible.
- [ ] GUI: "Mint farm invite" button in Records tab — pops a QR + base64 string + copy button.
- [ ] Phone (com.herdscout.app): existing ML Kit QR scanner accepts both `LiveTicket` and `FarmTicket` (discriminate by the envelope's type tag).
- [ ] On accept, joining peer: registers its author key in `user/<user-ulid>/device/<author-pubkey>`, opens the namespace in read+write mode, kicks off iroh-smol-kv reconciliation.
- [ ] herdctl: `herdctl farm join <ticket-string>` for the headless or remote-desktop path.
- [ ] Audit-log entries: `farm_invite_mint`, `farm_invite_redeem`, `farm_author_register`.

**Dependencies**: Phase 2 (records exist), Phase 3 (projection so the joiner has somewhere to read).

**Validation**: Cold second device redeems a ticket, sees the existing animal/log records appear in its list within reconciliation latency. No data leakage to non-redeemed peers.

**Wiki grounding**: [[../wiki/concepts/iroh-docs-fms-schema]] §Gotchas — "design onboarding (QR code on desktop scanned by phone) up front."

### Phase 6 — Validation, audit-log, README (1 week)

**Goal**: Document accuracy and stability commitments; verify the audit-log integration is correct end-to-end; add a Records section to the public README.

**Tasks**:
- [ ] Record-write audit-log smoke test: create one of each asset and one of each log type; grep `audit.log` and assert the record-event lines exist with the expected schema.
- [ ] Inventory derivation test ([[../wiki/concepts/fms-data-model]] §"Inventory is derived"): write a Weight log, then a Movement log; query "current weight" and "current paddock" via the SQLite projection; both should reflect the derived state without any inventory table.
- [ ] HLC drift test: simulate a 4-hour wallclock skew on a second peer, observe LWW still picks the correct winner (HLC `(ts, author)`).
- [ ] Phase-1-EID-pre-wire docstring: add a TODO comment in `Fms` near `create_asset` pointing to the inventoried EID feature and to layer-5 reconciliation. No code, just a hand-off marker.
- [ ] Update root `README.md` (the assess flagged that no public README exists): one paragraph on what records the system tracks; a tiny screenshot of the egui Records tab.
- [ ] Add documented accuracy commitments per [[../wiki/concepts/livestock-cv-accuracy]] (±5–10% pasture, ±15–25% bad-case) — this fits the assess P1 item but lands cheaply here.

**Dependencies**: Phases 4 and 5.

**Validation**: All tests pass; README renders the Records section; audit-log integration confirmed.

**Wiki grounding**: assess §"Recommended Actions" P1.7 (documented accuracy commitments).

## Risks & Mitigations

| Risk | Source | Mitigation |
|---|---|---|
| iroh-smol-kv API drift since wiki was written | [[../wiki/concepts/iroh-sync-stack]] §"Important correction" | Phase 0 audit before any code commits; lock to a specific branch SHA |
| iroh-blobs `#233` poisoned-store breaks photo lifecycle | [[../inventory/watch/iroh-blobs-233-poisoned-store]] | Pin iroh-blobs at 0.102.0; never bump in this plan; per-blob read-time verification considered as defense in depth |
| LWW with wallclock silently corrupts data | [[../wiki/concepts/iroh-docs-fms-schema]] §Gotchas, [[../wiki/concepts/mobile-desktop-architecture]] §Anti-patterns | HLC mandatory; never use wallclock; HLC drift test in Phase 6 |
| Storing media in the CRDT bloats the doc | [[../wiki/concepts/iroh-docs-fms-schema]] §"Large value / blob references", [[../wiki/concepts/mobile-desktop-architecture]] §Anti-patterns | Photo helper writes BLAKE3 hash refs only; bytes go to iroh-blobs |
| Per-field decomposition pessimizes reconciliation | [[../wiki/concepts/iroh-docs-fms-schema]] §Gotchas — "10M small keys heavier than 100k medium" | Stop at one scalar per logical field; no nesting trivia keys |
| Tombstone storage growth | [[../wiki/concepts/iroh-docs-fms-schema]] §Gotchas — "Tombstones are forever" | Measure in Phase 6; budget tiny; cross-replica GC out of scope |
| Author-key compromise = full namespace write | [[../wiki/concepts/iroh-docs-fms-schema]] §Gotchas | Treat like SSH keys; reuse Wave-14 [[../wiki/references]] identity-threat model; rotation = mint new author, deprecate old in `user/<user-ulid>/device/...` |
| Tauri 2 mobile churn for any future port | assess §"Anti-patterns" #5 + #8 | Stay on egui this plan; budget any Tauri 2 port as a Wave, not a sprint |
| Co-location detection fragile across containers / dev shells | new (this plan) | Allow `HERD_SCOUT_GUI_MODE` env override; document the three valid modes |
| Inventory derivation correctness | [[../wiki/concepts/fms-data-model]] §"Inventory is derived" | Phase-6 derivation test against a multi-log scenario |
| Schema version evolution (no flag day) | [[../wiki/concepts/iroh-docs-fms-schema]] §"Schema evolution" | `_schema=N` per entity, lazy on-read upgrade; never repurpose a key |

## Open Questions

- **iroh-smol-kv crate publication state**: published crate or branch ref? Phase 0 confirms.
- **Withdrawal-period units** on Medical logs: days vs hours? EU/AU regulators typically specify days; lock to `i32 days` for v1.
- **Term taxonomy bootstrap**: ship a default term set (animal types: cattle/sheep/horse + breed sub-terms; treatment categories) or start empty? Suggest: ship a tiny default set in `herd-scout-fms`'s `bootstrap.rs`, never auto-overwrite if present.
- **Group membership model**: store on the Group asset (`group/<id>/member/<animal-id>`) or on the Animal asset (`animal/<id>/group_ref`)? Wiki schema implies the latter via `parent`. Lock to: `asset/<animal-id>/parent → <group-id>`; the Group's member list is derived.
- **iroh 1.0 GA migration**: separate wave when iroh-blobs `#233` closes. Track in inventory.
- **EID hand-off**: where exactly does an EID scan create the Animal? Suggest the EID crate's CLI / phone integration writes via the IPC `CreateAsset` path; no FMS-internal wiring needed in this plan beyond the docstring marker.
- **Multi-farm support**: out of scope — one farm namespace per daemon for v1. Multi-namespace per device is a follow-up.

## Remote IPC bridge — herd-scout/ipc/1 ALPN (2026-06-02, post-Phase-3)

Adjacent to the FMS plan but driven by the same need: an operator
asked how to connect the GUI to the daemon running on `bigdeal`.
The local-only UDS surface didn't have an answer, so this lands as
a sixth ALPN on the daemon's iroh `Router` and a `--daemon
<NodeId>` flag (or `HERD_SCOUT_DAEMON` env) on the GUI.

What shipped:

- New `herd_scout_ipc::REMOTE_IPC_ALPN = b"herd-scout/ipc/1"`.
- `herd-scout-daemon::remote_ipc::RemoteIpcHandler` accepts one
  bi-stream per QUIC connection and embeds it into the daemon's
  existing `from_clients_tx` / `to_clients_tx` channels — remote
  GUIs are indistinguishable from UDS GUIs from the dispatcher's
  perspective, so all the FMS / pairing / upload RPCs Just Work.
- `ipc_predicate` reuses `[control_plane.admins]` for the gate
  (same scope as the admin RPC ALPN — anyone who can drive the
  full GUI surface can already mutate state, and a dedicated
  `ipc_clients` set adds operational complexity without security
  benefit). Self-dial rejected. Audit lines:
  `remote_ipc_rejected`, `remote_ipc_session_open`,
  `remote_ipc_session_close`.
- GUI gained a `--daemon <NodeId>` CLI flag /
  `HERD_SCOUT_DAEMON` env. When set, skips the local UDS and
  auto-spawn paths and dials the daemon's `herd-scout/ipc/1`
  ALPN over iroh. The reader/writer halves were refactored to be
  generic over `AsyncRead`/`AsyncWrite`, so the same dispatch
  code drives both transports.
- GUI gained its own `identity.toml` at
  `<config-dir>/herd-scout-gui/identity.toml` (shape identical to
  herdctl's). The operator's existing herdctl identity can be
  copied here to reuse a single NodeId across CLI and GUI.

Operator workflow for the bigdeal use case:
1. On bigdeal, run the daemon — note its NodeId at boot.
2. On the laptop, `cargo run -p herd-scout-gui` once to mint the
   GUI's identity; note the GUI's NodeId from the log line
   `GUI: local NodeId (must be in daemon's [control_plane.admins])`.
3. ssh into bigdeal, edit `~/.config/herd-scout/control.toml` to add
   the GUI's NodeId under `[[control_plane.admins]]` (or use
   `herdctl admin add-allowed`).
4. On the laptop:
   `cargo run -p herd-scout-gui -- --daemon <bigdeal_node_id>`.

Two new tests in `remote_ipc::handler::tests` verify the predicate
admits admins and rejects self-dial. Workspace test count: 160
(was 158).

## Phase 5 deferral — execution-time decision (2026-06-02)

The Phase 0 audit established that durable iroh-smol-kv does not
ship today: the live `Client::local(topic, config)` is in-memory +
gossip-coupled with a default 60-second `ExpiryConfig::horizon`.
`herd-scout-fms`'s on-disk store is therefore single-device by
design — there is no second-device replica for a farm-namespace
ticket to grant access *to*. QR farm-namespace onboarding adds
value only when (a) durable smol-kv lands upstream and the records
store opens a `Client::local` mirror, or (b) the daemon exposes a
records-exchange protocol on its own ALPN. Both paths reuse the
identity envelope and audit-log infrastructure already in the repo;
the plumbing in this plan stays valid. Phase 5 lands as the
follow-up wave once that decision is made.

## Phase 3 shipped — daemon-side SQLite projection + FTS5 search (2026-06-02)

Phase 3 lands as the daemon-only half of the originally-scoped
work. The "remote-mode GUI runs its own iroh peer" surface is still
gated on Phase 5 (no cross-device record sync to mirror), so
co-location detection (`HERD_SCOUT_GUI_MODE`) stays explicitly
deferred under the same condition: when there's a second peer to
duplicate, that switch becomes meaningful.

What shipped:

- `herd-scout-fms` gained a `projection` Cargo feature (default-on)
  pulling in `rusqlite` 0.32 with the bundled SQLite amalgamation
  (FTS5 enabled by `libsqlite3-sys`'s bundled build).
- New `herd-scout-fms::projection::Projection` module mirrors every
  asset/log into `<data_dir>/projection.sqlite` with tables
  `asset`, `asset_tag`, `log`, `log_asset_ref`, `quantity`, plus a
  standalone FTS5 virtual table `log_fts(log_id UNINDEXED, notes)`.
  Projection is wiped + rebuilt from `records.jsonl` on every
  daemon boot — projection is rebuildable, never the source of
  truth, so there's no migration story.
- `Projection::spawn_subscriber` consumes the `Fms` change stream
  and applies one upsert per `ChangeEvent`. Lagged subscriber →
  rebuild from the in-memory index; no checkpoint state.
- New IPC RPC `ClientMsg::FmsSearchLogs { query, limit }` and a
  daemon dispatcher (`fms_rpc::handle_search_logs`) that runs the
  FTS5 query through the projection, materializes each hit back
  through `Fms::read_log`, and replies via the existing
  `ServerMsg::FmsLogList` (with empty `asset_id` flagging "search"
  vs "asset-scoped").
- egui Records tab gained a search box: type a phrase, press
  Enter or click Search, results render in a 160px scrollable
  grid. "Clear" wipes the local result cache.

Three projection tests cover FTS round-trip (multi-term match +
ranking + empty-query early-out), live sync via the change-bridge,
and rebuild-after-wipe. Workspace test count: 158 (was 155).

Plan deviations preserved: Phase 5 still deferred until durable
smol-kv lands or the daemon owns records-exchange; the `_GUI_MODE`
co-location switch ships when Phase 5 does.

## Phase 0 audit findings — plan deviations (2026-06-02)

Phase 0 read the live iroh-smol-kv source at `~/.cargo/git/checkouts/iroh-smol-kv-0ba0306f243df5a9/35811d0` (branch `iroh-098`) and confirmed the warnings in [[../wiki/concepts/iroh-sync-stack]] and the existing `herd-scout-daemon/src/store/mod.rs` doc-comment.

**Three findings:**

1. **No persistent backend.** `Client::local(topic, config)` is the only constructor; records live in memory with a default `ExpiryConfig::horizon` of ~2 minutes (60s in the `iroh-live` Room actor). Phase-1 deviation: build `herd-scout-fms` as a durable on-disk store today, in a smol-kv-shaped layout (`scope = per-device PublicKey hex`, `key = bytes`, `value = bytes`, `ts_ns = u64`), so a future migration to durable smol-kv is a backend swap, not a crate rewrite.
2. **Outer timestamp is wallclock, not HLC.** `WriteScope::put` calls `util::current_timestamp()` = `SystemTime::now().to_nanos()`. The wiki's HLC mandate cannot be enforced inside smol-kv `SignedValue.timestamp`. Phase-1 deviation: embed an HLC `(ts_ns, author_counter)` *inside* the value bytes for LWW-comparable fields. The on-disk record then carries both the wallclock outer-ts (for a future smol-kv migration) and the HLC-comparable inner-ts (for correct LWW today).
3. **API name mismatch from the wiki sketch.** Real API: `Client::write(SecretKey) -> WriteScope`; `WriteScope::put(key, value)`; `Client::get(scope, key)`; `Client::iter_with_opts(Filter)`; `Subscribe { mode: SubscribeMode::Both, filter: Filter::ALL }`; events are `SubscribeItem::Entry/Expired/CurrentDone`. The crate-internal trait that `herd-scout-fms` exposes mirrors this shape so the future swap is mechanical.

**Net effect on phases:** Phase 1 builds `herd-scout-fms` as a sidecar-backed CRDT-shaped store with an in-memory mirror; Phase 2 keeps the same IPC surface; Phase 3's SQLite projector subscribes to the local store's change stream (functionally identical to subscribing to a smol-kv `Client`). Phases 4–6 are unchanged. The Phase 6 HLC drift test now exercises the in-value HLC, not a (nonexistent) outer-timestamp HLC.

## Sources Consulted

- [[../wiki/concepts/iroh-docs-fms-schema]] — every key-layout, conflict-strategy, blob-ref, schema-evolution, gotcha decision in this plan
- [[../wiki/concepts/fms-data-model]] — Asset/Log/Quantity/Term/Plan primitives + native Rust type sketch + "inventory is derived"
- [[../wiki/concepts/mobile-desktop-architecture]] — write-pattern, sync-engine matrix, anti-patterns
- [[../wiki/concepts/iroh-sync-stack]] — iroh-smol-kv API caveat (drives Phase 0 audit)
- [[../wiki/concepts/livestock-oss-gap-analysis]] — why FMS records are the wedge
- [[../wiki/concepts/herd-scout-positioning]] — what to consume vs build vs skip
- [[../wiki/concepts/fms-feature-taxonomy]] — MVP scope (Animal/Group/Land/Equipment + 5 logs + treatment-with-withdrawal)
- [[../inventory/features/iroh-smol-kv-fms-schema]] — open questions captured during assess
- [[../inventory/watch/iroh-blobs-233-poisoned-store]] — version-pin constraint
- [[../output/assess-herd-scout-2026-06-02]] — P0 prioritization rationale
