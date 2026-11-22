// SPDX-License-Identifier: AGPL-3.0-or-later
import AudioToolbox
import UserNotifications

/// Listener alerts as local notifications (FR-AL-1..4, platforms.md §2.3):
/// time-sensitive so they break through a Focus, with sound. Without
/// permission, sound and connection alerts still play a system sound and
/// vibrate while the app runs (IOS-5). Sound alerts and the alarm carry
/// "I'm on it" and "Not the baby"; tapping one acknowledges (FR-AL-6, FR-AL-7).
final class Alerts: NSObject, ObservableObject, UNUserNotificationCenterDelegate {
    static let shared = Alerts()
    private let center = UNUserNotificationCenter.current()
    @Published private(set) var authorized: Bool?

    /// The category of sound alerts and the alarm, with their actions.
    static let soundCategory = "nestwatch.sound"
    static let ackAction = "nestwatch.ack"
    static let notTheBabyAction = "nestwatch.not-the-baby"

    func install() {
        center.delegate = self
        let ack = UNNotificationAction(identifier: Self.ackAction, title: tr("im_on_it"), options: [])
        let notBaby = UNNotificationAction(identifier: Self.notTheBabyAction, title: tr("not_the_baby"), options: [])
        center.setNotificationCategories([
            UNNotificationCategory(identifier: Self.soundCategory, actions: [ack, notBaby], intentIdentifiers: [], options: []),
        ])
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
    /// `actions`: a sound alert or alarm, with "I'm on it" and "Not the baby".
    func post(stationId: String, slot: String, title: String, body: String, urgent: Bool, actions: Bool = false) {
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
            c.userInfo = ["station": stationId, "slot": slot]
            if actions { c.categoryIdentifier = Self.soundCategory }
            let r = UNNotificationRequest(identifier: "\(stationId).\(slot)", content: c, trigger: nil)
            self.center.add(r)
        }
    }

    /// Removes a station's delivered sound alert and alarm (acknowledged).
    func clearSound(stationId: String) {
        center.removeDeliveredNotifications(withIdentifiers: ["\(stationId).sound", "\(stationId).alarm"])
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

    /// The actions and taps of sound alerts and alarms: "I'm on it" and a tap
    /// on the notification acknowledge, "Not the baby" pauses (FR-AL-6,
    /// FR-AL-7). Other notifications only open the app.
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler done: @escaping () -> Void
    ) {
        let info = response.notification.request.content.userInfo
        guard response.notification.request.content.categoryIdentifier == Self.soundCategory,
              let id = info["station"] as? String else { return done() }
        let action = response.actionIdentifier
        DispatchQueue.main.async {
            switch action {
            case Self.notTheBabyAction: ListenerHost.shared.notTheBaby(id)
            case Self.ackAction, UNNotificationDefaultActionIdentifier: ListenerHost.shared.acknowledge(id)
            default: break
            }
            done()
        }
    }
}
