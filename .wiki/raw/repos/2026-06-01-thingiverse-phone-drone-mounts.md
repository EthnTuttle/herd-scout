---
title: "Phone-drone mount catalog — Thingiverse / Printables / Cults3D"
sources:
  - https://www.thingiverse.com/thing:2194278
  - https://www.printables.com/tag/dronemount
  - https://www.yeggi.com/q/phone+mount+for+drone/
type: repo
tags: [drone, phone, mount, 3d-print, tpu, hardware]
ingested: 2026-06-01
quality: 3
confidence: medium
---

# Phone-on-drone mount catalog

Community 3D-printable starting designs.

## Notable mounts

- **HeyVye Modular Mounting System** (thing:2194278) — targets >5.5" devices; phone clamp options.
- **Arashk Universal Handlebar V2** — 5.72–10.8 cm clamp range covers all modern phones.
- **BigOldLiar Drone Phone Mount** (thing:5320779) — CC-licensed; directly remixable for herd-scout.

## Aggregator collections

- Printables `dronemount` tag
- STLFinder `drone-phone-mount`
- Yeggi `phone+mount+for+drone`

## Important negative finding

**No academic UAV paper surfaced specifically on smartphone payload mounting.** Community designs exist; rigorous studies don't. This is a **herd-scout-distinctive synthesis opportunity** — publish-worthy if validated.

## Gimbal vs rigid

**Search produced no good DIY data on bolting a phone gimbal (Hohem/Feiyu) to a drone.** Strong negative signal: the community doesn't do this in practice. Reasons inferred:
- Adds 200–400 g
- Adds power burden
- Smartphone gimbals don't survive prop vibration well
- Android EIS + capped shutter handles nadir cattle counting at 30–60 m AGL adequately

## Implications for herd-scout

- **Don't design a tray from scratch.** Remix BigOldLiar (CC) or HeyVye Modular as starting geometry.
- **Skip drone-mounted phone gimbals** — community evidence is the absence of community evidence.
- **Rigid soft-mount + Android EIS + capped shutter ≈ 1/(2×fps)** is the consensus pattern. Combined with the 95A TPU + 50A grommet BOM from [[fpv-camera-mounting-tpu]] and the suspended topology from [[wildair-drone-vibration-frequencies]].
- Operator playbook: **printed cage around the phone** (not just a clamp) — protects rear glass on bad landings.
