//! Key-layout helpers. Produces the `bytes` keys used throughout the
//! crate.
//!
//! Mirrors [[wiki/concepts/iroh-docs-fms-schema]] §"Key layout":
//!
//! ```text
//! asset/<ulid>/kind                    → "animal" | ...
//! asset/<ulid>/name                    → utf-8
//! asset/<ulid>/notes                   → utf-8
//! asset/<ulid>/geom                    → CBOR (deferred — empty in v1)
//! asset/<ulid>/parent                  → ULID utf-8
//! asset/<ulid>/archived                → "true"
//! asset/<ulid>/_schema                 → "1"
//! asset/<ulid>/tag/<term-ulid>         → "1"  (add-wins-set)
//! asset/<ulid>/tag/<term-ulid>/_deleted → "1" (tombstone)
//!
//! log/<ulid>/kind                      → "observation" | ...
//! log/<ulid>/timestamp                 → 16-byte HLC
//! log/<ulid>/notes                     → utf-8
//! log/<ulid>/asset_ref/<asset-ulid>    → "1"  (add-wins-set)
//! log/<ulid>/quantity/<qid>/measure    → utf-8
//! log/<ulid>/quantity/<qid>/value      → f64 LE
//! log/<ulid>/quantity/<qid>/unit       → utf-8
//! log/<ulid>/quantity/<qid>/label      → utf-8
//! log/<ulid>/photo/<seq>               → BLAKE3 (32 bytes)
//! log/<ulid>/photo/<seq>/mime          → utf-8
//! log/<ulid>/photo/<seq>/size          → u64 LE
//! ```
//!
//! ULIDs are formatted as Crockford base32 (the standard 26-char form
//! `ulid::Ulid::to_string`). All keys are valid UTF-8 — no byte-level
//! tricks — keeping the JSONL records.jsonl debuggable.

use ulid::Ulid;

/// Builders for the byte-key layout. Every function returns `Vec<u8>`.
/// Callers reach for these via `crate::key::Key::asset_name(id)` (or
/// the re-export `crate::Key::...`).
#[allow(non_snake_case)]
pub mod Key {
    use super::*;

    pub fn asset_kind(id: Ulid) -> Vec<u8> {
        format!("asset/{id}/kind").into_bytes()
    }
    pub fn asset_name(id: Ulid) -> Vec<u8> {
        format!("asset/{id}/name").into_bytes()
    }
    pub fn asset_notes(id: Ulid) -> Vec<u8> {
        format!("asset/{id}/notes").into_bytes()
    }
    pub fn asset_geom(id: Ulid) -> Vec<u8> {
        format!("asset/{id}/geom").into_bytes()
    }
    pub fn asset_parent(id: Ulid) -> Vec<u8> {
        format!("asset/{id}/parent").into_bytes()
    }
    pub fn asset_archived(id: Ulid) -> Vec<u8> {
        format!("asset/{id}/archived").into_bytes()
    }
    pub fn asset_schema(id: Ulid) -> Vec<u8> {
        format!("asset/{id}/_schema").into_bytes()
    }
    pub fn asset_tag(asset_id: Ulid, term_id: Ulid) -> Vec<u8> {
        format!("asset/{asset_id}/tag/{term_id}").into_bytes()
    }
    pub fn asset_tag_tombstone(asset_id: Ulid, term_id: Ulid) -> Vec<u8> {
        format!("asset/{asset_id}/tag/{term_id}/_deleted").into_bytes()
    }

    pub fn log_kind(id: Ulid) -> Vec<u8> {
        format!("log/{id}/kind").into_bytes()
    }
    pub fn log_timestamp(id: Ulid) -> Vec<u8> {
        format!("log/{id}/timestamp").into_bytes()
    }
    pub fn log_notes(id: Ulid) -> Vec<u8> {
        format!("log/{id}/notes").into_bytes()
    }
    pub fn log_asset_ref(log_id: Ulid, asset_id: Ulid) -> Vec<u8> {
        format!("log/{log_id}/asset_ref/{asset_id}").into_bytes()
    }
    pub fn log_quantity_measure(log_id: Ulid, qid: u32) -> Vec<u8> {
        format!("log/{log_id}/quantity/{qid}/measure").into_bytes()
    }
    pub fn log_quantity_value(log_id: Ulid, qid: u32) -> Vec<u8> {
        format!("log/{log_id}/quantity/{qid}/value").into_bytes()
    }
    pub fn log_quantity_unit(log_id: Ulid, qid: u32) -> Vec<u8> {
        format!("log/{log_id}/quantity/{qid}/unit").into_bytes()
    }
    pub fn log_quantity_label(log_id: Ulid, qid: u32) -> Vec<u8> {
        format!("log/{log_id}/quantity/{qid}/label").into_bytes()
    }
    pub fn log_photo(log_id: Ulid, seq: u32) -> Vec<u8> {
        format!("log/{log_id}/photo/{seq:04}").into_bytes()
    }
    pub fn log_photo_mime(log_id: Ulid, seq: u32) -> Vec<u8> {
        format!("log/{log_id}/photo/{seq:04}/mime").into_bytes()
    }
    pub fn log_photo_size(log_id: Ulid, seq: u32) -> Vec<u8> {
        format!("log/{log_id}/photo/{seq:04}/size").into_bytes()
    }
}

pub fn asset_prefix(id: Ulid) -> Vec<u8> {
    format!("asset/{id}/").into_bytes()
}

pub fn log_prefix(id: Ulid) -> Vec<u8> {
    format!("log/{id}/").into_bytes()
}

pub fn asset_kind_str(kind: &[u8]) -> Option<&str> {
    std::str::from_utf8(kind).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_keys_roundtrip() {
        let id = Ulid::new();
        let k = Key::asset_kind(id);
        let s = std::str::from_utf8(&k).unwrap();
        assert!(s.starts_with("asset/"));
        assert!(s.ends_with("/kind"));
        assert!(s.contains(&id.to_string()));
    }

    #[test]
    fn photo_keys_zero_padded() {
        let log = Ulid::new();
        let k = Key::log_photo(log, 1);
        let s = std::str::from_utf8(&k).unwrap();
        assert!(s.ends_with("/photo/0001"), "got {s}");
    }

    #[test]
    fn log_prefix_matches_log_keys() {
        let id = Ulid::new();
        let prefix = log_prefix(id);
        let kind_key = Key::log_kind(id);
        assert!(kind_key.starts_with(&prefix));
    }
}
