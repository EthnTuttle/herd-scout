package com.herdscout.shared

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.widget.Button
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContract
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.core.CameraSelector
import androidx.camera.mlkit.vision.MlKitAnalyzer
import androidx.camera.view.CameraController.COORDINATE_SYSTEM_VIEW_REFERENCED
import androidx.camera.view.LifecycleCameraController
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import com.google.mlkit.vision.barcode.BarcodeScanner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode

/**
 * Tiny full-screen activity that opens the back camera, runs the ML Kit
 * barcode analyzer over each frame, and finishes with the first decoded
 * QR string as the result.
 *
 * Uses [LifecycleCameraController] (CameraX's high-level wrapper) so we
 * don't have to plumb our own ImageAnalysis pipeline — easier to read, and
 * the decode rate doesn't matter here. The final streaming path uses the
 * lower-level [androidx.camera.core.ImageAnalysis] in [StreamingController].
 *
 * Wave 5A polish:
 *   * A [QrViewfinderOverlay] reticle is drawn over the preview to tell
 *     the user where to aim. ML Kit still scans the full frame; the
 *     reticle is purely a visual aid.
 *   * After 4 s without a successful decode the top hint switches to a
 *     low-light tip and the torch button (if the device supports it) is
 *     surfaced. Tapping the torch toggles the rear flash.
 */
class QrScanActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "QrScanActivity"
        const val EXTRA_RESULT = "qr_result"
        /** How long we wait without a decode before showing the low-light tip. */
        private const val LOW_LIGHT_HINT_DELAY_MS = 4_000L

        /** ActivityResultContract that launches scanning and returns the decoded string. */
        class Contract : ActivityResultContract<Unit, String?>() {
            override fun createIntent(context: android.content.Context, input: Unit): Intent =
                Intent(context, QrScanActivity::class.java)

            override fun parseResult(resultCode: Int, intent: Intent?): String? {
                if (resultCode != Activity.RESULT_OK) return null
                return intent?.getStringExtra(EXTRA_RESULT)
            }
        }
    }

    private lateinit var previewView: PreviewView
    private lateinit var hintView: TextView
    private lateinit var torchButton: Button
    private lateinit var cameraController: LifecycleCameraController
    private lateinit var scanner: BarcodeScanner
    @Volatile private var done: Boolean = false
    private var torchOn: Boolean = false

    private val mainHandler = Handler(Looper.getMainLooper())
    private val showLowLightHint = Runnable {
        if (done) return@Runnable
        hintView.setText(R.string.qr_hint_low_light)
        if (cameraController.cameraInfo?.hasFlashUnit() == true) {
            torchButton.visibility = android.view.View.VISIBLE
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_qr_scan)

        previewView = findViewById(R.id.qrPreviewView)
        hintView = findViewById(R.id.qrHint)
        torchButton = findViewById(R.id.qrTorchButton)

        val options = BarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .build()
        scanner = BarcodeScanning.getClient(options)

        cameraController = LifecycleCameraController(this).apply {
            cameraSelector = CameraSelector.DEFAULT_BACK_CAMERA
            bindToLifecycle(this@QrScanActivity)
        }
        previewView.controller = cameraController

        torchButton.setOnClickListener { onTorchToggled() }

        val executor = ContextCompat.getMainExecutor(this)
        cameraController.setImageAnalysisAnalyzer(
            executor,
            MlKitAnalyzer(
                listOf(scanner),
                COORDINATE_SYSTEM_VIEW_REFERENCED,
                executor,
            ) { result ->
                if (done) return@MlKitAnalyzer
                val barcodes = result.getValue(scanner) ?: return@MlKitAnalyzer
                val raw = barcodes.firstOrNull { !it.rawValue.isNullOrBlank() }?.rawValue
                if (!raw.isNullOrBlank()) {
                    Log.i(TAG, "QR decoded (${raw.length} chars)")
                    done = true
                    mainHandler.removeCallbacks(showLowLightHint)
                    val data = Intent().putExtra(EXTRA_RESULT, raw)
                    setResult(Activity.RESULT_OK, data)
                    finish()
                }
            },
        )

        // Schedule the low-light hint. Cancelled on a successful scan.
        mainHandler.postDelayed(showLowLightHint, LOW_LIGHT_HINT_DELAY_MS)
    }

    /**
     * Toggle the rear-camera flash. Hidden by default and only shown
     * after [LOW_LIGHT_HINT_DELAY_MS] of unsuccessful scanning.
     */
    private fun onTorchToggled() {
        if (cameraController.cameraInfo?.hasFlashUnit() != true) return
        torchOn = !torchOn
        cameraController.enableTorch(torchOn)
        torchButton.setText(if (torchOn) R.string.qr_torch_off else R.string.qr_torch_on)
    }

    override fun onDestroy() {
        super.onDestroy()
        mainHandler.removeCallbacks(showLowLightHint)
        if (this::scanner.isInitialized) scanner.close()
    }
}
