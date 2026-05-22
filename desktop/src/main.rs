//! Herd-scout desktop viewer.
//!
//! Subscribes to a moq broadcast over iroh, decodes H.264, and renders
//! the live feed in egui. The publisher's address comes from a
//! [`LiveTicket`] resolved in this order:
//! 1. `--ticket <value>` (or `--ticket=<value>`) on the command line
//! 2. `HERD_SCOUT_TICKET` environment variable
//! 3. The last ticket saved to the on-disk prefs store (Wave 5B —
//!    written by 5A's pairing on-connect handler so a returning user
//!    auto-reconnects without re-pairing).
//! 4. **Wave 5C: auto-mint.** No env / CLI / saved ticket → the
//!    streaming task spins up its own iroh endpoint, mints a fresh
//!    [`LiveTicket`] under a randomly-generated broadcast name, surfaces
//!    it through `StreamHandle::current_ticket` so the UI renders a QR
//!    immediately, and saves it back to the store. The phone scans the
//!    QR, parses the ticket, and publishes under its `broadcast_name`.
//!    The paste-box pairing path (Wave 5A) is preserved as a hidden
//!    "Advanced" affordance for headless / debugging use.
//!
//! ```sh
//! HERD_SCOUT_TICKET="iroh-live:..." cargo run -p p2p-video-pipe-desktop
//! cargo run -p p2p-video-pipe-desktop -- --ticket "iroh-live:..."
//! cargo run -p p2p-video-pipe-desktop  # auto-mint + QR
//! ```

mod cv;
mod pairing;
mod store;
mod stream;
mod ui;

use std::str::FromStr;
use std::sync::Arc;

use iroh_live::ticket::LiveTicket;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const TICKET_ENV: &str = "HERD_SCOUT_TICKET";

fn main() -> eframe::Result<()> {
    init_tracing();

    let ticket = parse_ticket();

    // Build the tokio runtime *before* eframe so the streaming task can
    // spawn during `App::new`. Mirrors the pattern in the iroh-live
    // `split` and `viewer` examples.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    let _guard = rt.enter();

    // Open the prefs store eagerly so we can use it for both the
    // "load saved ticket" fallback and (Wave 5C) the auto-mint
    // save-on-mint hand-off. Failures are non-fatal — a corrupt or
    // unwritable store should never block app launch; the user simply
    // re-pairs / re-mints.
    let store = rt.block_on(async {
        match store::Store::open().await {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => {
                warn!("could not open prefs store: {e:#}");
                None
            }
        }
    });

    // Wave 5B: if no ticket from env/CLI, fall back to the last ticket
    // saved by the store (populated by 5A's pairing on-connect handler
    // and 5C's auto-mint save). When this also returns `None`, Wave 5C's
    // auto-mint path inside `stream::spawn` will produce a fresh ticket.
    let ticket = ticket.or_else(|| {
        let store = store.clone();
        rt.block_on(async {
            match store.as_deref() {
                Some(s) => s.load_last_ticket().await.unwrap_or(None),
                None => None,
            }
        })
    });

    match &ticket {
        Some(t) => info!(broadcast = %t.broadcast_name, "ticket loaded; will connect on launch"),
        None => info!(
            "no ticket available; auto-minting a fresh one — set {TICKET_ENV} or pass --ticket <ticket> to override",
        ),
    }

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("herd-scout"),
        ..Default::default()
    };

    eframe::run_native(
        "herd-scout",
        native_options,
        Box::new(move |cc| {
            // Always create a stream handle. With a `Some(ticket)` it
            // spawns the connect-decode-reconnect task immediately on
            // that ticket; with `None` it auto-mints (Wave 5C) on a
            // fresh iroh endpoint and surfaces the ticket back through
            // `StreamHandle::current_ticket()` for the UI to QR-render.
            let stream = stream::spawn(ticket.clone(), store.clone(), cc.egui_ctx.clone());
            // Wave 3: spin up the CV inference task on the same
            // tokio runtime. The shared `DetectionSnapshot` is read
            // from the egui paint loop and written from the CV task.
            // Even when no frames have arrived, the task is cheap to spawn —
            // it just observes a permanently-empty frame channel.
            let snapshot = cv::state::new_shared_snapshot();
            cv::spawn_cv_task(stream.frame_rx(), snapshot.clone(), cc.egui_ctx.clone());
            Ok(Box::new(ui::App::new(cc, stream, snapshot)))
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
