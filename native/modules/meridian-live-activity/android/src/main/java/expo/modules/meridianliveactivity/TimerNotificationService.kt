// Android live timer — the foreground-service notification half.
//
// Purpose: Android has no Live Activities; the platform equivalent for "a
// job timer is running" is an ongoing foreground-service notification. This
// service renders the SAME snapshot the iOS widget renders (one truth: the
// server-backed timer session), with a chronometer for the current segment
// and a Stop action that opens the app.
//
// Non-purpose: no timer logic. The JS side derives the snapshot and calls
// start/update/end; this service only draws it.

package expo.modules.meridianliveactivity

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

class TimerNotificationService : Service() {
    companion object {
        const val CHANNEL_ID = "meridian-live-timer"
        const val NOTIFICATION_ID = 7117
        const val ACTION_START = "tech.meridian.app.live_timer.START"
        const val ACTION_STOP = "tech.meridian.app.live_timer.STOP"

        const val EXTRA_SESSION_ID = "sessionId"
        const val EXTRA_TICKET_ID = "ticketId"
        const val EXTRA_TICKET_NUMBER = "ticketNumber"
        const val EXTRA_TITLE = "title"
        const val EXTRA_STATE = "state"
        const val EXTRA_SEGMENT_START = "segmentStartedAtMillis"
        const val EXTRA_ACCUMULATED_SECONDS = "accumulatedSeconds"

        fun intent(context: Context, payload: Map<String, Any?>): Intent {
            return Intent(context, TimerNotificationService::class.java).apply {
                action = ACTION_START
                putExtra(EXTRA_SESSION_ID, payload["sessionId"] as? String)
                putExtra(EXTRA_TICKET_ID, payload["ticketId"] as? String)
                putExtra(EXTRA_TICKET_NUMBER, payload["ticketNumber"] as? String)
                putExtra(EXTRA_TITLE, payload["title"] as? String)
                putExtra(EXTRA_STATE, payload["state"] as? String)
                putExtra(EXTRA_SEGMENT_START, (payload["segmentStartedAtMillis"] as? Number)?.toLong() ?: 0L)
                putExtra(
                    EXTRA_ACCUMULATED_SECONDS,
                    (payload["accumulatedSeconds"] as? Number)?.toDouble() ?: 0.0,
                )
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }
        val notification = buildNotification(intent)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun buildNotification(intent: Intent?): Notification {
        ensureChannel()
        val ticketNumber = intent?.getStringExtra(EXTRA_TICKET_NUMBER) ?: ""
        val title = intent?.getStringExtra(EXTRA_TITLE) ?: "Job timer"
        val state = intent?.getStringExtra(EXTRA_STATE) ?: "running"
        val segmentStart = intent?.getLongExtra(EXTRA_SEGMENT_START, 0L) ?: 0L
        val accumulated = intent?.getDoubleExtra(EXTRA_ACCUMULATED_SECONDS, 0.0) ?: 0.0
        val ticketId = intent?.getStringExtra(EXTRA_TICKET_ID)

        val openIntent = Intent(Intent.ACTION_VIEW).apply {
            data = android.net.Uri.parse(
                if (!ticketId.isNullOrEmpty()) "meridian://ticket/$ticketId" else "meridian://timer",
            )
            setPackage(packageName)
        }
        val openPending = PendingIntent.getActivity(
            this,
            0,
            openIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val stopPending = PendingIntent.getService(
            this,
            1,
            Intent(this, TimerNotificationService::class.java).apply { action = ACTION_STOP },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        val builder = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_menu_recent_history)
            .setContentTitle(title)
            .setContentText("$ticketNumber · ${if (state == "running") "running" else "paused"}")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(openPending)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .addAction(0, "Stop", stopPending)

        if (state == "running") {
            builder.setUsesChronometer(true)
            builder.setWhen(if (segmentStart > 0) segmentStart else System.currentTimeMillis())
        }
        if (accumulated >= 1) {
            builder.setSubText("Already banked ${formatClock(accumulated)}")
        }
        return builder.build()
    }

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (manager.getNotificationChannel(CHANNEL_ID) != null) return
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Running job timer",
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = "Shows the job timer that is running right now."
            setShowBadge(false)
        }
        manager.createNotificationChannel(channel)
    }

    private fun formatClock(seconds: Double): String {
        val total = seconds.toInt().coerceAtLeast(0)
        val hours = total / 3600
        val minutes = (total % 3600) / 60
        val secs = total % 60
        return if (hours > 0) {
            String.format("%d:%02d:%02d", hours, minutes, secs)
        } else {
            String.format("%02d:%02d", minutes, secs)
        }
    }
}
