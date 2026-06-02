# herd-scout

Open-source livestock-focused farm management — Rust + iroh P2P + drone vision + native mobile.

## What it does

- **Live phone-to-desktop streaming.** Pair an Android phone (CameraX → MediaCodec H.264) with a headless Linux daemon over iroh-live MoQ; an egui desktop app subscribes to the same broadcast and renders the live feed with CV overlays.
- **CV livestock counting.** A Python YOLO sidecar (YOLOv5n in production, YOLO11s + supervision.ByteTrack on the Phase-2 branch) runs on the daemon and emits per-frame detections + per-class counts for cattle, sheep, and horses.
- **Desktop video upload.** Drag-drop a clip on the GUI (or `herdctl push <node_id> <clip.mp4>`); the daemon imports the bytes via iroh-blobs, runs the same sidecar in file-decode mode, and writes a per-clip JSON report (median count, bootstrap 95% CI, per-class totals).
- **iroh-bound SSH access.** `herdctl proxy <daemon_node_id>` makes a remote daemon look like local sshd via OpenSSH `ProxyCommand` — no port forwarding, NodeId-allowlisted.
- **Android admin app.** A separate `com.herdscout.admin` flavor manages the daemon's allowlist, queries status, and tails the audit log.
- **Append-only audit log + Sigstore Rekor mirror.** Every control-plane action lands in `<data_dir>/herd-scout/audit.log`; an opt-in Wave-14 prototype mirrors Merkle-root commitments to the public Sigstore log.
- **Versioned identity envelope.** A single `identity.toml` schema (see `herd-scout-identity/`) is shared by daemon, herdctl, and the phone admin app; legacy formats migrate on first read.
- **FMS records (Phase 2 of [the FMS plan](.wiki/output/plan-fms-schema-and-records-2026-06-02.md)).** Animal / Group / Land / Equipment assets and Observation / Medical / Movement / Weight / Birth logs are stored in `herd-scout-fms`'s smol-kv-shaped on-disk store. The egui GUI's "Records" tab lists assets and creates new ones; every write fans out as a live `FmsChange` to all connected GUIs and lands in the audit log.

## Workspace layout

| Crate | Role |
|---|---|
| `herd-scout-daemon` | Headless Linux subscriber. Owns iroh + MoQ + CV sidecar + IPC + audit log + FMS records. |
| `herd-scout-gui` | egui desktop frontend. Auto-spawns the daemon if not running; renders live preview + Records tab. |
| `herd-scout-fms` | Asset / Log / Quantity FMS records on a smol-kv-shaped on-disk store with HLC-based per-field LWW, add-wins-set, and append-only conflict resolution. |
| `herd-scout-identity` | Versioned `identity.toml` envelope shared across daemon/herdctl/phone. |
| `herd-scout-ipc` | Wire types for the daemon ↔ GUI Unix-socket protocol and the iroh `herd-scout/admin/1` and `herd-scout/upload/1` ALPNs. |
| `herdctl` | iroh-bound CLI: `proxy`, `ping`, `whoami`, `push`, `uploads list/cancel/report`. |
| `android-jni` | Rust JNI cdylib for the Android phone publisher and admin app. |

## Status & accuracy commitments

- **Live mode**: 23 FPS sustained on a GTX 1060 (mobile, sm_61) with YOLO11s + supervision.ByteTrack.
- **Detection precision/recall** under good conditions: 0.90–0.95 / 0.85–0.92 (per the literature surveyed in [`livestock-cv-accuracy`](.wiki/wiki/concepts/livestock-cv-accuracy.md)).
- **Counting MAE**: ±5–10% on pasture-sized herds; ±15–25% in poor light or dense clumping. The on-the-clip 95% CI in `report.json` reflects the sample variance, not the absolute error.
- **HLC-based conflict resolution** is enforced inside the value bytes (`herd-scout-fms`), so devices with skewed wallclocks don't silently overwrite each other; see the Phase-6 drift test in `herd-scout-fms/src/lib.rs`.

## Running

```sh
# Open the egui GUI; it auto-spawns the daemon and shows a pairing QR.
cargo run -p herd-scout-gui

# Or run the daemon headless:
cargo run -p herd-scout-daemon

# CLI control:
cargo run -p herdctl -- whoami
cargo run -p herdctl -- ping <daemon_node_id>
```

Linux (production target) or macOS for development. Windows IPC is not implemented.

## Documentation

- [`.wiki/_index.md`](.wiki/_index.md) — research and design articles
- [`.wiki/output/plan-fms-schema-and-records-2026-06-02.md`](.wiki/output/plan-fms-schema-and-records-2026-06-02.md) — the active FMS implementation plan
- [`.wiki/output/assess-herd-scout-2026-06-02.md`](.wiki/output/assess-herd-scout-2026-06-02.md) — repo vs wiki vs market gap analysis
- [`deploy/README.md`](deploy/README.md) — operator deployment guide

## License

Source files declare `MIT OR Apache-2.0`. A top-level LICENSE file is pending.
