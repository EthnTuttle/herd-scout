//! Wave 12 — append-only JSONL audit log + shared `ControlMetrics`.
//!
//! Layout: one record per line at `<data_dir>/herd-scout/audit.log`.
//! Each record is a versioned JSON object. Daily rotation renames the
//! active file to `audit-YYYY-MM-DD.log` shortly after UTC midnight; a
//! 90-day retention sweep deletes anything older than that.
//!
//! Decisions 8/9/10 of `plan-android-admin-allowlist-app-2026-05-27`:
//! the daemon side is the source of truth for what the daemon did; the
//! phone keeps a complementary Room SQLite that records `rpc_attempt`
//! before the call. The two views together cover both directions of
//! partial failure.
//!
//! No per-record fsync — the cost on rotational/SD disks is too high.
//! The audit append happens *after* the user-visible op succeeds, so a
//! crash between op and audit append loses an audit record but not the
//! data change. The phone-side log catches what we miss.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use herd_scout_ipc::AuditRecord;
use time::{Duration as TimeDuration, OffsetDateTime, Time};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Schema version for audit records. Bump when adding fields that
/// downstream readers must branch on; new optional fields can land at
/// the same version because the wire `details` is `serde_json::Value`.
pub(crate) const AUDIT_SCHEMA_VERSION: u32 = 1;

/// Active log file name within the audit dir.
const ACTIVE_LOG: &str = "audit.log";

/// Retention horizon: rotated files older than this are swept.
const RETENTION_DAYS: i64 = 90;

/// Hard cap on records returned by a single `TailAudit` call.
const MAX_TAIL_RECORDS: usize = 500;

// ── Metrics shared between SSH + admin handlers ─────────────────────────

#[derive(Debug, Default)]
pub(crate) struct ControlMetrics {
    pub active_ssh_sessions: AtomicUsize,
    pub last_reload_unix_ms: AtomicU64,
    /// Source of the last reload. Stored as a `&'static str` slot via
    /// `ArcSwap` so the admin handler can render it without
    /// stringifying numeric tags.
    pub last_reload_source: ArcSwap<&'static str>,
}

impl ControlMetrics {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            active_ssh_sessions: AtomicUsize::new(0),
            last_reload_unix_ms: AtomicU64::new(0),
            last_reload_source: ArcSwap::from_pointee("boot"),
        })
    }

    pub(crate) fn record_reload(&self, source: &'static str) {
        self.last_reload_unix_ms
            .store(now_unix_ms(), Ordering::Release);
        self.last_reload_source.store(Arc::new(source));
    }
}

// ── Audit writer ────────────────────────────────────────────────────────

/// Append-only JSONL writer. Cheap clone (`Arc` inside).
#[derive(Debug, Clone)]
pub(crate) struct Audit {
    inner: Arc<AuditInner>,
}

#[derive(Debug)]
struct AuditInner {
    dir: PathBuf,
    file: Mutex<tokio::fs::File>,
}

impl Audit {
    /// Open / create the active log file. Creates the dir as needed.
    pub(crate) async fn open(dir: PathBuf) -> Result<Self> {
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create audit dir {}", dir.display()))?;
        let file = open_active(&dir)
            .await
            .with_context(|| format!("open audit log under {}", dir.display()))?;
        Ok(Self {
            inner: Arc::new(AuditInner {
                dir,
                file: Mutex::new(file),
            }),
        })
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// Append one record. Failures log a warn but do not propagate —
    /// the audit log is best-effort and must never block user-visible
    /// ops.
    pub(crate) async fn append(&self, record: AuditRecord) {
        let mut line = match serde_json::to_string(&record) {
            Ok(s) => s,
            Err(e) => {
                warn!("audit: serialize failed: {e:#}");
                return;
            }
        };
        line.push('\n');
        let mut f = self.inner.file.lock().await;
        if let Err(e) = f.write_all(line.as_bytes()).await {
            warn!("audit: write failed: {e:#}");
        }
        // Skip per-record fsync — accepted tradeoff documented in the
        // module header.
    }

    /// Convenience: build a record stamped now and append it.
    pub(crate) async fn log(
        &self,
        kind: &str,
        actor_node_id: Option<String>,
        actor_label: Option<String>,
        details: serde_json::Value,
    ) {
        self.append(AuditRecord {
            schema_version: AUDIT_SCHEMA_VERSION,
            ts_ms: now_unix_ms(),
            kind: kind.to_string(),
            actor_node_id,
            actor_label,
            details,
        })
        .await;
    }

    /// Atomically rotate the active log if its first record is from a
    /// previous UTC day, then sweep retention. No-op on parse errors
    /// (we'd rather keep an unrotateable log than lose data).
    pub(crate) async fn rotate_if_needed(&self) {
        let active_path = self.inner.dir.join(ACTIVE_LOG);
        let today = current_utc_date_string();
        let first_date = match read_first_record_date(&active_path).await {
            Ok(Some(d)) => d,
            Ok(None) => return, // empty file or no records yet
            Err(e) => {
                warn!("audit: cannot read first record for rotation: {e:#}");
                return;
            }
        };
        if first_date == today {
            return;
        }
        // Rotate: rename the active file to `audit-<first_date>.log`,
        // then swap in a fresh handle.
        let rotated = self.inner.dir.join(format!("audit-{first_date}.log"));
        // If the target already exists (clock skew across boots),
        // append a numeric suffix to avoid clobbering.
        let rotated = ensure_unused(rotated);
        let mut f = self.inner.file.lock().await;
        if let Err(e) = f.flush().await {
            warn!("audit: pre-rotate flush failed: {e:#}");
        }
        // Drop the file handle before renaming — Windows would refuse
        // otherwise; on Unix it's harmless.
        drop(f);
        if let Err(e) = tokio::fs::rename(&active_path, &rotated).await {
            warn!(
                "audit: rotate rename {} → {} failed: {e:#}",
                active_path.display(),
                rotated.display(),
            );
            // Re-open the active file and bail; we still want to
            // continue logging.
            if let Ok(reopened) = open_active(&self.inner.dir).await {
                *self.inner.file.lock().await = reopened;
            }
            return;
        }
        match open_active(&self.inner.dir).await {
            Ok(reopened) => {
                *self.inner.file.lock().await = reopened;
                info!(
                    rotated = %rotated.display(),
                    "audit: rotated active log",
                );
            }
            Err(e) => warn!("audit: reopen after rotate failed: {e:#}"),
        }
        // Sweep retention.
        if let Err(e) = sweep_retention(&self.inner.dir).await {
            warn!("audit: retention sweep failed: {e:#}");
        }
    }

    /// Read up to `last_n` records (capped at [`MAX_TAIL_RECORDS`])
    /// strictly older than `before_ts_ms` (if provided), in
    /// newest-first order. Walks the active file then rotated files
    /// in reverse-chronological-name order until the cap is hit.
    /// Sets `eof=true` when no older records exist for this filter.
    pub(crate) async fn tail(
        &self,
        last_n: u32,
        before_ts_ms: Option<u64>,
    ) -> (Vec<AuditRecord>, bool) {
        let cap = (last_n as usize).min(MAX_TAIL_RECORDS);
        if cap == 0 {
            return (Vec::new(), false);
        }
        let mut out: Vec<AuditRecord> = Vec::with_capacity(cap);

        // Active file first.
        let active = self.inner.dir.join(ACTIVE_LOG);
        let _ = collect_from_file(&active, &mut out, cap, before_ts_ms).await;
        if out.len() >= cap {
            return (out, false);
        }

        // Rotated files in reverse-chronological order.
        let mut rotated = list_rotated(&self.inner.dir).await;
        rotated.sort_by(|a, b| b.cmp(a)); // newest first by filename
        let mut more = false;
        for path in rotated {
            if out.len() >= cap {
                more = true;
                break;
            }
            let _ = collect_from_file(&path, &mut out, cap, before_ts_ms).await;
        }
        let eof = !more && out.len() < cap;
        (out, eof)
    }
}

// ── Background tasks ────────────────────────────────────────────────────

/// Spawn a task that calls `rotate_if_needed` shortly after each UTC
/// midnight (with a small jitter) plus every 6 h as a safety net.
pub(crate) fn spawn_rotation_task(audit: Audit) {
    tokio::spawn(async move {
        loop {
            let sleep = next_rotation_sleep();
            tokio::time::sleep(sleep).await;
            audit.rotate_if_needed().await;
        }
    });
}

fn next_rotation_sleep() -> std::time::Duration {
    let now = OffsetDateTime::now_utc();
    let next_midnight = (now + TimeDuration::days(1))
        .replace_time(Time::MIDNIGHT)
        + TimeDuration::seconds(30); // small jitter past midnight
    let dur = next_midnight - now;
    let dur_secs = dur.whole_seconds().max(60).min(6 * 3600);
    std::time::Duration::from_secs(dur_secs as u64)
}

// ── helpers ─────────────────────────────────────────────────────────────

async fn open_active(dir: &Path) -> Result<tokio::fs::File> {
    let path = dir.join(ACTIVE_LOG);
    open_with_mode_0600(&path).await
}

#[cfg(unix)]
async fn open_with_mode_0600(path: &Path) -> Result<tokio::fs::File> {
    let f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    Ok(f)
}

#[cfg(not(unix))]
async fn open_with_mode_0600(path: &Path) -> Result<tokio::fs::File> {
    let f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    Ok(f)
}

async fn read_first_record_date(path: &Path) -> Result<Option<String>> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let f = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("open {}", path.display())),
    };
    let mut lines = BufReader::new(f).lines();
    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<AuditRecord>(&line) {
            let date = unix_ms_to_utc_date_string(rec.ts_ms);
            return Ok(Some(date));
        }
    }
    Ok(None)
}

async fn list_rotated(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("audit-") && name_str.ends_with(".log") {
            out.push(entry.path());
        }
    }
    out
}

fn ensure_unused(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
    let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for n in 1..1000 {
        let candidate = parent.join(format!("{}.{}.log", stem.as_deref().unwrap_or("audit"), n));
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

async fn sweep_retention(dir: &Path) -> Result<()> {
    let cutoff = OffsetDateTime::now_utc() - TimeDuration::days(RETENTION_DAYS);
    let cutoff_str = cutoff
        .date()
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]"
        ))
        .unwrap_or_default();
    let mut removed = 0;
    let mut rd = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // audit-YYYY-MM-DD.log → compare YYYY-MM-DD lexically.
        let date_part = name_str
            .strip_prefix("audit-")
            .and_then(|s| s.strip_suffix(".log"));
        if let Some(date) = date_part {
            // Only sweep "pure" dated files; defensively skip
            // disambiguated rotations (e.g. "2026-05-27.1").
            if date.len() == 10 && date < cutoff_str.as_str() {
                if let Err(e) = tokio::fs::remove_file(entry.path()).await {
                    warn!(
                        "audit: failed to remove old log {}: {e:#}",
                        entry.path().display(),
                    );
                } else {
                    removed += 1;
                }
            }
        }
    }
    if removed > 0 {
        info!(removed, "audit: retention sweep removed old logs");
    }
    Ok(())
}

async fn collect_from_file(
    path: &Path,
    out: &mut Vec<AuditRecord>,
    cap: usize,
    before_ts_ms: Option<u64>,
) -> Result<()> {
    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let text = String::from_utf8_lossy(&bytes);
    // Newest-first within a single file: parse all, reverse, filter.
    let mut parsed: Vec<AuditRecord> = text
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str::<AuditRecord>(l).ok())
        .collect();
    parsed.reverse();
    for rec in parsed {
        if let Some(before) = before_ts_ms {
            if rec.ts_ms >= before {
                continue;
            }
        }
        out.push(rec);
        if out.len() >= cap {
            break;
        }
    }
    Ok(())
}

pub(crate) fn now_unix_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn current_utc_date_string() -> String {
    OffsetDateTime::now_utc()
        .date()
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]"
        ))
        .unwrap_or_default()
}

fn unix_ms_to_utc_date_string(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .map(|d| {
            d.date()
                .format(&time::macros::format_description!(
                    "[year]-[month]-[day]"
                ))
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

/// Compute the daemon's data dir for audit storage. Mirrors the path
/// scheme the daemon already uses for prefs (`Store::open`).
pub(crate) fn audit_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("net", "herd-scout", "herd-scout")
        .context("no user-data directory available on this platform")?;
    Ok(dirs.data_dir().to_path_buf())
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn rec(ts_ms: u64, kind: &str) -> AuditRecord {
        AuditRecord {
            schema_version: AUDIT_SCHEMA_VERSION,
            ts_ms,
            kind: kind.to_string(),
            actor_node_id: None,
            actor_label: None,
            details: json!({}),
        }
    }

    #[tokio::test]
    async fn append_then_tail_returns_newest_first() {
        let tmp = TempDir::new().unwrap();
        let audit = Audit::open(tmp.path().to_path_buf()).await.unwrap();
        audit.append(rec(100, "a")).await;
        audit.append(rec(200, "b")).await;
        audit.append(rec(300, "c")).await;
        let (records, eof) = audit.tail(10, None).await;
        assert!(eof);
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].kind, "c");
        assert_eq!(records[1].kind, "b");
        assert_eq!(records[2].kind, "a");
    }

    #[tokio::test]
    async fn tail_paginates_via_before_ts_ms() {
        let tmp = TempDir::new().unwrap();
        let audit = Audit::open(tmp.path().to_path_buf()).await.unwrap();
        for i in 0..10u64 {
            audit.append(rec(100 + i, &format!("k{i}"))).await;
        }
        let (page1, eof1) = audit.tail(3, None).await;
        assert!(!eof1);
        assert_eq!(page1.len(), 3);
        assert_eq!(page1[0].ts_ms, 109);
        let oldest = page1.last().unwrap().ts_ms;
        let (page2, _eof2) = audit.tail(3, Some(oldest)).await;
        assert_eq!(page2.len(), 3);
        assert!(page2[0].ts_ms < oldest);
    }

    #[tokio::test]
    async fn tail_caps_at_500() {
        let tmp = TempDir::new().unwrap();
        let audit = Audit::open(tmp.path().to_path_buf()).await.unwrap();
        for i in 0..600u64 {
            audit.append(rec(i, "x")).await;
        }
        let (records, eof) = audit.tail(1000, None).await;
        assert_eq!(records.len(), 500);
        assert!(!eof);
    }

    #[tokio::test]
    async fn skips_non_record_lines() {
        let tmp = TempDir::new().unwrap();
        // Pre-seed a corrupt line directly on disk, then open the
        // writer over it.
        let path = tmp.path().join(ACTIVE_LOG);
        tokio::fs::write(&path, "not-json\n").await.unwrap();
        let audit = Audit::open(tmp.path().to_path_buf()).await.unwrap();
        audit.append(rec(42, "valid")).await;
        let (records, _) = audit.tail(10, None).await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, "valid");
    }

    #[tokio::test]
    async fn tails_across_rotated_file() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();
        // Pre-seed a rotated file
        tokio::fs::write(
            dir.join("audit-2020-01-01.log"),
            format!("{}\n", serde_json::to_string(&rec(50, "old")).unwrap()),
        )
        .await
        .unwrap();
        let audit = Audit::open(dir.clone()).await.unwrap();
        audit.append(rec(150, "new")).await;
        let (records, eof) = audit.tail(10, None).await;
        assert!(eof);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].kind, "new");
        assert_eq!(records[1].kind, "old");
    }

    #[test]
    fn metrics_record_reload_updates_timestamp_and_source() {
        let m = ControlMetrics::new();
        m.record_reload("admin_rpc");
        assert!(m.last_reload_unix_ms.load(Ordering::Acquire) > 0);
        assert_eq!(**m.last_reload_source.load(), "admin_rpc");
    }
}
