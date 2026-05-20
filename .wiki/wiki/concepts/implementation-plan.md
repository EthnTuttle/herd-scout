# Implementation Plan: Phone-Based Drone Herd Counter

- title: Implementation Plan - Phone-Based Herd Counting Drone
- summary: Practical steps to build an open source drone herd counting system using a mounted Android phone
- tags: [implementation, plan, herd-counting, android, yolo]
- created: 2026-05-19
- type: plan
- status: proposed

## Recommended Approach: Phone-Only Minimal System

**Cost target: $300-500** (drone parts only, using phone you already have)

## Phase 1: Acquire & Build Drone

### Option A: Buy Kit (Recommended)
- **Holybro S500 V2 Kit** (~350) - frame, motors, ESCs, GPS, power module
- **Autopilot**: Holybro Kakute F7 AIO (~80)
- **Radio**: 915MHz telemetry (~40)
- **Battery**: 3S 5000mAh LiPo (~30)
- **Total**: ~$500

### Option B: Used DJI Frame
- Find used DJI F450/F550 on marketplace ($50-100)
- Add any compatible flight controller ($40-80)

## Phase 2: Install Phone & Software

### Hardware Mount
- Purchase phone gimbal mount (~30)
- Ensure phone faces downward for aerial view
- Secure with zip ties + vibration dampening

### On-Phone Software

#### Option 1: Stream to Ground Station
1. Install **IP Webcam** (free, open source)
2. Start server, note IP address
3. Configure RTSP output
4. On laptop: connect to same WiFi, run YOLO on video stream

#### Option 2: Run Everything on Phone
1. Install **Termux** (F-Droid)
2. Install Python + dependencies
3. Download YOLO TFLite model
4. Run detection loop

## Phase 3: Computer Vision Setup

### Model Selection

| Model | Size | Speed (phone) | Accuracy |
|-------|------|---------------|----------|
| YOLOv5n | 7MB | ~15 FPS | Low |
| YOLOv5s | 28MB | ~8 FPS | Medium |

**Recommendation: Start with YOLOv5n or YOLOv5s**

### Get Pre-trained Model
```bash
wget https://github.com/ultralytics/yolov5/releases/download/v7.0/yolov5n6.tflite
```

### COCO Livestock Classes
- `cow` (class 15)
- `horse` (class 17)
- `sheep` (class 18)

## Phase 4: Counting Logic

### Simple Counter
```python
detections = run_yolo(frame)
cattle_count = len([d for d in detections if d.class in [cow, horse, sheep]])
```

## Phase 5: Ground Station

- **QGroundControl** - Flight control
- **Custom Python script** - YOLO processing
- **MAVLink** - Get drone position for geotagging

## Testing Checklist

- [ ] Drone flies stable in Loiter mode
- [ ] Phone mounts securely, no vibration
- [ ] IP Webcam streams to ground station
- [ ] YOLO runs on video feed
- [ ] Detects cattle/sheep in frame
- [ ] Counts update in real-time

## Future Improvements

1. Fine-tune model on your specific cattle
2. Add fence detection with segmentation
3. Use Jetson for faster processing
4. Add 4G for beyond-line-of-sight
5. Autonomous missions with preset survey paths

## Budget Summary

| Item | Cost |
|------|------|
| S500 Kit | $350 |
| Kakute F7 AIO | $80 |
| Telemetry radio | $40 |
| Battery | $30 |
| Phone mount | $30 |
| **Total** | **$530** |

(Phone is "free" - use what you have)