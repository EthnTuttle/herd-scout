//! iroh ALPN handler for `UPLOAD_ALPN` plus the GUI's local-handoff
//! path (Wave 13 / Phase 2).
//!
//! Two entry points share the same staging logic:
//!
//! * [`UploadHandler::accept`] — implements `iroh::protocol::ProtocolHandler`
//!   for `herd-scout/upload/1`. Authenticates the remote against
//!   `[control_plane.admins]`, reads `UploadClientMsg::Push`, accepts
//!   the byte stream over the same QUIC bi-stream, hashes + verifies,
//!   and queues the clip.
//!
//! * [`handle_local_handoff`] — invoked from `main.rs` when the GUI
//!   sends a `ClientMsg::UploadHandoff { path, blake3_hex, size_bytes }`
//!   over its Unix socket. Reads the bytes from local disk and runs
//!   them through the same staging pipeline.
//!
//! The wire format for the QUIC path is documented in
//! [`super::protocol`]'s module doc; in short the post-`Accepted`
//! byte stream is raw clip bytes for exactly `size_bytes`. iroh-blobs
//! migration is a follow-up — the metadata wrapper (`Push`) already
//! names the BLAKE3 so the transport-only swap is clean.

use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use herd_scout_ipc::{
    ServerMsg, UploadClientMsg, UploadEntry, UploadServerMsg, UploadState,
};
use iroh::EndpointId;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use super::queue::Queue;
use super::store::{
    clip_dir, write_meta_json, ClipMeta, StagerOutcome, UploadStager, MAX_UPLOAD_BYTES,
};
use crate::audit::Audit;
use crate::control::ControlConfig;
use crate::ipc::frame;

/// Build the `AccessLimit` predicate for the upload ALPN.
///
/// Wave 14 refactor: the admins-set membership check moved out of
/// `UploadHandler::accept` into iroh's router-layer
/// `AccessLimit::new`. The pre-refactor handler did **not** include
/// a self-dial check (the daemon doesn't dial its own UPLOAD_ALPN);
/// we preserve that behavior — the predicate is membership-only.
/// Rejection audit lines (`upload_rejected` with
/// `reason: "not_in_admins"`) are now fire-and-forget via
/// `tokio::spawn` — accepted regression: a runtime-shutdown race may
/// drop the audit-log future. The `MAX_UPLOAD_BYTES` and
/// `hash_mismatch` audit lines stay inside the handler (they're per-
/// RPC outcomes, not gate decisions).
pub fn admins_predicate(
    cfg: Arc<ArcSwap<ControlConfig>>,
    audit: Audit,
) -> impl Fn(EndpointId) -> bool + Send + Sync + 'static {
    move |remote: EndpointId| {
        let snapshot = cfg.load();
        if !snapshot.admins.contains(&remote) {
            warn!(
                remote = %remote.fmt_short(),
                "upload: dropping unauthorized dial",
            );
            let audit = audit.clone();
            tokio::spawn(async move {
                audit
                    .log(
                        "upload_rejected",
                        Some(remote.to_string()),
                        None,
                        json!({ "reason": "not_in_admins" }),
                    )
                    .await;
            });
            return false;
        }
        true
    }
}

/// QUIC stream chunk size used while streaming bytes into the stager.
/// 64 KiB matches typical kernel socket buffers and is small enough to
/// keep the BLAKE3 hasher's working set in L1 cache.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Handler for the upload ALPN. Cheap to clone — all heavy state is
/// behind `Arc`s.
#[derive(Debug, Clone)]
pub struct UploadHandler {
    audit: Audit,
    queue: Queue,
    uploads_dir: PathBuf,
    server_tx: broadcast::Sender<ServerMsg>,
}

impl UploadHandler {
    pub fn new(
        audit: Audit,
        queue: Queue,
        uploads_dir: PathBuf,
        server_tx: broadcast::Sender<ServerMsg>,
    ) -> Self {
        Self {
            audit,
            queue,
            uploads_dir,
            server_tx,
        }
    }

    fn fan_out(&self, entry: &UploadEntry) {
        let _ = self.server_tx.send(ServerMsg::UploadStatus {
            blake3_hex: entry.blake3_hex.clone(),
            filename: entry.filename.clone(),
            state: entry.state.clone(),
            progress_pct: 0,
            eta_ms: None,
            summary: None,
        });
    }
}

/// Common entry point used after the bytes are on disk under the
/// expected `<uploads_dir>/<blake3>/clip.<ext>` path. Persists the
/// `meta.json`, appends the queue entry, and fans out `UploadStatus`
/// to the GUI socket.
async fn finalize_accepted(
    handler: &UploadHandler,
    blake3_hex: String,
    filename: String,
    size_bytes: u64,
    source_node_id: Option<String>,
) -> UploadEntry {
    let now_ms = crate::audit::now_unix_ms();
    let dir = clip_dir(&handler.uploads_dir, &blake3_hex);
    let meta = ClipMeta {
        blake3_hex: blake3_hex.clone(),
        filename: filename.clone(),
        size_bytes,
        upload_ts_ms: now_ms,
        source_node_id: source_node_id.clone(),
    };
    if let Err(e) = write_meta_json(&dir, &meta).await {
        warn!(blake3 = %blake3_hex, "upload: write_meta_json failed: {e:#}");
    }
    let entry = UploadEntry {
        blake3_hex: blake3_hex.clone(),
        filename: filename.clone(),
        size_bytes,
        state: UploadState::Queued,
        queued_ts_ms: now_ms,
        started_ts_ms: None,
        finished_ts_ms: None,
    };
    let stored = handler.queue.enqueue(entry).await;
    handler.fan_out(&stored);
    handler
        .audit
        .log(
            "upload_accepted",
            source_node_id.clone(),
            None,
            json!({
                "blake3_hex": blake3_hex,
                "filename": filename,
                "size_bytes": size_bytes,
            }),
        )
        .await;
    info!(
        blake3 = %blake3_hex,
        filename,
        size_bytes,
        "upload: accepted clip and queued"
    );
    stored
}

/// GUI-side handoff: the GUI has staged a local file path and wants
/// the daemon to take ownership. We read the bytes off disk and run
/// them through [`UploadStager`] just like a remote upload — this
/// preserves the integrity guarantee even for co-located clients.
///
/// Used from `main.rs`'s `ClientMsg::UploadHandoff` arm.
pub async fn handle_local_handoff(
    handler: &UploadHandler,
    path: &str,
    blake3_hex: &str,
    size_bytes: u64,
) {
    if size_bytes > MAX_UPLOAD_BYTES {
        let _ = handler.server_tx.send(ServerMsg::UploadStatus {
            blake3_hex: blake3_hex.to_string(),
            filename: filename_from_path(path),
            state: UploadState::Failed {
                reason: format!(
                    "size {size_bytes} exceeds cap {MAX_UPLOAD_BYTES}"
                ),
            },
            progress_pct: 0,
            eta_ms: None,
            summary: None,
        });
        handler
            .audit
            .log(
                "upload_rejected",
                None,
                None,
                json!({
                    "reason": "too_big",
                    "size_bytes": size_bytes,
                    "max_bytes": MAX_UPLOAD_BYTES,
                    "source": "gui_handoff",
                }),
            )
            .await;
        return;
    }

    let filename = filename_from_path(path);
    let mut stager =
        match UploadStager::create(&handler.uploads_dir, blake3_hex, size_bytes).await {
            Ok(s) => s,
            Err(e) => {
                warn!("upload: stager create failed: {e:#}");
                let _ = handler.server_tx.send(ServerMsg::UploadStatus {
                    blake3_hex: blake3_hex.to_string(),
                    filename,
                    state: UploadState::Failed {
                        reason: format!("stager_create_failed: {e}"),
                    },
                    progress_pct: 0,
                    eta_ms: None,
                    summary: None,
                });
                return;
            }
        };

    // Stream the file from disk in chunks — large clips would
    // otherwise allocate ~size_bytes for one read_to_end.
    let mut f = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) => {
            warn!(path, "upload: open handoff source failed: {e:#}");
            let _ = handler.server_tx.send(ServerMsg::UploadStatus {
                blake3_hex: blake3_hex.to_string(),
                filename,
                state: UploadState::Failed {
                    reason: format!("open_handoff_source: {e}"),
                },
                progress_pct: 0,
                eta_ms: None,
                summary: None,
            });
            return;
        }
    };
    let mut buf = vec![0u8; READ_CHUNK_BYTES];
    loop {
        let n = match f.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                warn!(path, "upload: read handoff source failed: {e:#}");
                let _ = handler.server_tx.send(ServerMsg::UploadStatus {
                    blake3_hex: blake3_hex.to_string(),
                    filename,
                    state: UploadState::Failed {
                        reason: format!("read_handoff_source: {e}"),
                    },
                    progress_pct: 0,
                    eta_ms: None,
                    summary: None,
                });
                return;
            }
        };
        if let Err(e) = stager.update(&buf[..n]).await {
            warn!("upload: stager update failed: {e:#}");
            let _ = handler.server_tx.send(ServerMsg::UploadStatus {
                blake3_hex: blake3_hex.to_string(),
                filename,
                state: UploadState::Failed {
                    reason: format!("stager_update: {e}"),
                },
                progress_pct: 0,
                eta_ms: None,
                summary: None,
            });
            return;
        }
    }

    match stager.finalize(&filename).await {
        Ok(StagerOutcome::Ok { .. }) => {
            finalize_accepted(handler, blake3_hex.to_string(), filename, size_bytes, None)
                .await;
        }
        Ok(StagerOutcome::HashMismatch { reported, computed }) => {
            warn!(
                reported,
                computed,
                "upload: gui handoff hash mismatch; rejecting"
            );
            let _ = handler.server_tx.send(ServerMsg::UploadStatus {
                blake3_hex: blake3_hex.to_string(),
                filename,
                state: UploadState::Failed {
                    reason: format!("hash_mismatch: computed={computed}"),
                },
                progress_pct: 0,
                eta_ms: None,
                summary: None,
            });
            handler
                .audit
                .log(
                    "upload_rejected",
                    None,
                    None,
                    json!({
                        "reason": "hash_mismatch",
                        "reported": reported,
                        "computed": computed,
                        "source": "gui_handoff",
                    }),
                )
                .await;
        }
        Err(e) => {
            warn!("upload: finalize gui handoff failed: {e:#}");
            let _ = handler.server_tx.send(ServerMsg::UploadStatus {
                blake3_hex: blake3_hex.to_string(),
                filename,
                state: UploadState::Failed {
                    reason: format!("finalize: {e:#}"),
                },
                progress_pct: 0,
                eta_ms: None,
                summary: None,
            });
        }
    }
}

/// Surface the queue's `cancel` outcome to the GUI as an
/// `UploadStatus` message. Idempotent — calling on an unknown hash
/// emits a `Failed { reason: "not_found" }` payload so the GUI can
/// remove the row.
pub async fn handle_local_cancel(handler: &UploadHandler, blake3_hex: &str) {
    use super::queue::CancelOutcome;
    let outcome = handler.queue.cancel(blake3_hex).await;
    let entry = handler.queue.get(blake3_hex).await;
    match outcome {
        CancelOutcome::Cancelled => {
            handler
                .audit
                .log(
                    "upload_cancelled",
                    None,
                    None,
                    json!({ "blake3_hex": blake3_hex }),
                )
                .await;
            if let Some(e) = entry {
                let _ = handler.server_tx.send(ServerMsg::UploadStatus {
                    blake3_hex: e.blake3_hex,
                    filename: e.filename,
                    state: UploadState::Failed {
                        reason: "cancelled".to_string(),
                    },
                    progress_pct: 0,
                    eta_ms: None,
                    summary: None,
                });
            }
        }
        CancelOutcome::NotCancellable => {
            debug!(blake3 = %blake3_hex, "upload: cancel ignored — entry past Queued");
        }
        CancelOutcome::NotFound => {
            debug!(blake3 = %blake3_hex, "upload: cancel ignored — unknown hash");
        }
    }
}

fn filename_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

// ---------------------------------------------------------------------
// iroh ProtocolHandler
// ---------------------------------------------------------------------

impl ProtocolHandler for UploadHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = connection.remote_id();

        // Admins-membership gate runs in `admins_predicate` via
        // `AccessLimit` (registered in `main.rs`). When this method is
        // entered, the connection has already been authorized.

        let (mut send, mut recv) = match connection.accept_bi().await {
            Ok(streams) => streams,
            Err(_) => return Ok(()),
        };

        // Read one Push (or other client message). One bi-stream per
        // upload.
        let req_bytes = match frame::read_frame(&mut recv).await {
            Ok(Some(b)) => b,
            Ok(None) => return Ok(()),
            Err(e) => {
                warn!(remote = %remote.fmt_short(), "upload: framing error: {e:#}");
                return Ok(());
            }
        };
        let req: UploadClientMsg = match serde_json::from_slice(&req_bytes) {
            Ok(r) => r,
            Err(e) => {
                let reply = UploadServerMsg::Error {
                    code: "bad_request".to_string(),
                    message: format!("parse error: {e}"),
                };
                let _ = write_reply(&mut send, &reply).await;
                let _ = send.finish();
                return Ok(());
            }
        };

        match req {
            UploadClientMsg::Push {
                filename,
                size_bytes,
                blake3_hex,
            } => {
                let result = self
                    .handle_push(
                        remote.to_string(),
                        filename,
                        size_bytes,
                        blake3_hex,
                        &mut send,
                        &mut recv,
                    )
                    .await;
                if let Err(e) = result {
                    warn!(remote = %remote.fmt_short(), "upload: push failed: {e:#}");
                }
            }
            UploadClientMsg::ListQueue => {
                let entries = self.queue.snapshot().await;
                let reply = UploadServerMsg::QueueSnapshot { entries };
                let _ = write_reply(&mut send, &reply).await;
            }
            UploadClientMsg::CancelQueued { blake3_hex } => {
                use super::queue::CancelOutcome;
                let outcome = self.queue.cancel(&blake3_hex).await;
                let reply = match outcome {
                    CancelOutcome::Cancelled => UploadServerMsg::Ok,
                    CancelOutcome::NotCancellable => UploadServerMsg::Error {
                        code: "not_cancellable".to_string(),
                        message: "entry is past Queued".to_string(),
                    },
                    CancelOutcome::NotFound => UploadServerMsg::Error {
                        code: "not_found".to_string(),
                        message: "no entry with that blake3_hex".to_string(),
                    },
                };
                if matches!(outcome, CancelOutcome::Cancelled) {
                    self.audit
                        .log(
                            "upload_cancelled",
                            Some(remote.to_string()),
                            None,
                            json!({ "blake3_hex": blake3_hex }),
                        )
                        .await;
                }
                let _ = write_reply(&mut send, &reply).await;
            }
            UploadClientMsg::FetchReport { blake3_hex } => {
                let dir = clip_dir(&self.uploads_dir, &blake3_hex);
                let report_path = dir.join("report.json");
                match tokio::fs::read(&report_path).await {
                    Ok(json_bytes) => {
                        let reply = UploadServerMsg::Report {
                            blake3_hex,
                            json_bytes,
                        };
                        let _ = write_reply(&mut send, &reply).await;
                    }
                    Err(e) => {
                        let reply = UploadServerMsg::Error {
                            code: "not_found".to_string(),
                            message: format!("read report.json: {e}"),
                        };
                        let _ = write_reply(&mut send, &reply).await;
                    }
                }
            }
        }
        let _ = send.finish();
        Ok(())
    }
}

impl UploadHandler {
    /// Handle the `Push` half of an upload: validate cap, accept bytes
    /// inline on the same QUIC stream, hash + verify, then queue.
    async fn handle_push(
        &self,
        actor: String,
        filename: String,
        size_bytes: u64,
        blake3_hex: String,
        send: &mut iroh::endpoint::SendStream,
        recv: &mut iroh::endpoint::RecvStream,
    ) -> anyhow::Result<()> {
        if size_bytes > MAX_UPLOAD_BYTES {
            let reply = UploadServerMsg::RejectedTooBig {
                actual_bytes: size_bytes,
                max_bytes: MAX_UPLOAD_BYTES,
            };
            write_reply(send, &reply).await?;
            self.audit
                .log(
                    "upload_rejected",
                    Some(actor),
                    None,
                    json!({
                        "reason": "too_big",
                        "size_bytes": size_bytes,
                        "max_bytes": MAX_UPLOAD_BYTES,
                    }),
                )
                .await;
            return Ok(());
        }

        // Reply Accepted *before* reading bytes so the client can begin
        // streaming. The client commits the bytes; we hash and either
        // reply Ok / RejectedHashMismatch on the same stream.
        let accepted = UploadServerMsg::Accepted {
            blake3_hex: blake3_hex.clone(),
        };
        write_reply(send, &accepted).await?;

        let mut stager = UploadStager::create(&self.uploads_dir, &blake3_hex, size_bytes)
            .await
            .map_err(|e| anyhow::anyhow!("stager_create: {e}"))?;

        let mut remaining = size_bytes;
        let mut buf = vec![0u8; READ_CHUNK_BYTES];
        while remaining > 0 {
            let want = std::cmp::min(remaining as usize, buf.len());
            // iroh's RecvStream impls `AsyncRead`; `read` returns
            // `Ok(0)` on clean EOF.
            let n = match AsyncReadExt::read(recv, &mut buf[..want]).await {
                Ok(0) => {
                    let reply = UploadServerMsg::Error {
                        code: "short_read".to_string(),
                        message: format!(
                            "stream ended {remaining} bytes early (expected {size_bytes})"
                        ),
                    };
                    let _ = write_reply(send, &reply).await;
                    return Ok(());
                }
                Ok(n) => n,
                Err(e) => {
                    let reply = UploadServerMsg::Error {
                        code: "read_failed".to_string(),
                        message: format!("read error: {e}"),
                    };
                    let _ = write_reply(send, &reply).await;
                    return Ok(());
                }
            };
            stager
                .update(&buf[..n])
                .await
                .map_err(|e| anyhow::anyhow!("stager_update: {e}"))?;
            // The `want = min(remaining, buf.len())` guard above bounds
            // `n <= remaining`, so this subtraction can never underflow
            // in practice. Using `checked_sub` keeps that invariant
            // explicit and surfaces a sidecar/transport regression
            // loudly instead of silently capping at zero.
            remaining = remaining.checked_sub(n as u64).ok_or_else(|| {
                anyhow::anyhow!(
                    "upload body read produced {n} bytes when only {remaining} were requested"
                )
            })?;
        }

        match stager.finalize(&filename).await? {
            StagerOutcome::Ok { .. } => {
                finalize_accepted(self, blake3_hex.clone(), filename, size_bytes, Some(actor))
                    .await;
                let _ = write_reply(send, &UploadServerMsg::Ok).await;
            }
            StagerOutcome::HashMismatch { reported, computed } => {
                let reply = UploadServerMsg::RejectedHashMismatch {
                    reported: reported.clone(),
                    computed: computed.clone(),
                };
                let _ = write_reply(send, &reply).await;
                self.audit
                    .log(
                        "upload_rejected",
                        Some(actor),
                        None,
                        json!({
                            "reason": "hash_mismatch",
                            "reported": reported,
                            "computed": computed,
                        }),
                    )
                    .await;
            }
        }
        Ok(())
    }
}

async fn write_reply(
    send: &mut iroh::endpoint::SendStream,
    reply: &UploadServerMsg,
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(reply).map_err(|e| anyhow::anyhow!("serialize: {e}"))?;
    frame::write_frame(send, &bytes)
        .await
        .map_err(|e| anyhow::anyhow!("write_frame: {e}"))?;
    Ok(())
}

