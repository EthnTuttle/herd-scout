---
title: "DroneKit-Android status — abandoned, but the canonical Android MAVLink lib"
source_url: https://github.com/dronekit/dronekit-android
type: project
tags: [dronekit, android, mavlink, abandoned, drone-companion]
created: 2026-05-20
confidence: medium
---

# DroneKit-Android (3DR)

- Last release: October 2016
- Develop branch: 5,810 commits, 37 open issues, no recent maintenance
- Status: **effectively dead**

## Why it matters anyway

It's the canonical Android library for talking MAVLink to ArduPilot/PX4. Anyone wanting an Android phone on a drone has historically built on this. Tower / DroidPlanner sit on top of it.

## Live alternatives for new builds

- **`io.dronefleet.mavlink`** — Java library, more actively maintained
- **MAVSDK Android port** — official MAVSDK has Android support; less polished than core
- **Roll-your-own** — MAVLink wire protocol is well-documented; for a focused use case (waypoint upload, telemetry stream) it's a few hundred lines
- **QGroundControl Android** — only works as a GCS app, not as a library

## Practical recommendation

If herd-scout puts a phone on the drone for ML + 4G bridge, **fork DroneKit-Android or use `io.dronefleet.mavlink`** — don't bet on DroneKit upstream returning to life.
