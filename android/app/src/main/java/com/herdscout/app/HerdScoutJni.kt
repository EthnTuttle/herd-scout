package com.herdscout.app

/**
 * Kotlin facade over the Rust JNI library.
 *
 * The Rust side stores all session state in an `Arc<Mutex<SessionHandle>>` and
 * exposes it as an opaque `Long` ("handle"). All native methods on this object
 * are blocking; call them from a background dispatcher (`Dispatchers.IO`).
 *
 * Lifecycle:
 *   1. [connectWithTicket] -> handle (or 0 on failure)
 *   2. [startStreaming] -> begins publishing
 *   3. [pushCameraNv12] called from the camera analyzer thread for each frame
 *   4. [stopStreaming] (optional) — stop frames but keep the handle alive
 *   5. [disconnect] — frees the handle. Do not use the handle after this.
 */
object HerdScoutJni {
    init {
        System.loadLibrary("herd_scout_jni")
    }

    fun connectWithTicket(ticket: String): Long = nativeConnectWithTicket(ticket)

    fun startStreaming(handle: Long, width: Int, height: Int): Boolean =
        nativeStartStreaming(handle, width, height)

    fun pushCameraNv12(
        handle: Long,
        yData: ByteArray,
        uvData: ByteArray,
        width: Int,
        height: Int,
        yStride: Int,
        uvStride: Int,
    ) = nativePushCameraNv12(handle, yData, uvData, width, height, yStride, uvStride)

    fun stopStreaming(handle: Long) = nativeStopStreaming(handle)

    fun disconnect(handle: Long) = nativeDisconnect(handle)

    fun statusLine(handle: Long): String = nativeGetStatusLine(handle)

    fun broadcastName(handle: Long): String = nativeGetBroadcastName(handle)

    private external fun nativeConnectWithTicket(ticket: String): Long

    private external fun nativeStartStreaming(handle: Long, width: Int, height: Int): Boolean

    private external fun nativePushCameraNv12(
        handle: Long,
        yData: ByteArray,
        uvData: ByteArray,
        width: Int,
        height: Int,
        yStride: Int,
        uvStride: Int,
    )

    private external fun nativeStopStreaming(handle: Long)

    private external fun nativeDisconnect(handle: Long)

    private external fun nativeGetStatusLine(handle: Long): String

    private external fun nativeGetBroadcastName(handle: Long): String
}
