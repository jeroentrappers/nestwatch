// SPDX-License-Identifier: AGPL-3.0-or-later
import AVFoundation
import Foundation

/// The engines' playback and talk-back rate (media.md §4).
let engineRate: Double = 16_000

/// Converts float samples in [-1, 1] to little-endian 16-bit PCM bytes.
func pcm16(from samples: UnsafeBufferPointer<Float>) -> Data {
    var out = Data(count: samples.count * 2)
    out.withUnsafeMutableBytes { raw in
        let dst = raw.bindMemory(to: Int16.self)
        for (i, s) in samples.enumerated() {
            let v = max(-1, min(1, s))
            dst[i] = Int16(v * 32767).littleEndian
        }
    }
    return out
}

/// Writes little-endian 16-bit PCM bytes as floats into `dst` (at most
/// `dst.count` samples; the rest is silence).
func floats(fromPCM16 data: Data, into dst: UnsafeMutableBufferPointer<Float>) {
    let n = min(data.count / 2, dst.count)
    data.withUnsafeBytes { raw in
        for i in 0..<n {
            let lo = UInt16(raw[2 * i]), hi = UInt16(raw[2 * i + 1])
            dst[i] = Float(Int16(bitPattern: lo | hi << 8)) / 32768
        }
    }
    for i in n..<dst.count { dst[i] = 0 }
}

/// Audio session set-up per role (platforms.md §2.3, IOS-3).
enum AudioSessionConfig {
    /// The station records and plays talk-back: `playAndRecord` without
    /// `mixWithOthers`, so it keeps the microphone in the background.
    static func station() throws {
        let s = AVAudioSession.sharedInstance()
        try s.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker, .allowBluetooth])
        try s.setActive(true)
    }

    /// The listener plays the mix and records only while talking. It mixes
    /// with other apps' audio, so a parent can keep their own audio on.
    static func listener() throws {
        let s = AVAudioSession.sharedInstance()
        try s.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker, .allowBluetooth, .allowBluetoothA2DP, .mixWithOthers])
        try s.setActive(true)
    }

    static func deactivate() {
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    static var micGranted: Bool { AVAudioSession.sharedInstance().recordPermission == .granted }

    static func requestMic(_ done: @escaping (Bool) -> Void) {
        AVAudioSession.sharedInstance().requestRecordPermission { ok in DispatchQueue.main.async { done(ok) } }
    }
}

/// An `AVAudioEngine` that restarts itself after interruptions (calls,
/// Siri) and configuration changes (route, sample rate), re-running
/// `configure` each time.
class RestartingEngine {
    private(set) var engine = AVAudioEngine()
    private var observers: [NSObjectProtocol] = []
    private(set) var running = false
    /// A microphone tap is installed (only then is the input node touched).
    private var tapped = false
    let name: String

    init(name: String) { self.name = name }

    /// Attaches nodes and taps to a fresh engine; returns false if it
    /// cannot run (no input, for example).
    func configure(_ engine: AVAudioEngine) -> Bool { true }

    @discardableResult
    func start() -> Bool {
        guard !running else { return true }
        running = true
        let nc = NotificationCenter.default
        observers = [
            nc.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] n in
                guard let self, let raw = n.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                      AVAudioSession.InterruptionType(rawValue: raw) == .ended else { return }
                self.rebuild()
            },
            nc.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: .main) { [weak self] _ in
                self?.rebuild()
            },
        ]
        return rebuild()
    }

    @discardableResult
    private func rebuild() -> Bool {
        guard running else { return false }
        teardown()
        let e = AVAudioEngine()
        engine = e
        observers.append(NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: e, queue: .main
        ) { [weak self] _ in self?.rebuild() })
        guard configure(e) else {
            NSLog("Nestwatch: %@ audio has no usable device", name)
            return false
        }
        do {
            e.prepare()
            try e.start()
            return true
        } catch {
            NSLog("Nestwatch: %@ audio failed to start: %@", name, String(describing: error))
            return false
        }
    }

    private func teardown() {
        engine.stop()
        if tapped { engine.inputNode.removeTap(onBus: 0) }
        tapped = false
        for n in engine.attachedNodes where n is AVAudioSourceNode { engine.detach(n) }
    }

    func stop() {
        guard running else { return }
        running = false
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        teardown()
    }

    /// Taps the microphone in its native format and hands mono float
    /// blocks (about 20 ms) with their rate to `sink`.
    func tapMic(_ e: AVAudioEngine, sink: @escaping (UnsafeBufferPointer<Float>, Double) -> Void) -> Bool {
        let input = e.inputNode
        let fmt = input.outputFormat(forBus: 0)
        guard fmt.channelCount > 0, fmt.sampleRate > 0 else { return false }
        let block = AVAudioFrameCount(fmt.sampleRate / 50)
        input.installTap(onBus: 0, bufferSize: block, format: fmt) { buf, _ in
            guard let ch = buf.floatChannelData?[0] else { return }
            sink(UnsafeBufferPointer(start: ch, count: Int(buf.frameLength)), fmt.sampleRate)
        }
        tapped = true
        return true
    }

    /// A 16 kHz mono source node that fills each render from `pull`.
    static func source(_ e: AVAudioEngine, pull: @escaping (UInt32) -> Data) {
        guard let fmt = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: engineRate, channels: 1, interleaved: false) else { return }
        let node = AVAudioSourceNode(format: fmt) { _, _, frames, abl -> OSStatus in
            let buffers = UnsafeMutableAudioBufferListPointer(abl)
            guard let b = buffers.first, let p = b.mData?.assumingMemoryBound(to: Float.self) else { return noErr }
            let dst = UnsafeMutableBufferPointer(start: p, count: Int(frames))
            floats(fromPCM16: pull(frames), into: dst)
            return noErr
        }
        e.attach(node)
        e.connect(node, to: e.mainMixerNode, format: fmt)
    }
}

/// The station's audio: microphone capture to the engine (FR-ST-1) and
/// talk-back playback (FR-TB-3). Pulling talk-back while nobody talks
/// yields silence, so the output runs for the station's whole life.
final class StationAudio: RestartingEngine {
    private let station: Station
    /// Whether the microphone delivered audio when last configured.
    private(set) var hasMic = false

    init(station: Station) {
        self.station = station
        super.init(name: "station")
    }

    override func configure(_ e: AVAudioEngine) -> Bool {
        let s = station
        hasMic = AudioSessionConfig.micGranted && tapMic(e) { samples, rate in
            let data = Data(buffer: samples)
            s.pushCaptureF32(pcm: data, rate: UInt32(rate), channels: 1)
        }
        RestartingEngine.source(e) { frames in s.pullTalkback(frames: frames).pcm }
        return true
    }
}

/// The listener's mix of all audible stations (FR-LS-2).
final class ListenerPlayback: RestartingEngine {
    private let listener: Listener

    init(listener: Listener) {
        self.listener = listener
        super.init(name: "playback")
    }

    override func configure(_ e: AVAudioEngine) -> Bool {
        let l = listener
        RestartingEngine.source(e) { frames in l.pullPlayback(frames: frames) }
        return true
    }
}

/// The listener's microphone while talk-back is granted (FR-TB-1).
final class TalkCapture: RestartingEngine {
    private let listener: Listener

    init(listener: Listener) {
        self.listener = listener
        super.init(name: "talk")
    }

    override func configure(_ e: AVAudioEngine) -> Bool {
        let l = listener
        return tapMic(e) { samples, rate in
            l.pushTalkCapture(pcm: pcm16(from: samples), rate: UInt32(rate), channels: 1)
        }
    }
}
