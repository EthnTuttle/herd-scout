---
title: "HerdNet deep-dive: integration assessment for herd-scout"
source_url: https://github.com/Alexandre-Delplanque/HerdNet
type: assessment
tags: [herdnet, livestock, aerial-imagery, deep-learning, computer-vision, oss, integration, onnx, ort, yolov5n]
created: 2026-05-21
confidence: medium-high
status: active
---

# HerdNet deep-dive: integration assessment for herd-scout

## TL;DR

HerdNet is **not** a drop-in replacement for YOLOv5n. It is a point-based density/heatmap detector (DLA-34 backbone + sigmoid heatmap head + Local-Maxima Decoding), trained on **African wildlife from a manned-aircraft / fixed-wing UAV at high altitude** — not pasture livestock from a phone-camera at low altitude. The pretrained checkpoints are also **CC BY-NC-SA-4.0** (research only — incompatible with a commercial herd-scout product even though the *code* is MIT). And there is no published ONNX export. Verdict: **wrong fit for Wave 10 as currently scoped.** See section 12 for what to do instead.

## 1. The repository

- **Canonical repo confirmed**: `https://github.com/Alexandre-Delplanque/HerdNet` (created 2022-07-21, default branch `main`, GitHub repo id 516515002).
- **Stats** (via `gh api repos/Alexandre-Delplanque/HerdNet`, 2026-05-22):
  - 57 stars, 17 forks, 3 open issues, 0 archived
  - `pushed_at: 2025-12-09T08:11:29Z` (note: that is a fork-push timestamp; the latest commit on `main` itself is older — see below)
  - Topics: `aerial-survey, africa, deep-learning, livestock, mammals, object-counting, object-detection, wildlife`
- **Latest commit on `main`**: `7e25f48` "Merge pull request #8 from simbamangu/patch-1: fix: use default PIL font if segoeui not available", 2024-11-05. Substantive code work effectively stopped after the v0.2.1 release on 2024-03-26. The repo is **maintained-but-cold**, not abandoned.
- **Releases**: `v0.1.0` (paper-snapshot, 2023-01-23), `v0.2.0` (2023-03-29), `v0.2.1` (2024-03-26).
- **Open issues** (3): #15 "Adding batch validation", #14 "Single class detection is yielding strange results", #10 "A few corrections" — all minor.
- **Notable forks / successors**:
  - `cwinkelmann/HerdNet` — pushed 2026-05-12, very active. Adds DINOv3 / ConvNeXt-camouflaged backbones, micromamba/Docker setup, integration tests, models hosted on HuggingFace, used in Winkelmann's 2025 thesis on Galápagos marine iguana detection (DOI 10.6084/m9.figshare.30719999). **No ONNX export here either.**
  - `idchacon28/HerdNet-Demo` (4 stars), `simbamangu/HerdNet`, `FadelMamar/HerdNet`, `sfoucher/HerdNet` (co-author's fork). All are forks of the original; no replacement project has emerged.
  - **Successor papers** (per Google Scholar search, 2024-2026): Delplanque et al. 2024 "Will artificial intelligence revolutionize aerial surveys?", Dethier 2024 "HerdNetSat" (HerdNet adapted to satellite imagery, 32% detection rate), Durand/Foucher/Delplanque 2026 "Lacking data? No worries! Synthetic images for wildlife surveys" (muskox detection). **All of these still use HerdNet itself as the detection backbone**, so HerdNet is still SOTA in its (narrow) niche of aerial wildlife counting.

## 2. License

- **Code**: MIT (`LICENSE.md`, copyright "University of Liège, Gembloux Agro-Bio Tech, Forest Is Life").
- **Pretrained weights**: **CC BY-NC-SA-4.0**, "available for academic research purposes only, no commercial use is permitted." Verbatim from the README. **This is the show-stopper for herd-scout** if it is or will be a commercial product. We could legally use the *code* and *architecture*, but we'd have to retrain weights from scratch on a license-clean dataset.

## 3. Model architecture

Confirmed by reading `animaloc/models/herdnet.py`:

- **Backbone**: DLA-34 (Deep Layer Aggregation, 34-layer variant; ImageNet-pretrained encoder by default), with a learned `DLAUp` decoder.
- **Heads** (two parallel):
  1. **Localization head**: `Conv2d -> ReLU -> Conv2d(out=1) -> Sigmoid`, producing a single-channel heatmap (an FIDT-style — Focal Inverse Distance Transform — proxy for animal locations).
  2. **Classification head**: `Conv2d -> ReLU -> Conv2d(out=num_classes)`, producing per-pixel species logits at low resolution (operates on the bottleneck features).
- **Output**: a heatmap (not bounding boxes). Decoding to discrete points is done by `LMDS` (Local Maxima Detection Strategy in `animaloc/eval/lmds.py`, ported from the FIDTM paper) — pick local maxima with a `(3,3)` max-pool, threshold them, then assign species via the classification map.
- **Inference at full image scale** is done by the `HerdNetStitcher` (`animaloc/eval/stitchers.py`): chop the image into `512x512` patches with **160 px overlap**, run the model on each, then `mean`-reduce the overlapping heatmaps into a stitched whole-image map.
- **Architectural lineage**: this is essentially a **CenterNet-style point detector adapted to aerial small-object counting**. Single-stage. **18M parameters** (vs YOLOv5n's ~1.9M).

What this means for our pipeline: roughly zero of the YOLOv5n postprocessing path (NMS over bounding boxes, anchor decoding, IoU thresholds) is reusable. We'd write new postprocessing: max-pool peak finding + score thresholding + classification-map sampling at peak coordinates + tile stitching. That's a real chunk of code on top of `preprocess.rs` and `postprocess.rs`.

## 4. Pretrained weights

From the README "Pretrained Models" table (verbatim):

| Model | Params | Dataset | Environment | Species | F1 | MAE | RMSE | AC |
|---|---|---|---|---|---|---|---|---|
| HerdNet | 18M | Ennedi 2019 | Desert, xeric shrubland and grassland | Camel, donkey, sheep and goat | **73.6%** | 6.1 | 9.8 | 15.8% |
| HerdNet | 18M | Delplanque et al. 2022 | Tropical forest, savanna, tropical shrubland & grassland | Buffalo, elephant, kob, topi, warthog, waterbuck | **83.5%** | 1.9 | 3.6 | 7.8% |

- **Format**: PyTorch `.pth` (state-dict + `mean`/`std`/`classes` metadata stuffed into the checkpoint dict). Hosted on `dataverse.uliege.be` (file IDs 28087 and 28088). Size not advertised; for an 18M-param network in fp32, expect ~70 MB per checkpoint.
- **No ONNX, no safetensors, no TorchScript** — verified by reading the repo file tree (no `*.onnx`, no `export.py`, no `torch.onnx.export` call anywhere in the codebase, including all 13 forks I checked).
- **Species coverage relevant to herd-scout**: only the **Ennedi** model includes "sheep and goat", but the pretrained class set is a categorical mix (camel/donkey/sheep-goat) with no cattle or horses. **The pretrained weights have never seen a cow.** That is the single most important fact for a herd-scout integration decision.

## 5. Training data

- **Ennedi 2019**: aerial survey in Chad's Ennedi massif (manned aircraft / UAV, large-format imagery). Public via dataverse.uliege.be. Camel/donkey/sheep+goat — pastoralist livestock in arid/desert.
- **Delplanque et al. 2022 dataset**: doi 10.58119/ULG/MIRUU5 — UAV nadir imagery from African savanna/tropical forest ecosystems. Six wildlife species (buffalo, elephant, kob, topi, warthog, waterbuck). Companion paper Delplanque et al. 2022, Remote Sens Ecol Conserv 8:166-179, doi 10.1002/rse2.234. Per the abstract of that companion paper: best baseline model "achieved 73% detection accuracy on an independent UAV dataset with processing speeds of approximately 12 seconds per image" — note that's an older Faster-RCNN baseline, not HerdNet itself; specific altitude/GSD numbers were not retrievable through the open-access surface (Elsevier/ResearchGate returned 403 to anonymous fetches). What is clear from the imagery in the README and the patch size (512 px from very large source images, hence the stitcher) is that **inputs are large-format airborne stills (typically 4000+ px on the long side), not video frames**.
- **Implication for herd-scout's phone-on-drone use case**: HerdNet was trained at altitudes/GSDs and on perspective-rectified or near-nadir framings that look essentially nothing like a 1280x720 H.264 frame from a phone strapped to a low-altitude drone over a paddock. Out-of-distribution use is essentially guaranteed without retraining.

## 6. Input requirements

- **Patch size**: `512x512` per the canonical config (`configs/test/herdnet.yaml`).
- **Color space**: RGB (Albumentations `Normalize`).
- **Normalization**: ImageNet mean/std (`[0.485, 0.456, 0.406]` / `[0.229, 0.224, 0.225]`) — embedded in the `.pth` checkpoint.
- **Whole-frame vs tile**: **tile-based**. The expected use is large mosaics chopped to 512x512 with 160 px overlap, then stitched — see `HerdNetStitcher`. You *can* run on a single 512x512 patch directly, but at our 1280x720 input that's a 3x2 = 6-tile sweep per frame with overlap. At 10 FPS that's 60 inference calls/sec.
- **Down-ratio output**: the `down_ratio=2` default means the heatmap is half-resolution — the model emits a 256x256 location map per 512x512 patch.

## 7. Inference performance

- **Published benchmarks**: I could not retrieve specific HerdNet inference-time numbers from the paper (Elsevier/ResearchGate paywalled; Semantic Scholar didn't expose the abstract; orbi.uliege.be PDF returned 403). The companion 2022 paper quotes ~12 s/image for an older Faster-RCNN baseline on full-resolution airborne stills — that's an upper bound, not a HerdNet number.
- **Reasonable estimates**: 18M-param DLA-34 on a 512x512 patch is ~20-40 ms on a desktop CPU with ONNX Runtime, and ~5-10 ms on a midrange GPU. **At 1280x720 with 6 overlapping tiles per frame, expect ~150-250 ms per frame on CPU** — i.e. ~5 FPS, well below our 10 FPS budget. YOLOv5n on the same hardware does the whole frame in ~50 ms (per herd-scout's own measurements). HerdNet would be **3-5x slower** in this configuration even before the stitching/decoding overhead.

## 8. ONNX export path

- **No existing export script** — confirmed.
- **Likelihood `torch.onnx.export` "just works"**: medium. The architecture is mostly stock PyTorch ops (Conv2d, BN, ReLU, Sigmoid, transpose-conv-style upsamples in `DLAUp`). The risk areas are (a) the `DLAUp` decoder uses iterative aggregation that historically tripped older ONNX opsets but is fine on opset 17+; (b) the `LossWrapper` wrapper around the model would need to be unwrapped before export; (c) postprocessing (LMDS local-max + thresholding) **must not** be exported — keep it in Rust. So: a few hours of Python work for someone fluent in PyTorch-to-ONNX, plus validation on a sample image.
- **`ort` (Rust) compatibility**: opset 17 / 18 ONNX with standard CNN ops will load fine in `ort` 2.0.0-rc.12. No exotic ops.

## 9. Real-time vs post-flight

**HerdNet is a post-flight / batch tool, not a real-time video model.** Evidence:
- The CLI entry point is `tools/infer.py` taking a *folder of JPGs*, not a video stream.
- The whole inference path is built around "stitch a large airborne still into 512x512 patches" — that's photogrammetry post-processing, not video.
- There is no temporal modeling, no tracking, no frame-to-frame state.
- The README's intended workflow is identical to the WebODM pipeline: capture a flight, post-process the orthomosaic, get counts.

This is the deepest architectural mismatch with herd-scout, which is fundamentally a **live-video-overlay product**. To make HerdNet "real-time" you'd have to bypass the stitcher entirely (run on whole 1280x720 frames as ~6 tiles, or upscale to a single 1024-ish patch and accept resolution loss) — neither matches what HerdNet was designed for or trained on.

## 10. Citation / paper

- **Paper**: Delplanque, Foucher, Théau, Bussière, Vermeulen, Lejeune (2023). "From Crowd to Herd Counting: How to Precisely Detect and Count African Mammals using Aerial Imagery and Deep Learning?" *ISPRS Journal of Photogrammetry and Remote Sensing*, vol. 197, pp. 167-180. **DOI: 10.1016/j.isprsjprs.2023.01.025**.
- I was unable to extract the abstract or detailed altitude/GSD numbers from the paper PDF directly (publisher paywall returned 403 to WebFetch; ResearchGate also 403; Semantic Scholar only exposed metadata, not abstract). The numerical claims in section 4 above are taken verbatim from the repo's own README, which the authors maintain.
- F1 numbers (73.6% / 83.5%) are measured on the Ennedi and Delplanque-2022 test sets respectively, on **full-size aerial test images** (per the README footnote). They are **not** comparable to YOLOv5n COCO mAP — different metric, different domain, different image scale.

## 11. Successor / state of the art (aerial livestock 2024-2026)

Per Google Scholar (May 2026):
- HerdNet itself is still cited as the SOTA point-based architecture for aerial wildlife counting in 2024-2026 review articles.
- Delplanque et al. 2024 "Will AI revolutionize aerial surveys?" extends HerdNet to large-scale semi-automated wildlife surveys with oblique imagery — same model, more scale.
- **HerdNetSat** (Dethier 2024) adapts HerdNet to satellite imagery — interesting but not relevant to us (32% detection rate / 26.3% precision; satellite-scale).
- The active fork `cwinkelmann/HerdNet` (2026-05) is experimenting with **DINOv3 and ConvNeXt-camouflaged backbones** as drop-in replacements for DLA-34 — worth watching but not a finished product, and based on the commit messages "so far it is worse than the dla34 implementation".
- I found **no work** specifically on aerial *cattle/livestock* (vs African wildlife) counting that surpasses HerdNet in OSS. There may be commercial models (e.g. CattleEye, OneCup AI), but none are openly available.

## 12. Practical integration verdict

**Verdict: HerdNet is the wrong fit for a Wave 10 "swap-in for YOLOv5n" sprint.** Three independent reasons, any one of which would be sufficient:

1. **License blocker**: pretrained weights are CC BY-NC-SA-4.0 — academic only. herd-scout cannot ship them. The MIT code license doesn't help if we don't have weights.
2. **Domain mismatch**: pretrained models have never seen cattle, horses, or sheep-on-pasture from low-altitude phone-camera framing. Out-of-distribution drift will likely make the 73-83% F1 numbers irrelevant. Best case is "noisy but workable"; expected case is "worse than YOLOv5n's COCO cow/horse/sheep classes". To validate this we'd need a labeled phone-camera-on-drone livestock dataset, which we don't have.
3. **Real-time mismatch**: it is a post-flight tile-and-stitch tool; it was never designed for 10-30 FPS streaming and at our budget will be ~3-5x slower than YOLOv5n even after a clean ONNX export.

**Effort if we did it anyway**: not a weekend. Realistically a 2-3 week port (PyTorch → ONNX export script and validation, Rust preprocessing for tile-and-stitch, Rust postprocessing for LMDS peak-finding + classification-map sampling, integration into `desktop/src/cv/`, perf tuning). And at the end you'd still have an out-of-distribution model with no commercial license on the weights.

**What I'd recommend instead** (for a future wave, not now):
- **Wave 10 = stay on YOLOv5n**, but improve it: per-class confidence thresholds, light temporal smoothing across frames, a dataset-collection mode that saves labeled crops for fine-tuning later.
- **Wave 11+ = fine-tune YOLOv8n on a small in-house aerial-livestock dataset** captured during field tests. This is the same effort as a HerdNet port but produces a license-clean, real-time, in-domain model.
- **Reserve HerdNet for a future "post-flight count report" feature** if/when herd-scout adds an orthomosaic mode (cf. WebODM bridge plan). That's the use case it was actually designed for.

## Sources

- Repo metadata + README + file tree: `gh api repos/Alexandre-Delplanque/HerdNet` (2026-05-22).
- Model code: `animaloc/models/herdnet.py`, `animaloc/eval/stitchers.py`, `animaloc/eval/lmds.py`, `animaloc/models/dla.py`, `tools/infer.py`, `configs/test/herdnet.yaml`.
- Latest commit SHA: `7e25f482d875522c59c446dc0c78c8f6f2dd448d` (2024-11-05).
- Paper DOI: 10.1016/j.isprsjprs.2023.01.025.
- Companion dataset paper: 10.1002/rse2.234.
- Successor work: Google Scholar search for "Delplanque HerdNet aerial livestock", 2026-05-22.
- Active fork with newer experiments: github.com/cwinkelmann/HerdNet (last push 2026-05-12).

## Couldn't verify

- Exact paper abstract text (publisher paywall + ResearchGate/orbi 403 to WebFetch).
- Specific altitude / GSD numbers from the paper — derived only indirectly from "aerial nadir UAV imagery" + 512 px patches of much larger source frames.
- HerdNet-specific inference-time benchmark on commodity CPU — estimated from architecture, not measured. Would take an afternoon to benchmark locally if the call is close.
