---
title: "Source candidate: WildlifeReID-10K + AnimalCLEF2026 + cattle-specific datasets"
type: source-candidate
priority: p1
created: 2026-06-02
source: assess-herd-scout-2026-06-02
status: queued
target_topic: herd-scout (local)
licenses_to_verify: yes
---

# Source candidate: WildlifeReID-10K, AnimalCLEF2026, and cattle-specific datasets

## Why ingest

The wildlife-CV ecosystem just published cattle-specific Re-ID benchmarks that should replace HerdNet as herd-scout's primary evaluation harness:

- **WildlifeReID-10K + WildlifeDatasets** — 53 datasets including cattle-specific: **CattleMuzzle** (added 2025-08), **CoBRAReIdentificationYoungstock** (added 2025-08), **HolsteinCattleRecognition** (added 2025-08), **MultiCamCows2024** (added 2025-04). https://github.com/WildlifeDatasets/wildlife-datasets
- **AnimalCLEF2026** — 2026-01 Re-ID challenge. https://wildlifedatasets.github.io/wildlife-datasets/
- **wildlife-tools 1.0.1** (2024-11) ships **WildFusion** calibrated score-fusion. https://github.com/WildlifeDatasets/wildlife-tools
- **MegaDescriptor-L-384** (Swin-L, 228M params, BVRA) — practical animal Re-ID backbone trained on the full WildlifeDatasets corpus. https://huggingface.co/BVRA/MegaDescriptor-L-384

## What to extract

1. Cattle-specific dataset licenses — MultiCamCows2024 is **CC BY-NC-SA 4.0** (non-commercial; per [[../../wiki/concepts/cattle-reid-self-supervised]]); confirm license terms for CattleMuzzle, CoBRAReIdentificationYoungstock, HolsteinCattleRecognition before relying on them in shipped product.
2. Whether MegaDescriptor + DINOv3 + WildFusion gives the "no per-farm fine-tune" zero-shot cattle Re-ID the wiki has been hoping for ([[../../wiki/concepts/cattle-reid-self-supervised]]).
3. AnimalCLEF2026 evaluation protocol — adopt it as herd-scout's standard Re-ID benchmark instead of HerdNet's stalled v0.2.1 (Mar 2024).
4. WildFusion calibrated score-fusion — could feed directly into layer-5 conformal validation.

## Suggested ingest commands

```
/wiki:ingest https://github.com/WildlifeDatasets/wildlife-datasets
/wiki:ingest https://github.com/WildlifeDatasets/wildlife-tools
/wiki:ingest https://wildlifedatasets.github.io/wildlife-datasets/
/wiki:ingest https://huggingface.co/BVRA/MegaDescriptor-L-384
```

## See also
- [[../../wiki/concepts/cattle-reid-self-supervised]]
- [[../../wiki/concepts/herd-counting-pipeline]] §Layer 5 validation
- [[../../wiki/concepts/livestock-cv-accuracy]]
- [[../../output/assess-herd-scout-2026-06-02]] §Emerging trends, §Market Gaps
- [[megadetector-v6-pytorch-wildlife]]
