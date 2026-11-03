// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.platform

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import org.nestwatch.android.MainActivity
import org.nestwatch.android.R

/** Notification channels and builders (product.md §3.5, platforms.md §2.2). */
object Notifications {
    /** Ongoing station / listener service notification. */
    const val CH_SERVICE = "service"
    /** Sound detected (FR-AL-1): high importance, sound and vibration. */
    const val CH_SOUND = "alert-sound"
    /** Unreachable / reachable again (FR-AL-2): high importance. */
    const val CH_CONNECTION = "alert-connection"
    /** Low battery (FR-AL-3). */
    const val CH_BATTERY = "alert-battery"

    const val ID_STATION = 1
    const val ID_LISTENER = 2

    fun createChannels(context: Context) {
        if (Build.VERSION.SDK_INT < 26) return
        val nm = context.getSystemService(NotificationManager::class.java)
        val service = NotificationChannel(
            CH_SERVICE, context.getString(R.string.channel_service), NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = context.getString(R.string.channel_service_desc)
            setShowBadge(false)
        }
        val sound = NotificationChannel(
            CH_SOUND, context.getString(R.string.channel_sound), NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = context.getString(R.string.channel_sound_desc)
            enableVibration(true)
            vibrationPattern = longArrayOf(0, 300, 200, 300)
            setBypassDnd(false)
        }
        val connection = NotificationChannel(
            CH_CONNECTION, context.getString(R.string.channel_connection), NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = context.getString(R.string.channel_connection_desc)
            enableVibration(true)
        }
        val battery = NotificationChannel(
            CH_BATTERY, context.getString(R.string.channel_battery), NotificationManager.IMPORTANCE_DEFAULT,
        ).apply { description = context.getString(R.string.channel_battery_desc) }
        nm.createNotificationChannels(listOf(service, sound, connection, battery))
    }

    fun openApp(context: Context): PendingIntent = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

    fun ongoing(context: Context, title: String, text: String) =
        NotificationCompat.Builder(context, CH_SERVICE)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(openApp(context))
            .build()

    fun canPost(context: Context): Boolean =
        Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED

    /** Posts an alert; `id` groups repeats of the same station and kind. */
    fun alert(context: Context, channel: String, id: Int, title: String, text: String) {
        if (!canPost(context)) return
        val high = channel != CH_BATTERY
        val n = NotificationCompat.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(text)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setPriority(if (high) NotificationCompat.PRIORITY_HIGH else NotificationCompat.PRIORITY_DEFAULT)
            .setDefaults(NotificationCompat.DEFAULT_ALL)
            .setContentIntent(openApp(context))
            .build()
        try {
            NotificationManagerCompat.from(context).notify(id, n)
        } catch (_: SecurityException) {
            // Permission revoked meanwhile; the in-app notice remains.
        }
    }

    fun update(context: Context, id: Int, n: android.app.Notification) {
        if (!canPost(context)) return
        try {
            NotificationManagerCompat.from(context).notify(id, n)
        } catch (_: SecurityException) {
        }
    }
}
