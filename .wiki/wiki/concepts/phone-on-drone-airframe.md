---
title: "Phone-on-drone airframe — concrete BOM, vibration, mounting, lifetime"
summary: "Buildable airframe spec for a phone-as-camera drone payload: 95A TPU tray + 50A grommets + suspended topology + 6-12mo damper replacement"
tags: [drone, phone, airframe, vibration, mount, tpu, sorbothane, ardupilot]
created: 2026-06-01
confidence: high
type: concept
---

# Phone-on-drone airframe — buildable spec

The wiki's [[android-on-drone]] gave the high-level verdict (phone-as-companion is good; phone-as-FC is bad). This article fills in the concrete airframe details that were missing — what to print, what to buy, how to mount, how often to maintain.

## The frequency target

Mini-quad motor vibration band: **100–300 Hz**, peaking at **70–80% throttle**. **Z-axis (vertical) dominates** — phone OIS/IMU is most sensitive to pitch/roll-coupled Z noise.

**Cheapest pre-mount win**: prop balancing alone changes vibration **>300%**. Always balance props before tuning the damper.

## The four mounting topologies

| Topology | Use |
|---|---|
| Sandwich (between two plates) | Compact, OK |
| Corner (4× foam blocks) | Standard FC mount |
| **Suspended (elastic hanging)** | **Best for cameras — what herd-scout should use for the phone tray** |
| Constrained-layer (viscoelastic) | Heavy applications |

## Concrete BOM

| Part | Spec |
|---|---|
| **Tray** | 95A TPU printed (or 98A if heavier phone) |
| **Isolators** | 4× **M3×8 mm silicone grommets, 50A durometer**, 20–30% compression |
| **Suspension lines** | Optional 1/2"–3/4" OD silicone O-rings (outperform Buna-N) |
| **Backup damper** | 30-durometer Sorbothane sandwich, 15–20% compression |
| **Phone case** | Printed cage (NOT just a clamp) — protects rear glass on bad landings |
| **Frame size** | **5" quad minimum**, 7–10" comfortable for 150–200 g phone |

## What to avoid

- **Moon Gel** (fails >100 °F — won't survive sun-baked cattle work).
- **Off-the-shelf hard plastic phone clamps** as the *only* mount — too rigid, no isolation.
- **Drone-mounted smartphone gimbals** (Hohem, Feiyu): community evidence is the **absence** of community evidence — the consensus is rigid soft-mount + Android EIS works for nadir cattle counting at 30–60 m AGL. Saves 200–400 g and the gimbal power burden.
- **Over-tight straps** — re-couples vibration to the frame, defeats the dampers.

## Mass-matching insight

A 150–200 g phone is **closer to the design mass that off-the-shelf dampers expect** than the typical 30–50 g flight controller. Don't over-engineer this — standard FPV camera-class hardware works.

## CameraX configuration

To minimize jello (rolling-shutter interaction with prop vibration):

- **Cap shutter time to ~1/(2×fps)** — at 30 FPS, ~1/60 s. Manual exposure mode in CameraX.
- **Enable EIS** (electronic image stabilization) — the suspended TPU tray is for shake; EIS handles residual at the pixel level.
- **Skip OIS** if the phone has it — at drone-vibration frequencies OIS often fights the gimbal-effect rather than helping. Disable in CameraX where possible.

## Maintenance schedule

- **Silicone gel pads / grommets: replace every 6–12 months.** Operational maintenance line item.
- **TPU printed tray: replace after 50–100 hard landings.** Print spares.
- **Phone rear glass cracks**: the printed cage is mandatory for field deployments. Plan for ~1 phone per 6 months as collateral damage budget.

## Frame / mount catalog (community remixable starting points)

- **BigOldLiar Drone Phone Mount** (Thingiverse 5320779) — CC-licensed, directly remixable.
- **HeyVye Modular Mounting System** (thing:2194278) — >5.5" device target.
- **Arashk Universal Handlebar V2** — 5.72–10.8 cm clamp range covers all modern phones.
- Aggregators: Printables `dronemount` tag, STLFinder, Yeggi.

**No academic UAV paper exists specifically on smartphone payload mounting** — herd-scout could publish a validated design as the first.

## See also

- [[android-on-drone]] — the high-level verdict this builds on
- [[drone-hardware]] — autopilot + frame choices
- [[phone-power-on-drone]] — power side of the same payload
- [[phone-thermal-management]] — thermal side
- [[phone-publisher-android-fgs]] — software side

## Sources

- raw: [[2026-06-01-ardupilot-vibration-damping]]
- raw: [[2026-06-01-fpv-camera-mounting-tpu]]
- raw: [[2026-06-01-wildair-drone-vibration-frequencies]]
- raw: [[2026-06-01-thingiverse-phone-drone-mounts]]
