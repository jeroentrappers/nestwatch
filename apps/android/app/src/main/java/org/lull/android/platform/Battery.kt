// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android.platform

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import androidx.core.content.ContextCompat

/**
 * Battery level and charging state from `ACTION_BATTERY_CHANGED` for the
 * station's `status` (FR-ST-6). `charging` means on external power, or the
 * battery reports charging or full (PLATFORM-8).
 */
class BatteryWatch(private val context: Context, private val onChange: (Int?, Boolean?) -> Unit) {
    private var last: Pair<Int?, Boolean?>? = null

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context?, intent: Intent?) {
            intent?.let { report(it) }
        }
    }

    private fun report(intent: Intent) {
        val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
        val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
        val present = intent.getBooleanExtra(BatteryManager.EXTRA_PRESENT, true)
        val percent = if (present && level >= 0 && scale > 0) (level * 100 / scale).coerceIn(0, 100) else null
        val plugged = intent.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0
        val status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, BatteryManager.BATTERY_STATUS_UNKNOWN)
        val charging = when {
            !present -> null
            plugged -> true
            status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL -> true
            status == BatteryManager.BATTERY_STATUS_UNKNOWN -> null
            else -> false
        }
        val now = percent to charging
        if (now != last) {
            last = now
            onChange(percent, charging)
        }
    }

    fun start() {
        // A sticky broadcast: the current state is returned at once.
        ContextCompat.registerReceiver(
            context,
            receiver,
            IntentFilter(Intent.ACTION_BATTERY_CHANGED),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )?.let { report(it) }
    }

    fun stop() {
        runCatching { context.unregisterReceiver(receiver) }
    }
}
