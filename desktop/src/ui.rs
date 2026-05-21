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
use std::time::{Duration, Instant};

use eframe::egui;
use moq_media_egui::FrameView;

use crate::cv::SharedSnapshot;
use crate::stream::{ConnectionStatus, StreamHandle};

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
    has_ticket: bool,
    /// Shared snapshot owned jointly with the CV inference task.
    snapshot: SharedSnapshot,
    /// Most recently observed source-frame dimensions, used for box
    /// projection. Updated in [`drain_frames`].
    last_frame_dims: Option<FrameDims>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        stream: StreamHandle,
        snapshot: SharedSnapshot,
        has_ticket: bool,
    ) -> Self {
        let frame_view = FrameView::new(&cc.egui_ctx, "herd-scout-video");
        Self {
            stream,
            frame_view,
            last_rendered_ts: None,
            has_ticket,
            snapshot,
            last_frame_dims: None,
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
                let img_response = ui.add_sized(avail, img);
                // The rect occupied by the just-painted video texture.
                // CV overlay coordinates are projected into this rect.
                let video_rect = img_response.rect;

                // === Wave 3: CV detection overlay ===
                self.draw_cv_overlay(ui, video_rect);

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
