# Output Index

Generated artifacts.

## Assessments

- [[assess-herd-scout-2026-06-02]] — Repo vs wiki vs market gap analysis (--retardmax): 14 alignments, 10 research gaps, 19 build opportunities, 17 market gaps; immediate research commands, P0/P1/P2 build queue, anti-patterns from the OSS livestock graveyard (2026-06-02)

## Playbooks

- [[playbook-herd-scout-2026-05-20]] — OSS livestock-focused FMS strategy with phased build plan (2026-05-20)
- [[playbook-accurate-herd-counting-2026-05-27]] — Accurate herd counting from CV detections (5-layer pipeline: detection → tracking → counting → aggregation → validation; EID reconciliation as the differentiator) (2026-05-27)
- [[playbook-mot-airframe-2026-06-01]] — Round-4 P0/P1/P2 counting upgrades + buildable phone-on-drone airframe spec (2026-06-01)

## Plans

- [[plan-fms-schema-and-records-2026-06-02]] — Roadmap: P0 iroh-smol-kv FMS schema + Animal/Group/Land/Equipment + Observation/Medical/Movement/Weight/Birth log CRUD; 7 architecture decisions, 7 phases (Phase 0 schema audit → Phase 6 validation/README), egui frontend, co-location-aware SQLite projection, QR farm-namespace onboarding, iroh 0.98.0 pin (2026-06-02)
- [[plan-mobile-to-desktop-iroh-rfc-2026-05-20]] — RFC: Mobile-to-desktop app on iroh; desktop driver + Android phone-on-drone camera (2026-05-20)
- [[plan-deploy-daemon-on-1060-laptop-2026-05-22]] — Roadmap: deploy herd-scout-daemon on GTX 1060 GS63VR laptop (Ubuntu 22.04, ort+CUDA, systemd, SSH UDS forward) (2026-05-22)
- [[plan-optimize-cv-sidecar-trt-yolo11s-2026-05-26]] — Roadmap: optimize CV sidecar postprocess (YOLO11s + embedded NMS, supervision/ByteTrack, TRT 8.6 sm_61 EFFICIENT_NMS gate) (2026-05-26)
- [[plan-iroh-bound-ssh-access-daemon-2026-05-26]] — Roadmap: iroh-bound SSH access for herd-scout-daemon (third ALPN on existing Live router, NodeId allowlist, `herdctl proxy` as ssh ProxyCommand) (2026-05-26)
- [[plan-android-admin-allowlist-app-2026-05-27]] — Roadmap: Android admin app for daemon NodeId allowlist (fourth ALPN `herd-scout/admin/1`, separate `[control_plane.admins]` set, atomic `control.toml` rewrites, append-only JSONL audit log + `TailAudit` RPC + phone-side Room SQLite, single-slot fleet switcher, versioned `identity.toml` envelope with export/import via SAF, separate `com.herdscout.admin` APK) (2026-05-27)
- [[plan-desktop-video-upload-2026-05-28]] — Roadmap: desktop video upload to daemon for CV processing (fifth ALPN `herd-scout/upload/1` over iroh-blobs, sidecar file-decode mode, single-clip queue behind live phone, per-clip JSON report applying the accurate-counting playbook, GUI drag-drop + `herdctl push`) (2026-05-28)
