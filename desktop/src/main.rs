//! Herd-scout desktop viewer.
//!
//! Subscribes to a moq broadcast over iroh, decodes H.264, and renders
//! the live feed in egui. The publisher's address comes from a
//! [`LiveTicket`] passed via the `HERD_SCOUT_TICKET` environment variable
//! or via `--ticket <ticket>` on the command line. (Wave 5A will replace
//! this with QR-scan pairing.)
//!
//! ```sh
//! HERD_SCOUT_TICKET="iroh-live:..." cargo run -p p2p-video-pipe-desktop
//! cargo run -p p2p-video-pipe-desktop -- --ticket "iroh-live:..."
//! ```

mod stream;
mod ui;

use std::str::FromStr;

use iroh_live::ticket::LiveTicket;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const TICKET_ENV: &str = "HERD_SCOUT_TICKET";

fn main() -> eframe::Result<()> {
    init_tracing();

    let ticket = parse_ticket();
    match &ticket {
        Some(t) => info!(broadcast = %t.broadcast_name, "ticket loaded; will connect on launch"),
        None => warn!(
            "no ticket provided — set {TICKET_ENV} or pass --ticket <ticket>; UI will show placeholder"
        ),
    }

    // Build the tokio runtime *before* eframe so the streaming task can
    // spawn during `App::new`. Mirrors the pattern in the iroh-live
    // `split` and `viewer` examples.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let _guard = rt.enter();

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("herd-scout"),
        ..Default::default()
    };

    let has_ticket = ticket.is_some();
    eframe::run_native(
        "herd-scout",
        native_options,
        Box::new(move |cc| {
            let stream = stream::spawn(ticket, cc.egui_ctx.clone());
            Ok(Box::new(ui::App::new(cc, stream, has_ticket)))
        }),
    )
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,p2p_video_pipe_desktop=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Reads a ticket from `--ticket <value>` if present, otherwise from the
/// `HERD_SCOUT_TICKET` environment variable. Returns `None` if neither is
/// set or if parsing fails (the failure case logs a warning).
fn parse_ticket() -> Option<LiveTicket> {
    let raw = cli_ticket().or_else(|| std::env::var(TICKET_ENV).ok());
    let raw = raw?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    match LiveTicket::from_str(&raw) {
        Ok(t) => Some(t),
        Err(e) => {
            warn!("failed to parse ticket: {e}");
            None
        }
    }
}

/// Minimal hand-rolled `--ticket <value>` (or `--ticket=<value>`) parser
/// to avoid pulling in a CLI crate just for one flag.
fn cli_ticket() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(eq) = arg.strip_prefix("--ticket=") {
            return Some(eq.to_string());
        }
        if arg == "--ticket" {
            return args.next();
        }
    }
    None
}
