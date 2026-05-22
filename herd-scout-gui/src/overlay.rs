//! CV overlay rendering: per-detection boxes + labels, top-right
//! counts panel, and the optional banner.
//!
//! Wave 6: detections arrive over IPC as `DetWire` with bbox in
//! normalised `[0, 1]` source-frame space (the daemon does the
//! division before sending). We just multiply by the rendered video
//! rect to project onto screen.

use std::time::{Duration, Instant};

use eframe::egui;
use herd_scout_ipc::{ClassCountsWire, DetWire};

/// Paint the CV overlay on top of `video_rect`.
pub fn draw(
    ui: &egui::Ui,
    video_rect: egui::Rect,
    dets: &[DetWire],
    counts: ClassCountsWire,
    cv_banner: Option<&str>,
    cv_disabled: bool,
    last_det_at: Option<Instant>,
) {
    let painter = ui.painter();

    if let Some(banner) = cv_banner {
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

    if cv_disabled {
        return;
    }

    let origin = video_rect.left_top();
    let width = video_rect.width();
    let height = video_rect.height();

    for det in dets {
        let (r, g, b) = det.class_rgb();
        let stroke_color = egui::Color32::from_rgb(r, g, b);
        let x1 = origin.x + det.bbox[0].clamp(0.0, 1.0) * width;
        let y1 = origin.y + det.bbox[1].clamp(0.0, 1.0) * height;
        let x2 = origin.x + det.bbox[2].clamp(0.0, 1.0) * width;
        let y2 = origin.y + det.bbox[3].clamp(0.0, 1.0) * height;
        let rect = egui::Rect::from_min_max(egui::pos2(x1, y1), egui::pos2(x2, y2));
        painter.rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(2.0, stroke_color),
            egui::StrokeKind::Outside,
        );
        let label = format!("{} {:.2}", det.class_label(), det.score);
        let label_pos = egui::pos2(x1, (y1 - 4.0).max(origin.y));
        painter.text(
            label_pos,
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(12.0),
            stroke_color,
        );
    }

    let counts_text = format!(
        "Cows: {}  Horses: {}  Sheep: {}",
        counts.cow, counts.horse, counts.sheep
    );
    let panel_pos = video_rect.right_top() + egui::vec2(-8.0, 8.0);
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

    if let Some(last) = last_det_at {
        if last.elapsed() > Duration::from_secs(2) {
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
