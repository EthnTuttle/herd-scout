#!/usr/bin/env python3
"""Standalone smoke test for cv-sidecar.

Connects to the running sidecar's Unix socket, sends a single synthetic
640x480 BGR frame, prints what comes back. Used to verify the wire
protocol + GPU path independently of the Rust daemon.

Usage:
    python3 smoke_client.py [path/to/cv.sock]
"""
import socket
import struct
import sys
import time
from pathlib import Path

import numpy as np

REQ_KIND = struct.Struct("<I")
REQ_KIND_FRAME = 0x00  # live frame mode (Phase 1, 2026-05-28: now an explicit prefix)
REQ_HDR = struct.Struct("<IIII")
RESP_HDR = struct.Struct("<II")
# class_id u32, track_id u32, conf f32, x1 f32, y1 f32, x2 f32, y2 f32
DET_PACK = struct.Struct("<IIfffff")


def main() -> int:
    sock_path = sys.argv[1] if len(sys.argv) > 1 else "/run/herd-scout/cv.sock"
    if not Path(sock_path).exists():
        print(f"socket not found: {sock_path}", file=sys.stderr)
        return 1

    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(sock_path)
    print(f"connected to {sock_path}")

    # Synthetic BGR frame: 640x480, mid-grey
    w, h = 640, 480
    bgr = np.full((h, w, 3), 128, dtype=np.uint8)
    payload = bgr.tobytes()
    assert len(payload) == w * h * 3

    for frame_id in range(5):
        kind = REQ_KIND.pack(REQ_KIND_FRAME)
        hdr = REQ_HDR.pack(frame_id, w, h, len(payload))
        t0 = time.time()
        s.sendall(kind + hdr + payload)

        resp_hdr = b""
        while len(resp_hdr) < RESP_HDR.size:
            chunk = s.recv(RESP_HDR.size - len(resp_hdr))
            if not chunk:
                print("server closed connection")
                return 1
            resp_hdr += chunk
        rcv_id, n_dets = RESP_HDR.unpack(resp_hdr)
        assert rcv_id == frame_id, f"frame_id mismatch: sent {frame_id}, got {rcv_id}"

        det_bytes = b""
        det_total = n_dets * DET_PACK.size
        while len(det_bytes) < det_total:
            chunk = s.recv(det_total - len(det_bytes))
            if not chunk:
                print("server closed mid-detection")
                return 1
            det_bytes += chunk

        rtt_ms = (time.time() - t0) * 1000
        dets = []
        for i in range(n_dets):
            cls, tid, conf, x1, y1, x2, y2 = DET_PACK.unpack_from(det_bytes, i * DET_PACK.size)
            dets.append((cls, tid, conf, x1, y1, x2, y2))
        print(f"frame {frame_id}: rtt={rtt_ms:.1f} ms, n_dets={n_dets} {dets[:3]}")

    s.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
