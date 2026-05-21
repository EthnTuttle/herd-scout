package com.herdscout.app

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffXfermode
import android.graphics.RectF
import android.util.AttributeSet
import android.view.View
import kotlin.math.min

/**
 * Wave 5A: scanning viewfinder.
 *
 * Draws a centered square reticle on top of the camera preview with four
 * corner brackets and a dimmed area outside the reticle. Pure static
 * decoration: no measurement of detected barcodes here — that lives in
 * [QrScanActivity]. The reticle does not constrain ML Kit's analyzer (it
 * still scans the whole frame) but it tells the user *where* to point the
 * phone, which dramatically improves first-try scan rates against a
 * desktop monitor.
 *
 * The reticle is sized as 70% of the shorter screen edge so it stays
 * roughly square on both portrait and landscape orientations and on any
 * phone aspect ratio.
 */
class QrViewfinderOverlay @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0,
) : View(context, attrs, defStyleAttr) {

    private val dimPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.BLACK
        alpha = 110 // ~43% black overscreen
    }

    /**
     * "Cut a hole" paint for the reticle interior. We draw the dim layer
     * over the whole canvas, then use SRC_OUT to punch through to the
     * camera preview underneath.
     */
    private val cutoutPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.TRANSPARENT
        xfermode = PorterDuffXfermode(PorterDuff.Mode.CLEAR)
    }

    private val cornerPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.WHITE
        style = Paint.Style.STROKE
        strokeWidth = 6f
        strokeCap = Paint.Cap.ROUND
    }

    init {
        // setLayerType is required for PorterDuff CLEAR to compose
        // correctly across the View hierarchy on hardware acceleration.
        setLayerType(LAYER_TYPE_HARDWARE, null)
        // Pass touches through to the underlying preview / activity.
        isClickable = false
        isFocusable = false
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val w = width.toFloat()
        val h = height.toFloat()
        val short = min(w, h)
        val side = short * 0.70f
        val cx = w / 2f
        val cy = h / 2f
        val rect = RectF(cx - side / 2f, cy - side / 2f, cx + side / 2f, cy + side / 2f)

        // Dim outside the reticle.
        canvas.drawRect(0f, 0f, w, h, dimPaint)
        // Cut a clear hole for the reticle interior.
        canvas.drawRect(rect, cutoutPaint)

        // Four corner brackets — 12% of the reticle side each.
        val arm = side * 0.12f
        // Top-left
        canvas.drawLine(rect.left, rect.top, rect.left + arm, rect.top, cornerPaint)
        canvas.drawLine(rect.left, rect.top, rect.left, rect.top + arm, cornerPaint)
        // Top-right
        canvas.drawLine(rect.right - arm, rect.top, rect.right, rect.top, cornerPaint)
        canvas.drawLine(rect.right, rect.top, rect.right, rect.top + arm, cornerPaint)
        // Bottom-left
        canvas.drawLine(rect.left, rect.bottom - arm, rect.left, rect.bottom, cornerPaint)
        canvas.drawLine(rect.left, rect.bottom, rect.left + arm, rect.bottom, cornerPaint)
        // Bottom-right
        canvas.drawLine(rect.right - arm, rect.bottom, rect.right, rect.bottom, cornerPaint)
        canvas.drawLine(rect.right, rect.bottom - arm, rect.right, rect.bottom, cornerPaint)
    }
}
