// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android.core

/**
 * The last minute of a station's room level for the calibration chart
 * (UI-5): one point per 200 ms bucket, the peak of the reports in that
 * bucket, kept in memory for 60 s.
 */
class LevelHistory(
    private val bucketMs: Long = 200,
    private val spanMs: Long = 60_000,
) {
    private val points = ArrayDeque<Pair<Long, Float>>()

    /** Records `dbfs` at monotonic time `nowMs`. */
    fun add(nowMs: Long, dbfs: Float) {
        if (dbfs.isNaN()) return
        val bucket = nowMs / bucketMs
        val last = points.lastOrNull()
        if (last != null && last.first == bucket) {
            if (dbfs > last.second) points[points.lastIndex] = bucket to dbfs
        } else if (last == null || bucket > last.first) {
            points.addLast(bucket to dbfs)
        }
        trim(nowMs)
    }

    private fun trim(nowMs: Long) {
        val oldest = (nowMs - spanMs) / bucketMs
        while (points.isNotEmpty() && points.first().first < oldest) points.removeFirst()
    }

    /**
     * Points as (age 0..1 from oldest to now, dBFS) for drawing; x = 1 is
     * `nowMs`.
     */
    fun series(nowMs: Long): List<Pair<Float, Float>> {
        trim(nowMs)
        val now = nowMs / bucketMs
        val span = (spanMs / bucketMs).toFloat()
        return points.map { (b, v) -> (1f - (now - b) / span).coerceIn(0f, 1f) to v }
    }

    /** Number of buckets held. */
    val size: Int get() = points.size

    /** Highest and lowest level held, or null. */
    fun range(): Pair<Float, Float>? =
        if (points.isEmpty()) null else points.minOf { it.second } to points.maxOf { it.second }
}

/** Meter position of a level: linear in dB from −90 (0) to −10 dBFS (1) (UI-1). */
fun levelFraction(dbfs: Float): Float = if (dbfs.isNaN()) 0f else ((dbfs + 90f) / 80f).coerceIn(0f, 1f)

/** The level for a meter position (inverse of [levelFraction]). */
fun fractionToDb(f: Float): Float = -90f + 80f * f.coerceIn(0f, 1f)

/** Thresholds the user can pick (UI-1): −85..−10 dBFS in 1 dB steps. */
const val THRESHOLD_MIN = -85f
const val THRESHOLD_MAX = -10f

/** Rounds and clamps a threshold to the allowed steps. */
fun clampThreshold(db: Float): Float = Math.round(db).toFloat().coerceIn(THRESHOLD_MIN, THRESHOLD_MAX)
