//! Egui rendering for the herd-scout desktop viewer.
//!
//! Lifecycle:
//! 1. `App::new` is called from inside `eframe::run_native`. It receives
//!    an idle (or pre-populated) [`StreamHandle`] from the streaming
//!    task and an optional initial [`LiveTicket`] resolved by `main.rs`.
//! 2. If no ticket was supplied, `update()` paints the **pairing
//!    screen** (Wave 5A): a paste box for the ticket string plus a QR
//!    of any current ticket so the phone can scan it back.
//! 3. Once a ticket is pasted (or already present), `update()` paints
//!    the live video, the CV overlay (Wave 3), the frame-age stamp,
//!    and the **reconnect overlay** (Wave 5A) when frames are stale or
//!    the connection is dropping.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use iroh_live::ticket::LiveTicket;
use moq_media_egui::FrameView;

use crate::cv::SharedSnapshot;
use crate::pairing;
use crate::stream::{self, ConnectionStatus, StreamHandle};

/// How stale the most recent frame must get before we draw the reconnect
/// overlay. 2s matches the CV "idle" threshold so the two overlays don't
/// flicker against each other on a healthy stream.
const RECONNECT_STALE_AFTER: Duration = Duration::from_secs(2);

/// Source frame dimensions — needed to project CV bounding boxes from
/// pixel space onto the egui rect that holds the video texture. We
/// remember the last frame's `(w, h)` so the overlay can keep using a
/// reasonable mapping even on a paint where `current_frame()` returned
/// `None`.
#[derive(Debug, Clone, Copy)]
struct FrameDims {
    w: f32,
    h: f32,
}

pub struct App {
    stream: StreamHandle,
    frame_view: FrameView,
    last_rendered_ts: Option<std::time::Duration>,
    /// Most recently observed source-frame dimensions, used for box
    /// projection. Updated in [`drain_frames`].
    last_frame_dims: Option<FrameDims>,
    /// Shared snapshot owned jointly with the CV inference task.
    snapshot: SharedSnapshot,
    /// The ticket the live stream is currently bound to, if any. When
    /// `None`, the pairing screen is shown. When `Some`, we keep a
    /// clone so the UI can render it back as a QR code (useful when
    /// the phone is the scanner side).
    current_ticket: Option<LiveTicket>,
    /// Live contents of the paste box on the pairing screen.
    pairing_input: String,
    /// Cached parse error for the paste box. Empty string means
    /// "no error / no input yet" so we draw no chrome.
    pairing_error: String,
    /// Cached QR texture for the current ticket. Re-rendered whenever
    /// `current_ticket` changes.
    qr_texture: Option<egui::TextureHandle>,
    /// Egui context kept around so [`Self::connect_with_ticket`] can
    /// hand it to a freshly-respawned streaming task. The streaming
    /// task uses it to call `request_repaint()` from the tokio
    /// thread when frames or status change.
    egui_ctx: egui::Context,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        stream: StreamHandle,
        snapshot: SharedSnapshot,
        ticket: Option<LiveTicket>,
    ) -> Self {
        let frame_view = FrameView::new(&cc.egui_ctx, "herd-scout-video");
        let mut app = Self {
            stream,
            frame_view,
            last_rendered_ts: None,
            last_frame_dims: None,
            snapshot,
            current_ticket: None, // set via `set_ticket` below so the QR cache is built
            pairing_input: String::new(),
            pairing_error: String::new(),
            qr_texture: None,
            egui_ctx: cc.egui_ctx.clone(),
        };
        if let Some(t) = ticket {
            app.set_ticket(t);
        }
        app
    }

    /// Updates the cached ticket and rebuilds the QR texture. Called
    /// from [`Self::new`] when a boot-time ticket was supplied and
    /// from [`Self::connect_with_ticket`] after a successful pairing.
    fn set_ticket(&mut self, ticket: LiveTicket) {
        // Rebuild the QR texture. `render_qr_image` returns Err only
        // for absurdly long inputs (>4 KB-ish); a `LiveTicket` has a
        // unit test asserting it fits, so this should never fail in
        // practice. Still: log and degrade silently rather than
        // panic — the paste box fallback keeps working without a QR.
        let serialized = ticket.to_string();
        match pairing::render_qr_image(&serialized, 4) {
            Ok(img) => {
                let handle = self.egui_ctx.load_texture(
                    "herd-scout-pairing-qr",
                    img,
                    egui::TextureOptions::NEAREST,
                );
                self.qr_texture = Some(handle);
            }
            Err(e) => {
                tracing::warn!("could not render ticket QR: {e}");
                self.qr_texture = None;
            }
        }
        self.current_ticket = Some(ticket);
    }

    /// Respawn the streaming task with a freshly-pasted ticket and
    /// transition the UI out of the pairing screen.
    ///
    /// Called from the pairing-screen "Connect" button. The previous
    /// stream task (if any) was either idle (`AwaitingTicket`) or
    /// already long-lived; in either case we just drop the old
    /// handle. Tokio's watch channels are reference-counted, so the
    /// CV inference task keeps observing the now-orphaned channel
    /// (it'll see no further frames there but its receiver stays
    /// open). For Wave 5A this means the **first** successful pairing
    /// is what the CV task actually consumes from; re-pairs after
    /// that get reflected in the UI but the CV stream stays bound to
    /// the original receiver. Acceptable for MVP — re-pairing is
    /// expected to be rare and the user can restart the app to
    /// rebind the CV task.
    fn connect_with_ticket(&mut self, ticket: LiveTicket) {
        tracing::info!(
            broadcast = %ticket.broadcast_name,
            "re-spawning stream task with paired ticket"
        );
        self.stream = stream::spawn(Some(ticket.clone()), self.egui_ctx.clone());
        self.last_rendered_ts = None; // force a fresh first-frame paint
        self.last_frame_dims = None;
        self.set_ticket(ticket.clone());
        self.pairing_input.clear();
        self.pairing_error.clear();

        // Wave 5B hand-off: persist the ticket so the next launch
        // skips the pairing screen. Errors are non-fatal — a
        // read-only store should never block the user from
        // streaming.
        let ticket_for_save = ticket;
        tokio::spawn(async move {
            match crate::store::Store::open().await {
                Ok(store) => {
                    if let Err(e) = store.save_ticket(&ticket_for_save).await {
                        tracing::warn!("failed to persist ticket: {e:#}");
                    }
                }
                Err(e) => tracing::warn!("could not open prefs store for save: {e:#}"),
            }
        });
    }

    /// Pulls the latest frame from the stream channel and uploads it to the
    /// egui texture if it's newer than what we last rendered.
    fn drain_frames(&mut self) {
        if let Some(frame) = self.stream.current_frame() {
            let ts = frame.timestamp;
            if self.last_rendered_ts != Some(ts) {
                self.frame_view.render_frame(&frame);
                self.last_rendered_ts = Some(ts);
            }
            // Track the source dimensions so the CV overlay can scale
            // detection boxes from frame-pixel space to screen space.
            self.last_frame_dims = Some(FrameDims {
                w: frame.width() as f32,
                h: frame.height() as f32,
            });
            // Drop the Arc to release our reference; the watch channel
            // still holds a copy for the CV inference task.
            drop(frame as Arc<_>);
        }
    }

    /// Paint the CV detection overlay (per-class boxes, labels, count
    /// panel, and the optional "CV idle"/disabled banner) on top of
    /// the just-rendered video texture.
    ///
    /// The egui paint loop is on the UI thread, so we take a synchronous
    /// `parking_lot::RwLock` read here. There is at most one writer (the
    /// CV inference task) and contention is negligible.
    fn draw_cv_overlay(&self, ui: &egui::Ui, video_rect: egui::Rect) {
        let snap = self.snapshot.read();

        // CV-disabled / shape-mismatch banner: drawn even before any
        // frames arrive so the user knows CV is off.
        if let Some(banner) = snap.banner.as_deref() {
            let painter = ui.painter();
            let bg = egui::Color32::from_rgba_unmultiplied(120, 0, 0, 180);
            let banner_rect = egui::Rect::from_min_size(
                video_rect.left_top(),
                egui::vec2(video_rect.width(), 22.0),
            );
            painter.rect_filled(banner_rect, 0.0, bg);
            painter.text(
                banner_rect.center(),
                egui::Align2::CENTER_CENTER,
                banner,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
        }

        if snap.disabled {
            return;
        }

        let Some(dims) = self.last_frame_dims else {
            // No frame painted yet → nothing to project onto.
            return;
        };
        if dims.w <= 0.0 || dims.h <= 0.0 {
            return;
        }

        let painter = ui.painter();
        let scale_x = video_rect.width() / dims.w;
        let scale_y = video_rect.height() / dims.h;
        let origin = video_rect.left_top();

        // Detection boxes + per-detection score labels.
        for det in &snap.detections {
            let (r, g, b) = det.class.rgb();
            let stroke_color = egui::Color32::from_rgb(r, g, b);
            let x1 = origin.x + det.bbox[0] * scale_x;
            let y1 = origin.y + det.bbox[1] * scale_y;
            let x2 = origin.x + det.bbox[2] * scale_x;
            let y2 = origin.y + det.bbox[3] * scale_y;
            let rect = egui::Rect::from_min_max(egui::pos2(x1, y1), egui::pos2(x2, y2));
            painter.rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(2.0, stroke_color),
                egui::StrokeKind::Outside,
            );
            let label = format!("{} {:.2}", det.class.label(), det.score);
            // Label background sits just above the box top edge.
            let label_pos = egui::pos2(x1, (y1 - 4.0).max(origin.y));
            painter.text(
                label_pos,
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::FontId::proportional(12.0),
                stroke_color,
            );
        }

        // Top-right counts panel.
        let counts = snap.rolling_counts();
        let counts_text = format!(
            "Cows: {}  Horses: {}  Sheep: {}",
            counts.cow, counts.horse, counts.sheep
        );
        let panel_pos = video_rect.right_top() + egui::vec2(-8.0, 8.0);
        // Filled background for legibility over bright pasture.
        let bg_size = egui::vec2(220.0, 22.0);
        let bg_rect = egui::Rect::from_min_size(panel_pos - egui::vec2(bg_size.x, 0.0), bg_size);
        painter.rect_filled(
            bg_rect,
            4.0,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
        );
        painter.text(
            bg_rect.center(),
            egui::Align2::CENTER_CENTER,
            counts_text,
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );

        // "CV idle" hint when the snapshot is older than 2 s.
        if snap.is_idle(Instant::now(), Duration::from_secs(2)) {
            let pos = video_rect.left_top() + egui::vec2(8.0, 8.0);
            painter.text(
                pos,
                egui::Align2::LEFT_TOP,
                "CV idle",
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 160),
            );
        }
    }

    /// Wave 5A: the "Pair with phone" screen.
    ///
    /// Drawn in the central panel when no ticket is bound yet. Shows a
    /// paste box, a Connect button (gated on a successful parse), and
    /// — when a ticket *is* bound but no frames have arrived yet — a
    /// QR rendering of the current ticket so the phone can scan it
    /// back. The QR display is also useful as a sanity check while
    /// pairing: the user can confirm the desktop has the same ticket
    /// the phone displays.
    fn draw_pairing_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.heading("Pair with phone");
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Open herd-scout on your phone, copy its ticket, and paste it below.",
                )
                .color(egui::Color32::GRAY)
                .size(13.0),
            );
            ui.add_space(20.0);

            // Optional QR of the *currently bound* ticket. Useful when
            // the desktop is the ticket-generator side (so the phone
            // scans this) — see the directional note in
            // `pairing/mod.rs`. Hidden in the no-ticket case.
            if let Some(tex) = self.qr_texture.as_ref() {
                ui.label(
                    egui::RichText::new("Or scan this from the phone:")
                        .color(egui::Color32::GRAY)
                        .size(12.0),
                );
                ui.add_space(4.0);
                ui.image((tex.id(), egui::vec2(256.0, 256.0)));
                ui.add_space(12.0);
            }

            // The paste box itself. We want a wide single-line text
            // input — `singleline=true` rejects newlines so paste of
            // a copied-with-trailing-newline ticket still parses.
            ui.scope(|ui| {
                ui.set_max_width(640.0);
                ui.horizontal(|ui| {
                    ui.label("Ticket:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.pairing_input)
                            .hint_text("iroh-live:…")
                            .desired_width(f32::INFINITY)
                            .font(egui::FontId::monospace(12.0)),
                    );
                    if resp.changed() {
                        // Re-validate on every keystroke so the
                        // Connect button stays in sync.
                        self.pairing_error = match pairing::validate_paste(&self.pairing_input) {
                            Ok(_) => String::new(),
                            Err(msg) => msg,
                        };
                    }
                });
            });

            ui.add_space(8.0);

            // Inline error message (only when we have one).
            if !self.pairing_error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &self.pairing_error);
                ui.add_space(8.0);
            }

            // Connect button. Enabled only on a clean parse of the
            // current paste-box contents.
            let parsed = pairing::validate_paste(&self.pairing_input).ok();
            let enabled = parsed.is_some();
            let button = egui::Button::new(
                egui::RichText::new("Connect")
                    .size(15.0)
                    .strong(),
            )
            .min_size(egui::vec2(140.0, 32.0));
            let clicked = ui.add_enabled(enabled, button).clicked();
            if clicked {
                if let Some(t) = parsed {
                    self.connect_with_ticket(t);
                }
            }

            ui.add_space(24.0);
            ui.label(
                egui::RichText::new(
                    "Headless launches: set HERD_SCOUT_TICKET or pass --ticket on the CLI.",
                )
                .color(egui::Color32::DARK_GRAY)
                .size(11.0),
            );
        });
    }

    /// Wave 5A: translucent "reconnecting…" overlay drawn on top of the
    /// stale video frame whenever frames are older than
    /// [`RECONNECT_STALE_AFTER`] or the streaming task reports a
    /// non-`Connected` state.
    ///
    /// The overlay is drawn into the rect that holds the video so it
    /// covers exactly the stale image and not the surrounding chrome.
    /// We intentionally do **not** clear the video texture: keeping
    /// the last frame visible underneath the dim layer signals
    /// "frozen" rather than "off".
    fn draw_reconnect_overlay(
        &self,
        ctx: &egui::Context,
        ui: &egui::Ui,
        video_rect: egui::Rect,
        status: &ConnectionStatus,
    ) {
        let frame_age = self.stream.frame_age();
        let stale = frame_age
            .map(|age| age >= RECONNECT_STALE_AFTER)
            .unwrap_or(true); // no frame yet → show overlay
        let unhealthy = !matches!(status, ConnectionStatus::Connected);

        if !(stale || unhealthy) {
            return;
        }
        // While the overlay is up we want the spinner to animate, so
        // ensure egui repaints at ~30 Hz independent of the streaming
        // task.
        ctx.request_repaint_after(Duration::from_millis(33));

        let painter = ui.painter_at(video_rect);

        // Dim layer over the video. ~50% black via premultiplied
        // alpha so the underlying frame is visibly the "frozen"
        // last picture without disappearing entirely.
        painter.rect_filled(
            video_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 128),
        );

        // Center pivot for the text + spinner stack.
        let center = video_rect.center();

        // Spinner: a ring of 8 dots fading in a rotating phase.
        // Tied to wallclock so frame rate doesn't matter.
        let phase = ctx.input(|i| i.time);
        let dots = 8usize;
        let radius = 18.0;
        for i in 0..dots {
            let frac = i as f64 / dots as f64;
            // Each dot's alpha follows a sine wave offset by its
            // position around the ring; the wave moves at ~1 rev/s.
            let alpha_f = 0.5 + 0.5 * (2.0 * std::f64::consts::PI * (phase - frac)).cos();
            let alpha = (alpha_f.clamp(0.0, 1.0) * 200.0 + 30.0) as u8;
            let angle = 2.0 * std::f64::consts::PI * frac;
            let dx = radius * (angle.cos() as f32);
            let dy = radius * (angle.sin() as f32);
            let dot_center = egui::pos2(center.x + dx, center.y - 36.0 + dy);
            painter.circle_filled(
                dot_center,
                3.0,
                egui::Color32::from_rgba_unmultiplied(220, 220, 220, alpha),
            );
        }

        // Headline.
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            "reconnecting…",
            egui::FontId::proportional(20.0),
            egui::Color32::from_rgba_unmultiplied(230, 230, 230, 230),
        );

        // Sub-line: elapsed-disconnect-time counter, plus reason if
        // the streaming task surfaced one.
        let sub = match (frame_age, status) {
            (Some(age), ConnectionStatus::Reconnecting { reason }) => {
                format!("{}s since last frame · {}", age.as_secs(), reason)
            }
            (Some(age), _) => {
                format!("{}s since last frame", age.as_secs())
            }
            (None, ConnectionStatus::Reconnecting { reason }) => {
                format!("waiting for first frame · {reason}")
            }
            (None, _) => "waiting for first frame".to_string(),
        };
        let sub_pos = egui::pos2(center.x, center.y + 26.0);
        painter.text(
            sub_pos,
            egui::Align2::CENTER_CENTER,
            sub,
            egui::FontId::proportional(13.0),
            egui::Color32::from_rgba_unmultiplied(200, 200, 200, 200),
        );
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Repaint at ~60 Hz so frame age + status stay fresh; the streaming
        // task also calls `ctx.request_repaint()` on every frame arrival to
        // keep latency low under load.
        ctx.request_repaint_after(Duration::from_millis(16));

        self.drain_frames();

        let status = self.stream.status();

        egui::TopBottomPanel::top("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (color, label) = status_chip(&status);
                ui.colored_label(color, format!("● {label}"));
                if let ConnectionStatus::Reconnecting { reason } = &status {
                    ui.label(
                        egui::RichText::new(format!("({reason})"))
                            .color(egui::Color32::DARK_GRAY)
                            .small(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("herd-scout")
                            .color(egui::Color32::DARK_GRAY)
                            .small(),
                    );
                });
            });
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(0.0))
            .show(ctx, |ui| {
                // No ticket bound yet → pairing screen.
                if self.current_ticket.is_none() {
                    self.draw_pairing_screen(ui);
                    return;
                }

                if self.last_rendered_ts.is_none() {
                    placeholder(
                        ui,
                        match &status {
                            ConnectionStatus::Connected => "Connected — waiting for first frame…",
                            ConnectionStatus::Connecting => "Connecting to publisher…",
                            ConnectionStatus::Reconnecting { .. } => {
                                "Lost connection — reconnecting…"
                            }
                            ConnectionStatus::AwaitingTicket => "Waiting for ticket…",
                            ConnectionStatus::Stopped => "Stream stopped.",
                        },
                    );
                    return;
                }

                let avail = ui.available_size();
                let img = self.frame_view.image();
                let img_response = ui.add_sized(avail, img);
                // The rect occupied by the just-painted video texture.
                // CV overlay coordinates are projected into this rect.
                let video_rect = img_response.rect;

                // === Wave 3: CV detection overlay ===
                self.draw_cv_overlay(ui, video_rect);

                // === Wave 5A: reconnect overlay ===
                // Drawn last so it sits on top of both the video and
                // the CV overlay when the stream is stale.
                self.draw_reconnect_overlay(ctx, ui, video_rect, &status);

                // Frame-age overlay anchored bottom-right of the central panel.
                let age_text = self
                    .stream
                    .frame_age()
                    .map(|d| format!("frame age: {} ms", d.as_millis()))
                    .unwrap_or_else(|| "frame age: —".to_string());

                let painter = ui.painter();
                let rect = ui.max_rect();
                let pos = rect.right_bottom() - egui::vec2(8.0, 8.0);
                painter.text(
                    pos,
                    egui::Align2::RIGHT_BOTTOM,
                    age_text,
                    egui::FontId::monospace(12.0),
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 220),
                );
            });
    }
}

fn placeholder(ui: &mut egui::Ui, msg: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(egui::RichText::new(msg).color(egui::Color32::GRAY).size(16.0));
    });
}

fn status_chip(status: &ConnectionStatus) -> (egui::Color32, &'static str) {
    match status {
        ConnectionStatus::AwaitingTicket => (egui::Color32::GRAY, status.label()),
        ConnectionStatus::Connecting => (egui::Color32::YELLOW, status.label()),
        ConnectionStatus::Connected => (egui::Color32::GREEN, status.label()),
        ConnectionStatus::Reconnecting { .. } => (egui::Color32::ORANGE, status.label()),
        ConnectionStatus::Stopped => (egui::Color32::RED, status.label()),
    }
}
