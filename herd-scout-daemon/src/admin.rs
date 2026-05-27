//! Wave 12 — admin RPC plane over a fourth iroh ALPN.
//!
//! Registers `herd_scout_ipc::ADMIN_ALPN` on the same `Router` that
//! already serves moq + gossip + the SSH bridge. Authorized peers
//! (entries in `[control_plane.admins]`) open a bi-stream and exchange
//! one [`AdminClientMsg`] / [`AdminServerMsg`] pair, then close.
//!
//! Phase 1 ships only the gate: connections from non-admin peers are
//! dropped after a `WARN`. Real RPC handlers land in Phase 2/3.
//!
//! See `.wiki/output/plan-android-admin-allowlist-app-2026-05-27.md`.

mod handler;

pub(crate) use handler::AdminHandler;
