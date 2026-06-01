---
title: "Grounding DINO for livestock — zero-shot ceiling and few-shot adaptation"
sources:
  - https://arxiv.org/abs/2509.06427
  - https://openaccess.thecvf.com/content/CVPR2025W/V4A/html/Singh_Few-Shot_Adaptation_of_Grounding_DINO_for_Agricultural_Domain_CVPRW_2025_paper.html
authors:
  - Dulal, Zheng, Kabir (Sep 2025)
  - Singh et al. (CVPRW 2025)
type: paper
tags: [grounding-dino, open-vocab, livestock, cattle, few-shot]
ingested: 2026-06-01
quality: 4
confidence: medium
---

# Grounding DINO for livestock — zero-shot vs few-shot

Two papers triangulate the realistic accuracy ceiling for open-vocabulary detection on livestock.

## Zero-shot ceiling (Dulal et al. 2025)

- Zero-shot Grounding DINO on cattle muzzle detection: **mAP@0.5 = 76.8%** with **no annotations**.
- First annotation-free industry-oriented livestock detection result.
- No GPU/latency reported. Implication: **Grounding DINO is too heavy for Pascal-era edge inference** — server-side enrichment tool only.

## Few-shot adaptation (Singh et al. CVPRW 2025)

- Few-shot adapted Grounding DINO **beats fully fine-tuned YOLO by ~24% mAP** on agricultural datasets.
- Method: drop BERT, use trainable text embeddings.
- **Zero-shot fails on visually-similar classes** — directly relevant for multi-breed or sex-classification tasks.
- ~10% improvement over SOTA in few-shot remote sensing.

## Implications for herd-scout

- Zero-shot open-vocab is **NOT a replacement** for fine-tuned YOLO11s/YOLO26 on the live edge sidecar (76.8% mAP < 90%+ for fine-tuned YOLO).
- **Server-side audit / labeling tool** is the right role — use Grounding DINO to bootstrap labeled frames for YOLO fine-tuning.
- Few-shot adaptation (BERT-removed, learned text embeddings) is the production-quality path if open-vocab is genuinely needed for operator queries — but it requires its own training pipeline, not a free zero-shot lift.
- Open-vocab queries like "limping cow" or "calf near fence" — flagged as a market opportunity in assess 2026-06-01 — are **research-grade**, not near-term shippable.
