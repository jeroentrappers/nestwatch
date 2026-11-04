// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI

@main
struct NestwatchApp: App {
    @Environment(\.scenePhase) private var phase
    @StateObject private var model = Nestwatch.shared
    @StateObject private var nav: Nav
    /// An invitation opened from a `nestwatch://invite` link (FR-PAIR-1).
    @State private var pendingInvite: String?

    init() {
        let role = Nestwatch.shared.prefs.value.role
        _nav = StateObject(wrappedValue: Nav(role == .station ? .station : role == .listener ? .listener : .welcome))
        Alerts.shared.install()
    }

    var body: some Scene {
        WindowGroup {
            RootView(pendingInvite: $pendingInvite)
                .environmentObject(model)
                .environmentObject(nav)
                .onOpenURL { url in
                    if let i = invitation(from: url) { pendingInvite = i }
                }
                .onAppear(perform: resume)
        }
        .onChange(of: phase) { p in
            StationHost.shared.setForeground(p != .background)
            if p == .active { Alerts.shared.refresh() }
        }
    }

    /// Keeps the engines running for the chosen role across launches.
    private func resume() {
        let p = model.prefs.value
        switch p.role {
        case .listener:
            startListening()
        case .station:
            if p.stationWasRunning && AudioSessionConfig.micGranted { StationHost.shared.start() }
        case .none:
            break
        }
    }
}
