package com.herdscout.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.lifecycle.LifecycleService

/**
 * Foreground service that keeps the camera + iroh stream alive while the
 * screen is off. The drone use case = phone strapped to a frame, locked, in
 * the air; the OS would otherwise kill the camera session within ~30s.
 *
 * The service does not own the JNI handle directly — that lives in the
 * [StreamingController] (process-singleton). The service exists to keep the
 * process from being backgrounded into oblivion. Telling Android we have a
 * `foregroundServiceType="camera"` is the magic incantation.
 */
class StreamingService : LifecycleService() {

    companion object {
        private const val TAG = "HerdScoutSvc"
        private const val CHANNEL_ID = "herd_scout_streaming"
        private const val NOTIFICATION_ID = 1

        const val ACTION_START = "com.herdscout.app.action.START"
        const val ACTION_STOP = "com.herdscout.app.action.STOP"

        fun start(context: Context) {
            val intent = Intent(context, StreamingService::class.java).apply {
                action = ACTION_START
            }
            // startForegroundService is required on O+; the service itself
            // must call startForeground() within 5s.
            context.startForegroundService(intent)
        }

        fun stop(context: Context) {
            val intent = Intent(context, StreamingService::class.java).apply {
                action = ACTION_STOP
            }
            context.startService(intent)
        }
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        when (intent?.action) {
            ACTION_START -> {
                Log.i(TAG, "Foreground service starting")
                val notification = buildNotification("Streaming to herd-scout")
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    startForeground(
                        NOTIFICATION_ID,
                        notification,
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA,
                    )
                } else {
                    startForeground(NOTIFICATION_ID, notification)
                }
            }
            ACTION_STOP -> {
                Log.i(TAG, "Foreground service stopping")
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent): IBinder? {
        super.onBind(intent)
        return null
    }

    private fun createNotificationChannel() {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Streaming",
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = "Active herd-scout video stream"
            setShowBadge(false)
        }
        nm.createNotificationChannel(channel)
    }

    private fun buildNotification(text: String): Notification {
        val openIntent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val pi = PendingIntent.getActivity(
            this, 0, openIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("herd-scout")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_menu_camera)
            .setOngoing(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(pi)
            .build()
    }
}
