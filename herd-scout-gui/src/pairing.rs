//! GUI-side pairing helpers: QR rendering and paste-box validation.
//!
//! Wave 6 split: the daemon owns ticket minting (and parses
//! `LiveTicket` itself); the GUI handles only the *string-level*
//! sanity check on a pasted ticket so the "Connect" button can stay
//! disabled until something plausible has been typed. The full parse
//! happens daemon-side.

use egui::ColorImage;
use qrcode::{Color, QrCode};

/// Smallest plausible serialised ticket length. Real iroh-live tickets
/// run a few hundred bytes; this catches "user pasted nothing /
/// pasted random text" without depending on `LiveTicket`'s parser.
const MIN_TICKET_LEN: usize = 40;
const TICKET_PREFIX: &str = "iroh-live:";

/// Live-validate the contents of the paste box.
///
/// Empty input returns `Err(String::new())` so the UI can suppress
/// error chrome when the user hasn't typed anything yet.
pub fn validate_paste(s: &str) -> Result<String, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(String::new());
    }
    if !trimmed.starts_with(TICKET_PREFIX) {
        return Err(format!("Tickets start with '{TICKET_PREFIX}…'"));
    }
    if trimmed.len() < MIN_TICKET_LEN {
        return Err("Ticket looks too short to be valid".to_string());
    }
    Ok(trimmed.to_string())
}

/// Render `text` as a QR code into an `egui::ColorImage`, with a
/// configurable quiet zone (in modules / pixels).
pub fn render_qr_image(text: &str, quiet_zone: usize) -> Result<ColorImage, String> {
    let code = QrCode::new(text.as_bytes())
        .map_err(|e| format!("failed to encode ticket as QR: {e}"))?;
    let modules = code.width();
    let total = modules + 2 * quiet_zone;
    let cells = code.to_colors();

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
    fn rejects_empty() {
        assert_eq!(validate_paste(""), Err(String::new()));
        assert_eq!(validate_paste("   \n"), Err(String::new()));
    }

    #[test]
    fn rejects_garbage() {
        let err = validate_paste("not-a-ticket").unwrap_err();
        assert!(!err.is_empty(), "got: {err}");
    }

    #[test]
    fn accepts_plausible_prefix() {
        let s = format!("{}{}", TICKET_PREFIX, "x".repeat(MIN_TICKET_LEN));
        assert!(validate_paste(&s).is_ok());
    }

    #[test]
    fn render_qr_produces_square_image() {
        let img = render_qr_image("iroh-live:test", 2).unwrap();
        assert_eq!(img.size[0], img.size[1]);
        assert!(img.size[0] >= 21 + 4);
    }
}
