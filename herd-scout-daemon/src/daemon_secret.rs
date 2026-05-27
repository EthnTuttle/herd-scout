//! Persist the daemon's iroh `SecretKey` across restarts.
//!
//! Wave 12 Phase 0: the on-disk format is a versioned TOML envelope at
//! `<config_dir>/herd-scout/identity.toml` (see [`herd_scout_identity`]).
//! The legacy 64-hex `<data_dir>/herd-scout/iroh_secret` file is
//! auto-migrated on first run.
//!
//! `Live::from_env()` reads `IROH_SECRET` (64 lowercase-hex chars), so
//! after we resolve the identity we still set that env var in-process —
//! no patch to `iroh-live` required.
//!
//! Resolution order:
//! 1. If `IROH_SECRET` is already set, honor the operator override and
//!    persist it as the canonical identity (one-shot migration so the
//!    env var becomes optional next time).
//! 2. Else load `identity.toml` if present (with auto-migration from
//!    legacy formats — see `herd_scout_identity::load_or_generate`).
//! 3. Else generate a fresh identity, persist, and use it.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use herd_scout_identity::{Identity, SCHEMA_VERSION};
use iroh::SecretKey;

const ENV_VAR: &str = "IROH_SECRET";

const APP_QUALIFIER: &str = "net";
const APP_ORG: &str = "herd-scout";
const APP_NAME: &str = "herd-scout";

const IDENTITY_LABEL: &str = "herd-scout-daemon";

fn identity_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .ok_or_else(|| anyhow!("no user-config directory available on this platform"))?;
    Ok(dirs.config_dir().join("identity.toml"))
}

/// Pre-Wave-12 location of the daemon's iroh secret (64-hex). The
/// `herd_scout_identity` crate looks for legacy files *next to* the
/// identity.toml; this one lived in a different directory entirely
/// (`<data_dir>/herd-scout/iroh_secret`), so we migrate it explicitly.
fn legacy_data_dir_secret_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .ok_or_else(|| anyhow!("no user-data directory available on this platform"))?;
    Ok(dirs.data_dir().join("iroh_secret"))
}

/// One-shot migration of `<data_dir>/herd-scout/iroh_secret` (64-hex)
/// into the new `identity.toml` envelope at `<config_dir>/...`.
/// Returns Some(Identity) only when the legacy file existed and was
/// successfully migrated.
fn try_migrate_data_dir_legacy(target: &std::path::Path) -> Result<Option<Identity>> {
    let legacy = legacy_data_dir_secret_path()?;
    if !legacy.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&legacy)
        .with_context(|| format!("read legacy {}", legacy.display()))?;
    let hex = text.trim().to_ascii_lowercase();
    let bytes = decode_hex(&hex).with_context(|| format!("parse legacy {}", legacy.display()))?;
    let secret = SecretKey::from_bytes(&bytes);
    let id = Identity {
        secret,
        label: IDENTITY_LABEL.into(),
        created_at: String::new(),
    };
    herd_scout_identity::save(target, &id, IDENTITY_LABEL)
        .with_context(|| format!("persist migrated identity to {}", target.display()))?;
    let _ = std::fs::remove_file(&legacy);
    tracing::info!(
        from = %legacy.display(),
        to = %target.display(),
        node_id = %id.node_id(),
        "identity: migrated legacy <data_dir>/iroh_secret → identity.toml",
    );
    Ok(Some(id))
}

/// Ensure the daemon has a stable iroh identity, persist it as a v1
/// `identity.toml` envelope, and export it via `IROH_SECRET` for
/// `Live::from_env()`.
pub fn ensure_iroh_secret_persisted() -> Result<()> {
    let path = identity_path()?;

    let identity = if let Ok(env_hex) = std::env::var(ENV_VAR) {
        // Operator override: build an Identity from the env-var hex,
        // then persist if disk doesn't already match.
        let env_hex = env_hex.trim().to_ascii_lowercase();
        let bytes = decode_hex(&env_hex).context("IROH_SECRET env var")?;
        let secret = SecretKey::from_bytes(&bytes);
        let id = Identity {
            secret,
            label: IDENTITY_LABEL.into(),
            created_at: String::new(),
        };
        let needs_write = match herd_scout_identity::load(&path) {
            Ok(existing) => existing.node_id() != id.node_id(),
            Err(_) => true,
        };
        if needs_write {
            herd_scout_identity::save(&path, &id, IDENTITY_LABEL)
                .with_context(|| format!("persist identity to {}", path.display()))?;
            tracing::info!(
                path = %path.display(),
                "persisted IROH_SECRET from env to identity.toml",
            );
        }
        id
    } else if path.exists() {
        // Already migrated; just load.
        herd_scout_identity::load(&path)
            .with_context(|| format!("load identity at {}", path.display()))?
    } else if let Some(id) = try_migrate_data_dir_legacy(&path)? {
        // First-run after Wave 12: pull the legacy 64-hex from `<data_dir>`.
        id
    } else {
        // No identity anywhere — generate fresh and persist.
        herd_scout_identity::load_or_generate(&path, IDENTITY_LABEL)
            .with_context(|| format!("load or create identity at {}", path.display()))?
    };

    let hex = encode_hex(&identity.secret.to_bytes());
    // SAFETY: we are the daemon's single-threaded startup path.
    unsafe { std::env::set_var(ENV_VAR, &hex) };

    tracing::info!(
        path = %path.display(),
        node_id = %identity.node_id(),
        schema_version = SCHEMA_VERSION,
        "iroh identity ready",
    );
    Ok(())
}

fn encode_hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    out
}

fn decode_hex(s: &str) -> Result<[u8; 32]> {
    if s.len() != 64 {
        return Err(anyhow!(
            "iroh secret must be 64 hex chars, got {}",
            s.len()
        ));
    }
    let bytes = s.as_bytes();
    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = hex_digit(bytes[i * 2])?;
        let lo = hex_digit(bytes[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_digit(b: u8) -> Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(anyhow!("non-hex character in iroh secret")),
    }
}
