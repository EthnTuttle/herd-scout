// Top-level build file for the herd-scout Android workspace.
//
// Versions kept in sync with the iroh-live demo at
// vendor/iroh-live/demos/android/build.gradle.kts so we hit the same
// well-trodden Gradle/Kotlin/AGP toolchain. Wave 12 adds the admin
// app + a shared library module + Compose / Room / kotlinx.serialization.
plugins {
    id("com.android.application") version "8.7.3" apply false
    id("com.android.library") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.1.0" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.1.0" apply false
    id("org.jetbrains.kotlin.plugin.serialization") version "2.1.0" apply false
    id("com.google.devtools.ksp") version "2.1.0-1.0.29" apply false
}
