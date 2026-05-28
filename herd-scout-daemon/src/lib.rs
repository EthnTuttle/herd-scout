//! herd-scout-daemon library surface.
//!
//! The daemon binary (`src/main.rs`) and any `examples/` share these modules.
//! Public surface is intentionally narrow — only what's needed by examples
//! and integration tests.
//!
//! Note: Wave 13 promotes a handful of formerly-bin-private modules
//! (`audit`, `control`, `ipc`, `admin`) into the library so the
//! `upload` pipeline (which lives here too) can share them with the
//! daemon binary. Each module's types stay `pub(crate)` where
//! appropriate, so the published surface area is unchanged in practice
//! — only the path resolution moved.

pub mod cv;
pub mod upload;

pub mod audit;
pub mod control;
pub mod ipc;
pub mod admin;
