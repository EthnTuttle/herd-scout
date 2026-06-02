//! Domain types and materialization (record-set → entity) helpers.
//!
//! Materialization implements the per-field conflict strategies from
//! [[wiki/concepts/iroh-docs-fms-schema]] §"Conflict resolution":
//!
//! - LWW reads pick the record with the highest `(hlc, scope)` tuple.
//! - Add-wins-set reads compare each tag's add-records' max HLC
//!   against any tombstone HLC at `…/_deleted` and drop on tombstone-
//!   ≥-add.
//! - Append-only reads concatenate every record without merging.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::hlc::Hlc;

/// Pre-image-style 32-byte hash. Wraps the 32 bytes BLAKE3 produces;
/// kept separate from `blake3::Hash` so the on-disk encoding is stable
/// even if we swap hashers later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_blake3(h: &blake3::Hash) -> Self {
        Self(*h.as_bytes())
    }
}

impl From<blake3::Hash> for Hash {
    fn from(value: blake3::Hash) -> Self {
        Self(*value.as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    pub hash: Hash,
    pub mime: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Animal,
    Group,
    Land,
    Equipment,
}

impl AssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Animal => "animal",
            Self::Group => "group",
            Self::Land => "land",
            Self::Equipment => "equipment",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "animal" => Some(Self::Animal),
            "group" => Some(Self::Group),
            "land" => Some(Self::Land),
            "equipment" => Some(Self::Equipment),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    Observation,
    Medical,
    Movement,
    Weight,
    Birth,
}

impl LogKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Medical => "medical",
            Self::Movement => "movement",
            Self::Weight => "weight",
            Self::Birth => "birth",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "observation" => Some(Self::Observation),
            "medical" => Some(Self::Medical),
            "movement" => Some(Self::Movement),
            "weight" => Some(Self::Weight),
            "birth" => Some(Self::Birth),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub measure: String,
    pub value: f64,
    pub unit: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub id: Ulid,
    pub kind: AssetKind,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub parent: Option<Ulid>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub tags: Vec<Ulid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Log {
    pub id: Ulid,
    pub kind: LogKind,
    pub timestamp: Hlc,
    pub asset_refs: Vec<Ulid>,
    pub quantities: Vec<Quantity>,
    pub photos: Vec<BlobRef>,
    #[serde(default)]
    pub notes: String,
}

/// One on-disk record. Mirrors smol-kv's `(scope, key, SignedValue)`
/// shape plus the in-value HLC for correct LWW.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordEnvelope {
    /// Scope = author public-key tag, hex-encoded. One value per
    /// device; the daemon owns its own tag.
    pub scope: String,
    /// Key bytes, base64url-no-pad. (Stored as a string so the JSONL
    /// records.jsonl is debuggable.)
    pub key_b64: String,
    /// Value bytes, base64url-no-pad.
    pub value_b64: String,
    /// Wallclock-ish nanos. Mirrors smol-kv's `SignedValue.timestamp`
    /// for migration parity.
    pub ts_ns: u64,
    /// HLC `(ts_ns, counter)` controlling LWW. Bytes are
    /// little-endian inside `value_b64` for LWW-comparable fields,
    /// but we also stamp the same HLC here for fast filter scans.
    pub hlc: Hlc,
    /// LWW / AddWinsSet / AppendOnly. The store uses this to choose
    /// the projection rule when materializing.
    pub strategy: String,
}

/// Change event broadcast by the [`Store`](crate::store::Store) on
/// every successful commit. Carries enough information for a thin
/// projector (e.g. SQLite in Phase 3 of the plan) to update without
/// re-reading the whole store.
#[derive(Debug, Clone)]
pub struct ChangeEvent {
    pub scope: String,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub hlc: Hlc,
    pub strategy: ChangeStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStrategy {
    LastWriteWins,
    AddWinsSet,
    AppendOnly,
}

// ---------------------------------------------------------------------
// Materialization
// ---------------------------------------------------------------------

/// Building block: pick the LWW winner from a slice of records all
/// targeting the same key. Returns `None` if `records` is empty.
fn pick_lww(records: &[(String, Hlc, Vec<u8>)]) -> Option<&(String, Hlc, Vec<u8>)> {
    records
        .iter()
        .max_by(|a, b| match a.1.cmp(&b.1) {
            std::cmp::Ordering::Equal => a.0.cmp(&b.0),
            o => o,
        })
}

/// Extract `(hlc, value)` pairs from a record-list filtered by exact
/// key match.
fn pluck<'a>(
    records: &'a [(String, Vec<u8>, Hlc, Vec<u8>)],
    key: &[u8],
) -> Vec<(String, Hlc, Vec<u8>)> {
    records
        .iter()
        .filter(|(_scope, k, _hlc, _v)| k == key)
        .map(|(scope, _k, hlc, v)| (scope.clone(), *hlc, v.clone()))
        .collect()
}

/// Materializes a single asset from the records under its prefix.
///
/// `records` is `(scope, key, hlc, value)` tuples produced by
/// [`crate::store::Store::scan_prefix`].
pub fn materialize_asset(
    id: Ulid,
    records: &[(String, Vec<u8>, Hlc, Vec<u8>)],
) -> Option<Asset> {
    use crate::key;

    let kind_records = pluck(records, &key::Key::asset_kind(id));
    let kind = pick_lww(&kind_records)
        .and_then(|(_, _, v)| std::str::from_utf8(v).ok().and_then(AssetKind::parse))?;
    let name = pick_lww(&pluck(records, &key::Key::asset_name(id)))
        .and_then(|(_, _, v)| std::str::from_utf8(v).ok().map(str::to_string))
        .unwrap_or_default();
    let notes = pick_lww(&pluck(records, &key::Key::asset_notes(id)))
        .and_then(|(_, _, v)| std::str::from_utf8(v).ok().map(str::to_string))
        .unwrap_or_default();
    let parent = pick_lww(&pluck(records, &key::Key::asset_parent(id)))
        .and_then(|(_, _, v)| std::str::from_utf8(v).ok().and_then(|s| s.parse().ok()));
    let archived = pick_lww(&pluck(records, &key::Key::asset_archived(id)))
        .map(|(_, _, v)| v.as_slice() == b"true")
        .unwrap_or(false);

    let tags = materialize_tags(id, records);

    Some(Asset {
        id,
        kind,
        name,
        notes,
        parent,
        archived,
        tags,
    })
}

/// Add-wins-set materialization for tags. For each `term_id` seen,
/// compare max-add HLC vs any tombstone HLC and include the term iff
/// add-max ≥ tombstone (or there is no tombstone). Tombstone-and-
/// add-equal-HLC keeps the tag (add-wins on tie, per the wiki).
fn materialize_tags(
    asset_id: Ulid,
    records: &[(String, Vec<u8>, Hlc, Vec<u8>)],
) -> Vec<Ulid> {
    let prefix = format!("asset/{asset_id}/tag/");
    let mut adds: BTreeMap<Ulid, Hlc> = BTreeMap::new();
    let mut tombs: BTreeMap<Ulid, Hlc> = BTreeMap::new();

    for (_scope, key_bytes, hlc, _v) in records {
        let Ok(key_str) = std::str::from_utf8(key_bytes) else {
            continue;
        };
        if !key_str.starts_with(&prefix) {
            continue;
        }
        let tail = &key_str[prefix.len()..];
        if let Some(term_str) = tail.strip_suffix("/_deleted") {
            if let Ok(term) = term_str.parse::<Ulid>() {
                tombs
                    .entry(term)
                    .and_modify(|existing| {
                        if *hlc > *existing {
                            *existing = *hlc;
                        }
                    })
                    .or_insert(*hlc);
            }
        } else if let Ok(term) = tail.parse::<Ulid>() {
            adds.entry(term)
                .and_modify(|existing| {
                    if *hlc > *existing {
                        *existing = *hlc;
                    }
                })
                .or_insert(*hlc);
        }
    }

    adds.into_iter()
        .filter(|(term, add_hlc)| match tombs.get(term) {
            Some(tomb_hlc) => add_hlc >= tomb_hlc,
            None => true,
        })
        .map(|(term, _)| term)
        .collect()
}

/// Materializes every asset of the given kind from a flat record set.
/// Groups records by asset id (parsed from the key), then defers to
/// [`materialize_asset`] per group.
pub fn materialize_assets_by_kind(
    records: &[(String, Vec<u8>, Hlc, Vec<u8>)],
    kind: AssetKind,
    include_archived: bool,
) -> Vec<Asset> {
    let mut groups: BTreeMap<Ulid, Vec<(String, Vec<u8>, Hlc, Vec<u8>)>> = BTreeMap::new();
    for rec in records {
        if let Some(id) = parse_asset_id(&rec.1) {
            groups.entry(id).or_default().push(rec.clone());
        }
    }
    let mut out = Vec::new();
    for (id, recs) in groups {
        if let Some(asset) = materialize_asset(id, &recs) {
            if asset.kind == kind && (include_archived || !asset.archived) {
                out.push(asset);
            }
        }
    }
    out
}

/// Parses the ULID out of an `asset/<ULID>/...` key.
fn parse_asset_id(key: &[u8]) -> Option<Ulid> {
    let s = std::str::from_utf8(key).ok()?;
    let rest = s.strip_prefix("asset/")?;
    let id_str = rest.split('/').next()?;
    id_str.parse().ok()
}

/// Parses the ULID out of a `log/<ULID>/...` key.
fn parse_log_id(key: &[u8]) -> Option<Ulid> {
    let s = std::str::from_utf8(key).ok()?;
    let rest = s.strip_prefix("log/")?;
    let id_str = rest.split('/').next()?;
    id_str.parse().ok()
}

pub fn materialize_log(
    id: Ulid,
    records: &[(String, Vec<u8>, Hlc, Vec<u8>)],
) -> Option<Log> {
    use crate::key;

    let kind = pick_lww(&pluck(records, &key::Key::log_kind(id)))
        .and_then(|(_, _, v)| std::str::from_utf8(v).ok().and_then(LogKind::parse))?;
    let timestamp = pick_lww(&pluck(records, &key::Key::log_timestamp(id)))
        .and_then(|(_, _, v)| {
            if v.len() == 16 {
                let mut buf = [0u8; 16];
                buf.copy_from_slice(v);
                Some(Hlc::from_bytes(&buf))
            } else {
                None
            }
        })
        .unwrap_or_else(|| Hlc::new(0, 0));
    let notes = pick_lww(&pluck(records, &key::Key::log_notes(id)))
        .and_then(|(_, _, v)| std::str::from_utf8(v).ok().map(str::to_string))
        .unwrap_or_default();

    let asset_refs = materialize_log_asset_refs(id, records);
    let quantities = materialize_log_quantities(id, records);
    let photos = materialize_log_photos(id, records);

    Some(Log {
        id,
        kind,
        timestamp,
        asset_refs,
        quantities,
        photos,
        notes,
    })
}

fn materialize_log_asset_refs(
    log_id: Ulid,
    records: &[(String, Vec<u8>, Hlc, Vec<u8>)],
) -> Vec<Ulid> {
    let prefix = format!("log/{log_id}/asset_ref/");
    let mut refs: BTreeMap<Ulid, Hlc> = BTreeMap::new();
    for (_scope, key_bytes, hlc, _v) in records {
        let Ok(key_str) = std::str::from_utf8(key_bytes) else {
            continue;
        };
        if let Some(tail) = key_str.strip_prefix(&prefix) {
            if let Ok(term) = tail.parse::<Ulid>() {
                refs.entry(term)
                    .and_modify(|existing| {
                        if *hlc > *existing {
                            *existing = *hlc;
                        }
                    })
                    .or_insert(*hlc);
            }
        }
    }
    refs.into_keys().collect()
}

fn materialize_log_quantities(
    log_id: Ulid,
    records: &[(String, Vec<u8>, Hlc, Vec<u8>)],
) -> Vec<Quantity> {
    let prefix = format!("log/{log_id}/quantity/");
    // qid -> (measure, value, unit, label)
    let mut buckets: BTreeMap<u32, (Option<String>, Option<f64>, Option<String>, Option<String>)> =
        BTreeMap::new();

    for (_scope, key_bytes, _hlc, v) in records {
        let Ok(key_str) = std::str::from_utf8(key_bytes) else {
            continue;
        };
        let Some(tail) = key_str.strip_prefix(&prefix) else {
            continue;
        };
        let mut parts = tail.split('/');
        let Some(qid_str) = parts.next() else {
            continue;
        };
        let Ok(qid) = qid_str.parse::<u32>() else {
            continue;
        };
        let Some(field) = parts.next() else {
            continue;
        };
        let bucket = buckets.entry(qid).or_default();
        match field {
            "measure" => {
                if let Ok(s) = std::str::from_utf8(v) {
                    bucket.0 = Some(s.to_string());
                }
            }
            "value" => {
                if v.len() == 8 {
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(v);
                    bucket.1 = Some(f64::from_le_bytes(buf));
                }
            }
            "unit" => {
                if let Ok(s) = std::str::from_utf8(v) {
                    bucket.2 = Some(s.to_string());
                }
            }
            "label" => {
                if let Ok(s) = std::str::from_utf8(v) {
                    bucket.3 = Some(s.to_string());
                }
            }
            _ => {}
        }
    }

    buckets
        .into_iter()
        .filter_map(|(_qid, (measure, value, unit, label))| {
            Some(Quantity {
                measure: measure?,
                value: value?,
                unit: unit?,
                label,
            })
        })
        .collect()
}

fn materialize_log_photos(
    log_id: Ulid,
    records: &[(String, Vec<u8>, Hlc, Vec<u8>)],
) -> Vec<BlobRef> {
    let prefix = format!("log/{log_id}/photo/");
    // seq -> (hash, mime, size)
    let mut buckets: BTreeMap<u32, (Option<Hash>, Option<String>, Option<u64>)> = BTreeMap::new();

    for (_scope, key_bytes, _hlc, v) in records {
        let Ok(key_str) = std::str::from_utf8(key_bytes) else {
            continue;
        };
        let Some(tail) = key_str.strip_prefix(&prefix) else {
            continue;
        };
        let mut parts = tail.split('/');
        let Some(seq_str) = parts.next() else {
            continue;
        };
        let Ok(seq) = seq_str.parse::<u32>() else {
            continue;
        };
        let bucket = buckets.entry(seq).or_default();
        match parts.next() {
            None => {
                if v.len() == 32 {
                    let mut buf = [0u8; 32];
                    buf.copy_from_slice(v);
                    bucket.0 = Some(Hash(buf));
                }
            }
            Some("mime") => {
                if let Ok(s) = std::str::from_utf8(v) {
                    bucket.1 = Some(s.to_string());
                }
            }
            Some("size") => {
                if v.len() == 8 {
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(v);
                    bucket.2 = Some(u64::from_le_bytes(buf));
                }
            }
            _ => {}
        }
    }

    buckets
        .into_iter()
        .filter_map(|(_seq, (hash, mime, size))| {
            Some(BlobRef {
                hash: hash?,
                mime: mime?,
                size: size?,
            })
        })
        .collect()
}

pub fn materialize_logs_for_asset(
    records: &[(String, Vec<u8>, Hlc, Vec<u8>)],
    asset_id: Ulid,
) -> Vec<Log> {
    let mut groups: BTreeMap<Ulid, Vec<(String, Vec<u8>, Hlc, Vec<u8>)>> = BTreeMap::new();
    for rec in records {
        if let Some(id) = parse_log_id(&rec.1) {
            groups.entry(id).or_default().push(rec.clone());
        }
    }
    let mut out = Vec::new();
    for (id, recs) in groups {
        if let Some(log) = materialize_log(id, &recs) {
            if log.asset_refs.contains(&asset_id) {
                out.push(log);
            }
        }
    }
    out.sort_by_key(|l| l.timestamp);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(scope: &str, key: &str, hlc: Hlc, val: &[u8]) -> (String, Vec<u8>, Hlc, Vec<u8>) {
        (scope.to_string(), key.as_bytes().to_vec(), hlc, val.to_vec())
    }

    #[test]
    fn lww_picks_highest_hlc() {
        let id = Ulid::new();
        let recs = vec![
            rec(
                "a",
                &format!("asset/{id}/kind"),
                Hlc::new(1, 0),
                b"animal",
            ),
            rec(
                "a",
                &format!("asset/{id}/name"),
                Hlc::new(1, 0),
                b"old",
            ),
            rec(
                "a",
                &format!("asset/{id}/name"),
                Hlc::new(2, 0),
                b"new",
            ),
        ];
        let asset = materialize_asset(id, &recs).unwrap();
        assert_eq!(asset.name, "new");
    }

    #[test]
    fn add_wins_set_drops_on_tombstone() {
        let asset_id = Ulid::new();
        let term = Ulid::new();
        let recs = vec![
            rec(
                "a",
                &format!("asset/{asset_id}/kind"),
                Hlc::new(1, 0),
                b"animal",
            ),
            rec(
                "a",
                &format!("asset/{asset_id}/tag/{term}"),
                Hlc::new(1, 0),
                b"1",
            ),
            rec(
                "a",
                &format!("asset/{asset_id}/tag/{term}/_deleted"),
                Hlc::new(2, 0),
                b"1",
            ),
        ];
        let asset = materialize_asset(asset_id, &recs).unwrap();
        assert!(!asset.tags.contains(&term));
    }

    #[test]
    fn add_wins_on_tie() {
        let asset_id = Ulid::new();
        let term = Ulid::new();
        // Same HLC for tag and tombstone — wiki says add-wins on tie.
        let recs = vec![
            rec(
                "a",
                &format!("asset/{asset_id}/kind"),
                Hlc::new(1, 0),
                b"animal",
            ),
            rec(
                "a",
                &format!("asset/{asset_id}/tag/{term}"),
                Hlc::new(2, 0),
                b"1",
            ),
            rec(
                "a",
                &format!("asset/{asset_id}/tag/{term}/_deleted"),
                Hlc::new(2, 0),
                b"1",
            ),
        ];
        let asset = materialize_asset(asset_id, &recs).unwrap();
        assert!(asset.tags.contains(&term));
    }
}
