//! Long-lived upload processor (Wave 13 / Phase 2).
//!
//! Pops the head of the queue when no live phone session is active and
//! drives the cv-sidecar through one clip via the file-mode (`0x01`)
//! wire path. Per-frame detections are forwarded to the GUI's
//! `ServerMsg::Detections` fan-out (with `clip_id = Some(blake3_hex)`
//! so the GUI can disambiguate live vs replay frames). At end-of-clip
//! the processor builds a [`super::report::ClipReport`], writes it
//! atomically as `report.json`, and marks the queue entry `Done`.
//!
//! ## Live preemption simplification (plan Decision 4 Risk row)
//!
//! The plan offers two strategies for the "phone pairs mid-clip" race:
//! (a) suspend the upload between frames and resume after the live
//! session ends, or (b) finish the current clip if it's > 90 % done.
//! v1 takes a third, simpler path: **once a clip starts, run it to
//! completion**. The sidecar mutex ensures the live frame path waits
//! its turn; a 10-min cap (per Decision 7) bounds the worst-case live
//! delay. This satisfies the plan's "v1 acceptable" stipulation and
//! avoids a tricky preempt path. The wait-on-Idle gate at the top of
//! the loop still ensures we *don't start* a new clip while a phone
//! is connected.

use std::path::Path;
use std::time::Instant;

use herd_scout_ipc::{
    ClassCountsWire, ConnectionStatus, DetWire, ServerMsg, UploadEntry, UploadState,
};
use tokio::sync::{broadcast, watch};
use tracing::{debug, info, warn};

use super::queue::Queue;
use super::report::{ByteTrackParams, ClipReport, FrameRecord};
use super::store::{clip_dir, clip_filename_from};
use crate::audit::Audit;
use crate::cv::model::SidecarHandle;

/// Plan Decision 7: 10 minutes max per clip. Enforced after the
/// sidecar's probe response.
const MAX_CLIP_DURATION_SEC: f32 = 10.0 * 60.0;

/// Cadence at which the processor publishes `ServerMsg::UploadStatus
/// { state: Decoding, progress_pct }`. Decoupled from the
/// per-frame fan-out so we don't spam the GUI.
const PROGRESS_BATCH: u32 = 30;

/// Spawn the long-lived upload processor task. Returns immediately;
/// the task runs until the runtime shuts down.
pub fn spawn_processor(
    queue: Queue,
    uploads_dir: std::path::PathBuf,
    sidecar_rx: watch::Receiver<Option<SidecarHandle>>,
    status_rx: watch::Receiver<ConnectionStatus>,
    server_tx: broadcast::Sender<ServerMsg>,
    audit: Audit,
) {
    tokio::spawn(async move {
        info!("upload-processor: task started");
        run_loop(queue, uploads_dir, sidecar_rx, status_rx, server_tx, audit).await;
        warn!("upload-processor: task exited");
    });
}

/// Inner driver loop. Factored out for readability; doesn't take an
/// "exit" signal because the task lives for the daemon's lifetime.
async fn run_loop(
    queue: Queue,
    uploads_dir: std::path::PathBuf,
    mut sidecar_rx: watch::Receiver<Option<SidecarHandle>>,
    mut status_rx: watch::Receiver<ConnectionStatus>,
    server_tx: broadcast::Sender<ServerMsg>,
    audit: Audit,
) {
    // Wait for the sidecar to become available before doing anything.
    // The CV task publishes the handle once `Detector::new` succeeds.
    while sidecar_rx.borrow().is_none() {
        if sidecar_rx.changed().await.is_err() {
            warn!("upload-processor: sidecar handle channel closed; exiting");
            return;
        }
    }

    let notify = queue.notify_handle();
    loop {
        // Gate 1: live phone session must be Idle/Stopped before we
        // touch the sidecar. The plan's Decision 4 picks "live wins"
        // for the steady-state case.
        if !is_idle(&*status_rx.borrow()) {
            tokio::select! {
                changed = status_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    continue;
                }
                _ = notify.notified() => {
                    // A new entry might have been queued; recheck status.
                    continue;
                }
            }
        }

        // Gate 2: queue must have a Queued entry. `pop_for_processing`
        // also flips state → `Decoding` and persists.
        let entry = match queue.pop_for_processing().await {
            Some(e) => e,
            None => {
                // Nothing to do — wait for either a queue insertion
                // or a status change.
                tokio::select! {
                    _ = notify.notified() => continue,
                    res = status_rx.changed() => {
                        if res.is_err() {
                            return;
                        }
                        continue;
                    }
                }
            }
        };

        // Take a snapshot of the sidecar handle for this clip's run.
        let Some(handle) = sidecar_rx.borrow().clone() else {
            warn!(
                blake3 = %entry.blake3_hex,
                "upload-processor: sidecar disappeared between gates; failing entry"
            );
            queue.mark_failed(&entry.blake3_hex, "sidecar_unavailable").await;
            audit
                .log(
                    "upload_failed",
                    None,
                    None,
                    serde_json::json!({
                        "blake3_hex": entry.blake3_hex,
                        "reason": "sidecar_unavailable",
                    }),
                )
                .await;
            continue;
        };

        // Resolve the clip path. The handler/handoff path is responsible
        // for placing `clip.<ext>` under `<uploads_dir>/<blake3>/`.
        let clip_dir_path = clip_dir(&uploads_dir, &entry.blake3_hex);
        let clip_filename = clip_filename_from(&entry.filename);
        let clip_path = clip_dir_path.join(&clip_filename);
        if !clip_path.exists() {
            let reason = format!("clip_path_missing: {}", clip_path.display());
            warn!(
                blake3 = %entry.blake3_hex,
                clip_path = %clip_path.display(),
                "upload-processor: staged clip not found; failing entry"
            );
            publish_status(
                &server_tx,
                &entry,
                UploadState::Failed {
                    reason: reason.clone(),
                },
                100,
                None,
            );
            queue.mark_failed(&entry.blake3_hex, reason.clone()).await;
            audit
                .log(
                    "upload_failed",
                    None,
                    None,
                    serde_json::json!({
                        "blake3_hex": entry.blake3_hex,
                        "reason": reason,
                    }),
                )
                .await;
            continue;
        }

        // Tell the GUI the clip has started.
        publish_status(
            &server_tx,
            &entry,
            UploadState::Decoding,
            0,
            None,
        );
        audit
            .log(
                "upload_started",
                None,
                None,
                serde_json::json!({ "blake3_hex": entry.blake3_hex }),
            )
            .await;

        let clip_started = Instant::now();
        match process_one_clip(
            &entry,
            &clip_path,
            handle,
            &server_tx,
        )
        .await
        {
            Ok(outcome) => {
                let processing_ms = clip_started.elapsed().as_millis() as u64;
                let report = ClipReport::build(
                    &entry.blake3_hex,
                    &entry.filename,
                    outcome.duration_ms,
                    outcome.fps,
                    outcome.frame_count,
                    processing_ms,
                    ByteTrackParams::default(),
                    &outcome.frames,
                );
                let inline = report.inline_summary();
                if let Err(e) = report.write_atomic(&clip_dir_path) {
                    warn!(
                        blake3 = %entry.blake3_hex,
                        "upload-processor: write_atomic(report.json) failed: {e:#}",
                    );
                }
                publish_status(
                    &server_tx,
                    &entry,
                    UploadState::Done,
                    100,
                    Some(inline.clone()),
                );
                queue.mark_done(&entry.blake3_hex).await;
                audit
                    .log(
                        "upload_done",
                        None,
                        None,
                        serde_json::json!({
                            "blake3_hex": entry.blake3_hex,
                            "processing_ms": processing_ms,
                            "frame_count": outcome.frame_count,
                            "median_active_count_total": inline.median_active_count_total,
                            "bootstrap_ci_95_total": inline.bootstrap_ci_95_total,
                        }),
                    )
                    .await;
                info!(
                    blake3 = %entry.blake3_hex,
                    frames = outcome.frame_count,
                    processing_ms,
                    "upload-processor: clip done",
                );
            }
            Err(e) => {
                let reason = format!("{e:#}");
                warn!(
                    blake3 = %entry.blake3_hex,
                    "upload-processor: clip failed: {reason}",
                );
                publish_status(
                    &server_tx,
                    &entry,
                    UploadState::Failed {
                        reason: reason.clone(),
                    },
                    100,
                    None,
                );
                queue.mark_failed(&entry.blake3_hex, reason.clone()).await;
                audit
                    .log(
                        "upload_failed",
                        None,
                        None,
                        serde_json::json!({
                            "blake3_hex": entry.blake3_hex,
                            "reason": reason,
                        }),
                    )
                    .await;
            }
        }
    }
}

/// Outcome of a successful single-clip run.
struct ClipOutcome {
    frame_count: u32,
    duration_ms: u64,
    fps: f32,
    frames: Vec<FrameRecord>,
}

/// Drive the sidecar through one file-mode clip. Pure I/O — no queue
/// or audit calls; the caller wraps that around us.
///
/// Returns an error string when the sidecar emits the error
/// terminator, the duration cap is exceeded, or stream I/O fails.
async fn process_one_clip(
    entry: &UploadEntry,
    clip_path: &Path,
    handle: SidecarHandle,
    server_tx: &broadcast::Sender<ServerMsg>,
) -> anyhow::Result<ClipOutcome> {
    let blake3_hex = entry.blake3_hex.clone();
    let path_str = clip_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("clip path not utf-8: {}", clip_path.display()))?
        .to_string();
    let clip_id_bytes = decode_blake3_hex(&blake3_hex)?;
    let entry_for_progress = entry.clone();
    let server_tx_for_progress = server_tx.clone();

    // The sidecar I/O is synchronous; do the whole clip on a blocking
    // thread. The `SidecarHandle`'s std `Mutex` is held for the
    // duration of the clip — live frame requests will wait their turn.
    //
    // Why `std::sync::Mutex` is correct here: every caller of the
    // sidecar (this processor and `cv/task.rs`'s live inference loop)
    // does its work inside `tokio::task::spawn_blocking`. A live frame
    // request that arrives mid-clip blocks on the blocking thread pool,
    // not on the async runtime — async tasks keep making progress. See
    // `cv/model.rs::SidecarHandle` for the full contract.
    let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<ClipOutcome> {
        use std::io::Write;
        use super::protocol::{
            encode_file_request, read_response, FileResponse, MAX_DETS_PER_FRAME,
        };
        let _ = MAX_DETS_PER_FRAME; // referenced for documentation purposes.
        let mut stream = handle
            .lock()
            .map_err(|e| anyhow::anyhow!("sidecar handle poisoned: {e}"))?;

        // Send the file-mode request.
        let req = encode_file_request(&clip_id_bytes, &path_str);
        stream.write_all(&req).map_err(|e| {
            anyhow::anyhow!("upload-processor: write file request: {e}")
        })?;

        // First response must be the probe.
        let probe = match read_response(&mut *stream)? {
            FileResponse::Probe(info) => info,
            FileResponse::Error { reason } => {
                anyhow::bail!("sidecar reported error: {reason}");
            }
            other => {
                anyhow::bail!(
                    "expected probe response, got: {:?}",
                    discriminant_name(&other)
                );
            }
        };

        // Duration cap (Decision 7).
        if probe.fps > 0.0 {
            let duration_sec = probe.frame_count as f32 / probe.fps;
            if duration_sec > MAX_CLIP_DURATION_SEC {
                anyhow::bail!(
                    "duration_cap_exceeded: {:.1}s > {:.0}s cap",
                    duration_sec,
                    MAX_CLIP_DURATION_SEC,
                );
            }
        }

        let mut frames: Vec<FrameRecord> = Vec::with_capacity(probe.frame_count as usize);
        let mut frames_seen: u32 = 0;
        let probe_frame_count = probe.frame_count.max(1);

        loop {
            match read_response(&mut *stream)? {
                FileResponse::Frame {
                    decode_index,
                    pts_ms,
                    dets,
                } => {
                    frames_seen += 1;
                    let wire_dets: Vec<DetWire> = dets
                        .iter()
                        .filter_map(|d| {
                            if d.class_id > 2 {
                                None
                            } else {
                                Some(DetWire {
                                    class: d.class_id as u8,
                                    bbox: [d.x1, d.y1, d.x2, d.y2],
                                    score: d.conf,
                                    track_id: d.track_id,
                                })
                            }
                        })
                        .collect();
                    let counts = counts_from_dets(&wire_dets);

                    // Forward per-frame detections to the GUI for the
                    // overlay-replay rendering path. `clip_id` lets
                    // the GUI route these to the right pane.
                    let _ = server_tx_for_progress.send(ServerMsg::Detections {
                        frame_pts_ms: pts_ms,
                        dets: wire_dets,
                        counts,
                        clip_id: Some(blake3_hex.clone()),
                    });

                    // Per-batch progress ping for the GUI's queue panel.
                    if frames_seen % PROGRESS_BATCH == 0 {
                        let progress_pct =
                            ((frames_seen as f32 / probe_frame_count as f32) * 100.0)
                                .clamp(0.0, 99.0) as u8;
                        let _ = server_tx_for_progress.send(ServerMsg::UploadStatus {
                            blake3_hex: entry_for_progress.blake3_hex.clone(),
                            filename: entry_for_progress.filename.clone(),
                            state: UploadState::Decoding,
                            progress_pct,
                            eta_ms: None,
                            summary: None,
                        });
                    }

                    let report_dets: Vec<herd_scout_ipc::DetWire> = dets
                        .iter()
                        .filter_map(|d| {
                            if d.class_id > 2 {
                                None
                            } else {
                                Some(herd_scout_ipc::DetWire {
                                    class: d.class_id as u8,
                                    bbox: [d.x1, d.y1, d.x2, d.y2],
                                    score: d.conf,
                                    track_id: d.track_id,
                                })
                            }
                        })
                        .collect();
                    frames.push(FrameRecord {
                        frame_id: decode_index,
                        pts_ms,
                        detections: report_dets,
                    });
                }
                FileResponse::End => break,
                FileResponse::Error { reason } => {
                    anyhow::bail!("sidecar terminator: {reason}");
                }
                FileResponse::Probe(_) => {
                    anyhow::bail!("unexpected second probe response");
                }
            }
        }

        let duration_ms = if probe.fps > 0.0 {
            (probe.frame_count as f64 / probe.fps as f64 * 1000.0) as u64
        } else {
            0
        };
        Ok(ClipOutcome {
            frame_count: probe.frame_count,
            duration_ms,
            fps: probe.fps,
            frames,
        })
    })
    .await
    .map_err(|e| anyhow::anyhow!("clip task panicked: {e}"))??;

    debug!(
        blake3_hex = %entry.blake3_hex,
        frames = outcome.frame_count,
        "upload-processor: file-mode pass complete",
    );
    Ok(outcome)
}

fn discriminant_name(r: &super::protocol::FileResponse) -> &'static str {
    use super::protocol::FileResponse;
    match r {
        FileResponse::Probe(_) => "Probe",
        FileResponse::Frame { .. } => "Frame",
        FileResponse::End => "End",
        FileResponse::Error { .. } => "Error",
    }
}

fn is_idle(status: &ConnectionStatus) -> bool {
    matches!(
        status,
        ConnectionStatus::Idle
            | ConnectionStatus::Stopped
            | ConnectionStatus::Reconnecting { .. }
    )
}

fn counts_from_dets(dets: &[DetWire]) -> ClassCountsWire {
    let mut c = ClassCountsWire::default();
    for d in dets {
        match d.class {
            0 => c.horse += 1,
            1 => c.sheep += 1,
            2 => c.cow += 1,
            _ => {}
        }
    }
    c
}

/// Best-effort publish of an `UploadStatus` message to the GUI fan-out
/// broadcast. Failures are silent — there may be no subscribers.
fn publish_status(
    server_tx: &broadcast::Sender<ServerMsg>,
    entry: &UploadEntry,
    state: UploadState,
    progress_pct: u8,
    summary: Option<herd_scout_ipc::UploadSummaryInline>,
) {
    let _ = server_tx.send(ServerMsg::UploadStatus {
        blake3_hex: entry.blake3_hex.clone(),
        filename: entry.filename.clone(),
        state,
        progress_pct,
        eta_ms: None,
        summary,
    });
}

/// Decode a 64-char hex BLAKE3 string into a 32-byte array. Returns an
/// error rather than panicking so a malformed entry can be marked
/// `Failed` instead of crashing the processor.
fn decode_blake3_hex(hex: &str) -> anyhow::Result<[u8; 32]> {
    if hex.len() != 64 {
        anyhow::bail!("blake3 hex must be 64 chars, got {}", hex.len());
    }
    let bytes = hex.as_bytes();
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = (hex_digit(bytes[i * 2])? << 4) | hex_digit(bytes[i * 2 + 1])?;
    }
    Ok(out)
}

fn hex_digit(b: u8) -> anyhow::Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => anyhow::bail!("non-hex byte 0x{b:02x} in blake3 hex"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_blake3_hex_round_trips() {
        let bytes = [0x9cu8; 32];
        let hex = bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let parsed = decode_blake3_hex(&hex).unwrap();
        assert_eq!(parsed, bytes);
    }

    #[test]
    fn decode_blake3_hex_rejects_short() {
        assert!(decode_blake3_hex("abc").is_err());
    }

    #[test]
    fn decode_blake3_hex_rejects_non_hex() {
        let bad = "z".repeat(64);
        assert!(decode_blake3_hex(&bad).is_err());
    }

    #[test]
    fn is_idle_matches_expected_states() {
        assert!(is_idle(&ConnectionStatus::Idle));
        assert!(is_idle(&ConnectionStatus::Stopped));
        assert!(is_idle(&ConnectionStatus::Reconnecting {
            reason: "x".into()
        }));
        assert!(!is_idle(&ConnectionStatus::Connecting));
        assert!(!is_idle(&ConnectionStatus::Connected));
    }
}
