//! Egui rendering for the herd-scout viewer (Wave 6 split).
//!
//! Lifecycle:
//!  1. `App::new` is called from `eframe::run_native`. It receives an
//!     [`IpcClientHandle`] that is already connected to the daemon.
//!  2. Until at least one preview frame has arrived, `update()` paints
//!     the **pairing screen**: a large QR of the daemon-supplied
//!     ticket, plus a collapsed "Advanced" expander.
//!  3. Once a frame arrives, `update()` paints the live JPEG, the CV
//!     overlay, the frame-age stamp, and the reconnect overlay when
//!     frames are stale or the connection drops.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use herd_scout_ipc::{ClientMsg, ConnectionStatus};

use crate::frame_view::FrameView;
use crate::ipc::client::{IpcClientHandle, SharedClientState};
use crate::overlay;
use crate::pairing;

const RECONNECT_STALE_AFTER: Duration = Duration::from_secs(2);

/// Small ASCII-art glyph rendered in the bottom HUD. Two-eyed cow head;
/// monospace, four lines tall. Kept short so the HUD stays compact even
/// on a 720p viewport.
const HUD_ASCII: &str = r#"   /^^^\
  ( o.o )
   > v <
   ^^ ^^"#;

pub struct App {
    state: Arc<SharedClientState>,
    handle: IpcClientHandle,
    frame_view: FrameView,
    /// QR texture, rebuilt when the ticket string changes.
    qr_texture: Option<egui::TextureHandle>,
    last_qr_for: Option<String>,
    /// Paste-box buffer + cached error.
    pairing_input: String,
    pairing_error: String,
    /// When the most recent detection batch arrived (for the "CV
    /// idle" hint after 2 s).
    last_det_at: Option<Instant>,
    /// Last seen frame pts to detect "first frame yet?".
    seen_first_frame: bool,
    egui_ctx: egui::Context,
    /// Total frames rendered into `frame_view` since startup. Bumped
    /// from `drain_frame` whenever a *new* frame is ingested.
    frame_count: u64,
    /// Total bytes of JPEG payload received since startup. Bumped from
    /// `drain_frame` so we don't have to mutate `SharedClientState`.
    bytes_rx: u64,
    /// `pts_ms` of the last frame we counted, so we don't double-count
    /// on repaints when no new frame has arrived.
    last_counted_pts_ms: Option<u64>,
    /// Wall-clock start of the GUI process, for the HUD uptime counter.
    start_instant: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, handle: IpcClientHandle) -> Self {
        Self {
            state: handle.state.clone(),
            handle,
            frame_view: FrameView::new("herd-scout-jpeg"),
            qr_texture: None,
            last_qr_for: None,
            pairing_input: String::new(),
            pairing_error: String::new(),
            last_det_at: None,
            seen_first_frame: false,
            egui_ctx: cc.egui_ctx.clone(),
            frame_count: 0,
            bytes_rx: 0,
            last_counted_pts_ms: None,
            start_instant: Instant::now(),
        }
    }

    fn sync_qr_from_ticket(&mut self) {
        let ticket = self.state.current_ticket.read().clone();
        if self.last_qr_for == ticket {
            return;
        }
        match ticket.as_deref() {
            Some(t) => match pairing::render_qr_image(t, 4) {
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
            },
            None => {
                self.qr_texture = None;
            }
        }
        self.last_qr_for = ticket;
    }

    fn drain_frame(&mut self, ctx: &egui::Context) {
        let snap = self.state.latest_frame.read().clone();
        if snap.jpeg.is_empty() {
            return;
        }
        let jpeg_len = snap.jpeg.len();
        let pts_ms = snap.pts_ms;
        match self.frame_view.ingest(ctx, &snap.jpeg, snap.pts_ms, snap.width, snap.height) {
            Ok(true) => {
                self.seen_first_frame = true;
                // Only count when ingest reports a new frame (returns
                // true). Belt-and-suspenders against monotonic-pts
                // reuse: also dedupe by last-seen pts_ms.
                if self.last_counted_pts_ms != Some(pts_ms) {
                    self.frame_count = self.frame_count.saturating_add(1);
                    self.bytes_rx = self.bytes_rx.saturating_add(jpeg_len as u64);
                    self.last_counted_pts_ms = Some(pts_ms);
                }
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!("JPEG ingest failed: {e:#}");
            }
        }
    }

    fn frame_age(&self) -> Option<Duration> {
        let g = self.state.latest_frame.read();
        g.received_at.map(|t| t.elapsed())
    }

    fn draw_pairing_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.heading("Scan this on your phone");
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Open herd-scout on your phone and point its camera at this QR.",
                )
                .color(egui::Color32::GRAY)
                .size(13.0),
            );
            ui.add_space(20.0);

            if let Some(tex) = self.qr_texture.as_ref() {
                ui.image((tex.id(), egui::vec2(256.0, 256.0)));
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("herd-scout is waiting for the phone to publish…")
                        .color(egui::Color32::GRAY)
                        .size(12.0),
                );
            } else {
                ui.add_space(64.0);
                ui.spinner();
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Waiting for the daemon to mint a pairing ticket…")
                        .color(egui::Color32::GRAY)
                        .size(13.0),
                );
                ui.add_space(64.0);
            }

            ui.add_space(20.0);

            ui.scope(|ui| {
                ui.set_max_width(640.0);
                egui::CollapsingHeader::new("Paste a ticket from another daemon instead")
                    .default_open(false)
                    .show(ui, |ui| {
                        self.draw_paste_box(ui);
                    });
            });

            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(
                    "Headless launches: set HERD_SCOUT_TICKET or pass --ticket on the daemon CLI.",
                )
                .color(egui::Color32::DARK_GRAY)
                .size(11.0),
            );
        });
    }

    fn draw_paste_box(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(
                "Useful when you have a ticket from another daemon and want to dial out.",
            )
            .color(egui::Color32::GRAY)
            .size(11.0),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Ticket:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.pairing_input)
                    .hint_text("iroh-live:…")
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::monospace(12.0)),
            );
            if resp.changed() {
                self.pairing_error = match pairing::validate_paste(&self.pairing_input) {
                    Ok(_) => String::new(),
                    Err(msg) => msg,
                };
            }
        });
        ui.add_space(6.0);
        if !self.pairing_error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.pairing_error);
            ui.add_space(6.0);
        }
        let parsed = pairing::validate_paste(&self.pairing_input).ok();
        let enabled = parsed.is_some();
        let button = egui::Button::new(
            egui::RichText::new("Connect")
                .size(14.0)
                .strong(),
        )
        .min_size(egui::vec2(120.0, 28.0));
        let clicked = ui.add_enabled(enabled, button).clicked();
        if clicked {
            if let Some(t) = parsed {
                self.handle.try_send(ClientMsg::ConnectTicket { ticket: t });
                self.pairing_input.clear();
                self.pairing_error.clear();
            }
        }
    }

    fn draw_reconnect_overlay(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        video_rect: egui::Rect,
        status: &ConnectionStatus,
    ) {
        let frame_age = self.frame_age();
        let stale = frame_age.map(|a| a >= RECONNECT_STALE_AFTER).unwrap_or(true);
        let unhealthy = !matches!(status, ConnectionStatus::Connected);
        if !(stale || unhealthy) {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(33));

        // Paint everything non-interactive into the central panel's
        // painter at video_rect. The interactive Cancel button is hoisted
        // into a foreground egui::Area below so its hit-rect lives on a
        // higher layer than the video Image and actually receives clicks.
        let painter = ui.painter_at(video_rect);
        painter.rect_filled(
            video_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 128),
        );

        let center = video_rect.center();
        let phase = ctx.input(|i| i.time);
        let dots = 8usize;
        let radius = 18.0;
        for i in 0..dots {
            let frac = i as f64 / dots as f64;
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

        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            "reconnecting…",
            egui::FontId::proportional(20.0),
            egui::Color32::from_rgba_unmultiplied(230, 230, 230, 230),
        );
        let sub = match (frame_age, status) {
            (Some(age), ConnectionStatus::Reconnecting { reason }) => {
                format!("{}s since last frame · {}", age.as_secs(), reason)
            }
            (Some(age), _) => format!("{}s since last frame", age.as_secs()),
            (None, ConnectionStatus::Reconnecting { reason }) => {
                format!("waiting for first frame · {reason}")
            }
            (None, _) => "waiting for first frame".to_string(),
        };
        painter.text(
            egui::pos2(center.x, center.y + 26.0),
            egui::Align2::CENTER_CENTER,
            sub,
            egui::FontId::proportional(13.0),
            egui::Color32::from_rgba_unmultiplied(200, 200, 200, 200),
        );

        // Issue 1: always-visible small QR thumbnail in the bottom-left
        // corner of the overlay so the user can re-pair without leaving
        // the screen. The texture is the same one used by the pairing
        // screen, just rendered smaller.
        if let Some(tex) = self.qr_texture.as_ref() {
            let thumb_size = 128.0_f32;
            let margin = 16.0_f32;
            let thumb_rect = egui::Rect::from_min_size(
                egui::pos2(
                    video_rect.left() + margin,
                    video_rect.bottom() - thumb_size - margin - 18.0,
                ),
                egui::vec2(thumb_size, thumb_size),
            );
            // White backing so dark QR modules stay readable on the dim
            // overlay.
            painter.rect_filled(
                thumb_rect.expand(4.0),
                4.0,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 235),
            );
            painter.image(
                tex.id(),
                thumb_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            painter.text(
                egui::pos2(thumb_rect.center().x, thumb_rect.bottom() + 4.0),
                egui::Align2::CENTER_TOP,
                "scan to re-pair",
                egui::FontId::proportional(11.0),
                egui::Color32::from_rgba_unmultiplied(230, 230, 230, 220),
            );
        }

        // Cancel button — hoisted into a foreground egui::Area so its
        // hit-rect lives in a layer ABOVE the central panel's video
        // Image. Without this, the Image (created earlier in the frame
        // with Sense::click via `add_sized`) swallows the click for the
        // entire video_rect, and the Cancel button's `interact` rect
        // never receives the press. Areas are hit-tested separately
        // and ordered above the central panel.
        //
        // The Area is anchored to the screen using a fixed_pos derived
        // from video_rect.center() + offset so it tracks the spinner.
        let cancel_pos = egui::pos2(center.x - 60.0, center.y + 50.0);
        egui::Area::new(egui::Id::new("herd-scout-reconnect-cancel-area"))
            .order(egui::Order::Foreground)
            .fixed_pos(cancel_pos)
            .interactable(true)
            .show(ctx, |ui| {
                let resp = ui.add(
                    egui::Button::new(
                        egui::RichText::new("Cancel")
                            .size(13.0)
                            .color(egui::Color32::from_rgba_unmultiplied(240, 240, 240, 240)),
                    )
                    .min_size(egui::vec2(120.0, 28.0))
                    .fill(egui::Color32::from_rgba_unmultiplied(50, 50, 50, 220))
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_unmultiplied(180, 180, 180, 220),
                    )),
                );
                if resp.clicked() {
                    tracing::info!("GUI: Cancel button clicked on reconnect overlay");
                    self.handle.try_send(ClientMsg::CancelStream);
                }
            });
    }

    fn draw_hud(&self, ui: &mut egui::Ui, status: &ConnectionStatus) {
        // Two-column layout: ASCII art + version on the left, live stats
        // on the right. Compact (<=100 px tall) so it doesn't crowd the
        // video viewport.
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(HUD_ASCII)
                        .font(egui::FontId::monospace(11.0))
                        .color(egui::Color32::from_rgb(210, 200, 160)),
                );
            });
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);

                // Row 1: brand + version
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("herd-scout")
                            .strong()
                            .size(13.0),
                    );
                    ui.label(
                        egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                            .color(egui::Color32::DARK_GRAY)
                            .size(11.0),
                    );
                    let daemon_ver = self.state.daemon_version.read().clone();
                    if let Some(dv) = daemon_ver {
                        ui.label(
                            egui::RichText::new(format!("· daemon {dv}"))
                                .color(egui::Color32::DARK_GRAY)
                                .size(11.0),
                        );
                    }
                });

                // Row 2: status + frame stats
                ui.horizontal(|ui| {
                    let (col, label) = status_chip(status);
                    ui.colored_label(col, format!("● {label}"));
                    let snap = self.state.latest_frame.read().clone();
                    let res_text = if snap.width > 0 && snap.height > 0 {
                        format!("{}x{}", snap.width, snap.height)
                    } else {
                        "—".to_string()
                    };
                    let age_text = self
                        .frame_age()
                        .map(|d| format!("age {} ms", d.as_millis()))
                        .unwrap_or_else(|| "age —".to_string());
                    ui.label(
                        egui::RichText::new(format!(
                            "Frame: {} · {} · {} frames",
                            res_text, age_text, self.frame_count
                        ))
                        .font(egui::FontId::monospace(11.0))
                        .color(egui::Color32::LIGHT_GRAY),
                    );
                });

                // Row 3: CV stats
                ui.horizontal(|ui| {
                    let dets = self.state.latest_dets.read().clone();
                    let cv_text = if dets.cv_disabled {
                        "CV: disabled".to_string()
                    } else {
                        format!(
                            "CV: {} cow · {} horse · {} sheep",
                            dets.counts.cow, dets.counts.horse, dets.counts.sheep
                        )
                    };
                    ui.label(
                        egui::RichText::new(cv_text)
                            .font(egui::FontId::monospace(11.0))
                            .color(egui::Color32::LIGHT_GRAY),
                    );
                });

                // Row 4: net + uptime
                ui.horizontal(|ui| {
                    let mb = (self.bytes_rx as f64) / (1024.0 * 1024.0);
                    let uptime = format_uptime(self.start_instant.elapsed());
                    ui.label(
                        egui::RichText::new(format!(
                            "Net: {:.1} MB rx · uptime {}",
                            mb, uptime
                        ))
                        .font(egui::FontId::monospace(11.0))
                        .color(egui::Color32::LIGHT_GRAY),
                    );
                });
            });
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(33));

        self.sync_qr_from_ticket();
        self.drain_frame(ctx);

        // If the daemon's apply_msg cleared `latest_frame` (e.g. user
        // pressed Cancel and the daemon transitioned back to Idle),
        // also reset our local "seen first frame" flag so the central
        // panel falls back to the pairing screen.
        if self.state.latest_frame.read().received_at.is_none() {
            self.seen_first_frame = false;
        }

        let status = self.state.status.read().clone();
        let dets_snap = self.state.latest_dets.read().clone();
        if !dets_snap.dets.is_empty() {
            self.last_det_at = Some(Instant::now());
        }

        // Bottom HUD: ASCII glyph + live stats. Drawn before the central
        // panel so the central panel's available_size() is computed
        // against the remaining viewport.
        egui::TopBottomPanel::bottom("hud")
            .min_height(82.0)
            .show(ctx, |ui| {
                self.draw_hud(ui, &status);
            });

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
                if *self.state.disconnected.read() {
                    ui.label(
                        egui::RichText::new("· daemon offline")
                            .color(egui::Color32::LIGHT_RED)
                            .small(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "herd-scout v{}",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .color(egui::Color32::DARK_GRAY)
                        .small(),
                    );
                });
            });
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(0.0))
            .show(ctx, |ui| {
                if !self.seen_first_frame {
                    self.draw_pairing_screen(ui);
                    return;
                }

                let avail = ui.available_size();
                let (video_rect, _resp) = if let Some(tex) = self.frame_view.texture() {
                    let resp = ui.add_sized(avail, egui::Image::from_texture(tex));
                    (resp.rect, resp)
                } else {
                    let (rect, resp) = ui.allocate_exact_size(avail, egui::Sense::hover());
                    (rect, resp)
                };

                overlay::draw(
                    ui,
                    video_rect,
                    &dets_snap.dets,
                    dets_snap.counts,
                    dets_snap.cv_banner.as_deref(),
                    dets_snap.cv_disabled,
                    self.last_det_at,
                );

                self.draw_reconnect_overlay(ctx, ui, video_rect, &status);

                let age_text = self
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

fn status_chip(status: &ConnectionStatus) -> (egui::Color32, &'static str) {
    match status {
        ConnectionStatus::Idle => (egui::Color32::GRAY, status.label()),
        ConnectionStatus::Connecting => (egui::Color32::YELLOW, status.label()),
        ConnectionStatus::Connected => (egui::Color32::GREEN, status.label()),
        ConnectionStatus::Reconnecting { .. } => (egui::Color32::ORANGE, status.label()),
        ConnectionStatus::Stopped => (egui::Color32::RED, status.label()),
    }
}

/// Format a duration as `HH:MM:SS` for the HUD uptime row.
fn format_uptime(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_uptime_zero() {
        assert_eq!(format_uptime(Duration::from_secs(0)), "00:00:00");
    }

    #[test]
    fn format_uptime_minutes_seconds() {
        assert_eq!(format_uptime(Duration::from_secs(125)), "00:02:05");
    }

    #[test]
    fn format_uptime_hours() {
        assert_eq!(
            format_uptime(Duration::from_secs(3 * 3600 + 7 * 60 + 42)),
            "03:07:42"
        );
    }

    #[test]
    fn hud_ascii_is_four_lines() {
        // Sanity-check the constant so a future edit doesn't blow out
        // the bottom panel height.
        assert_eq!(HUD_ASCII.lines().count(), 4);
    }
}
