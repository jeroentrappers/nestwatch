// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android

import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.lull.android.core.LevelHistory
import org.lull.android.core.clampThreshold
import org.lull.android.core.fractionToDb
import org.lull.android.core.levelFraction
import org.lull.android.ui.screens.QrDecoder
import org.lull.android.ui.screens.clock

class LogicTest {
    @Test
    fun levelMappingMatchesUi1() {
        assertEquals(0f, levelFraction(-90f), 1e-6f)
        assertEquals(0f, levelFraction(-120f), 1e-6f)
        assertEquals(1f, levelFraction(-10f), 1e-6f)
        assertEquals(1f, levelFraction(0f), 1e-6f)
        assertEquals(0.5f, levelFraction(-50f), 1e-6f)
        assertEquals(-50f, fractionToDb(0.5f), 1e-4f)
        assertEquals(0f, levelFraction(Float.NaN), 0f)
    }

    @Test
    fun thresholdsAreWholeDecibelsInRange() {
        assertEquals(-85f, clampThreshold(-99f), 0f)
        assertEquals(-10f, clampThreshold(-3f), 0f)
        assertEquals(-42f, clampThreshold(-42.4f), 0f)
    }

    @Test
    fun historyKeepsPeakPerBucketForOneMinute() {
        val h = LevelHistory()
        h.add(1_000, -60f)
        h.add(1_050, -40f)
        h.add(1_100, -70f)
        assertEquals(1, h.size)
        assertEquals(-40f, h.range()!!.second, 0f)
        h.add(1_200, -55f)
        assertEquals(2, h.size)
        // Older than 60 s falls out.
        h.add(62_000, -80f)
        val s = h.series(62_000)
        assertTrue(s.all { it.first in 0f..1f })
        assertEquals(1f, s.last().first, 1e-6f)
        h.add(200_000, -30f)
        assertEquals(1, h.size)
    }

    @Test
    fun countdownClock() {
        assertEquals("10:00", clock(600_000))
        assertEquals("0:05", clock(5_400))
        assertEquals("0:00", clock(-5))
    }

    @Test
    fun zxingDecodesAnInvitationQr() {
        val text = "lull://invite?v=1&k=abc&t=def&n=Nursery&e=192.168.1.20:47100"
        val m = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 240, 240)
        val w = m.width
        val h = m.height
        val y = ByteArray(w * h) { i -> if (m.get(i % w, i / w)) 0 else 255.toByte() }
        assertEquals(text, QrDecoder().decode(y, w, w, h))
        assertNull(QrDecoder().decode(ByteArray(w * h) { 127 }, w, w, h))
    }
}
