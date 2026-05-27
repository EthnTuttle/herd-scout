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
}

/// Wave 12 admin-plane replies (daemon → phone).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminServerMsg {
    Allowed { entries: Vec<AllowedEntry> },
    Status(StatusReply),
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
        #[serde(with = "base64_bytes")]
        jpeg: Vec<u8>,
    },
    /// Detections for a single frame, identified by `frame_pts_ms`.
    Detections {
        frame_pts_ms: u64,
        dets: Vec<DetWire>,
        counts: ClassCountsWire,
    },
    /// CV banner state (e.g. "CV disabled: shape mismatch"). Empty
    /// `text` and `disabled = false` clears the banner.
    CvBanner {
        text: Option<String>,
        disabled: bool,
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
}
