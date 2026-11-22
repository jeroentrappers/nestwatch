// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android

import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneId
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.nestwatch.android.audio.AlarmPlayer
import org.nestwatch.android.core.LogEntry
import org.nestwatch.android.core.alarmRampVolume
import org.nestwatch.android.core.escalateChoice
import org.nestwatch.android.core.groupNights
import org.nestwatch.android.core.hoursMinutes
import org.nestwatch.android.core.nightOf

class NightLogTest {
    private val zone = ZoneId.of("Europe/Brussels")
    private fun at(y: Int, mo: Int, d: Int, h: Int, mi: Int) =
        LocalDateTime.of(y, mo, d, h, mi).atZone(zone).toInstant().toEpochMilli()
    private val min = 60_000L

    @Test
    fun nightsRunFromNoonToNoon() {
        assertEquals(LocalDate.of(2026, 11, 21), nightOf(at(2026, 11, 21, 12, 0), zone))
        assertEquals(LocalDate.of(2026, 11, 21), nightOf(at(2026, 11, 22, 3, 0), zone))
        assertEquals(LocalDate.of(2026, 11, 21), nightOf(at(2026, 11, 22, 11, 59), zone))
        assertEquals(LocalDate.of(2026, 11, 20), nightOf(at(2026, 11, 21, 11, 59), zone))
    }

    @Test
    fun groupsNewestNightFirstWithLongestQuiet() {
        val a = LogEntry(at(2026, 11, 20, 23, 0), at(2026, 11, 20, 23, 5), alarmed = false, notTheBaby = false)
        val b = LogEntry(at(2026, 11, 21, 2, 0), at(2026, 11, 21, 2, 10), alarmed = true, notTheBaby = false)
        val c = LogEntry(at(2026, 11, 21, 4, 0), at(2026, 11, 21, 4, 1), alarmed = false, notTheBaby = true)
        val d = LogEntry(at(2026, 11, 21, 22, 0), null, alarmed = false, notTheBaby = false)
        val now = at(2026, 11, 21, 22, 30)
        val nights = groupNights(listOf(d, c, a, b), now, zone)
        assertEquals(listOf(LocalDate.of(2026, 11, 21), LocalDate.of(2026, 11, 20)), nights.map { it.date })
        // Tonight: one ongoing wake-up, no gap yet.
        assertNull(nights[0].longestQuietMs)
        assertEquals(30 * min, nights[0].entries.single().durationMs(now))
        // Last night: 23:05 → 02:00 is the longest gap (2 h 55 min).
        assertEquals(listOf(a, b, c), nights[1].entries)
        assertEquals((2 * 60 + 55) * min, nights[1].longestQuietMs)
    }

    @Test
    fun tonightCountsQuietUntilNow() {
        val e = LogEntry(at(2026, 11, 21, 20, 0), at(2026, 11, 21, 20, 10), alarmed = false, notTheBaby = false)
        val n = groupNights(listOf(e), at(2026, 11, 22, 1, 10), zone).single()
        assertEquals(5 * 60 * min, n.longestQuietMs)
    }

    @Test
    fun durationsAndChoices() {
        assertEquals(0 to 4, hoursMinutes(4 * min + 59_000))
        assertEquals(1 to 20, hoursMinutes(80 * min))
        assertEquals(30, escalateChoice(10))
        assertEquals(60, escalateChoice(60))
        assertEquals(300, escalateChoice(600))
    }

    @Test
    fun alarmRampsFromThirtyPercentOverThirtySeconds() {
        assertEquals(0.3f, alarmRampVolume(0), 1e-6f)
        assertEquals(0.65f, alarmRampVolume(15_000), 1e-6f)
        assertEquals(1f, alarmRampVolume(30_000), 1e-6f)
        assertEquals(1f, alarmRampVolume(90_000), 1e-6f)
        val pcm = AlarmPlayer.alarmPattern()
        assertTrue(pcm.size in 22_050..22_050 * 3)
        assertTrue(pcm.any { it > 20_000 })
    }
}
