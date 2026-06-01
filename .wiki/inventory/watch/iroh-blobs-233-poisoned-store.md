---
title: "iroh-blobs #233 — poisoned-store unresolved"
type: watch
priority: p1
created: 2026-06-01
status: open
upstream: https://github.com/n0-computer/iroh-blobs/issues/233
affects:
  - herd-scout-daemon (Wave 13 batch upload over `herd-scout/upload/1` ALPN)
---

# Watch: iroh-blobs #233 (poisoned-store)

## Why this is on the watch list

Wave 13 (desktop video upload) ships bytes over **iroh-blobs**. The 2026-06-01 hub-wiki research on Iroh application patterns surfaced that **iroh-blobs 0.102.0 is pinned to iroh 1.0-rc.1 but issue #233 (poisoned-store) is unresolved**. The blob store can enter an invalid state in some failure modes; restart recovery is not guaranteed.

## Current herd-scout exposure

- Daemon imports clips via `iroh-blobs` and persists blob data under `<data_dir>/herd-scout/uploads/<blake3>/`.
- A poisoned store could surface as: failed reads on previously-imported clips; failed `herdctl push` after a network blip; possible silent data corruption depending on the failure path.
- Mitigation today: per-clip BLAKE3 hash is recorded and the daemon could re-verify on read. Not currently wired — see Action below.

## Watch triggers

Re-evaluate this watch item when any of:

- iroh-blobs #233 closes (or a fix lands in a release)
- We plan to upgrade iroh-blobs past **0.102.0**
- We see an iroh-blobs error in production logs that smells like store invariant violation

## Actions while open

1. **Do not** bump iroh-blobs to 0.102.0 or beyond without first re-checking #233 status.
2. Consider adding a **fsck-on-startup** path: walk `<data_dir>/herd-scout/uploads/`, verify each blob's BLAKE3 against its directory name, log + quarantine mismatches. ~50 LOC. Not yet implemented.
3. Consider per-blob **read-time verification** (re-hash on read) as a defense in depth. Cost: ~30 ms per MB on Pascal. Worth it for the upload-batch path; too expensive for any live-frame path.

## Source

- Hub research round 2026-06-01: `~/wiki/topics/gtx-1060-headless-ai-server/` — "Iroh application patterns 2026" deep research, 36 sources ingested. Specific concept: `iroh-blobs-resumable-uploads`.
