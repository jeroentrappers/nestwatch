// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI

/// The screens of design/README.md §5.
enum Screen: Hashable {
    case welcome
    case station
    case stationSettings
    case listener
    case stationPrefs(String)
    case camera(String)
    case addStation(String?)
    case about
}

/// A simple back stack (platform conventions: a back button in the header;
/// on iPad the listener keeps a sidebar next to it).
final class Nav: ObservableObject {
    @Published private(set) var stack: [Screen]

    init(_ start: Screen) { stack = [start] }

    var top: Screen { stack.last ?? .welcome }

    func push(_ s: Screen) { stack.append(s) }

    func pop() {
        if stack.count > 1 { stack.removeLast() }
    }

    func reset(_ s: Screen) { stack = [s] }

    /// Shows `s` directly above the role's home screen (sidebar choices).
    func show(_ s: Screen) {
        let home = stack.first ?? .listener
        stack = s == home ? [home] : [home, s]
    }
}

/// Opens the app's page in the Settings app (after a denial).
func openAppSettings() {
    if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
}

/// Starts the listener for the listener role.
func startListening() {
    ListenerHost.shared.start()
    Alerts.shared.request()
}

struct RootView: View {
    @EnvironmentObject private var model: Nestwatch
    @EnvironmentObject private var nav: Nav
    @ObservedObject private var prefs = Nestwatch.shared.prefs
    @Environment(\.colorScheme) private var scheme
    @Environment(\.horizontalSizeClass) private var hsize
    @Environment(\.scenePhase) private var phase
    @Binding var pendingInvite: String?
    @State private var dimmed = false

    var body: some View {
        let palette = Palette.resolve(prefs.value.theme, system: scheme, dimmed: dimmed)
        content
            .environment(\.palette, palette)
            .background(palette.ink0.ignoresSafeArea())
            .preferredColorScheme(prefs.value.theme == .system && !dimmed ? nil : (palette.dark ? .dark : .light))
            .onChange(of: dimmed) { KeepAwake.dim($0) }
            // The brightness is the system's: give it back outside the app.
            .onChange(of: phase) { KeepAwake.dim($0 == .active && dimmed) }
            .onAppear {
                takeInvite()
                #if DEBUG
                debugScreen()
                #endif
            }
            .onChange(of: pendingInvite) { _ in takeInvite() }
            .onChange(of: prefs.value.role) { _ in takeInvite() }
            .statusBar(hidden: dimmed)
    }

    #if DEBUG
    /// Opens a screen named by `NESTWATCH_DEBUG_SCREEN` (simulator
    /// screenshots; debug builds only): about, stationSettings, addStation,
    /// pairing, dim, stationPrefs, camera (the last two use the first station).
    private func debugScreen() {
        guard let name = ProcessInfo.processInfo.environment["NESTWATCH_DEBUG_SCREEN"] else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            let first = model.listenerState?.stations.first?.id
            switch name {
            case "about": nav.push(.about)
            case "stationSettings": nav.push(.stationSettings)
            case "addStation": nav.show(.addStation(nil))
            case "pairing": model.debugPairing = true
            case "dim": dimmed = true
            case "stationPrefs": if let first { nav.show(.stationPrefs(first)) }
            case "camera": if let first { nav.push(.camera(first)) }
            default: break
            }
        }
    }
    #endif

    private func takeInvite() {
        guard let i = pendingInvite, prefs.value.role == .listener else { return }
        pendingInvite = nil
        nav.show(.addStation(i))
    }

    @ViewBuilder private var content: some View {
        if prefs.value.role == .listener && hsize == .regular {
            HStack(spacing: 0) {
                ListenerSidebar()
                    .frame(width: 320)
                Divider()
                screen(nav.top, wide: true)
                    .frame(maxWidth: .infinity)
            }
        } else {
            screen(nav.top, wide: hsize == .regular)
        }
    }

    @ViewBuilder private func screen(_ s: Screen, wide: Bool) -> some View {
        switch s {
        case .welcome:
            WelcomeScreen { role in
                prefs.update {
                    $0.role = role
                    $0.disclaimerAccepted = true
                }
                if role == .listener { startListening() }
                nav.reset(role == .station ? .station : .listener)
            }
        case .station:
            StationHome(dimmed: $dimmed, wide: wide)
        case .stationSettings:
            StationSettingsScreen(onSwitchRole: {
                StationHost.shared.stop()
                prefs.update { $0.role = .none }
                nav.reset(.welcome)
            })
        case .listener:
            ListenerHome(wide: wide, onSwitchRole: switchFromListener)
        case let .stationPrefs(id):
            ListenerStationSettings(id: id)
        case let .camera(id):
            CameraScreen(id: id)
        case let .addStation(invite):
            AddStationScreen(initialInvite: invite)
        case .about:
            AboutScreen()
        }
    }

    private func switchFromListener() {
        ListenerHost.shared.stop()
        prefs.update { $0.role = .none }
        nav.reset(.welcome)
    }
}

/// A screen with a title row, scrolling content and safe-area padding.
struct Page<Actions: View, Content: View>: View {
    let title: String
    var back: Bool
    var maxWidth: CGFloat
    @ViewBuilder let actions: () -> Actions
    @ViewBuilder let content: () -> Content
    @EnvironmentObject private var nav: Nav
    @Environment(\.palette) private var c

    init(
        _ title: String,
        back: Bool = true,
        maxWidth: CGFloat = 760,
        @ViewBuilder actions: @escaping () -> Actions = { EmptyView() },
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.title = title
        self.back = back
        self.maxWidth = maxWidth
        self.actions = actions
        self.content = content
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 4) {
                if back && nav.stack.count > 1 {
                    Button { nav.pop() } label: {
                        HStack(spacing: 2) {
                            Image(systemName: "chevron.left")
                            Text(tr("back"))
                        }
                        .font(NWFont.body)
                        .foregroundColor(c.text2)
                        .frame(minWidth: 44, minHeight: 44)
                    }
                    .buttonStyle(.plain)
                }
                Text(title).font(NWFont.title).foregroundColor(c.text1).lineLimit(1)
                    .accessibilityAddTraits(.isHeader)
                    .padding(.horizontal, 4)
                Spacer(minLength: 8)
                actions()
            }
            .padding(.horizontal, 12)
            .frame(minHeight: 56)
            ScrollView {
                VStack(alignment: .leading, spacing: 16, content: content)
                    .frame(maxWidth: maxWidth)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 24)
                    .frame(maxWidth: .infinity)
            }
        }
        .background(c.ink0.ignoresSafeArea())
    }
}

/// Theme choice: system, night or day.
struct ThemeCard: View {
    @ObservedObject private var prefs = Nestwatch.shared.prefs

    var body: some View {
        Card {
            SectionTitle(tr("theme"))
            Segmented(
                options: [(ThemeChoice.system, tr("theme_system")), (.night, tr("theme_night")), (.day, tr("theme_day"))],
                selected: prefs.value.theme
            ) { t in prefs.update { $0.theme = t } }
        }
    }
}

/// "Change role": stops what this device does and shows the welcome screen.
struct ChangeRoleCard: View {
    let onSwitchRole: () -> Void
    @State private var confirm = false

    var body: some View {
        Card {
            SectionTitle(tr("change_role"))
            Caption(tr("change_role_hint"))
            DangerButton(tr("change_role")) { confirm = true }
        }
        .alert(tr("change_role"), isPresented: $confirm) {
            Button(tr("change_role"), role: .destructive, action: onSwitchRole)
            Button(tr("cancel"), role: .cancel) {}
        } message: {
            Text(tr("change_role_hint"))
        }
    }
}
