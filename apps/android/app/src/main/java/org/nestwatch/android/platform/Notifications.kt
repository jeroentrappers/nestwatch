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
    /**
     * Escalating alarm (FR-AL-5): high importance, category alarm, no
     * channel sound or vibration (the listener service plays the alarm on
     * the alarm stream and vibrates itself).
     */
    const val CH_ALARM = "alert-alarm"

    /** Notification slots per station. */
    const val SLOT_SOUND = 0
    const val SLOT_CONNECTION = 1
    const val SLOT_BATTERY = 2
    const val SLOT_ALARM = 3

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
        val alarm = NotificationChannel(
            CH_ALARM, context.getString(R.string.channel_alarm), NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = context.getString(R.string.channel_alarm_desc)
            setSound(null, null)
            enableVibration(false)
            lockscreenVisibility = android.app.Notification.VISIBILITY_PUBLIC
        }
        nm.createNotificationChannels(listOf(service, sound, connection, battery, alarm))
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

    /** Notification ID of a station's alert slot; repeats replace each other. */
    fun alertId(stationId: String, slot: Int): Int = 1000 + (stationId.hashCode() and 0xffff) * 4 + slot

    /**
     * Opens the app; with a station, tapping also acknowledges that
     * station's sound alert or alarm (FR-AL-6, see MainActivity).
     */
    private fun openAndAcknowledge(context: Context, stationId: String, slot: Int): PendingIntent =
        PendingIntent.getActivity(
            context,
            alertId(stationId, slot),
            Intent(context, MainActivity::class.java)
                .setAction(AlertActionReceiver.ACTION_ACKNOWLEDGE)
                .putExtra(AlertActionReceiver.EXTRA_STATION, stationId)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

    private fun NotificationCompat.Builder.stationActions(context: Context, stationId: String) = this
        .addAction(
            0,
            context.getString(R.string.action_on_it),
            AlertActionReceiver.intent(context, AlertActionReceiver.ACTION_ACKNOWLEDGE, stationId),
        )
        .addAction(
            0,
            context.getString(R.string.action_not_baby),
            AlertActionReceiver.intent(context, AlertActionReceiver.ACTION_NOT_THE_BABY, stationId),
        )

    /**
     * Posts an alert; `id` groups repeats of the same station and kind. A
     * sound alert ([stationId] set) carries "I'm on it" and "Not the baby"
     * and acknowledges when tapped (FR-AL-6, FR-AL-7).
     */
    fun alert(context: Context, channel: String, id: Int, title: String, text: String, stationId: String? = null) {
        if (!canPost(context)) return
        val high = channel != CH_BATTERY
        val b = NotificationCompat.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(text)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setPriority(if (high) NotificationCompat.PRIORITY_HIGH else NotificationCompat.PRIORITY_DEFAULT)
            .setDefaults(NotificationCompat.DEFAULT_ALL)
        if (stationId != null) {
            b.setContentIntent(openAndAcknowledge(context, stationId, SLOT_SOUND)).stationActions(context, stationId)
        } else {
            b.setContentIntent(openApp(context))
        }
        notify(context, id, b.build())
    }

    /**
     * The alarm notification of a station (FR-AL-5): heads-up on its own
     * channel, category alarm, shown on the lock screen, with "I'm on it"
     * and "Not the baby". No full-screen intent: Google Play grants
     * USE_FULL_SCREEN_INTENT by default only to calling and alarm-clock
     * apps. Swiping it away does not acknowledge; it comes back while the
     * alarm rings.
     */
    fun alarm(context: Context, stationId: String, stationName: String) {
        if (!canPost(context)) return
        val n = NotificationCompat.Builder(context, CH_ALARM)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.alarm_notif_title, stationName))
            .setContentText(context.getString(R.string.alarm_notif_body))
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(false)
            .setContentIntent(openAndAcknowledge(context, stationId, SLOT_ALARM))
            .setDeleteIntent(AlertActionReceiver.intent(context, AlertActionReceiver.ACTION_DISMISSED, stationId))
            .stationActions(context, stationId)
            .build()
        notify(context, alertId(stationId, SLOT_ALARM), n)
    }

    fun cancelAlarm(context: Context, stationId: String) {
        NotificationManagerCompat.from(context).cancel(alertId(stationId, SLOT_ALARM))
    }

    /** Removes a station's sound alert and alarm (after an acknowledgement). */
    fun clearStation(context: Context, stationId: String) {
        val nm = NotificationManagerCompat.from(context)
        nm.cancel(alertId(stationId, SLOT_SOUND))
        nm.cancel(alertId(stationId, SLOT_ALARM))
    }

    private fun notify(context: Context, id: Int, n: android.app.Notification) {
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
