//! IPC client for the GUI: connect to the daemon's UDS, run a
//! reader/writer task pair, and surface the latest state through cheap
//! `parking_lot::RwLock`s the egui paint loop can read every frame.
//!
//! Wire formats and message enums live in `herd_scout_ipc`.

pub mod client;
pub mod frame;
