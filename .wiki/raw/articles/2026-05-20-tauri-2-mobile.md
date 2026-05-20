---
title: "Tauri 2 — Rust-native cross-platform mobile + desktop app framework"
source_url: https://v2.tauri.app
type: project
tags: [tauri, rust, cross-platform, mobile, desktop, ui-stack, oss]
created: 2026-05-20
confidence: high
---

# Tauri 2

- License: Apache-2.0 / MIT
- Architecture: Rust backend + system webview frontend
- Mobile: iOS + Android (added in 2.0)
- Desktop: Windows + macOS + Linux

## Why it matters for herd-scout

Tauri 2 is the only mature OSS path that lets a project keep a single Rust process — and therefore a single iroh node + single sync stack — across mobile and desktop. This is the user's "mobile to desktop" requirement made literal.

## Production status (2026)

- **Desktop**: stable, mature, broadly shipped
- **Mobile**: shipped distribution guides for App Store / Play Store, but mobile path started as alpha (2.0.0-alpha.0 late 2022). Known issues persist:
  - TLS / OpenSSL cross-compile complexity
  - Xcode device deployment quirks
  - Thinner mobile plugin ecosystem than Capacitor / Flutter

## Alternatives evaluated

| Stack | Mobile | Desktop | Rust-native | Maturity |
|---|---|---|---|---|
| **Tauri 2** | iOS+Android (alpha-tier) | Win/Mac/Linux (stable) | Yes | Mobile rough; desktop solid |
| **Compose Multiplatform (Kotlin)** | iOS+Android (stable) | Win/Mac/Linux (stable) | No | Most mature today; production at Wrike, Physics Wallah (17M users) |
| **Flutter** | iOS+Android (mature) | Win/Mac/Linux (production-capable) | No (Dart) | Mature mobile, smaller desktop install base |
| **Dioxus** | Roughly Tauri-mobile tier | Solid | Yes | Smaller production footprint |
| **KMP w/o Compose / RN+Electron / Capacitor+Electron** | — | — | No | Two UI codebases — rejected |

## Recommendation for herd-scout

**Primary pick**: Tauri 2 (Rust unified mobile+desktop) + iroh + iroh-docs + iroh-blobs + local SQLite.

**Fallback if Tauri 2 mobile alpha pain blocks shipping**: Compose Multiplatform UI calling into the Rust/iroh core via UniFFI. Keeps the data plane Rust; UI gets Compose's stability.
