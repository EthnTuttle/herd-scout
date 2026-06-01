---
title: "ArduPilot — Common Vibration Damping"
source: https://ardupilot.org/copter/docs/common-vibration-damping.html
type: article
tags: [drone, vibration, damping, sorbothane, mounting, hardware]
ingested: 2026-06-01
quality: 5
confidence: high
---

# ArduPilot vibration damping — canonical playbook

Decade of community refinement; drone-agnostic. Authoritative source for damping a phone payload.

## Materials ranked

- **Kyosho Zeal Gel Tape** (best in tests)
- US Silicone V10Z62MGT5
- 3M foam
- Du-Bro 1/4" RC foam
- Moon Gel (**fails >100 °F** — relevant for sun-baked cattle work, fails for herd-scout daytime ops)
- 30-durometer Sorbothane

## Compression targets

- **15–20% for Sorbothane**
- **~20% for sandwich mounts**
- Over-compression kills isolation.

## Mounting pattern

- **Four corners**, 1–2 cm pads.

## O-ring suspension

- 1/2"–3/4" OD
- **Silicone O-rings outperform Buna-N**

## Critical mass-matching insight (load-bearing for herd-scout)

Most off-the-shelf dampers are tuned for masses 5–10× heavier than autopilots. **A 150–200 g phone is closer to the design mass these dampers expect — actually a good fit**, unlike the typical FC.

## Frequency targets

Targets **high-frequency, low-amplitude** prop vibration. Favors rigid frames since flex adds mechanical delay.

## Implications for herd-scout

- Drop Moon Gel from any phone-tray candidate (heat fails).
- Sorbothane 30-durometer at 15–20% compression in a four-corner sandwich is the canonical baseline.
- Phone is mass-favorable for off-the-shelf dampers — don't over-engineer.
