---
title: "Android 14/15/16 Foreground Service Types — camera + connectedDevice + dataSync"
sources:
  - https://developer.android.com/about/versions/14/changes/fgs-types-required
  - https://developer.android.com/develop/background-work/services/fgs/service-types
  - https://developer.android.com/about/versions/15/behavior-changes-15
  - https://developer.android.com/about/versions/16/behavior-changes-16
type: article
tags: [android, foreground-service, fgs, api-34, api-35, api-36, camera]
ingested: 2026-06-01
quality: 5
confidence: high
---

# Foreground service constraints — Android 14/15/16

Canonical Google docs.

## Android 14 (API 34) — types required

- Targeting API 34+ mandates `android:foregroundServiceType` or **`MissingForegroundServiceTypeException`** at `startForeground()`.
- Multiple types combine with bitwise OR.
- **Critical ordering**: all runtime permissions must be granted **BEFORE `startForeground()`** or `SecurityException`. Bites apps starting FGS from `BOOT_COMPLETED` or before user grants CAMERA.

## Type-by-type for a drone camera+upload app

| Type | Use | Permissions |
|---|---|---|
| `camera` | Camera2/CameraX session | `FOREGROUND_SERVICE_CAMERA` + `CAMERA` runtime |
| `connectedDevice` | Drone radio link / external device | `FOREGROUND_SERVICE_CONNECTED_DEVICE` + at least one of network/Bluetooth perms |
| `dataSync` | Generic network upload | `FOREGROUND_SERVICE_DATA_SYNC` |
| `mediaProjection` | Screen capture only — irrelevant |
| `shortService` | ~3 min cap — wrong fit |
| `mediaProcessing` | 6 h cap — wrong fit |

## camera type background restriction

- **Cannot be created while app is in background** — must start from foreground/visible context.
- Cannot launch from `BOOT_COMPLETED`.
- Once running with screen on, **survives screen-off**.

## Android 15 (API 35) — `dataSync` 6-hour cap

**Big surprise for long-running drone sessions:**

- `dataSync` FGS now **capped at 6 hours per 24-hour rolling window** when targeting API 35+.
- After cap: system invokes `Service.onTimeout(int, int)`.
- Failure to `stopSelf()` → `RemoteServiceException` ("did not stop within its timeout").
- Subsequent starts → `ForegroundServiceStartNotAllowedException` ("Time limit already exhausted for foreground service type dataSync").
- Timer **resets** when user brings app to foreground.

**Practical fix for herd-scout publisher**: drop `dataSync` from the type set; rely on `camera | connectedDevice`. Both **uncapped** through API 36.

`SYSTEM_ALERT_WINDOW` background-FGS-launch loophole closed in Android 15: now requires a *visible* `TYPE_APPLICATION_OVERLAY`.

## Android 16 (API 36)

- **No new FGS time limits** beyond API-35 `dataSync` / `mediaProcessing` 6-hour caps.
- `camera` and `connectedDevice` remain uncapped.
- Only FGS-adjacent change: `FOREGROUND_SERVICE_TYPE_HEALTH` granular permissions — irrelevant for drone camera apps.

## Recommended manifest for herd-scout publisher

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

Skip `dataSync` to dodge the 6-hour cap; classify the iroh upload as part of the `connectedDevice` link to the drone (justifiable since the drone radio path *is* the network).

## Test flags

- `adb shell am compat enable FGS_INTRODUCE_TIME_LIMITS <pkg>`
- `device_config put activity_manager data_sync_fgs_timeout_duration <ms>`

## Gaps not in official docs

- Empirical screen-off camera reliability on **OEMs with aggressive battery managers** (Xiaomi, Samsung One UI, OPPO) — these often kill FGS regardless of spec. Operator playbook must include "test on actual donor phone model."
- Whether AOSP runaway-resource killer trips on sustained CameraX + network on Android 15+ — needs AOSP commit grep.
