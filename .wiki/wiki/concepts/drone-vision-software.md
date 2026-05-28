# Drone Vision Software Research

- title: Drone Vision Software for Herd Counting and Fenceline Inspection
- summary: Research into open source computer vision tools for analyzing live drone footage to count herds and inspect fencelines
- tags: [drone, computer-vision, yolo, object-detection, livestock, agriculture]
- created: 2026-05-19
- confidence: medium
- type: research

## Problem Statement

Create an open source vision software tool that can:
1. Quickly ingest live footage from a drone
2. Analyze what it's seeing in real-time
3. Count herds (livestock)
4. Check fencelines for damage/gaps

## Key Technologies

### Object Detection Models

#### YOLO (You Only Look Once)
- **YOLOv4/YOLOv5**: State-of-the-art real-time object detection
- **YOLOv7**: Current fastest/most accurate (56.8% AP at 30+ FPS on V100)
- Pre-trained on COCO dataset (80 classes including cattle, horses, sheep)
- Can be fine-tuned for custom livestock detection

**Key Repositories**:
- [AlexeyAB/darknet](https://github.com/AlexeyAB/darknet) - YOLOv4 implementation (22.2k stars)
- [ultralytics/yolov5](https://github.com/ultralytics/yolov5) - YOLOv5 in PyTorch (57.4k stars)
- [WongKinYiu/yolov7](https://github.com/WongKinYiu/yolov7) - YOLOv7 implementation

**Performance Comparison (V100 GPU)**:
| Model | mAP@0.5 | FPS |
|-------|---------|-----|
| YOLOv5n | 45.7% | 123 |
| YOLOv5s | 56.8% | 83 |
| YOLOv5m | 64.1% | 40 |
| YOLOv4-tiny | 40.2% | 330 |

### Edge Deployment

#### NVIDIA Jetson Family
- **Jetson Nano**: Entry-level, ~10-15 FPS with YOLOv4-tiny
- **Jetson Xavier NX**: Mid-range, 30+ FPS possible
- **Jetson AGX Orin**: High-end, 60+ FPS real-time processing
- JetPack SDK provides optimized TensorRT inference

#### OpenDataCam
- [opendatacam/opendatacam](https://github.com/opendatacam/opendatacam) (1.7k stars)
- Ready-to-use counting tool with YOLOv4
- Runs on Jetson devices
- Supports multiple video sources
- Built-in counter/tracking logic

## Video Input Options

### Drone Video Streaming Protocols

1. **RTSP (Real Time Streaming Protocol)**
   - Most common for IP cameras/drones
   - Low latency (~500ms)
   - `rtsp://<drone-ip>:8554/stream`

2. **RTMP (Real-Time Messaging Protocol)**
   - Used by many drone apps (DJI, etc.)
   - Slightly higher latency
   - Can be converted to other protocols

3. **UDP Streaming**
   - Lowest latency option
   - No error correction (packet loss possible)
   - Good for local networks

4. **Direct Video File**
   - Post-flight analysis
   - Highest quality

### Tools for Video Ingestion

- **OpenCV**: `cv2.VideoCapture('rtsp://...')`
- **FFmpeg**: `ffmpeg -i rtsp://... -f rawvideo -`
- **GStreamer**: Pipeline-based, very flexible

## Implementation Approach

### Phase 1: Core Detection

```
1. Use YOLOv5s or YOLOv7 as base model
2. Fine-tune on livestock dataset
3. Add custom class for "cattle", "sheep", "fence"
```

### Phase 2: Counting Logic

1. Object tracking (SORT, DeepSORT, or ByteTrack)
2. Define counting zones (polygon/line)
3. Increment counter when object crosses zone

### Phase 3: Fenceline Detection

1. Train on fence-specific dataset
2. Use segmentation (YOLOv5-seg) for precise boundaries
3. Detect gaps/changes between frames

### Phase 4: Edge Deployment

1. Optimize with TensorRT
2. Quantize to INT8 for speed
3. Deploy on Jetson Orin or Xavier NX

## Relevant Open Source Projects

| Project | Purpose | Stars |
|---------|---------|-------|
| ultralytics/yolov5 | Detection framework | 57.4k |
| AlexeyAB/darknet | YOLOv4 implementation | 22.2k |
| opendatacam/opendatacam | Counting system | 1.7k |
| bochinski/iou-tracker | Object tracking | 1.1k |
| WongKinYiu/yolov7 | YOLOv7 implementation | 8k+ |

## Data Collection

### Livestock Detection Datasets
- COCO dataset has "cow", "horse", "sheep" classes
- Custom dataset needed for specific cattle breeds
- Aerial viewpoint datasets limited but growing

### Fence Detection
- No pre-built datasets found
- Would need to create custom dataset
- Consider synthetic data generation

## Recommendations

1. **Start with YOLOv5s** - Good balance of speed/accuracy
2. **Use OpenDataCam architecture** as reference for counting
3. **Target Jetson Xavier NX** for edge deployment
4. **Collect custom dataset** for cattle/fence specific detection
5. **Use RTSP** for live drone feed input

## See also

- [[herd-counting-pipeline]] — how detector output becomes an accurate, calibrated count
- [[livestock-cv-accuracy]] — realistic precision / recall / MAE numbers from the literature
- [[oss-drone-fms-pipeline]] — where this fits in the broader pipeline (L6 real-time inference layer)
- [[precision-ag-drone-use-cases]] — adjacent use cases worth roadmap consideration
- [[herdnet-livestock-cv]] — alternative aerial livestock detection model
- [[android-on-drone]] — running this on a phone instead of Jetson
- [[drone-hardware]]
- [[implementation-plan]]
- [[herd-scout-positioning]]