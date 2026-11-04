// SPDX-License-Identifier: AGPL-3.0-or-later
import Combine
import Foundation
import UIKit

/// Station lifecycle as the UI sees it.
enum StationRun: Equatable {
    case stopped
    case starting
    case running(StationSnapshot)
    case failed(String)

    var snapshot: StationSnapshot? {
        if case let .running(s) = self { return s }
        return nil
    }
}

/// A listener that paired in the open window, with its verification number.
struct JustPaired: Equatable, Identifiable {
    let id = UUID()
    let name: String
    let verification: String
}

/// Process-wide owner of the engines (architecture.md §5): starts and stops
/// the [Station] and [Listener] from nestwatch-ffi and republishes their
/// snapshots, events and video frames for the UI and the platform hosts.
/// Observer callbacks arrive on engine threads and hop to the main queue;
/// everything else runs on the main thread.
final class Nestwatch: ObservableObject {
    static let shared = Nestwatch()

    let prefs = Prefs()
    let storage = KeychainStorage()

    /// A default display name for this device.
    static func deviceName() -> String {
        let n = UIDevice.current.name
        return n.isEmpty ? UIDevice.current.model : n
    }

    // MARK: station

    @Published private(set) var station: StationRun = .stopped
    @Published private(set) var justPaired: [JustPaired] = []
    /// Debug builds: show the pairing sheet (NESTWATCH_DEBUG_SCREEN).
    @Published var debugPairing = false
    let stationEvents = PassthroughSubject<StationEventInfo, Never>()
    private(set) var stationEngine: Station?
    private var stationStarting = false
    private lazy var stationObserver = StationBridge(model: self)

    /// Starts the station engine with the app's interface list.
    func startStation() async -> Station? {
        if let s = stationEngine { return s }
        guard !stationStarting else { return nil }
        stationStarting = true
        defer { stationStarting = false }
        station = .starting
        let options = StationOptions(
            name: Nestwatch.deviceName(),
            port: 0,
            // Advertising goes through Bonjour (platforms.md §2.1, IOS-1).
            mdns: false,
            interfaces: NetInfo.interfaces(),
            extraEndpoints: [],
            loopback: false
        )
        do {
            let s = try await ffiStartStation(options: options, storage: storage, observer: stationObserver)
            stationEngine = s
            station = .running(s.snapshot())
            return s
        } catch {
            NSLog("Nestwatch: station failed to start: %@", String(describing: type(of: error)))
            station = .failed(error.localizedDescription)
            return nil
        }
    }

    /// Stops the station engine.
    func stopStation() async {
        guard let s = stationEngine else {
            if case .failed = station {} else { station = .stopped }
            return
        }
        stationEngine = nil
        await s.stop()
        justPaired = []
        station = .stopped
    }

    fileprivate func stationState(_ s: StationSnapshot) {
        if stationEngine != nil { station = .running(s) }
    }

    fileprivate func stationEvent(_ e: StationEventInfo) {
        switch e {
        case let .paired(_, name, verification):
            justPaired.append(JustPaired(name: name, verification: verification))
        case .pairingOpened:
            justPaired = []
        default:
            break
        }
        stationEvents.send(e)
    }

    /// Runs a station intent if the station runs, ignoring engine errors.
    func withStation(_ f: (Station) throws -> Void) {
        guard let s = stationEngine else { return }
        do { try f(s) } catch { NSLog("Nestwatch: station intent failed: %@", String(describing: type(of: error))) }
    }

    // MARK: listener

    @Published private(set) var listenerState: ListenerSnapshot?
    /// Bumps whenever a level history changes (at most every 200 ms).
    @Published private(set) var historyVersion: Int64 = 0
    let listenerEvents = PassthroughSubject<ListenerEventInfo, Never>()
    let frames = VideoFrames()
    private var histories: [String: LevelHistory] = [:]
    private(set) var listenerEngine: Listener?
    private lazy var listenerObserver = ListenerBridge(model: self)

    /// Starts the listener engine if needed and returns it.
    @discardableResult
    func listener() -> Listener? {
        if let l = listenerEngine { return l }
        do {
            let l = try Listener.start(
                options: ListenerOptions(name: Nestwatch.deviceName(), discovery: false),
                storage: storage,
                observer: listenerObserver
            )
            listenerEngine = l
            listenerState = l.snapshot()
            return l
        } catch {
            NSLog("Nestwatch: listener failed to start: %@", String(describing: type(of: error)))
            return nil
        }
    }

    /// Stops the listener engine.
    func stopListener() async {
        guard let l = listenerEngine else { return }
        listenerEngine = nil
        await l.stop()
        listenerState = nil
        frames.clear()
        histories = [:]
    }

    fileprivate func listenerSnapshot(_ s: ListenerSnapshot) {
        guard listenerEngine != nil else { return }
        listenerState = s
        recordLevels(s)
    }

    fileprivate func listenerEvent(_ e: ListenerEventInfo) {
        if case let .forgotten(id) = e {
            frames.remove(id)
            histories[id] = nil
        }
        listenerEvents.send(e)
    }

    private func recordLevels(_ s: ListenerSnapshot) {
        let now = Int64(ProcessInfo.processInfo.systemUptime * 1000)
        var changed = false
        for t in s.stations {
            guard let level = t.levelDbfs, case .live = t.connection else { continue }
            var h = histories[t.id] ?? LevelHistory()
            let before = h.count
            h.add(nowMs: now, dbfs: level)
            changed = changed || h.count != before
            histories[t.id] = h
        }
        if changed { historyVersion = now }
    }

    /// Chart points of a station's last minute (age 0..1, dBFS).
    func history(_ stationId: String) -> [(x: Float, db: Float)] {
        guard var h = histories[stationId] else { return [] }
        let points = h.series(nowMs: Int64(ProcessInfo.processInfo.systemUptime * 1000))
        histories[stationId] = h
        return points
    }

    /// Runs a listener intent, ignoring "not connected" style failures.
    func withListener(_ f: (Listener) throws -> Void) {
        guard let l = listenerEngine else { return }
        do { try f(l) } catch { NSLog("Nestwatch: listener intent failed: %@", String(describing: type(of: error))) }
    }

    /// This device's ID, from whichever engine runs.
    var deviceId: String? { listenerEngine?.deviceId() ?? stationEngine?.deviceId() }
}

/// Decoded camera images per station, kept apart from the listener state
/// so that frames only redraw the camera view. Main thread, except
/// [offer].
final class VideoFrames: ObservableObject {
    @Published private(set) var images: [String: UIImage] = [:]
    private let decode = DispatchQueue(label: "nestwatch.jpeg", qos: .userInitiated)
    private var pending: Set<String> = []

    func offer(_ f: VideoFrame) {
        DispatchQueue.main.async {
            // Drop frames while the previous one of that station decodes.
            guard !self.pending.contains(f.stationId) else { return }
            self.pending.insert(f.stationId)
            self.decode.async {
                let img = UIImage(data: f.jpeg)
                DispatchQueue.main.async {
                    self.pending.remove(f.stationId)
                    if let img { self.images[f.stationId] = img }
                }
            }
        }
    }

    func remove(_ id: String) { images[id] = nil }
    func clear() { images = [:] }
}

/// Station observer: forwards to the main actor.
private final class StationBridge: StationObserver, @unchecked Sendable {
    weak var model: Nestwatch?
    init(model: Nestwatch) { self.model = model }

    func onState(state: StationSnapshot) {
        DispatchQueue.main.async { [weak model] in model?.stationState(state) }
    }

    func onEvent(event: StationEventInfo) {
        DispatchQueue.main.async { [weak model] in model?.stationEvent(event) }
    }
}

/// Listener observer: forwards to the main actor.
private final class ListenerBridge: ListenerObserver, @unchecked Sendable {
    weak var model: Nestwatch?
    private let frames: VideoFrames
    init(model: Nestwatch) {
        self.model = model
        frames = model.frames
    }

    func onState(state: ListenerSnapshot) {
        DispatchQueue.main.async { [weak model] in model?.listenerSnapshot(state) }
    }

    func onEvent(event: ListenerEventInfo) {
        DispatchQueue.main.async { [weak model] in model?.listenerEvent(event) }
    }

    func onVideoFrame(frame: VideoFrame) {
        frames.offer(frame)
    }
}
