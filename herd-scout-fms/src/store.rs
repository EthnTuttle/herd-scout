//! Smol-kv-shaped on-disk record store.
//!
//! Records are appended to `<root>/records.jsonl` (one [`RecordEnvelope`]
//! per line). The in-memory index is a `BTreeMap<(scope, key), Vec<HlcEntry>>`
//! so `scan_prefix` is a range scan and per-key conflict resolution
//! lives in the model layer (LWW, add-wins-set, append-only).
//!
//! ## Concurrency
//!
//! - Writes go through a single async-aware `Mutex` to serialize file
//!   appends (atomicity per envelope is just `write_all` of the line —
//!   the OS guarantees POSIX atomic appends below `PIPE_BUF`, but our
//!   lines exceed that, so we take the lock).
//! - Reads share an `RwLock` over the in-memory index. Readers are
//!   non-blocking; writers grab a write lock at commit time.
//!
//! ## Future swap
//!
//! When durable iroh-smol-kv lands, replace the file backend with the
//! real `Client` and keep the same external API. Records are
//! shape-compatible: `(scope, key, value, ts_ns)` with the HLC inside
//! `value`.

use std::collections::BTreeMap;
use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::{debug, warn};

use crate::hlc::{Hlc, HlcGenerator};
use crate::model::{ChangeEvent, ChangeStrategy, RecordEnvelope};

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub root: PathBuf,
    pub event_buffer: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStrategy {
    LastWriteWins,
    AddWinsSet,
    AppendOnly,
}

impl ReadStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LastWriteWins => "lww",
            Self::AddWinsSet => "add_wins_set",
            Self::AppendOnly => "append_only",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "lww" => Some(Self::LastWriteWins),
            "add_wins_set" => Some(Self::AddWinsSet),
            "append_only" => Some(Self::AppendOnly),
            _ => None,
        }
    }

    fn as_change(self) -> ChangeStrategy {
        match self {
            Self::LastWriteWins => ChangeStrategy::LastWriteWins,
            Self::AddWinsSet => ChangeStrategy::AddWinsSet,
            Self::AppendOnly => ChangeStrategy::AppendOnly,
        }
    }
}

/// Per-`(scope, key)` storage entry.
///
/// `ts_ns` and `strategy` are kept on every entry so the in-memory
/// index round-trips with the on-disk envelope when we eventually
/// snapshot it; they aren't read in v1 because materialization works
/// off `(scope, key, hlc, value)` alone.
#[derive(Debug, Clone)]
#[allow(dead_code, reason = "kept for snapshot parity with on-disk envelope")]
struct IndexEntry {
    hlc: Hlc,
    value: Vec<u8>,
    ts_ns: u64,
    strategy: ReadStrategy,
}

#[derive(Debug, Default)]
struct Index {
    /// `(scope, key) → list of entries`. For LWW, the latest wins
    /// at materialization time; for AppendOnly we keep all entries.
    rows: BTreeMap<(String, Vec<u8>), Vec<IndexEntry>>,
}

#[derive(Debug, Clone)]
pub struct Store {
    inner: Arc<StoreInner>,
}

#[derive(Debug)]
struct StoreInner {
    root: PathBuf,
    /// Write side: holds the open log file plus a mutex for append
    /// serialization.
    writer: Mutex<Writer>,
    index: RwLock<Index>,
    hlc: HlcGenerator,
    events: broadcast::Sender<ChangeEvent>,
}

#[derive(Debug)]
struct Writer {
    path: PathBuf,
    file: tokio::fs::File,
}

impl Store {
    pub async fn open(cfg: StoreConfig) -> Result<Self> {
        fs::create_dir_all(&cfg.root)
            .await
            .with_context(|| format!("creating FMS root {}", cfg.root.display()))?;

        let log_path = cfg.root.join("records.jsonl");
        let mut index = Index::default();
        let mut max_hlc = Hlc::new(0, 0);

        // Replay the on-disk log into the in-memory index. Records are
        // applied in file-order; later records win for LWW (the file
        // order is also append-order, so this is correct).
        if log_path.exists() {
            let f = fs::File::open(&log_path)
                .await
                .with_context(|| format!("opening {}", log_path.display()))?;
            let mut reader = BufReader::new(f);
            let mut line = String::new();
            let mut count: u64 = 0;
            loop {
                line.clear();
                let n = reader
                    .read_line(&mut line)
                    .await
                    .with_context(|| format!("reading {}", log_path.display()))?;
                if n == 0 {
                    break;
                }
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<RecordEnvelope>(trimmed) {
                    Ok(env) => {
                        let key = match decode_b64(&env.key_b64) {
                            Ok(k) => k,
                            Err(e) => {
                                warn!("skipping record with bad key_b64: {e:#}");
                                continue;
                            }
                        };
                        let value = match decode_b64(&env.value_b64) {
                            Ok(v) => v,
                            Err(e) => {
                                warn!("skipping record with bad value_b64: {e:#}");
                                continue;
                            }
                        };
                        let strategy = ReadStrategy::parse(&env.strategy)
                            .unwrap_or(ReadStrategy::LastWriteWins);
                        if env.hlc > max_hlc {
                            max_hlc = env.hlc;
                        }
                        index.rows
                            .entry((env.scope, key))
                            .or_default()
                            .push(IndexEntry {
                                hlc: env.hlc,
                                value,
                                ts_ns: env.ts_ns,
                                strategy,
                            });
                        count += 1;
                    }
                    Err(e) => warn!("skipping malformed record: {e:#}"),
                }
            }
            debug!(replayed = count, "FMS records.jsonl replayed");
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .with_context(|| format!("opening append handle on {}", log_path.display()))?;

        let (tx, _rx) = broadcast::channel(cfg.event_buffer.max(16));

        Ok(Self {
            inner: Arc::new(StoreInner {
                root: cfg.root,
                writer: Mutex::new(Writer {
                    path: log_path,
                    file,
                }),
                index: RwLock::new(index),
                hlc: HlcGenerator::new(max_hlc),
                events: tx,
            }),
        })
    }

    pub fn root(&self) -> &std::path::Path {
        &self.inner.root
    }

    /// Subscribes to the change stream. Lagging subscribers see a
    /// `RecvError::Lagged(n)` and re-read from the index on next
    /// `recv()`.
    pub fn subscribe(&self) -> broadcast::Sender<ChangeEvent> {
        self.inner.events.clone()
    }

    /// Advances the HLC by one tick (max(local, wallclock) → tick).
    pub async fn advance_hlc(&self) -> Hlc {
        self.inner.hlc.advance()
    }

    /// Begins a write transaction. Pending puts are buffered; commit
    /// appends every record to the log under one mutex acquisition.
    pub async fn begin_transaction(&self, scope: String) -> Transaction<'_> {
        Transaction {
            store: self,
            scope,
            pending: Vec::new(),
        }
    }

    /// Returns every record under a key prefix. Output shape matches
    /// the materialization helpers in [`crate::model`]: each tuple is
    /// `(scope, key, hlc, value)`. Append-only keys produce one tuple
    /// per write; LWW/AddWinsSet keys produce one tuple per write
    /// (caller picks the winner via `pick_lww` or set semantics).
    pub async fn scan_prefix(
        &self,
        prefix: &[u8],
    ) -> Result<Vec<(String, Vec<u8>, Hlc, Vec<u8>)>> {
        let index = self.inner.index.read().await;
        let mut out = Vec::new();
        for ((scope, key), entries) in index.rows.iter() {
            if key.starts_with(prefix) {
                for e in entries {
                    out.push((scope.clone(), key.clone(), e.hlc, e.value.clone()));
                }
            }
        }
        Ok(out)
    }

    /// Returns the count of records on disk. Useful for tests that
    /// need to confirm append behavior.
    #[doc(hidden)]
    pub async fn record_count_on_disk(&self) -> Result<usize> {
        let writer = self.inner.writer.lock().await;
        let mut f = fs::File::open(&writer.path)
            .await
            .with_context(|| format!("opening {}", writer.path.display()))?;
        f.seek(SeekFrom::Start(0)).await.ok();
        let mut reader = BufReader::new(f);
        let mut line = String::new();
        let mut count = 0;
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            if !line.trim().is_empty() {
                count += 1;
            }
        }
        Ok(count)
    }
}

/// Buffered write transaction. All puts inside a transaction commit
/// atomically with respect to readers (one `RwLock` write acquisition).
/// Inter-line atomicity on the log file is handled by the `Mutex`
/// guarding the `Writer`.
#[derive(Debug)]
pub struct Transaction<'a> {
    store: &'a Store,
    scope: String,
    pending: Vec<PendingPut>,
}

#[derive(Debug)]
struct PendingPut {
    key: Vec<u8>,
    value: Vec<u8>,
    hlc: Hlc,
    strategy: ReadStrategy,
}

impl<'a> Transaction<'a> {
    pub fn put(
        &mut self,
        key: impl Into<Vec<u8>>,
        value: impl Into<Vec<u8>>,
        hlc: Hlc,
        strategy: ReadStrategy,
    ) {
        self.pending.push(PendingPut {
            key: key.into(),
            value: value.into(),
            hlc,
            strategy,
        });
    }

    pub async fn commit(self) -> Result<()> {
        let now_ns = self
            .pending
            .first()
            .map(|p| p.hlc.ts_ns)
            .unwrap_or(0);

        // Serialize all envelopes first so we can write them under a
        // single lock acquisition.
        let mut wire = String::new();
        let mut envelopes = Vec::with_capacity(self.pending.len());
        for p in &self.pending {
            let env = RecordEnvelope {
                scope: self.scope.clone(),
                key_b64: encode_b64(&p.key),
                value_b64: encode_b64(&p.value),
                ts_ns: now_ns,
                hlc: p.hlc,
                strategy: p.strategy.as_str().to_string(),
            };
            let line = serde_json::to_string(&env).context("serializing record envelope")?;
            wire.push_str(&line);
            wire.push('\n');
            envelopes.push(env);
        }

        // 1) Append to disk under the writer mutex. fsync is deferred
        //    to OS scheduler — the daemon's audit log is the system
        //    of record for ordering, FMS only needs durable-on-shutdown
        //    semantics for the MVP.
        {
            let mut writer = self.store.inner.writer.lock().await;
            writer
                .file
                .write_all(wire.as_bytes())
                .await
                .context("appending to records.jsonl")?;
            writer.file.flush().await.ok();
        }

        // 2) Update the in-memory index under the write lock.
        {
            let mut idx = self.store.inner.index.write().await;
            for p in &self.pending {
                idx.rows
                    .entry((self.scope.clone(), p.key.clone()))
                    .or_default()
                    .push(IndexEntry {
                        hlc: p.hlc,
                        value: p.value.clone(),
                        ts_ns: now_ns,
                        strategy: p.strategy,
                    });
            }
        }

        // 3) Broadcast change events. Failed sends mean no one is
        //    subscribed — fine, drop them silently.
        for p in self.pending {
            let _ = self.store.inner.events.send(ChangeEvent {
                scope: self.scope.clone(),
                key: p.key,
                value: p.value,
                hlc: p.hlc,
                strategy: p.strategy.as_change(),
            });
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------
// base64url-no-pad helpers (no extra deps; mirror the prefs sidecar)
// ---------------------------------------------------------------------

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn encode_b64(input: &[u8]) -> String {
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
    }
    out
}

#[derive(Debug, thiserror::Error)]
#[error("invalid base64url: {0}")]
struct B64Error(String);

fn decode_b64(input: &str) -> std::result::Result<Vec<u8>, B64Error> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4 + 2);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            b'=' => continue,
            _ => return Err(B64Error(format!("byte {:#x}", b))),
        };
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1u32 << bits) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn round_trip_one_record() {
        let dir = tempdir().unwrap();
        let store = Store::open(StoreConfig {
            root: dir.path().to_path_buf(),
            event_buffer: 16,
        })
        .await
        .unwrap();

        let hlc = store.advance_hlc().await;
        let mut tx = store.begin_transaction("scope-a".into()).await;
        tx.put(b"asset/1/name".to_vec(), b"first".to_vec(), hlc, ReadStrategy::LastWriteWins);
        tx.commit().await.unwrap();

        let recs = store.scan_prefix(b"asset/").await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].3, b"first");
    }

    #[tokio::test]
    async fn replays_after_restart() {
        let dir = tempdir().unwrap();
        let cfg = StoreConfig {
            root: dir.path().to_path_buf(),
            event_buffer: 16,
        };

        {
            let store = Store::open(cfg.clone()).await.unwrap();
            let hlc = store.advance_hlc().await;
            let mut tx = store.begin_transaction("scope-a".into()).await;
            tx.put(b"k".to_vec(), b"v".to_vec(), hlc, ReadStrategy::LastWriteWins);
            tx.commit().await.unwrap();
        }

        let store = Store::open(cfg).await.unwrap();
        let recs = store.scan_prefix(b"k").await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].3, b"v");
    }

    #[tokio::test]
    async fn b64_round_trip_binary() {
        for input in [b"".as_slice(), b"\x00\xff\x10\x42", b"hello"] {
            let encoded = encode_b64(input);
            let decoded = decode_b64(&encoded).unwrap();
            assert_eq!(decoded, input);
        }
    }
}
