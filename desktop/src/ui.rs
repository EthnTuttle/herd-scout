//! Egui rendering for the herd-scout desktop viewer.
//!
//! Lifecycle:
//! 1. `App::new` is called from inside `eframe::run_native`. It spins up
//!    the background streaming task (via [`crate::stream::spawn`]) and
//!    creates a [`FrameView`] from the egui context.
//! 2. On every `update()` call, we poll the stream's frame channel; if a
//!    new frame arrived since last paint, we hand it to the `FrameView`
//!    for texture upload and request a near-immediate repaint.

use std::sync::Arc;

use eframe::egui;
use moq_media_egui::FrameView;

use crate::stream::{ConnectionStatus, StreamHandle};

pub struct App {
    stream: StreamHandle,
    frame_view: FrameView,
    last_rendered_ts: Option<std::time::Duration>,
    has_ticket: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, stream: StreamHandle, has_ticket: bool) -> Self {
        let frame_view = FrameView::new(&cc.egui_ctx, "herd-scout-video");
        Self {
            stream,
            frame_view,
            last_rendered_ts: None,
            has_ticket,
        }
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
            // Drop the Arc to release our reference; the watch channel
            // still holds a copy for Wave 3 inference consumers.
            drop(frame as Arc<_>);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Repaint at ~60 Hz so frame age + status stay fresh; the streaming
        // task also calls `ctx.request_repaint()` on every frame arrival to
        // keep latency low under load.
        ctx.request_repaint_after(std::time::Duration::from_millis(16));

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
                if !self.has_ticket {
                    placeholder(ui, "Waiting for ticket — set HERD_SCOUT_TICKET env var");
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
                ui.add_sized(avail, img);

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
