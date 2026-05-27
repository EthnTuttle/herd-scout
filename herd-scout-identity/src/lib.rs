//! Versioned iroh-identity envelope.
//!
//! Replaces the ad-hoc 32-raw-byte (`herdctl`) and 64-hex-char
//! (`herd-scout-daemon`) on-disk formats with a single TOML envelope
//! carrying a schema version, an integrity-check `node_id`, and human
//! metadata. Used by daemon, herdctl, and the Android admin app.
//!
//! Wave 12 Phase 0 of `plan-android-admin-allowlist-app-2026-05-27`.

#![deny(missing_debug_implementations)]

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tracing::info;

/// Latest schema version this build understands. Files with a higher
/// version are refused (forward-compat: install a newer build).
pub const SCHEMA_VERSION: u32 = 1;

/// In-memory identity. The `secret` is the source of truth for the
/// NodeId; `label` and `created_at` are metadata.
#[derive(Debug, Clone)]
pub struct Identity {
    pub secret: SecretKey,
    pub label: String,
    pub created_at: String,
}

impl Identity {
    /// Generate a fresh identity with the given label.
    pub fn generate(label: impl Into<String>) -> Self {
        Self {
            secret: SecretKey::generate(),
            label: label.into(),
            created_at: now_rfc3339(),
        }
    }

    /// The canonical NodeId string for this identity.
    pub fn node_id(&self) -> String {
        self.secret.public().to_string()
    }
}

/// Errors returned by identity load / parse paths.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("io error reading identity at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("toml parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error(
        "unsupported identity schema_version {found} (this build supports up to {max_supported}); install a newer herd-scout"
    )]
    UnsupportedSchema { found: u32, max_supported: u32 },
    #[error("identity secret_key must be 64 lowercase hex chars (32 bytes), got {0} chars")]
    BadKeyLength(usize),
    #[error("identity secret_key contains non-hex characters")]
    BadKeyEncoding,
    #[error(
        "identity integrity check failed: file claims node_id {claimed:?} but secret_key derives {derived:?}"
    )]
    IntegrityCheckFailed { claimed: String, derived: String },
}

// ── On-disk envelope ────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    schema_version: u32,
    identity: EnvelopeIdentity,
    #[serde(default)]
    origin: EnvelopeOrigin,
}

#[derive(Debug, Serialize, Deserialize)]
struct EnvelopeIdentity {
    secret_key: String, // 64 lowercase-hex chars
    node_id: String,    // canonical EndpointId; integrity gate
    #[serde(default)]
    label: String,
    #[serde(default)]
    created_at: String, // RFC3339, informational
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct EnvelopeOrigin {
    #[serde(default = "default_device")]
    device: String,
    #[serde(default)]
    app_version: String,
}

fn default_device() -> String {
    if cfg!(target_os = "android") {
        "android".into()
    } else if cfg!(target_os = "linux") {
        "linux".into()
    } else if cfg!(target_os = "macos") {
        "macos".into()
    } else {
        "unknown".into()
    }
}

// ── Public API ──────────────────────────────────────────────────────────

/// Parse an envelope from text (no I/O). Used for both file-load and
/// import-from-user-blob paths so they share validation.
pub fn parse_envelope(s: &str) -> Result<Identity, IdentityError> {
    let env: Envelope = toml::from_str(s)?;

    if env.schema_version > SCHEMA_VERSION {
        return Err(IdentityError::UnsupportedSchema {
            found: env.schema_version,
            max_supported: SCHEMA_VERSION,
        });
    }

    let bytes = decode_secret_hex(&env.identity.secret_key)?;
    let secret = SecretKey::from_bytes(&bytes);

    let derived = secret.public().to_string();
    if env.identity.node_id != derived {
        return Err(IdentityError::IntegrityCheckFailed {
            claimed: env.identity.node_id,
            derived,
        });
    }

    Ok(Identity {
        secret,
        label: env.identity.label,
        created_at: env.identity.created_at,
    })
}

/// Render an identity as the canonical envelope text. `label` overrides
/// `id.label` so callers can stamp the export label without mutating the
/// in-memory identity.
pub fn render_envelope(id: &Identity, label: &str) -> String {
    let env = Envelope {
        schema_version: SCHEMA_VERSION,
        identity: EnvelopeIdentity {
            secret_key: encode_secret_hex(&id.secret.to_bytes()),
            node_id: id.node_id(),
            label: label.to_string(),
            created_at: if id.created_at.is_empty() {
                now_rfc3339()
            } else {
                id.created_at.clone()
            },
        },
        origin: EnvelopeOrigin {
            device: default_device(),
            app_version: env!("CARGO_PKG_VERSION").into(),
        },
    };
    toml::to_string_pretty(&env).expect("envelope always serializes")
}

/// Load an identity from `path`. Returns the typed error so callers can
/// surface schema-mismatch / integrity-failure differently from "file
/// missing."
pub fn load(path: &Path) -> Result<Identity, IdentityError> {
    let text = fs::read_to_string(path).map_err(|e| IdentityError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    parse_envelope(&text)
}

/// Atomically persist an identity to `path`. Writes `<path>.tmp` with
/// mode `0600`, fsyncs, then renames. Creates parent dirs as needed.
pub fn save(path: &Path, id: &Identity, label: &str) -> Result<(), IdentityError> {
    let text = render_envelope(id, label);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| IdentityError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let tmp = with_tmp_suffix(path);
    let _ = fs::remove_file(&tmp); // best-effort cleanup of crashed prior write
    write_atomic(&tmp, text.as_bytes()).map_err(|e| IdentityError::Io {
        path: tmp.clone(),
        source: e,
    })?;
    fs::rename(&tmp, path).map_err(|e| IdentityError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// Load `path` if it exists; otherwise generate, save, and return a
/// fresh identity. Also handles the one-time legacy migrations from
/// raw-32-byte (`herdctl`) and 64-hex (`herd-scout-daemon`) formats —
/// see `try_migrate_legacy`.
pub fn load_or_generate(path: &Path, label: &str) -> Result<Identity, IdentityError> {
    match load(path) {
        Ok(id) => Ok(id),
        Err(IdentityError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            if let Some(id) = try_migrate_legacy(path, label)? {
                return Ok(id);
            }
            let id = Identity::generate(label);
            save(path, &id, label)?;
            info!(path = %path.display(), node_id = %id.node_id(), "identity: generated new");
            Ok(id)
        }
        Err(e) => Err(e),
    }
}

/// Import an identity from arbitrary user-supplied envelope text.
/// Identical validation to `load`; named differently for self-documentation
/// at call sites in the Android Import flow.
pub fn import_from_user_blob(s: &str) -> Result<Identity, IdentityError> {
    parse_envelope(s)
}

/// Render an identity for export. Same as `render_envelope`; named
/// differently for self-documentation at Export-button call sites.
pub fn export_to_user_blob(id: &Identity, label: &str) -> String {
    render_envelope(id, label)
}

// ── Legacy migration ────────────────────────────────────────────────────
//
// Two pre-Wave-12 formats exist on disk in real deployments:
//
//   1. `<config_dir>/herdctl/secret.key`           — raw 32 bytes
//   2. `<data_dir>/herd-scout/iroh_secret`         — 64 lowercase-hex chars (+ optional trailing newline)
//
// On first call to `load_or_generate(path, ...)` where `path` doesn't
// exist, we look for either legacy file *next to* the requested path
// (same parent dir, fixed legacy name) and upgrade in place if found.
// The legacy file is removed only after the new envelope is durable on
// disk.

fn try_migrate_legacy(path: &Path, label: &str) -> Result<Option<Identity>, IdentityError> {
    let parent = match path.parent() {
        Some(p) => p,
        None => return Ok(None),
    };
    // herdctl-style: 32 raw bytes at "secret.key"
    let legacy_raw = parent.join("secret.key");
    if legacy_raw.exists() {
        let bytes = fs::read(&legacy_raw).map_err(|e| IdentityError::Io {
            path: legacy_raw.clone(),
            source: e,
        })?;
        if bytes.len() == 32 {
            let arr: [u8; 32] = bytes.as_slice().try_into().expect("len checked");
            let secret = SecretKey::from_bytes(&arr);
            let id = Identity {
                secret,
                label: label.to_string(),
                created_at: now_rfc3339(),
            };
            save(path, &id, label)?;
            // Only remove the legacy file once the new one is durable.
            let _ = fs::remove_file(&legacy_raw);
            info!(
                from = %legacy_raw.display(),
                to = %path.display(),
                node_id = %id.node_id(),
                "identity: migrated legacy raw secret.key → identity.toml",
            );
            return Ok(Some(id));
        }
    }
    // daemon-style: 64-hex at "iroh_secret"
    let legacy_hex = parent.join("iroh_secret");
    if legacy_hex.exists() {
        let text = fs::read_to_string(&legacy_hex).map_err(|e| IdentityError::Io {
            path: legacy_hex.clone(),
            source: e,
        })?;
        let hex = text.trim().to_ascii_lowercase();
        let bytes = decode_secret_hex(&hex)?;
        let secret = SecretKey::from_bytes(&bytes);
        let id = Identity {
            secret,
            label: label.to_string(),
            created_at: now_rfc3339(),
        };
        save(path, &id, label)?;
        let _ = fs::remove_file(&legacy_hex);
        info!(
            from = %legacy_hex.display(),
            to = %path.display(),
            node_id = %id.node_id(),
            "identity: migrated legacy iroh_secret hex → identity.toml",
        );
        return Ok(Some(id));
    }
    Ok(None)
}

/// Type-check that the given canonical NodeId string round-trips
/// through `EndpointId::from_str`. Useful when callers want to
/// pre-validate a node_id field before stuffing it into a config.
pub fn validate_node_id_string(s: &str) -> Result<(), String> {
    EndpointId::from_str(s).map(|_| ()).map_err(|e| e.to_string())
}

// ── helpers ─────────────────────────────────────────────────────────────

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

fn encode_secret_hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    out
}

fn decode_secret_hex(s: &str) -> Result<[u8; 32], IdentityError> {
    if s.len() != 64 {
        return Err(IdentityError::BadKeyLength(s.len()));
    }
    let mut out = [0u8; 32];
    let bytes = s.as_bytes();
    for i in 0..32 {
        let hi = hex_digit(bytes[i * 2]).ok_or(IdentityError::BadKeyEncoding)?;
        let lo = hex_digit(bytes[i * 2 + 1]).ok_or(IdentityError::BadKeyEncoding)?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn with_tmp_suffix(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(".tmp");
    PathBuf::from(p)
}

#[cfg(unix)]
fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_envelope() {
        let id = Identity::generate("Gary's Pixel");
        let s = render_envelope(&id, "Gary's Pixel");
        let parsed = parse_envelope(&s).unwrap();
        assert_eq!(parsed.node_id(), id.node_id());
        assert_eq!(parsed.label, "Gary's Pixel");
    }

    #[test]
    fn save_then_load() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("identity.toml");
        let id = Identity::generate("test");
        save(&path, &id, "test").unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.node_id(), id.node_id());
    }

    #[test]
    fn integrity_check_rejects_tampered_secret() {
        let id = Identity::generate("test");
        let mut s = render_envelope(&id, "test");
        // Tamper with the first hex byte of secret_key (find and flip)
        let pos = s.find("secret_key = \"").unwrap() + "secret_key = \"".len();
        let mut bytes: Vec<u8> = s.into_bytes();
        bytes[pos] = if bytes[pos] == b'a' { b'b' } else { b'a' };
        s = String::from_utf8(bytes).unwrap();
        match parse_envelope(&s) {
            Err(IdentityError::IntegrityCheckFailed { .. }) => {}
            other => panic!("expected IntegrityCheckFailed, got {other:?}"),
        }
    }

    #[test]
    fn future_schema_rejected() {
        let s = format!(
            "schema_version = {}\n[identity]\nsecret_key = \"{}\"\nnode_id = \"x\"\nlabel = \"\"\ncreated_at = \"\"\n",
            SCHEMA_VERSION + 1,
            "0".repeat(64)
        );
        match parse_envelope(&s) {
            Err(IdentityError::UnsupportedSchema { found, max_supported }) => {
                assert_eq!(found, SCHEMA_VERSION + 1);
                assert_eq!(max_supported, SCHEMA_VERSION);
            }
            other => panic!("expected UnsupportedSchema, got {other:?}"),
        }
    }

    #[test]
    fn bad_key_length_rejected() {
        let s = "schema_version = 1\n[identity]\nsecret_key = \"abc\"\nnode_id = \"x\"\n";
        match parse_envelope(s) {
            Err(IdentityError::BadKeyLength(3)) => {}
            other => panic!("expected BadKeyLength, got {other:?}"),
        }
    }

    #[test]
    fn load_or_generate_creates_fresh_when_missing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("subdir").join("identity.toml");
        let id = load_or_generate(&path, "fresh").unwrap();
        assert!(path.exists());
        let again = load_or_generate(&path, "ignored").unwrap();
        assert_eq!(id.node_id(), again.node_id());
    }

    #[test]
    fn migrates_legacy_raw_32_bytes() {
        let tmp = TempDir::new().unwrap();
        let raw_path = tmp.path().join("secret.key");
        std::fs::write(&raw_path, [42u8; 32]).unwrap();
        let new_path = tmp.path().join("identity.toml");
        let id = load_or_generate(&new_path, "migrated").unwrap();
        assert!(new_path.exists());
        assert!(!raw_path.exists(), "legacy file should be gone");
        // NodeId should be derivable from [42; 32]
        let expected = SecretKey::from_bytes(&[42u8; 32]).public().to_string();
        assert_eq!(id.node_id(), expected);
    }

    #[test]
    fn migrates_legacy_hex_iroh_secret() {
        let tmp = TempDir::new().unwrap();
        let hex_path = tmp.path().join("iroh_secret");
        std::fs::write(&hex_path, format!("{}\n", "ab".repeat(32))).unwrap();
        let new_path = tmp.path().join("identity.toml");
        let id = load_or_generate(&new_path, "daemon").unwrap();
        assert!(new_path.exists());
        assert!(!hex_path.exists());
        let expected = SecretKey::from_bytes(&[0xabu8; 32]).public().to_string();
        assert_eq!(id.node_id(), expected);
    }
}
