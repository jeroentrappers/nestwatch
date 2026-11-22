// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.core

import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId
import org.nestwatch.ffi.WakeUpEntry

/** One wake-up of the night log in app terms (product.md FR-SL-4). */
data class LogEntry(
    val startMs: Long,
    /** `null` while the wake-up goes on. */
    val endMs: Long?,
    val alarmed: Boolean,
    val notTheBaby: Boolean,
) {
    /** Duration so far (ongoing: until `nowMs`). */
    fun durationMs(nowMs: Long): Long = ((endMs ?: nowMs) - startMs).coerceAtLeast(0)
}

fun WakeUpEntry.toLogEntry() = LogEntry(
    startMs = startUnixMs.toLong(),
    endMs = endUnixMs?.toLong(),
    alarmed = alarmed,
    notTheBaby = notTheBaby,
)

/**
 * One night of the log: noon of [date] to noon of the next day in local
 * time. [entries] are oldest first; [longestQuietMs] is the longest gap
 * between consecutive wake-ups (and, for the night that is going on, from
 * the last wake-up's end until now), `null` when there is no gap.
 */
data class Night(val date: LocalDate, val entries: List<LogEntry>, val longestQuietMs: Long?)

/** The night (by its starting date) a moment belongs to: noon → noon. */
fun nightOf(unixMs: Long, zone: ZoneId): LocalDate {
    val local = Instant.ofEpochMilli(unixMs).atZone(zone)
    val d = local.toLocalDate()
    return if (local.toLocalTime() < LocalTime.NOON) d.minusDays(1) else d
}

/** Unix ms of the noon that starts the night of [date]. */
fun nightStartMs(date: LocalDate, zone: ZoneId): Long =
    date.atTime(LocalTime.NOON).atZone(zone).toInstant().toEpochMilli()

/** Groups log entries (any order) by night, newest night first. */
fun groupNights(entries: List<LogEntry>, nowMs: Long, zone: ZoneId): List<Night> {
    val tonight = nightOf(nowMs, zone)
    return entries
        .sortedBy { it.startMs }
        .groupBy { nightOf(it.startMs, zone) }
        .map { (date, list) ->
            var longest: Long? = null
            fun gap(ms: Long) {
                if (ms > 0 && (longest == null || ms > longest!!)) longest = ms
            }
            for (i in 1 until list.size) {
                val end = list[i - 1].endMs ?: continue
                gap(list[i].startMs - end)
            }
            val last = list.last()
            if (date == tonight && last.endMs != null) {
                val nightEnd = nightStartMs(date.plusDays(1), zone)
                gap(minOf(nowMs, nightEnd) - last.endMs)
            }
            Night(date, list, longest)
        }
        .sortedByDescending { it.date }
}

/** Whole hours and the remaining whole minutes of a duration. */
fun hoursMinutes(ms: Long): Pair<Int, Int> {
    val mins = (ms.coerceAtLeast(0) / 60_000).toInt()
    return mins / 60 to mins % 60
}

/** The escalation choices of FR-AL-5, in seconds. */
val ESCALATE_CHOICES: List<Int> = listOf(30, 60, 120, 300)

/** The choice closest to a stored value. */
fun escalateChoice(seconds: Int): Int = ESCALATE_CHOICES.minBy { kotlin.math.abs(it - seconds) }

/** Player volume of the alarm ramp: 0.3 → 1.0 over 30 s (FR-AL-5). */
fun alarmRampVolume(elapsedMs: Long): Float {
    val f = (elapsedMs.coerceAtLeast(0).toFloat() / 30_000f).coerceAtMost(1f)
    return 0.3f + 0.7f * f
}
