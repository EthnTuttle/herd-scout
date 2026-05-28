//! `herdctl push` and `herdctl uploads …` — desktop-video-upload Phase 4.
//!
//! Talks to a daemon over [`herd_scout_ipc::UPLOAD_ALPN`]. The wire
//! protocol on each bi-directional QUIC stream is:
//!
//! 1. Client writes a length-prefixed JSON
//!    [`UploadClientMsg`][herd_scout_ipc::UploadClientMsg] (`u32 BE
//!    length` + `JSON bytes`, mirroring [`CONTROL_ALPN`] /
//!    [`ADMIN_ALPN`] framing — see
//!    `herd-scout-daemon/src/ipc/frame.rs`).
//! 2. Daemon writes a length-prefixed JSON
//!    [`UploadServerMsg`][herd_scout_ipc::UploadServerMsg] reply.
//! 3. **For `Push` only:** if the server replied `Accepted`, the client
//!    streams the raw clip bytes on the *same* stream (no length
//!    prefix; the server already knows `size_bytes` from the JSON
//!    metadata). The daemon then writes a final length-prefixed JSON
//!    `Ok` / `RejectedHashMismatch` / `Error` reply.
//!
//! Tail mode polls `ListQueue` on a fresh stream every ~1 s; the
//! per-frame `ServerMsg::UploadStatus` push channel lives on the GUI's
//! Unix socket, not on `UPLOAD_ALPN`.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use herd_scout_ipc::{
    UPLOAD_ALPN, UploadClientMsg, UploadEntry, UploadServerMsg, UploadState,
};
use iroh::endpoint::Connection;
use iroh::{EndpointAddr, EndpointId};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::make_endpoint;

/// Hard cap on a clip body. Mirrors the daemon's server-side cap from
/// `plan-desktop-video-upload-2026-05-28.md` § Decision 7.
const MAX_CLIP_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Hard cap on a single length-prefixed JSON control message. The
/// daemon's `ipc::frame` uses 8 MiB; we match it. Clip bytes don't
/// ride this framing — they're written raw on the same stream after
/// `Accepted`.
const MAX_FRAME: u32 = 8 * 1024 * 1024;

/// Body chunk size for the raw-bytes upload phase. 256 KiB is enough
/// to amortise QUIC + tokio overhead without locking up the runtime.
const UPLOAD_CHUNK: usize = 256 * 1024;

/// Threshold above which BLAKE3 hashing offloads to a blocking thread
/// pool. Below this we just hash inline.
const HASH_OFFLOAD_THRESHOLD: u64 = 50 * 1024 * 1024;

/// Allowed clip extensions. MP4/MOV/M4V — H.264 only per Decision 7.
const ALLOWED_EXTS: &[&str] = &["mp4", "mov", "m4v"];

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// `herdctl push <node-id> <path> [--no-wait]`.
pub async fn push(node_id: &str, path: &Path, no_wait: bool) -> Result<()> {
    validate_clip(path)?;
    let size_bytes = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    let filename = path
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| anyhow!("path has no filename: {}", path.display()))?
        .to_string();

    eprintln!("hashing {} ({})", filename, fmt_bytes(size_bytes));
    let blake3_hex = hash_file(path).await?;
    eprintln!("blake3: {blake3_hex}");

    let id = EndpointId::from_str(node_id).context("parse NodeId")?;
    let ep = make_endpoint().await?;
    let result = async {
        let conn = ep
            .connect(EndpointAddr::new(id), UPLOAD_ALPN)
            .await
            .context("dial daemon (UPLOAD_ALPN)")?;

        // Upload phase: one bi-stream that carries Push → reply →
        // bytes → reply.
        push_one(&conn, path, &filename, size_bytes, &blake3_hex).await?;

        if no_wait {
            return Ok::<_, anyhow::Error>(());
        }

        // Tail mode: poll ListQueue once per second until the entry
        // hits a terminal state.
        let final_state = tail_until_terminal(&conn, &blake3_hex).await?;
        match final_state {
            UploadState::Done => {
                eprintln!("Pushed: {filename}");
                let summary = fetch_report_summary_line(&conn, &blake3_hex).await?;
                println!("{summary}");
                Ok(())
            }
            UploadState::Failed { reason } => {
                bail!("Failed: {reason}");
            }
            other => {
                // tail_until_terminal returns only Done | Failed; any
                // other value is a logic bug in our polling loop.
                bail!("unexpected non-terminal state at exit: {other:?}");
            }
        }
    }
    .await;
    ep.close().await;
    result
}

/// `herdctl uploads list <node-id>`.
pub async fn list(node_id: &str) -> Result<()> {
    let id = EndpointId::from_str(node_id).context("parse NodeId")?;
    let ep = make_endpoint().await?;
    let result = async {
        let conn = ep
            .connect(EndpointAddr::new(id), UPLOAD_ALPN)
            .await
            .context("dial daemon (UPLOAD_ALPN)")?;
        let entries = list_queue(&conn).await?;
        render_queue_table(&entries);
        Ok::<_, anyhow::Error>(())
    }
    .await;
    ep.close().await;
    result
}

/// `herdctl uploads cancel <node-id> <prefix>`.
pub async fn cancel(node_id: &str, prefix: &str) -> Result<()> {
    let id = EndpointId::from_str(node_id).context("parse NodeId")?;
    let ep = make_endpoint().await?;
    let result = async {
        let conn = ep
            .connect(EndpointAddr::new(id), UPLOAD_ALPN)
            .await
            .context("dial daemon (UPLOAD_ALPN)")?;
        let entries = list_queue(&conn).await?;
        let blake3_hex = resolve_prefix(prefix, &entries)?;

        let (mut send, mut recv) = conn.open_bi().await.context("open bi-stream")?;
        write_framed_json(
            &mut send,
            &UploadClientMsg::CancelQueued {
                blake3_hex: blake3_hex.clone(),
            },
        )
        .await?;
        let _ = send.finish();
        let reply: UploadServerMsg = read_framed_json(&mut recv).await?;
        match reply {
            UploadServerMsg::Ok => {
                println!("cancelled: {blake3_hex}");
                Ok(())
            }
            UploadServerMsg::Error { code, message } => {
                bail!("cancel failed: {code}: {message}");
            }
            other => bail!("unexpected reply to cancel: {other:?}"),
        }
    }
    .await;
    ep.close().await;
    result
}

/// `herdctl uploads report <node-id> <prefix> [--json]`.
pub async fn report(node_id: &str, prefix: &str, as_json: bool) -> Result<()> {
    let id = EndpointId::from_str(node_id).context("parse NodeId")?;
    let ep = make_endpoint().await?;
    let result = async {
        let conn = ep
            .connect(EndpointAddr::new(id), UPLOAD_ALPN)
            .await
            .context("dial daemon (UPLOAD_ALPN)")?;
        let entries = list_queue(&conn).await?;
        let blake3_hex = resolve_prefix(prefix, &entries)?;
        let bytes = fetch_report_bytes(&conn, &blake3_hex).await?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .context("parse report.json from daemon")?;
        if as_json {
            let pretty = serde_json::to_string_pretty(&value)
                .context("re-serialize report as pretty JSON")?;
            println!("{pretty}");
        } else {
            println!("{}", summary_line_from_report(&value));
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    ep.close().await;
    result
}

// ---------------------------------------------------------------------------
// Push: one bi-stream end-to-end
// ---------------------------------------------------------------------------

async fn push_one(
    conn: &Connection,
    path: &Path,
    filename: &str,
    size_bytes: u64,
    blake3_hex: &str,
) -> Result<()> {
    let (mut send, mut recv) = conn.open_bi().await.context("open bi-stream")?;

    // 1. Send the metadata Push message.
    write_framed_json(
        &mut send,
        &UploadClientMsg::Push {
            filename: filename.to_string(),
            size_bytes,
            blake3_hex: blake3_hex.to_string(),
        },
    )
    .await?;

    // 2. Read the accept/reject reply.
    let reply: UploadServerMsg = read_framed_json(&mut recv).await?;
    match reply {
        UploadServerMsg::Accepted { .. } => { /* fall through to body */ }
        UploadServerMsg::RejectedTooBig {
            actual_bytes,
            max_bytes,
        } => {
            let _ = send.finish();
            bail!(
                "rejected: clip is {} but daemon cap is {}",
                fmt_bytes(actual_bytes),
                fmt_bytes(max_bytes)
            );
        }
        UploadServerMsg::Error { code, message } => {
            let _ = send.finish();
            bail!("rejected: {code}: {message}");
        }
        other => {
            let _ = send.finish();
            bail!("unexpected reply to Push: {other:?}");
        }
    }

    // 3. Stream the raw clip body on the same stream.
    let file = File::open(path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    let mut reader = BufReader::with_capacity(UPLOAD_CHUNK, file);
    let mut buf = vec![0u8; UPLOAD_CHUNK];
    let mut sent: u64 = 0;
    let mut next_progress_at: u64 = 0;
    let total = size_bytes;
    while sent < total {
        let want = std::cmp::min(buf.len() as u64, total - sent) as usize;
        let n = reader
            .read(&mut buf[..want])
            .await
            .context("read clip body")?;
        if n == 0 {
            bail!(
                "file ended early: sent {sent}/{total} bytes; \
                 did it shrink between hashing and upload?"
            );
        }
        send.write_all(&buf[..n]).await.context("send clip body")?;
        sent += n as u64;
        if sent >= next_progress_at || sent == total {
            eprintln!(
                "uploaded {} / {}",
                fmt_bytes(sent),
                fmt_bytes(total)
            );
            // ~5% steps; never less than 1 MiB so small clips don't
            // spam.
            let step = std::cmp::max(total / 20, 1024 * 1024);
            next_progress_at = sent.saturating_add(step);
        }
    }
    // Half-close so the daemon's body-reader sees EOF on the read
    // side. We still need to read its final reply on `recv`.
    let _ = send.finish();

    // 4. Read the final reply.
    let final_reply: UploadServerMsg = read_framed_json(&mut recv).await?;
    match final_reply {
        UploadServerMsg::Ok => Ok(()),
        UploadServerMsg::RejectedHashMismatch { reported, computed } => {
            bail!(
                "hash mismatch: client reported {reported}, daemon computed {computed}"
            );
        }
        UploadServerMsg::Error { code, message } => {
            bail!("upload failed: {code}: {message}");
        }
        other => bail!("unexpected final reply: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tail: poll ListQueue until terminal
// ---------------------------------------------------------------------------

async fn tail_until_terminal(
    conn: &Connection,
    blake3_hex: &str,
) -> Result<UploadState> {
    let mut last_label: Option<String> = None;
    loop {
        let entries = list_queue(conn).await?;
        let entry = entries
            .iter()
            .find(|e| e.blake3_hex == blake3_hex);
        match entry {
            Some(e) => {
                let label = match &e.state {
                    UploadState::Queued => "queued".to_string(),
                    UploadState::Decoding => "decoding".to_string(),
                    UploadState::Done => "done".to_string(),
                    UploadState::Failed { reason } => format!("failed: {reason}"),
                };
                if last_label.as_deref() != Some(label.as_str()) {
                    eprintln!("state: {label}");
                    last_label = Some(label);
                }
                match e.state.clone() {
                    UploadState::Done => return Ok(UploadState::Done),
                    UploadState::Failed { reason } => {
                        return Ok(UploadState::Failed { reason });
                    }
                    _ => {}
                }
            }
            None => {
                // The daemon may evict finished entries from the
                // queue snapshot once they're old; treat a missing
                // entry as Done and let `FetchReport` succeed or
                // surface its own error.
                if last_label.as_deref() == Some("decoding")
                    || last_label.as_deref() == Some("queued")
                {
                    eprintln!("state: not in queue (assumed done)");
                    return Ok(UploadState::Done);
                }
                if last_label.is_none() {
                    eprintln!("state: not in queue (assumed done)");
                    return Ok(UploadState::Done);
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn list_queue(conn: &Connection) -> Result<Vec<UploadEntry>> {
    let (mut send, mut recv) = conn.open_bi().await.context("open bi-stream")?;
    write_framed_json(&mut send, &UploadClientMsg::ListQueue).await?;
    let _ = send.finish();
    let reply: UploadServerMsg = read_framed_json(&mut recv).await?;
    match reply {
        UploadServerMsg::QueueSnapshot { entries } => Ok(entries),
        UploadServerMsg::Error { code, message } => {
            bail!("ListQueue failed: {code}: {message}");
        }
        other => bail!("unexpected reply to ListQueue: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// FetchReport
// ---------------------------------------------------------------------------

async fn fetch_report_bytes(conn: &Connection, blake3_hex: &str) -> Result<Vec<u8>> {
    let (mut send, mut recv) = conn.open_bi().await.context("open bi-stream")?;
    write_framed_json(
        &mut send,
        &UploadClientMsg::FetchReport {
            blake3_hex: blake3_hex.to_string(),
        },
    )
    .await?;
    let _ = send.finish();
    let reply: UploadServerMsg = read_framed_json(&mut recv).await?;
    match reply {
        UploadServerMsg::Report { json_bytes, .. } => Ok(json_bytes),
        UploadServerMsg::Error { code, message } => {
            bail!("FetchReport failed: {code}: {message}");
        }
        other => bail!("unexpected reply to FetchReport: {other:?}"),
    }
}

async fn fetch_report_summary_line(
    conn: &Connection,
    blake3_hex: &str,
) -> Result<String> {
    let bytes = fetch_report_bytes(conn, blake3_hex).await?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .context("parse report.json from daemon")?;
    Ok(summary_line_from_report(&value))
}

/// Render a one-line summary from `report.json`. Falls back to a
/// minimal placeholder if the schema doesn't match what we expect —
/// the report is the daemon's source of truth, not ours, so we don't
/// hard-fail on missing fields.
fn summary_line_from_report(value: &serde_json::Value) -> String {
    let summary = value.get("summary");
    let per_class = summary.and_then(|s| s.get("median_active_count_per_class"));
    let cow = per_class.and_then(|c| c.get("cow")).and_then(|v| v.as_u64());
    let sheep = per_class
        .and_then(|c| c.get("sheep"))
        .and_then(|v| v.as_u64());
    let horse = per_class
        .and_then(|c| c.get("horse"))
        .and_then(|v| v.as_u64());
    let total_median = summary
        .and_then(|s| s.get("median_active_count_total"))
        .and_then(|v| v.as_u64());
    let ci = summary
        .and_then(|s| s.get("bootstrap_ci_95_total"))
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            let lo = arr.first().and_then(|x| x.as_u64())?;
            let hi = arr.get(1).and_then(|x| x.as_u64())?;
            Some((lo, hi))
        });
    let frame_count = value.get("frame_count").and_then(|v| v.as_u64());
    let duration_ms = value.get("duration_ms").and_then(|v| v.as_u64());

    // Prefer the "cow: X (CI [a, b]) | sheep: Y | horse: Z | …" form
    // requested by the plan; fall back if cow's missing.
    let mut out = String::new();
    let cow_str = match (cow, total_median, ci) {
        (Some(c), _, Some((lo, hi))) => format!("cow: {c} (CI [{lo}, {hi}])"),
        (Some(c), _, None) => format!("cow: {c}"),
        (None, Some(t), Some((lo, hi))) => format!("total: {t} (CI [{lo}, {hi}])"),
        (None, Some(t), None) => format!("total: {t}"),
        (None, None, _) => "summary unavailable".to_string(),
    };
    out.push_str(&cow_str);
    if let Some(s) = sheep {
        out.push_str(&format!(" | sheep: {s}"));
    }
    if let Some(h) = horse {
        out.push_str(&format!(" | horse: {h}"));
    }
    if let Some(f) = frame_count {
        out.push_str(&format!(" | frames: {f}"));
    }
    if let Some(d) = duration_ms {
        out.push_str(&format!(" | duration: {:.1}s", d as f64 / 1000.0));
    }
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate that `path` exists, has an allowed extension, and is
/// under the 2 GiB cap.
pub(crate) fn validate_clip(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("file does not exist: {}", path.display());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    let ext_ok = match ext.as_deref() {
        Some(e) => ALLOWED_EXTS.contains(&e),
        None => false,
    };
    if !ext_ok {
        bail!(
            "unsupported extension; expected one of {:?}, got {}",
            ALLOWED_EXTS,
            path.display()
        );
    }
    let meta = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?;
    if !meta.is_file() {
        bail!("not a regular file: {}", path.display());
    }
    if meta.len() > MAX_CLIP_BYTES {
        bail!(
            "clip is {} but cap is {} (2 GiB)",
            fmt_bytes(meta.len()),
            fmt_bytes(MAX_CLIP_BYTES)
        );
    }
    if meta.len() == 0 {
        bail!("clip is empty: {}", path.display());
    }
    Ok(())
}

/// Resolve a (possibly truncated) BLAKE3 prefix to a unique full hex
/// from the queue snapshot. Errors if there are 0 or >1 matches.
fn resolve_prefix(prefix: &str, entries: &[UploadEntry]) -> Result<String> {
    if prefix.is_empty() {
        bail!("empty prefix");
    }
    let lower = prefix.to_ascii_lowercase();
    let mut matches: Vec<&UploadEntry> = entries
        .iter()
        .filter(|e| e.blake3_hex.to_ascii_lowercase().starts_with(&lower))
        .collect();
    match matches.len() {
        0 => bail!("no upload matches prefix {prefix:?}"),
        1 => Ok(matches.remove(0).blake3_hex.clone()),
        n => bail!("prefix {prefix:?} matches {n} uploads; please disambiguate"),
    }
}

fn render_queue_table(entries: &[UploadEntry]) {
    if entries.is_empty() {
        println!("(queue empty)");
        return;
    }
    println!(
        "{:<10}  {:<32}  {:>10}  {:<12}  {:>13}",
        "BLAKE3", "FILENAME", "SIZE", "STATE", "QUEUED_TS_MS"
    );
    for e in entries {
        let prefix: String = e.blake3_hex.chars().take(10).collect();
        let fname: String = if e.filename.len() > 32 {
            let mut t: String = e.filename.chars().take(29).collect();
            t.push_str("...");
            t
        } else {
            e.filename.clone()
        };
        let state = match &e.state {
            UploadState::Queued => "queued".to_string(),
            UploadState::Decoding => "decoding".to_string(),
            UploadState::Done => "done".to_string(),
            UploadState::Failed { reason } => {
                let mut s = format!("failed:{reason}");
                if s.len() > 12 {
                    s.truncate(12);
                }
                s
            }
        };
        println!(
            "{:<10}  {:<32}  {:>10}  {:<12}  {:>13}",
            prefix,
            fname,
            fmt_bytes(e.size_bytes),
            state,
            e.queued_ts_ms
        );
    }
}

fn fmt_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if n >= GB {
        format!("{:.2} GiB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.1} MiB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.1} KiB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

/// Streaming BLAKE3 hash. Files larger than [`HASH_OFFLOAD_THRESHOLD`]
/// run on `spawn_blocking` so the tokio runtime stays responsive.
async fn hash_file(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?;
    let size = meta.len();
    let path_buf: PathBuf = path.to_path_buf();
    if size >= HASH_OFFLOAD_THRESHOLD {
        let h = tokio::task::spawn_blocking(move || hash_file_sync(&path_buf, size))
            .await
            .context("hash thread join")??;
        Ok(h)
    } else {
        hash_file_sync(&path_buf, size)
    }
}

fn hash_file_sync(path: &Path, total: u64) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut read: u64 = 0;
    let mut next_pct: u64 = 5;
    loop {
        let n = f.read(&mut buf).context("read clip for hashing")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        read += n as u64;
        if total > 0 {
            let pct = (read * 100) / total;
            if pct >= next_pct {
                eprintln!("hashing... {pct}%");
                next_pct = pct + 5;
            }
        }
    }
    let h = hasher.finalize();
    Ok(h.to_hex().to_string())
}

// ---------------------------------------------------------------------------
// Length-prefixed JSON framing — mirrors `herd-scout-daemon::ipc::frame`
// ---------------------------------------------------------------------------

/// Read one length-prefixed JSON value. Length is `u32 BE`, capped at
/// [`MAX_FRAME`].
async fn read_framed_json<R: AsyncRead + Unpin, T: serde::de::DeserializeOwned>(
    r: &mut R,
) -> Result<T> {
    let bytes = read_framed(r).await?;
    let value: T = serde_json::from_slice(&bytes).context("parse framed JSON")?;
    Ok(value)
}

/// Write a value as length-prefixed JSON.
async fn write_framed_json<W: AsyncWrite + Unpin, T: serde::Serialize>(
    w: &mut W,
    value: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(value).context("serialize framed JSON")?;
    write_framed(w, &bytes).await
}

async fn read_framed<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .await
        .context("read frame length")?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        bail!("frame size {len} exceeds cap {MAX_FRAME}");
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await.context("read frame body")?;
    Ok(buf)
}

async fn write_framed<W: AsyncWrite + Unpin>(w: &mut W, payload: &[u8]) -> Result<()> {
    if payload.len() as u64 > MAX_FRAME as u64 {
        bail!("frame size {} exceeds cap {MAX_FRAME}", payload.len());
    }
    let len = (payload.len() as u32).to_be_bytes();
    w.write_all(&len).await.context("write frame length")?;
    w.write_all(payload).await.context("write frame body")?;
    w.flush().await.context("flush frame")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use tokio::io::duplex;

    #[test]
    fn validate_clip_rejects_unknown_extension() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("clip.avi");
        std::fs::write(&path, b"x").unwrap();
        let err = validate_clip(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("unsupported extension"),
            "expected 'unsupported extension' in {msg}"
        );
    }

    #[test]
    fn validate_clip_rejects_missing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.mp4");
        let err = validate_clip(&path).unwrap_err();
        assert!(format!("{err:#}").contains("does not exist"));
    }

    #[test]
    fn validate_clip_rejects_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.mp4");
        std::fs::write(&path, b"").unwrap();
        let err = validate_clip(&path).unwrap_err();
        assert!(format!("{err:#}").contains("empty"));
    }

    #[test]
    fn validate_clip_accepts_small_mp4() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("clip.mp4");
        std::fs::write(&path, b"some bytes").unwrap();
        validate_clip(&path).unwrap();
    }

    #[test]
    fn validate_clip_accepts_uppercase_ext() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("CLIP.MOV");
        std::fs::write(&path, b"some bytes").unwrap();
        validate_clip(&path).unwrap();
    }

    #[test]
    fn validate_clip_rejects_too_big() {
        // We fake "too big" by stubbing a sparse file. On macOS
        // `set_len` produces a sparse file with the requested size
        // without consuming disk; on Linux the same. If the platform
        // refuses (unlikely on tmpdir), we skip rather than fail.
        let dir = tempdir().unwrap();
        let path = dir.path().join("big.mp4");
        let f = std::fs::File::create(&path).unwrap();
        if f.set_len(MAX_CLIP_BYTES + 1).is_err() {
            return; // platform won't let us; skip
        }
        drop(f);
        let err = validate_clip(&path).unwrap_err();
        assert!(format!("{err:#}").contains("cap"));
    }

    #[tokio::test]
    async fn blake3_helper_matches_canonical() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        let payload = b"the quick brown fox jumps over the lazy dog";
        std::fs::write(&path, payload).unwrap();

        let got = hash_file(&path).await.unwrap();
        let want = blake3::hash(payload).to_hex().to_string();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn blake3_helper_handles_large_offload_path() {
        // Construct a file just over the offload threshold to drive
        // the spawn_blocking branch. Use a small repeating pattern so
        // we don't allocate >50 MiB in test output.
        let dir = tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        let chunk = vec![0xa5u8; 1024 * 1024]; // 1 MiB
        let mut hasher = blake3::Hasher::new();
        let target = HASH_OFFLOAD_THRESHOLD + 1024 * 1024;
        let mut written: u64 = 0;
        while written < target {
            let want = std::cmp::min(chunk.len() as u64, target - written) as usize;
            f.write_all(&chunk[..want]).unwrap();
            hasher.update(&chunk[..want]);
            written += want as u64;
        }
        drop(f);

        let got = hash_file(&path).await.unwrap();
        let want = hasher.finalize().to_hex().to_string();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn frame_length_prefix_roundtrip_push() {
        let (a, b) = duplex(64 * 1024);
        let (_ar, mut aw) = tokio::io::split(a);
        let (mut br, _bw) = tokio::io::split(b);

        let msg = UploadClientMsg::Push {
            filename: "drone-flyover.mp4".into(),
            size_bytes: 12_345_678,
            blake3_hex: "9c2f".repeat(16),
        };
        let writer = tokio::spawn(async move {
            write_framed_json(&mut aw, &msg).await.unwrap();
            drop(aw);
        });
        let parsed: UploadClientMsg = read_framed_json(&mut br).await.unwrap();
        writer.await.unwrap();
        match parsed {
            UploadClientMsg::Push {
                filename,
                size_bytes,
                blake3_hex,
            } => {
                assert_eq!(filename, "drone-flyover.mp4");
                assert_eq!(size_bytes, 12_345_678);
                assert_eq!(blake3_hex, "9c2f".repeat(16));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn frame_length_prefix_roundtrip_server_msgs() {
        // Server-side enums round-trip through the same framing —
        // covers the response-decode path the CLI relies on.
        let cases = vec![
            UploadServerMsg::Accepted {
                blake3_hex: "ab".repeat(32),
            },
            UploadServerMsg::Ok,
            UploadServerMsg::Error {
                code: "x".into(),
                message: "y".into(),
            },
            UploadServerMsg::QueueSnapshot { entries: vec![] },
        ];
        for msg in cases {
            let (a, b) = duplex(64 * 1024);
            let (_ar, mut aw) = tokio::io::split(a);
            let (mut br, _bw) = tokio::io::split(b);
            let dbg = format!("{msg:?}");
            let writer = tokio::spawn(async move {
                write_framed_json(&mut aw, &msg).await.unwrap();
                drop(aw);
            });
            let parsed: UploadServerMsg = read_framed_json(&mut br).await.unwrap();
            writer.await.unwrap();
            assert_eq!(format!("{parsed:?}"), dbg);
        }
    }

    #[test]
    fn resolve_prefix_finds_unique_match() {
        let entries = vec![
            UploadEntry {
                blake3_hex: "abc123".into(),
                filename: "a.mp4".into(),
                size_bytes: 1,
                state: UploadState::Queued,
                queued_ts_ms: 0,
                started_ts_ms: None,
                finished_ts_ms: None,
            },
            UploadEntry {
                blake3_hex: "def456".into(),
                filename: "b.mp4".into(),
                size_bytes: 2,
                state: UploadState::Done,
                queued_ts_ms: 1,
                started_ts_ms: None,
                finished_ts_ms: None,
            },
        ];
        let got = resolve_prefix("abc", &entries).unwrap();
        assert_eq!(got, "abc123");
    }

    #[test]
    fn resolve_prefix_errors_on_no_match() {
        let entries = vec![UploadEntry {
            blake3_hex: "abc123".into(),
            filename: "a.mp4".into(),
            size_bytes: 1,
            state: UploadState::Queued,
            queued_ts_ms: 0,
            started_ts_ms: None,
            finished_ts_ms: None,
        }];
        let err = resolve_prefix("xyz", &entries).unwrap_err();
        assert!(format!("{err:#}").contains("no upload"));
    }

    #[test]
    fn resolve_prefix_errors_on_ambiguous() {
        let entries = vec![
            UploadEntry {
                blake3_hex: "abc111".into(),
                filename: "a.mp4".into(),
                size_bytes: 1,
                state: UploadState::Queued,
                queued_ts_ms: 0,
                started_ts_ms: None,
                finished_ts_ms: None,
            },
            UploadEntry {
                blake3_hex: "abc222".into(),
                filename: "b.mp4".into(),
                size_bytes: 2,
                state: UploadState::Queued,
                queued_ts_ms: 0,
                started_ts_ms: None,
                finished_ts_ms: None,
            },
        ];
        let err = resolve_prefix("abc", &entries).unwrap_err();
        assert!(format!("{err:#}").contains("matches 2"));
    }

    #[test]
    fn summary_line_renders_full_schema() {
        let report = serde_json::json!({
            "frame_count": 2624,
            "duration_ms": 87520,
            "summary": {
                "median_active_count_total": 47,
                "median_active_count_per_class": { "horse": 0, "sheep": 0, "cow": 47 },
                "bootstrap_ci_95_total": [44, 51]
            }
        });
        let line = summary_line_from_report(&report);
        assert!(line.contains("cow: 47"), "got: {line}");
        assert!(line.contains("CI [44, 51]"), "got: {line}");
        assert!(line.contains("sheep: 0"), "got: {line}");
        assert!(line.contains("horse: 0"), "got: {line}");
        assert!(line.contains("frames: 2624"), "got: {line}");
        assert!(line.contains("87.5s"), "got: {line}");
    }

    #[test]
    fn summary_line_handles_missing_fields() {
        let report = serde_json::json!({});
        let line = summary_line_from_report(&report);
        // Should not panic; should produce some non-empty placeholder.
        assert!(!line.is_empty());
    }
}
