// Wave 12 — herd-scout-admin Android app.
//
// Distinct applicationId (`com.herdscout.admin`) so it sideloads
// alongside the streaming app without conflict. Compose UI; depends on
// the shared library module for QR scanning, the JNI loader, and
// formatting helpers. Reuses the JNI cdylib produced by `:app`'s
// cargo-ndk task — both apps load the same `libherd_scout_jni.so`.
import org.gradle.internal.os.OperatingSystem

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
    alias(libs.plugins.kotlin.ksp)
}

android {
    namespace = "com.herdscout.admin"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.herdscout.admin"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            abiFilters += listOf("arm64-v8a")
        }

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
        }
        release {
            isMinifyEnabled = false
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
        compose = true
    }

    // Reuse the JNI cdylib already produced by :app's cargo-ndk task.
    sourceSets["main"].jniLibs.srcDirs(rootProject.file("app/src/main/jniLibs"))

    packaging {
        // Compose pulls in conflicting META-INF entries from multiple
        // dependencies; standard exclusions keep the APK lean and the
        // build deterministic.
        resources.excludes += setOf(
            "/META-INF/{AL2.0,LGPL2.1}",
            "/META-INF/INDEX.LIST",
            "/META-INF/io.netty.versions.properties",
        )
    }
}

dependencies {
    implementation(project(":shared"))

    implementation(libs.core.ktx)
    implementation(libs.appcompat)
    implementation(libs.activity.ktx)
    implementation(libs.lifecycle.runtime)
    implementation(libs.coroutines.android)

    // Compose
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.viewmodel.compose)
    debugImplementation(libs.compose.ui.tooling)

    // Wire format
    implementation(libs.kotlinx.serialization.json)

    // Local audit log (Decision 9)
    implementation(libs.room.runtime)
    implementation(libs.room.ktx)
    implementation(libs.room.paging)
    implementation(libs.paging.runtime)
    implementation(libs.paging.compose)
    ksp(libs.room.compiler)

    // QR rendering (NodeId → QR for the My Identity screen). The
    // shared module already brings ML Kit for QR *scanning*; we only
    // need pure-Java ZXing core for *encoding*, no Play Services dep.
    implementation(libs.zxing.core)
}

// ── cargo-ndk dependency ────────────────────────────────────────────────
// `:admin` reuses :app's cargo-ndk output. Wire the merge task to wait
// on :app's build so a clean checkout's first `:admin:assembleDebug`
// produces the .so.
afterEvaluate {
    android.applicationVariants.forEach { variant ->
        val mergeJniTaskName = "merge${variant.name.replaceFirstChar { it.uppercase() }}JniLibFolders"
        tasks.matching { it.name == mergeJniTaskName }
            .configureEach { dependsOn(":app:cargoNdkBuildDebug") }
    }
}

// Keep this around to silence "unused" warnings if cargo-ndk plumbing
// is later inlined here too.
@Suppress("unused")
private fun isWindows() = OperatingSystem.current().isWindows
