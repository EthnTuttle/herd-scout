---
title: "Repo Comparison: herd-scout vs herd-scout Wiki vs Market"
type: comparison
sources: [
  "wiki/concepts/herd-scout-positioning",
  "wiki/concepts/oss-fms-landscape",
  "wiki/concepts/livestock-oss-gap-analysis",
  "wiki/concepts/livestock-eid-rfid",
  "wiki/concepts/fms-feature-taxonomy",
  "wiki/concepts/oss-drone-fms-pipeline",
  "wiki/concepts/herd-counting-pipeline",
  "wiki/concepts/iroh-sync-stack",
  "wiki/concepts/mobile-desktop-architecture",
  "wiki/concepts/livestock-cv-accuracy",
  "wiki/concepts/cattle-reid-self-supervised",
  "wiki/concepts/yolo26-and-tracker-compat",
  "wiki/concepts/track-recovery-busca-hit",
  "output/playbook-mot-airframe-2026-06-01"
]
mode: assess --retardmax
generated: 2026-06-02
---

# herd-scout vs Local Wiki vs Market — Gap Analysis (--retardmax)

## Executive Summary

**herd-scout has built the harder half first.** The repo today is a sophisticated P2P video-streaming + CV-counting platform: 5 iroh ALPNs (MoQ, gossip, SSH bridge, admin RPC, blob upload), a versioned identity envelope, an append-only audit log with optional Sigstore Rekor mirror, a Wave-14 `AccessLimit` predicate gate at the router layer, an Android phone publisher (CameraX + MediaCodec H.264), an egui desktop subscriber, a herdctl iroh-bound CLI, and a Python YOLO sidecar invoked over a Unix-domain socket because in-process `ort` deadlocks on Pascal GPUs. Waves 1–14 have shipped. None of this is generic OSS-FMS plumbing — it is the technically risky set herd-scout cannot consume from anywhere else.

**The FMS half — the part the wiki argues is the actual differentiator — is not yet in the codebase.** The wiki's strongest wedge claim is "OSS livestock-FMS that closes the EID-reconciliation loop and ships a native offline-first mobile app on top of farmOS-compatible records." The repo currently has zero `Animal`/`Group`/`Land`/`Log` records, no iroh-docs/iroh-smol-kv KV CRDT wired up (despite being declared as the data plane), no EID reader bridge, no paddock map, no farmOS-compat JSON:API export, and no compliance reporting. The desktop frontend is egui, not Tauri 2 (a wiki/repo mismatch). Drone integration is research-only — buildable BOM and post-hoc playbooks live in `output/`, but no MAVLink or ODM ingest exists in `src/`. The CV pipeline implements layers 1–2 of the wiki's five-layer counting stack; layers 3 (counting policy), 4 (aggregation), and 5 (validation/EID-reconciliation) are designs only.

**The market gap herd-scout claims is real and getting more crowded.** Closed incumbents (AgriWebb, Performance Beef, 701x, Datamars Livestock, Allflex SenseHub) own ranch UX with monthly subscriptions; OSS competitors (farmOS, LiteFarm, Ekylibre) have stable but web-PWA-first UX with no livestock-CV integration; abandoned attempts (Tania, OpenFarm, OpenATK, Piquetear, AVAT, Livestock Tracker, NSIP-example) littered the field 2019–2025. Distinct from all of them: nobody — open or closed — currently ships the integrated stack of `(Rust + P2P + drone-CV + EID-reconciliation + native-mobile)`. Recommended actions cluster into three priorities: (P0) ship a minimum farmOS-compatible record layer over iroh-smol-kv so the existing video/CV/audit infrastructure has a place to write; (P1) the EID Bluetooth-reader Rust crate (weekend MVP per the wiki); (P2) the count-validation layer (conformal + EID reconciliation) that converts the existing detections into the auditable count-with-confidence the wiki specifies.

## Repo Overview

- **What it is**: P2P livestock-monitoring system — Android phone (CameraX → H.264 MediaCodec → iroh-live MoQ) publishes video to a headless Linux daemon that runs Python-sidecar YOLO + ByteTrack inference and exposes the result to an egui desktop subscriber over Unix-domain IPC.
- **Tech stack**:
  - Rust workspace (10+ crates) on iroh 0.98 + iroh-live (vendored) + iroh-moq + moq-lite.
  - Frontend: **egui/eframe 0.33** (NOT Tauri 2, contra wiki).
  - Mobile: Android (Kotlin + CameraX + ML Kit QR) with Rust JNI cdylib, `cargo-ndk`. arm64-v8a only. iOS not present.
  - CV: Python sidecar — `onnxruntime-gpu` 1.23, YOLOv5n in production, YOLO11s + supervision.ByteTrack on `feat/cv-sidecar-phase2`. CUDA on GTX 1060 mobile.
  - Identity: shared `herd-scout-identity` crate, versioned TOML envelope (Wave 12 Phase 0), ed25519 secret, EndpointId integrity check.
  - Audit: append-only JSONL, daily rotation, 90-day retention, Wave-14 Sigstore Rekor mirror (feature-gated, prototype).
  - Build: `cargo` for daemon/gui/herdctl, Gradle + cargo-ndk for Android, no `.github/workflows/`.
- **Key features (implemented)**:
  1. Phone-to-daemon MoQ video publish + receive over public NAT
  2. Live YOLO inference offload to Python sidecar
  3. egui live preview with bbox overlays + class counts
  4. iroh-bound SSH (Wave 11) — `herdctl proxy` as `ProxyCommand`
  5. Android admin app (Wave 12) — allowlist CRUD, status, audit tail (with local Room cache)
  6. Versioned identity envelope shared by daemon, herdctl, phone (Wave 12 Phase 0)
  7. Append-only audit log + Sigstore Rekor mirror (Wave 14, feature-gated)
  8. AccessLimit router-layer predicate gate (Wave 14)
  9. Desktop video upload over fifth ALPN (Wave 13) — file → iroh-blobs → sidecar decode → `report.json` with bootstrap CI
  10. Five iroh ALPNs, all NodeId-allowlisted at the router layer
- **Key features (NOT yet implemented despite wiki coverage)**:
  - Animal / Group / Land / Equipment record types and CRUD UX
  - Observation / Medical / Movement / Weight / Birth log types
  - iroh-smol-kv (or iroh-docs) KV CRDT (declared as the planned data plane in the playbook; not wired)
  - Paddock polygon mapping (no GIS / GeoJSON / map widget)
  - farmOS-compat JSON:API import/export
  - Bluetooth EID reader bridge
  - Compliance exports (USDA 840, NLIS, CCIA, EU passport)
  - Drone-vision integration (no MAVLink, no ODM ingest, no QGroundControl bridge)
  - Phone-on-drone airframe playbook artifacts (BOM only — no shipped airframe code)
  - Counting layers 3 (deployment-aware policy), 4 (Lincoln-Petersen / N-mixture aggregation), 5 (conformal + EID reconciliation)
  - Pasture rotation planner
  - iOS / Tauri 2 / Compose Multiplatform path

## Alignment (Repo + Wiki agree)

| Feature | Repo Implementation | Wiki Research | Notes |
|---|---|---|---|
| Rust as primary language | 10-crate workspace on iroh 0.98 | [[herd-scout-positioning]] §1: "Rust-native FMS" | Wedge fully held in code |
| iroh transport, dial-by-NodeId, hole-punch + relay | `iroh::Endpoint` in daemon, herdctl, admin, phone | [[iroh-sync-stack]], [[mobile-desktop-architecture]] | Holds |
| Per-device ed25519 author keys | `herd-scout-identity` versioned TOML on each peer | [[iroh-docs-fms-schema]] "per-device authors" | Holds |
| YOLO + ByteTrack live counting | Python sidecar at `/run/herd-scout/cv.sock`, supervision.ByteTrack | [[herd-counting-pipeline]] L1+L2, [[cv-sidecar-bench-2026-05-27]], [[tracker-choice-bot-oc-byte]] | Live mode hits ~23 FPS sustained on GTX 1060 — matches wiki spec |
| YOLO11s + embedded NMS over YOLOv5n | `feat/cv-sidecar-phase2` branch present | [[yolo26-and-tracker-compat]] "embed NMS, prefer end2end head" | Holds (in-flight, default still YOLOv5n) |
| Bootstrap confidence interval for counts | `upload::report::ClipReport::build` emits `bootstrap_ci_95` | [[bootstrap-conformal-count-ci]] block bootstrap + BCa | Implemented in upload mode; live-mode aggregation still missing |
| Append-only audit log + Sigstore Rekor mirror | `audit.rs`, `rekor-mirror` Cargo feature | [[output/plan-android-admin-allowlist-app-2026-05-27]] §audit, Wave 14 commit | Holds (prototype) |
| Android FGS `camera\|connectedDevice` | Wave 14 manifest entries | [[phone-publisher-android-fgs]], [[phone-thermal-management]] | Holds |
| Iroh-bound SSH over Wave-11 ALPN | `control.rs` byte-pump to 127.0.0.1:22 | [[output/plan-iroh-bound-ssh-access-daemon-2026-05-26]] | Holds |
| Admin allowlist split (`[control_plane.admins]` vs `[control_plane.allowed]`) | `control.toml` two distinct lists | [[output/plan-android-admin-allowlist-app-2026-05-27]] §separate-allowlists | Holds |
| Versioned identity envelope shared by daemon/herdctl/phone | `herd_scout_identity` crate | [[output/plan-android-admin-allowlist-app-2026-05-27]] Phase 0 | Holds |
| Desktop video upload over iroh-blobs | Wave 13, fifth ALPN `herd-scout/upload/1` | [[output/plan-desktop-video-upload-2026-05-28]] | Holds |
| Single-clip queue behind live phone | `upload::queue` enforces ordering | [[output/plan-desktop-video-upload-2026-05-28]] | Holds |
| Per-clip JSON report with median active count | `ClipReport::build` returns `median_active_count` | [[playbook-accurate-herd-counting-2026-05-27]] §counting-policy | Holds — but only for upload mode |

## Research Gaps (Repo does it, Wiki doesn't cover it)

| Repo Feature | What's missing from wiki | Suggested research |
|---|---|---|
| **MoQ over iroh-live** as the live video transport | Wiki has [[iroh-sync-stack]] for KV CRDT but no article on `iroh-moq` / MoQ broadcast / `moq-lite` choice rationale, latency tuning, codec config | `/wiki:research "iroh-moq vs iroh-live MoQ vs alternative low-latency video over QUIC for farm/drone use"` |
| **Python CV sidecar over Unix-domain socket** because in-process `ort` deadlocks on Pascal | No wiki article documents the Pascal/`ort` deadlock root cause, the binary wire protocol design, or sidecar resilience patterns | `/wiki:research "rust ort onnxruntime pascal sm_61 deadlock and python sidecar IPC patterns"` |
| **Wave 14 AccessLimit router-layer predicate** as a defense-in-depth pattern | No wiki article covers the iroh `AccessLimit` API, when Endpoint-level vs ProtocolHandler-level vs predicate gates are appropriate, or the Wave-14 identity threat model | `/wiki:research "iroh access control patterns: AccessLimit predicate, ProtocolHandler gate, Endpoint allowlist"` |
| **Sigstore Rekor mirror of audit log** as a tamper-evidence design | No wiki article on Rekor for non-binary-signing use cases, transparency logs as audit pinning, key management, replay safety | `/wiki:research "sigstore rekor for transparency logs of farm/IoT audit data"` |
| **Versioned TOML identity envelope** shared cross-language (Rust + Kotlin/JNI) | No wiki article on the design of an identity envelope schema, migration story, integrity-check approach (NodeId echoed in file) | `/wiki:research "cross-language local identity file design with version migration and integrity check"` |
| **`herdctl proxy` as `ProxyCommand` for OpenSSH** — the iroh-bound SSH bridge UX | Wiki [[output/plan-iroh-bound-ssh-access-daemon-2026-05-26]] has the daemon side; not documented from the *operator* side or compared to `tailscale ssh` / `cloudflared access ssh` | `/wiki:research "iroh QUIC bi-stream as OpenSSH ProxyCommand vs tailscale ssh vs cloudflared access ssh"` |
| **JNI bridge with shared Tokio runtime** for Android admin app | Wiki has [[mobile-desktop-architecture]] but doesn't cover JNI cdylib + cargo-ndk + shared `ADMIN_RUNTIME` patterns | `/wiki:research "shared tokio runtime in JNI cdylib for android: lifecycle, panic handling, leaked threads"` |
| **Desktop video upload + per-clip report** as a separate processing mode | Wiki [[output/plan-desktop-video-upload-2026-05-28]] covers it but counting-policy article ([[herd-counting-pipeline]]) is live-mode-centric — no article on file-mode aggregation differences (e.g., closure assumption is *valid* in upload mode) | `/wiki:research "offline video file herd counting: closure-assumption-valid Lincoln-Petersen variants"` |
| **Daemon dependency on Python sidecar** as a systemd ordering / failure-mode question | No wiki article on systemd unit composition, sidecar restart policy, OOM behavior, version compatibility between Rust daemon and Python wire format | `/wiki:research "systemd unit dependency between rust daemon and python ML sidecar: restart policy, version pinning, OOM"` |
| **GTX 1060 (sm_61) as a deployment target** in the daemon plan | [[output/plan-deploy-daemon-on-1060-laptop-2026-05-22]] covers the box but no article surveys long-term Pascal viability vs Jetson Nano vs Orin Nano vs CPU-only fallback | `/wiki:research "pascal sm_61 lifecycle for ml inference vs jetson orin nano vs cpu-only onnxruntime"` |

## Opportunities (Wiki knows it, Repo doesn't do it)

| Wiki Knowledge | Potential feature | Priority | Complexity |
|---|---|---|---|
| [[fms-data-model]] Asset/Log/Quantity/Term/Plan | **P0**: Animal / Group / Land / Equipment record CRDT records on iroh-smol-kv, JSON:API export/import compatible with farmOS | **P0** — without it, all the other wedges are inert | Medium (define schema, wire iroh-smol-kv, build minimal CRUD UI) |
| [[livestock-eid-rfid]] §"weekend MVP" | **P0–P1**: `herd-scout-eid` Rust crate — `serialport` + `bluer` + `btleplug` line-buffered parser for ISO 11784/11785 ASCII over Bluetooth SPP / BLE Nordic UART | **P0** — the strongest unique wedge per the positioning article | Low (wiki says "weekend MVP" for ~70-80% reader coverage) |
| [[herd-counting-pipeline]] L3 counting policies | Per-deployment counting policy: `MaxDistinctIDs`, `WorldCoordDedup`, `MedianActiveWindow`, `LineZone`, `MultiCamPool` — the wiki has the decision matrix | **P1** — current upload mode hard-codes one strategy | Medium |
| [[herd-counting-pipeline]] L4 aggregation | Lincoln-Petersen / Chapman estimator + Royle 2004 N-mixture for multi-pass | **P2** — needed for drone multi-pass | Medium (closed-form formulas; bootstrap CI already wired) |
| [[herd-counting-pipeline]] L5 validation | Conformal prediction + 🟢/🟡/🔴 confidence chip; EID reconciliation residual | **P1** (conformal solo) → **P0** once EID lands (reconciliation is the wedge) | Low (split conformal is ~30 lines) |
| [[cattle-reid-self-supervised]] | DINOv2/v3-based per-farm self-supervised cattle ReID embeddings; replaces ResNet50 stub | **P2** | Medium (per-farm fine-tune; needs datasets) |
| [[track-recovery-busca-hit]] | BUSCA online + HIT post-hoc tracklet recovery to fix re-entry double-count | **P2** | Medium |
| [[tracker-choice-bot-oc-byte]] | OC-SORT A/B test in upload-mode | **P2** | Low (already lined up in playbook) |
| [[livestock-cv-accuracy]] | Documented accuracy commitments (±5–10% pasture, ±15–25% bad-case) on the public README | **P1** — public claims | Low (text only) |
| [[oss-fms-landscape]] §farmOS-compat | farmOS JSON:API consumer/producer to make herd-scout interoperable with the dominant OSS FMS | **P1** — keeps the door open to farmOS users | Medium |
| [[oss-drone-fms-pipeline]] §L5 | WebODM / DroneDB → herd-scout ingest module: read ODM outputs, attach orthos to paddocks, run zonal stats, write Observation logs | **P2** — owns the broken-link gap | Medium (Python sidecar already exists; a second decoder mode for orthos fits) |
| [[fms-feature-taxonomy]] §6 spatial | Paddock polygon CRUD over GeoJSON, animal-last-seen pin, offline map tiles | **P1** | Medium (need a map widget — egui-map, slint, or Tauri 2 leaflet) |
| [[fms-feature-taxonomy]] §2 logs | Treatment log w/ withdrawal flag (EU/AU regulatory minimum) | **P1** | Low |
| [[mobile-desktop-architecture]] §Tauri 2 | Tauri 2 frontend reuse for iOS — current egui frontend is desktop-only | **P2** — reverses the egui choice; high-cost decision | High (rewrite GUI) |
| [[android-on-drone]] / [[phone-on-drone-airframe]] | MAVLink + 4G bridge wired into the existing iroh-live publisher; phone-on-drone airframe | **P2** | High (needs MAVLink Rust crate; airframe build) |
| [[phone-power-on-drone]] / [[phone-thermal-management]] | Foreground-service thermal listener + sustained-perf throttling already specified in playbook | **P1** for drone deployment, **P2** otherwise | Low (Wave 14 manifest has the FGS pieces; thermal listener pending) |
| [[iroh-docs-fms-schema]] | Concrete iroh-smol-kv key layout (`asset/<id>/name`, append-only logs, blob refs) | **P0** prerequisite to record types | Medium |
| [[implementation-plan]] | The original ~$530 BOM end-to-end MVP build | **P2** — operator playbook | Low (text only) |
| [[ag-data-standards]] | ICAR/ADAPT/USDA-840 export adapters | **P2** | Medium-High |

## Market Gaps (Neither covers, but competitors/market does)

| Capability | Who has it | Relevance | Notes |
|---|---|---|---|
| Solar GPS / virtual-fence collars + per-animal real-time location | **Halter** (NZ/AU/US, satellite SKU 2026), **Vence** (Merck), **701x** (xT/xTpro/xTlite) | High — collar telemetry could feed herd-scout records | Wiki has nothing on collar hardware integration; herd-scout could be the OSS aggregator over collar manufacturers |
| In-parlour BCS / mobility / lameness scoring from fixed cameras | **CattleEye** (AWS-backed), **Allflex SenseHub** (MSD/Merck), **CattleCare/Cainthus** (now Ever.Ag) | Medium — adjacent CV problem, dairy-skewed | Wiki has [[livestock-cv-accuracy]] but no article on dairy-parlour BCS — different camera geometry, sub-second behavior windows |
| Per-tag/per-head SaaS billing + tag inventory tracking | 701x (tag-locked), Allflex, Datamars Livestock, Tru-Test | Medium — every commercial offering bundles tags | herd-scout's wedge is OSS-reader-bridge; tag inventory tracking is a feature gap, not a wedge |
| Pay-per-report intelligence / report-as-a-product | **Optifarm** (25 countries, 200 languages, pay-per-use) | Low-Medium — interesting business model reference | Could inspire herd-scout's commercial sustainability story |
| **Sentera/Pix4Dfields-grade prescription map round-trip** with FMS ingest | Sentera FieldAgent (crops only), John Deere Ops Center | Low — crops, not livestock | Wiki [[oss-drone-fms-pipeline]] §L5 calls this out generally |
| **Wildlife-grade detector/tracker stack** (MegaDetectorV6 + PyTorch-Wildlife + WildFusion + WildlifeReID-10K + AnimalCLEF2026) | Microsoft AI for Good, BVRA, WildlifeDatasets | High — directly transferable; HerdNet is a subset | Wiki has [[cattle-reid-self-supervised]] but doesn't mention MegaDetectorV6, WildFusion, or the AnimalCLEF2026 challenge |
| **Edge appliance** w/ solar + Jetson Orin Nano + queue-and-sync | **SPARROW** (Microsoft, OSS, MIT) | High — close pattern match for herd-scout's Pascal laptop / future Orin daemon | Wiki [[output/plan-deploy-daemon-on-1060-laptop-2026-05-22]] has Pascal box; SPARROW pattern (queue → sync when reachable) maps directly with iroh as the transport |
| **NIP-15 marketplace + NIP-44 DMs** for breeder/sale provenance | Nostr ecosystem | Medium — bridges to BeefChain-class use case without a chain | Wiki has nothing on Nostr; herd-scout could host a livestock-stalls (event 30017) and provenance feed |
| **MoQ live video relay** as a maturing standard (moq-lite, moq-relay) | n0-computer, IETF MoQ working group | High — already in repo, but no wiki article on MoQ vs WebRTC vs RTSP for farm video | Repo has it; wiki should document it (research gap, see above) |
| **YOLO26 NMS-free + Qualcomm QNN export** for Snapdragon-X edge devices | Ultralytics, Qualcomm | High — opens cheap OSS edge boxes (Snapdragon-X mini PCs ~$300-500) as deployment targets | Wiki [[yolo26-and-tracker-compat]] has YOLO26 retrain plan but doesn't cover QNN/Snapdragon-X export |
| **DINOv3 zero-shot animal Re-ID** (no per-farm fine-tune) | Meta / FAIR | High — replaces "ResNet50 stub" entirely | Wiki [[cattle-reid-self-supervised]] is DINOv2-era; needs DINOv3 update + license check |
| **On-device LLM (Gemma 4 E2B / Qwen3.6-A3B) for "ask the herd"** | Google DeepMind, Alibaba | Medium — feature-class differentiator | Wiki has nothing on on-device LLMs; would be a new arc |
| **AgIsoStack-rs** (ISO11783/ISOBUS in Rust) for tractor/implement CAN | Open-Agriculture | Medium — adjacent OSS Rust ag dependency | Wiki has nothing on ISOBUS; not a herd-scout wedge but a cheap dependency for any "tractor + cattle" overlap |
| **OADA `formats` + JSON-Schema** as a living interop layer | OADA | Medium — closest to a ratified, alive ag JSON interop standard | Wiki [[ag-data-standards]] mentions adoption pragmatics; doesn't single out OADA |
| **AgGateway TraceabilityAPI / Modus** as a 2025-2026-active spec | AgGateway | Medium — alive, near-canonical ag spec | Wiki notes ADAPT (stalled at v3.1.0, .NET-only, no livestock); doesn't mention TraceabilityAPI/Modus |
| **iNaturalist / eBird two-tier verification UX** | iNaturalist (~300M obs), Cornell eBird (~104M checklists) | High — directly addresses herd-scout's count-validation UX | Wiki [[herd-counting-pipeline]] L5 has active-learning loop; doesn't reference iNaturalist's "Research Grade" or eBird's regional-reviewer pattern |
| **CSRD Scope 3 reporting pull-through** (EU emissions traceability) | EU directive 2022/2464 | High — mid/large dairies/feedlots have a regulatory deadline | Wiki has nothing on CSRD as a commercial driver; this is a B2B revenue angle |
| **USDA APHIS 840 EID rule (effective Nov 2024)** | USDA APHIS | High — already in force; demand signal for any FMS that captures EID at chute | Wiki [[livestock-eid-rfid]] mentions USDA 840 in passing, but not the live federal rule that creates compliance demand right now |

## Competitive Landscape

| Competitor / Tool | Overlap with repo | Unique features | Weaknesses |
|---|---|---|---|
| **farmOS** (GPL-2.0, ~1.3k stars, last commit 2026-05-30) | Asset/Log/Quantity/Term/Plan model, JSON:API; Field Kit PWA offline-first | Mature plugin ecosystem, multi-language, GIS layers | PHP/Drupal stack; no CV; no P2P; PWA mobile not native; livestock-shallow |
| **LiteFarm** (GPL-3.0, ~220 stars, active) | Sustainability/cert focus; Node + React + Postgres | Non-profit governance; LatAm/NA presence; clean modern stack | Crop-first; livestock + offline + P2P "on roadmap, not shipped"; no CV |
| **Ekylibre** (AGPL, ~479 stars) | Heavyweight ERP+accounting (EU/FR), Rails | Best financial integration; vineyard-deep | **SaaS sunsetting in 2026**; web-only; no CV/drone; centralized |
| **Tania-core** (Apache-2.0, ~813 stars, last commit 2026-03-03) | Smallholder Go FMS | None notable | Hobbyist-tier; no livestock-specific; effectively dormant since rewrite stall |
| **HerdNet** (MIT code / CC-BY-NC-SA weights, ~57 stars) | Aerial mammal counting (point-detection + stitcher); 73-83% F1 | The only published OSS aerial counter at quality | **Weights non-commercial — poison pill**; no release since v0.2.1 (Mar 2024) |
| **MegaDetectorV6 + PyTorch-Wildlife + SPARROW** (MIT) | Edge animal detection; queue-and-sync architecture | MIT throughout; Microsoft AI for Good support; active 2025-2026 | Wildlife (not livestock) primary; cattle requires fine-tune; edge image not P2P |
| **AgIsoStack-rs / livestock-rs / nsip / ersha-os** (MIT, Rust) | Adjacent OSS Rust ag — ISOBUS, breed metadata, sheep EBVs, DPI | Rust ecosystem reuse | Narrow scope each; herd-scout could *consume* not compete |
| **AgriWebb** (closed, SaaS) | EID hardware integration, GPS tasks; 17k producers AU/NZ/UK/US/ZA | Largest livestock-specific footprint; embedded sales | Cloud-only; no drone/CV; no offline; CV/drone integration would commoditize wedge |
| **Halter** (closed, hardware-locked) | Solar collars + virtual fencing + satellite SKU 2026 (NZ/US/AU) | Animal-level real-time location, low-infra deployment | Hardware-locked; closed mobile app; no CV ingest API today |
| **Vence** (Merck Animal Health, closed) | Virtual-fence collars, 5-10k acre base stations | Per-collar pricing model | Hardware-locked; integrated with Merck SenseHub stack |
| **701x** (closed, US-centric) | xT/xTpro/xTlite GPS ear tags + "#1 cow-calf app" | Tag-software vertical stack | Tag-locked; closed; SaaS subscription |
| **Performance Beef / PLA** (closed) | Records ergonomics, market feeds | Best feed-cost UX | No CV/drone; no offline-first |
| **Datamars Livestock / Tru-Test** (closed) | Stick-reader hardware (XRS2 etc.) + DataLink SaaS | EID reader market | Closed protocols; vendor lock-in — exactly herd-scout's target |
| **Allflex SenseHub** (MSD/Merck, closed) | Dairy collars + analytics | Dairy depth | Dairy-skewed; closed |
| **CattleEye** (closed, AWS-backed) | Parlour-cam mobility + BCS scoring | Best dairy-parlour CV | Parlour only; no extensive grazing |
| **Optifarm** (closed, pay-per-report) | Pay-per-use intelligence in 25 countries | Genuinely novel pricing model | Poultry/pig/dairy heavy; closed |

## Emerging Trends

What's coming that both the repo and wiki should prepare for (cited inline in the per-section sources):

1. **YOLO26 NMS-free + QNN export (shipping today)** — migrate `cv-sidecar` Phase 3: YOLO11s → YOLO26-s by 2026-Q4; the NMS-free head removes a real ByteTrack edge case (overlapping cattle at gate counts) and unlocks `coreml`/`qnn` exports for cheap Snapdragon-X mini-PC daemons (~$300–500).
2. **DINOv3 zero-shot Re-ID + WildFusion calibrated score-fusion (shipping today)** — replace the wiki's ResNet50 stub with DINOv3 + MegaDescriptor cosine; per-farm fine-tune becomes optional, not required. **License-check DINOv3** (custom license, not Apache/MIT) before redistribution.
3. **Iroh 1.0 (rc.1 published 2026-05-27, GA imminent)** — pin to 1.0 within 90 days of GA; track post-1.0 binding pattern (note `iroh-ffi` archived); evaluate **Loro 1.x** (stable) and **Automerge 3** (10× memory cut) for richer collaborative docs (treatment plans, grazing rotations) where iroh-smol-kv KV CRDT is too thin.
4. **Gemma 4 E2B / Qwen3.6-A3B (Apache-2.0) on-device** — "ask the herd" assistant becomes feasible on Snapdragon 8 Gen 4-class phones; ship the daemon hook in 2026-Q4 to be ready for 2027.
5. **MegaDetectorV6 + WildlifeReID-10K + AnimalCLEF2026 challenge dataset** — the wildlife-CV ecosystem just published cattle-specific benchmarks (CattleMuzzle, HolsteinCattleRecognition added 2025-08; MultiCamCows2024 added 2025-04). Treat them as the new evaluation harness; ditch HerdNet as primary baseline.
6. **USDA APHIS 840 EID rule** in force since 2024-11-05 (cattle/bison ages 18 mo+ sexually intact + rodeo/show animals crossing state lines) — paying-customer demand for any FMS that captures EID at chute. Pair with **CSRD Scope 3** rolling in for EU exports.
7. **Android 15 / 16 FGS rules** — `dataSync`/`mediaProcessing` capped at **6 hours per 24-hour rolling window**; `BOOT_COMPLETED` cannot launch dataSync/camera/mediaPlayback/microphone. Treat the 6-hour cap as a hard sync budget; chunk drone-video uploads via WorkManager.
8. **Sigstore Cosign v3.0.6 + SLSA L2** — sign images, blobs, and SBOMs (in-toto/DSSE attestations) on every release. Rekor mirror in Wave 14 already pays the audit-trust cost; add `slsa-github-generator` to ride the signed-provenance momentum.
9. **AgGateway TraceabilityAPI + Modus + OADA `formats`** are the *living* ag-data interop specs in 2026; **ADAPT** is stalled (.NET-only, no livestock, last release 2024-03-22) — don't depend on it.
10. **Halter satellite SKU + AgriWebb potential CV/drone partnership** — the two competitive moves most likely to erode herd-scout's wedges. Watch quarterly.

## Recommended Actions

### Immediate research (`/wiki:research` commands)

```
/wiki:research "iroh-moq vs iroh-live MoQ vs alternative low-latency video over QUIC for farm/drone use"
/wiki:research "rust ort onnxruntime pascal sm_61 deadlock and python sidecar IPC patterns"
/wiki:research "iroh access control patterns: AccessLimit predicate, ProtocolHandler gate, Endpoint allowlist"
/wiki:research "sigstore rekor for transparency logs of farm/IoT audit data"
/wiki:research "cross-language local identity file design with version migration and integrity check"
/wiki:research "iroh QUIC bi-stream as OpenSSH ProxyCommand vs tailscale ssh vs cloudflared access ssh"
/wiki:research "shared tokio runtime in JNI cdylib for android: lifecycle, panic handling, leaked threads"
/wiki:research "offline video file herd counting: closure-assumption-valid Lincoln-Petersen variants"
/wiki:research "systemd unit dependency between rust daemon and python ML sidecar: restart policy, version pinning, OOM"
/wiki:research "pascal sm_61 lifecycle for ml inference vs jetson orin nano vs cpu-only onnxruntime"
/wiki:research "DINOv3 zero-shot animal re-id license terms and on-device deployment"
/wiki:research "MegaDetectorV6 + PyTorch-Wildlife + SPARROW edge architecture for livestock"
/wiki:research "Nostr NIP-15 marketplace + NIP-44 DMs for livestock breeder/sale provenance"
/wiki:research "AgGateway TraceabilityAPI + Modus + OADA formats as ag interop layer for OSS FMS"
/wiki:research "USDA APHIS 840 EID rule 2024 — compliance UX for an FMS at the chute"
/wiki:research "CSRD Scope 3 emissions reporting demand on livestock FMS"
/wiki:research "iNaturalist / eBird two-tier community verification UX patterns transferred to herd counting"
/wiki:research "Halter satellite SKU 2026 + AgriWebb integrations as competitive watch"
```

### Build (feature candidates ranked by impact × feasibility)

**P0 — without these, the existing CV/audit/identity infrastructure has no records to write:**

1. **iroh-smol-kv FMS schema crate** — implement [[iroh-docs-fms-schema]]: namespace per farm, per-device authors, HLC timestamps, append-only logs, SQLite projection. ~2–4 weeks.
2. **Animal / Group / Land / Equipment record types + minimal CRUD UX** in egui (or first iteration of Tauri 2 frontend if [[mobile-desktop-architecture]] §Tauri-2 is still the long-term plan). ~3–6 weeks.
3. **`herd-scout-eid` Rust crate (weekend MVP)** per [[livestock-eid-rfid]]: `serialport` + `bluer` + `btleplug` line-buffered ISO 11784/11785 ASCII parser; covers Allflex/Agrident/Tru-Test SPP and Nordic-UART BLE readers (~70-80% of deployed sticks). One weekend.

**P1 — turns existing CV detections into the auditable count-with-confidence the wiki specifies:**

4. **Counting layers 3+5** — deployment-aware policy (`MaxDistinctIDs` / `MedianActiveWindow` / `LineZone`) + split conformal per-site calibration + 🟢/🟡/🔴 confidence chip. Conformal is ~30 lines; policy switch is medium. Once EID is wired, layer 5's **EID reconciliation residual** becomes the primary wedge.
5. **farmOS JSON:API consumer/producer** — keeps door open to farmOS users ([[oss-fms-landscape]]). Medium.
6. **Treatment log + withdrawal flag** — EU/AU regulatory minimum ([[fms-feature-taxonomy]] §2). Low.
7. **Documented accuracy commitments** ([[livestock-cv-accuracy]]: ±5–10% pasture, ±15–25% bad-case) on the public README.
8. **Paddock polygon CRUD + animal-last-seen pin** ([[fms-feature-taxonomy]] §6) — needs a map widget; egui-map / slint / Tauri 2 leaflet.

**P2 — drone integration, cross-platform, regulatory exports:**

9. **YOLO26 + OccluBoost / BoostTrack + RF-DETR-M server preset** in `cv-sidecar` Phase 3.
10. **DINOv3 + WildFusion cattle Re-ID** — replace ResNet50 stub.
11. **Counting layer 4 — Lincoln-Petersen / Chapman + Royle 2004 N-mixture** for multi-pass drone counts.
12. **WebODM / DroneDB ingest module** — owns the broken-link L5 gap ([[oss-drone-fms-pipeline]] §L5).
13. **MAVLink (`mavlink` crate, not DroneKit) + 4G bridge** — phone-on-drone airframe ([[android-on-drone]], [[phone-on-drone-airframe]]).
14. **Tauri 2 frontend reuse for iOS** — [[mobile-desktop-architecture]] §Tauri-2; reverses egui choice. High cost, but unlocks iOS.
15. **Compliance exports** (USDA 840 reporting, NLIS, CCIA, EU passport).
16. **ICAR ADE / OADA `formats` / AgGateway TraceabilityAPI adapters** as the living interop set.
17. **On-device LLM hook** — Gemma 4 E2B / Qwen3.6-A3B for "ask the herd" UX; daemon RPC stub now, full feature 2027.

### Monitor

- **Halter satellite SKU + integrations page** — quarterly.
- **AgriWebb integrations / partner directory** — quarterly. A Roboflow-backed counting partnership commoditises herd-scout's drone wedge fastest.
- **DroneDeploy livestock/agriculture page** (currently 404) — re-check Q3 2026.
- **iroh GA + breaking changes** between 1.0-rc.1 and 1.0 stable.
- **HerdNet weight-license change** (currently CC-BY-NC-SA — poison pill); replace with MIT/Apache backbone (RF-DETR + DINOv2 / DINOv3) regardless.
- **DINOv3 license terms** — custom license, not Apache/MIT; check before redistribution.
- **FAA Part 108 BVLOS final rule** — promised but not surfaced.
- **CSRD Scope 3 enforcement timeline** for mid/large dairies into EU supply chains.
- **`iroh-blobs#233`** (poisoned-store) already in `inventory/watch/` — keep tracking.

## Anti-patterns (failures herd-scout must avoid)

From the failures market scan — concrete "do not do X because Y died from it":

1. **Don't ship a "Wikipedia for X" donation-funded model.** OpenFarm ran 10 years on OpenCollective and still couldn't pay for security updates; archived 2025-04-22. Plan revenue from day one.
2. **Don't bet on a single corporate-sponsored SDK.** DroneKit-Android died the moment 3DR exited consumer hardware (2016). Use **MAVSDK** or speak MAVLink directly via the `mavlink` Rust crate; treat any vendor SDK as disposable.
3. **Don't ingest non-commercial-licensed weights.** HerdNet's CC-BY-NC-SA pretrained models would taint herd-scout. Train your own, or use weights with permissive licenses (Apache/MIT).
4. **Don't bet sync on unproven P2P stacks.** Zero documented production survivors for GUN/OrbitDB in ag. iroh + iroh-smol-kv is fine (paying production deployments at Paycode), but keep an offline-only fallback that doesn't require sync to work.
5. **Don't rewrite from scratch.** Tania PHP → Tania-core Go: the rewrite stalled and both are now dead. Refactor in place. (egui → Tauri 2 must be a *port*, not a rewrite-and-throw-out.)
6. **Don't depend on incumbent free tiers.** PastureMap (acquired 2020), Cainthus → Ever.Ag (2022), Vence → Merck (2022) — every notable livestock SaaS got acquired and reshaped within ~5 years.
7. **Don't promise blockchain traceability.** BeefChain reached USDA certification and still couldn't find a revenue model; TradeLens shut down for the same reason. Audit logs via append-only signed Rekor mirror (already shipped Wave 14) is the right answer; skip the chain.
8. **Tauri 2 mobile churn is the single biggest *technical* risk** for any future mobile pivot — capabilities/permissions model rewrite causes recurring "invoke command not allowed" errors. If the egui → Tauri 2 port happens, budget for it as Wave-class work, not a sprint.

## Confidence Notes

- **High confidence (repo + wiki claims directly verifiable from artefacts):** Repo features list, ALPN inventory, identity envelope schema, audit-log layout, CV pipeline implementation status, wiki gap matrix, alignment table.
- **Medium confidence (cited 2026 web sources):** YOLO26 release date and headline features, RF-DETR 1.7.1 numbers, DINOv3 license caveat, Iroh 1.0-rc.1 (May 27 2026), Loro 1.x stable, Automerge 3 (July 2025), Gemma 4 E2B (April 2026), Qwen3.6-A3B (April 2026), Android 15 6-hour FGS cap, USDA APHIS 840 effective date, MegaDetectorV6 / SPARROW status, BoxMOT v20.0.0 OccluBoost numbers, Halter satellite SKU 2026, Ekylibre SaaS sunset 2026, Tauri 2 capabilities-model breakage, OpenFarm archival 2025-04-22.
- **Low confidence / sparse 2026 evidence (re-check before betting on):** FAA Part 108 BVLOS final-rule status, NLIS (AU) 2026 API changes, Cainthus rebrand current state under Cargill/Ever.Ag, AgriWebb pricing, PastureMap/Soilworks current product cadence, exact text of HerdNet weight license terms, EASA Reg (EU) 2025/870 livestock-survey applicability, vendor EID-reader SDKs (most pages redirect-blocked).
- **Speculative (clearly labeled):** Top-3 threats list (informed inference, not vendor announcements), exact priority ordering of build candidates (depends on operator preferences not surveyed here), claim that Halter or AgriWebb will move into CV/drone counting (treat as risk, not forecast).

No archived-wiki sources used; `--include-archived` was not passed.
