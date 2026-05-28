#!/usr/bin/env python3
"""GPU-accelerated YOLO inference sidecar for herd-scout-daemon.

Listens on a Unix socket, reads BGR24 frames in a tiny framed binary
protocol, runs ONNX Runtime CUDA EP inference, writes detections back.

Why this exists: the Rust `ort` crate wedges in static-init on this
hardware (Pascal sm_61 + Ubuntu 22.04) regardless of ORT version or
load mode. Python's `onnxruntime-gpu` 1.23 on the same box loads in
180 ms and runs YOLO11s at >25 FPS. The daemon writes BGR frames here,
gets bounding boxes back, ships them to the GUI as ServerMsg::Detections.

Model: YOLO11s exported with `nms=True` (Ultralytics export):
  input  images: [1, 3, 640, 640] fp32
  output output0: [1, 300, 6] fp32 — (x1, y1, x2, y2, conf, class) in 640-space.
  NMS runs INSIDE the ONNX graph, so postprocess is just a tensor decode.

Wire protocol (little-endian, framed binary)
============================================

Every request is now prefixed with a `request_kind: u32` selector:

  request_kind = 0x00  -> live frame mode (today's behavior, byte-identical)
  request_kind = 0x01  -> file-decode mode (new in Phase 1, 2026-05-28)

Live frame mode (0x00) — request:
    u32 request_kind = 0x00
    u32 frame_id
    u32 width
    u32 height
    u32 payload_len  (= width * height * 3)
    [payload_len bytes: BGR24, row-major top-to-bottom, contiguous]

Live frame mode (0x00) — response (UNCHANGED from pre-Phase-1):
    u32 frame_id
    u32 n_dets
    For each det:
        u32 class_id   (0=horse, 1=sheep, 2=cow)
        u32 track_id   (persistent across frames; 0xFFFFFFFF = no track)
        f32 conf
        f32 x1, y1, x2, y2  (pixel coords in source frame, top-left origin)

File-decode mode (0x01) — request (single-shot per request):
    u32 request_kind = 0x01
    [u8; 32]  clip_id_blake3   (raw 32 bytes; NOT hex)
    u32       path_len
    [utf8 bytes: absolute path readable by the sidecar process]

File-decode mode (0x01) — responses (a stream of three response shapes):

  1. Probe response (sent first, exactly once, so the daemon can enforce a
     duration cap before committing to a long decode). The 8-byte header is
     the same shape as the live response so the daemon's reader can dispatch
     on the sentinel `frame_id`:
        u32 frame_id    = 0xFFFFFFF0   (sentinel: probe)
        u32 n_dets      = 0            (no det rows)
        u32 frame_count
        f32 fps
        u32 width
        u32 height

  2. Per-decoded-frame response (one per frame, in decode order):
        u32 frame_id    = decode_index   (0, 1, 2, ...)
        u32 n_dets
        u64 pts_ms                       (cv2.CAP_PROP_POS_MSEC; 0 if unavailable)
        For each det: same 28-byte DET_PACK as live mode
     NOTE: the `pts_ms` field is appended ONLY in file mode. Live (0x00)
     responses do NOT carry pts_ms — that wire shape is sacred.

  3. Terminator (one of):
        success:   u32 frame_id = 0xFFFFFFFF, u32 n_dets = 0xFFFFFFFF
        error:     u32 frame_id = 0xFFFFFFFE, u32 n_dets = 0xFFFFFFFE,
                   u32 reason_len, [utf8 reason]

After a 0x01 request the sidecar handles only that one clip; once the
terminator is sent it loops back and reads the next 4-byte request_kind
from the same connection. Single-client invariant preserved.

Live preemption: between frames in file mode the sidecar does a
non-blocking peek on the socket. If the daemon writes a single 0x01 byte
mid-clip, that's a "cancel current clip" sentinel — the sidecar emits the
error terminator with reason "cancelled_by_daemon" and returns to the
outer request loop.

ByteTrack lifecycle:
  - Live (0x00) frames share one persistent `live_tracker` for the full
    connection (preserves today's behavior).
  - Each 0x01 clip spins up a fresh `sv.ByteTrack` with playbook params
    derived from the probe's fps; that tracker is dropped at the
    terminator. Track IDs are per-clip.

Single client. The daemon connects once at startup and the connection
lives for the daemon's lifetime. If it disconnects, the sidecar accepts
a new one. No multi-tenancy.
"""
from __future__ import annotations

import logging
import os
import socket
import struct
import sys
import time
from pathlib import Path

import cv2
import numpy as np
import onnxruntime as ort
import supervision as sv

LOG = logging.getLogger("cv-sidecar")

MODEL_INPUT_SIZE = 640
SCORE_THRESHOLD = 0.25
TRACKER_FRAME_RATE = 10  # matches the daemon's task.rs TICK = 100 ms cap
NO_TRACK_ID = 0xFFFFFFFF  # wire sentinel for "supervision returned tracker_id=None"
# COCO class indices we care about (matches herd-scout-daemon/src/cv/model.rs).
COCO_CLASS_TO_WIRE = {17: 0, 18: 1, 19: 2}  # horse, sheep, cow

# Request-kind selector (the new u32 prefix on every request).
REQ_KIND = struct.Struct("<I")
REQ_KIND_FRAME = 0x00
REQ_KIND_FILE = 0x01

# Live-frame request body (post request_kind).
REQ_HDR = struct.Struct("<IIII")  # frame_id, w, h, payload_len
# File-mode request body (post request_kind): 32-byte clip_id then path_len.
FILE_REQ_CLIP_ID = struct.Struct("<32s")
FILE_REQ_PATH_LEN = struct.Struct("<I")

RESP_HDR = struct.Struct("<II")  # frame_id, n_dets — shared by live + file modes
# class_id u32, track_id u32, conf f32, x1 f32, y1 f32, x2 f32, y2 f32 — 28 bytes
DET_PACK = struct.Struct("<IIfffff")
# File-mode-only fields:
FILE_PROBE_TRAILER = struct.Struct("<IfII")  # frame_count, fps, width, height
FILE_FRAME_PTS = struct.Struct("<Q")  # pts_ms (u64) appended after RESP_HDR in 0x01

# Sentinel frame_ids for file-mode responses.
SENTINEL_PROBE = 0xFFFFFFF0
SENTINEL_END = 0xFFFFFFFF
SENTINEL_ERROR = 0xFFFFFFFE
# Daemon -> sidecar single-byte cancel marker (peeked between frames).
CANCEL_MARKER = 0x01

# Playbook ByteTrack params for upload clips (per
# playbook-accurate-herd-counting-2026-05-27 § P0 #1; cited in
# plan-desktop-video-upload-2026-05-28 Phase 1).
CLIP_BYTETRACK_PARAMS = dict(
    track_activation_threshold=0.35,
    lost_track_buffer=60,
    minimum_matching_threshold=0.85,
    minimum_consecutive_frames=3,
)


def setup_session() -> ort.InferenceSession:
    """Create an ORT session with CUDA EP (CPU fallback)."""
    model_path = os.environ.get(
        "CV_SIDECAR_MODEL", "/srv/aiworker/yolo11s.onnx"
    )
    LOG.info("loading model: %s", model_path)
    sess_options = ort.SessionOptions()
    sess_options.log_severity_level = 3  # warnings + errors only

    # Default cap: 256 MiB. Empirical floor probed on bigdeal at 720p was
    # 160 MiB (sessions OOM at 144 MiB during the embedded-NMS ArgMax node);
    # 256 = floor + ~50% safety, rounded to a tidy power-of-two-friendly
    # number. Note: nvidia-smi will show ~340 MiB actual VRAM held —
    # the gap is model weights + cuDNN workspace + per-stream CUDA
    # state, none of which gpu_mem_limit constrains.
    gpu_mem_mib = int(os.environ.get("CV_SIDECAR_GPU_MEM_MIB", "256"))
    force_cpu = os.environ.get("CV_SIDECAR_FORCE_CPU", "").lower() in ("1", "true", "yes")

    if force_cpu:
        LOG.warning("CV_SIDECAR_FORCE_CPU set; CUDA EP disabled, CPU only")
        providers: list = ["CPUExecutionProvider"]
    else:
        providers = [
            (
                "CUDAExecutionProvider",
                {
                    "device_id": 0,
                    "arena_extend_strategy": "kNextPowerOfTwo",
                    "gpu_mem_limit": gpu_mem_mib * 1024 * 1024,
                },
            ),
            "CPUExecutionProvider",
        ]

    t0 = time.time()
    sess = ort.InferenceSession(
        model_path, sess_options=sess_options, providers=providers
    )
    LOG.info(
        "session ready in %.2fs, providers=%s, gpu_mem_cap=%d MiB",
        time.time() - t0,
        sess.get_providers(),
        gpu_mem_mib if not force_cpu else 0,
    )
    return sess


def model_input_dtype(sess: ort.InferenceSession) -> np.dtype:
    """Return the model's input dtype (fp16 vs fp32)."""
    type_str = sess.get_inputs()[0].type  # e.g. "tensor(float16)"
    if "float16" in type_str:
        return np.float16
    return np.float32


def preprocess(bgr: np.ndarray, dtype: np.dtype) -> np.ndarray:
    """BGR HxWx3 uint8 -> NCHW 1x3x640x640 normalized in `dtype`."""
    rgb = cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB)
    resized = cv2.resize(rgb, (MODEL_INPUT_SIZE, MODEL_INPUT_SIZE), interpolation=cv2.INTER_LINEAR)
    chw = resized.transpose(2, 0, 1).astype(np.float32) / 255.0
    nchw = chw[np.newaxis, ...]
    return nchw.astype(dtype, copy=False)


def postprocess(raw: np.ndarray, src_w: int, src_h: int) -> sv.Detections:
    """YOLO11 nms=True head output -> sv.Detections in source-pixel coords.

    Output shape: [1, top_k, 6] = (x1, y1, x2, y2, conf, class_id) in 640-space.
    NMS already done in-graph; we filter to horse/sheep/cow and rescale to source.
    Returns supervision Detections so the caller can run sv.ByteTrack on it.
    """
    pred = raw[0]
    conf = pred[:, 4]
    cls = pred[:, 5].astype(np.int32)

    keep = (conf >= SCORE_THRESHOLD) & np.isin(cls, list(COCO_CLASS_TO_WIRE.keys()))
    if not keep.any():
        return sv.Detections.empty()

    pred = pred[keep]
    cls = cls[keep]

    sx = src_w / float(MODEL_INPUT_SIZE)
    sy = src_h / float(MODEL_INPUT_SIZE)
    xyxy = pred[:, 0:4].astype(np.float32)
    xyxy[:, 0::2] *= sx
    xyxy[:, 1::2] *= sy

    wire_cls = np.array([COCO_CLASS_TO_WIRE[int(c)] for c in cls], dtype=np.int32)

    return sv.Detections(
        xyxy=xyxy,
        confidence=pred[:, 4].astype(np.float32),
        class_id=wire_cls,
    )


def recv_exact(conn: socket.socket, n: int) -> bytes | None:
    """Read exactly n bytes from conn. Returns None on EOF."""
    buf = bytearray(n)
    view = memoryview(buf)
    pos = 0
    while pos < n:
        chunk = conn.recv_into(view[pos:])
        if chunk == 0:
            return None
        pos += chunk
    return bytes(buf)


def pack_dets(dets: sv.Detections) -> bytes:
    """Serialize a supervision Detections to the on-wire DET_PACK[] tail."""
    n = len(dets)
    if n == 0:
        return b""
    out = bytearray(n * DET_PACK.size)
    xyxy = dets.xyxy
    cls = dets.class_id
    conf = dets.confidence
    tids = dets.tracker_id  # may be None when ByteTrack hasn't assigned yet
    for i in range(n):
        tid = NO_TRACK_ID if tids is None or tids[i] is None else int(tids[i])
        DET_PACK.pack_into(
            out,
            i * DET_PACK.size,
            int(cls[i]),
            tid,
            float(conf[i]),
            float(xyxy[i, 0]),
            float(xyxy[i, 1]),
            float(xyxy[i, 2]),
            float(xyxy[i, 3]),
        )
    return bytes(out)


def run_inference(
    bgr: np.ndarray,
    sess: ort.InferenceSession,
    in_name: str,
    in_dtype: np.dtype,
    tracker: sv.ByteTrack,
) -> sv.Detections:
    """Single-frame inference + tracker update; shared by live and file paths."""
    h, w = bgr.shape[:2]
    x = preprocess(bgr, in_dtype)
    raw = sess.run(None, {in_name: x})[0]
    dets = postprocess(raw, w, h)
    return tracker.update_with_detections(dets)


def peek_cancel(conn: socket.socket) -> bool:
    """Best-effort non-blocking peek for a single CANCEL_MARKER byte from
    the daemon. Returns True iff cancel was observed. Defensive: any error
    (including no data available) returns False without disturbing state.
    """
    try:
        # MSG_PEEK so we don't consume bytes if it isn't a cancel.
        data = conn.recv(1, socket.MSG_DONTWAIT | socket.MSG_PEEK)
    except (BlockingIOError, OSError):
        return False
    except Exception:  # noqa: BLE001 — defensive
        return False
    if not data:
        # Could be EOF; outer loop will discover that on its next real read.
        return False
    if data[0] == CANCEL_MARKER:
        # Consume the byte so it doesn't poison the next request_kind read.
        try:
            conn.recv(1)
        except OSError:
            pass
        return True
    return False


def maybe_get_orientation(cap: cv2.VideoCapture) -> int | None:
    """Return cv2.ROTATE_* constant if the source has a non-zero orientation
    metadata tag (mobile-recorded clips often do); None otherwise.

    `CAP_PROP_ORIENTATION_META` only exists in cv2 >= 4.7. On older builds
    `getattr` returns None and we silently skip rotation.
    """
    prop = getattr(cv2, "CAP_PROP_ORIENTATION_META", None)
    if prop is None:
        return None
    try:
        deg = int(round(cap.get(prop)))
    except Exception:  # noqa: BLE001 — some backends throw on missing props
        return None
    if deg == 90:
        return cv2.ROTATE_90_CLOCKWISE
    if deg == 180:
        return cv2.ROTATE_180
    if deg == 270:
        return cv2.ROTATE_90_COUNTERCLOCKWISE
    return None


def write_terminator_end(conn: socket.socket) -> None:
    conn.sendall(RESP_HDR.pack(SENTINEL_END, SENTINEL_END))


def write_terminator_error(conn: socket.socket, reason: str) -> None:
    reason_bytes = reason.encode("utf-8")
    conn.sendall(
        RESP_HDR.pack(SENTINEL_ERROR, SENTINEL_ERROR)
        + struct.pack("<I", len(reason_bytes))
        + reason_bytes
    )


def handle_live_frame(
    conn: socket.socket,
    sess: ort.InferenceSession,
    in_name: str,
    in_dtype: np.dtype,
    live_tracker: sv.ByteTrack,
    stats: dict,
) -> bool:
    """Read one 0x00-mode request, run inference, write the response.
    Returns False on EOF/protocol error so the outer loop can exit.
    Response framing here is byte-identical to the pre-Phase-1 sidecar.
    """
    hdr = recv_exact(conn, REQ_HDR.size)
    if hdr is None:
        return False
    frame_id, w, h, payload_len = REQ_HDR.unpack(hdr)
    expected = w * h * 3
    if payload_len != expected:
        LOG.error("bad payload_len: got %d, expected %d (w=%d h=%d)", payload_len, expected, w, h)
        return False

    payload = recv_exact(conn, payload_len)
    if payload is None:
        LOG.warning("client disconnected mid-payload (frame_id=%d)", frame_id)
        return False

    bgr = np.frombuffer(payload, dtype=np.uint8).reshape((h, w, 3))

    t_pre = time.time()
    x = preprocess(bgr, in_dtype)
    t_run = time.time()
    raw = sess.run(None, {in_name: x})[0]
    t_post = time.time()
    dets = postprocess(raw, w, h)
    dets = live_tracker.update_with_detections(dets)
    t_done = time.time()

    n = len(dets)
    resp = bytearray(RESP_HDR.size + n * DET_PACK.size)
    RESP_HDR.pack_into(resp, 0, frame_id, n)
    if n:
        resp[RESP_HDR.size:] = pack_dets(dets)
    conn.sendall(bytes(resp))

    stats["n_processed"] += 1
    if stats["n_processed"] % 30 == 0:
        LOG.info(
            "frame %d: pre=%.1fms run=%.1fms post=%.1fms dets=%d (rolling FPS=%.1f)",
            frame_id,
            (t_run - t_pre) * 1000,
            (t_post - t_run) * 1000,
            (t_done - t_post) * 1000,
            n,
            stats["n_processed"] / (time.time() - stats["t_start"]),
        )
    return True


def handle_file_clip(
    conn: socket.socket,
    sess: ort.InferenceSession,
    in_name: str,
    in_dtype: np.dtype,
) -> None:
    """Read one 0x01-mode request, decode the file, stream per-frame responses.

    Sends a probe response, then per-frame responses, then a terminator
    (success / error / cancelled). Uses a fresh ByteTrack instance whose
    `frame_rate` is derived from the probe; that tracker is dropped at the
    end of this function.
    """
    clip_id_buf = recv_exact(conn, FILE_REQ_CLIP_ID.size)
    if clip_id_buf is None:
        return
    (clip_id_raw,) = FILE_REQ_CLIP_ID.unpack(clip_id_buf)

    path_len_buf = recv_exact(conn, FILE_REQ_PATH_LEN.size)
    if path_len_buf is None:
        return
    (path_len,) = FILE_REQ_PATH_LEN.unpack(path_len_buf)

    if path_len == 0 or path_len > 4096:
        write_terminator_error(conn, f"bad_path_len: {path_len}")
        return

    path_bytes = recv_exact(conn, path_len)
    if path_bytes is None:
        return
    try:
        path = path_bytes.decode("utf-8")
    except UnicodeDecodeError:
        write_terminator_error(conn, "path_not_utf8")
        return

    clip_id_hex = clip_id_raw.hex()
    LOG.info("clip %s: opening %s", clip_id_hex[:16], path)

    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        LOG.warning("clip %s: cv2.VideoCapture failed to open path", clip_id_hex[:16])
        write_terminator_error(conn, f"open_failed: {path}")
        return

    try:
        # Probe — defaults if a property comes back unreliable.
        try:
            frame_count = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))
        except Exception:  # noqa: BLE001
            frame_count = 0
        if frame_count < 0:
            frame_count = 0

        try:
            fps_raw = float(cap.get(cv2.CAP_PROP_FPS))
        except Exception:  # noqa: BLE001
            fps_raw = 0.0
        fps = fps_raw if fps_raw and fps_raw > 0 else 30.0

        try:
            width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
        except Exception:  # noqa: BLE001
            width = 0
        try:
            height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
        except Exception:  # noqa: BLE001
            height = 0

        rotate_const = maybe_get_orientation(cap)

        # Probe response: header (sentinel) + 16-byte trailer.
        probe = (
            RESP_HDR.pack(SENTINEL_PROBE, 0)
            + FILE_PROBE_TRAILER.pack(frame_count, fps, width, height)
        )
        conn.sendall(probe)
        LOG.info(
            "clip %s: probe frame_count=%d fps=%.2f w=%d h=%d rotate=%s",
            clip_id_hex[:16],
            frame_count,
            fps,
            width,
            height,
            rotate_const,
        )

        # Per-clip ByteTrack instance using playbook params.
        clip_tracker = sv.ByteTrack(
            frame_rate=int(round(fps)) if fps > 0 else 30,
            **CLIP_BYTETRACK_PARAMS,
        )

        decode_index = 0
        t_clip_start = time.time()

        while True:
            # Live preemption: between frames, see if the daemon sent a
            # cancel byte. Defensive — never crash if the peek fails.
            try:
                if peek_cancel(conn):
                    LOG.info("clip %s: cancelled_by_daemon at frame %d", clip_id_hex[:16], decode_index)
                    write_terminator_error(conn, "cancelled_by_daemon")
                    return
            except Exception:  # noqa: BLE001 — paranoid
                pass

            ok, bgr = cap.read()
            if not ok or bgr is None:
                # End of stream (clean EOF).
                break

            if rotate_const is not None:
                bgr = cv2.rotate(bgr, rotate_const)

            # CAP_PROP_POS_MSEC is updated *after* each read.
            try:
                pts_ms_raw = cap.get(cv2.CAP_PROP_POS_MSEC)
            except Exception:  # noqa: BLE001
                pts_ms_raw = 0.0
            pts_ms = int(pts_ms_raw) if pts_ms_raw and pts_ms_raw > 0 else 0

            try:
                dets = run_inference(bgr, sess, in_name, in_dtype, clip_tracker)
            except Exception as e:  # noqa: BLE001 — surface, don't crash sidecar
                LOG.exception("clip %s: inference failed at frame %d", clip_id_hex[:16], decode_index)
                write_terminator_error(conn, f"inference_failed: {e!r}")
                return

            n = len(dets)
            head = RESP_HDR.pack(decode_index, n) + FILE_FRAME_PTS.pack(pts_ms)
            tail = pack_dets(dets) if n else b""
            conn.sendall(head + tail)

            decode_index += 1
            if decode_index % 100 == 0:
                elapsed = time.time() - t_clip_start
                LOG.info(
                    "clip %s: %d frames decoded (%.1f FPS)",
                    clip_id_hex[:16],
                    decode_index,
                    decode_index / elapsed if elapsed > 0 else 0.0,
                )

        elapsed = time.time() - t_clip_start
        LOG.info(
            "clip %s: end-of-stream after %d frames in %.1fs (%.1f FPS)",
            clip_id_hex[:16],
            decode_index,
            elapsed,
            decode_index / elapsed if elapsed > 0 else 0.0,
        )
        write_terminator_end(conn)
    finally:
        try:
            cap.release()
        except Exception:  # noqa: BLE001
            pass


def serve_one_connection(conn: socket.socket, sess: ort.InferenceSession) -> None:
    in_name = sess.get_inputs()[0].name
    in_dtype = model_input_dtype(sess)
    # Persistent live tracker across the whole connection (preserves today's
    # behavior). File-mode clips spin up their own tracker per clip.
    # sv.ByteTrack is deprecated in supervision >= 0.30 (we have 0.28). When
    # we upgrade, this constructor will need to move to sv.Tracker — the
    # update_with_detections call shape is identical.
    live_tracker = sv.ByteTrack(frame_rate=TRACKER_FRAME_RATE)
    LOG.info("client connected; model input dtype=%s; live tracker=ByteTrack@%dfps", in_dtype, TRACKER_FRAME_RATE)

    stats = {"n_processed": 0, "t_start": time.time()}

    while True:
        kind_buf = recv_exact(conn, REQ_KIND.size)
        if kind_buf is None:
            LOG.info(
                "client disconnected after %d frames in %.1fs",
                stats["n_processed"],
                time.time() - stats["t_start"],
            )
            return
        (kind,) = REQ_KIND.unpack(kind_buf)

        if kind == REQ_KIND_FRAME:
            if not handle_live_frame(conn, sess, in_name, in_dtype, live_tracker, stats):
                return
        elif kind == REQ_KIND_FILE:
            handle_file_clip(conn, sess, in_name, in_dtype)
            # Loop back and read the next request_kind from the same connection.
        else:
            LOG.error("unknown request_kind=0x%02x; closing connection", kind)
            return


def main() -> int:
    logging.basicConfig(
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
        level=logging.INFO,
        stream=sys.stdout,
    )

    sock_path = os.environ.get("CV_SIDECAR_SOCKET", "/run/herd-scout/cv.sock")
    sock_dir = Path(sock_path).parent
    sock_dir.mkdir(parents=True, exist_ok=True)
    if Path(sock_path).exists():
        Path(sock_path).unlink()

    sess = setup_session()

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(sock_path)
    os.chmod(sock_path, 0o660)
    server.listen(1)
    LOG.info("cv-sidecar listening on %s", sock_path)

    try:
        while True:
            conn, _addr = server.accept()
            try:
                serve_one_connection(conn, sess)
            finally:
                conn.close()
    except KeyboardInterrupt:
        LOG.info("shutdown via Ctrl-C")
        return 0
    finally:
        server.close()
        try:
            Path(sock_path).unlink()
        except FileNotFoundError:
            pass


if __name__ == "__main__":
    sys.exit(main())
