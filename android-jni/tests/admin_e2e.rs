//! End-to-end test for `admin_client` — spins up a tiny iroh
//! `ProtocolHandler` that speaks the admin wire format, then dials it
//! with `connect_session` and round-trips one ListAllowed +
//! one Status. Verifies framing, ALPN, and JSON encoding without
//! depending on the daemon binary's internal modules.

use std::sync::Arc;

use herd_scout_ipc::{
    ADMIN_ALPN, AdminClientMsg, AdminServerMsg, AllowedEntry, StatusReply,
};
use herd_scout_jni::admin_client;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, endpoint::presets};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
struct EchoAdmin;

impl std::fmt::Debug for EchoAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EchoAdmin")
    }
}

const MAX: u32 = 8 * 1024 * 1024;

async fn read_frame<R: tokio::io::AsyncRead + Unpin>(
    r: &mut R,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf);
    if len > MAX {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too big",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(
    w: &mut W,
    payload: &[u8],
) -> std::io::Result<()> {
    let len = (payload.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(payload).await?;
    Ok(())
}

impl ProtocolHandler for EchoAdmin {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        loop {
            let (mut send, mut recv) = match connection.accept_bi().await {
                Ok(s) => s,
                Err(_) => return Ok(()),
            };
            let req_bytes = match read_frame(&mut recv).await {
                Ok(Some(b)) => b,
                _ => continue,
            };
            let req: AdminClientMsg = match serde_json::from_slice(&req_bytes) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let reply = match req {
                AdminClientMsg::ListAllowed => AdminServerMsg::Allowed {
                    entries: vec![AllowedEntry {
                        node_id:
                            "0000000000000000000000000000000000000000000000000000000000000000"
                                .into(),
                        label: "stub".into(),
                    }],
                },
                AdminClientMsg::Status => AdminServerMsg::Status(StatusReply {
                    daemon_version: "test".into(),
                    own_node_id: "stub".into(),
                    active_ssh_sessions: 0,
                    admins_count: 1,
                    allowed_count: 1,
                    last_reload_unix_ms: 12345,
                    last_reload_source: "boot".into(),
                    identity_schema_version: 1,
                }),
                AdminClientMsg::TailAudit { .. } => AdminServerMsg::AuditTail {
                    records: Vec::new(),
                    eof: true,
                },
                _ => AdminServerMsg::Ok,
            };
            let bytes = serde_json::to_vec(&reply).unwrap();
            let _ = write_frame(&mut send, &bytes).await;
            let _ = send.finish();
        }
    }
}

async fn spawn_echo_server() -> (Router, iroh::EndpointId) {
    let ep = Endpoint::builder(presets::N0).bind().await.unwrap();
    let id = ep.id();
    let router = Router::builder(ep)
        .accept(ADMIN_ALPN, EchoAdmin)
        .spawn();
    (router, id)
}

#[tokio::test]
async fn end_to_end_list_status_tail() {
    let (_router, daemon_id) = spawn_echo_server().await;
    let tmp = TempDir::new().unwrap();
    let session = admin_client::connect_session(tmp.path(), &daemon_id.to_string())
        .await
        .expect("dial echo server");

    let entries = session.list_allowed().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].label, "stub");

    let status = session.status().await.unwrap();
    assert_eq!(status.daemon_version, "test");
    assert_eq!(status.identity_schema_version, 1);

    let (records, eof) = session.tail_audit(10, None).await.unwrap();
    assert!(eof);
    assert!(records.is_empty());

    // Disconnect tears the session down.
    let was_open = admin_client::disconnect_session().await;
    assert!(was_open);

    // Idempotent.
    let was_open = admin_client::disconnect_session().await;
    assert!(!was_open);
}

#[tokio::test]
async fn reconnect_to_same_daemon_reuses_session() {
    let (_router, daemon_id) = spawn_echo_server().await;
    let tmp = TempDir::new().unwrap();
    let s1 = admin_client::connect_session(tmp.path(), &daemon_id.to_string())
        .await
        .unwrap();
    let s2 = admin_client::connect_session(tmp.path(), &daemon_id.to_string())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&s1, &s2), "same daemon → same session");
    admin_client::disconnect_session().await;
}
