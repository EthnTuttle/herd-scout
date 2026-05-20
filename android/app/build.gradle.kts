import org.gradle.internal.os.OperatingSystem

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "com.herdscout.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.herdscout.app"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            // Only arm64-v8a for now — covers all modern Android phones we care
            // about. Adding armeabi-v7a or x86_64 means more `rustup target add`
            // and a fatter APK; opt in when needed.
            abiFilters += listOf("arm64-v8a")
        }

        // Standard AndroidX test runner; we don't have tests yet but Gradle
        // bakes one in by default and complains if it's missing.
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
        }
        release {
            isMinifyEnabled = false
            // Use the debug signing key so a release APK can install on device
            // without setting up a real keystore. Tighten before any public
            // distribution.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        viewBinding = true
        // Compose intentionally disabled — single-screen XML keeps the app
        // light and gradle init fast.
    }

    // jniLibs are shipped via cargo-ndk into src/main/jniLibs/<abi>/.
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")

    // The cargo-ndk task below populates jniLibs; ensure preBuild waits for it.
    // Gradle wires task dependencies via `dependsOn(...)` below.
}

dependencies {
    implementation(libs.core.ktx)
    implementation(libs.appcompat)
    implementation(libs.activity.ktx)
    implementation(libs.lifecycle.runtime)
    implementation(libs.lifecycle.service)
    implementation(libs.coroutines.android)
    implementation(libs.camerax.core)
    implementation(libs.camerax.camera2)
    implementation(libs.camerax.lifecycle)
    implementation(libs.camerax.view)
    implementation(libs.camerax.mlkit)
    implementation(libs.mlkit.barcode)
}

// ── cargo-ndk integration ────────────────────────────────────────────────────
//
// We invoke cargo-ndk via an Exec task rather than depending on the
// org.mozilla.rust-android-gradle plugin: cargo-ndk is what the iroh-live
// demo uses, it's lighter weight, and it stays out of the way when the user
// just wants `./gradlew tasks` without an NDK installed.
//
// Requires:
//   - $ANDROID_HOME pointing at the Android SDK
//   - An NDK installed under $ANDROID_HOME/ndk/<version>/ (we auto-pick the
//     newest one)
//   - `cargo install cargo-ndk` on the developer's machine
//   - `rustup target add aarch64-linux-android`
//
// Skips (with a warning) when the NDK or cargo-ndk is missing so the Gradle
// configuration phase still succeeds for IDE indexing.

val rustCrateName = "herd-scout-jni"
val rustLibName = "libherd_scout_jni.so"
val rustWorkspaceDir = file("../..").canonicalFile
val jniLibsDir = file("src/main/jniLibs")

fun detectNdkHome(): File? {
    val androidHome = System.getenv("ANDROID_HOME")
        ?: System.getenv("ANDROID_SDK_ROOT")
        ?: return null
    val ndkDir = File(androidHome, "ndk")
    if (!ndkDir.isDirectory) return null
    return ndkDir.listFiles()?.filter { it.isDirectory }
        ?.maxByOrNull { it.name }
}

fun cargoNdkAvailable(): Boolean {
    val which = if (OperatingSystem.current().isWindows) "where" else "which"
    return try {
        val proc = ProcessBuilder(which, "cargo-ndk").redirectErrorStream(true).start()
        proc.waitFor() == 0
    } catch (_: Exception) {
        false
    }
}

tasks.register<Exec>("cargoNdkBuildDebug") {
    group = "build"
    description = "Builds the herd-scout-jni Rust cdylib for arm64-v8a via cargo-ndk."

    val ndkHome = detectNdkHome()
    val available = cargoNdkAvailable() && ndkHome != null
    onlyIf {
        if (!available) {
            logger.warn(
                "cargoNdkBuildDebug: skipping (cargo-ndk available=${cargoNdkAvailable()}, " +
                    "NDK home=${ndkHome?.absolutePath ?: "missing"}). " +
                    "Install with: cargo install cargo-ndk; rustup target add aarch64-linux-android"
            )
        }
        available
    }

    workingDir = rustWorkspaceDir

    if (ndkHome != null) {
        environment("ANDROID_NDK_HOME", ndkHome.absolutePath)
    }

    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a",
        "-P", "26",
        "-o", jniLibsDir.absolutePath,
        "build", "-p", rustCrateName,
    )

    // After cargo-ndk runs, drop sibling `.so` files it copied from C deps so
    // the APK stays small. We keep only our cdylib.
    doLast {
        val abiDir = File(jniLibsDir, "arm64-v8a")
        if (abiDir.isDirectory) {
            abiDir.listFiles()?.forEach { f ->
                if (f.name != rustLibName && f.name.endsWith(".so")) {
                    logger.info("Pruning extra jniLib: ${f.name}")
                    f.delete()
                }
            }
        }
    }
}

// Ensure cargoNdkBuildDebug runs before any APK assembly that would package
// the native lib. We hook every variant so debug + release both wait.
afterEvaluate {
    android.applicationVariants.forEach { variant ->
        val mergeJniTaskName = "merge${variant.name.replaceFirstChar { it.uppercase() }}JniLibFolders"
        tasks.matching { it.name == mergeJniTaskName }
            .configureEach { dependsOn("cargoNdkBuildDebug") }
    }
}
