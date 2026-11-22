// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.platform

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import org.nestwatch.android.R
import org.nestwatch.android.core.Nestwatch
import org.nestwatch.ffi.AlarmState

/**
 * "I'm on it" and "Not the baby" (product.md FR-AL-6, FR-AL-7), from the
 * in-app banner, the notification actions and the notification tap. Each
 * goes to the listener engine and removes the station's sound alert and
 * alarm notifications; the alarm sound stops when the engine reports the
 * station no longer ringing.
 */
object AlertActions {
    fun acknowledge(context: Context, stationId: String) {
        Nestwatch.withListener { it.acknowledge(stationId) }
        Notifications.clearStation(context, stationId)
    }

    fun notTheBaby(context: Context, stationId: String) {
        Nestwatch.withListener { it.notTheBaby(stationId) }
        Notifications.clearStation(context, stationId)
    }

    fun resume(stationId: String) {
        Nestwatch.withListener { it.resumeAlerts(stationId) }
    }
}

/** Receives the notification actions (not exported; explicit intents only). */
class AlertActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getStringExtra(EXTRA_STATION) ?: return
        when (intent.action) {
            ACTION_ACKNOWLEDGE -> AlertActions.acknowledge(context, id)
            ACTION_NOT_THE_BABY -> AlertActions.notTheBaby(context, id)
            ACTION_DISMISSED -> {
                // A swipe is not an answer: show the alarm again while it rings.
                val t = Nestwatch.listenerState.value?.stations?.firstOrNull { it.id == id } ?: return
                if (t.alarm is AlarmState.Ringing) Notifications.alarm(context, id, t.name.ifBlank { context.getString(R.string.unnamed) })
            }
        }
    }

    companion object {
        const val ACTION_ACKNOWLEDGE = "org.nestwatch.android.action.ACKNOWLEDGE"
        const val ACTION_NOT_THE_BABY = "org.nestwatch.android.action.NOT_THE_BABY"
        const val ACTION_DISMISSED = "org.nestwatch.android.action.ALARM_DISMISSED"
        const val EXTRA_STATION = "station"

        fun intent(context: Context, action: String, stationId: String): PendingIntent = PendingIntent.getBroadcast(
            context,
            (action + stationId).hashCode(),
            Intent(context, AlertActionReceiver::class.java).setAction(action).putExtra(EXTRA_STATION, stationId),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }
}
