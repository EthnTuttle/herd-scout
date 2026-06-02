//! Hybrid Logical Clock embedded inside record value bytes.
//!
//! Per the Phase 0 audit (see crate-level docs), the live iroh-smol-kv
//! `SignedValue.timestamp` is wallclock nanos — not LWW-correct in the
//! field where devices drift hours. We therefore tag every record we
//! write with an HLC `(ts_ns, counter)` *inside* the value envelope
//! (see [`crate::model::RecordEnvelope`]). The on-disk wallclock-ish
//! `ts_ns` is preserved so a future migration to durable smol-kv can
//! replay every record through `WriteScope::put` losslessly.
//!
//! ## Algorithm
//!
//! Standard HLC (Kulkarni et al.):
//! - On read of a remote record, advance to `max(local, remote_hlc) +
//!   epsilon`.
//! - On local write, advance to `max(local, wallclock_ns).tick()`.
//! - Comparator is `(ts_ns, counter)` lexicographic; ties broken by
//!   author-id at the materialization layer.
//!
//! The `counter` is monotonic per `ts_ns`. When wallclock advances
//! past `ts_ns`, `counter` resets to 0.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// 16-byte HLC: 8 bytes wallclock-ish nanos, 4 bytes counter, 4 bytes
/// reserved (kept zero today). Stable on-disk shape across schema
/// versions.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Hlc {
    /// Nanos since UNIX epoch when the HLC was advanced.
    pub ts_ns: u64,
    /// Monotonic per-ts counter; resets to 0 when ts_ns advances.
    pub counter: u32,
}

impl Hlc {
    pub fn new(ts_ns: u64, counter: u32) -> Self {
        Self { ts_ns, counter }
    }

    /// Returns a new HLC with `counter += 1` (same `ts_ns`).
    /// Useful for stamping multiple keys inside one logical operation
    /// where the order matters but we don't want to take a wallclock
    /// reading per key.
    pub fn tick(self) -> Self {
        Self {
            ts_ns: self.ts_ns,
            counter: self.counter.saturating_add(1),
        }
    }

    /// 16-byte little-endian on-disk encoding. 4 bytes reserved at
    /// the tail are always zero today.
    pub fn to_bytes(self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..8].copy_from_slice(&self.ts_ns.to_le_bytes());
        out[8..12].copy_from_slice(&self.counter.to_le_bytes());
        out
    }

    /// Inverse of [`Self::to_bytes`].
    pub fn from_bytes(b: &[u8; 16]) -> Self {
        let mut ts = [0u8; 8];
        ts.copy_from_slice(&b[0..8]);
        let mut ctr = [0u8; 4];
        ctr.copy_from_slice(&b[8..12]);
        Self {
            ts_ns: u64::from_le_bytes(ts),
            counter: u32::from_le_bytes(ctr),
        }
    }
}

/// Persistent generator for [`Hlc`]. Owned by [`crate::store::Store`].
///
/// Concurrency: a single atomic packs `(ts_ns, counter)` into 64+32
/// bits. We use two atomics with a CAS-on-ts_ns loop to keep writes
/// lock-free.
#[derive(Debug)]
pub struct HlcGenerator {
    ts_ns: AtomicU64,
    counter: AtomicU64, // u32 used; AtomicU64 for cheap CAS
}

impl HlcGenerator {
    pub fn new(initial: Hlc) -> Self {
        Self {
            ts_ns: AtomicU64::new(initial.ts_ns),
            counter: AtomicU64::new(initial.counter as u64),
        }
    }

    /// Advances the clock (max(local, wallclock).tick()) and returns
    /// the new HLC.
    pub fn advance(&self) -> Hlc {
        let now = wallclock_ns();
        loop {
            let cur_ts = self.ts_ns.load(Ordering::Acquire);
            let cur_ctr = self.counter.load(Ordering::Acquire);
            let (new_ts, new_ctr) = if now > cur_ts {
                (now, 0u64)
            } else {
                (cur_ts, cur_ctr.saturating_add(1))
            };
            // CAS on ts_ns first to take ownership of the slot.
            if self
                .ts_ns
                .compare_exchange(cur_ts, new_ts, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.counter.store(new_ctr, Ordering::Release);
                return Hlc {
                    ts_ns: new_ts,
                    counter: u32::try_from(new_ctr).unwrap_or(u32::MAX),
                };
            }
            // CAS lost — retry. Spin is bounded by concurrent writers.
        }
    }

    /// Observes a remote HLC; advances the local clock to dominate
    /// `(remote, local).max() + epsilon`. Used when applying remote
    /// records replayed from disk or received from another peer.
    pub fn observe(&self, remote: Hlc) {
        loop {
            let cur_ts = self.ts_ns.load(Ordering::Acquire);
            let cur_ctr = self.counter.load(Ordering::Acquire);
            let now = wallclock_ns();
            let max_ts = cur_ts.max(remote.ts_ns).max(now);
            let new_ctr = if max_ts == cur_ts && max_ts == remote.ts_ns {
                cur_ctr.max(remote.counter as u64).saturating_add(1)
            } else if max_ts == cur_ts {
                cur_ctr.saturating_add(1)
            } else if max_ts == remote.ts_ns {
                (remote.counter as u64).saturating_add(1)
            } else {
                0
            };
            if self
                .ts_ns
                .compare_exchange(cur_ts, max_ts, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.counter.store(new_ctr, Ordering::Release);
                return;
            }
        }
    }

    /// Returns the current HLC without advancing. For diagnostics.
    pub fn current(&self) -> Hlc {
        Hlc {
            ts_ns: self.ts_ns.load(Ordering::Acquire),
            counter: u32::try_from(self.counter.load(Ordering::Acquire)).unwrap_or(u32::MAX),
        }
    }
}

fn wallclock_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_strictly_monotonic() {
        let g = HlcGenerator::new(Hlc::new(0, 0));
        let mut last = Hlc::new(0, 0);
        for _ in 0..1000 {
            let next = g.advance();
            assert!(next > last, "{next:?} > {last:?}");
            last = next;
        }
    }

    #[test]
    fn observe_dominates_remote() {
        let g = HlcGenerator::new(Hlc::new(100, 5));
        let remote = Hlc::new(1_000_000_000_000_000_000, 0);
        g.observe(remote);
        let next = g.advance();
        assert!(next > remote, "local must dominate after observe");
    }

    #[test]
    fn ordering_lexicographic() {
        let a = Hlc::new(10, 5);
        let b = Hlc::new(10, 6);
        let c = Hlc::new(11, 0);
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn bytes_round_trip() {
        let cases = [
            Hlc::new(0, 0),
            Hlc::new(1, 1),
            Hlc::new(u64::MAX, u32::MAX),
            Hlc::new(1_700_000_000_000_000_000, 42),
        ];
        for h in cases {
            let bytes = h.to_bytes();
            let parsed = Hlc::from_bytes(&bytes);
            assert_eq!(h, parsed);
        }
    }

    #[test]
    fn tick_increments_counter_keeps_ts() {
        let h = Hlc::new(10, 5);
        let t = h.tick();
        assert_eq!(t.ts_ns, 10);
        assert_eq!(t.counter, 6);
    }
}
