package com.herdscout.app

import android.Manifest
import android.content.Context
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.View
import android.widget.Button
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import com.herdscout.shared.QrScanActivity
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
 *
 * Wave 9: a small top-right indicator shows which physical edge of the
 * phone the user has chosen to be the "top" of the captured video, with
 * a gear icon that opens an AlertDialog picker. The arrow on the
 * indicator rotates to point toward the chosen edge of the device. This
 * replaces Wave 8's "rotate to landscape" banner-nag — the phone no
 * longer needs to be physically rotated; the user just declares which
 * edge is up and CameraX's targetRotation is set accordingly.
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
    private lateinit var topEdgeArrow: ImageView
    private lateinit var topEdgeLabel: TextView
    private lateinit var topEdgeSettings: ImageButton
    private lateinit var prefs: SharedPreferences

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

        prefs = getSharedPreferences(TopEdge.PREFS_NAME, Context.MODE_PRIVATE)

        scanButton = findViewById(R.id.scanButton)
        startStopButton = findViewById(R.id.startStopButton)
        disconnectButton = findViewById(R.id.disconnectButton)
        statusOverlay = findViewById(R.id.statusOverlay)
        previewView = findViewById(R.id.cameraPreview)
        topEdgeArrow = findViewById(R.id.topEdgeArrow)
        topEdgeLabel = findViewById(R.id.topEdgeLabel)
        topEdgeSettings = findViewById(R.id.topEdgeSettings)

        scanButton.setOnClickListener { onScanClicked() }
        startStopButton.setOnClickListener { onStartStopClicked() }
        disconnectButton.setOnClickListener { onDisconnectClicked() }
        topEdgeSettings.setOnClickListener { showTopEdgePicker() }

        requestRuntimePermissions()
        wireStateObservers()
        refreshButtons()
        updateOrientationIndicator()
    }

    /**
     * Wave 9: reflect the saved [TopEdge] in the corner indicator. The
     * arrow drawable points up at 0deg, so we just rotate it by
     * [TopEdge.arrowRotationDeg] (0 / 90 / 180 / 270) and set the label.
     *
     * No physical-rotation listener: the indicator is part of the
     * activity's view hierarchy so Android's natural orientation
     * handling rotates it along with the rest of the UI, and the
     * arrow's rotation relative to the phone's edges is preserved by
     * that. The only times this needs to re-run are onCreate and after
     * the user picks a new edge in the dialog.
     */
    private fun updateOrientationIndicator() {
        val edge = TopEdge.fromPrefs(prefs)
        topEdgeArrow.rotation = edge.arrowRotationDeg
        topEdgeLabel.text = getString(R.string.top_edge_label_format, edge.label.uppercase())
    }

    /**
     * Wave 9: show a 4-option radio dialog letting the user pick which
     * edge of the phone is the "top" of the captured video. Persists to
     * SharedPreferences and updates the indicator immediately. If the
     * user changes the edge while a stream is active, we toast that a
     * restart is needed — CameraX's targetRotation is read at
     * bind-time, and re-binding mid-stream is risky (the JNI side
     * holds an active publish). Simpler to ask the user to Stop + Start.
     */
    private fun showTopEdgePicker() {
        val current = TopEdge.fromPrefs(prefs)
        val options = arrayOf(TopEdge.TOP, TopEdge.RIGHT, TopEdge.BOTTOM, TopEdge.LEFT)
        val labels = arrayOf(
            getString(R.string.top_edge_option_top),
            getString(R.string.top_edge_option_right),
            getString(R.string.top_edge_option_bottom),
            getString(R.string.top_edge_option_left),
        )
        val checkedIndex = options.indexOf(current).coerceAtLeast(0)
        var pendingIndex = checkedIndex

        AlertDialog.Builder(this)
            .setTitle(R.string.top_edge_dialog_title)
            .setSingleChoiceItems(labels, checkedIndex) { _, which -> pendingIndex = which }
            .setPositiveButton(android.R.string.ok) { dialog, _ ->
                val chosen = options[pendingIndex]
                if (chosen != current) {
                    TopEdge.save(prefs, chosen)
                    updateOrientationIndicator()
                    if (StreamingController.state.value ==
                        StreamingController.State.STREAMING) {
                        Toast.makeText(
                            this,
                            R.string.top_edge_restart_toast,
                            Toast.LENGTH_LONG,
                        ).show()
                    }
                }
                dialog.dismiss()
            }
            .setNegativeButton(android.R.string.cancel) { dialog, _ -> dialog.dismiss() }
            .show()
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
