// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.ui

import android.content.Context
import android.content.res.Resources
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import java.text.SimpleDateFormat
import java.time.LocalDate
import java.time.ZoneId
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.delay
import org.nestwatch.android.R
import org.nestwatch.android.core.hoursMinutes

/** "4 min", "1 h 20 min", "2 h"; under a minute: "under 1 min". */
fun formatDuration(res: Resources, ms: Long): String {
    if (ms < 60_000) return res.getString(R.string.duration_under_minute)
    val (h, m) = hoursMinutes(ms)
    val mins = res.getQuantityString(R.plurals.duration_minutes, m, m)
    if (h == 0) return mins
    val hours = res.getQuantityString(R.plurals.duration_hours, h, h)
    return if (m == 0) hours else res.getString(R.string.duration_hours_minutes, hours, mins)
}

/** Short duration label of a choice in seconds: "30 s", "1 min", "2 min". */
fun formatChoice(res: Resources, seconds: Int): String =
    if (seconds < 60) res.getQuantityString(R.plurals.duration_seconds_short, seconds, seconds)
    else (seconds / 60).let { res.getQuantityString(R.plurals.duration_minutes, it, it) }

/** Time of day in the user's 12/24 h setting and locale ("03:40"). */
fun formatTime(context: Context, unixMs: Long): String =
    android.text.format.DateFormat.getTimeFormat(context).format(Date(unixMs))

/** A short date with weekday in the locale's order ("Sat 21 Nov"). */
fun formatNightDate(locale: Locale, date: LocalDate, zone: ZoneId = ZoneId.systemDefault()): String {
    val pattern = android.text.format.DateFormat.getBestDateTimePattern(locale, "EEEMMMd")
    val ms = date.atStartOfDay(zone).toInstant().toEpochMilli()
    return SimpleDateFormat(pattern, locale).format(Date(ms))
}

/** "Awake for 4 min" / "Quiet for 1 h 20 min" (FR-SL-3). */
fun roomStatusText(res: Resources, awake: Boolean, forMs: Long): String = when {
    forMs < 60_000 -> res.getString(if (awake) R.string.room_awake_brief else R.string.room_quiet_brief)
    awake -> res.getString(R.string.room_awake_for, formatDuration(res, forMs))
    else -> res.getString(R.string.room_quiet_for, formatDuration(res, forMs))
}

/** The wall clock, refreshed every [periodMs] on the period boundary. */
@Composable
fun rememberNow(periodMs: Long = 15_000): Long {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(periodMs) {
        while (true) {
            val t = System.currentTimeMillis()
            delay(periodMs - t % periodMs + 5)
            now = System.currentTimeMillis()
        }
    }
    return now
}
