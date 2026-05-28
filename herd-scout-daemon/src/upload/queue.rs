//! In-memory + persisted upload queue (Wave 13 / Phase 2).
//!
//! Per `plan-desktop-video-upload-2026-05-28.md` Decision 4 + 7 the
//! queue is single-clip-at-a-time and lives at
//! `<data_dir>/uploads/queue.json`. The daemon's [`UploadProcessor`]
//! pops the head when no live phone session is active; new entries are
//! appended by [`super::handler`].
//!
//! Persistence is "rewrite the whole file atomically on every
//! mutation". The queue holds at most a handful of entries (the cap is
//! soft — we trust the user not to enqueue 10k clips), so a full
//! serialize cost is negligible.
//!
//! On daemon restart [`Queue::load_or_init`] clears any `Decoding`
//! state back to `Queued` — the in-flight clip will be re-decoded from
//! frame zero. The plan's risk-table accepts this v1 trade.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use herd_scout_ipc::{UploadEntry, UploadState};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Notify};

use super::store::QUEUE_FILENAME;

// ---------------------------------------------------------------------
// Persisted shape
// ---------------------------------------------------------------------

/// On-disk envelope for `queue.json`. Versioned so future changes
/// (e.g. switching to a SQLite-backed queue) can migrate cleanly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct QueueFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    entries: Vec<UploadEntry>,
}

fn default_version() -> u32 {
    1
}

// ---------------------------------------------------------------------
// In-memory queue
// ---------------------------------------------------------------------

#[derive(Debug)]
struct QueueInner {
    path: PathBuf,
    entries: Vec<UploadEntry>,
}

/// Thread-safe upload queue. Cheap to clone (`Arc` internals).
#[derive(Debug, Clone)]
pub struct Queue {
    inner: Arc<Mutex<QueueInner>>,
    /// Notified whenever an entry transitions to/into `Queued` or a
    /// new entry is appended. The processor task uses this to wake
    /// up without polling.
    notify: Arc<Notify>,
}

/// Outcome of a `cancel` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The entry was in `Queued`, marked as `Failed { reason: "cancelled" }`.
    Cancelled,
    /// The entry was past `Queued` (e.g. `Decoding`); cancel is a no-op
    /// at the queue layer. The processor's mid-clip cancel path is a
    /// follow-up; v1 lets the clip finish.
    NotCancellable,
    /// No entry with that hash was found.
    NotFound,
}

impl Queue {
    /// Load the queue from `<uploads_dir>/queue.json`, creating a fresh
    /// empty file if it doesn't exist. Any `Decoding` entries left
    /// over from a previous run are reset to `Queued`.
    pub async fn load_or_init(uploads_dir: &Path) -> Result<Self> {
        let path = uploads_dir.join(QUEUE_FILENAME);
        let entries = match fs::read(&path).await {
            Ok(bytes) => {
                let file: QueueFile = serde_json::from_slice(&bytes).with_context(|| {
                    format!("parse {} (delete or fix the file)", path.display())
                })?;
                let mut entries = file.entries;
                for e in entries.iter_mut() {
                    if matches!(e.state, UploadState::Decoding) {
                        // Restart from `Queued` — the bytes are still
                        // on disk (`<uploads_dir>/<blake3>/clip.<ext>`),
                        // but the in-progress sidecar response stream
                        // is gone with the previous process.
                        e.state = UploadState::Queued;
                        e.started_ts_ms = None;
                        tracing::info!(
                            blake3 = %e.blake3_hex,
                            "upload-queue: resetting Decoding → Queued on restart",
                        );
                    }
                }
                entries
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(e).with_context(|| format!("read {}", path.display()));
            }
        };
        let q = Self {
            inner: Arc::new(Mutex::new(QueueInner {
                path,
                entries,
            })),
            notify: Arc::new(Notify::new()),
        };
        q.persist().await?;
        Ok(q)
    }

    /// Subscribe to "queue changed" notifications. Each `notified()`
    /// future returns when there's been at least one mutation since
    /// it was created. Used by the processor's wait loop.
    pub fn notify_handle(&self) -> Arc<Notify> {
        self.notify.clone()
    }

    /// Snapshot of all entries in queue order.
    pub async fn snapshot(&self) -> Vec<UploadEntry> {
        self.inner.lock().await.entries.clone()
    }

    /// Lookup one entry by full BLAKE3 hex.
    pub async fn get(&self, blake3_hex: &str) -> Option<UploadEntry> {
        self.inner
            .lock()
            .await
            .entries
            .iter()
            .find(|e| e.blake3_hex == blake3_hex)
            .cloned()
    }

    /// Append a new `Queued` entry, or — if an entry with the same
    /// `blake3_hex` already exists in any state — return the existing
    /// entry without modification (the BLAKE3 dedupe behavior promised
    /// in plan Decision 3).
    pub async fn enqueue(&self, entry: UploadEntry) -> UploadEntry {
        let mut inner = self.inner.lock().await;
        if let Some(existing) = inner.entries.iter().find(|e| e.blake3_hex == entry.blake3_hex) {
            return existing.clone();
        }
        inner.entries.push(entry.clone());
        let path = inner.path.clone();
        let snapshot = inner.entries.clone();
        drop(inner);
        let _ = persist_to(&path, &snapshot).await;
        self.notify.notify_waiters();
        entry
    }

    /// Cancel a queued entry by exact hash. Marks it `Failed { reason:
    /// "cancelled" }` so the GUI can render the same row in its
    /// "recent" history without an extra variant.
    ///
    /// Returns the outcome — see [`CancelOutcome`] — so callers can
    /// surface "not found" / "too late" distinctly.
    pub async fn cancel(&self, blake3_hex: &str) -> CancelOutcome {
        let mut inner = self.inner.lock().await;
        let Some(idx) = inner
            .entries
            .iter()
            .position(|e| e.blake3_hex == blake3_hex)
        else {
            return CancelOutcome::NotFound;
        };
        if !matches!(inner.entries[idx].state, UploadState::Queued) {
            return CancelOutcome::NotCancellable;
        }
        inner.entries[idx].state = UploadState::Failed {
            reason: "cancelled".to_string(),
        };
        inner.entries[idx].finished_ts_ms = Some(crate::audit::now_unix_ms());
        let path = inner.path.clone();
        let snapshot = inner.entries.clone();
        drop(inner);
        let _ = persist_to(&path, &snapshot).await;
        self.notify.notify_waiters();
        CancelOutcome::Cancelled
    }

    /// Pop the next `Queued` entry (head-of-queue) and atomically
    /// transition it to `Decoding` with `started_ts_ms = now`. Returns
    /// `None` when no `Queued` entry exists.
    pub async fn pop_for_processing(&self) -> Option<UploadEntry> {
        let mut inner = self.inner.lock().await;
        let now_ms = crate::audit::now_unix_ms();
        for entry in inner.entries.iter_mut() {
            if matches!(entry.state, UploadState::Queued) {
                entry.state = UploadState::Decoding;
                entry.started_ts_ms = Some(now_ms);
                let updated = entry.clone();
                let path = inner.path.clone();
                let snapshot = inner.entries.clone();
                drop(inner);
                let _ = persist_to(&path, &snapshot).await;
                return Some(updated);
            }
        }
        None
    }

    /// Mark a `Decoding` entry `Done` with `finished_ts_ms = now`. No-op
    /// if the entry isn't found or isn't currently `Decoding`.
    pub async fn mark_done(&self, blake3_hex: &str) {
        self.update_finished(blake3_hex, UploadState::Done).await;
    }

    /// Mark a `Decoding` entry `Failed { reason }` with `finished_ts_ms = now`.
    pub async fn mark_failed(&self, blake3_hex: &str, reason: impl Into<String>) {
        self.update_finished(
            blake3_hex,
            UploadState::Failed {
                reason: reason.into(),
            },
        )
        .await;
    }

    async fn update_finished(&self, blake3_hex: &str, new_state: UploadState) {
        let mut inner = self.inner.lock().await;
        let now_ms = crate::audit::now_unix_ms();
        if let Some(entry) = inner.entries.iter_mut().find(|e| e.blake3_hex == blake3_hex) {
            entry.state = new_state;
            entry.finished_ts_ms = Some(now_ms);
            let path = inner.path.clone();
            let snapshot = inner.entries.clone();
            drop(inner);
            let _ = persist_to(&path, &snapshot).await;
            self.notify.notify_waiters();
        }
    }

    async fn persist(&self) -> Result<()> {
        let inner = self.inner.lock().await;
        let path = inner.path.clone();
        let snapshot = inner.entries.clone();
        drop(inner);
        persist_to(&path, &snapshot).await
    }
}

async fn persist_to(path: &Path, entries: &[UploadEntry]) -> Result<()> {
    let file = QueueFile {
        version: 1,
        entries: entries.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&file).context("serialize queue.json")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create queue.json parent {}", parent.display()))?;
    }
    let tmp_path = path.with_extension(format!(
        "json.tmp.{}.{}",
        std::process::id(),
        crate::audit::now_unix_ms(),
    ));
    {
        let mut f = fs::File::create(&tmp_path)
            .await
            .with_context(|| format!("create temp {}", tmp_path.display()))?;
        f.write_all(&bytes).await.context("write queue.json")?;
        f.sync_all().await.context("fsync queue.json")?;
    }
    fs::rename(&tmp_path, path)
        .await
        .with_context(|| format!("rename {} → {}", tmp_path.display(), path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(blake3_hex: &str, filename: &str) -> UploadEntry {
        UploadEntry {
            blake3_hex: blake3_hex.to_string(),
            filename: filename.to_string(),
            size_bytes: 1024,
            state: UploadState::Queued,
            queued_ts_ms: 1_700_000_000_000,
            started_ts_ms: None,
            finished_ts_ms: None,
        }
    }

    #[tokio::test]
    async fn queue_persists_across_load() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        let q1 = Queue::load_or_init(&dir).await.unwrap();
        q1.enqueue(entry("a".repeat(64).as_str(), "first.mp4")).await;
        q1.enqueue(entry("b".repeat(64).as_str(), "second.mp4")).await;
        q1.enqueue(entry("c".repeat(64).as_str(), "third.mp4")).await;

        let q2 = Queue::load_or_init(&dir).await.unwrap();
        let snapshot = q2.snapshot().await;
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].filename, "first.mp4");
        assert_eq!(snapshot[1].filename, "second.mp4");
        assert_eq!(snapshot[2].filename, "third.mp4");
    }

    #[tokio::test]
    async fn cancel_queued_returns_no_op_for_unknown() {
        let tmp = TempDir::new().unwrap();
        let q = Queue::load_or_init(tmp.path()).await.unwrap();
        let outcome = q.cancel(&"00".repeat(32)).await;
        assert_eq!(outcome, CancelOutcome::NotFound);
    }

    #[tokio::test]
    async fn cancel_queued_marks_failed() {
        let tmp = TempDir::new().unwrap();
        let q = Queue::load_or_init(tmp.path()).await.unwrap();
        let hex = "9c2f".repeat(16);
        q.enqueue(entry(&hex, "pending.mp4")).await;
        let outcome = q.cancel(&hex).await;
        assert_eq!(outcome, CancelOutcome::Cancelled);
        let snap = q.snapshot().await;
        match &snap[0].state {
            UploadState::Failed { reason } => assert_eq!(reason, "cancelled"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_decoding_is_noop() {
        let tmp = TempDir::new().unwrap();
        let q = Queue::load_or_init(tmp.path()).await.unwrap();
        let hex = "abcd".repeat(16);
        q.enqueue(entry(&hex, "running.mp4")).await;
        let popped = q.pop_for_processing().await.unwrap();
        assert_eq!(popped.blake3_hex, hex);
        // Now in Decoding. Cancel should be a no-op.
        let outcome = q.cancel(&hex).await;
        assert_eq!(outcome, CancelOutcome::NotCancellable);
        // State is still Decoding.
        let snap = q.snapshot().await;
        assert!(matches!(snap[0].state, UploadState::Decoding));
    }

    #[tokio::test]
    async fn pop_returns_none_when_empty() {
        let tmp = TempDir::new().unwrap();
        let q = Queue::load_or_init(tmp.path()).await.unwrap();
        assert!(q.pop_for_processing().await.is_none());
    }

    #[tokio::test]
    async fn enqueue_dedupes_by_hash() {
        let tmp = TempDir::new().unwrap();
        let q = Queue::load_or_init(tmp.path()).await.unwrap();
        let hex = "00".repeat(32);
        let first = q.enqueue(entry(&hex, "first.mp4")).await;
        let second = q.enqueue(entry(&hex, "second.mp4")).await;
        assert_eq!(first.filename, "first.mp4");
        // Second enqueue returns the *existing* entry (the first one).
        assert_eq!(second.filename, "first.mp4");
        let snap = q.snapshot().await;
        assert_eq!(snap.len(), 1);
    }

    #[tokio::test]
    async fn restart_resets_decoding_to_queued() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();
        let q = Queue::load_or_init(&dir).await.unwrap();
        let hex = "ee".repeat(32);
        q.enqueue(entry(&hex, "x.mp4")).await;
        q.pop_for_processing().await.unwrap();
        // Simulate a daemon restart — drop q1, reopen.
        drop(q);
        let q2 = Queue::load_or_init(&dir).await.unwrap();
        let snap = q2.snapshot().await;
        assert!(matches!(snap[0].state, UploadState::Queued));
        assert_eq!(snap[0].started_ts_ms, None);
    }

    #[tokio::test]
    async fn mark_done_transitions_state() {
        let tmp = TempDir::new().unwrap();
        let q = Queue::load_or_init(tmp.path()).await.unwrap();
        let hex = "ff".repeat(32);
        q.enqueue(entry(&hex, "x.mp4")).await;
        q.pop_for_processing().await.unwrap();
        q.mark_done(&hex).await;
        let snap = q.snapshot().await;
        assert!(matches!(snap[0].state, UploadState::Done));
        assert!(snap[0].finished_ts_ms.is_some());
    }
}
