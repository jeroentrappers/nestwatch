// SPDX-License-Identifier: AGPL-3.0-or-later
import AVFoundation
import AudioToolbox
import Foundation

/// The alarm tone (FR-AL-5): an original pattern synthesised here, three
/// rising chimes and a pause, about 1.6 s, played in a loop. 16-bit mono
/// WAV bytes, so `AVAudioPlayer` can loop it without a bundled file.
func alarmToneWav(rate: Int = 22_050) -> Data {
    // Notes (Hz) with their start (s) and length (s); a soft square-ish
    // timbre (fundamental plus odd harmonics) cuts through room noise.
    let notes: [(f: Double, at: Double, len: Double)] = [
        (880, 0.00, 0.16), (1_108.7, 0.20, 0.16), (1_318.5, 0.40, 0.30),
    ]
    let total = 1.6
    let n = Int(total * Double(rate))
    var samples = [Int16](repeating: 0, count: n)
    for note in notes {
        let start = Int(note.at * Double(rate))
        let len = Int(note.len * Double(rate))
        for i in 0..<len where start + i < n {
            let t = Double(i) / Double(rate)
            let w = 2 * Double.pi * note.f * t
            let v = sin(w) + 0.33 * sin(3 * w) + 0.15 * sin(5 * w)
            // 8 ms attack, exponential decay: no clicks, a bell-like tail.
            let attack = min(1, t / 0.008)
            let release = min(1, Double(len - i) / (0.012 * Double(rate)))
            let env = attack * release * exp(-3 * t / note.len)
            let s = 0.6 * v / 1.48 * env
            samples[start + i] = Int16(max(-1, min(1, s)) * 32_767)
        }
    }
    var d = Data()
    func put32(_ v: UInt32) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
    func put16(_ v: UInt16) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
    let bytes = UInt32(n * 2)
    d.append(contentsOf: Array("RIFF".utf8)); put32(36 + bytes)
    d.append(contentsOf: Array("WAVE".utf8))
    d.append(contentsOf: Array("fmt ".utf8)); put32(16); put16(1); put16(1)
    put32(UInt32(rate)); put32(UInt32(rate * 2)); put16(2); put16(16)
    d.append(contentsOf: Array("data".utf8)); put32(bytes)
    for s in samples { put16(UInt16(bitPattern: s)) }
    return d
}

/// The player volume of the alarm `elapsed` seconds after it started:
/// 30 % rising linearly to 100 % over 30 s (FR-AL-5).
func alarmVolume(elapsed: TimeInterval) -> Float {
    Float(0.3 + 0.7 * min(max(elapsed, 0), 30) / 30)
}

/// Plays the alarm while any station's alarm rings (FR-AL-5): one loop for
/// all stations, through the listener's audio session (`playAndRecord`
/// ignores the ring/silent switch, and the running playback keeps the app
/// alive in the background, platforms.md §2.3), with vibration. The
/// station's mute does not apply: this player is not part of the mix.
final class AlarmPlayer {
    private var player: AVAudioPlayer?
    private var vibration: Timer?
    private lazy var tone = alarmToneWav()

    private(set) var ringing = false

    func update(ringing: Bool) {
        if ringing { start() } else { stop() }
    }

    private func start() {
        guard !ringing else { return }
        ringing = true
        try? AVAudioSession.sharedInstance().setActive(true)
        do {
            let p = try AVAudioPlayer(data: tone)
            p.numberOfLoops = -1
            p.volume = alarmVolume(elapsed: 0)
            p.prepareToPlay()
            p.play()
            p.setVolume(1, fadeDuration: 30)
            player = p
        } catch {
            NSLog("Nestwatch: alarm player failed: %@", String(describing: error))
            AudioServicesPlayAlertSound(SystemSoundID(1005))
        }
        AudioServicesPlaySystemSound(kSystemSoundID_Vibrate)
        let t = Timer(timeInterval: 2, repeats: true) { _ in AudioServicesPlaySystemSound(kSystemSoundID_Vibrate) }
        RunLoop.main.add(t, forMode: .common)
        vibration = t
    }

    private func stop() {
        guard ringing else { return }
        ringing = false
        vibration?.invalidate()
        vibration = nil
        player?.stop()
        player = nil
    }
}
