//! Build script for the herd-scout desktop binary.
//!
//! Adds a Swift runtime rpath on macOS so the binary can load
//! `libswift_Concurrency.dylib`. This is required because iroh-live's
//! default features pull in `screencapturekit` (via `rusty-capture`),
//! which links against the Swift Concurrency runtime via `@rpath`.
//!
//! The vendored iroh-live workspace has the same rpath in its
//! `.cargo/config.toml`, but that config only applies inside the vendor
//! checkout. We replicate it here at the desktop-binary scope so this
//! crate links correctly when built from the herd-scout workspace.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // System Swift runtime location used by Apple's tooling.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
