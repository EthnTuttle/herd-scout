//! Shared IPC types between `herd-scout-daemon` and `herd-scout-gui`.
//!
//! Wave 6: the desktop monolith has been split into a daemon (owns iroh
//! node + moq + CV) and a GUI (egui frontend). They communicate over a
//! Unix domain socket (Linux/macOS); Windows is currently
//! unsupported for MVP.
//!
//! Wire framing (implemented in each binary's `ipc` module): a 4-byte
//! big-endian length prefix followed by JSON-encoded payloads of the
//! [`ServerMsg`] / [`ClientMsg`] enums. JSON is debuggable on the wire
//! and small enough at the typical control-message rate (~30/s).
//!
//! Frame data (the JPEG bytes in [`ServerMsg::Frame`]) rides the same
//! socket. At 720p / quality 80 the JPEGs run ~50–200 KB at 30 FPS,
//! well within local IPC throughput.

#![deny(missing_debug_implementations)]

use serde::{Deserialize, Serialize};

/// ALPN for the daemon's control plane (Wave 11).
///
/// Registered as a third protocol on the daemon's iroh `Router` alongside
/// `iroh_moq::ALPN` and `iroh_gossip::ALPN`. The daemon accepts
/// bi-directional QUIC streams on this ALPN, gates on a NodeId allowlist,
/// then byte-pumps the stream into local sshd. Clients (`herdctl proxy`)
/// dial this ALPN and pipe stdin/stdout, designed for use as an OpenSSH
/// `ProxyCommand`.
///
/// Versioned: future framing changes get `herd-scout/ssh/2` and old daemons
/// keep accepting v1 for one release.
pub const CONTROL_ALPN: &[u8] = b"herd-scout/ssh/1";

/// ALPN for the daemon's admin RPC plane (Wave 12).
///
/// A fourth ALPN registered on the daemon's iroh `Router`. Authorized
/// peers (entries in `[control_plane.admins]`) open a bi-directional
/// QUIC stream and send a length-prefixed JSON [`AdminClientMsg`]; the
/// daemon replies with one [`AdminServerMsg`] then closes. One RPC per
/// stream.
///
/// Versioned the same way `CONTROL_ALPN` is. v1 framing: 4-byte
/// big-endian length + JSON body, single round-trip per stream.
pub const ADMIN_ALPN: &[u8] = b"herd-scout/admin/1";

/// ALPN for the daemon's batch video upload + processing plane.
///
/// Fifth ALPN registered on the daemon's iroh `Router` (after
/// moq-live, gossip, [`CONTROL_ALPN`], and [`ADMIN_ALPN`]). Authorized
/// peers (entries in `[control_plane.admins]`) open a bi-directional
/// QUIC stream and speak length-prefixed JSON [`UploadClientMsg`] /
/// [`UploadServerMsg`] framing. The clip bytes ride iroh-blobs over
/// the same iroh node — the JSON message wraps the BLAKE3 hash plus
/// metadata; the daemon `get`s the blob via the existing iroh-blobs
/// store.
///
/// Versioned the same way [`CONTROL_ALPN`] / [`ADMIN_ALPN`] are.
pub const UPLOAD_ALPN: &[u8] = b"herd-scout/upload/1";

/// One entry in the daemon's SSH allowlist. `node_id` is a canonical
/// `EndpointId` string; `label` is human-readable and may be empty
/// (legacy entries that pre-date the labeled-schema migration).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedEntry {
    pub node_id: String,
    #[serde(default)]
    pub label: String,
}

/// Reply payload for `AdminClientMsg::Status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusReply {
    pub daemon_version: String,
    pub own_node_id: String,
    pub active_ssh_sessions: u32,
    pub admins_count: u32,
    pub allowed_count: u32,
    /// Wall-clock timestamp of the last successful config swap (boot,
    /// SIGHUP, or admin RPC). Milliseconds since UNIX epoch.
    pub last_reload_unix_ms: u64,
    /// Source of the last reload: `"boot"`, `"sighup"`, or `"admin_rpc"`.
    pub last_reload_source: String,
    /// `herd-scout-identity` envelope schema version the daemon was
    /// built against. Useful for the phone client to surface "your
    /// daemon understands schema N."
    pub identity_schema_version: u32,
}

/// One audit record on the wire. The daemon's on-disk JSONL log is the
/// source of truth; `TailAudit` returns these to the admin app.
///
/// `kind` is a short string identifier (e.g. `"ssh_session_open"`).
/// `details` is a free-form JSON object holding kind-specific fields
/// (`target_node_id`, `bytes_to_sshd`, `duration_ms`, …) — the phone
/// renders unknown kinds as a generic gray-bullet row, which keeps the
/// wire format forward-compatible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub schema_version: u32,
    pub ts_ms: u64,
    pub kind: String,
    #[serde(default)]
    pub actor_node_id: Option<String>,
    #[serde(default)]
    pub actor_label: Option<String>,
    #[serde(default)]
    pub details: serde_json::Value,
}

// =====================================================================
// FMS records (Phase 2 of plan-fms-schema-and-records-2026-06-02)
// =====================================================================

/// Asset kinds the FMS records layer tracks. Mirrors
/// `herd_scout_fms::AssetKind` but kept as a plain enum here so the
/// IPC crate doesn't depend on `herd-scout-fms` (one-way dependency:
/// daemon → fms, ipc independent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKindWire {
    Animal,
    Group,
    Land,
    Equipment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKindWire {
    Observation,
    Medical,
    Movement,
    Weight,
    Birth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuantityWire {
    pub measure: String,
    pub value: f64,
    pub unit: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// Mutable scalar fields on an asset. Used by
/// [`ClientMsg::FmsUpdateAssetField`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetFieldWire {
    Name,
    Notes,
    Geom,
    Parent,
}

/// Asset payload returned to the GUI in [`ServerMsg::FmsAsset`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetWire {
    /// ULID, base32 (the standard 26-char form).
    pub id: String,
    pub kind: AssetKindWire,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub archived: bool,
    /// Term ULIDs.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Log payload returned to the GUI in [`ServerMsg::FmsLog`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogWire {
    pub id: String,
    pub kind: LogKindWire,
    /// Wallclock-ish nanos (HLC `ts_ns` half). Cosmetic — the daemon's
    /// HLC drives ordering; the GUI shows this for human display.
    pub ts_ns: u64,
    #[serde(default)]
    pub asset_refs: Vec<String>,
    #[serde(default)]
    pub quantities: Vec<QuantityWire>,
    #[serde(default)]
    pub notes: String,
}

/// Per-write change notification pushed to every connected GUI on a
/// successful FMS commit. The IPC server fans these out to all
/// connected GUIs from a broadcast channel; per-GUI projections refresh
/// the affected list views.
///
/// `kind_hint` is `Some(asset_kind)` when the changed key belongs to
/// an asset entity; the GUI uses it to skip refreshes for unrelated
/// kinds without parsing the key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FmsChangeWire {
    pub scope: String,
    /// UTF-8 key (smol-kv-shaped), e.g. `asset/<ULID>/name`.
    pub key: String,
    /// Wallclock-ish nanos at write time.
    pub ts_ns: u64,
    /// `lww` / `add_wins_set` / `append_only`.
    pub strategy: String,
    /// Cheap dispatch hint — `Some("animal" | "group" | "land" |
    /// "equipment")` when key targets an asset; `Some("log")` when key
    /// targets a log; `None` otherwise.
    #[serde(default)]
    pub entity_hint: Option<String>,
}

/// Lifecycle of a queued upload. Surfaced to the GUI via
/// [`ServerMsg::UploadStatus`] and to admin clients via
/// [`UploadServerMsg::QueueSnapshot`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UploadState {
    Queued,
    Decoding,
    Done,
    Failed { reason: String },
}

/// One row in the upload queue. The BLAKE3 hash is the canonical id;
/// `filename` is human-readable and may collide across uploads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadEntry {
    pub blake3_hex: String,
    pub filename: String,
    pub size_bytes: u64,
    pub state: UploadState,
    pub queued_ts_ms: u64,
    #[serde(default)]
    pub started_ts_ms: Option<u64>,
    #[serde(default)]
    pub finished_ts_ms: Option<u64>,
}

/// Headline numbers from a finished clip's `report.json`. Inlined in
/// [`ServerMsg::UploadStatus`] so the GUI can render the queue panel
/// without a second round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadSummaryInline {
    pub median_active_count_total: u32,
    pub bootstrap_ci_95_total: [u32; 2],
    pub horse: u32,
    pub sheep: u32,
    pub cow: u32,
    pub frame_count: u32,
    pub duration_ms: u64,
}

/// Phase 2 upload-plane requests (admin client → daemon over [`UPLOAD_ALPN`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UploadClientMsg {
    /// Announce a clip the client has already imported into iroh-blobs
    /// under `blake3_hex`. The daemon `get`s the blob, stages it
    /// under `<data_dir>/uploads/<blake3>/`, and queues processing.
    /// Replies with [`UploadServerMsg::Accepted`] or one of the
    /// `Rejected*` variants.
    Push {
        filename: String,
        size_bytes: u64,
        blake3_hex: String,
    },
    /// Return the current queue snapshot (pending + recently
    /// finished entries; daemon decides retention).
    ListQueue,
    /// Drop a queued (not-yet-`Decoding`) entry. No-op if the entry
    /// is not in the queue or already past `Queued`.
    CancelQueued { blake3_hex: String },
    /// Read the persisted `report.json` for a finished clip.
    FetchReport { blake3_hex: String },
}

/// Phase 2 upload-plane replies (daemon → admin client).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UploadServerMsg {
    Accepted { blake3_hex: String },
    RejectedTooBig { actual_bytes: u64, max_bytes: u64 },
    RejectedHashMismatch { reported: String, computed: String },
    QueueSnapshot { entries: Vec<UploadEntry> },
    /// Body is the raw bytes of the clip's `report.json`. Wrapped in
    /// a `Vec<u8>` (base64 on the wire — reuses the
    /// [`ServerMsg::Frame`] base64 helper) so the daemon doesn't need
    /// to parse and re-serialize.
    Report {
        blake3_hex: String,
        #[serde(with = "base64_bytes")]
        json_bytes: Vec<u8>,
    },
    Ok,
    Error { code: String, message: String },
}

/// Wave 12 admin-plane requests (phone → daemon).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminClientMsg {
    /// Return the current SSH allowlist.
    ListAllowed,
    /// Add a new entry to the SSH allowlist. `node_id` must parse as a
    /// canonical `EndpointId`; `label` is required (cannot be empty).
    AddAllowed { node_id: String, label: String },
    /// Remove an entry by `node_id`. No-op if not present (returns
    /// `Error { code: "not_found" }`).
    RemoveAllowed { node_id: String },
    /// Snapshot of daemon state for the admin app's status header.
    Status,
    /// Read up to `last_n` audit records (capped server-side at 500),
    /// optionally filtering to records strictly older than
    /// `before_ts_ms`. Used by the admin app's "From daemon" history
    /// view; pagination = call again with the oldest record's
    /// `ts_ms` as the next `before_ts_ms`.
    TailAudit {
        last_n: u32,
        #[serde(default)]
        before_ts_ms: Option<u64>,
    },
}

/// Wave 12 admin-plane replies (daemon → phone).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminServerMsg {
    Allowed { entries: Vec<AllowedEntry> },
    Status(StatusReply),
    AuditTail { records: Vec<AuditRecord>, eof: bool },
    Ok,
    Error { code: String, message: String },
}

/// Mirror of the daemon's connection-status state machine. This used to
/// live in `desktop/src/stream.rs`; the GUI now sees only the
/// daemon-reported value.
///
/// `AwaitingTicket` from Wave 5C is intentionally absent: the daemon
/// mints its rendezvous ticket synchronously on boot before accepting
/// GUI connections, so the GUI never observes a no-ticket state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// Pairing ticket minted; daemon awaiting a phone session.
    Idle,
    /// A session has connected; daemon is subscribing to the broadcast.
    Connecting,
    /// Subscribed and decoding frames.
    Connected,
    /// The previous subscription failed; the loop is sleeping before retrying.
    Reconnecting { reason: String },
    /// The daemon has stopped permanently.
    Stopped,
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self::Idle
    }
}

impl ConnectionStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting { .. } => "reconnecting",
            Self::Stopped => "stopped",
        }
    }
}

/// CV detection on the wire. `class` matches the index returned by
/// `CocoClass::label_index()` so we don't need to serialise a string
/// per box at 30 FPS.
///
/// 0 = horse, 1 = sheep, 2 = cow.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DetWire {
    pub class: u8,
    /// `[x1, y1, x2, y2]` in source-frame pixel space.
    pub bbox: [f32; 4],
    pub score: f32,
    /// Persistent ByteTrack id for this detection across frames.
    /// `None` when the tracker has not yet attached an ID.
    #[serde(default)]
    pub track_id: Option<u32>,
}

impl DetWire {
    pub fn class_label(&self) -> &'static str {
        match self.class {
            0 => "horse",
            1 => "sheep",
            2 => "cow",
            _ => "?",
        }
    }

    /// Per-class colour (RGB) for overlay rendering. Mirrors
    /// `desktop/src/cv/model.rs::CocoClass::rgb`.
    pub fn class_rgb(&self) -> (u8, u8, u8) {
        match self.class {
            0 => (0, 200, 255),    // horse / cyan
            1 => (240, 50, 230),   // sheep / magenta
            2 => (255, 165, 0),    // cow / orange
            _ => (200, 200, 200),
        }
    }
}

/// Mirror of the desktop crate's `ClassCounts` for the rolling-window
/// counts panel.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ClassCountsWire {
    pub horse: u32,
    pub sheep: u32,
    pub cow: u32,
}

/// Messages sent daemon → GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// First message after the GUI connects. Lets the GUI confirm
    /// daemon version + capability bits before subscribing to
    /// frame/detection traffic.
    Hello {
        daemon_version: String,
        capabilities: Vec<String>,
    },
    /// The current pairing ticket (a serialised `LiveTicket`).
    /// Pushed on connect (so the GUI can render the QR immediately) and
    /// any time the daemon re-mints.
    Pairing { ticket: String },
    /// Periodic status update. `last_frame_age_ms` is `None` when no
    /// frame has been received yet.
    Status {
        state: ConnectionStatus,
        last_frame_age_ms: Option<u64>,
    },
    /// A JPEG-encoded preview frame.
    ///
    /// `pts_ms` is the source frame timestamp in milliseconds (so the
    /// GUI can dedupe and so detections can be correlated to a
    /// specific frame). `width`/`height` are the JPEG's encoded
    /// dimensions, NOT the source — the daemon downscales to a 720p
    /// preview before encoding to keep wire bandwidth bounded.
    Frame {
        width: u16,
        height: u16,
        pts_ms: u64,
        /// `Some(blake3_hex)` when this frame originates from an
        /// upload-replay; `None` for live phone broadcast frames.
        #[serde(default)]
        clip_id: Option<String>,
        #[serde(with = "base64_bytes")]
        jpeg: Vec<u8>,
    },
    /// Detections for a single frame, identified by `frame_pts_ms`.
    Detections {
        frame_pts_ms: u64,
        dets: Vec<DetWire>,
        counts: ClassCountsWire,
        /// `Some(blake3_hex)` when these detections originate from an
        /// upload-replay; `None` for live phone broadcast frames.
        #[serde(default)]
        clip_id: Option<String>,
    },
    /// Lifecycle update for a queued upload. The GUI uses this to
    /// drive the "Uploads" side panel.
    UploadStatus {
        blake3_hex: String,
        filename: String,
        state: UploadState,
        progress_pct: u8,
        #[serde(default)]
        eta_ms: Option<u64>,
        /// Inlined headline from the persisted report; populated only
        /// when `state == Done`. Clients that want full detail call
        /// [`UploadClientMsg::FetchReport`].
        #[serde(default)]
        summary: Option<UploadSummaryInline>,
    },
    /// CV banner state (e.g. "CV disabled: shape mismatch"). Empty
    /// `text` and `disabled = false` clears the banner.
    CvBanner {
        text: Option<String>,
        disabled: bool,
    },

    // === FMS RPC replies (Phase 2) ===
    /// Reply to [`ClientMsg::FmsCreateAsset`] / `FmsReadAsset` /
    /// `FmsUpdateAssetField` / `FmsArchiveAsset`. `request_id` echoes
    /// the client's request id so out-of-order responses correlate.
    FmsAsset {
        request_id: u64,
        /// `Some` when the request resolved to a real asset (read /
        /// create / update); `None` when the asset was not found.
        asset: Option<AssetWire>,
    },
    /// Reply to [`ClientMsg::FmsListAssets`].
    FmsAssetList {
        request_id: u64,
        kind: AssetKindWire,
        assets: Vec<AssetWire>,
    },
    /// Reply to [`ClientMsg::FmsAppendLog`] / `FmsReadLog`.
    FmsLog {
        request_id: u64,
        log: Option<LogWire>,
    },
    /// Reply to [`ClientMsg::FmsListLogsForAsset`].
    FmsLogList {
        request_id: u64,
        asset_id: String,
        logs: Vec<LogWire>,
    },
    /// Live change notification. Pushed to every connected GUI on
    /// every FMS commit. Not request-correlated.
    FmsChange { event: FmsChangeWire },
    /// Generic FMS error reply. `request_id` echoes the failing
    /// request; `message` is human-readable.
    FmsError {
        request_id: u64,
        code: String,
        message: String,
    },
}

/// Messages sent GUI → daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First message after the GUI connects.
    Hello { gui_version: String },
    /// Ask the daemon to (re-)mint the pairing ticket. Daemon will
    /// reply with a fresh `ServerMsg::Pairing`.
    RequestPairing,
    /// Connect using a manually-supplied ticket (the "Advanced /
    /// Paste" path on the pairing screen, plus the `--ticket` CLI
    /// fallback). The daemon will dial the ticket's endpoint and
    /// subscribe to its broadcast.
    ConnectTicket { ticket: String },
    /// Forget the saved ticket; the daemon will mint a fresh one on
    /// the next request.
    ClearSavedTicket,
    /// User pressed "Cancel" on the reconnect overlay. The daemon should
    /// drop any in-flight session, return to `Idle`, and re-publish the
    /// pairing ticket so the GUI can re-render the QR. The daemon's
    /// `incoming_sessions` listener stays alive — the next phone dial
    /// will be accepted as a fresh session.
    CancelStream,
    /// Ask the daemon to shut down (graceful — actor drains).
    Shutdown,
    /// GUI → daemon: stage a local file for upload. The daemon
    /// imports the bytes into iroh-blobs locally (no network) and
    /// kicks off the upload pipeline as if it had received the file
    /// over [`UPLOAD_ALPN`]. `path` must be readable by the daemon
    /// process; co-located GUI + daemon is the typical case. The
    /// daemon will reply via the existing `ServerMsg::UploadStatus`
    /// stream.
    UploadHandoff {
        path: String,
        blake3_hex: String,
        size_bytes: u64,
    },
    /// GUI → daemon: cancel a queued upload by BLAKE3 hex prefix
    /// (full hash; truncation is a GUI-side affordance).
    UploadCancel { blake3_hex: String },

    // === FMS record CRUD (Phase 2) ===
    /// Create a new asset.
    FmsCreateAsset {
        request_id: u64,
        kind: AssetKindWire,
        name: String,
    },
    /// Read an asset by id.
    FmsReadAsset {
        request_id: u64,
        id: String,
    },
    /// Update a single mutable scalar field on an asset.
    FmsUpdateAssetField {
        request_id: u64,
        id: String,
        field: AssetFieldWire,
        /// Bytes go on the wire base64-url-no-pad encoded so JSON
        /// stays printable.
        #[serde(with = "base64_bytes")]
        value: Vec<u8>,
    },
    /// Soft-archive an asset.
    FmsArchiveAsset {
        request_id: u64,
        id: String,
    },
    /// List assets by kind (optionally including archived).
    FmsListAssets {
        request_id: u64,
        kind: AssetKindWire,
        #[serde(default)]
        include_archived: bool,
    },
    /// Append a log entry. `id` is a fresh ULID picked by the GUI
    /// (so the GUI can render the log immediately and reconcile on
    /// the FmsLog reply).
    FmsAppendLog {
        request_id: u64,
        id: String,
        kind: LogKindWire,
        /// Cosmetic display-time wallclock nanos. The daemon stamps
        /// the authoritative HLC.
        ts_ns: u64,
        #[serde(default)]
        asset_refs: Vec<String>,
        #[serde(default)]
        quantities: Vec<QuantityWire>,
        #[serde(default)]
        notes: String,
    },
    /// Read a log by id.
    FmsReadLog {
        request_id: u64,
        id: String,
    },
    /// List logs that reference the given asset.
    FmsListLogsForAsset {
        request_id: u64,
        asset_id: String,
    },
    /// Add or remove a tag (term reference) on an asset. `present` =
    /// true writes the add-wins-set entry; `false` writes the
    /// tombstone.
    FmsTagAsset {
        request_id: u64,
        asset_id: String,
        term_id: String,
        present: bool,
    },
    /// Plan-FMS Phase 3b: full-text search across log notes.
    /// Returns up to `limit` matching logs as `ServerMsg::FmsLogList`
    /// with `asset_id` set to an empty string (the search isn't
    /// scoped to a single asset). The daemon's projection answers
    /// the query; if the projection is unavailable the daemon
    /// replies with `FmsError`.
    FmsSearchLogs {
        request_id: u64,
        /// FTS5 MATCH expression. Leading/trailing whitespace
        /// trimmed by the daemon; empty queries return zero hits.
        query: String,
        /// Max results. Daemon may cap further internally.
        limit: u32,
    },
}

mod base64_bytes {
    //! Serde helper that JSON-encodes `Vec<u8>` as base64-url (no pad).
    //! Avoids JSON-array-of-numbers for binary payloads which makes the
    //! wire ~3x larger and forces character-by-character UTF-8
    //! validation on every byte.

    use serde::{Deserialize, Deserializer, Serializer};

    const ALPHA: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    pub fn serialize<S>(bytes: &Vec<u8>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
        let mut i = 0;
        while i + 3 <= bytes.len() {
            let n = ((bytes[i] as u32) << 16)
                | ((bytes[i + 1] as u32) << 8)
                | (bytes[i + 2] as u32);
            out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
            out.push(ALPHA[((n >> 6) & 0x3f) as usize] as char);
            out.push(ALPHA[(n & 0x3f) as usize] as char);
            i += 3;
        }
        let rem = bytes.len() - i;
        if rem == 1 {
            let n = (bytes[i] as u32) << 16;
            out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        } else if rem == 2 {
            let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
            out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
            out.push(ALPHA[((n >> 6) & 0x3f) as usize] as char);
        }
        ser.serialize_str(&out)
    }

    pub fn deserialize<'de, D>(de: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: String = String::deserialize(de)?;
        decode(&s).map_err(serde::de::Error::custom)
    }

    fn decode(input: &str) -> Result<Vec<u8>, &'static str> {
        let bytes = input.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() * 3 / 4 + 2);
        let mut buf: u32 = 0;
        let mut bits: u32 = 0;
        for &b in bytes {
            let v = match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'-' => 62,
                b'_' => 63,
                b'=' => continue,
                _ => return Err("invalid base64url byte"),
            };
            buf = (buf << 6) | u32::from(v);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
                buf &= (1u32 << bits) - 1;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_hello_roundtrips_through_json() {
        let msg = ServerMsg::Hello {
            daemon_version: "0.1.0".to_string(),
            capabilities: vec!["jpeg-preview".to_string()],
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            ServerMsg::Hello { daemon_version, .. } => assert_eq!(daemon_version, "0.1.0"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn frame_jpeg_bytes_roundtrip() {
        let raw = vec![0u8, 1, 2, 3, 4, 0xff, 0xfe, 0x80, 0x7f];
        let msg = ServerMsg::Frame {
            width: 1280,
            height: 720,
            pts_ms: 12345,
            clip_id: None,
            jpeg: raw.clone(),
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            ServerMsg::Frame { jpeg, .. } => assert_eq!(jpeg, raw),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn client_connect_ticket_roundtrips() {
        let msg = ClientMsg::ConnectTicket {
            ticket: "iroh-live:abc".to_string(),
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: ClientMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            ClientMsg::ConnectTicket { ticket } => assert_eq!(ticket, "iroh-live:abc"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn admin_client_msg_roundtrips() {
        let cases = [
            AdminClientMsg::ListAllowed,
            AdminClientMsg::Status,
            AdminClientMsg::AddAllowed {
                node_id: "abc".into(),
                label: "phone".into(),
            },
            AdminClientMsg::RemoveAllowed {
                node_id: "abc".into(),
            },
        ];
        for msg in cases {
            let s = serde_json::to_string(&msg).unwrap();
            let parsed: AdminClientMsg = serde_json::from_str(&s).unwrap();
            // shallow comparison via Debug
            assert_eq!(format!("{msg:?}"), format!("{parsed:?}"));
        }
    }

    #[test]
    fn admin_server_msg_roundtrips() {
        let entries = vec![AllowedEntry {
            node_id: "abc".into(),
            label: "phone".into(),
        }];
        let s = serde_json::to_string(&AdminServerMsg::Allowed { entries }).unwrap();
        match serde_json::from_str::<AdminServerMsg>(&s).unwrap() {
            AdminServerMsg::Allowed { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].node_id, "abc");
                assert_eq!(entries[0].label, "phone");
            }
            _ => panic!("wrong variant"),
        }

        let status = StatusReply {
            daemon_version: "0.1.0".into(),
            own_node_id: "xyz".into(),
            active_ssh_sessions: 2,
            admins_count: 1,
            allowed_count: 3,
            last_reload_unix_ms: 1717000000000,
            last_reload_source: "boot".into(),
            identity_schema_version: 1,
        };
        let s = serde_json::to_string(&AdminServerMsg::Status(status)).unwrap();
        match serde_json::from_str::<AdminServerMsg>(&s).unwrap() {
            AdminServerMsg::Status(r) => {
                assert_eq!(r.daemon_version, "0.1.0");
                assert_eq!(r.active_ssh_sessions, 2);
                assert_eq!(r.last_reload_source, "boot");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn upload_client_msg_roundtrips() {
        let cases = [
            UploadClientMsg::Push {
                filename: "drone-flyover.mp4".into(),
                size_bytes: 12_345_678,
                blake3_hex: "9c2f".repeat(16),
            },
            UploadClientMsg::ListQueue,
            UploadClientMsg::CancelQueued {
                blake3_hex: "9c2f".repeat(16),
            },
            UploadClientMsg::FetchReport {
                blake3_hex: "9c2f".repeat(16),
            },
        ];
        for msg in cases {
            let s = serde_json::to_string(&msg).unwrap();
            let parsed: UploadClientMsg = serde_json::from_str(&s).unwrap();
            assert_eq!(format!("{msg:?}"), format!("{parsed:?}"));
        }
    }

    #[test]
    fn upload_server_msg_accepted_roundtrips() {
        let msg = UploadServerMsg::Accepted {
            blake3_hex: "ab".repeat(32),
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: UploadServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            UploadServerMsg::Accepted { blake3_hex } => {
                assert_eq!(blake3_hex, "ab".repeat(32));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn upload_server_msg_rejected_too_big_roundtrips() {
        let msg = UploadServerMsg::RejectedTooBig {
            actual_bytes: 3_000_000_000,
            max_bytes: 2_147_483_648,
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: UploadServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            UploadServerMsg::RejectedTooBig {
                actual_bytes,
                max_bytes,
            } => {
                assert_eq!(actual_bytes, 3_000_000_000);
                assert_eq!(max_bytes, 2_147_483_648);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn upload_server_msg_rejected_hash_mismatch_roundtrips() {
        let msg = UploadServerMsg::RejectedHashMismatch {
            reported: "aa".repeat(32),
            computed: "bb".repeat(32),
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: UploadServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            UploadServerMsg::RejectedHashMismatch { reported, computed } => {
                assert_eq!(reported, "aa".repeat(32));
                assert_eq!(computed, "bb".repeat(32));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn upload_server_msg_queue_snapshot_roundtrips() {
        let entry = UploadEntry {
            blake3_hex: "9c2f".repeat(16),
            filename: "clip.mp4".into(),
            size_bytes: 1024,
            state: UploadState::Queued,
            queued_ts_ms: 1_717_000_000_000,
            started_ts_ms: None,
            finished_ts_ms: None,
        };
        let msg = UploadServerMsg::QueueSnapshot {
            entries: vec![entry.clone()],
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: UploadServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            UploadServerMsg::QueueSnapshot { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].blake3_hex, entry.blake3_hex);
                assert_eq!(entries[0].filename, "clip.mp4");
                assert_eq!(entries[0].state, UploadState::Queued);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn upload_server_msg_report_roundtrips() {
        let raw = br#"{"schema_version":1,"clip_id":"abc"}"#.to_vec();
        let msg = UploadServerMsg::Report {
            blake3_hex: "9c2f".repeat(16),
            json_bytes: raw.clone(),
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: UploadServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            UploadServerMsg::Report {
                blake3_hex,
                json_bytes,
            } => {
                assert_eq!(blake3_hex, "9c2f".repeat(16));
                assert_eq!(json_bytes, raw);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn upload_server_msg_ok_and_error_roundtrip() {
        let s = serde_json::to_string(&UploadServerMsg::Ok).unwrap();
        match serde_json::from_str::<UploadServerMsg>(&s).unwrap() {
            UploadServerMsg::Ok => {}
            _ => panic!("wrong variant"),
        }

        let msg = UploadServerMsg::Error {
            code: "hash_mismatch".into(),
            message: "computed != reported".into(),
        };
        let s = serde_json::to_string(&msg).unwrap();
        match serde_json::from_str::<UploadServerMsg>(&s).unwrap() {
            UploadServerMsg::Error { code, message } => {
                assert_eq!(code, "hash_mismatch");
                assert_eq!(message, "computed != reported");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn server_upload_status_queued_roundtrips() {
        let msg = ServerMsg::UploadStatus {
            blake3_hex: "9c2f".repeat(16),
            filename: "clip.mp4".into(),
            state: UploadState::Queued,
            progress_pct: 0,
            eta_ms: None,
            summary: None,
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            ServerMsg::UploadStatus {
                state, progress_pct, ..
            } => {
                assert_eq!(state, UploadState::Queued);
                assert_eq!(progress_pct, 0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn server_upload_status_decoding_roundtrips() {
        let msg = ServerMsg::UploadStatus {
            blake3_hex: "9c2f".repeat(16),
            filename: "clip.mp4".into(),
            state: UploadState::Decoding,
            progress_pct: 42,
            eta_ms: Some(15_000),
            summary: None,
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            ServerMsg::UploadStatus {
                state,
                progress_pct,
                eta_ms,
                ..
            } => {
                assert_eq!(state, UploadState::Decoding);
                assert_eq!(progress_pct, 42);
                assert_eq!(eta_ms, Some(15_000));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn server_upload_status_done_roundtrips() {
        let summary = UploadSummaryInline {
            median_active_count_total: 47,
            bootstrap_ci_95_total: [44, 51],
            horse: 0,
            sheep: 0,
            cow: 47,
            frame_count: 2624,
            duration_ms: 87_520,
        };
        let msg = ServerMsg::UploadStatus {
            blake3_hex: "9c2f".repeat(16),
            filename: "clip.mp4".into(),
            state: UploadState::Done,
            progress_pct: 100,
            eta_ms: None,
            summary: Some(summary),
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            ServerMsg::UploadStatus {
                state,
                progress_pct,
                summary,
                ..
            } => {
                assert_eq!(state, UploadState::Done);
                assert_eq!(progress_pct, 100);
                let s = summary.expect("summary should be present");
                assert_eq!(s.median_active_count_total, 47);
                assert_eq!(s.bootstrap_ci_95_total, [44, 51]);
                assert_eq!(s.cow, 47);
                assert_eq!(s.frame_count, 2624);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn client_upload_handoff_roundtrips() {
        let msg = ClientMsg::UploadHandoff {
            path: "/tmp/clip.mp4".into(),
            blake3_hex: "9c2f".repeat(16),
            size_bytes: 12_345_678,
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: ClientMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            ClientMsg::UploadHandoff {
                path,
                blake3_hex,
                size_bytes,
            } => {
                assert_eq!(path, "/tmp/clip.mp4");
                assert_eq!(blake3_hex, "9c2f".repeat(16));
                assert_eq!(size_bytes, 12_345_678);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn client_upload_cancel_roundtrips() {
        let msg = ClientMsg::UploadCancel {
            blake3_hex: "9c2f".repeat(16),
        };
        let s = serde_json::to_string(&msg).unwrap();
        let parsed: ClientMsg = serde_json::from_str(&s).unwrap();
        match parsed {
            ClientMsg::UploadCancel { blake3_hex } => {
                assert_eq!(blake3_hex, "9c2f".repeat(16));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn legacy_frame_without_clip_id_still_deserializes() {
        // Old wire format: no `clip_id` field. `#[serde(default)]`
        // must deserialize this cleanly into `ServerMsg::Frame` with
        // `clip_id == None`.
        let legacy = r#"{
            "type": "frame",
            "width": 1280,
            "height": 720,
            "pts_ms": 12345,
            "jpeg": "AAEC"
        }"#;
        let parsed: ServerMsg = serde_json::from_str(legacy).unwrap();
        match parsed {
            ServerMsg::Frame {
                width,
                height,
                pts_ms,
                clip_id,
                jpeg,
            } => {
                assert_eq!(width, 1280);
                assert_eq!(height, 720);
                assert_eq!(pts_ms, 12345);
                assert_eq!(clip_id, None);
                assert_eq!(jpeg, vec![0u8, 1, 2]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn legacy_detections_without_clip_id_still_deserializes() {
        // Sibling of `legacy_frame_without_clip_id_still_deserializes`.
        // Old wire format: no `clip_id` field on `Detections`.
        // `#[serde(default)]` must deserialize this cleanly into
        // `ServerMsg::Detections` with `clip_id == None`.
        let legacy = r#"{
            "type": "detections",
            "frame_pts_ms": 4242,
            "dets": [
                {
                    "class": 2,
                    "bbox": [10.0, 20.0, 110.0, 120.0],
                    "score": 0.91,
                    "track_id": 7
                }
            ],
            "counts": { "horse": 0, "sheep": 0, "cow": 1 }
        }"#;
        let parsed: ServerMsg = serde_json::from_str(legacy).unwrap();
        match parsed {
            ServerMsg::Detections {
                frame_pts_ms,
                dets,
                counts,
                clip_id,
            } => {
                assert_eq!(frame_pts_ms, 4242);
                assert_eq!(dets.len(), 1);
                assert_eq!(dets[0].class, 2);
                assert_eq!(dets[0].track_id, Some(7));
                assert_eq!(counts.cow, 1);
                assert_eq!(clip_id, None);
            }
            _ => panic!("wrong variant"),
        }
    }
}
