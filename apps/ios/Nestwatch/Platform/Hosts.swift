// SPDX-License-Identifier: AGPL-3.0-or-later
import AVFoundation
import Combine
import UIKit

/// Runs a station on this device (FR-ST-1..8, platforms.md §2.3): the
/// audio session and microphone, talk-back playback, the on-demand camera
/// while in the foreground, the battery report and Bonjour advertising.
/// Audio keeps the app alive in the background (`UIBackgroundModes`
/// audio); the camera does not, so it reports `unavailable` there.
final class StationHost {
    static let shared = StationHost()

    private var audio: StationAudio?
    private var camera: StationCamera?
    private var battery: BatteryWatch?
    private let bonjour = BonjourAdvertiser()
    private var subs: Set<AnyCancellable> = []
    /// The camera request in force, kept across a trip to the background.
    private var wanted: CameraRequest?
    private var foreground = true
    private var starting = false

    private var model: Nestwatch { Nestwatch.shared }

    var running: Bool { audio != nil }

    /// Starts the station (the microphone permission was asked before).
    func start() {
        guard audio == nil, !starting else { return }
        starting = true
        model.prefs.update { $0.stationWasRunning = true }
        do { try AudioSessionConfig.station() } catch { NSLog("Nestwatch: station audio session: %@", String(describing: error)) }
        Task { @MainActor in
            defer { self.starting = false }
            guard let s = await self.model.startStation() else {
                AudioSessionConfig.deactivate()
                return
            }
            self.setUp(s)
        }
    }

    private func setUp(_ s: Station) {
        let a = StationAudio(station: s)
        a.start()
        if !a.hasMic { NSLog("Nestwatch: station runs without a microphone") }
        audio = a
        camera = StationCamera(sink: { img in _ = s.pushCameraFrame(image: img) }, onUnavailable: {
            s.setCameraAvailability(availability: .unavailable)
        })
        reportCamera(s)
        let b = BatteryWatch { p, c in s.setBattery(percent: p, charging: c) }
        b.start()
        battery = b
        model.$station
            .compactMap(\.snapshot)
            .sink { [weak self] snap in self?.bonjour.advertise(name: snap.name, port: snap.tcpPort, txt: snap.mdnsTxt) }
            .store(in: &subs)
        model.stationEvents
            .sink { [weak self] e in self?.onEvent(s, e) }
            .store(in: &subs)
    }

    private func reportCamera(_ s: Station) {
        let availability: CameraAvailability
        if !StationCamera.hasCamera || !foreground {
            availability = .unavailable
        } else {
            switch StationCamera.authorization {
            case .authorized: availability = .available
            case .notDetermined: availability = .available
            default: availability = .denied
            }
        }
        s.setCameraAvailability(availability: availability)
    }

    private func onEvent(_ s: Station, _ e: StationEventInfo) {
        switch e {
        case let .cameraWanted(req):
            wanted = req
            startCamera(s)
        case .cameraNotWanted:
            wanted = nil
            camera?.stop()
        default:
            break
        }
    }

    private func startCamera(_ s: Station) {
        guard let req = wanted, foreground else { return }
        switch StationCamera.authorization {
        case .authorized:
            camera?.start(req, orientation: AVCaptureVideoOrientation(Self.interfaceOrientation))
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { _ in
                DispatchQueue.main.async {
                    self.reportCamera(s)
                    if StationCamera.authorization == .authorized { self.startCamera(s) }
                }
            }
        default:
            s.setCameraAvailability(availability: .denied)
        }
    }

    private static var interfaceOrientation: UIInterfaceOrientation {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        return scene?.interfaceOrientation ?? .portrait
    }

    /// The app moved to or from the foreground: the camera stops in the
    /// background (iOS rule) and resumes on return.
    func setForeground(_ fg: Bool) {
        guard foreground != fg else { return }
        foreground = fg
        guard let s = model.stationEngine else { return }
        if !fg { camera?.stop() }
        reportCamera(s)
        if fg { startCamera(s) }
    }

    func stop() {
        model.prefs.update { $0.stationWasRunning = false }
        subs = []
        audio?.stop()
        audio = nil
        camera?.stop()
        camera = nil
        battery?.stop()
        battery = nil
        wanted = nil
        bonjour.stop()
        Task { @MainActor in
            await self.model.stopStation()
            AudioSessionConfig.deactivate()
        }
    }
}

/// Keeps a listener receiving and alerting (FR-LS-4, FR-AL-1..4): plays the
/// mix (which keeps the app alive in the background), captures talk-back
/// while granted, browses Bonjour, follows network changes and turns
/// alerts into notifications.
final class ListenerHost {
    static let shared = ListenerHost()

    private var playback: ListenerPlayback?
    private var talk: TalkCapture?
    private var browser: BonjourBrowser?
    private var network: NetworkWatch?
    private var subs: Set<AnyCancellable> = []

    private var model: Nestwatch { Nestwatch.shared }

    var running: Bool { playback != nil }

    /// Starts the listener engine and its platform side.
    func start() {
        guard playback == nil, let l = model.listener() else { return }
        do { try AudioSessionConfig.listener() } catch { NSLog("Nestwatch: listener audio session: %@", String(describing: error)) }
        let p = ListenerPlayback(listener: l)
        p.start()
        playback = p
        let b = BonjourBrowser(
            onFound: { s in try? l.addDiscovered(service: s) },
            onLost: { name in l.removeDiscovered(fullname: name) }
        )
        b.start()
        browser = b
        let n = NetworkWatch { l.networkChanged() }
        n.start()
        network = n
        model.listenerEvents
            .sink { [weak self] e in self?.onEvent(l, e) }
            .store(in: &subs)
    }

    private func stationName(_ id: String) -> String {
        let n = model.listenerState?.stations.first { $0.id == id }?.name ?? ""
        return n.isEmpty ? tr("unnamed") : n
    }

    private func onEvent(_ l: Listener, _ e: ListenerEventInfo) {
        switch e {
        case let .alert(id, kind):
            alert(id, kind)
        case .microphoneWanted:
            guard talk == nil, AudioSessionConfig.micGranted else { return }
            let t = TalkCapture(listener: l)
            if !t.start() { NSLog("Nestwatch: no microphone for talk-back") }
            talk = t
        case .microphoneNotWanted:
            talk?.stop()
            talk = nil
        default:
            break
        }
    }

    private func alert(_ id: String, _ kind: AlertKind) {
        let name = stationName(id)
        switch kind {
        case .soundStarted:
            Alerts.shared.post(stationId: id, slot: "sound", title: tr("alert_sound", name), body: tr("alert_body"), urgent: true)
        case .unreachable:
            Alerts.shared.post(stationId: id, slot: "connection", title: tr("alert_unreachable", name), body: tr("alert_body"), urgent: true)
        case .reachable:
            Alerts.shared.post(stationId: id, slot: "connection", title: tr("alert_reachable", name), body: tr("alert_body"), urgent: false)
        case let .batteryLow(percent):
            Alerts.shared.post(stationId: id, slot: "battery", title: tr("alert_battery", name, Int(percent)), body: tr("alert_body"), urgent: false)
        }
    }

    func stop() {
        subs = []
        playback?.stop()
        playback = nil
        talk?.stop()
        talk = nil
        browser?.stop()
        browser = nil
        network?.stop()
        network = nil
        Task { @MainActor in
            await self.model.stopListener()
            AudioSessionConfig.deactivate()
        }
    }
}
