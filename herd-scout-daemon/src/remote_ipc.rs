//! Remote IPC bridge over `herd_scout_ipc::REMOTE_IPC_ALPN`.
//!
//! Sixth ALPN on the daemon's iroh `Router`. Authorized peers
//! (`[control_plane.admins]`) open one bi-directional QUIC stream
//! that speaks the same length-prefixed JSON `ClientMsg` /
//! `ServerMsg` framing the local UDS does. The handler embeds the
//! stream into the daemon's `from_clients_tx` /
//! `to_clients_tx` channels so the rest of the daemon can't tell
//! local-UDS-GUI from remote-iroh-GUI apart.
//!
//! ## Why we reuse the admins allowlist
//!
//! A GUI session has the full daemon surface: read everything,
//! create records, archive assets, append logs, search. That's the
//! same surface the admin app exposes minus the SSH-allowlist
//! mutations. Reusing `[control_plane.admins]` is the right
//! least-privilege scope; we don't introduce a third allowlist
//! ("ipc_clients") because there's no operational case where a
//! peer should have GUI-level access without admin-level access.
//!
//! ## Threat model
//!
//! - Peer-NodeId in `[control_plane.admins]`: can read every record,
//!   create/archive/tag assets, append logs, full-text search.
//! - Peer-NodeId NOT in `[control_plane.admins]`: dial rejected at
//!   the router layer via `AccessLimit::new(handler, predicate)`.
//!   `accept` is never reached.
//! - Self-dial (own NodeId): rejected by the predicate.
//!
//! Wire shape is identical to the UDS so the GUI's existing
//! reader/writer split works against either transport — the only
//! difference at the GUI is `IoSplit` (Unix socket vs QUIC bi-stream).

mod handler;

pub use handler::{ipc_predicate, RemoteIpcHandler};
