# desktop/assets

Static assets bundled with the herd-scout desktop binary via `include_bytes!`.

## `yolov5n.onnx`

YOLOv5n (nano) image-classification + detection model exported to ONNX.
Trained on the COCO 80-class dataset; herd-scout filters for class
indices 17 (horse), 18 (sheep), and 19 (cow). All other classes are
masked out before NMS.

* About 3.8 MB on disk (smaller than the cv-design.md estimate of
  7.5 MB; the v7.0 release ships the post-fuse export, not the
  unfused training graph).
* ONNX opset 12, fixed input shape `[1, 3, 640, 640]`, output shape
  `[1, 25200, 85]`.
* Loaded at runtime via `include_bytes!("../assets/yolov5n.onnx")`.

If the file is missing the binary will fail to **compile** with a clear
`include_bytes!` error pointing at this README — that is the deliberate
fail-loudly state per `desktop/docs/cv-design.md`.

### How to (re-)obtain the file

The Wave 2C design doc explicitly forbids network fetches from
`build.rs`. The model has to land here by hand.

#### Source of record (verified 2026-05-21)

Despite the design doc's claim to the contrary, Ultralytics **does**
ship `yolov5n.onnx` directly in the v7.0 release. This is the smallest
post-fuse export of the nano weights, opset-12, 640×640, COCO80.

```sh
curl -L --fail \
  -o desktop/assets/yolov5n.onnx \
  https://github.com/ultralytics/yolov5/releases/download/v7.0/yolov5n.onnx

# Verify:
shasum -a 256 desktop/assets/yolov5n.onnx
# expected:
# 04f0e55c26f58d17145b36045780fe1250d5bd2187543e11568e5141d05b3262
```

#### Fallback — re-export from upstream weights

If the GitHub release URL ever 404s, re-export from the pinned `.pt`:

```sh
git clone --depth 1 --branch v7.0 https://github.com/ultralytics/yolov5
cd yolov5
pip install -r requirements.txt onnx onnx-simplifier
curl -L -o yolov5n.pt \
  https://github.com/ultralytics/yolov5/releases/download/v7.0/yolov5n.pt
python export.py --weights yolov5n.pt --include onnx --opset 12 --img 640
cp yolov5n.onnx <herd-scout-checkout>/desktop/assets/yolov5n.onnx
```

### Provenance / pin

* Upstream: `ultralytics/yolov5` tag `v7.0`, released 2022-11-22.
* Asset: `yolov5n.onnx` from the release-attached files.
* SHA256: `04f0e55c26f58d17145b36045780fe1250d5bd2187543e11568e5141d05b3262`
* Equivalent export command (kept for reproducibility):
  `python export.py --weights yolov5n.pt --include onnx --opset 12 --img 640`.
