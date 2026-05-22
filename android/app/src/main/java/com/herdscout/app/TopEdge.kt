package com.herdscout.app

import android.content.SharedPreferences
import android.view.Surface

/**
 * Which physical edge of the phone the user wants to be the "top" of the
 * captured video. Wave 9 replaces Wave 8's "rotate to landscape" nag with
 * a user-configurable mapping: instead of telling the user how to hold
 * the phone, we let them tell us which edge of the device should appear
 * at the top of the desktop's video feed.
 *
 * The default is [RIGHT] because most phone rear cameras have their
 * sensor's "natural" landscape orientation matching `Surface.ROTATION_90`
 * (i.e. holding the phone upright in portrait, the right edge of the
 * device becomes the top of the encoded landscape frame). This matches
 * Wave 7's hardcoded behaviour so existing users see no change unless
 * they pick another option.
 *
 * The values plug into two places:
 *  - [rotation] feeds CameraX `setTargetRotation(...)` on the Preview
 *    and ImageAnalysis builders in [StreamingController.startStreaming].
 *  - [arrowRotationDeg] rotates the on-screen arrow ImageView so it
 *    physically points toward the chosen edge of the device. The arrow
 *    drawable points up at 0deg, so [TOP] = 0, [RIGHT] = 90, etc.
 */
enum class TopEdge(
    val rotation: Int,
    val arrowRotationDeg: Float,
    val label: String,
) {
    TOP(Surface.ROTATION_0, 0f, "Top"),
    RIGHT(Surface.ROTATION_90, 90f, "Right"),
    BOTTOM(Surface.ROTATION_180, 180f, "Bottom"),
    LEFT(Surface.ROTATION_270, 270f, "Left");

    companion object {
        const val PREFS_NAME = "herd-scout-prefs"
        const val PREF_KEY = "top_edge"
        const val DEFAULT_VALUE = "right"

        /** Read the user's saved choice, defaulting to [RIGHT]. */
        fun fromPrefs(prefs: SharedPreferences): TopEdge =
            when (prefs.getString(PREF_KEY, DEFAULT_VALUE)) {
                "top" -> TOP
                "bottom" -> BOTTOM
                "left" -> LEFT
                else -> RIGHT
            }

        /** Persist [edge] to [prefs] under [PREF_KEY]. */
        fun save(prefs: SharedPreferences, edge: TopEdge) {
            prefs.edit().putString(PREF_KEY, edge.name.lowercase()).apply()
        }
    }
}
