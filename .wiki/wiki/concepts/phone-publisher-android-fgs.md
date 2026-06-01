---
title: "Android publisher foreground-service constraints (API 34/35/36)"
summary: "Manifest, permissions, and the dataSync 6-hour cap that affect the herd-scout publisher when targeting Android 14/15/16"
tags: [android, foreground-service, fgs, api-34, api-35, api-36, camera, manifest]
created: 2026-06-01
confidence: high
type: concept
---

# Android publisher foreground-service constraints

The herd-scout publisher already runs as a foreground service ("keeps streaming with screen locked" per the Wave-13 docs). Android 14 introduced FGS-types-required; Android 15 added a **6-hour cap on `dataSync`**. This article codifies the right manifest for the drone-streaming use case.

## Recommended manifest

```xml
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_CAMERA" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE" />
<uses-permission android:name="android.permission.CAMERA" />
<uses-permission android:name="android.permission.CHANGE_WIFI_STATE" />

<service
  android:name=".PublisherService"
  android:foregroundServiceType="camera|connectedDevice"
  android:exported="false" />
```

**Critical: skip `dataSync`.** Classify the iroh upload as part of the `connectedDevice` link to the drone radio (justifiable since that's the network path).

## Why skip dataSync

[[2026-06-01-android-foreground-service-types|developer.android.com Android 15 docs]]:

- `dataSync` FGS is **capped at 6 hours per 24-hour rolling window** when targeting API 35+.
- After cap: system invokes `Service.onTimeout(int, int)`.
- Failure to `stopSelf()` → `RemoteServiceException`.
- Subsequent starts → `ForegroundServiceStartNotAllowedException` ("Time limit already exhausted for foreground service type dataSync").
- Timer **resets when user brings app to foreground**.

**`camera` and `connectedDevice` are NOT in the 6-hour list.** They remain uncapped through Android 16 (API 36). This is the practical fix for long multi-flight sessions.

## Background-launch restrictions

- **Camera-type FGS cannot be created while app is in background.** Must start from a foreground/visible context.
- Cannot launch from `BOOT_COMPLETED`.
- **All runtime permissions must be granted BEFORE `startForeground()`** or `SecurityException`. Watch out for any code path that starts the FGS before the user grants CAMERA.
- Once running with screen on, **survives screen-off** — the desired behavior.

## What's NOT in the official docs

These are real risks the operator playbook needs to capture:

- **OEMs with aggressive battery managers** (Xiaomi, Samsung One UI, OPPO) often kill FGS regardless of Android spec. Test the publisher on the actual donor phone model before fielding it; check dontkillmyapp.com per-OEM guidance.
- **AOSP runaway-resource killer** behavior on sustained CameraX + network on Android 15+ — not in official docs; would need AOSP commit grep to verify.

## Test flags (debug)

```bash
adb shell am compat enable FGS_INTRODUCE_TIME_LIMITS <pkg>
adb shell device_config put activity_manager data_sync_fgs_timeout_duration <ms>
```

## Forward compatibility

Android 16 (API 36) adds **no new FGS time limits**. `camera | connectedDevice` is stable through current and next-API future-proofing as of June 2026.

## Implications summary

- Use `camera|connectedDevice` foreground service type, **not** `dataSync`.
- Start the service from a foreground/visible activity, not from `BOOT_COMPLETED`.
- Grant runtime permissions **before** `startForeground()`.
- Test on the actual donor phone model — OEM kill behavior is OS-spec-independent.
- Document OEM exceptions in the operator playbook.

## See also

- [[android-on-drone]]
- [[phone-on-drone-airframe]]
- [[phone-power-on-drone]]
- [[phone-thermal-management]]

## Sources

- raw: [[2026-06-01-android-foreground-service-types]]
