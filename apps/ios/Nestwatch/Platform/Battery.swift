// SPDX-License-Identifier: AGPL-3.0-or-later
import UIKit

/// Reports the station's battery to listeners (FR-ST-6). `nil` when the
/// device does not know (simulator).
final class BatteryWatch {
    private var observers: [NSObjectProtocol] = []
    private let report: (UInt8?, Bool?) -> Void

    init(report: @escaping (UInt8?, Bool?) -> Void) { self.report = report }

    func start() {
        let d = UIDevice.current
        d.isBatteryMonitoringEnabled = true
        let nc = NotificationCenter.default
        for name in [UIDevice.batteryLevelDidChangeNotification, UIDevice.batteryStateDidChangeNotification] {
            observers.append(nc.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in self?.send() })
        }
        send()
    }

    private func send() {
        let d = UIDevice.current
        let level = d.batteryLevel
        let percent: UInt8? = level < 0 ? nil : UInt8((level * 100).rounded())
        let charging: Bool?
        switch d.batteryState {
        case .charging, .full: charging = true
        case .unplugged: charging = false
        default: charging = nil
        }
        report(percent, charging)
    }

    func stop() {
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        UIDevice.current.isBatteryMonitoringEnabled = false
    }
}

/// Station night mode (FR-ST-4, IOS-4): lowest backlight and no auto-lock
/// while dimmed; the previous brightness comes back when brightened or
/// when the app leaves the foreground.
enum KeepAwake {
    private static var saved: CGFloat?

    static func dim(_ on: Bool) {
        UIApplication.shared.isIdleTimerDisabled = on
        if on {
            if saved == nil { saved = UIScreen.main.brightness }
            UIScreen.main.brightness = 0.01
        } else if let b = saved {
            UIScreen.main.brightness = b
            saved = nil
        }
    }
}
