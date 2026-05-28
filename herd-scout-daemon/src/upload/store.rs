//! On-disk staging layout for batch-uploaded clips (Wave 13 / Phase 2).
//!
//! Per `plan-desktop-video-upload-2026-05-28.md` Decision 6 the
//! canonical layout is:
//!
//! ```text
//! <data_dir>/uploads/
//! ├── queue.json
//! └── <blake3-hex>/
//!     ├── clip.<ext>
//!     ├── meta.json
//!     └── report.json   (written separately by upload::report)
//! ```
//!
//! `<data_dir>` is resolved via the same `directories::ProjectDirs`
//! triple as the audit log + identity envelope, so all daemon state
//! lives under one root.
//!
//! This module owns:
//! * [`resolve_uploads_dir`] — the canonical path resolver.
//! * [`UploadStager`] — streaming-bytes-to-disk writer that hashes the
//!   payload with BLAKE3, verifies it against the client-reported hex,
//!   atomically renames into the per-clip layout, and writes a
//!   `meta.json` companion file.
//! * [`write_meta_json`] — atomic `meta.json` writer (used by the
//!   stager and exposed for re-staging cases).
//!
//! The stager is intentionally a thin builder rather than a `tokio`
//! actor — the `handler.rs` tasks own their stream lifetime and call
//! `update` per chunk, then `finalize` once. That keeps cancellation
//! semantics obvious (drop the stager → temp file is cleaned up by the
//! `Drop` impl).
//!
//! ## v1 deviation from plan Decision 6
//!
//! The plan called for hardlinking (fallback symlink, fallback copy) the
//! clip from a separate iroh-blobs store into
//! `<data_dir>/uploads/<blake3-hex>/clip.<ext>`. The shipped code does
//! not yet host an iroh-blobs store on the daemon — bytes ride inline on
//! the same QUIC bi-stream as the JSON `Push` metadata (see
//! [`super::protocol`] for that wire detail), and `UploadStager`
//! atomically renames a single concrete file at the staging path. The
//! clip is therefore the *only* copy.
//!
//! When iroh-blobs lands in a follow-up Wave, replace the direct
//! atomic-rename in [`UploadStager::finalize`] with a hardlink (fallback
//! symlink, fallback copy) from the blob store. The wire format already
//! names a BLAKE3, so the transport-only swap is clean.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Subdirectory name under `<data_dir>` that holds the upload pipeline's
/// state. Kept consistent across the codebase via this single constant.
pub const UPLOADS_SUBDIR: &str = "uploads";

/// Filename of the persisted queue snapshot.
pub const QUEUE_FILENAME: &str = "queue.json";

// ---------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------

/// Resolve `<data_dir>/uploads/`, creating any missing directories.
///
/// Mirrors `audit::audit_dir`'s `ProjectDirs` triple so all daemon
/// state lives under one root.
pub fn resolve_uploads_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("net", "herd-scout", "herd-scout")
        .ok_or_else(|| anyhow!("no user-data directory available on this platform"))?;
    let path = dirs.data_dir().join(UPLOADS_SUBDIR);
    std::fs::create_dir_all(&path)
        .with_context(|| format!("create uploads dir {}", path.display()))?;
    Ok(path)
}

/// Per-clip directory: `<uploads_dir>/<blake3_hex>/`.
pub fn clip_dir(uploads_dir: &Path, blake3_hex: &str) -> PathBuf {
    uploads_dir.join(blake3_hex)
}

/// Choose the on-disk filename for the staged clip from the original
/// uploaded filename's extension. Falls back to `clip.bin` when the
/// extension is missing or contains directory separators.
pub fn clip_filename_from(original: &str) -> String {
    let ext = Path::new(original)
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && !s.contains('/') && !s.contains('\\'))
        .unwrap_or("bin");
    format!("clip.{ext}")
}

// ---------------------------------------------------------------------
// meta.json
// ---------------------------------------------------------------------

/// Companion JSON written next to `clip.<ext>` so a finished upload
/// directory is self-describing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipMeta {
    pub blake3_hex: String,
    pub filename: String,
    pub size_bytes: u64,
    pub upload_ts_ms: u64,
    /// `EndpointId` (canonical) of the iroh peer that pushed the clip,
    /// when known. `None` for local GUI handoffs.
    #[serde(default)]
    pub source_node_id: Option<String>,
}

/// Atomically write `meta.json` into `dir`. Same temp+rename pattern
/// as `report.rs::ClipReport::write_atomic`.
pub async fn write_meta_json(dir: &Path, meta: &ClipMeta) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(meta).context("serialize meta.json")?;
    let final_path = dir.join("meta.json");
    let tmp_path = dir.join(format!(
        "meta.json.tmp.{}.{}",
        std::process::id(),
        crate::audit::now_unix_ms(),
    ));
    {
        let mut f = fs::File::create(&tmp_path)
            .await
            .with_context(|| format!("create temp {}", tmp_path.display()))?;
        f.write_all(&bytes).await.context("write meta.json bytes")?;
        f.sync_all().await.context("fsync meta.json")?;
    }
    fs::rename(&tmp_path, &final_path)
        .await
        .with_context(|| format!("rename {} → {}", tmp_path.display(), final_path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------
// Streaming clip stager
// ---------------------------------------------------------------------

/// Fixed cap on accepted upload byte-stream size. Per plan Decision 7
/// (Phase 2 task list).
pub const MAX_UPLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Streaming writer used while the daemon receives the clip's bytes
/// over the upload ALPN. Hashes the payload with BLAKE3 as it goes,
/// verifies against the client-reported hex on `finalize`, and renames
/// the temp file into place atomically.
///
/// Ownership semantics: the stager owns the temp file until either
/// `finalize` succeeds (rename atomically commits) or the stager is
/// dropped (best-effort temp cleanup).
pub struct UploadStager {
    uploads_dir: PathBuf,
    /// `Some(file)` while bytes are still being written; `None` after
    /// `finalize` (or after `drop` if it cleaned up).
    file: Option<fs::File>,
    tmp_path: PathBuf,
    expected_hex: String,
    expected_size: u64,
    bytes_written: u64,
    hasher: blake3::Hasher,
}

impl std::fmt::Debug for UploadStager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadStager")
            .field("tmp_path", &self.tmp_path)
            .field("expected_hex", &self.expected_hex)
            .field("expected_size", &self.expected_size)
            .field("bytes_written", &self.bytes_written)
            .finish_non_exhaustive()
    }
}

/// Outcome of [`UploadStager::finalize`]. The successful case carries
/// the per-clip directory the caller should write `report.json` into.
#[derive(Debug, Clone)]
pub enum StagerOutcome {
    /// All bytes received and hashes matched. Final file lives at
    /// `clip_path`.
    Ok {
        clip_dir: PathBuf,
        clip_path: PathBuf,
    },
    /// Hash mismatch — the temp file has been removed. `computed` is
    /// the hex actually observed; `reported` is what the client said.
    HashMismatch { reported: String, computed: String },
}

impl UploadStager {
    /// Open a fresh temp file under `<uploads_dir>/.staging/` for the
    /// incoming clip. Creates the staging dir on first use.
    pub async fn create(
        uploads_dir: &Path,
        blake3_hex: &str,
        expected_size: u64,
    ) -> Result<Self> {
        if expected_size > MAX_UPLOAD_BYTES {
            return Err(anyhow!(
                "expected_size {expected_size} exceeds cap {MAX_UPLOAD_BYTES}"
            ));
        }
        let staging = uploads_dir.join(".staging");
        fs::create_dir_all(&staging)
            .await
            .with_context(|| format!("create staging dir {}", staging.display()))?;
        // Use the (already-known) hex + pid + nanos so concurrent
        // stagers can't collide. Only the first 16 chars of hex are
        // used to keep filenames short on long-hash paths.
        let stem: String = blake3_hex.chars().take(16).collect();
        let tmp_path = staging.join(format!(
            "{}.{}.{}.staging",
            if stem.is_empty() { "anon" } else { &stem },
            std::process::id(),
            crate::audit::now_unix_ms(),
        ));
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)
            .await
            .with_context(|| format!("open staging temp {}", tmp_path.display()))?;
        Ok(Self {
            uploads_dir: uploads_dir.to_path_buf(),
            file: Some(file),
            tmp_path,
            expected_hex: blake3_hex.to_string(),
            expected_size,
            bytes_written: 0,
            hasher: blake3::Hasher::new(),
        })
    }

    /// Write `chunk` to the temp file and update the running hash.
    pub async fn update(&mut self, chunk: &[u8]) -> Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        if self.bytes_written + chunk.len() as u64 > self.expected_size {
            return Err(anyhow!(
                "received more bytes than expected: written={} chunk={} cap={}",
                self.bytes_written,
                chunk.len(),
                self.expected_size
            ));
        }
        let f = self
            .file
            .as_mut()
            .ok_or_else(|| anyhow!("stager already finalized"))?;
        f.write_all(chunk).await.context("append staging chunk")?;
        self.hasher.update(chunk);
        self.bytes_written += chunk.len() as u64;
        Ok(())
    }

    /// Bytes written so far. Caller can use this to drive the upload
    /// progress percentage shown to the GUI.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Finish the stager. On hash match, renames the temp file into
    /// `<uploads_dir>/<blake3>/clip.<ext>` (creating the per-clip dir).
    /// On mismatch, deletes the temp file and returns
    /// [`StagerOutcome::HashMismatch`].
    pub async fn finalize(mut self, original_filename: &str) -> Result<StagerOutcome> {
        // Sanity: must have written exactly `expected_size`.
        if self.bytes_written != self.expected_size {
            return Err(anyhow!(
                "byte-count mismatch: written={} expected={}",
                self.bytes_written,
                self.expected_size
            ));
        }
        // Flush + close the file before rename.
        if let Some(mut f) = self.file.take() {
            f.flush().await.context("flush staging file")?;
            f.sync_all().await.context("fsync staging file")?;
            drop(f);
        }
        let computed = self.hasher.finalize();
        let computed_hex = computed.to_hex().to_string();
        if computed_hex != self.expected_hex {
            // Reject — clean up the temp file.
            let _ = fs::remove_file(&self.tmp_path).await;
            return Ok(StagerOutcome::HashMismatch {
                reported: self.expected_hex.clone(),
                computed: computed_hex,
            });
        }
        // Rename into the canonical layout.
        let clip_dir = clip_dir(&self.uploads_dir, &self.expected_hex);
        fs::create_dir_all(&clip_dir)
            .await
            .with_context(|| format!("create clip dir {}", clip_dir.display()))?;
        let clip_path = clip_dir.join(clip_filename_from(original_filename));
        fs::rename(&self.tmp_path, &clip_path)
            .await
            .with_context(|| {
                format!(
                    "rename {} → {}",
                    self.tmp_path.display(),
                    clip_path.display()
                )
            })?;
        Ok(StagerOutcome::Ok {
            clip_dir,
            clip_path,
        })
    }
}

impl Drop for UploadStager {
    fn drop(&mut self) {
        // Best-effort cleanup of the staging file if `finalize` wasn't
        // called (e.g. the stream errored mid-upload, or the task was
        // cancelled). We can't `await` here, so use the sync `std::fs`
        // path — the stager only ever holds a file by absolute path.
        if self.file.is_some() {
            let _ = std::fs::remove_file(&self.tmp_path);
        }
    }
}

/// Stage an in-memory clip for tests + the GUI's local-file handoff
/// path. Returns the same outcome shape as the streaming stager.
///
/// Thin convenience wrapper over [`UploadStager`]. Used by both unit
/// tests and the [`super::handler::handle_local_handoff`] path.
pub async fn stage_clip_bytes(
    uploads_dir: &Path,
    blake3_hex: &str,
    bytes: &[u8],
    original_filename: &str,
) -> Result<StagerOutcome> {
    let mut stager = UploadStager::create(uploads_dir, blake3_hex, bytes.len() as u64).await?;
    stager.update(bytes).await?;
    stager.finalize(original_filename).await
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn known_blake3(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    #[test]
    fn clip_filename_from_keeps_extension() {
        assert_eq!(clip_filename_from("video.mp4"), "clip.mp4");
        assert_eq!(clip_filename_from("VIDEO.MOV"), "clip.MOV");
        assert_eq!(clip_filename_from("nodot"), "clip.bin");
        assert_eq!(clip_filename_from(""), "clip.bin");
    }

    #[test]
    fn clip_filename_from_rejects_separator_in_ext() {
        assert_eq!(clip_filename_from("a.b/c"), "clip.bin");
    }

    #[tokio::test]
    async fn stage_clip_creates_layout() {
        let tmp = TempDir::new().unwrap();
        let bytes = b"hello world. this is a tiny test clip payload.";
        let hex = known_blake3(bytes);
        let outcome = stage_clip_bytes(tmp.path(), &hex, bytes, "drone.mp4")
            .await
            .expect("stage_clip_bytes ok");
        match outcome {
            StagerOutcome::Ok {
                clip_dir,
                clip_path,
            } => {
                assert_eq!(clip_dir, tmp.path().join(&hex));
                assert_eq!(clip_path, clip_dir.join("clip.mp4"));
                let on_disk = std::fs::read(&clip_path).unwrap();
                assert_eq!(on_disk, bytes);
            }
            other => panic!("expected Ok, got {other:?}"),
        }
        // Staging dir is empty (temp file was renamed).
        let staging = tmp.path().join(".staging");
        let entries: Vec<_> = std::fs::read_dir(&staging)
            .map(|rd| rd.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();
        assert!(entries.is_empty(), "staging dir not empty: {entries:?}");
    }

    #[tokio::test]
    async fn blake3_mismatch_rejects() {
        let tmp = TempDir::new().unwrap();
        let bytes = b"some payload bytes";
        // Lie about the hash.
        let bogus = "00".repeat(32);
        let outcome = stage_clip_bytes(tmp.path(), &bogus, bytes, "x.mp4")
            .await
            .expect("call ok");
        match outcome {
            StagerOutcome::HashMismatch { reported, computed } => {
                assert_eq!(reported, bogus);
                assert_eq!(computed, known_blake3(bytes));
            }
            other => panic!("expected HashMismatch, got {other:?}"),
        }
        // No clip dir for the bogus hash.
        let dir = tmp.path().join(&bogus);
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn stager_drop_cleans_up_temp() {
        let tmp = TempDir::new().unwrap();
        let staging = tmp.path().join(".staging");
        {
            let stager = UploadStager::create(tmp.path(), "abcd", 100)
                .await
                .unwrap();
            // Note tmp path before dropping.
            assert!(stager.tmp_path.exists());
            // Drop without finalize.
        }
        // Temp dir should be empty after drop.
        let entries: Vec<_> = std::fs::read_dir(&staging)
            .map(|rd| rd.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();
        assert!(entries.is_empty(), "leftover after drop: {entries:?}");
    }

    #[tokio::test]
    async fn stager_rejects_overwrite() {
        let tmp = TempDir::new().unwrap();
        let mut stager = UploadStager::create(tmp.path(), "ab", 4).await.unwrap();
        stager.update(b"abcd").await.unwrap();
        // Sending more bytes after the cap must error.
        assert!(stager.update(b"e").await.is_err());
    }

    #[tokio::test]
    async fn stager_rejects_oversize_create() {
        let tmp = TempDir::new().unwrap();
        let res = UploadStager::create(tmp.path(), "x", MAX_UPLOAD_BYTES + 1).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn write_meta_json_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();
        let meta = ClipMeta {
            blake3_hex: "9c2f".repeat(16),
            filename: "drone.mp4".into(),
            size_bytes: 12_345,
            upload_ts_ms: 1_700_000_000_000,
            source_node_id: Some("abcd".into()),
        };
        write_meta_json(&dir, &meta).await.unwrap();
        let bytes = std::fs::read(dir.join("meta.json")).unwrap();
        let parsed: ClipMeta = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, meta);
    }
}
