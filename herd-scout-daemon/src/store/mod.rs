//! Local-only persistence for desktop user prefs.
//!
//! Wave 5B deliverable. Persists the last-paired [`LiveTicket`] to disk so
//! a returning user does not have to re-pair on every launch. The pairing
//! UI (Wave 5A) calls [`Store::save_ticket`] on a successful connect; the
//! `main.rs` boot path calls [`Store::load_last_ticket`] as the **last**
//! fallback after env var, CLI flag, and (Wave 5A) any pasted/scanned
//! ticket from the pairing screen.
//!
//! ## Why a local file instead of `iroh-smol-kv` today
//!
//! The repo declares `iroh-smol-kv` (n0-computer, branch `iroh-098`) as
//! the eventual KV CRDT backing store — see the
//! [[iroh-docs-fms-schema]] design article. Inspection of the live
//! `iroh-098` source on 2026-05-21 shows that crate currently exposes
//! **only** an in-memory, gossip-coupled `Client` (`Client::local(topic,
//! config)`) with a 24-hour default TTL. There is no persistent backend
//! and instantiating a `Client` requires a full iroh `Endpoint` plus
//! `iroh-gossip` plus a `Router` — heavy machinery for what is, at MVP,
//! a single-key local pref. Until upstream lands a persistent backend
//! (or a durable-prefs fork is selected), this module persists prefs as
//! a tiny on-disk JSON sidecar whose schema mirrors smol-kv's
//! `(scope, key, value, timestamp)` shape so the migration is
//! mechanical: read sidecar → replay each entry through
//! `WriteScope::put(key, value)` (which stamps its own HLC `timestamp`
//! at write time).
//!
//! `iroh-smol-kv` is still declared as a dep so the dep graph is stable
//! across the future pivot and so the schema-aligned `current_timestamp`
//! helper can be reused (HLC-equivalent: nanos since UNIX epoch).
//!
//! ## On-disk layout
//!
//! Under the OS-specific user-data directory derived from
//! `directories::ProjectDirs::from("net", "herd-scout", "herd-scout")`:
//!
//! - `device_author.bin` — 32 raw bytes; per-device ed25519 secret-key
//!   material. Generated on first launch, persisted, reused. This is
//!   the per-device author key from the design doc's "author key
//!   strategy" section. (For now we only use its public half as the
//!   `scope` field in the sidecar; signing kicks in once we move to
//!   real iroh-smol-kv.)
//! - `prefs.json` — JSON map: `key (string) -> { value (base64), ts_ns
//!   (u64), author (hex) }`. LWW per `(ts_ns, author)` per the
//!   design doc.
//!
//! ## Schema (mirrors `iroh-docs-fms-schema`)
//!
//! Namespace: a **single deterministic personal-prefs namespace** —
//! "one personal namespace per user/device" per the design doc. Because
//! we are local-only the namespace is implicit (one file = one
//! namespace), but the file header records `_namespace` for
//! forward-compat.
//!
//! Keys:
//!
//! ```text
//! prefs/_schema       → "1"
//! prefs/last_ticket   → ticket URI bytes (iroh-live: …)
//! prefs/last_ticket_at→ ISO 8601 UTC timestamp (string)
//! ```
//!
//! ## Failure modes
//!
//! - First launch (no files): [`Store::load_last_ticket`] returns
//!   `Ok(None)`.
//! - Unreadable / corrupt sidecar: logs `warn!` and returns `Ok(None)`.
//!   Never panics. The pairing-screen code path (Wave 5A) still works.
//! - Two desktop instances open simultaneously: each instance reads on
//!   open and writes on save; the on-disk file is rewritten atomically
//!   (write-temp-then-rename). Last writer wins; in practice users do
//!   not run two desktops at once. This matches the LWW conflict
//!   strategy spelled out in the design doc.
//!
//! ## Dead-code allow
//!
//! [`Store::save_ticket`] is the public hand-off point for Wave 5A's
//! pairing-screen on-connect handler. The `main.rs` fallback path only
//! *reads* — saves come from the UI layer in a follow-up wave. Until
//! 5A's pairing module merges, the writer-side helpers (and the
//! base64 / time helpers they depend on) appear unused at the binary
//! level. They are intentional public API; allow the warnings.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow};
use iroh_live::ticket::LiveTicket;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::{debug, info, warn};

/// Application identifier used for the user-data directory.
///
/// `("net", "herd-scout", "herd-scout")` resolves to:
/// - macOS:   `~/Library/Application Support/net.herd-scout.herd-scout/`
/// - Linux:   `~/.local/share/herd-scout/`
/// - Windows: `%APPDATA%\herd-scout\herd-scout\data\`
const APP_QUALIFIER: &str = "net";
const APP_ORG: &str = "herd-scout";
const APP_NAME: &str = "herd-scout";

const DEVICE_AUTHOR_FILE: &str = "device_author.bin";
const PREFS_FILE: &str = "prefs.json";
const SCHEMA_VERSION: &str = "1";
const NAMESPACE_TAG: &str = "herd-scout-personal-prefs-v1";

const KEY_SCHEMA: &str = "prefs/_schema";
const KEY_LAST_TICKET: &str = "prefs/last_ticket";
const KEY_LAST_TICKET_AT: &str = "prefs/last_ticket_at";

/// Local prefs store.
///
/// Cheap to clone; all state is immutable after [`Store::open`].
#[derive(Debug, Clone)]
pub struct Store {
    data_dir: PathBuf,
    /// Hex-encoded public half of the per-device author key.
    /// Used as the LWW `author` tiebreaker in the sidecar.
    author_pub_hex: String,
}

impl Store {
    /// Opens (or creates) the local prefs store.
    ///
    /// On first launch this creates the data directory and a new
    /// per-device author key. Subsequent launches reuse them.
    ///
    /// Returns `Err` only when the OS does not expose a user-data
    /// directory at all (very rare). All other failure paths log a
    /// warning and degrade gracefully via [`Store::load_last_ticket`]
    /// returning `Ok(None)`.
    pub async fn open() -> Result<Self> {
        let dirs = directories::ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
            .ok_or_else(|| anyhow!("no user-data directory available on this platform"))?;
        let data_dir = dirs.data_dir().to_path_buf();
        fs::create_dir_all(&data_dir)
            .await
            .with_context(|| format!("creating data dir {}", data_dir.display()))?;

        let author_pub_hex = load_or_create_author(&data_dir).await?;
        debug!(
            data_dir = %data_dir.display(),
            author = %short_hex(&author_pub_hex),
            "store opened",
        );
        Ok(Self {
            data_dir,
            author_pub_hex,
        })
    }

    /// Returns the data directory for this store. Useful for logs and
    /// for tests that need to inspect on-disk state.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Loads the most-recent saved ticket, if any.
    ///
    /// Never errors on a missing or corrupt sidecar — both yield
    /// `Ok(None)` so the caller can fall through to the pairing UI.
    pub async fn load_last_ticket(&self) -> Result<Option<LiveTicket>> {
        let prefs = match read_prefs(&self.data_dir).await {
            Ok(Some(p)) => p,
            Ok(None) => {
                debug!("no prefs.json present yet (first launch)");
                return Ok(None);
            }
            Err(e) => {
                warn!("failed to read prefs.json — ignoring: {e:#}");
                return Ok(None);
            }
        };
        let Some(entry) = prefs.entries.get(KEY_LAST_TICKET) else {
            return Ok(None);
        };
        let raw = match entry.decode_value_utf8() {
            Ok(s) => s,
            Err(e) => {
                warn!("last_ticket entry not utf-8 — ignoring: {e:#}");
                return Ok(None);
            }
        };
        match raw.parse::<LiveTicket>() {
            Ok(t) => {
                info!(broadcast = %t.broadcast_name, "loaded last-paired ticket from store");
                Ok(Some(t))
            }
            Err(e) => {
                warn!("last_ticket entry failed to parse — ignoring: {e:#}");
                Ok(None)
            }
        }
    }

    /// Saves a ticket as the most-recently-paired one.
    ///
    /// Writes are LWW per `(timestamp_ns, author_pub)` — newer
    /// timestamps win, ties broken by author. This matches the design
    /// doc's LWW strategy for benign last-write fields.
    pub async fn save_ticket(&self, ticket: &LiveTicket) -> Result<()> {
        let mut prefs = read_prefs(&self.data_dir).await.unwrap_or_default().unwrap_or_default();
        prefs.namespace = NAMESPACE_TAG.to_string();

        let now_ns = current_timestamp_ns();
        let now_iso = iso8601_utc(now_ns);

        prefs.put(
            KEY_SCHEMA,
            SCHEMA_VERSION.as_bytes(),
            now_ns,
            &self.author_pub_hex,
        );
        prefs.put(
            KEY_LAST_TICKET,
            ticket.serialize().as_bytes(),
            now_ns,
            &self.author_pub_hex,
        );
        prefs.put(
            KEY_LAST_TICKET_AT,
            now_iso.as_bytes(),
            now_ns,
            &self.author_pub_hex,
        );

        write_prefs_atomic(&self.data_dir, &prefs).await?;
        info!(broadcast = %ticket.broadcast_name, "saved last-paired ticket to store");
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Sidecar format
// ---------------------------------------------------------------------

/// On-disk JSON shape. Mirrors smol-kv's `(scope, key, value, timestamp)`.
///
/// `entries[k] = Entry { value, ts_ns, author }` is one
/// `(scope=author, key=k, signed_value)` row in CRDT terms. LWW reads
/// pick the entry with the highest `(ts_ns, author)` tuple. Today the
/// file holds only one author (this device); the structure is built
/// to absorb additional authors when prefs sync becomes a feature.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Prefs {
    /// Schema version of the file format itself (NOT the entity schema
    /// — that lives at `prefs/_schema`).
    #[serde(default = "default_file_version")]
    file_version: u32,
    /// The personal-prefs namespace tag. Recorded for forward-compat
    /// when this moves to real iroh-smol-kv where namespace identity
    /// is a `TopicId`.
    #[serde(default)]
    namespace: String,
    /// Map of `key -> (value, ts_ns, author)`. Last-Writer-Wins.
    #[serde(default)]
    entries: std::collections::BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    /// Base64-url-no-pad encoded value bytes. Base64 because JSON does
    /// not natively carry binary; values are typically UTF-8 today but
    /// tickets may carry non-printable bytes if the scheme ever shifts.
    value_b64: String,
    /// HLC-equivalent timestamp: nanos since UNIX epoch.
    ts_ns: u64,
    /// Hex-encoded public half of the author's ed25519 key.
    author: String,
}

impl Entry {
    fn decode_value_utf8(&self) -> Result<String> {
        let bytes = b64_decode(&self.value_b64).context("decoding entry value (base64)")?;
        String::from_utf8(bytes).context("entry value not valid UTF-8")
    }
}

impl Prefs {
    /// Inserts (or LWW-overwrites) `key`. Strictly newer
    /// `(ts_ns, author)` tuples win.
    fn put(&mut self, key: &str, value: &[u8], ts_ns: u64, author_hex: &str) {
        let new_entry = Entry {
            value_b64: b64_encode(value),
            ts_ns,
            author: author_hex.to_string(),
        };
        if let Some(existing) = self.entries.get(key) {
            if (existing.ts_ns, existing.author.as_str())
                >= (new_entry.ts_ns, new_entry.author.as_str())
            {
                return;
            }
        }
        self.entries.insert(key.to_string(), new_entry);
    }
}

fn default_file_version() -> u32 {
    1
}

async fn read_prefs(data_dir: &Path) -> Result<Option<Prefs>> {
    let path = data_dir.join(PREFS_FILE);
    let bytes = match fs::read(&path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let prefs: Prefs = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing JSON in {}", path.display()))?;
    Ok(Some(prefs))
}

async fn write_prefs_atomic(data_dir: &Path, prefs: &Prefs) -> Result<()> {
    let final_path = data_dir.join(PREFS_FILE);
    let tmp_path = data_dir.join(format!("{PREFS_FILE}.tmp"));
    let bytes = serde_json::to_vec_pretty(prefs).context("serialising prefs to JSON")?;
    fs::write(&tmp_path, &bytes)
        .await
        .with_context(|| format!("writing {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &final_path).await.with_context(|| {
        format!(
            "renaming {} -> {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    Ok(())
}

// ---------------------------------------------------------------------
// Author key persistence
// ---------------------------------------------------------------------

/// Loads the per-device author secret if present, otherwise generates a
/// fresh 32-byte secret and persists it. Returns the **public** half as
/// hex; the secret stays on disk for future smol-kv signing.
async fn load_or_create_author(data_dir: &Path) -> Result<String> {
    let path = data_dir.join(DEVICE_AUTHOR_FILE);
    let secret_bytes = match fs::read(&path).await {
        Ok(b) if b.len() == 32 => {
            let mut buf = [0u8; 32];
            buf.copy_from_slice(&b);
            buf
        }
        Ok(_) => {
            warn!(
                "device_author.bin has unexpected size — regenerating; downstream LWW author will rotate",
            );
            generate_and_store_author(&path).await?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            generate_and_store_author(&path).await?
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    // The "public" representation we use today is just a hash of the
    // secret — sufficient for LWW tiebreaking. When this module moves
    // to real iroh-smol-kv the secret feeds an `iroh::SecretKey` and
    // we use `.public()` proper.
    Ok(public_tag_hex(&secret_bytes))
}

async fn generate_and_store_author(path: &Path) -> Result<[u8; 32]> {
    use rand::TryRngCore;
    let mut buf = [0u8; 32];
    rand::rng()
        .try_fill_bytes(&mut buf)
        .map_err(|e| anyhow!("rng: {e}"))?;
    fs::write(path, &buf)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    info!(path = %path.display(), "generated new device author key");
    Ok(buf)
}

/// Derives a stable public tag from the 32-byte secret material.
///
/// Today this is just a labelled hash; once the store moves to real
/// iroh-smol-kv, the same 32 bytes feed `iroh::SecretKey::from_bytes`
/// and we use the proper ed25519 public key. The migration is a
/// one-line swap because the on-disk format already records this as
/// an opaque hex string.
fn public_tag_hex(secret: &[u8; 32]) -> String {
    // Mix in a domain tag so this can't collide with any other
    // secret-derived identifier this app ever stores.
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(secret);
    buf[32..].copy_from_slice(b"herd-scout/personal-prefs/author");
    let hash = blake3_like(&buf);
    hex_encode(&hash)
}

/// Tiny content-tag hash. Not BLAKE3 (no extra dep); a folding XOR
/// over 8-byte chunks is sufficient for an author tiebreaker because
/// it only has to be stable + unique-per-device locally. Replaced
/// when we adopt real ed25519 signing.
fn blake3_like(input: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in input.iter().enumerate() {
        out[i % 32] ^= b.rotate_left((i as u32) % 8);
    }
    // One pass of state-mixing so adjacent bytes can't reach the
    // same output cell with the same value.
    for i in 1..out.len() {
        out[i] = out[i].wrapping_add(out[i - 1].rotate_left(3));
    }
    out
}

// ---------------------------------------------------------------------
// Encoding helpers (kept inline; no extra deps)
// ---------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(hex_nibble(b >> 4));
        out.push(hex_nibble(b & 0x0f));
    }
    out
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => unreachable!("nibble"),
    }
}

fn short_hex(s: &str) -> &str {
    &s[..s.len().min(8)]
}

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n =
            ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
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

fn b64_decode(input: &str) -> Result<Vec<u8>> {
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
            _ => return Err(anyhow!("invalid base64url byte: {:#x}", b)),
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

// ---------------------------------------------------------------------
// Time helpers
// ---------------------------------------------------------------------

/// Nanos since UNIX epoch. Matches `iroh_smol_kv::util::current_timestamp`
/// so the on-disk values are bit-comparable when (eventually) replayed
/// into the real CRDT.
fn current_timestamp_ns() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_nanos()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// Format a nanos-since-epoch timestamp as ISO-8601 UTC, without
/// pulling in `chrono`. Truncates to second precision (sufficient for
/// human-readable display; the LWW comparator uses `ts_ns` directly).
fn iso8601_utc(ts_ns: u64) -> String {
    let secs = ts_ns / 1_000_000_000;
    // Days since 1970-01-01.
    let mut days = (secs / 86_400) as i64;
    let mut secs_today = secs % 86_400;
    let hour = secs_today / 3600;
    secs_today %= 3600;
    let minute = secs_today / 60;
    let second = secs_today % 60;

    // Convert `days` to (year, month, day) using a civil-from-days
    // algorithm (Howard Hinnant). Public-domain; pasted in to avoid a
    // chrono dependency for one timestamp.
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = (days - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hour, minute, second
    )
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use iroh::{EndpointAddr, SecretKey};

    use super::*;

    fn fixture_ticket(name: &str) -> LiveTicket {
        let key = SecretKey::generate();
        LiveTicket::new(EndpointAddr::from(key.public()), name)
    }

    /// Bypass `Store::open`'s use of `directories::ProjectDirs` so tests
    /// don't pollute the real user-data dir.
    async fn open_in(dir: &Path) -> Store {
        fs::create_dir_all(dir).await.unwrap();
        let author_pub_hex = load_or_create_author(dir).await.unwrap();
        Store {
            data_dir: dir.to_path_buf(),
            author_pub_hex,
        }
    }

    #[tokio::test]
    async fn first_launch_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let store = open_in(tmp.path()).await;
        assert!(store.load_last_ticket().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn save_then_load_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let store = open_in(tmp.path()).await;
        let ticket = fixture_ticket("test-stream");
        store.save_ticket(&ticket).await.unwrap();

        // Re-open over the same directory and confirm the ticket
        // survives the "process restart".
        let store2 = open_in(tmp.path()).await;
        let loaded = store2
            .load_last_ticket()
            .await
            .unwrap()
            .expect("ticket present after reopen");
        assert_eq!(loaded.broadcast_name, ticket.broadcast_name);
        assert_eq!(loaded.endpoint.id, ticket.endpoint.id);
    }

    #[tokio::test]
    async fn corrupt_prefs_returns_none_not_err() {
        let tmp = tempfile::tempdir().unwrap();
        let store = open_in(tmp.path()).await;
        // Write garbage over the prefs file.
        fs::write(tmp.path().join(PREFS_FILE), b"not json")
            .await
            .unwrap();
        // Should NOT panic, NOT err — just no ticket.
        assert!(store.load_last_ticket().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn lww_keeps_newer_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = open_in(tmp.path()).await;
        let first = fixture_ticket("first");
        let second = fixture_ticket("second");
        store.save_ticket(&first).await.unwrap();
        // Tiny sleep to guarantee a newer ts_ns.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        store.save_ticket(&second).await.unwrap();
        let loaded = store.load_last_ticket().await.unwrap().unwrap();
        assert_eq!(loaded.broadcast_name, "second");
    }

    #[test]
    fn b64_roundtrip() {
        for raw in [
            b"".as_slice(),
            b"a",
            b"abc",
            b"\x00\xff\x10\x42",
            b"the quick brown fox",
        ] {
            let encoded = b64_encode(raw);
            let decoded = b64_decode(&encoded).unwrap();
            assert_eq!(decoded, raw);
        }
    }

    #[test]
    fn ticket_roundtrip_through_string() {
        // `LiveTicket::serialize()` is what we store on disk; this
        // belt-and-braces test covers the assumption that it
        // round-trips through `FromStr` losslessly.
        let key = SecretKey::generate();
        let t = LiveTicket::new(EndpointAddr::from(key.public()), "round-trip");
        let s = t.serialize();
        let parsed = LiveTicket::from_str(&s).unwrap();
        assert_eq!(parsed.broadcast_name, t.broadcast_name);
        assert_eq!(parsed.endpoint.id, t.endpoint.id);
    }
}
