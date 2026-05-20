# herd-scout-jni

Rust JNI bridge for the herd-scout Android camera publisher. Builds as a
`cdylib` (`.so`) on `aarch64-linux-android` and is loaded by the Kotlin app via
`System.loadLibrary("herd_scout_jni")`.

## What it does

Wraps `iroh-live`, `moq-media`, `moq-media-android`, and `rusty-codecs` to
expose four JNI entry points:

| JNI symbol | Purpose |
|------------|---------|
| `nativeConnectWithTicket(ticket: String) -> jlong` | Parse a `LiveTicket`, bind the iroh endpoint, return an opaque handle |
| `nativeStartStreaming(handle, w, h) -> jboolean` | Build a `LocalBroadcast` with a `CameraFrameSource` and call `Live::publish` |
| `nativePushCameraNv12(handle, y, uv, w, h, yStride, uvStride)` | Hot path — push one NV12 frame from CameraX into the encoder pipeline |
| `nativeDisconnect(handle)` | Tear down the iroh endpoint and free the handle |

Plus minor helpers (`nativeStopStreaming`, `nativeGetStatusLine`,
`nativeGetBroadcastName`).

The handle is an `Arc<Mutex<SessionHandle>>` leaked into a `jlong` via
`moq_media_android::handle`. All async work runs on a global tokio multi-thread
runtime initialized lazily on first use.

## Build

```sh
# Set up once
rustup target add aarch64-linux-android
cargo install cargo-ndk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<latest>

# Build the .so directly (or let Gradle do it via the cargoNdkBuildDebug task)
cargo ndk -t arm64-v8a -P 26 \
    -o ../android/app/src/main/jniLibs \
    build -p herd-scout-jni
```

The Gradle `cargoNdkBuildDebug` task in `../android/app/build.gradle.kts`
automates this on each `assembleDebug`.

## Workspace inclusion

This crate uses `workspace = true` for all the iroh-live deps. It must be
added to the workspace before `cargo build` will work. See the diff in this
wave's deliverable.

The crate is `cfg(target_os = "android")`-gated end-to-end: on host targets
(macOS / Linux desktop), the lib is essentially empty (modulo a `tracing`
dependency). That keeps `cargo build --workspace` cheap on dev machines
without an Android NDK installed.
