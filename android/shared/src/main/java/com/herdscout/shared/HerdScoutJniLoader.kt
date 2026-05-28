package com.herdscout.shared

/**
 * Single load-point for the `herd_scout_jni` cdylib.
 *
 * Both `:app` (streaming) and `:admin` (allowlist manager) consume
 * the same `.so`; loading it twice in one process is fine but
 * routing both apps through this object keeps the call site uniform
 * and makes future "split into two cdylibs" work trivial.
 */
object HerdScoutJniLoader {
    @Volatile
    private var loaded: Boolean = false

    @Synchronized
    fun ensureLoaded() {
        if (loaded) return
        System.loadLibrary("herd_scout_jni")
        loaded = true
    }
}
