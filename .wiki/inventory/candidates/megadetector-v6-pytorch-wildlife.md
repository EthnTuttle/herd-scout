---
title: "Source candidate: MegaDetectorV6 + PyTorch-Wildlife + SPARROW"
type: source-candidate
priority: p1
created: 2026-06-02
source: assess-herd-scout-2026-06-02
status: queued
target_topic: herd-scout (local)
licenses_to_verify: yes
---

# Source candidate: MegaDetectorV6 + PyTorch-Wildlife + SPARROW

## Why ingest

The closest direct technical analog to herd-scout's drone-counting pipeline. Three coordinated MIT-licensed Microsoft AI for Good releases:

- **MegaDetectorV6** — YOLOv10/YOLOv9/RT-DETR backbone, MIT, several-million-image training set. Detects animal/person/vehicle in camera-trap frames. https://github.com/agentmorris/MegaDetector
- **PyTorch-Wildlife** — wraps MegaDetector with classifiers and HerdNet for aerial point-detection. https://github.com/microsoft/Pytorch-Wildlife
- **SPARROW** — solar + Jetson Orin Nano edge device that runs MegaDetectorV6 offline, queues findings for satellite sync, ships privacy-scrubbing on-device. Almost exactly the "edge counter that uplinks when reachable" pattern herd-scout needs (swap satellite for iroh P2P). https://github.com/microsoft/SPARROW

## What to extract

1. MegaDetectorV6 weights + license terms (MIT for code; verify weight licenses are not poison-pilled like HerdNet's CC-BY-NC-SA).
2. PyTorch-Wildlife integration pattern — could replace or supplement the current Python sidecar's YOLO11s.
3. SPARROW's queue-and-sync architecture (Docker Compose, on-device inference, privacy scrubbing, sat uplink) as a reference for the herd-scout daemon edge-deployment story.

## Suggested ingest commands

```
/wiki:ingest https://github.com/agentmorris/MegaDetector
/wiki:ingest https://github.com/microsoft/Pytorch-Wildlife
/wiki:ingest https://github.com/microsoft/SPARROW
```

## See also
- [[../../wiki/concepts/herd-counting-pipeline]]
- [[../../wiki/concepts/cattle-reid-self-supervised]]
- [[../../output/assess-herd-scout-2026-06-02]] §Market Gaps, §Adjacent fields
