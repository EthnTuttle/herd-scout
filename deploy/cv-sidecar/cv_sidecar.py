#!/usr/bin/env python3
"""GPU-accelerated YOLOv5n inference sidecar for herd-scout-daemon.

Listens on a Unix socket, reads BGR24 frames in a tiny framed binary
protocol, runs ONNX Runtime CUDA EP inference, writes detections back.

Why this exists: the Rust `ort` crate wedges in static-init on this
hardware (Pascal sm_61 + Ubuntu 22.04) regardless of ORT version or
load mode. Python's `onnxruntime-gpu` 1.23 on the same box loads in
180 ms and runs YOLOv5n at 59 FPS. The daemon writes BGR frames here,
gets bounding boxes back, ships them to the GUI as ServerMsg::Detections.

Wire protocol (little-endian, framed binary)
============================================

Daemon -> sidecar (request):
    u32 frame_id
    u32 width
    u32 height
    u32 payload_len  (= width * height * 3)
    [payload_len bytes: BGR24, row-major top-to-bottom, contiguous]

Sidecar -> daemon (response):
    u32 frame_id
    u32 n_dets
    For each det:
        u32 class_id   (0=horse, 1=sheep, 2=cow)
        f32 conf
        f32 x1, y1, x2, y2  (pixel coords in source frame, top-left origin)

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

LOG = logging.getLogger("cv-sidecar")

MODEL_INPUT_SIZE = 640
NMS_IOU_THRESHOLD = 0.45
SCORE_THRESHOLD = 0.25
# COCO class indices we care about (matches herd-scout-daemon/src/cv/model.rs).
COCO_CLASS_TO_WIRE = {17: 0, 18: 1, 19: 2}  # horse, sheep, cow

REQ_HDR = struct.Struct("<IIII")  # frame_id, w, h, payload_len
RESP_HDR = struct.Struct("<II")  # frame_id, n_dets
# class_id u32, conf f32, x1 f32, y1 f32, x2 f32, y2 f32 — 24 bytes total
DET_PACK = struct.Struct("<Ifffff")


def setup_session() -> ort.InferenceSession:
    """Create an ORT session with CUDA EP (CPU fallback)."""
    model_path = os.environ.get(
        "CV_SIDECAR_MODEL", "/srv/aiworker/yolov5n.onnx"
    )
    LOG.info("loading model: %s", model_path)
    sess_options = ort.SessionOptions()
    sess_options.log_severity_level = 3  # warnings + errors only

    providers = [
        (
            "CUDAExecutionProvider",
            {"device_id": 0, "arena_extend_strategy": "kSameAsRequested"},
        ),
        "CPUExecutionProvider",
    ]
    t0 = time.time()
    sess = ort.InferenceSession(
        model_path, sess_options=sess_options, providers=providers
    )
    LOG.info(
        "session ready in %.2fs, providers=%s",
        time.time() - t0,
        sess.get_providers(),
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
    # YOLOv5 ONNX export expects RGB, normalized to [0, 1].
    rgb = cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB)
    resized = cv2.resize(rgb, (MODEL_INPUT_SIZE, MODEL_INPUT_SIZE), interpolation=cv2.INTER_LINEAR)
    chw = resized.transpose(2, 0, 1).astype(np.float32) / 255.0
    nchw = chw[np.newaxis, ...]
    return nchw.astype(dtype, copy=False)


def postprocess(
    raw: np.ndarray,
    src_w: int,
    src_h: int,
    score_threshold: float = SCORE_THRESHOLD,
    iou_threshold: float = NMS_IOU_THRESHOLD,
) -> list[tuple[int, float, float, float, float, float]]:
    """YOLOv5 head output -> list of (wire_class, conf, x1, y1, x2, y2) in source-pixel coords.

    YOLOv5 ONNX output shape: [1, 25200, 85] = batch, anchors, (cx, cy, w, h, obj, *cls80).
    Filters to horse/sheep/cow only; runs class-aware NMS.
    """
    pred = raw[0]  # (25200, 85)
    obj = pred[:, 4]
    cls_scores = pred[:, 5:]
    scores = cls_scores * obj[:, None]  # (25200, 80)

    # Pick per-anchor best class
    best_cls = scores.argmax(axis=1)  # (25200,)
    best_score = scores[np.arange(scores.shape[0]), best_cls]

    # Filter: score over threshold AND class is one we care about
    keep_mask = (best_score >= score_threshold) & np.isin(best_cls, list(COCO_CLASS_TO_WIRE.keys()))
    if not keep_mask.any():
        return []

    pred = pred[keep_mask]
    best_cls = best_cls[keep_mask]
    best_score = best_score[keep_mask]

    # cxcywh -> xyxy in 640-space
    cx, cy, ww, hh = pred[:, 0], pred[:, 1], pred[:, 2], pred[:, 3]
    x1 = cx - ww / 2
    y1 = cy - hh / 2
    x2 = cx + ww / 2
    y2 = cy + hh / 2

    # OpenCV NMSBoxes expects (x, y, w, h) with native floats.
    # Per-class NMS: run NMSBoxes once per class, concat the kept indices.
    boxes_xywh = np.stack([x1, y1, ww, hh], axis=1).astype(np.float32)
    scores_f = best_score.astype(np.float32)

    keep_indices: list[int] = []
    for cls in np.unique(best_cls):
        cls_mask = best_cls == cls
        idxs = np.where(cls_mask)[0]
        kept = cv2.dnn.NMSBoxes(
            boxes_xywh[idxs].tolist(),
            scores_f[idxs].tolist(),
            score_threshold,
            iou_threshold,
        )
        if len(kept) == 0:
            continue
        kept = np.asarray(kept).flatten()
        keep_indices.extend(idxs[kept].tolist())

    if not keep_indices:
        return []
    keep_idx = np.asarray(keep_indices, dtype=np.int64)

    # Scale 640-space xyxy back to source-frame pixels
    sx = src_w / float(MODEL_INPUT_SIZE)
    sy = src_h / float(MODEL_INPUT_SIZE)

    out: list[tuple[int, float, float, float, float, float]] = []
    for k in keep_idx:
        coco_cls = int(best_cls[k])
        wire_cls = COCO_CLASS_TO_WIRE[coco_cls]
        out.append(
            (
                wire_cls,
                float(best_score[k]),
                float(x1[k] * sx),
                float(y1[k] * sy),
                float(x2[k] * sx),
                float(y2[k] * sy),
            )
        )
    return out


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


def serve_one_connection(conn: socket.socket, sess: ort.InferenceSession) -> None:
    in_name = sess.get_inputs()[0].name
    in_dtype = model_input_dtype(sess)
    LOG.info("client connected; model input dtype=%s", in_dtype)

    n_processed = 0
    t_start = time.time()

    while True:
        hdr = recv_exact(conn, REQ_HDR.size)
        if hdr is None:
            LOG.info("client disconnected after %d frames in %.1fs", n_processed, time.time() - t_start)
            return
        frame_id, w, h, payload_len = REQ_HDR.unpack(hdr)
        expected = w * h * 3
        if payload_len != expected:
            LOG.error("bad payload_len: got %d, expected %d (w=%d h=%d)", payload_len, expected, w, h)
            return

        payload = recv_exact(conn, payload_len)
        if payload is None:
            LOG.warning("client disconnected mid-payload (frame_id=%d)", frame_id)
            return

        bgr = np.frombuffer(payload, dtype=np.uint8).reshape((h, w, 3))

        t_pre = time.time()
        x = preprocess(bgr, in_dtype)
        t_run = time.time()
        raw = sess.run(None, {in_name: x})[0]
        t_post = time.time()
        dets = postprocess(raw, w, h)
        t_done = time.time()

        # Pack response
        resp = bytearray(RESP_HDR.size + len(dets) * DET_PACK.size)
        RESP_HDR.pack_into(resp, 0, frame_id, len(dets))
        for i, (cls, conf, x1, y1, x2, y2) in enumerate(dets):
            DET_PACK.pack_into(resp, RESP_HDR.size + i * DET_PACK.size, cls, conf, x1, y1, x2, y2)
        conn.sendall(bytes(resp))

        n_processed += 1
        if n_processed % 30 == 0:
            LOG.info(
                "frame %d: pre=%.1fms run=%.1fms post=%.1fms dets=%d (rolling FPS=%.1f)",
                frame_id,
                (t_run - t_pre) * 1000,
                (t_post - t_run) * 1000,
                (t_done - t_post) * 1000,
                len(dets),
                n_processed / (time.time() - t_start),
            )


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
