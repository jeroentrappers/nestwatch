// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.audio

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.os.VibrationAttributes
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Log
import kotlin.math.PI
import kotlin.math.exp
import kotlin.math.min
import kotlin.math.sin
import org.nestwatch.android.core.alarmRampVolume

private const val TAG = "NestwatchAlarm"
private const val RATE = 22_050

/**
 * The escalating alarm (product.md FR-AL-5, platforms.md §2.2): a looping
 * tone pattern synthesised here on the alarm stream (`USAGE_ALARM`, which
 * rings in silent and vibrate mode and ignores the station's mute), with
 * the player volume ramping from 0.3 to 1.0 over 30 s under the user's
 * alarm volume, and a repeating vibration. One player for all ringing
 * stations; call [start] and [stop] from the main thread.
 */
class AlarmPlayer(context: Context) {
    private val app = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private var track: AudioTrack? = null
    private var startedAt = 0L
    private var vibrating = false

    val isPlaying: Boolean get() = track != null || vibrating

    private val attributes: AudioAttributes = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_ALARM)
        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
        .build()

    private val ramp = object : Runnable {
        override fun run() {
            val t = track ?: return
            val v = alarmRampVolume(SystemClock.elapsedRealtime() - startedAt)
            runCatching { t.setVolume(v) }
            if (v < 1f) main.postDelayed(this, 250)
        }
    }

    fun start() {
        if (isPlaying) return
        startedAt = SystemClock.elapsedRealtime()
        track = openTrack()
        track?.let { main.post(ramp) }
        vibrate()
    }

    fun stop() {
        main.removeCallbacks(ramp)
        track?.let { t ->
            runCatching { t.pause(); t.flush(); t.stop() }
            t.release()
        }
        track = null
        if (vibrating) runCatching { vibrator()?.cancel() }
        vibrating = false
    }

    private fun openTrack(): AudioTrack? {
        val pcm = alarmPattern()
        return try {
            val t = AudioTrack(
                attributes,
                AudioFormat.Builder()
                    .setSampleRate(RATE)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .build(),
                pcm.size * 2,
                AudioTrack.MODE_STATIC,
                AudioManager.AUDIO_SESSION_ID_GENERATE,
            )
            if (t.state == AudioTrack.STATE_UNINITIALIZED) {
                t.release()
                return null
            }
            t.write(pcm, 0, pcm.size)
            t.setLoopPoints(0, pcm.size, -1)
            t.setVolume(alarmRampVolume(0))
            t.play()
            t
        } catch (e: Exception) {
            Log.w(TAG, "alarm track: ${e.javaClass.simpleName}")
            null
        }
    }

    @Suppress("DEPRECATION")
    private fun vibrator(): Vibrator? = if (Build.VERSION.SDK_INT >= 31) {
        app.getSystemService(VibratorManager::class.java)?.defaultVibrator
    } else {
        app.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator
    }

    @Suppress("DEPRECATION")
    private fun vibrate() {
        val v = vibrator()?.takeIf { it.hasVibrator() } ?: return
        val pattern = longArrayOf(0, 700, 300, 700, 1300)
        try {
            when {
                Build.VERSION.SDK_INT >= 33 -> v.vibrate(
                    VibrationEffect.createWaveform(pattern, 0),
                    VibrationAttributes.createForUsage(VibrationAttributes.USAGE_ALARM),
                )
                Build.VERSION.SDK_INT >= 26 -> v.vibrate(VibrationEffect.createWaveform(pattern, 0), attributes)
                else -> v.vibrate(pattern, 0, attributes)
            }
            vibrating = true
        } catch (e: Exception) {
            Log.w(TAG, "vibrate: ${e.javaClass.simpleName}")
        }
    }

    companion object {
        /**
         * One loop of the alarm, generated here (no recorded sound): three
         * rising soft-square chimes (E5, A5, C#6), a repeat, then a pause;
         * about 2 s at 22.05 kHz, 16-bit mono.
         */
        fun alarmPattern(): ShortArray {
            val notes = listOf(659.25 to 0.16, 880.0 to 0.16, 1108.73 to 0.30)
            val gap = 0.07
            val out = ArrayList<Double>(RATE * 2)
            fun silence(s: Double) = repeat((s * RATE).toInt()) { out.add(0.0) }
            repeat(2) {
                for ((f, len) in notes) {
                    val n = (len * RATE).toInt()
                    for (i in 0 until n) {
                        val t = i.toDouble() / RATE
                        // Fundamental plus odd harmonics: bright enough to wake, not harsh.
                        val w = sin(2 * PI * f * t) + sin(2 * PI * 3 * f * t) / 4 + sin(2 * PI * 5 * f * t) / 10
                        val attack = min(1.0, t / 0.008)
                        val release = min(1.0, (len - t) / 0.02)
                        val env = attack * release * (0.55 + 0.45 * exp(-t * 6))
                        out.add(w * env)
                    }
                    silence(gap)
                }
                silence(0.18)
            }
            silence(0.55)
            // Normalised to 85 % of full scale; the ramp and the alarm volume do the rest.
            val peak = out.maxOf { kotlin.math.abs(it) }.coerceAtLeast(1e-9)
            return ShortArray(out.size) { (out[it] / peak * 0.85 * Short.MAX_VALUE).toInt().toShort() }
        }
    }
}
