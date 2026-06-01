---
title: "Drone Vibration Damping — Z-axis and 100-300 Hz prop band"
source: https://blog.wildair.ch/drone-vibration-damping/
type: article
tags: [drone, vibration, frequency, mounting, suspended, durometer]
ingested: 2026-06-01
quality: 4
confidence: high
---

# Drone vibration damping — frequency-aware analysis

Independent analysis with the Hz numbers and topology vocabulary missing from ArduPilot/FPV docs.

## Frequency band

- **Mini-quad motor frequency band: 100–300 Hz**
- Peaks at **70–80% throttle** — the band a phone tray must dampen.
- **Z-axis (vertical) vibration dominates** in quads.
- Phone OIS / IMU is most sensitive to pitch/roll-coupled Z noise — Z dampening is what matters.

## Cheapest pre-mount win

**Prop balancing alone changes vibration by >300%.**

## Four mounting topologies

1. **Sandwich** — between two plates with damper material
2. **Corner** — four-corner foam blocks
3. **Suspended** — elastic hanging
4. **Constrained-layer** — viscoelastic between two stiff layers

**Suspended is called out as best for cameras** — relevant for nadir phone trays.

## Material lifetime

**Silicone gel pads need replacement every 6–12 months** — operational maintenance line item for a herd-scout fleet.

## Durometer guidance

- **30–40A** for FC class
- **50–60A** for GPS

A phone (heavier than either) sits in the **50–60A** range.

## Implications for herd-scout

- Target the 100–300 Hz Z-axis band specifically.
- Balance props before adding more damping material — the cheapest single win.
- **Suspended (elastic hanging) tray** is the right topology for the phone, not a corner-mount.
- **6–12 mo replacement schedule** for damping material — operator playbook line item.
- Confirms 50–60A durometer lower bound; combines with FPV-camera 95A TPU (the *frame*) and 50A grommets (the *isolators*) for a coherent BOM.
