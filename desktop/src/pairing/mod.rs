//! Wave 5A: QR pairing for the desktop subscriber.
//!
//! The Wave 1 boot path read a [`LiveTicket`] from `HERD_SCOUT_TICKET` /
//! `--ticket`. That works for headless / scripted launches but is hostile to
//! a "open the app on a Pi, hand the phone to the operator" flow. This
//! module replaces the env-var path with an interactive pairing screen:
//!
//!   1. The desktop opens with no ticket; egui shows a paste box and a QR
//!      placeholder.
//!   2. The user pastes a ticket string (read off the phone, copied to the
//!      clipboard, or in-the-future scanned). [`parse_pasted_ticket`] turns
//!      it into a [`LiveTicket`] and the App swaps in a fresh
//!      [`crate::stream::StreamHandle`] bound to it.
//!
//! The env-var / CLI path is **kept as a fallback** for headless launches —
//! see `main.rs::parse_ticket`. When set, `App` skips the pairing screen and
//! goes straight to streaming.
//!
//! ## QR rendering
//!
//! `qrcode` 0.14 with `default-features = false` gives us
//! [`qrcode::QrCode::to_colors`] → `Vec<Color>`. We walk that matrix into an
//! [`egui::ColorImage`] (one pixel per QR module, then upscaled by the egui
//! texture filter when drawn). No `image::DynamicImage` round-trip is
//! required, which keeps the dep graph small and avoids pulling in the
//! `image` crate's PNG/JPEG decoders just to render a couple hundred bool
//! cells.
//!
//! ## Direction-of-flow note
//!
//! In the herd-scout MVP the **phone is the publisher** and the **desktop is
//! the subscriber**. A correct pairing therefore starts with the phone
//! emitting a ticket containing its *own* `EndpointAddr` (see
//! `vendor/iroh-live/iroh-live/examples/publish.rs`) and the desktop dialing
//! that endpoint via [`Live::subscribe`]. Wave 2's JNI surface, however,
//! takes a ticket from the desktop and reuses only its `broadcast_name`
//! field — i.e. it expects the **desktop** to generate the ticket. That
//! cross-wiring is not Wave 5A's to fix (the JNI is read-only territory);
//! we expose **both** directions in the UI so whichever Wave 2 settles on
//! works:
//!
//!   * **Paste box** (this module): operator copies a ticket from the phone
//!     and pastes it on the desktop — fits the publish.rs pattern.
//!   * **QR display** (this module): renders any ticket string the desktop
//!     happens to hold — useful when the desktop is the ticket *generator*
//!     and the phone is the scanner (Wave 2's existing path).
//!
//! Either way, [`parse_pasted_ticket`] is the single source of truth for
//! turning a string into a validated [`LiveTicket`].

use std::str::FromStr;

use egui::ColorImage;
use iroh_live::ticket::LiveTicket;
use qrcode::{Color, QrCode};

/// Parses a pasted (or scanned) ticket string into a [`LiveTicket`].
///
/// Whitespace is trimmed; an empty input is rejected explicitly so the UI
/// can keep the "Connect" button disabled until something has been typed.
/// The error string is short and user-facing — it shows up inline below
/// the paste box.
pub fn parse_pasted_ticket(s: &str) -> Result<LiveTicket, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("Paste a ticket to connect".to_string());
    }
    LiveTicket::from_str(trimmed).map_err(|e| format!("Invalid ticket: {e}"))
}

/// Live-validates the current contents of the paste box.
///
/// Returns `Ok(ticket)` when the input parses, `Err("")` when the input is
/// empty (so we render no error chrome — the user hasn't typed anything),
/// and `Err(message)` for a parse failure (so we can render the message
/// inline in red).
pub fn validate_paste(s: &str) -> Result<LiveTicket, String> {
    if s.trim().is_empty() {
        return Err(String::new());
    }
    parse_pasted_ticket(s)
}

/// Render `text` as a QR code into an [`egui::ColorImage`].
///
/// One QR "module" maps to one image pixel; we pad with a `quiet_zone`-pixel
/// white border on every side because QR scanners reject codes without one.
/// Egui's nearest-neighbour upscale handles enlargement at paint time, so
/// the texture stays small (a typical iroh-live ticket fits in a 33x33-module
/// version-4 QR with `EcLevel::M`).
///
/// Returns `Err` if `text` is too long for any QR version (rare — iroh-live
/// tickets have a unit test asserting they fit in <2 KB).
pub fn render_qr_image(text: &str, quiet_zone: usize) -> Result<ColorImage, String> {
    let code = QrCode::new(text.as_bytes())
        .map_err(|e| format!("failed to encode ticket as QR: {e}"))?;
    let modules = code.width();
    let total = modules + 2 * quiet_zone;
    let cells = code.to_colors();

    // egui::ColorImage stores `Vec<egui::Color32>` row-major.
    let white = egui::Color32::WHITE;
    let black = egui::Color32::BLACK;
    let mut pixels = vec![white; total * total];

    for y in 0..modules {
        for x in 0..modules {
            let cell = cells[y * modules + x];
            if matches!(cell, Color::Dark) {
                let dst = (y + quiet_zone) * total + (x + quiet_zone);
                pixels[dst] = black;
            }
        }
    }

    Ok(ColorImage {
        size: [total, total],
        pixels,
        source_size: egui::vec2(total as f32, total as f32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_input() {
        assert!(parse_pasted_ticket("").is_err());
        assert!(parse_pasted_ticket("   \n  \t").is_err());
    }

    #[test]
    fn rejects_garbage() {
        let err = parse_pasted_ticket("not-a-ticket").unwrap_err();
        assert!(err.starts_with("Invalid ticket:"), "got: {err}");
    }

    #[test]
    fn validate_paste_empty_returns_empty_err() {
        // The paste-box validator deliberately returns Err("") for empty
        // input so the UI can suppress the error chrome.
        assert_eq!(validate_paste(""), Err(String::new()));
        assert_eq!(validate_paste("   "), Err(String::new()));
    }

    #[test]
    fn render_qr_produces_square_image_with_quiet_zone() {
        // Use a known short string; we just want to confirm the shape.
        let img = render_qr_image("iroh-live:test", 2).expect("render");
        assert_eq!(img.size[0], img.size[1], "QR images are square");
        assert!(img.size[0] >= 21 + 4, "must include quiet zone");
        // Corner pixels are quiet-zone white.
        assert_eq!(img.pixels[0], egui::Color32::WHITE);
    }
}
