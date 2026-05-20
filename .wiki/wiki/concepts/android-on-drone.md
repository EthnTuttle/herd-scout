---
title: "Android phone on a drone — verdict and architecture"
tags: [android, drone, phone, companion-computer, mavlink, flight-controller, ardupilot, bvlos]
created: 2026-05-20
updated: 2026-05-20
confidence: high
type: concept
---

# Android phone on a drone

The user asked: "can we strap an android onto a drone?" Yes, with one critical reframing.

## TL;DR verdict

| Role | Verdict |
|---|---|
| Phone as **flight controller** (PID stabilization) | **NO.** Android scheduling jitter (tens of ms) too high for hard real-time. `pkourany/android-flight-controller` is dead. |
| Phone as **standalone mission computer driving a "dumb" drone** | Mediocre. Builds on dead libraries (DroneKit-Android, last release Oct 2016). |
| Phone as **(a) onboard ML/CV camera + (b) 4G BVLOS bridge**, talking MAVLink to a real Pixhawk/PX4 FC | **GOOD.** This is FlytBase architecture, DIY at ~$200 instead of $$$$. |

## Why phone-as-FC fails

- Real flight controllers run PID at 400 Hz–1 kHz over IMU data
- Android scheduling jitter ~tens of ms; sensor fusion latency too high
- You'd lose the airframe
- No serious project ships phone-as-FC

## Why phone-as-companion works

A companion computer's job is non-real-time intelligence: vision, mission planning, comm bridging. Android phones are competitive with the SBCs typically used for this:

| Companion | NPU | Power | Notes |
|---|---|---|---|
| Raspberry Pi 5 | modest | 5W | Best documented MAVSDK/ROS2 path |
| Jetson Nano | 0.5 TFLOPS | 5-10W | Entry edge AI |
| Jetson Xavier NX | 6 TFLOPS | 10-15W | Real-time YOLOv5 30+ FPS |
| **Used flagship Android** | **NPU comparable to Xavier NX for MobileNet** | 2-5W | Includes camera + GPS + LTE + battery + display |

Benchmarks:
- Pixel 4 CPU-only TFLite, MobileNet v1 224 quantized = **5 ms** (~200 FPS theoretical)
- MobileNet SSD / EfficientDet-Lite0 on Snapdragon 8xx with NNAPI/GPU: **15-40 ms (25-60 FPS)** — comparable to or faster than Jetson Nano
- NCNN/MNN: YOLOv5n/YOLOv8n at 15-25 FPS on flagship Snapdragon

## Connection to autopilot

| Connection | Bandwidth | Use |
|---|---|---|
| USB-OTG → Pixhawk USB | High | MAVLink + telemetry; ties up phone USB |
| BLE / SPP → HC-05/HM-10 → autopilot UART | ~115 kbps | Low-rate MAVLink (1-2 Hz) |
| **WiFi → ESP8266/ESP32 MAVLink bridge → autopilot UART** | High | **Most popular DIY**; UDP MAVLink |
| Built-in 4G LTE | High | Backhaul for BVLOS supervisory control |

## Risks

- **Weight**: 150-200g. OK on 5"+ quad; penalty on sub-250g class
- **Vibration**: Phone IMU/OIS hates prop vibration → soft-mount foam/gel
- **Thermal**: Mostly fine in flight (slipstream cooling); ground idle is risky
- **GPS**: Phone GNSS is consumer-grade; backup, not primary nav
- **FAA**: BVLOS via phone needs Part 107 BVLOS waiver (US)
- **Lost-link**: phone dying must NOT take down the aircraft — real FC owns flight; phone advisory only

## Critical software finding

**DroneKit-Android is dead** (last release Oct 2016, 5,810 commits on develop, 37 open issues, no recent maintenance). Tower / DroidPlanner sit on top of it.

For new builds, use:
- **`io.dronefleet.mavlink`** — Java library, more actively maintained
- **MAVSDK Android port** — official MAVSDK has Android support
- Roll-your-own — MAVLink wire protocol is documented; few hundred lines for a focused use case
- (Avoid) QGroundControl Android — works only as a GCS app, not a library

## Easier-path comparison

- **Pi 5 + Pi Cam + 4G hat (~$150)**: well-documented MAVSDK/ROS2, no battery management, slower NPU than flagship phone NPU
- **Used Pixel 6a/7a**: cheaper for compute, includes battery/cam/GPS/LTE/display, fragmented software stack

## Why this matters for herd-scout

- The "P2P video pipe" that already exists in this repo (iroh-live workspace) makes the phone-as-camera path even more attractive — phone streams to ground station/desktop via iroh, ML happens on either end (phone for low-latency on-device, desktop for richer batch passes)
- BVLOS via phone 4G enables ranches larger than visual-line-of-sight without buying enterprise drone hardware
- Re-uses [[implementation-plan]]'s "phone gimbal mount" plan but elevates phone from passive camera to active companion

## See also
- [[drone-hardware]]
- [[drone-vision-software]]
- [[implementation-plan]]
- [[oss-drone-fms-pipeline]]
- [[herd-scout-positioning]]

## Sources
- raw: [[2026-05-20-ardupilot-companion-computers]]
- raw: [[2026-05-20-dronekit-android-status]]
