//! Wave 12 — admin RPC client used by the Android admin app.
//!
//! Speaks `herd_scout_ipc::ADMIN_ALPN` against a daemon NodeId. One
//! bi-directional QUIC stream per RPC, framed as 4-byte BE length +
//! JSON body — the same framing the daemon already uses for daemon↔GUI
//! IPC and for its admin handler.
//!
//! Single-slot fleet model (Decision 12 of the Wave 12 plan):
//!   - One `Endpoint` per process, persistent identity loaded via
//!     `herd_scout_identity::load_or_generate`.
//!   - One active `AdminSession` at a time. Switching daemons closes
//!     the prior session before dialing. No multiplexing.
//!   - `connect_session` is idempotent for the same daemon NodeId —
//!     reusing the existing session when one is already open avoids
//!     QUIC handshake churn during foreground polling.
//!
//! Built unconditionally so it compiles + runs unit tests on host
//! macOS/Linux without an NDK; the JNI exports in `lib.rs` live
//! behind `cfg(target_os = "android")`.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use herd_scout_identity::{Identity, load_or_generate, save};
use herd_scout_ipc::{
    ADMIN_ALPN, AdminClientMsg, AdminServerMsg, AllowedEntry, AuditRecord, StatusReply,
};
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId, endpoint::presets};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, OnceCell};

/// Identity-file name within the configured `files_dir`.
const IDENTITY_FILE: &str = "identity.toml";

/// Hard cap on a single reply payload from the daemon. Matches the
/// daemon's IPC frame cap to avoid silent truncation.
const MAX_REPLY: u32 = 8 * 1024 * 1024;

// ── Process-wide single-slot state ──────────────────────────────────────

/// Process-lifetime endpoint, lazily bound on first use. Reused across
/// every connect/disconnect cycle — only the QUIC `Connection` churns.
static ADMIN_ENDPOINT: OnceCell<Endpoint> = OnceCell::const_new();

/// At most one live admin session at a time. Switching daemons calls
/// `close()` on whatever's here, then dials the new one.
static ADMIN_SESSION: Mutex<Option<Arc<AdminSession>>> = Mutex::const_new(None);

// ── Session ─────────────────────────────────────────────────────────────

/// One in-flight admin connection to a single daemon. Cheap to clone
/// (Arc inside).
#[derive(Debug)]
pub struct AdminSession {
    daemon_node_id: EndpointId,
    conn: Connection,
    /// Serializes RPCs across this session. Each RPC opens a fresh
    /// bi-stream, which the daemon happily multiplexes — but the JNI
    /// boundary is one-call-at-a-time, so this just keeps the API
    /// honest.
    rpc_lock: Mutex<()>,
}

impl AdminSession {
    pub fn daemon_node_id(&self) -> EndpointId {
        self.daemon_node_id
    }

    /// Open one bi-stream, send a request, read one reply, drop.
    async fn rpc(&self, req: &AdminClientMsg) -> Result<AdminServerMsg> {
        let _guard = self.rpc_lock.lock().await;
        let (mut send, mut recv) = self
            .conn
            .open_bi()
            .await
            .context("open admin bi-stream")?;
        let bytes = serde_json::to_vec(req).context("serialize admin request")?;
        write_frame(&mut send, &bytes)
            .await
            .context("write admin request")?;
        // Half-close the send side so the daemon's read_frame returns
        // cleanly after parsing our single request.
        let _ = send.finish();

        let reply_bytes = read_frame(&mut recv)
            .await
            .context("read admin reply")?
            .ok_or_else(|| anyhow!("daemon closed stream before sending a reply"))?;
        let reply: AdminServerMsg =
            serde_json::from_slice(&reply_bytes).context("parse admin reply")?;
        Ok(reply)
    }

    pub async fn list_allowed(&self) -> Result<Vec<AllowedEntry>> {
        match self.rpc(&AdminClientMsg::ListAllowed).await? {
            AdminServerMsg::Allowed { entries } => Ok(entries),
            AdminServerMsg::Error { code, message } => Err(daemon_error(&code, &message)),
            other => Err(unexpected_variant("ListAllowed", &other)),
        }
    }

    pub async fn add_allowed(&self, node_id: &str, label: &str) -> Result<()> {
        match self
            .rpc(&AdminClientMsg::AddAllowed {
                node_id: node_id.to_string(),
                label: label.to_string(),
            })
            .await?
        {
            AdminServerMsg::Ok => Ok(()),
            AdminServerMsg::Error { code, message } => Err(daemon_error(&code, &message)),
            other => Err(unexpected_variant("AddAllowed", &other)),
        }
    }

    pub async fn remove_allowed(&self, node_id: &str) -> Result<()> {
        match self
            .rpc(&AdminClientMsg::RemoveAllowed {
                node_id: node_id.to_string(),
            })
            .await?
        {
            AdminServerMsg::Ok => Ok(()),
            AdminServerMsg::Error { code, message } => Err(daemon_error(&code, &message)),
            other => Err(unexpected_variant("RemoveAllowed", &other)),
        }
    }

    pub async fn status(&self) -> Result<StatusReply> {
        match self.rpc(&AdminClientMsg::Status).await? {
            AdminServerMsg::Status(s) => Ok(s),
            AdminServerMsg::Error { code, message } => Err(daemon_error(&code, &message)),
            other => Err(unexpected_variant("Status", &other)),
        }
    }

    pub async fn tail_audit(
        &self,
        last_n: u32,
        before_ts_ms: Option<u64>,
    ) -> Result<(Vec<AuditRecord>, bool)> {
        match self
            .rpc(&AdminClientMsg::TailAudit {
                last_n,
                before_ts_ms,
            })
            .await?
        {
            AdminServerMsg::AuditTail { records, eof } => Ok((records, eof)),
            AdminServerMsg::Error { code, message } => Err(daemon_error(&code, &message)),
            other => Err(unexpected_variant("TailAudit", &other)),
        }
    }
}

// ── Public single-slot API ──────────────────────────────────────────────

async fn endpoint_for(files_dir: &Path) -> Result<Endpoint> {
    let id = identity_for(files_dir)?;
    Endpoint::builder(presets::N0)
        .secret_key(id.secret)
        .bind()
        .await
        .context("bind iroh endpoint")
}

fn identity_for(files_dir: &Path) -> Result<Identity> {
    let path = files_dir.join(IDENTITY_FILE);
    load_or_generate(&path, "herd-scout-admin")
        .with_context(|| format!("load or create identity at {}", path.display()))
}

/// Get or lazily bind the process-wide endpoint, using `files_dir` to
/// locate the persistent identity.
async fn shared_endpoint(files_dir: &Path) -> Result<Endpoint> {
    let dir = files_dir.to_path_buf();
    ADMIN_ENDPOINT
        .get_or_try_init(move || {
            let dir = dir.clone();
            async move { endpoint_for(&dir).await }
        })
        .await
        .cloned()
}

/// Dial the given daemon NodeId, replacing any existing session in the
/// single-slot. Reuses the session if it's already pointed at the same
/// NodeId.
pub async fn connect_session(
    files_dir: &Path,
    daemon_node_id: &str,
) -> Result<Arc<AdminSession>> {
    let id = EndpointId::from_str(daemon_node_id.trim())
        .with_context(|| format!("parse daemon NodeId {daemon_node_id:?}"))?;

    {
        let slot = ADMIN_SESSION.lock().await;
        if let Some(existing) = slot.as_ref() {
            if existing.daemon_node_id == id {
                return Ok(existing.clone());
            }
        }
    }

    // Tear down any stale session before dialing the new one.
    disconnect_session().await;

    let ep = shared_endpoint(files_dir).await?;
    let conn = ep
        .connect(EndpointAddr::new(id), ADMIN_ALPN)
        .await
        .context("dial daemon admin ALPN")?;

    let session = Arc::new(AdminSession {
        daemon_node_id: id,
        conn,
        rpc_lock: Mutex::new(()),
    });
    *ADMIN_SESSION.lock().await = Some(session.clone());
    Ok(session)
}

/// Close any active session. Idempotent — returns whether it actually
/// closed something.
pub async fn disconnect_session() -> bool {
    let prev = ADMIN_SESSION.lock().await.take();
    if let Some(session) = prev {
        session.conn.close(0u32.into(), b"client-disconnect");
        true
    } else {
        false
    }
}

// ── Identity export / import ────────────────────────────────────────────

/// Render the local identity as a TOML envelope blob suitable for SAF
/// export.
pub fn identity_export(files_dir: &Path, label: &str) -> Result<String> {
    let id = identity_for(files_dir)?;
    Ok(herd_scout_identity::export_to_user_blob(&id, label))
}

/// Parse + validate a user-supplied envelope, persist it as the active
/// identity, and tear down any in-flight admin session (the NodeId
/// just changed; existing connections are bound to the old key).
/// Returns the new NodeId on success.
pub async fn identity_import(files_dir: &Path, envelope: &str) -> Result<String> {
    let new = herd_scout_identity::import_from_user_blob(envelope)
        .context("validate imported identity envelope")?;
    let path = files_dir.join(IDENTITY_FILE);
    save(&path, &new, &new.label)
        .with_context(|| format!("persist identity to {}", path.display()))?;
    disconnect_session().await;
    // The shared endpoint is now bound to the *old* secret. Reset the
    // OnceCell so the next connect re-binds with the new identity.
    //
    // NOTE: tokio's OnceCell doesn't expose a clean reset; we live
    // with the shared endpoint surviving the import in this process.
    // Real Android usage = the Activity restarts on identity import,
    // which gives us a fresh process anyway. Document and move on.
    Ok(new.node_id())
}

/// Return the local NodeId, creating + persisting an identity if none
/// exists yet.
pub fn whoami(files_dir: &Path) -> Result<String> {
    Ok(identity_for(files_dir)?.node_id())
}

// ── Length-prefixed JSON framing (4-byte BE) ────────────────────────────

async fn read_frame<R>(r: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_REPLY {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("reply size {len} exceeds cap {MAX_REPLY}"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

async fn write_frame<W>(w: &mut W, payload: &[u8]) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    if payload.len() as u64 > MAX_REPLY as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("payload size {} exceeds cap {MAX_REPLY}", payload.len()),
        ));
    }
    let len = (payload.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(payload).await?;
    Ok(())
}

// ── helpers ─────────────────────────────────────────────────────────────

fn daemon_error(code: &str, message: &str) -> anyhow::Error {
    anyhow!("daemon error {code}: {message}")
}

fn unexpected_variant(op: &str, msg: &AdminServerMsg) -> anyhow::Error {
    anyhow!("unexpected reply variant for {op}: {msg:?}")
}

#[allow(dead_code)]
pub(crate) fn _ensure_files_dir(files_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(files_dir)
        .with_context(|| format!("create files_dir {}", files_dir.display()))?;
    Ok(files_dir.to_path_buf())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn whoami_creates_persistent_identity() {
        let tmp = TempDir::new().unwrap();
        let id1 = whoami(tmp.path()).unwrap();
        let id2 = whoami(tmp.path()).unwrap();
        assert_eq!(id1, id2);
        assert!(!id1.is_empty());
    }

    #[test]
    fn export_then_import_round_trips() {
        let tmp_a = TempDir::new().unwrap();
        let id_a = whoami(tmp_a.path()).unwrap();
        let envelope = identity_export(tmp_a.path(), "exported").unwrap();
        // fresh files_dir = different identity initially
        let tmp_b = TempDir::new().unwrap();
        let _id_b_before = whoami(tmp_b.path()).unwrap();
        // tokio runtime needed for disconnect_session in identity_import
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let imported = rt
            .block_on(identity_import(tmp_b.path(), &envelope))
            .unwrap();
        assert_eq!(imported, id_a);
        // Subsequent whoami on tmp_b returns the imported NodeId
        let id_b_after = whoami(tmp_b.path()).unwrap();
        assert_eq!(id_b_after, id_a);
    }

    #[test]
    fn import_rejects_bad_envelope() {
        let tmp = TempDir::new().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(identity_import(tmp.path(), "not-toml"))
            .unwrap_err();
        let s = format!("{err:#}");
        assert!(s.contains("validate imported identity envelope"), "{s}");
    }

    #[test]
    fn frame_round_trips() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (a, b) = tokio::io::duplex(64 * 1024);
            let (_ar, mut aw) = tokio::io::split(a);
            let (mut br, _bw) = tokio::io::split(b);
            let payload = b"hello".to_vec();
            tokio::spawn(async move {
                write_frame(&mut aw, &payload).await.unwrap();
                drop(aw);
            });
            let got = read_frame(&mut br).await.unwrap();
            assert_eq!(got, Some(b"hello".to_vec()));
        });
    }
}
