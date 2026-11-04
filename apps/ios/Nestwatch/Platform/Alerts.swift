// SPDX-License-Identifier: AGPL-3.0-or-later
import AudioToolbox
import UserNotifications

/// Listener alerts as local notifications (FR-AL-1..4, platforms.md §2.3):
/// time-sensitive so they break through a Focus, with sound. Without
/// permission, sound and connection alerts still play a system sound and
/// vibrate while the app runs (IOS-5).
final class Alerts: NSObject, ObservableObject, UNUserNotificationCenterDelegate {
    static let shared = Alerts()
    private let center = UNUserNotificationCenter.current()
    @Published private(set) var authorized: Bool?

    func install() {
        center.delegate = self
        refresh()
    }

    func refresh() {
        center.getNotificationSettings { s in
            let ok = s.authorizationStatus == .authorized || s.authorizationStatus == .provisional
                || s.authorizationStatus == .ephemeral
            let undecided = s.authorizationStatus == .notDetermined
            DispatchQueue.main.async { self.authorized = undecided ? nil : ok }
        }
    }

    /// Asks for permission once; later calls only refresh the state.
    func request() {
        center.requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in self.refresh() }
    }

    /// Kinds share one notification slot per station, so a newer alert
    /// replaces an older one of the same kind.
    func post(stationId: String, slot: String, title: String, body: String, urgent: Bool) {
        center.getNotificationSettings { s in
            guard s.authorizationStatus == .authorized || s.authorizationStatus == .provisional
                || s.authorizationStatus == .ephemeral else {
                if urgent { DispatchQueue.main.async { Alerts.fallbackSound() } }
                return
            }
            let c = UNMutableNotificationContent()
            c.title = title
            c.body = body
            c.sound = .default
            c.threadIdentifier = stationId
            c.interruptionLevel = urgent ? .timeSensitive : .active
            let r = UNNotificationRequest(identifier: "\(stationId).\(slot)", content: c, trigger: nil)
            self.center.add(r)
        }
    }

    /// A short alert sound with vibration (FR-AL-4: also while muted).
    static func fallbackSound() {
        AudioServicesPlayAlertSound(SystemSoundID(1005))
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        done([.banner, .list, .sound])
    }
}
