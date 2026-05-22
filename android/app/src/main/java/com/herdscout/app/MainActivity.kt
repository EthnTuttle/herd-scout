package com.herdscout.app

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.View
import android.widget.Button
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Single-screen UI for herd-scout MVP.
 *
 * Three buttons drive the state machine in [StreamingController]:
 *  - **Scan Ticket** opens [QrScanActivity], decodes a QR, calls
 *    `connectWithTicket`, and on success enables Start Streaming.
 *  - **Start Streaming / Stop Streaming** toggles the CameraX -> JNI -> moq
 *    pipeline.
 *    Wave 7 Issue 2: Stop now drops the JNI handle and returns to IDLE.
 *    To start streaming again the user must rescan the QR (the desktop
 *    re-shows the QR automatically when the session ends). This avoids
 *    the "phantom session" bug where the daemon stayed in Reconnecting
 *    forever because the phone reused a torn-down `MoqSession`.
 *  - **Disconnect** tears the session down completely.
 *
 * The status TextView shows: "Idle" / "Connected: <name>" / "Streaming
 * 1280x720 | frames:N | 12s" / "Disconnected".
 */
class MainActivity : AppCompatActivity() {

    companion object {
        private const val TAG = "HerdScoutMain"
    }

    private lateinit var scanButton: Button
    private lateinit var startStopButton: Button
    private lateinit var disconnectButton: Button
    private lateinit var statusOverlay: TextView
    private lateinit var previewView: PreviewView

    private val scanLauncher = registerForActivityResult(QrScanActivity.Companion.Contract()) { result ->
        if (result.isNullOrBlank()) {
            statusOverlay.text = "Scan cancelled"
            return@registerForActivityResult
        }
        Log.i(TAG, "Got ticket from QR scan (${result.length} chars)")
        statusOverlay.text = "Connecting..."
        lifecycleScope.launch {
            val ok = StreamingController.connect(result.trim())
            if (ok) {
                refreshButtons()
            }
        }
    }

    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { grants ->
        val missing = grants.filterValues { !it }.keys
        if (missing.isNotEmpty()) {
            Log.w(TAG, "Permissions denied: $missing")
            statusOverlay.text = "Permissions denied: ${missing.joinToString()}"
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        scanButton = findViewById(R.id.scanButton)
        startStopButton = findViewById(R.id.startStopButton)
        disconnectButton = findViewById(R.id.disconnectButton)
        statusOverlay = findViewById(R.id.statusOverlay)
        previewView = findViewById(R.id.cameraPreview)

        scanButton.setOnClickListener { onScanClicked() }
        startStopButton.setOnClickListener { onStartStopClicked() }
        disconnectButton.setOnClickListener { onDisconnectClicked() }

        requestRuntimePermissions()
        wireStateObservers()
        refreshButtons()
    }

    override fun onDestroy() {
        // Don't disconnect on destroy: a configuration change (rotation) or
        // the user briefly leaving will recreate the activity, but the
        // foreground service + StreamingController keep the session alive.
        // Disconnect is explicit-only.
        super.onDestroy()
    }

    private fun requestRuntimePermissions() {
        val needed = mutableListOf(Manifest.permission.CAMERA)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            needed += Manifest.permission.POST_NOTIFICATIONS
        }
        // Optional location permission for geotagging metadata. Asked
        // best-effort; nothing breaks if denied.
        needed += Manifest.permission.ACCESS_FINE_LOCATION

        val toRequest = needed.filter {
            ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED
        }
        if (toRequest.isNotEmpty()) {
            permissionLauncher.launch(toRequest.toTypedArray())
        }
    }

    private fun wireStateObservers() {
        lifecycleScope.launch {
            StreamingController.statusText.collect { text ->
                statusOverlay.text = text
            }
        }
        lifecycleScope.launch {
            StreamingController.state.collect { _ ->
                refreshButtons()
            }
        }
        // Poll the native status line at 1Hz while streaming so the overlay
        // shows updated frame counts and rtt.
        lifecycleScope.launch {
            while (isActive) {
                if (StreamingController.state.value == StreamingController.State.STREAMING) {
                    StreamingController.refreshStatusFromNative()
                }
                delay(1000)
            }
        }
    }

    private fun refreshButtons() {
        when (StreamingController.state.value) {
            StreamingController.State.IDLE,
            StreamingController.State.DISCONNECTED -> {
                scanButton.isEnabled = true
                startStopButton.isEnabled = false
                startStopButton.text = getString(R.string.start_streaming)
                disconnectButton.isEnabled = false
                previewView.visibility = View.INVISIBLE
            }
            StreamingController.State.CONNECTED -> {
                scanButton.isEnabled = false
                startStopButton.isEnabled = true
                startStopButton.text = getString(R.string.start_streaming)
                disconnectButton.isEnabled = true
                previewView.visibility = View.INVISIBLE
            }
            StreamingController.State.STREAMING -> {
                scanButton.isEnabled = false
                startStopButton.isEnabled = true
                startStopButton.text = getString(R.string.stop_streaming)
                disconnectButton.isEnabled = true
                previewView.visibility = View.VISIBLE
            }
        }
    }

    // ── Click handlers ─────────────────────────────────────────────────

    private fun onScanClicked() {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA)
            != PackageManager.PERMISSION_GRANTED
        ) {
            statusOverlay.text = "Camera permission required"
            requestRuntimePermissions()
            return
        }
        scanLauncher.launch(Unit)
    }

    private fun onStartStopClicked() {
        when (StreamingController.state.value) {
            StreamingController.State.CONNECTED -> {
                lifecycleScope.launch {
                    StreamingController.startStreaming(this@MainActivity, this@MainActivity, previewView)
                }
            }
            StreamingController.State.STREAMING -> {
                StreamingController.stopStreaming(this)
            }
            else -> {
                Log.w(TAG, "startStop pressed in unexpected state ${StreamingController.state.value}")
            }
        }
    }

    private fun onDisconnectClicked() {
        StreamingController.disconnect(this)
    }
}
