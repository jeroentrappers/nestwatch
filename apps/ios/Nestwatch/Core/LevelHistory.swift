// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// The last minute of a station's room level for the calibration chart
/// (UI-5): one point per 200 ms bucket, the peak of the reports in that
/// bucket, kept in memory for 60 s.
struct LevelHistory {
    let bucketMs: Int64
    let spanMs: Int64
    private var points: [(bucket: Int64, db: Float)] = []

    init(bucketMs: Int64 = 200, spanMs: Int64 = 60_000) {
        self.bucketMs = bucketMs
        self.spanMs = spanMs
    }

    /// Records `dbfs` at monotonic time `nowMs`.
    mutating func add(nowMs: Int64, dbfs: Float) {
        guard !dbfs.isNaN else { return }
        let bucket = nowMs / bucketMs
        if let last = points.last, last.bucket == bucket {
            if dbfs > last.db { points[points.count - 1] = (bucket, dbfs) }
        } else if points.last.map({ bucket > $0.bucket }) ?? true {
            points.append((bucket, dbfs))
        }
        trim(nowMs: nowMs)
    }

    private mutating func trim(nowMs: Int64) {
        let oldest = (nowMs - spanMs) / bucketMs
        if let i = points.firstIndex(where: { $0.bucket >= oldest }) {
            if i > 0 { points.removeFirst(i) }
        } else {
            points.removeAll()
        }
    }

    /// Points as (age 0..1 from oldest to now, dBFS) for drawing; x = 1 is
    /// `nowMs`.
    mutating func series(nowMs: Int64) -> [(x: Float, db: Float)] {
        trim(nowMs: nowMs)
        let now = nowMs / bucketMs
        let span = Float(spanMs / bucketMs)
        return points.map { p in (min(max(1 - Float(now - p.bucket) / span, 0), 1), p.db) }
    }

    /// Number of buckets held.
    var count: Int { points.count }

    /// Lowest and highest level held, or nil.
    var range: (min: Float, max: Float)? {
        guard !points.isEmpty else { return nil }
        return (points.map(\.db).min()!, points.map(\.db).max()!)
    }
}

/// Meter position of a level: linear in dB from −90 (0) to −10 dBFS (1) (UI-1).
func levelPosition(_ dbfs: Float) -> Float {
    dbfs.isNaN ? 0 : min(max((dbfs + 90) / 80, 0), 1)
}

/// The level for a meter position (inverse of [levelPosition]).
func positionToDb(_ f: Float) -> Float { -90 + 80 * min(max(f, 0), 1) }

/// Thresholds the user can pick (UI-1): −85..−10 dBFS in 1 dB steps.
let thresholdMin: Float = -85
let thresholdMax: Float = -10

/// Rounds and clamps a threshold to the allowed steps.
func clampThreshold(_ db: Float) -> Float { min(max(db.rounded(), thresholdMin), thresholdMax) }
