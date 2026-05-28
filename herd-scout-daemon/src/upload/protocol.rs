//! Sidecar wire-protocol helpers for the new file-decode (`0x01`) path.
//!
//! Wave 13 / Phase 2 of `plan-desktop-video-upload-2026-05-28.md`. This
//! module knows how to *encode* a file-mode request and *parse* the
//! sidecar's stream of responses. The parsing surface is intentionally
//! event-shaped — `read_response` returns a `FileResponse` enum so the
//! upload processor can drive a small state machine
//! (probe → frames → terminator) without re-implementing the framing
//! mid-loop.
//!
//! The wire shape is documented in `deploy/cv-sidecar/cv_sidecar.py`
//! (the canonical source of truth). A short summary:
//!
//! * Request: `u32 0x01` + `[u8; 32] clip_id` + `u32 path_len` +
//!   `path utf8`.
//! * Probe response: header `(0xFFFFFFF0, 0)` + 16-byte trailer
//!   `(frame_count u32, fps f32, width u32, height u32)`.
//! * Per-frame response: header `(decode_index, n_dets)` + 8-byte
//!   `pts_ms u64` + `n_dets * 28` det rows (`<IIfffff>` per row).
//! * End terminator: header `(0xFFFFFFFF, 0xFFFFFFFF)`.
//! * Error terminator: header `(0xFFFFFFFE, 0xFFFFFFFE)` + `u32
//!   reason_len` + utf8 reason.
//!
//! Cancel sentinel (daemon → sidecar): a single `0x01` byte written
//! between frames. Helpers below expose `write_cancel`.
//!
//! Note on iroh-blobs: the plan calls for iroh-blobs as the byte
//! transport. The first cut streams clip bytes over the same QUIC
//! bi-stream that carries the JSON `Push` / `Accepted` exchange (see
//! [`super::handler`]). That avoids a second ALPN registration; when
//! we wire iroh-blobs in a follow-up wave, the wrapping JSON message
//! already names a BLAKE3, so the migration is a swap of the byte path
//! only.

use std::io::{self, Read, Write};

// ---------------------------------------------------------------------
// Wire-level constants — kept in lockstep with cv_sidecar.py.
// ---------------------------------------------------------------------

/// Request kind selector for the live-frame path. Prepended to every
/// per-frame request the daemon's CV task sends.
pub const REQ_KIND_FRAME: u32 = 0x00;

/// Request kind selector for the file-mode path.
pub const REQ_KIND_FILE: u32 = 0x01;

/// Sentinel `frame_id` marking the probe response.
pub const SENTINEL_PROBE: u32 = 0xFFFF_FFF0;

/// Sentinel `frame_id` marking a successful end-of-clip terminator.
pub const SENTINEL_END: u32 = 0xFFFF_FFFF;

/// Sentinel `frame_id` marking an error terminator.
pub const SENTINEL_ERROR: u32 = 0xFFFF_FFFE;

/// Single-byte sentinel the daemon writes to cancel an in-flight clip.
pub const CANCEL_MARKER: u8 = 0x01;

/// Per-detection on-wire length: `<IIfffff>` (class, track_id, conf,
/// x1, y1, x2, y2).
pub const DET_PACK_BYTES: usize = 28;

/// Defensive cap on per-frame detections from the sidecar. Mirrors the
/// guard in `cv::model::Detector::infer`; runaway responses would
/// otherwise OOM the daemon.
pub const MAX_DETS_PER_FRAME: u32 = 1024;

/// Wire sentinel for "no track ID assigned yet" — must match the
/// sidecar's `NO_TRACK_ID`.
pub const NO_TRACK_ID: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------
// Request encoding
// ---------------------------------------------------------------------

/// Encode a file-mode request body (everything *after* the `u32`
/// `request_kind` selector, which the caller is expected to prepend).
///
/// The selector itself is included for convenience — the caller can
/// either write `&[REQ_KIND_FILE.to_le_bytes(), encode_file_request(...).as_slice()]`
/// or use [`write_file_request`] which does the right thing in one
/// call.
///
/// Returns `Vec<u8>` rather than borrowing a buffer because the path
/// length is dynamic and the call rate is once per clip — allocation
/// cost is negligible.
pub fn encode_file_request(clip_id: &[u8; 32], path: &str) -> Vec<u8> {
    let path_bytes = path.as_bytes();
    let mut out = Vec::with_capacity(4 + 32 + 4 + path_bytes.len());
    out.extend_from_slice(&REQ_KIND_FILE.to_le_bytes());
    out.extend_from_slice(clip_id);
    out.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(path_bytes);
    out
}

/// Write a complete file-mode request to `w`. Equivalent to building
/// the buffer with [`encode_file_request`] and writing it as one
/// `write_all`. Single syscall path on most stream types.
pub fn write_file_request<W: Write>(
    w: &mut W,
    clip_id: &[u8; 32],
    path: &str,
) -> io::Result<()> {
    let buf = encode_file_request(clip_id, path);
    w.write_all(&buf)
}

/// Write the daemon → sidecar single-byte cancel marker. Caller must
/// have exclusive write access to the sidecar stream (the same `Mutex`
/// that gates `infer` / clip processing).
pub fn write_cancel<W: Write>(w: &mut W) -> io::Result<()> {
    w.write_all(&[CANCEL_MARKER])
}

// ---------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------

/// One per-frame detection row, parsed from the sidecar's `<IIfffff>`
/// pack format. Bounding box is in source-frame pixel space.
///
/// `track_id == None` when the wire sentinel is `NO_TRACK_ID`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileDet {
    pub class_id: u32,
    pub track_id: Option<u32>,
    pub conf: f32,
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

/// One probe response — sent exactly once at the start of every
/// file-mode clip so the daemon can enforce duration caps before
/// committing to a long decode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProbeInfo {
    pub frame_count: u32,
    pub fps: f32,
    pub width: u32,
    pub height: u32,
}

/// One parsed file-mode response. The processor's read loop dispatches
/// on this enum — `Probe` arrives once at the head, `Frame` arrives
/// per-decoded-frame, and `End` / `Error` close the stream.
#[derive(Debug, Clone, PartialEq)]
pub enum FileResponse {
    Probe(ProbeInfo),
    Frame {
        decode_index: u32,
        pts_ms: u64,
        dets: Vec<FileDet>,
    },
    End,
    Error {
        reason: String,
    },
}

/// Read one [`FileResponse`] from the sidecar. Blocks until a complete
/// message is available (or until the underlying reader returns an
/// error). Defensive against malformed `n_dets` values.
pub fn read_response<R: Read>(r: &mut R) -> io::Result<FileResponse> {
    // 8-byte header is the same across probe / frame / terminators —
    // we dispatch on the (frame_id, n_dets) tuple.
    let mut hdr = [0u8; 8];
    r.read_exact(&mut hdr)?;
    let frame_id = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
    let n_dets = u32::from_le_bytes(hdr[4..8].try_into().unwrap());

    match frame_id {
        SENTINEL_PROBE => {
            // Trailer: <IfII> = 16 bytes.
            let mut trailer = [0u8; 16];
            r.read_exact(&mut trailer)?;
            let frame_count = u32::from_le_bytes(trailer[0..4].try_into().unwrap());
            let fps = f32::from_le_bytes(trailer[4..8].try_into().unwrap());
            let width = u32::from_le_bytes(trailer[8..12].try_into().unwrap());
            let height = u32::from_le_bytes(trailer[12..16].try_into().unwrap());
            Ok(FileResponse::Probe(ProbeInfo {
                frame_count,
                fps,
                width,
                height,
            }))
        }
        SENTINEL_END => {
            // n_dets is also the sentinel; nothing more to read.
            Ok(FileResponse::End)
        }
        SENTINEL_ERROR => {
            // u32 reason_len + utf8 reason.
            let mut len_buf = [0u8; 4];
            r.read_exact(&mut len_buf)?;
            let reason_len = u32::from_le_bytes(len_buf) as usize;
            // Defensive cap — the sidecar keeps these short, but a
            // corrupt response shouldn't be allowed to OOM us.
            if reason_len > 64 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("error reason_len {reason_len} exceeds cap"),
                ));
            }
            let mut reason_buf = vec![0u8; reason_len];
            r.read_exact(&mut reason_buf)?;
            let reason = String::from_utf8_lossy(&reason_buf).to_string();
            Ok(FileResponse::Error { reason })
        }
        _ => {
            // Per-frame response: <Q> pts_ms (8 bytes) then n_dets * 28.
            if n_dets > MAX_DETS_PER_FRAME {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("absurd n_dets={n_dets} from sidecar"),
                ));
            }
            let mut pts_buf = [0u8; 8];
            r.read_exact(&mut pts_buf)?;
            let pts_ms = u64::from_le_bytes(pts_buf);
            let det_bytes = (n_dets as usize) * DET_PACK_BYTES;
            let mut det_buf = vec![0u8; det_bytes];
            if det_bytes > 0 {
                r.read_exact(&mut det_buf)?;
            }
            let mut dets = Vec::with_capacity(n_dets as usize);
            for i in 0..n_dets as usize {
                let off = i * DET_PACK_BYTES;
                let class_id = u32::from_le_bytes(det_buf[off..off + 4].try_into().unwrap());
                let track_wire =
                    u32::from_le_bytes(det_buf[off + 4..off + 8].try_into().unwrap());
                let conf = f32::from_le_bytes(det_buf[off + 8..off + 12].try_into().unwrap());
                let x1 = f32::from_le_bytes(det_buf[off + 12..off + 16].try_into().unwrap());
                let y1 = f32::from_le_bytes(det_buf[off + 16..off + 20].try_into().unwrap());
                let x2 = f32::from_le_bytes(det_buf[off + 20..off + 24].try_into().unwrap());
                let y2 = f32::from_le_bytes(det_buf[off + 24..off + 28].try_into().unwrap());
                let track_id = (track_wire != NO_TRACK_ID).then_some(track_wire);
                dets.push(FileDet {
                    class_id,
                    track_id,
                    conf,
                    x1,
                    y1,
                    x2,
                    y2,
                });
            }
            Ok(FileResponse::Frame {
                decode_index: frame_id,
                pts_ms,
                dets,
            })
        }
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn pack_det(d: &FileDet) -> [u8; 28] {
        let mut out = [0u8; 28];
        out[0..4].copy_from_slice(&d.class_id.to_le_bytes());
        out[4..8].copy_from_slice(&d.track_id.unwrap_or(NO_TRACK_ID).to_le_bytes());
        out[8..12].copy_from_slice(&d.conf.to_le_bytes());
        out[12..16].copy_from_slice(&d.x1.to_le_bytes());
        out[16..20].copy_from_slice(&d.y1.to_le_bytes());
        out[20..24].copy_from_slice(&d.x2.to_le_bytes());
        out[24..28].copy_from_slice(&d.y2.to_le_bytes());
        out
    }

    #[test]
    fn encode_file_request_layout() {
        let clip_id = [0xABu8; 32];
        let path = "/tmp/clip.mp4";
        let buf = encode_file_request(&clip_id, path);
        assert_eq!(&buf[0..4], &REQ_KIND_FILE.to_le_bytes());
        assert_eq!(&buf[4..36], &clip_id[..]);
        assert_eq!(
            &buf[36..40],
            &(path.as_bytes().len() as u32).to_le_bytes()
        );
        assert_eq!(&buf[40..], path.as_bytes());
    }

    #[test]
    fn parse_probe_response() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&SENTINEL_PROBE.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&100u32.to_le_bytes()); // frame_count
        buf.extend_from_slice(&30.0f32.to_le_bytes()); // fps
        buf.extend_from_slice(&1280u32.to_le_bytes()); // width
        buf.extend_from_slice(&720u32.to_le_bytes()); // height
        let mut c = Cursor::new(buf);
        let parsed = read_response(&mut c).unwrap();
        assert_eq!(
            parsed,
            FileResponse::Probe(ProbeInfo {
                frame_count: 100,
                fps: 30.0,
                width: 1280,
                height: 720,
            })
        );
    }

    #[test]
    fn parse_frame_response_with_dets() {
        let det = FileDet {
            class_id: 2,
            track_id: Some(7),
            conf: 0.95,
            x1: 10.0,
            y1: 20.0,
            x2: 110.0,
            y2: 220.0,
        };
        let det_no_track = FileDet {
            class_id: 1,
            track_id: None,
            conf: 0.4,
            x1: 5.0,
            y1: 6.0,
            x2: 50.0,
            y2: 60.0,
        };
        let mut buf = Vec::new();
        buf.extend_from_slice(&42u32.to_le_bytes()); // decode_index
        buf.extend_from_slice(&2u32.to_le_bytes()); // n_dets
        buf.extend_from_slice(&123_456u64.to_le_bytes()); // pts_ms
        buf.extend_from_slice(&pack_det(&det));
        buf.extend_from_slice(&pack_det(&det_no_track));
        let mut c = Cursor::new(buf);
        let parsed = read_response(&mut c).unwrap();
        match parsed {
            FileResponse::Frame {
                decode_index,
                pts_ms,
                dets,
            } => {
                assert_eq!(decode_index, 42);
                assert_eq!(pts_ms, 123_456);
                assert_eq!(dets.len(), 2);
                assert_eq!(dets[0], det);
                assert_eq!(dets[1], det_no_track);
            }
            other => panic!("expected Frame, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_response_zero_dets() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&5u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&777u64.to_le_bytes());
        let mut c = Cursor::new(buf);
        let parsed = read_response(&mut c).unwrap();
        match parsed {
            FileResponse::Frame {
                decode_index,
                pts_ms,
                dets,
            } => {
                assert_eq!(decode_index, 5);
                assert_eq!(pts_ms, 777);
                assert!(dets.is_empty());
            }
            other => panic!("expected Frame, got {other:?}"),
        }
    }

    #[test]
    fn parse_end_terminator() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&SENTINEL_END.to_le_bytes());
        buf.extend_from_slice(&SENTINEL_END.to_le_bytes());
        let mut c = Cursor::new(buf);
        assert_eq!(read_response(&mut c).unwrap(), FileResponse::End);
    }

    #[test]
    fn parse_error_terminator() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&SENTINEL_ERROR.to_le_bytes());
        buf.extend_from_slice(&SENTINEL_ERROR.to_le_bytes());
        let reason = "open_failed: /missing";
        buf.extend_from_slice(&(reason.as_bytes().len() as u32).to_le_bytes());
        buf.extend_from_slice(reason.as_bytes());
        let mut c = Cursor::new(buf);
        match read_response(&mut c).unwrap() {
            FileResponse::Error { reason: r } => assert_eq!(r, reason),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_absurd_n_dets() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&(MAX_DETS_PER_FRAME + 1).to_le_bytes());
        let mut c = Cursor::new(buf);
        let err = read_response(&mut c).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn write_cancel_emits_one_byte() {
        let mut buf = Vec::new();
        write_cancel(&mut buf).unwrap();
        assert_eq!(buf, vec![CANCEL_MARKER]);
    }
}
