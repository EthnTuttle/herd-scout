//! Daemon-side pairing helper: random broadcast-name generator.
//!
//! Wave 6 split: the QR-render side of pairing moved to the GUI; the
//! daemon only needs to mint a fresh name when it auto-mints a ticket
//! at boot.

use rand::TryRngCore;
use tracing::warn;

/// Generate a fresh broadcast name of the form `herd-scout-<8 hex chars>`.
///
/// 32 random bits is plenty of namespace for the MVP — collisions are
/// astronomically unlikely on a single device's lifetime, and the name
/// is only used as a string key inside the iroh-moq broadcast registry.
pub fn generate_broadcast_name() -> String {
    let mut buf = [0u8; 4];
    if rand::rng().try_fill_bytes(&mut buf).is_err() {
        warn!("OS RNG unavailable; falling back to time-derived broadcast name");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u32)
            .unwrap_or(0);
        buf.copy_from_slice(&now.to_be_bytes());
    }
    format!(
        "herd-scout-{:02x}{:02x}{:02x}{:02x}",
        buf[0], buf[1], buf[2], buf[3]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_name_has_expected_shape() {
        let name = generate_broadcast_name();
        assert!(name.starts_with("herd-scout-"));
        let suffix = &name["herd-scout-".len()..];
        assert_eq!(suffix.len(), 8);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn broadcast_names_are_unique_in_practice() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let n = generate_broadcast_name();
            assert!(seen.insert(n));
        }
    }
}
