//! Wave 11 — `control.toml` parsing for the SSH-bridge allowlist.
//!
//! Fail-closed: if the file is missing the allowlist is empty and every
//! incoming control-plane dial is dropped. Parse errors propagate so the
//! caller can decide whether startup is fatal (it is, at boot) or whether
//! to keep the previous good config (SIGHUP reload).

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use herd_scout_ipc::AllowedEntry;
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use tracing::warn;

const DEFAULT_SSH_TARGET: &str = "127.0.0.1:22";

/// Resolved control-plane config.
#[derive(Debug, Clone)]
pub(crate) struct ControlConfig {
    /// SSH-bridge allowlist with labels (Wave 12 schema). Source of
    /// truth for serialization back to disk; `allowed_node_ids` is the
    /// O(1) gate-check projection.
    pub(crate) allowed: Vec<AllowedEntry>,
    /// O(1) lookup set rebuilt from `allowed` on every load. Wave 11
    /// SSH handler uses this directly.
    pub(crate) allowed_node_ids: HashSet<EndpointId>,
    /// Admin RPC allowlist: peers that may dial `ADMIN_ALPN` and call
    /// allowlist-mutation RPCs. Wave 12.
    pub(crate) admins: HashSet<EndpointId>,
    pub(crate) ssh_target: SocketAddr,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            allowed: Vec::new(),
            allowed_node_ids: HashSet::new(),
            admins: HashSet::new(),
            ssh_target: DEFAULT_SSH_TARGET
                .parse()
                .expect("hardcoded 127.0.0.1:22 always parses"),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct RawFile {
    pub(crate) control_plane: RawSection,
}

/// Both the parser input and the rewriter output. Backwards-compat:
/// reads either `allowed_node_ids = [...]` (Wave 11) *or*
/// `[[control_plane.allowed]]` array-of-tables (Wave 12). Always
/// writes the new shape.
#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct RawSection {
    /// Wave 12 schema: labeled SSH allowlist. `[[control_plane.allowed]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) allowed: Vec<RawAllowed>,
    /// Wave 11 schema: bare NodeId list. Honored on read; never written.
    #[serde(default, skip_serializing)]
    pub(crate) allowed_node_ids: Vec<String>,
    /// Wave 12: admin RPC allowlist. Plain list of canonical NodeId
    /// strings. Empty by default (fail-closed); requires hand-edit +
    /// SIGHUP for the first admin device.
    #[serde(default)]
    pub(crate) admins: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ssh_target: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct RawAllowed {
    pub(crate) node_id: String,
    #[serde(default)]
    pub(crate) label: String,
}

/// Resolve the `control.toml` path. Honors `$HERD_SCOUT_CONFIG_DIR` for
/// tests / non-standard installs; otherwise uses the platform config dir.
pub(crate) fn config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("HERD_SCOUT_CONFIG_DIR") {
        return PathBuf::from(dir).join("control.toml");
    }
    match directories::ProjectDirs::from("net", "herd-scout", "herd-scout") {
        Some(p) => p.config_dir().join("control.toml"),
        None => PathBuf::from("control.toml"),
    }
}

/// Load `control.toml` from `path`. Returns the default (empty allowlist)
/// when the file is missing. Returns Err only when the file exists but
/// is malformed.
pub(crate) fn load_or_default(path: &Path) -> Result<ControlConfig> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                path = %path.display(),
                "no control.toml found — control plane closed to all peers",
            );
            return Ok(ControlConfig::default());
        }
        Err(e) => {
            return Err(e).with_context(|| format!("reading {}", path.display()));
        }
    };

    let parsed: RawFile =
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

    // Build the labeled allowlist from whichever schema the file uses.
    // Prefer `[[control_plane.allowed]]` (Wave 12) when present; fall
    // back to bare `allowed_node_ids` (Wave 11) for backwards compat.
    let mut allowed: Vec<AllowedEntry> =
        Vec::with_capacity(parsed.control_plane.allowed.len().max(
            parsed.control_plane.allowed_node_ids.len(),
        ));
    let mut allowed_set: HashSet<EndpointId> = HashSet::new();

    for raw in &parsed.control_plane.allowed {
        let trimmed = raw.node_id.trim();
        if trimmed.is_empty() {
            continue;
        }
        let id = EndpointId::from_str(trimmed)
            .with_context(|| format!("invalid node id in allowed: {trimmed:?}"))?;
        if allowed_set.insert(id) {
            allowed.push(AllowedEntry {
                node_id: trimmed.to_string(),
                label: raw.label.clone(),
            });
        }
    }
    for s in &parsed.control_plane.allowed_node_ids {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            continue;
        }
        let id = EndpointId::from_str(trimmed)
            .with_context(|| format!("invalid node id in allowed_node_ids: {trimmed:?}"))?;
        if allowed_set.insert(id) {
            allowed.push(AllowedEntry {
                node_id: trimmed.to_string(),
                label: String::new(),
            });
        }
    }

    let mut admins = HashSet::with_capacity(parsed.control_plane.admins.len());
    for s in &parsed.control_plane.admins {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            continue;
        }
        let id = EndpointId::from_str(trimmed)
            .with_context(|| format!("invalid node id in admins: {trimmed:?}"))?;
        admins.insert(id);
    }

    let ssh_target = match parsed.control_plane.ssh_target.as_deref() {
        Some(s) => SocketAddr::from_str(s.trim())
            .with_context(|| format!("invalid ssh_target {s:?}"))?,
        None => DEFAULT_SSH_TARGET
            .parse()
            .expect("hardcoded default parses"),
    };

    Ok(ControlConfig {
        allowed,
        allowed_node_ids: allowed_set,
        admins,
        ssh_target,
    })
}

/// Atomically rewrite `control.toml` to match `cfg`. Writes a temp file
/// in the same parent dir with mode `0600`, fsyncs, then renames over
/// the target. On any failure the original file is untouched.
///
/// **Comments are not preserved** — the file becomes machine-managed
/// once the admin RPC starts writing it. The first admin RPC write
/// also rewrites the schema in the new `[[control_plane.allowed]]`
/// shape (Wave 11 `allowed_node_ids = [...]` is read but never written).
pub(crate) fn write_atomic(path: &Path, cfg: &ControlConfig) -> Result<()> {
    let raw = RawFile {
        control_plane: RawSection {
            allowed: cfg
                .allowed
                .iter()
                .map(|e| RawAllowed {
                    node_id: e.node_id.clone(),
                    label: e.label.clone(),
                })
                .collect(),
            allowed_node_ids: Vec::new(), // never written
            admins: cfg.admins.iter().map(|id| id.to_string()).collect(),
            ssh_target: Some(cfg.ssh_target.to_string()),
        },
    };
    let text = toml::to_string_pretty(&raw).context("serializing control.toml")?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create parent dir {}", parent.display()))?;
    }
    let tmp = with_tmp_suffix(path);
    let _ = std::fs::remove_file(&tmp);
    write_file_0600(&tmp, text.as_bytes())
        .with_context(|| format!("writing temp {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

fn with_tmp_suffix(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(".tmp");
    PathBuf::from(p)
}

#[cfg(unix)]
fn write_file_0600(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_file_0600(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_node_id(seed: u8) -> EndpointId {
        let bytes = [seed; 32];
        let secret = iroh::SecretKey::from_bytes(&bytes);
        secret.public()
    }

    #[test]
    fn parses_wave11_legacy_schema() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("control.toml");
        let id = make_node_id(7);
        std::fs::write(
            &path,
            format!(
                r#"
[control_plane]
allowed_node_ids = ["{id}"]
ssh_target = "127.0.0.1:22"
"#
            ),
        )
        .unwrap();
        let cfg = load_or_default(&path).unwrap();
        assert_eq!(cfg.allowed.len(), 1);
        assert_eq!(cfg.allowed[0].node_id, id.to_string());
        assert_eq!(cfg.allowed[0].label, "");
        assert!(cfg.allowed_node_ids.contains(&id));
    }

    #[test]
    fn parses_wave12_labeled_schema() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("control.toml");
        let id = make_node_id(11);
        let admin_id = make_node_id(13);
        std::fs::write(
            &path,
            format!(
                r#"
[control_plane]
admins = ["{admin_id}"]
ssh_target = "127.0.0.1:22"

[[control_plane.allowed]]
node_id = "{id}"
label = "phone"
"#
            ),
        )
        .unwrap();
        let cfg = load_or_default(&path).unwrap();
        assert_eq!(cfg.allowed.len(), 1);
        assert_eq!(cfg.allowed[0].label, "phone");
        assert!(cfg.allowed_node_ids.contains(&id));
        assert!(cfg.admins.contains(&admin_id));
    }

    #[test]
    fn write_atomic_round_trips_through_load() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("control.toml");
        let id = make_node_id(17);
        let admin_id = make_node_id(19);

        let cfg = ControlConfig {
            allowed: vec![AllowedEntry {
                node_id: id.to_string(),
                label: "Pixel".into(),
            }],
            allowed_node_ids: [id].into_iter().collect(),
            admins: [admin_id].into_iter().collect(),
            ssh_target: "127.0.0.1:2222".parse().unwrap(),
        };
        write_atomic(&path, &cfg).unwrap();
        let loaded = load_or_default(&path).unwrap();
        assert_eq!(loaded.allowed.len(), 1);
        assert_eq!(loaded.allowed[0].label, "Pixel");
        assert!(loaded.admins.contains(&admin_id));
        assert_eq!(loaded.ssh_target.to_string(), "127.0.0.1:2222");
    }
}
