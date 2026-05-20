package com.herdscout.app

import android.content.Context
import android.graphics.ImageFormat
import android.util.Log
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Process-singleton that owns the JNI handle and the active CameraX
 * pipeline. The Activity asks it to do things; rotation or screen-off is
 * survived because the [StreamingService] keeps the process alive.
 *
 * State machine:
 *
 *   IDLE
 *     -> connect(ticket): IDLE -> CONNECTED (or back to IDLE on failure)
 *   CONNECTED
 *     -> startStreaming(...): CONNECTED -> STREAMING
 *     -> disconnect(): CONNECTED -> IDLE
 *   STREAMING
 *     -> stopStreaming(): STREAMING -> CONNECTED
 *     -> disconnect(): STREAMING -> IDLE
 *
 * Frames flow:
 *   CameraX.ImageAnalysis (background executor)
 *     -> [pushNv12] reads Y + UV planes
 *     -> JNI [HerdScoutJni.pushCameraNv12] (briefly locks the SessionHandle
 *        mutex on the Rust side)
 *     -> Rust encoder pipeline picks up the frame asynchronously
 */
object StreamingController {
    private const val TAG = "StreamingCtrl"
    private const val CAMERA_WIDTH = 1280
    private const val CAMERA_HEIGHT = 720

    enum class State { IDLE, CONNECTED, STREAMING, DISCONNECTED }

    private val _state = MutableStateFlow(State.IDLE)
    val state: StateFlow<State> = _state.asStateFlow()

    private val _statusText = MutableStateFlow("Idle")
    val statusText: StateFlow<String> = _statusText.asStateFlow()

    @Volatile
    private var handle: Long = 0
    private var cameraProvider: ProcessCameraProvider? = null
    private var imageAnalysis: ImageAnalysis? = null
    private var analyzerExecutor: ExecutorService? = null

    /** Connects using a scanned ticket. Returns true on success. */
    suspend fun connect(ticket: String): Boolean = withContext(Dispatchers.IO) {
        if (handle != 0L) {
            Log.w(TAG, "connect: already have a handle; disconnect first")
            return@withContext true
        }
        val h = HerdScoutJni.connectWithTicket(ticket)
        if (h == 0L) {
            _state.value = State.DISCONNECTED
            _statusText.value = "Connection failed — check ticket"
            return@withContext false
        }
        handle = h
        _state.value = State.CONNECTED
        _statusText.value = "Connected: ${HerdScoutJni.broadcastName(h)}"
        true
    }

    /**
     * Binds CameraX to [lifecycleOwner], wires the ImageAnalysis analyzer to
     * push NV12 frames into the JNI bridge, and calls into Rust to publish
     * the broadcast.
     */
    suspend fun startStreaming(
        context: Context,
        lifecycleOwner: LifecycleOwner,
        previewView: PreviewView,
    ): Boolean {
        val h = handle
        if (h == 0L) {
            Log.w(TAG, "startStreaming: no handle")
            return false
        }
        // Bind CameraX on main, but the Rust publish call is blocking I/O.
        val provider = ProcessCameraProvider.getInstance(context).get()
        cameraProvider = provider

        val resolutionSelector = ResolutionSelector.Builder()
            .setResolutionStrategy(
                ResolutionStrategy(
                    android.util.Size(CAMERA_WIDTH, CAMERA_HEIGHT),
                    ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER,
                )
            )
            .build()

        val preview = Preview.Builder()
            .setResolutionSelector(resolutionSelector)
            .build()
            .also { it.surfaceProvider = previewView.surfaceProvider }

        val analysis = ImageAnalysis.Builder()
            .setResolutionSelector(resolutionSelector)
            .setOutputImageFormat(ImageAnalysis.OUTPUT_IMAGE_FORMAT_YUV_420_888)
            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
            .build()
        imageAnalysis = analysis

        val executor = Executors.newSingleThreadExecutor()
        analyzerExecutor = executor
        analysis.setAnalyzer(executor) { image -> onFrame(image) }

        try {
            provider.unbindAll()
            provider.bindToLifecycle(
                lifecycleOwner,
                CameraSelector.DEFAULT_BACK_CAMERA,
                preview,
                analysis,
            )
        } catch (e: Exception) {
            Log.e(TAG, "CameraX bind failed", e)
            _statusText.value = "Camera failed: ${e.message}"
            return false
        }

        // Tell Rust to spin up the encoder pipeline + iroh publish. This
        // happens on Dispatchers.IO so we don't block the main thread.
        val ok = withContext(Dispatchers.IO) {
            HerdScoutJni.startStreaming(h, CAMERA_WIDTH, CAMERA_HEIGHT)
        }
        if (!ok) {
            _statusText.value = "Streaming failed"
            stopStreamingInternal()
            return false
        }

        StreamingService.start(context.applicationContext)
        _state.value = State.STREAMING
        _statusText.value = "Streaming"
        return true
    }

    fun stopStreaming(context: Context) {
        val h = handle
        if (h != 0L) {
            HerdScoutJni.stopStreaming(h)
        }
        stopStreamingInternal()
        StreamingService.stop(context.applicationContext)
        _state.value = if (handle != 0L) State.CONNECTED else State.IDLE
        _statusText.value = if (handle != 0L) {
            "Connected: ${HerdScoutJni.broadcastName(handle)}"
        } else {
            "Idle"
        }
    }

    fun disconnect(context: Context) {
        val h = handle
        handle = 0
        stopStreamingInternal()
        StreamingService.stop(context.applicationContext)
        if (h != 0L) {
            // Native disconnect blocks on tokio shutdown; do not call from main
            // in production, but we tolerate a small stall here.
            HerdScoutJni.disconnect(h)
        }
        _state.value = State.IDLE
        _statusText.value = "Idle"
    }

    /** Called by the UI on a 1Hz timer to refresh stats. */
    fun refreshStatusFromNative() {
        val h = handle
        if (h == 0L) return
        val line = HerdScoutJni.statusLine(h)
        if (line.isNotEmpty()) {
            _statusText.value = line
        }
    }

    private fun stopStreamingInternal() {
        cameraProvider?.unbindAll()
        cameraProvider = null
        imageAnalysis = null
        analyzerExecutor?.shutdown()
        analyzerExecutor = null
    }

    /**
     * Camera analyzer callback — runs on [analyzerExecutor]. Reads NV12
     * planes from the [ImageProxy] and pushes them through JNI. CameraX
     * gives us YUV_420_888 with UV pixel-stride 2 on virtually all modern
     * devices, which IS NV12; if we ever land on a planar I420 device we
     * fall back to interleaving manually.
     *
     * Identical pattern to the iroh-live demo's `pushNv12` so we inherit
     * its battle-tested handling of stride padding.
     */
    private fun onFrame(image: ImageProxy) {
        val h = handle
        if (h == 0L || image.format != ImageFormat.YUV_420_888) {
            image.close()
            return
        }
        try {
            val yPlane = image.planes[0]
            val uvPlane = image.planes[1]
            val vPlane = image.planes[2]

            val width = image.width
            val height = image.height
            val yStride = yPlane.rowStride
            val uvStride = uvPlane.rowStride
            val uvPixelStride = uvPlane.pixelStride

            val yBuf = yPlane.buffer
            val ySize = yStride * height
            val yData = ByteArray(ySize)
            yBuf.position(0)
            yBuf.get(yData, 0, ySize.coerceAtMost(yBuf.remaining()))

            val uvHeight = height / 2

            if (uvPixelStride == 2) {
                // Native NV12: interleaved UV, just copy.
                val uvBuf = uvPlane.buffer
                val uvSize = uvStride * uvHeight
                val uvData = ByteArray(uvSize)
                uvBuf.position(0)
                uvBuf.get(uvData, 0, uvSize.coerceAtMost(uvBuf.remaining()))
                HerdScoutJni.pushCameraNv12(h, yData, uvData, width, height, yStride, uvStride)
            } else {
                // Planar I420 (rare): manually interleave U+V.
                val uBuf = uvPlane.buffer
                val vBuf = vPlane.buffer
                val uvWidth = width / 2
                val uvData = ByteArray(uvStride * uvHeight)
                for (row in 0 until uvHeight) {
                    for (col in 0 until uvWidth) {
                        val srcIdx = row * uvPlane.rowStride + col
                        val dstIdx = row * uvStride + col * 2
                        uvData[dstIdx] = uBuf.get(srcIdx)
                        uvData[dstIdx + 1] = vBuf.get(srcIdx)
                    }
                }
                HerdScoutJni.pushCameraNv12(h, yData, uvData, width, height, yStride, width)
            }
        } finally {
            image.close()
        }
    }

    /** True if a session handle exists (Connected or Streaming). */
    fun hasHandle(): Boolean = handle != 0L
}
