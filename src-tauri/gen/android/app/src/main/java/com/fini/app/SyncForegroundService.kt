package com.fini.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat

/**
 * Keeps Fini's sync/BLE work alive while the app is in the background.
 *
 * Without this the process is simply not running: Android throttles a
 * backgrounded WebView's timers and will freeze or kill the process outright.
 * Observed directly on a Pixel 6 Pro -- backgrounding the app during a
 * Bluetooth session silenced it completely (not one log line from our process
 * in a full logcat capture) and the peer's session died of missed pings.
 *
 * This is the same shape wearable companion apps use (a fitness band showing a
 * persistent "connected" entry in the shade): a foreground service with type
 * `connectedDevice`, which is precisely the category Android defines for
 * maintaining a link to an external device.
 *
 * The notification is not decoration -- a foreground service must post one, and
 * Android shows it for as long as the service runs. It doubles as the honest
 * disclosure that Fini is holding a device connection, and as the user's way
 * to notice and stop it.
 *
 * Started/stopped from Rust over the plain-JNI bridge in
 * `utils::android_context`, so the entry points are `@JvmStatic` methods
 * taking a `Context`, matching that bridge's `(Landroid/content/Context;)V`
 * signature.
 */
class SyncForegroundService : Service() {

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        ensureChannel(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val status = intent?.getStringExtra(EXTRA_STATUS) ?: DEFAULT_STATUS
        val notification = buildNotification(this, status)

        // The typed overload is required from Android 10 (API 29) on, and from
        // Android 14 the type must also be backed by a matching
        // FOREGROUND_SERVICE_* permission in the manifest.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }

        // START_STICKY: if Android reclaims the process under memory pressure,
        // restart the service. A sync daemon that silently stays dead after one
        // low-memory event is the failure this whole class exists to prevent.
        return START_STICKY
    }

    companion object {
        private const val TAG = "SyncForegroundService"
        private const val CHANNEL_ID = "fini.sync"
        private const val NOTIFICATION_ID = 4211
        private const val EXTRA_STATUS = "status"
        private const val DEFAULT_STATUS = "Syncing with your devices"

        /// Set when Android refused `start`; see `startIfRefused`.
        @Volatile
        private var startRefused = false

        @JvmStatic
        fun start(context: Context) {
            val intent = Intent(context, SyncForegroundService::class.java)
            // startForegroundService is mandatory from Android 8 when the app
            // is not already in the foreground; it obliges the service to call
            // startForeground within ~5s, which onStartCommand does above.
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    context.startForegroundService(intent)
                } else {
                    context.startService(intent)
                }
                startRefused = false
            } catch (error: Exception) {
                // From Android 12, an app in the background may not start a
                // foreground service (ForegroundServiceStartNotAllowedException,
                // an IllegalStateException). Android can also refuse with
                // another exception, for example a SecurityException, so each
                // exception counts as a refusal. Rust asks for the service once,
                // so a refusal used to be final: on a Pixel 6 Pro, Fini sent to
                // the background before its first start was frozen about 1 min
                // later, its advertisement stayed on, and centrals that
                // connected got no answer to their GATT reads.
                // The Activity tries again when it comes to the foreground.
                startRefused = true
                Log.w(TAG, "foreground service start refused; trying again when the app is in the foreground", error)
            }
        }

        /// Starts the service again if Android refused the last `start`.
        /// For `MainActivity.onResume`: an app with a visible Activity may
        /// start a foreground service.
        @JvmStatic
        fun startIfRefused(context: Context) {
            if (startRefused) {
                start(context.applicationContext)
            }
        }

        @JvmStatic
        fun stop(context: Context) {
            context.stopService(Intent(context, SyncForegroundService::class.java))
        }

        /**
         * Updates the shade text in place. Posting the *same* notification id
         * on an already-running service replaces its content without
         * restarting the service or re-showing the notification.
         */
        @JvmStatic
        fun updateStatus(context: Context, status: String) {
            ensureChannel(context)
            val manager = context.getSystemService(NotificationManager::class.java) ?: return
            manager.notify(NOTIFICATION_ID, buildNotification(context, status))
        }

        private fun ensureChannel(context: Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val manager = context.getSystemService(NotificationManager::class.java) ?: return
            if (manager.getNotificationChannel(CHANNEL_ID) != null) return
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Device sync",
                // LOW, not DEFAULT: this notification is permanent, so it must
                // never make a sound or peek. It is a status line, not an alert.
                NotificationManager.IMPORTANCE_LOW,
            )
            channel.description = "Shows while Fini is keeping a connection to your other devices."
            channel.setShowBadge(false)
            manager.createNotificationChannel(channel)
        }

        private fun buildNotification(context: Context, status: String): Notification {
            val openApp = Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            }
            val contentIntent = PendingIntent.getActivity(
                context,
                0,
                openApp,
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )

            return NotificationCompat.Builder(context, CHANNEL_ID)
                .setContentTitle("Fini")
                .setContentText(status)
                .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
                .setOngoing(true)
                .setShowWhen(false)
                .setContentIntent(contentIntent)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .setCategory(NotificationCompat.CATEGORY_SERVICE)
                .build()
        }
    }
}
