// SPDX-License-Identifier: AGPL-3.0-or-later
import Combine
import Foundation

/// Which role this installation plays (design/README.md §5, screen 1).
enum Role: String, CaseIterable {
    case none, station, listener
}

/// Colour theme: follow the system, or force night or day.
enum ThemeChoice: String, CaseIterable {
    case system, night, day
}

/// UI preferences; nothing secret, so plain user defaults. Keys and trust
/// stores live in the Keychain ([KeychainStorage]).
struct UiPrefs: Equatable {
    var role: Role = .none
    /// The medical disclaimer was acknowledged by choosing a role (UI-6).
    var disclaimerAccepted = false
    var theme: ThemeChoice = .system
    /// The station was running when the app last stopped.
    var stationWasRunning = false
    /// The station setup asked about the relay (rendezvous.md FR-RV-1).
    var relayAsked = false
}

/// Loads and stores [UiPrefs] and publishes changes.
final class Prefs: ObservableObject {
    private let defaults: UserDefaults
    @Published private(set) var value: UiPrefs

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        var p = UiPrefs()
        p.role = Role(rawValue: defaults.string(forKey: "role") ?? "") ?? .none
        p.disclaimerAccepted = defaults.bool(forKey: "disclaimer")
        p.theme = ThemeChoice(rawValue: defaults.string(forKey: "theme") ?? "") ?? .system
        p.stationWasRunning = defaults.bool(forKey: "station-running")
        p.relayAsked = defaults.bool(forKey: "relay-asked")
        value = p
    }

    func update(_ f: (inout UiPrefs) -> Void) {
        var p = value
        f(&p)
        guard p != value else { return }
        value = p
        defaults.set(p.role.rawValue, forKey: "role")
        defaults.set(p.disclaimerAccepted, forKey: "disclaimer")
        defaults.set(p.theme.rawValue, forKey: "theme")
        defaults.set(p.stationWasRunning, forKey: "station-running")
        defaults.set(p.relayAsked, forKey: "relay-asked")
    }
}
