// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.platform

import android.annotation.SuppressLint
import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.os.PowerManager

/**
 * A partial wake lock and a Wi-Fi lock held while a station or listener is
 * active (platforms.md §2.2), so audio keeps flowing with the screen off.
 */
class KeepAwake(context: Context, tag: String) {
    private val wake = (context.getSystemService(Context.POWER_SERVICE) as PowerManager)
        .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "nestwatch:$tag")
        .apply { setReferenceCounted(false) }

    @Suppress("DEPRECATION")
    private val wifi = (context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager?)
        ?.createWifiLock(
            if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY
            else WifiManager.WIFI_MODE_FULL_HIGH_PERF,
            "nestwatch:$tag",
        )?.apply { setReferenceCounted(false) }

    // Held for as long as monitoring runs; released in release().
    @SuppressLint("WakelockTimeout")
    fun acquire() {
        if (!wake.isHeld) wake.acquire()
        wifi?.let { if (!it.isHeld) it.acquire() }
    }

    fun release() {
        if (wake.isHeld) wake.release()
        wifi?.let { if (it.isHeld) it.release() }
    }
}

/** A multicast lock held while browsing NSD (platforms.md §2.2). */
class Multicast(context: Context) {
    private val lock = (context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager?)
        ?.createMulticastLock("nestwatch:nsd")?.apply { setReferenceCounted(false) }

    fun acquire() {
        lock?.let { if (!it.isHeld) it.acquire() }
    }

    fun release() {
        lock?.let { if (it.isHeld) it.release() }
    }
}

/** Battery optimisation exemption (platforms.md §2.2). */
object BatteryOptimisation {
    fun isExempt(context: Context): Boolean =
        (context.getSystemService(Context.POWER_SERVICE) as PowerManager)
            .isIgnoringBatteryOptimizations(context.packageName)
}
