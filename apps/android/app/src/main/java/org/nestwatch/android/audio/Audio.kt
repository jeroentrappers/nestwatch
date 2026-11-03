// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.audio

import android.annotation.SuppressLint
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.os.Process
import android.util.Log

private const val TAG = "NestwatchAudio"

/** The engines' playback and talk-back rate (media.md §4). */
const val ENGINE_RATE = 16_000

/** 20 ms at 16 kHz. */
const val FRAME_SAMPLES = 320

/**
 * Microphone capture on its own thread: 16-bit mono at 16 kHz where the
 * device offers it, else 48 or 44.1 kHz (the core resamples), delivered
 * in 20 ms blocks of little-endian PCM to [sink]. Requires `RECORD_AUDIO`.
 */
class MicCapture(
    private val name: String,
    private val source: Int = MediaRecorder.AudioSource.MIC,
    private val sink: (pcm: ByteArray, rate: Int) -> Unit,
) {
    @Volatile private var running = false
    private var thread: Thread? = null

    /** The rate in use, or 0 if the microphone could not be opened. */
    @Volatile var rate: Int = 0
        private set

    @SuppressLint("MissingPermission")
    private fun open(): AudioRecord? {
        for (r in intArrayOf(ENGINE_RATE, 48_000, 44_100)) {
            val min = AudioRecord.getMinBufferSize(r, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
            if (min <= 0) continue
            val rec = try {
                AudioRecord(source, r, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(min, r / 10 * 2))
            } catch (e: Exception) {
                continue
            }
            if (rec.state == AudioRecord.STATE_INITIALIZED) {
                rate = r
                return rec
            }
            rec.release()
        }
        return null
    }

    /** Starts capturing; returns false if no microphone could be opened. */
    fun start(): Boolean {
        if (running) return true
        val rec = open() ?: run {
            Log.w(TAG, "$name: no microphone")
            return false
        }
        running = true
        thread = Thread({
            Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
            val block = ByteArray(rate / 50 * 2)
            try {
                rec.startRecording()
                while (running) {
                    var off = 0
                    while (running && off < block.size) {
                        val n = rec.read(block, off, block.size - off)
                        if (n < 0) {
                            Log.w(TAG, "$name: read error $n")
                            running = false
                            break
                        }
                        off += n
                    }
                    if (off == block.size) sink(block.copyOf(), rate)
                }
            } catch (e: Exception) {
                Log.w(TAG, "$name: ${e.javaClass.simpleName}")
            } finally {
                runCatching { rec.stop() }
                rec.release()
            }
        }, "nestwatch-$name").apply { start() }
        return true
    }

    fun stop() {
        running = false
        thread?.join(500)
        thread = null
    }
}

/**
 * Playback on its own thread at 16 kHz mono: pulls 20 ms blocks from
 * [source] and writes them in blocking mode, so pulls follow the output
 * device's pace (CORE-12).
 */
class SpeakerPlayback(
    private val name: String,
    private val usage: Int = AudioAttributes.USAGE_MEDIA,
    private val source: (frames: Int) -> ByteArray,
) {
    @Volatile private var running = false
    private var thread: Thread? = null

    fun start(): Boolean {
        if (running) return true
        val min = AudioTrack.getMinBufferSize(ENGINE_RATE, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
        if (min <= 0) return false
        val track = try {
            AudioTrack(
                AudioAttributes.Builder()
                    .setUsage(usage)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
                AudioFormat.Builder()
                    .setSampleRate(ENGINE_RATE)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .build(),
                // About 80 ms of buffer: low latency, room for scheduling jitter.
                maxOf(min, FRAME_SAMPLES * 2 * 4),
                AudioTrack.MODE_STREAM,
                android.media.AudioManager.AUDIO_SESSION_ID_GENERATE,
            )
        } catch (e: Exception) {
            Log.w(TAG, "$name: ${e.javaClass.simpleName}")
            return false
        }
        if (track.state != AudioTrack.STATE_INITIALIZED) {
            track.release()
            return false
        }
        running = true
        thread = Thread({
            Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
            try {
                track.play()
                while (running) {
                    val pcm = source(FRAME_SAMPLES)
                    var off = 0
                    while (running && off < pcm.size) {
                        val n = track.write(pcm, off, pcm.size - off)
                        if (n < 0) {
                            running = false
                            break
                        }
                        off += n
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "$name: ${e.javaClass.simpleName}")
            } finally {
                runCatching { track.pause(); track.flush(); track.stop() }
                track.release()
            }
        }, "nestwatch-$name").apply { start() }
        return true
    }

    fun stop() {
        running = false
        thread?.join(500)
        thread = null
    }
}
