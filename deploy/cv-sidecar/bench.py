#!/usr/bin/env python3
"""Synthetic-frame benchmark against a running cv-sidecar over its Unix socket.

Two modes:

* Live frame mode (default) — sends N pre-generated 720p BGR24 frames at the
  wire format the live daemon uses (request_kind=0x00 prefix added in
  Phase 1 2026-05-28), times each round-trip, reports run/post/fps
  percentiles. Run after each optimization phase to confirm we still meet
  the 10 FPS daemon-cap budget.

* File-decode mode (`--file PATH`) — opens the sidecar socket, sends a
  request_kind=0x01 request with a synthetic 32-byte clip_id and the
  given absolute path, reads the probe response, then reads per-frame
  responses (with the file-mode-only `pts_ms` field) until the
  terminator. Reports throughput, p50/p95/p99 RTT for the per-frame
  responses (excluding the probe) and total wall-clock for the clip.

Usage:
    python bench.py [--socket PATH] [--frames N] [--width W] [--height H]
    python bench.py [--socket PATH] --file /abs/path/to/clip.mp4
"""
from __future__ import annotations

import argparse
import os
import socket
import struct
import sys
import time

import numpy as np

REQ_KIND = struct.Struct("<I")
REQ_KIND_FRAME = 0x00
REQ_KIND_FILE = 0x01

REQ_HDR = struct.Struct("<IIII")
RESP_HDR = struct.Struct("<II")

# File-mode wire pieces (mirror cv_sidecar.py).
FILE_REQ_CLIP_ID = struct.Struct("<32s")
FILE_REQ_PATH_LEN = struct.Struct("<I")
FILE_PROBE_TRAILER = struct.Struct("<IfII")  # frame_count, fps, width, height
FILE_FRAME_PTS = struct.Struct("<Q")  # pts_ms

SENTINEL_PROBE = 0xFFFFFFF0
SENTINEL_END = 0xFFFFFFFF
SENTINEL_ERROR = 0xFFFFFFFE


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--socket", default="/run/herd-scout/cv.sock")
    ap.add_argument("--frames", type=int, default=300)
    ap.add_argument("--width", type=int, default=1280)
    ap.add_argument("--height", type=int, default=720)
    ap.add_argument(
        "--det-pack-size",
        type=int,
        default=28,
        help="bytes per detection on the wire (28 with track_id; 24 for pre-Phase-2 sidecars)",
    )
    ap.add_argument(
        "--file",
        default=None,
        help="if set, exercise file-decode mode (request_kind=0x01) on the given absolute path",
    )
    args = ap.parse_args()

    if args.file is not None:
        return run_file_bench(args)
    return run_frame_bench(args)


def run_frame_bench(args: argparse.Namespace) -> int:
    rng = np.random.default_rng(42)
    payload = rng.integers(0, 255, size=args.height * args.width * 3, dtype=np.uint8).tobytes()

    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(args.socket)

    rtts: list[float] = []
    n_dets_total = 0
    t_start = time.perf_counter()

    for frame_id in range(args.frames):
        t0 = time.perf_counter()
        s.sendall(
            REQ_KIND.pack(REQ_KIND_FRAME)
            + REQ_HDR.pack(frame_id, args.width, args.height, len(payload))
            + payload
        )
        hdr = recv_exact(s, RESP_HDR.size)
        if hdr is None:
            print("sidecar closed connection mid-bench", file=sys.stderr)
            return 1
        _fid, n_dets = RESP_HDR.unpack(hdr)
        if n_dets:
            body = recv_exact(s, n_dets * args.det_pack_size)
            if body is None:
                print("sidecar closed mid-detection-payload", file=sys.stderr)
                return 1
        rtts.append(time.perf_counter() - t0)
        n_dets_total += n_dets

    elapsed = time.perf_counter() - t_start
    s.close()

    arr = np.array(rtts) * 1000.0  # ms
    print(f"mode:         frame (request_kind=0x00)")
    print(f"frames:       {args.frames}")
    print(f"resolution:   {args.width}x{args.height}")
    print(f"socket:       {args.socket}")
    print(f"det_pack:     {args.det_pack_size} bytes/det")
    print(f"elapsed:      {elapsed:.2f}s")
    print(f"throughput:   {args.frames / elapsed:.1f} FPS")
    print(f"rtt p50/p95/p99: {np.percentile(arr, 50):.1f} / {np.percentile(arr, 95):.1f} / {np.percentile(arr, 99):.1f} ms")
    print(f"rtt min/max:  {arr.min():.1f} / {arr.max():.1f} ms")
    print(f"avg dets/frame: {n_dets_total / args.frames:.1f}")
    return 0


def run_file_bench(args: argparse.Namespace) -> int:
    path = os.path.abspath(args.file)
    if not os.path.exists(path):
        print(f"file not found: {path}", file=sys.stderr)
        return 1

    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(args.socket)

    # Synthetic clip_id (random 32 bytes) — the bench doesn't care about
    # actual BLAKE3 here; the sidecar only logs it.
    clip_id = os.urandom(32)
    path_bytes = path.encode("utf-8")
    req = (
        REQ_KIND.pack(REQ_KIND_FILE)
        + FILE_REQ_CLIP_ID.pack(clip_id)
        + FILE_REQ_PATH_LEN.pack(len(path_bytes))
        + path_bytes
    )
    t_clip_start = time.perf_counter()
    s.sendall(req)

    # Probe response.
    hdr = recv_exact(s, RESP_HDR.size)
    if hdr is None:
        print("sidecar closed before probe", file=sys.stderr)
        return 1
    fid, n_dets = RESP_HDR.unpack(hdr)
    if fid != SENTINEL_PROBE or n_dets != 0:
        print(f"unexpected probe header: frame_id=0x{fid:08x} n_dets={n_dets}", file=sys.stderr)
        return 1
    probe_buf = recv_exact(s, FILE_PROBE_TRAILER.size)
    if probe_buf is None:
        print("sidecar closed mid-probe", file=sys.stderr)
        return 1
    frame_count, fps, width, height = FILE_PROBE_TRAILER.unpack(probe_buf)
    print(f"probe: frame_count={frame_count} fps={fps:.2f} {width}x{height}")

    rtts: list[float] = []
    n_frames = 0
    n_dets_total = 0
    last_t = time.perf_counter()

    while True:
        hdr = recv_exact(s, RESP_HDR.size)
        if hdr is None:
            print("sidecar closed before terminator", file=sys.stderr)
            return 1
        fid, n_dets = RESP_HDR.unpack(hdr)

        if fid == SENTINEL_END and n_dets == SENTINEL_END:
            break
        if fid == SENTINEL_ERROR and n_dets == SENTINEL_ERROR:
            len_buf = recv_exact(s, 4)
            if len_buf is None:
                print("sidecar closed before error reason length", file=sys.stderr)
                return 1
            (rlen,) = struct.unpack("<I", len_buf)
            reason_buf = recv_exact(s, rlen) if rlen > 0 else b""
            reason = (reason_buf or b"").decode("utf-8", errors="replace")
            print(f"sidecar error terminator: {reason}", file=sys.stderr)
            return 1

        # Per-frame response: pts_ms (u64) then det rows.
        pts_buf = recv_exact(s, FILE_FRAME_PTS.size)
        if pts_buf is None:
            print("sidecar closed mid-frame-header", file=sys.stderr)
            return 1
        (_pts_ms,) = FILE_FRAME_PTS.unpack(pts_buf)
        if n_dets:
            body = recv_exact(s, n_dets * args.det_pack_size)
            if body is None:
                print("sidecar closed mid-detection-payload", file=sys.stderr)
                return 1

        now = time.perf_counter()
        rtts.append(now - last_t)
        last_t = now
        n_frames += 1
        n_dets_total += n_dets

    elapsed = time.perf_counter() - t_clip_start
    s.close()

    print(f"mode:         file (request_kind=0x01)")
    print(f"path:         {path}")
    print(f"socket:       {args.socket}")
    print(f"frames:       {n_frames}")
    print(f"elapsed:      {elapsed:.2f}s (wall-clock incl. probe + terminator)")
    if n_frames > 0:
        arr = np.array(rtts) * 1000.0
        print(f"throughput:   {n_frames / elapsed:.1f} FPS")
        print(
            "rtt p50/p95/p99: "
            f"{np.percentile(arr, 50):.1f} / {np.percentile(arr, 95):.1f} / {np.percentile(arr, 99):.1f} ms"
        )
        print(f"rtt min/max:  {arr.min():.1f} / {arr.max():.1f} ms")
        print(f"avg dets/frame: {n_dets_total / n_frames:.1f}")
    else:
        print("(no frames decoded)")
    return 0


def recv_exact(s: socket.socket, n: int) -> bytes | None:
    buf = bytearray(n)
    view = memoryview(buf)
    pos = 0
    while pos < n:
        chunk = s.recv_into(view[pos:])
        if chunk == 0:
            return None
        pos += chunk
    return bytes(buf)


if __name__ == "__main__":
    sys.exit(main())
