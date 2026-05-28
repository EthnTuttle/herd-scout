package com.herdscout.shared

import java.util.concurrent.TimeUnit

/**
 * Display helpers for iroh NodeIds and timestamps.
 *
 * iroh `EndpointId`s are 64 lowercase-hex chars — too long for a list
 * row. The convention everywhere in the herd-scout UI is "first 8 +
 * `…` + last 4," matching how `EndpointId::fmt_short` renders on the
 * Rust side.
 */
object NodeIdFormat {
    fun short(nodeId: String): String {
        val trimmed = nodeId.trim()
        if (trimmed.length <= 12) return trimmed
        return trimmed.take(8) + "…" + trimmed.takeLast(4)
    }

    /**
     * "23s ago", "3m ago", "4h ago", "2d ago" — short relative
     * timestamp. Returns "—" for `tsMs <= 0` (used as a "no data"
     * sentinel by `StatusReply.last_reload_unix_ms`).
     */
    fun relative(tsMs: Long, nowMs: Long = System.currentTimeMillis()): String {
        if (tsMs <= 0) return "—"
        val deltaMs = (nowMs - tsMs).coerceAtLeast(0)
        val sec = TimeUnit.MILLISECONDS.toSeconds(deltaMs)
        val min = TimeUnit.MILLISECONDS.toMinutes(deltaMs)
        val hr = TimeUnit.MILLISECONDS.toHours(deltaMs)
        val day = TimeUnit.MILLISECONDS.toDays(deltaMs)
        return when {
            sec < 60 -> "${sec}s ago"
            min < 60 -> "${min}m ago"
            hr < 24 -> "${hr}h ago"
            else -> "${day}d ago"
        }
    }
}
