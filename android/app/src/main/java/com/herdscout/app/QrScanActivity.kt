package com.herdscout.app

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.util.Log
import androidx.activity.result.contract.ActivityResultContract
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.core.CameraSelector
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.lifecycle.ProcessCameraProvider
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
 */
class QrScanActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "QrScanActivity"
        const val EXTRA_RESULT = "qr_result"

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
    private lateinit var cameraController: LifecycleCameraController
    private lateinit var scanner: BarcodeScanner
    @Volatile private var done: Boolean = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_qr_scan)

        previewView = findViewById(R.id.qrPreviewView)

        val options = BarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .build()
        scanner = BarcodeScanning.getClient(options)

        cameraController = LifecycleCameraController(this).apply {
            cameraSelector = CameraSelector.DEFAULT_BACK_CAMERA
            bindToLifecycle(this@QrScanActivity)
        }
        previewView.controller = cameraController

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
                    val data = Intent().putExtra(EXTRA_RESULT, raw)
                    setResult(Activity.RESULT_OK, data)
                    finish()
                }
            },
        )
    }

    override fun onDestroy() {
        super.onDestroy()
        if (this::scanner.isInitialized) scanner.close()
    }
}
