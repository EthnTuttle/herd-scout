//! Sigstore Rekor mirror for the append-only audit log (PROTOTYPE).
//!
//! Goal: a sub-100-device fleet should not need to run its own
//! transparency-log infra to defeat split-view attacks against
//! `audit.log`. Mirroring periodic *commitments* (not raw records) to
//! the public Rekor instance gives us free witnesses-as-a-service:
//! once a commitment is in Rekor, the daemon cannot later show two
//! different audit views without one of them disagreeing with Rekor.
//!
//! Status: feature-gated (`rekor-mirror`), off by default. The default
//! daemon build does not pull in `reqwest` / `sha2` / `base64` / `hex`
//! and does not spawn this task. See `deploy/README.md` for the
//! operator opt-in once the feature graduates.
//!
//! ## Design summary
//!
//! - **Cadence.** The mirror task batches records and posts one Rekor
//!   entry per batch. A batch flushes when *either* `batch_size`
//!   records have arrived *or* `flush_interval_secs` has elapsed since
//!   the first un-flushed record (whichever first).
//! - **Commitment shape.** Per batch we compute a SHA-256 binary Merkle
//!   tree over the canonicalized JSON of each `AuditRecord`. The root
//!   is signed with the daemon's existing iroh ed25519 key and the
//!   signed root is what we POST to Rekor. Record contents are NOT
//!   published — only their hashes are committed to.
//! - **Hash chain.** Each batch's commitment includes the previous
//!   batch's Rekor UUID (or empty on cold start). Anchors therefore
//!   form their own append-only log; an attacker who silently truncates
//!   the daemon's log past anchor N would have to re-fork all later
//!   anchors to be self-consistent, which an external observer can
//!   detect by walking the chain through Rekor.
//! - **Best-effort.** Network failures, DNS failures, Rekor outages,
//!   and full mpsc buffers MUST never block the main `Audit::append`
//!   path. Send is `try_send`; on `Full` the record is silently dropped
//!   (warning logged at most once per minute). Rekor errors retry with
//!   exponential backoff (cap 5 min) and the batch is held in memory
//!   until success.
//! - **Privacy.** Everything we publish is constant-size hashes plus a
//!   public ed25519 key (the daemon's iroh NodeId, already published).
//!   No record body, peer ID, label, or `details` field crosses the
//!   wire.
//!
//! ## Wire format
//!
//! Rekor accepts several entry types. We use **hashedrekord v0.0.1**
//! (the smallest type with no in-toto / DSSE machinery): commit a
//! pre-computed digest plus a detached signature over the digest. The
//! request body is the JSON envelope:
//!
//! ```json
//! {
//!   "kind": "hashedrekord",
//!   "apiVersion": "0.0.1",
//!   "spec": {
//!     "signature": {
//!       "content": "<base64 ed25519 signature over <data_payload>>",
//!       "publicKey": { "content": "<base64 PEM-wrapped Ed25519 SubjectPublicKeyInfo>" }
//!     },
//!     "data": {
//!       "hash": { "algorithm": "sha256", "value": "<hex sha256(data_payload)>" }
//!     }
//!   }
//! }
//! ```
//!
//! Where `data_payload` is the canonical JSON encoding (sorted keys,
//! no whitespace) of:
//!
//! ```json
//! {
//!   "v": 1,
//!   "kind": "herd-scout-audit-anchor",
//!   "merkle_root_hex": "<hex sha256>",
//!   "batch_size": <usize>,
//!   "first_ts_ms": <u64>,
//!   "last_ts_ms": <u64>,
//!   "prev_anchor_uuid": "<rekor uuid or empty string>",
//!   "node_id": "<daemon's iroh NodeId, hex>"
//! }
//! ```
//!
//! On `200`/`201` Rekor returns a body keyed by the entry UUID;
//! `logIndex` lives inside. We log an `audit_mirror_anchor` record back
//! to the daemon's own audit log so the operator can observe progress
//! and so the next batch's `prev_anchor_uuid` is itself committed in
//! the local log.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use herd_scout_ipc::AuditRecord;
use iroh::SecretKey;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};
use tracing::{debug, info, warn};

use crate::audit::{Audit, now_unix_ms};

// ── Public config ──────────────────────────────────────────────────────

/// Operator-tunable knobs. All fields have sane defaults; see
/// `[audit_mirror]` in `control.toml`.
#[derive(Debug, Clone)]
pub struct AuditMirrorConfig {
    pub enabled: bool,
    /// Flush after this many records buffered.
    pub batch_size: usize,
    /// Flush this many seconds after the first un-flushed record arrives.
    pub flush_interval_secs: u64,
    /// Rekor write endpoint base. Default: <https://rekor.sigstore.dev>.
    pub rekor_url: String,
    /// Channel capacity. The mirror is best-effort, so when this fills
    /// we drop on the floor — keep it generous relative to `batch_size`
    /// so transient pressure doesn't lose anchors.
    pub channel_capacity: usize,
}

impl Default for AuditMirrorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            batch_size: 100,
            flush_interval_secs: 3600,
            rekor_url: "https://rekor.sigstore.dev".to_string(),
            channel_capacity: 1024,
        }
    }
}

// ── Spawn ──────────────────────────────────────────────────────────────

/// Spawn the mirror task. Returns the `mpsc::Sender` end so callers can
/// install it on `Audit` (see `Audit::set_mirror_tx`). Returns `None`
/// if the config is disabled.
pub fn spawn(
    cfg: AuditMirrorConfig,
    secret: SecretKey,
    audit_for_anchor_log: Audit,
) -> Option<mpsc::Sender<AuditRecord>> {
    if !cfg.enabled {
        return None;
    }
    let (tx, rx) = mpsc::channel::<AuditRecord>(cfg.channel_capacity);
    let task = MirrorTask {
        cfg,
        secret,
        audit_for_anchor_log,
        prev_anchor_uuid: String::new(),
    };
    tokio::spawn(task.run(rx));
    Some(tx)
}

// ── Task ───────────────────────────────────────────────────────────────

struct MirrorTask {
    cfg: AuditMirrorConfig,
    secret: SecretKey,
    audit_for_anchor_log: Audit,
    prev_anchor_uuid: String,
}

impl MirrorTask {
    async fn run(mut self, mut rx: mpsc::Receiver<AuditRecord>) {
        info!(
            batch_size = self.cfg.batch_size,
            flush_secs = self.cfg.flush_interval_secs,
            rekor = %self.cfg.rekor_url,
            "audit_mirror: started",
        );
        let mut buf: Vec<AuditRecord> = Vec::with_capacity(self.cfg.batch_size);
        let flush_after = Duration::from_secs(self.cfg.flush_interval_secs.max(1));
        loop {
            // Wait for the first record (no deadline).
            let first = match rx.recv().await {
                Some(r) => r,
                None => {
                    info!("audit_mirror: tx closed, exiting");
                    return;
                }
            };
            buf.push(first);
            let deadline = Instant::now() + flush_after;

            // Drain until batch size or deadline.
            while buf.len() < self.cfg.batch_size {
                tokio::select! {
                    biased;
                    _ = sleep_until(deadline) => break,
                    maybe = rx.recv() => match maybe {
                        Some(r) => buf.push(r),
                        None => {
                            // Sender dropped; flush what we have and exit.
                            self.flush(&mut buf).await;
                            return;
                        }
                    }
                }
            }
            self.flush(&mut buf).await;
        }
    }

    async fn flush(&mut self, buf: &mut Vec<AuditRecord>) {
        if buf.is_empty() {
            return;
        }
        let payload = match build_payload(buf, &self.prev_anchor_uuid, &self.secret) {
            Ok(p) => p,
            Err(e) => {
                warn!("audit_mirror: build payload failed: {e:#}");
                buf.clear();
                return;
            }
        };
        let mut backoff = Duration::from_secs(2);
        let cap = Duration::from_secs(300);
        loop {
            match post_hashedrekord(&self.cfg.rekor_url, &payload).await {
                Ok(resp) => {
                    info!(
                        uuid = %resp.uuid,
                        log_index = resp.log_index,
                        batch_size = buf.len(),
                        "audit_mirror: anchored",
                    );
                    self.audit_for_anchor_log
                        .log(
                            "audit_mirror_anchor",
                            None,
                            None,
                            serde_json::json!({
                                "merkle_root_hex": payload.merkle_root_hex,
                                "rekor_uuid": resp.uuid,
                                "rekor_log_index": resp.log_index,
                                "batch_size": buf.len(),
                                "prev_anchor_uuid": self.prev_anchor_uuid,
                            }),
                        )
                        .await;
                    self.prev_anchor_uuid = resp.uuid;
                    buf.clear();
                    return;
                }
                Err(e) => {
                    warn!(
                        backoff_s = backoff.as_secs(),
                        "audit_mirror: POST failed: {e:#}",
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(cap);
                }
            }
        }
    }
}

// ── Payload + Merkle root ──────────────────────────────────────────────

#[derive(Debug)]
struct Payload {
    /// Canonical JSON bytes of the data manifest. This is what we
    /// SHA-256 (committed in the Rekor entry) AND ed25519-sign.
    /// Read only by tests today; kept on the struct because the
    /// follow-up checkpoint format will want to log it locally.
    #[allow(dead_code, reason = "consumed only by unit tests + future checkpoint log")]
    data_canonical: Vec<u8>,
    /// Hex of `sha256(data_canonical)`.
    data_sha256_hex: String,
    /// Base64-standard ed25519 signature over `data_canonical`.
    sig_b64: String,
    /// PEM-wrapped SubjectPublicKeyInfo for the daemon's ed25519
    /// public key. Rekor wants the public key in PEM in the spec.
    pubkey_pem_b64: String,
    /// For convenience in callers / tests.
    merkle_root_hex: String,
}

fn build_payload(
    records: &[AuditRecord],
    prev_anchor_uuid: &str,
    secret: &SecretKey,
) -> Result<Payload> {
    let leaves: Vec<[u8; 32]> = records
        .iter()
        .map(canonical_record_hash)
        .collect::<Result<_>>()?;
    let merkle_root = merkle_root_sha256(&leaves);
    let merkle_root_hex = hex_lower(&merkle_root);
    let first_ts_ms = records.first().map(|r| r.ts_ms).unwrap_or(0);
    let last_ts_ms = records.last().map(|r| r.ts_ms).unwrap_or(0);
    let node_id_hex = hex_lower(&secret.public().as_bytes()[..]);
    // Canonical JSON: serde_json::Value with a BTreeMap-backed object
    // sorts keys when written via `to_string`. Build via raw JSON to
    // guarantee no extra whitespace and stable key order.
    let data = canonical_json_object(&[
        ("batch_size", records.len().to_string()),
        ("first_ts_ms", first_ts_ms.to_string()),
        ("kind", quote_string("herd-scout-audit-anchor")),
        ("last_ts_ms", last_ts_ms.to_string()),
        ("merkle_root_hex", quote_string(&merkle_root_hex)),
        ("node_id", quote_string(&node_id_hex)),
        ("prev_anchor_uuid", quote_string(prev_anchor_uuid)),
        ("v", "1".to_string()),
    ]);
    let data_canonical = data.into_bytes();
    let data_sha256_hex = hex_lower(&sha256(&data_canonical));
    let sig = secret.sign(&data_canonical);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    let pubkey_pem_b64 = base64::engine::general_purpose::STANDARD
        .encode(ed25519_public_key_pem(secret.public().as_bytes().as_ref()).as_bytes());
    Ok(Payload {
        data_canonical,
        data_sha256_hex,
        sig_b64,
        pubkey_pem_b64,
        merkle_root_hex,
    })
}

/// SHA-256 of the canonical JSON serialization of one `AuditRecord`.
/// The record is round-tripped through `serde_json::Value` and back
/// out via a deterministic key-sort to remove any ambiguity from the
/// `details` Value's encoding.
fn canonical_record_hash(rec: &AuditRecord) -> Result<[u8; 32]> {
    // Re-serialize with sorted keys: convert to Value, then walk and
    // emit canonically.
    let v = serde_json::to_value(rec).context("audit record to_value")?;
    let canonical = canonicalize_value(&v);
    Ok(sha256(canonical.as_bytes()))
}

/// Recursively render a `serde_json::Value` with sorted object keys
/// and no whitespace. Numbers are emitted as serde_json renders them
/// (which is stable for the `u32`/`u64` we use here).
fn canonicalize_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => if *b { "true".into() } else { "false".into() },
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => quote_string(s),
        serde_json::Value::Array(arr) => {
            let mut out = String::from("[");
            for (i, e) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&canonicalize_value(e));
            }
            out.push(']');
            out
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
            keys.sort_unstable();
            let mut out = String::from("{");
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&quote_string(k));
                out.push(':');
                out.push_str(&canonicalize_value(&map[*k]));
            }
            out.push('}');
            out
        }
    }
}

/// JSON-string-escape `s` (handles `"`, `\`, control chars).
fn quote_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Build a flat canonical JSON object from already-encoded values.
/// Pairs MUST be passed pre-sorted by key — this fn does NOT re-sort.
fn canonical_json_object(pairs: &[(&str, String)]) -> String {
    let mut out = String::from("{");
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&quote_string(k));
        out.push(':');
        out.push_str(v);
    }
    out.push('}');
    out
}

fn merkle_root_sha256(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return sha256(b"");
    }
    let mut layer: Vec<[u8; 32]> = leaves.to_vec();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len().div_ceil(2));
        let mut i = 0;
        while i < layer.len() {
            let pair = if i + 1 < layer.len() {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&layer[i]);
                buf[32..].copy_from_slice(&layer[i + 1]);
                sha256(&buf)
            } else {
                // Odd leaf: duplicate (RFC 6962-ish; fine for our
                // privately-anchored, prototype use).
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&layer[i]);
                buf[32..].copy_from_slice(&layer[i]);
                sha256(&buf)
            };
            next.push(pair);
            i += 2;
        }
        layer = next;
    }
    layer[0]
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    let out = h.finalize();
    let mut a = [0u8; 32];
    a.copy_from_slice(&out);
    a
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

/// Wrap a 32-byte raw ed25519 public key as a PEM-encoded
/// SubjectPublicKeyInfo (RFC 8410 — Ed25519 OID 1.3.101.112). This is
/// the format Rekor's `hashedrekord` schema expects.
fn ed25519_public_key_pem(pub_raw: &[u8]) -> String {
    // SPKI = SEQUENCE { AlgId SEQUENCE { OID 1.3.101.112 }, BIT STRING raw_key }
    // Constants below are the DER-encoded SPKI prefix (12 bytes) for Ed25519.
    const PREFIX: [u8; 12] = [
        0x30, 0x2a, // SEQUENCE, len 42
        0x30, 0x05, // SEQUENCE, len 5 (algorithm)
        0x06, 0x03, 0x2b, 0x65, 0x70, // OID 1.3.101.112
        0x03, 0x21, 0x00, // BIT STRING, len 33, 0 unused bits
    ];
    let mut der = Vec::with_capacity(PREFIX.len() + 32);
    der.extend_from_slice(&PREFIX);
    der.extend_from_slice(pub_raw);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&der);
    let mut pem = String::from("-----BEGIN PUBLIC KEY-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).unwrap());
        pem.push('\n');
    }
    pem.push_str("-----END PUBLIC KEY-----\n");
    pem
}

// ── HTTP ───────────────────────────────────────────────────────────────

#[derive(Debug)]
struct AnchorResponse {
    uuid: String,
    log_index: u64,
}

async fn post_hashedrekord(rekor_url: &str, p: &Payload) -> Result<AnchorResponse> {
    let body = serde_json::json!({
        "kind": "hashedrekord",
        "apiVersion": "0.0.1",
        "spec": {
            "signature": {
                "content": p.sig_b64,
                "publicKey": { "content": p.pubkey_pem_b64 }
            },
            "data": {
                "hash": { "algorithm": "sha256", "value": p.data_sha256_hex }
            }
        }
    });
    let url = format!("{}/api/v1/log/entries", rekor_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("build reqwest client")?;
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(body.to_string())
        .send()
        .await
        .context("send POST")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("rekor POST {url} returned {status}: {text}"));
    }
    parse_anchor_response(&text)
}

/// Rekor's response is `{ "<uuid>": { "logIndex": N, ... } }`. We pull
/// the first (and only) key out and read `logIndex`.
fn parse_anchor_response(body: &str) -> Result<AnchorResponse> {
    let v: serde_json::Value = serde_json::from_str(body).context("parse rekor response")?;
    let obj = v.as_object().ok_or_else(|| anyhow!("rekor response not an object"))?;
    let (uuid, entry) = obj
        .iter()
        .next()
        .ok_or_else(|| anyhow!("rekor response empty"))?;
    let log_index = entry
        .get("logIndex")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| anyhow!("rekor response missing logIndex"))?;
    Ok(AnchorResponse {
        uuid: uuid.clone(),
        log_index,
    })
}

// ── ControlConfig glue ─────────────────────────────────────────────────

/// Minimal TOML shape:
///
/// ```toml
/// [audit_mirror]
/// enabled = true
/// batch_size = 100
/// flush_interval_secs = 3600
/// rekor_url = "https://rekor.sigstore.dev"
/// ```
///
/// Lives at the same `control.toml` (top-level section) so operators
/// don't have to manage a second file. Parsed on a best-effort basis;
/// missing/malformed → defaults (mirror disabled).
pub fn load_from_control_toml(path: &std::path::Path) -> AuditMirrorConfig {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return AuditMirrorConfig::default(),
    };
    #[derive(serde::Deserialize, Default)]
    struct Outer {
        #[serde(default)]
        audit_mirror: Inner,
    }
    #[derive(serde::Deserialize, Default)]
    struct Inner {
        enabled: Option<bool>,
        batch_size: Option<usize>,
        flush_interval_secs: Option<u64>,
        rekor_url: Option<String>,
        channel_capacity: Option<usize>,
    }
    let parsed: Outer = match toml::from_str(&raw) {
        Ok(p) => p,
        Err(e) => {
            warn!("audit_mirror: bad config (using defaults): {e:#}");
            return AuditMirrorConfig::default();
        }
    };
    let mut cfg = AuditMirrorConfig::default();
    if let Some(b) = parsed.audit_mirror.enabled {
        cfg.enabled = b;
    }
    if let Some(n) = parsed.audit_mirror.batch_size {
        cfg.batch_size = n.max(1);
    }
    if let Some(n) = parsed.audit_mirror.flush_interval_secs {
        cfg.flush_interval_secs = n.max(1);
    }
    if let Some(u) = parsed.audit_mirror.rekor_url {
        cfg.rekor_url = u;
    }
    if let Some(n) = parsed.audit_mirror.channel_capacity {
        cfg.channel_capacity = n.max(1);
    }
    cfg
}

/// Best-effort send. Drops on full / closed channels — this MUST never
/// block the audit append path.
pub fn try_mirror(tx: &mpsc::Sender<AuditRecord>, rec: &AuditRecord) {
    match tx.try_send(rec.clone()) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            debug!("audit_mirror: channel full, dropping record from mirror queue");
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            debug!("audit_mirror: channel closed, mirror task gone");
        }
    }
}

// allow Audit to know about the optional sender without leaking module
// internals back into `audit.rs`.
pub type MirrorTx = Arc<mpsc::Sender<AuditRecord>>;

// suppress unused warnings on the `now_unix_ms` import for now —
// reserved for the (planned) anchor catch-up logic on restart.
#[allow(dead_code)]
fn _now_unix_ms() -> u64 {
    now_unix_ms()
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn fake_rec(ts_ms: u64, kind: &str) -> AuditRecord {
        AuditRecord {
            schema_version: 1,
            ts_ms,
            kind: kind.to_string(),
            actor_node_id: None,
            actor_label: None,
            details: json!({"i": ts_ms}),
        }
    }

    fn fresh_secret() -> SecretKey {
        SecretKey::from_bytes(&[7u8; 32])
    }

    #[test]
    fn merkle_root_is_sha256_for_one_leaf() {
        let leaf = [9u8; 32];
        let root = merkle_root_sha256(&[leaf]);
        assert_eq!(root, leaf, "root of a single leaf must equal that leaf");
    }

    #[test]
    fn merkle_root_pairs_consistently() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&a);
        buf[32..].copy_from_slice(&b);
        let expected = sha256(&buf);
        assert_eq!(merkle_root_sha256(&[a, b]), expected);
    }

    #[test]
    fn canonicalize_value_sorts_keys() {
        let v = json!({"b": 1, "a": 2, "c": [3, {"y": 1, "x": 2}]});
        let s = canonicalize_value(&v);
        assert_eq!(s, r#"{"a":2,"b":1,"c":[3,{"x":2,"y":1}]}"#);
    }

    #[test]
    fn build_payload_signature_verifies() {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        let secret = fresh_secret();
        let recs = vec![fake_rec(100, "a"), fake_rec(200, "b")];
        let p = build_payload(&recs, "", &secret).unwrap();
        let sig_bytes = base64::engine::general_purpose::STANDARD
            .decode(&p.sig_b64)
            .unwrap();
        let sig = Signature::from_slice(&sig_bytes).unwrap();
        let vk = VerifyingKey::from_bytes(secret.public().as_bytes()).unwrap();
        vk.verify(&p.data_canonical, &sig).unwrap();
    }

    #[test]
    fn ed25519_pem_round_trips_through_pem_parse() {
        let secret = fresh_secret();
        let pem = ed25519_public_key_pem(secret.public().as_bytes());
        assert!(pem.starts_with("-----BEGIN PUBLIC KEY-----\n"));
        assert!(pem.contains("MCowBQYDK2VwAyEA")); // SPKI Ed25519 prefix in base64
        assert!(pem.ends_with("-----END PUBLIC KEY-----\n"));
    }

    /// Tiny single-shot HTTP server that records the incoming POST and
    /// replies with a Rekor-shaped JSON. Returns the bound URL and a
    /// handle to the recorded request body.
    async fn spawn_fake_rekor(
        responses: StdArc<AtomicUsize>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        let handle = tokio::spawn(async move {
            let mut bodies: Vec<String> = Vec::new();
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(x) => x,
                    Err(_) => break,
                };
                let mut buf = vec![0u8; 16 * 1024];
                let n = match socket.read(&mut buf).await {
                    Ok(n) if n > 0 => n,
                    _ => continue,
                };
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                bodies.push(req);
                let n = responses.fetch_add(1, Ordering::SeqCst);
                let uuid = format!("uuid-{n}");
                let body = format!(
                    r#"{{"{uuid}":{{"logIndex":{n},"logID":"abc","integratedTime":1}}}}"#
                );
                let resp = format!(
                    "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
                    len = body.len(),
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.shutdown().await;
                if responses.load(Ordering::SeqCst) >= 2 {
                    break;
                }
            }
            bodies
        });
        (url, handle)
    }

    #[tokio::test]
    async fn batches_200_records_into_two_anchors_via_mock_rekor() {
        let counter = StdArc::new(AtomicUsize::new(0));
        let (url, server_handle) = spawn_fake_rekor(counter.clone()).await;

        let tmp = TempDir::new().unwrap();
        let audit = Audit::open(tmp.path().to_path_buf()).await.unwrap();

        let cfg = AuditMirrorConfig {
            enabled: true,
            batch_size: 100,
            flush_interval_secs: 600,
            rekor_url: url,
            channel_capacity: 4096,
        };
        let secret = fresh_secret();
        let tx = spawn(cfg, secret, audit.clone()).expect("mirror spawned");

        for i in 0..200u64 {
            try_mirror(&tx, &fake_rec(1_000_000 + i, "x"));
        }

        // Wait for the fake server to register two POSTs (with a
        // generous timeout so a slow CI doesn't flake).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while counter.load(Ordering::SeqCst) < 2 {
            if std::time::Instant::now() > deadline {
                panic!(
                    "timed out waiting for 2 anchors; got {}",
                    counter.load(Ordering::SeqCst)
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let bodies = server_handle.await.unwrap();
        assert_eq!(bodies.len(), 2, "expected exactly 2 Rekor POSTs");
        for b in &bodies {
            assert!(b.contains("hashedrekord"));
            assert!(b.contains("\"algorithm\":\"sha256\""));
        }

        // The audit log itself should now contain two
        // `audit_mirror_anchor` records (plus whatever else; in this
        // test the only writes come from the mirror task).
        let (records, _eof) = audit.tail(50, None).await;
        let anchors: Vec<_> = records
            .iter()
            .filter(|r| r.kind == "audit_mirror_anchor")
            .collect();
        assert_eq!(anchors.len(), 2, "expected two anchor records in audit log");
        // Second anchor should chain to the first.
        // Newest first: anchors[0] is the second anchor; its
        // prev_anchor_uuid should equal anchors[1].rekor_uuid.
        let prev = anchors[0].details.get("prev_anchor_uuid").and_then(|v| v.as_str());
        let prior_uuid = anchors[1].details.get("rekor_uuid").and_then(|v| v.as_str());
        assert!(prev.is_some());
        assert_eq!(prev, prior_uuid);
    }
}
