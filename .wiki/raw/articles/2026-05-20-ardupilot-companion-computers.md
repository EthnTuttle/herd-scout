---
title: "ArduPilot companion computers — official guidance"
source_url: https://ardupilot.org/dev/docs/companion-computers.html
type: doc
tags: [ardupilot, companion-computer, mavlink, jetson, raspberry-pi, drone-architecture]
created: 2026-05-20
confidence: high
---

# ArduPilot companion computers

ArduPilot's official guidance pattern: real flight controller (Pixhawk / Cube / PX4) handles hard real-time stabilization at 400 Hz–1 kHz. A companion computer attached via UART/USB/WiFi handles non-real-time intelligence: vision, mission planning, comm bridging.

## Standard companion computers in OSS world

| Companion | AI perf | Power | Notes |
|---|---|---|---|
| Raspberry Pi 5 | modest | 5W | Best documented MAVSDK/ROS2 path; battery + camera + LTE all separate add-ons |
| Jetson Nano | 0.5 TFLOPS | 5-10W | Entry edge AI |
| Jetson Xavier NX | 6 TFLOPS | 10-15W | Real-time YOLOv5 30+ FPS |
| Jetson AGX Orin | 12-275 TFLOPS | 15-60W | Overkill for most farm CV |
| **Android phone** (used flagship) | NPU comparable to Xavier NX for many models | 2-5W | Not in official ArduPilot docs but viable; see notes |

## Phone-as-companion specifics (not in official docs)

- **Pixel 4 CPU-only TFLite**: MobileNet v1 224 quantized = 5 ms (~200 FPS theoretical)
- **MobileNet SSD / EfficientDet-Lite0** on Snapdragon 8xx with NNAPI/GPU: 15-40 ms (25-60 FPS) — comparable to or faster than Jetson Nano (~25 FPS for SSD-MobileNet)
- **NCNN / MNN**: YOLOv5n/YOLOv8n at 15-25 FPS on flagship Snapdragon
- Bottleneck is usually YUV→RGB conversion in Camera2/CameraX → ImageReader pipe, not inference

## Phone-on-drone connection options

| Connection | Bandwidth | Use |
|---|---|---|
| USB-OTG → Pixhawk USB | High | MAVLink + telemetry; ties up phone USB port |
| Bluetooth SPP/BLE → HC-05/HM-10 → autopilot UART | ~115 kbps | Low-rate MAVLink (1-2 Hz) |
| WiFi → ESP8266/ESP32 MAVLink bridge → autopilot UART | High | Most popular DIY; UDP MAVLink |
| Built-in 4G LTE | High | Backhaul for BVLOS supervisory control |

## Risks / constraints

- **Weight**: phone 150-200g; OK on 5"+ quad, penalty on sub-250g class
- **Vibration**: Phone IMU/OIS hates prop vibration — soft-mount foam/gel
- **Thermal**: Mostly fine in-flight (slipstream cooling); ground idle is the risk window
- **GPS**: phone GNSS is consumer-grade; use as backup, not primary nav
- **FAA**: BVLOS via phone needs Part 107 BVLOS waiver
- **Lost-link**: phone dying must NOT take down the aircraft. Real FC owns flight; phone is advisory only

## Verdict for herd-scout

- **Phone as FC**: NO. Android scheduling jitter is too high. `pkourany/android-flight-controller` is dead. Don't.
- **Phone as standalone mission computer driving a "dumb" drone**: mediocre — builds on dead libraries (DroneKit-Android last release Oct 2016).
- **Phone bolted to a Pixhawk/PX4 quad as (a) onboard ML/CV camera + (b) 4G BVLOS bridge, talking MAVLink to the real FC**: GOOD. This is the FlytBase architecture but DIY at $200 instead of $$$$.

## Easier-path comparison

- **Raspberry Pi 5 + Pi Camera + 4G hat (~$150)**: well-documented MAVSDK/ROS2, no battery management, slower NPU than flagship phone NPU
- **Used phone (Pixel 6a/7a)**: cheaper for compute, includes battery + camera + GPS + LTE + display, but software stack is fragmented (DroneKit-Android dead — fork it or use raw `io.dronefleet.mavlink`)
