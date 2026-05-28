//! CV inference task — bridges the streaming `watch::Receiver<VideoFrame>`
//! to the shared `DetectionSnapshot` and the daemon's IPC fan-out.
//!
//! Pacing rule (from `cv-design.md`): cap inference at 10 FPS via a
//! `tokio::time::Interval`. If the latest frame is older than the tick,
//! we skip it and wait for the next tick. ORT's `Session::run` is
//! synchronous and CPU-heavy, so we punt it to `spawn_blocking`.
//!
//! ## Wave 6 changes
//!
//! Replaced the egui `Context::request_repaint` side-channel with an
//! `mpsc::Sender<ServerMsg>` so the daemon (which has no egui) can fan
//! detections out to GUIs over IPC. The `SharedSnapshot` is still
//! written so headless ("dump-to-disk") modes can keep observing the
//! rolling state.

use std::sync::Arc;
use std::time::{Duration, Instant};

use herd_scout_ipc::{ClassCountsWire, DetWire, ServerMsg};
use iroh_live::media::format::VideoFrame;
use tokio::sync::{mpsc, watch};
use tokio::time::{MissedTickBehavior, interval};
use tracing::{error, info, warn};

use super::model::{CocoClass, Detector, SidecarHandle};
use super::state::{ClassCounts, SharedSnapshot};

/// Frame budget. Locked at 10 FPS by the design doc.
const TICK: Duration = Duration::from_millis(100);

fn class_to_wire(c: CocoClass) -> u8 {
    match c {
        CocoClass::Horse => 0,
        CocoClass::Sheep => 1,
        CocoClass::Cow => 2,
    }
}

fn counts_to_wire(c: ClassCounts) -> ClassCountsWire {
    ClassCountsWire {
        horse: c.horse,
        sheep: c.sheep,
        cow: c.cow,
    }
}

/// Spawn the long-lived CV inference task.
///
/// `ipc_tx`: each successful inference emits a `ServerMsg::Detections`
/// to this channel; an init failure emits a `ServerMsg::CvBanner` and
/// the task exits.
///
/// `handle_tx`: when set, the task publishes the sidecar's
/// [`SidecarHandle`] once the detector successfully connects. The
/// upload processor (Wave 13) subscribes to this so it can drive the
/// sidecar through file-mode requests when no live phone session is
/// active.
pub fn spawn_cv_task(
    mut frame_rx: watch::Receiver<Option<Arc<VideoFrame>>>,
    snapshot: SharedSnapshot,
    ipc_tx: mpsc::Sender<ServerMsg>,
    handle_tx: Option<watch::Sender<Option<SidecarHandle>>>,
) {
    tokio::spawn(async move {
        // Build the detector on the inference task so any init cost
        // doesn't block other tasks. If it fails we still keep the
        // task alive enough to send a banner, then exit.
        let detector = match tokio::task::spawn_blocking(Detector::new).await {
            Ok(Ok(d)) => d,
            Ok(Err(e)) => {
                error!(error = %e, "CV: failed to build YOLOv5n session; CV disabled for session");
                snapshot
                    .write()
                    .disable(format!("CV disabled: {e}"));
                let _ = ipc_tx
                    .send(ServerMsg::CvBanner {
                        text: Some(format!("CV disabled: {e}")),
                        disabled: true,
                    })
                    .await;
                return;
            }
            Err(e) => {
                error!(error = %e, "CV: spawn_blocking for Detector::new panicked");
                snapshot
                    .write()
                    .disable(format!("CV disabled: detector init panicked: {e}"));
                let _ = ipc_tx
                    .send(ServerMsg::CvBanner {
                        text: Some(format!("CV disabled: detector init panicked: {e}")),
                        disabled: true,
                    })
                    .await;
                return;
            }
        };
        info!("CV: YOLOv5n session ready (10 FPS budget)");

        // Publish the sidecar handle to whichever consumers are
        // waiting on it (the upload processor is the only one today;
        // future "headless dump" modes could subscribe similarly).
        if let Some(tx) = handle_tx {
            let _ = tx.send(Some(detector.handle()));
        }

        // The detector is owned by an `Arc<Mutex>` so each
        // `spawn_blocking` body can take it for the duration of one
        // inference call.
        let detector = Arc::new(tokio::sync::Mutex::new(detector));

        let mut tick = interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        tick.tick().await;

        loop {
            tokio::select! {
                _ = tick.tick() => {}
                changed = frame_rx.changed() => {
                    if changed.is_err() {
                        info!("CV: frame channel closed; inference task exiting");
                        break;
                    }
                    continue;
                }
            }

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
                    let frame_pts_ms = frame.timestamp.as_millis() as u64;
                    // CV bboxes are in source-frame pixel space; the
                    // GUI sees the daemon's downscaled JPEG preview,
                    // so we ship normalised coordinates [0, 1] and
                    // the GUI multiplies by the rendered video rect.
                    let src_w = frame.width().max(1) as f32;
                    let src_h = frame.height().max(1) as f32;
                    let wire_dets: Vec<DetWire> = dets
                        .iter()
                        .map(|d| DetWire {
                            class: class_to_wire(d.class),
                            bbox: [
                                d.bbox[0] / src_w,
                                d.bbox[1] / src_h,
                                d.bbox[2] / src_w,
                                d.bbox[3] / src_h,
                            ],
                            score: d.score,
                            track_id: d.track_id,
                        })
                        .collect();

                    snapshot.write().update(dets, frame.timestamp, now);
                    let counts_wire = counts_to_wire(snapshot.read().rolling_counts());

                    let _ = ipc_tx
                        .send(ServerMsg::Detections {
                            frame_pts_ms,
                            dets: wire_dets,
                            counts: counts_wire,
                            clip_id: None,
                        })
                        .await;
                }
                Ok(Err(e)) => {
                    warn!(error = %e, "CV: single-frame inference failed; skipping");
                    let msg = format!("{e:#}");
                    if msg.contains("unexpected output shape") || msg.contains("unexpected row length") {
                        snapshot.write().disable("CV: model output shape unexpected");
                        let _ = ipc_tx
                            .send(ServerMsg::CvBanner {
                                text: Some("CV: model output shape unexpected".to_string()),
                                disabled: true,
                            })
                            .await;
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
