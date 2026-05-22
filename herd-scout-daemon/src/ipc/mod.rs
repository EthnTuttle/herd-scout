//! IPC layer for the daemon: a Unix-domain-socket server that speaks
//! length-prefixed JSON-encoded `ServerMsg`/`ClientMsg` to the GUI.
//!
//! - [`frame`] — 4-byte BE length prefix codec.
//! - [`server`] — bind + accept loop + per-connection task.
//! - [`socket_path`] — derive the daemon-socket path from
//!   `directories::ProjectDirs`.

pub mod frame;
pub mod server;

use std::path::PathBuf;

use anyhow::{Result, anyhow};

/// Returns the path the daemon should bind its UDS at.
///
/// macOS: `~/Library/Application Support/net.herd-scout.herd-scout/daemon.sock`
/// Linux: `$XDG_RUNTIME_DIR/herd-scout/daemon.sock` (fallback to data dir)
pub fn socket_path() -> Result<PathBuf> {
    // Prefer XDG_RUNTIME_DIR on Linux for performance and lifetime
    // (it gets cleaned up on logout) but fall back to the data dir
    // on macOS/Windows where it's typically empty.
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let mut p = PathBuf::from(xdg);
        p.push("herd-scout");
        return Ok(p.join("daemon.sock"));
    }
    let dirs = directories::ProjectDirs::from("net", "herd-scout", "herd-scout")
        .ok_or_else(|| anyhow!("no user-data directory available"))?;
    Ok(dirs.data_dir().join("daemon.sock"))
}
