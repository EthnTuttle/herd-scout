//! CV inference task — bridges the streaming `watch::Receiver<VideoFrame>`
//! to the shared `DetectionSnapshot` consumed by egui.
//!
//! Pacing rule (from `cv-design.md`): cap inference at 10 FPS via a
//! `tokio::time::Interval`. If the latest frame is older than the tick,
//! we skip it and wait for the next tick. ORT's `Session::run` is
//! synchronous and CPU-heavy, so we punt it to `spawn_blocking`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh_live::media::format::VideoFrame;
use tokio::sync::watch;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{error, info, warn};

use super::model::Detector;
use super::state::SharedSnapshot;

/// Frame budget. Locked at 10 FPS by the design doc.
const TICK: Duration = Duration::from_millis(100);

/// Spawn the long-lived CV inference task. Returns immediately; the
/// task runs until the runtime shuts down.
///
/// The task:
/// * subscribes to the same `watch::Receiver<Option<Arc<VideoFrame>>>`
///   the UI reads from,
/// * builds a `Detector` (logs ERROR + sets the snapshot to "disabled"
///   if construction fails — does **not** panic),
/// * ticks every 100 ms, runs inference on the latest frame via
///   `spawn_blocking`,
/// * writes the result into the shared snapshot for the UI thread.
pub fn spawn_cv_task(
    mut frame_rx: watch::Receiver<Option<Arc<VideoFrame>>>,
    snapshot: SharedSnapshot,
    egui_ctx: egui::Context,
) {
    tokio::spawn(async move {
        // Build the detector on the inference task so any init cost
        // doesn't block the UI thread. If it fails we still keep the
        // task alive (per design doc: "video keeps playing") but mark
        // the snapshot as disabled and exit.
        let detector = match tokio::task::spawn_blocking(Detector::new).await {
            Ok(Ok(d)) => d,
            Ok(Err(e)) => {
                error!(error = %e, "CV: failed to build YOLOv5n session; CV disabled for session");
                snapshot
                    .write()
                    .disable(format!("CV disabled: {e}"));
                egui_ctx.request_repaint();
                return;
            }
            Err(e) => {
                error!(error = %e, "CV: spawn_blocking for Detector::new panicked");
                snapshot
                    .write()
                    .disable(format!("CV disabled: detector init panicked: {e}"));
                egui_ctx.request_repaint();
                return;
            }
        };
        info!("CV: YOLOv5n session ready (10 FPS budget)");

        // The detector is owned by an `Arc<Mutex>` so each
        // `spawn_blocking` body can take it for the duration of one
        // inference call. (Single-task means there's never real
        // contention on this mutex; it's just to satisfy `Send`
        // bounds on the moved closure.)
        let detector = Arc::new(tokio::sync::Mutex::new(detector));

        let mut tick = interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Skip the immediate-fire first tick.
        tick.tick().await;

        loop {
            tokio::select! {
                _ = tick.tick() => {}
                changed = frame_rx.changed() => {
                    // Channel closed → streaming task is gone; we can
                    // exit cleanly.
                    if changed.is_err() {
                        info!("CV: frame channel closed; inference task exiting");
                        break;
                    }
                    // Don't infer on every frame change; coalesce until
                    // the next tick.
                    continue;
                }
            }

            // Snapshot the latest frame.
            let Some(frame) = frame_rx.borrow().clone() else {
                continue;
            };

            let detector = detector.clone();
            let frame_for_blocking = frame.clone();
            let join = tokio::task::spawn_blocking(move || {
                let mut guard = detector.blocking_lock();
                guard.infer(&frame_for_blocking)
            })
            .await;

            match join {
                Ok(Ok(dets)) => {
                    let now = Instant::now();
                    snapshot.write().update(dets, frame.timestamp, now);
                    egui_ctx.request_repaint();
                }
                Ok(Err(e)) => {
                    warn!(error = %e, "CV: single-frame inference failed; skipping");
                    // If decode_yolov5 raised an unexpected-shape error,
                    // surface it in the UI — design doc calls this out.
                    let msg = format!("{e:#}");
                    if msg.contains("unexpected output shape") || msg.contains("unexpected row length") {
                        snapshot.write().disable("CV: model output shape unexpected");
                        egui_ctx.request_repaint();
                        error!("CV: disabling due to persistent shape mismatch");
                        return;
                    }
                }
                Err(e) => {
                    error!(error = %e, "CV: spawn_blocking inference panicked; skipping");
                }
            }
        }
    });
}
