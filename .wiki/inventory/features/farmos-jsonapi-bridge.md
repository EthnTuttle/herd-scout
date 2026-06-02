---
title: "Feature: farmOS JSON:API consumer/producer"
type: feature-candidate
priority: p1
created: 2026-06-02
source: assess-herd-scout-2026-06-02
status: open
wiki_evidence:
  - concepts/oss-fms-landscape
  - concepts/fms-data-model
  - concepts/herd-scout-positioning
---

# Feature: farmOS JSON:API bridge

## Why P1

farmOS is the gravity well of OSS FMS — biggest user base, mature data model, JSON:API, real ecosystem (Field Kit PWA, modules). Per [[../../wiki/concepts/herd-scout-positioning]] the positioning bet is to **stay data-compatible** rather than rebuild what farmOS does. A JSON:API consumer/producer keeps the door open to farmOS users (including the existing Field Kit PWA users) and gives herd-scout a credible "interop, not lock-in" story when pitched to farmers already running farmOS instances.

## Scope

Two halves:

1. **Producer** — herd-scout exposes its `Animal` / `Group` / `Land` / `Equipment` records and `Observation` / `Medical` / `Movement` / `Weight` / `Birth` logs as JSON:API resources matching farmOS's Asset+Log+Quantity+Term+Plan model.
2. **Consumer** — herd-scout can pull from a farmOS instance over JSON:API (read-only initial; bidirectional optional later).

Either half depends on the iroh-smol-kv FMS schema crate ([[iroh-smol-kv-fms-schema]]) being in place — without it there are no records to translate.

## Open questions

- Authentication: farmOS uses OAuth2 or basic auth; how does herd-scout's NodeId-based identity bridge to a farmOS user account?
- Conflict resolution if both sides edit the same animal record.
- JSON:API library in Rust — is there a maintained crate, or is hand-rolled the path?

## See also
- [[../../wiki/concepts/oss-fms-landscape]]
- [[../../wiki/concepts/fms-data-model]]
- [[../../wiki/concepts/herd-scout-positioning]]
- [[../../output/assess-herd-scout-2026-06-02]] §Opportunities
- [[iroh-smol-kv-fms-schema]] (depends on)
