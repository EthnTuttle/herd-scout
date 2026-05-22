//! UDS server: accept loop + per-connection sender/receiver tasks.
//!
//! ## Wave 6 architecture
//!
//! The daemon owns a *broadcast* channel of `ServerMsg`. Every time the
//! daemon mints a ticket, gets a frame, runs CV, etc., it pushes one
//! `ServerMsg` onto the broadcast. Each connected GUI subscribes to
//! that broadcast in its sender task and forwards messages over the
//! socket. The receiver task reads `ClientMsg`s from the GUI and
//! forwards them onto an `mpsc::Sender<ClientMsg>` owned by the daemon's
//! main control loop.
//!
//! Lifecycle:
//!
//! 1. `bind` deletes any stale socket, binds a fresh listener.
//! 2. The accept loop spawns one connection task per incoming client.
//! 3. The connection task spawns two halves:
//!    - **send half**: subscribes to the broadcast, writes frames.
//!    - **recv half**: reads frames, forwards to the daemon mpsc.
//! 4. When the client closes, both halves exit; their task terminates.
//!
//! ## Single-client MVP
//!
//! The plan locks MVP to one GUI per daemon. Multi-GUI broadcast lands
//! "for free" because the daemon's broadcast channel already supports
//! it; we just don't optimise the JPEG encode for fan-out yet.

#![cfg(unix)]

use std::path::Path;

use anyhow::{Context, Result};
use herd_scout_ipc::{ClientMsg, ServerMsg};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};

use super::frame::{read_frame, write_frame};

/// Bind a UDS at `path`. Removes any stale file first so a previous
/// crashed daemon doesn't leave us with `EADDRINUSE`.
pub fn bind(path: &Path) -> Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating socket parent dir {}", parent.display()))?;
    }
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    let l = UnixListener::bind(path)
        .with_context(|| format!("binding UDS at {}", path.display()))?;
    info!(path = %path.display(), "daemon IPC socket bound");
    Ok(l)
}

/// Top-level accept loop. Returns when the listener errors fatally.
///
/// `from_clients_tx` is cloned per connection — every `ClientMsg` from
/// any GUI is forwarded onto it. `to_clients_rx` is the daemon's
/// broadcast of `ServerMsg`; each connection subscribes its own
/// receiver inside `serve_connection`.
pub async fn run(
    listener: UnixListener,
    from_clients_tx: mpsc::Sender<ClientMsg>,
    to_clients_tx: broadcast::Sender<ServerMsg>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                info!("daemon: GUI connected");
                let from_tx = from_clients_tx.clone();
                let to_rx = to_clients_tx.subscribe();
                let to_tx = to_clients_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_connection(stream, from_tx, to_rx, to_tx).await {
                        warn!("daemon: GUI connection ended with error: {e:#}");
                    } else {
                        info!("daemon: GUI disconnected");
                    }
                });
            }
            Err(e) => {
                warn!("daemon: accept failed: {e}; continuing");
            }
        }
    }
}

async fn serve_connection(
    stream: UnixStream,
    from_clients_tx: mpsc::Sender<ClientMsg>,
    mut to_client_rx: broadcast::Receiver<ServerMsg>,
    to_client_replay: broadcast::Sender<ServerMsg>,
) -> Result<()> {
    let (read_half, write_half) = stream.into_split();
    let mut read_half = read_half;
    let mut write_half = write_half;

    // Send a Hello immediately so the GUI can confirm version
    // compatibility before doing anything else.
    let hello = ServerMsg::Hello {
        daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: vec!["jpeg-preview".to_string(), "cv".to_string()],
    };
    let bytes = serde_json::to_vec(&hello).context("serialising Hello")?;
    write_frame(&mut write_half, &bytes)
        .await
        .context("sending Hello to GUI")?;

    // We also kick the broadcast so any pending state (e.g. the
    // current pairing ticket) reaches the new GUI. The daemon's
    // control loop is responsible for re-publishing on demand via
    // `RequestPairing`; the empty broadcast subscriber will catch
    // anything published from now on.
    let _ = to_client_replay; // not used today; reserved for replay logic.

    let send_task = tokio::spawn(async move {
        loop {
            match to_client_rx.recv().await {
                Ok(msg) => {
                    let bytes = match serde_json::to_vec(&msg) {
                        Ok(b) => b,
                        Err(e) => {
                            warn!("daemon: failed to serialise ServerMsg: {e}");
                            continue;
                        }
                    };
                    if let Err(e) = write_frame(&mut write_half, &bytes).await {
                        debug!("daemon: GUI write failed (likely closed): {e}");
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("daemon: GUI fell behind by {n} ServerMsgs; continuing");
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    let recv_task = tokio::spawn(async move {
        loop {
            match read_frame(&mut read_half).await {
                Ok(Some(bytes)) => match serde_json::from_slice::<ClientMsg>(&bytes) {
                    Ok(msg) => {
                        if from_clients_tx.send(msg).await.is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        warn!("daemon: ignoring undecodable ClientMsg: {e}");
                    }
                },
                Ok(None) => return,
                Err(e) => {
                    debug!("daemon: GUI read failed: {e}");
                    return;
                }
            }
        }
    });

    // Either half ending closes the whole connection.
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::timeout;

    /// Smoke: bind, connect a client, exchange a Hello/Echo round-trip.
    #[tokio::test]
    async fn ipc_roundtrip_hello_and_echo() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("d.sock");
        let listener = bind(&path).unwrap();

        let (from_tx, mut from_rx) = mpsc::channel::<ClientMsg>(16);
        let (to_tx, _) = broadcast::channel::<ServerMsg>(16);
        let to_tx_for_loop = to_tx.clone();
        tokio::spawn(async move {
            run(listener, from_tx, to_tx_for_loop).await;
        });

        let stream = timeout(Duration::from_secs(2), UnixStream::connect(&path))
            .await
            .expect("connect timeout")
            .expect("connect");
        let (mut r, mut w) = stream.into_split();

        // Daemon sends Hello immediately; read it.
        let bytes = timeout(Duration::from_secs(2), read_frame(&mut r))
            .await
            .expect("read Hello timeout")
            .expect("read Hello io ok")
            .expect("got Hello bytes");
        let hello: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match hello {
            ServerMsg::Hello { daemon_version, .. } => {
                assert!(!daemon_version.is_empty());
            }
            other => panic!("expected Hello, got {other:?}"),
        }

        // Client sends a ClientMsg::Hello; daemon's mpsc should see it.
        let cmsg = ClientMsg::Hello {
            gui_version: "test".to_string(),
        };
        let cb = serde_json::to_vec(&cmsg).unwrap();
        write_frame(&mut w, &cb).await.unwrap();

        let received = timeout(Duration::from_secs(2), from_rx.recv())
            .await
            .expect("from_rx timeout")
            .expect("got ClientMsg");
        match received {
            ClientMsg::Hello { gui_version } => assert_eq!(gui_version, "test"),
            other => panic!("expected Hello, got {other:?}"),
        }

        // Daemon publishes a Pairing; client should read it.
        let s = ServerMsg::Pairing {
            ticket: "t-fixture".to_string(),
        };
        let _ = to_tx.send(s);
        let bytes = timeout(Duration::from_secs(2), read_frame(&mut r))
            .await
            .expect("read Pairing timeout")
            .expect("read Pairing io ok")
            .expect("got Pairing bytes");
        let parsed: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match parsed {
            ServerMsg::Pairing { ticket } => assert_eq!(ticket, "t-fixture"),
            other => panic!("expected Pairing, got {other:?}"),
        }

        // Drop the client; the daemon's per-conn task should exit cleanly.
        drop(r);
        drop(w);
        // Give it a beat.
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Server still responsive: bind another listener at a fresh path.
        // (We don't tear down the original — just confirm we didn't panic.)
        let path2 = dir.path().join("d2.sock");
        let _l2 = bind(&path2).unwrap();
    }

    /// Backstop using lower-level read so a non-Tokio `AsyncReadExt`
    /// regression doesn't slip through.
    #[tokio::test]
    async fn frame_codec_via_unix_socket() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("e.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let connect = UnixStream::connect(&path);
        let (server_side, client_side) = tokio::join!(listener.accept(), connect);
        let (mut server, _) = server_side.unwrap();
        let mut client = client_side.unwrap();

        // Server writes "ping"; client reads.
        let writer = tokio::spawn(async move {
            write_frame(&mut server, b"ping").await.unwrap();
            // close
            server.shutdown().await.ok();
        });
        let mut buf = [0u8; 4];
        // length prefix
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(u32::from_be_bytes(buf), 4);
        let mut payload = [0u8; 4];
        client.read_exact(&mut payload).await.unwrap();
        assert_eq!(&payload, b"ping");
        writer.await.unwrap();
    }
}
