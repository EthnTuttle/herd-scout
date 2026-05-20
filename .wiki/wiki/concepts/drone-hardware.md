# Open Source Drone Hardware for Computer Vision

- title: Open Source Compatible Drones for Computer Vision
- summary: Research into drone hardware options that work with open source autopilot software (ArduPilot) for computer vision applications
- tags: [drone, hardware, ardupilot, computer-vision, companion-computer, jetson]
- created: 2026-05-19
- confidence: high
- type: research

## Recommended Autopilot Hardware

| Autopilot | Price | Features |
|-----------|-------|----------|
| CubePilot Cube Orange+ | ~$300 | Redundant IMUs, GPS, AI-ready |
| Holybro Pixhawk 6X | ~$200 | Dual IMU, temperature compensated |
| Holybro Kakute H7 V2 | ~$120 | Compact, 9 UARTs |
| Holybro Kakute F7 AIO | ~$80 | All-in-one FC+ESC |
| SpeedyBee F405 AIO | ~$60 | Budget-friendly |

## Ready-to-Fly Drone Kits

1. **Holybro S500 V2 Kit** (~$350) - 500mm frame, payload ~500g, 20-25 min flight
2. **Holybro X500 Kit** (~$400) - 500mm symmetrical, payload ~600g
3. **NWBlue Hexsoon EDU450** (~$300) - 450mm foldable

## Companion Computers

| Model | AI Performance | Power | Price |
|-------|---------------|-------|-------|
| Jetson Nano | 0.5 TFLOPS | 5-10W | ~$200 |
| Jetson Xavier NX | 6 TFLOPS | 10-15W | ~$400 |
| Jetson AGX Orin | 12-275 TFLOPS | 15-60W | $1,000-$2,000 |

## Smartphone as Camera (Recommended)

Using a mounted smartphone avoids purchasing separate camera hardware.

### Open Source Android Apps

| App | Type | Features |
|-----|------|----------|
| **IP Webcam** | Open Source | RTSP/MJPEG streaming |
| **TinyCam Monitor** | Open Source | Multi-camera support |

### Open Source ML on Android

| Framework | Type | Performance |
|-----------|------|-------------|
| **TensorFlow Lite** | Open Source | Good on flagship phones |
| **MediaPipe** | Open Source | Excellent, optimized |
| **NCNN** | Open Source | Best for mobile |

## Recommended Configurations

### Ultra-Budget (~$300-400) - Phone Only
- Frame: DJI F450 (used) or similar
- Autopilot: Holybro Kakute F7 AIO
- Camera: Your existing Android phone
- Mount: Phone gimbal mount (~30)

### Budget (~$800-1000)
- Frame: Holybro S500 V2 Kit
- Autopilot: Holybro Kakute H7 V2
- Companion: Jetson Nano
- Camera: Reolink RLC-410 (5MP IP camera)

## Open Source Software Stack

- **ArduPilot** / **PX4** - Autopilot
- **QGroundControl** - Ground control
- **YOLOv5/YOLOv7** - Object detection
- **OpenDataCam** - Counting application

## See also

- [[android-on-drone]] — phone as companion / ML / 4G bridge
- [[drone-vision-software]]
- [[oss-drone-fms-pipeline]]
- [[implementation-plan]]
- [[herd-scout-positioning]]