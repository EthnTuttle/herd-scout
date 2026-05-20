# herd-scout Android app

The "phone bolted to anything" side of the herd-scout MVP.

## What it does

Captures back-camera frames at 720p via CameraX, hardware-encodes H.264 via
Android `MediaCodec` (through the Rust `moq-media` crate), and publishes a moq
broadcast over iroh QUIC to a desktop subscriber. Pairing is by QR code: the
desktop generates an iroh-live ticket, displays it as a QR, the phone scans
and connects.

## Layout

```
android/
  build.gradle.kts       Top-level Gradle build
  settings.gradle.kts    Module list (just :app)
  gradle/
    libs.versions.toml   Version catalog (AGP, Kotlin, CameraX, ML Kit)
    wrapper/             Gradle wrapper (jar fetched on first run)
  app/
    build.gradle.kts     App module + cargo-ndk integration task
    src/main/
      AndroidManifest.xml
      java/com/herdscout/app/
        MainActivity.kt           Single-screen UI
        StreamingController.kt    JNI handle + CameraX glue (process-singleton)
        StreamingService.kt       Foreground service (camera type)
        QrScanActivity.kt         ML Kit barcode scanner activity
        HerdScoutJni.kt           Kotlin facade over the Rust JNI library
      res/                        Layouts, strings, themes, icons
```

The Rust JNI crate lives at `../android-jni/`.

## Build

Prerequisites:

- JDK 17+
- Android Studio with SDK 35 and at least one NDK installed under
  `$ANDROID_HOME/ndk/`. NDK 26+ recommended.
- Rust target: `rustup target add aarch64-linux-android`
- `cargo install cargo-ndk`
- `ANDROID_HOME` env var pointing at the Android SDK

Then:

```sh
cd android
./gradlew :app:assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

The Gradle build invokes `cargo-ndk` automatically via the `cargoNdkBuildDebug`
task to produce `app/src/main/jniLibs/arm64-v8a/libherd_scout_jni.so`. If the
NDK or `cargo-ndk` is missing, the task is skipped with a warning so IDE
indexing still works — but the resulting APK won't run.

## Wiring it up to the desktop

1. Run the desktop app (`cargo run -p p2p-video-pipe-desktop`).
2. Desktop displays a ticket as a QR code (Wave 5A) or prints to stdout.
3. On the phone: tap **Scan Ticket**, point at the QR code.
4. Tap **Start Streaming**. Desktop should show the live camera feed within
   ~3 seconds.

## Permissions

| Permission | Why |
|------------|-----|
| `CAMERA` | Capture frames + scan pairing QR |
| `INTERNET` | iroh QUIC + relay traffic |
| `ACCESS_NETWORK_STATE` | Detect connectivity changes |
| `WAKE_LOCK` | Keep network awake during locked-screen flight |
| `FOREGROUND_SERVICE` + `FOREGROUND_SERVICE_CAMERA` | Stream survives screen-off |
| `POST_NOTIFICATIONS` | Foreground-service notification |
| `ACCESS_FINE_LOCATION` | Geotagging metadata (optional) |
