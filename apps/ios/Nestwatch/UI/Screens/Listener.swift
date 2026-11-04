// SPDX-License-Identifier: AGPL-3.0-or-later
import AVFoundation
import SwiftUI

/// Listener home (README §5.6): stacked station tiles on a phone, a grid
/// next to the sidebar on an iPad.
struct ListenerHome: View {
    let wide: Bool
    let onSwitchRole: () -> Void
    @EnvironmentObject private var model: Nestwatch
    @EnvironmentObject private var nav: Nav
    @ObservedObject private var alerts = Alerts.shared
    @Environment(\.palette) private var c
    @State private var menu = false

    var body: some View {
        let stations = model.listenerState?.stations ?? []
        Page(wide ? tr("all_stations") : tr("app_name"), back: false, maxWidth: wide ? 1200 : 760) {
            if !wide {
                IconButton("info.circle", label: tr("about")) { nav.push(.about) }
                IconButton("gearshape", label: tr("settings")) { menu.toggle() }
            }
        } content: {
            if menu && !wide {
                ThemeCard()
                ChangeRoleCard(onSwitchRole: onSwitchRole)
            }
            if stations.isEmpty {
                Card { Text(tr("no_stations")).font(NWFont.body).foregroundColor(c.text2) }
            }
            if wide {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 340), spacing: 16, alignment: .top)], spacing: 16) {
                    ForEach(stations, id: \.id) { t in StationTileView(t: t, solo: model.listenerState?.solo) }
                }
            } else {
                ForEach(stations, id: \.id) { t in StationTileView(t: t, solo: model.listenerState?.solo) }
            }
            PrimaryButton(tr("add_station")) { nav.push(.addStation(nil)) }
            if !stations.isEmpty { Caption(tr("keeps_listening")) }
            if alerts.authorized == false {
                Card {
                    Text(tr("perm_denied_notifications")).font(NWFont.bodySmall).foregroundColor(c.amber)
                    SecondaryButton(tr("open_settings"), action: openAppSettings)
                }
            }
        }
        .onAppear {
            if !ListenerHost.shared.running { startListening() }
            Alerts.shared.refresh()
        }
    }
}

/// The iPad sidebar: stations with their status, add, settings and about.
struct ListenerSidebar: View {
    @EnvironmentObject private var model: Nestwatch
    @EnvironmentObject private var nav: Nav
    @ObservedObject private var prefs = Nestwatch.shared.prefs
    @Environment(\.palette) private var c

    var body: some View {
        let stations = model.listenerState?.stations ?? []
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 10) {
                    Logo(size: 32)
                    Text(tr("app_name")).font(NWFont.title).foregroundColor(c.text1).accessibilityAddTraits(.isHeader)
                }
                .padding(.vertical, 12)
                row(tr("all_stations"), selected: nav.top == .listener) { nav.show(.listener) }
                Caption(tr("stations")).padding(.top, 8)
                ForEach(stations, id: \.id) { t in
                    let st = tileStatus(t)
                    Button { nav.show(.stationPrefs(t.id)) } label: {
                        HStack {
                            Text(t.name.isEmpty ? tr("unnamed") : t.name).font(NWFont.body).foregroundColor(c.text1).lineLimit(1)
                            Spacer()
                            Pill(text: st.text, kind: st.kind)
                        }
                        .padding(.horizontal, 12)
                        .frame(minHeight: 48)
                        .background(RoundedRectangle(cornerRadius: Radius.medium).fill(nav.top == .stationPrefs(t.id) ? c.lavenderSoft : Color.clear))
                    }
                    .buttonStyle(.plain)
                }
                row(tr("add_station"), selected: nav.top == .addStation(nil), accent: true) { nav.show(.addStation(nil)) }
                Divider().padding(.vertical, 8)
                ThemeCard()
                ChangeRoleCard {
                    ListenerHost.shared.stop()
                    prefs.update { $0.role = .none }
                    nav.reset(.welcome)
                }
                row(tr("about"), selected: nav.top == .about) { nav.show(.about) }
            }
            .padding(16)
        }
        .background(c.ink1.ignoresSafeArea())
    }

    private func row(_ title: String, selected: Bool, accent: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title).font(NWFont.body).foregroundColor(accent ? c.honey : c.text1)
                .padding(.horizontal, 12)
                .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: Radius.medium).fill(selected ? c.lavenderSoft : Color.clear))
        }
        .buttonStyle(.plain)
    }
}

/// One station tile: name and status, level meter with this listener's
/// threshold, details, talk-back and the per-station actions.
struct StationTileView: View {
    let t: StationTile
    let solo: String?
    @EnvironmentObject private var model: Nestwatch
    @EnvironmentObject private var nav: Nav
    @Environment(\.palette) private var c
    @State private var talkRequested = false
    @State private var askMic = false

    var body: some View {
        let status = tileStatus(t)
        let live: Bool = { if case .live = t.connection { return true }; return false }()
        let sound = tileHasSound(t)
        let activated = t.prefs.listen.mode == .activated
        let thr = t.prefs.listen.thresholdDbfs
        Card(alert: sound) {
            HStack {
                Text(t.name.isEmpty ? tr("unnamed") : t.name).font(NWFont.subtitle).foregroundColor(c.text1).lineLimit(2)
                Spacer()
                Pill(text: status.text, kind: status.kind)
            }
            let levelText = t.levelDbfs.map { tr(live ? "level_now" : "level_last", "\(Int($0.rounded())) dB") } ?? tr("no_level")
            LevelMeter(levelDb: live ? t.levelDbfs : nil, thresholdDb: activated ? thr : nil, sound: sound)
                .accessibilityElement()
                .accessibilityLabel(Text("\(tr("ring_label")): \(levelText)"))
            Caption(details(levelText: levelText, thr: thr, activated: activated))
            connectionHint
            if live && t.canTalk {
                let enabled: Bool = {
                    switch t.talk {
                    case .disabled, .busy: return false
                    default: return true
                    }
                }()
                let granted: Bool = { if case .granted = t.talk { return true }; return false }()
                HoldToTalk(active: granted || talkRequested, enabled: enabled, label: talkLabel(t.talk)) { on in
                    if on && !AudioSessionConfig.micGranted {
                        askMic = true
                        return
                    }
                    talkRequested = on
                    model.withListener { try $0.talk(stationId: t.id, on: on) }
                }
            }
            HStack(spacing: 8) {
                let muted = t.prefs.muted
                SecondaryButton(tr(muted ? "unmute" : "mute")) { model.withListener { try $0.setMuted(stationId: t.id, muted: !muted) } }
                let isSolo = solo == t.id
                SecondaryButton(tr(isSolo ? "unsolo" : "solo")) { model.withListener { try $0.setSolo(stationId: isSolo ? nil : t.id) } }
                if live && t.canVideo {
                    SecondaryButton(tr("camera")) { nav.push(.camera(t.id)) }
                }
            }
            LinkButton(tr("station_settings_of", t.name), color: c.lavender) { nav.show(.stationPrefs(t.id)) }
        }
        .alert(tr("perm_mic_title"), isPresented: $askMic) {
            Button(tr("perm_continue")) {
                if AVAudioSession.sharedInstance().recordPermission == .denied { openAppSettings() } else { AudioSessionConfig.requestMic { _ in } }
            }
            Button(tr("perm_not_now"), role: .cancel) {}
        } message: {
            Text(tr("perm_mic_talk"))
        }
    }

    private func details(levelText: String, thr: Float, activated: Bool) -> String {
        var bits = [levelText, activated ? tr("opens_above", "\(Int(thr.rounded())) dB") : tr("mode_always_on")]
        if let b = t.battery { bits.append(t.charging == true ? tr("battery_charging", Int(b)) : tr("battery", Int(b))) }
        if let p = t.path { bits.append(pathLabel(p)) }
        if t.cameraOn { bits.append(tr("camera_on")) }
        return bits.joined(separator: " · ")
    }

    @ViewBuilder private var connectionHint: some View {
        switch t.connection {
        case let .reconnecting(attempt, tried):
            Caption(tr("attempt", Int(attempt)) + " · " + (tried.isEmpty ? tr("trying") : trp("trying_addresses", tried.count)), color: c.amber)
        case let .connecting(_, tried):
            Caption(tried.isEmpty ? tr("trying") : trp("trying_addresses", tried.count))
        case .needsPairing:
            Caption(tr("needs_pairing_hint"), color: c.amber)
        case .idle:
            Caption(tr("paused_hint"))
            SecondaryButton(tr("resume")) { model.withListener { try $0.follow(stationId: t.id) } }
        case .live:
            EmptyView()
        }
    }
}

private let holdChoices: [UInt32] = [2_000, 4_000, 8_000, 15_000, 30_000]

/// Station settings on the listener (README §5.7, FR-AU-2, FR-AU-6).
struct ListenerStationSettings: View {
    let id: String
    @EnvironmentObject private var model: Nestwatch
    @EnvironmentObject private var nav: Nav
    @Environment(\.palette) private var c
    @State private var threshold: Float = -50
    @State private var editingThreshold = false
    @State private var volume: Float = 1
    @State private var editingVolume = false
    @State private var address = ""
    @State private var added = false
    @State private var forgetting = false

    var body: some View {
        if let t = model.listenerState?.stations.first(where: { $0.id == id }) {
            content(t)
        } else {
            Page(tr("settings")) { EmptyView() }
        }
    }

    private func setListen(_ f: (inout ListenSettingsInfo) -> Void, _ t: StationTile) {
        var l = t.prefs.listen
        f(&l)
        model.withListener { try $0.setListen(stationId: id, listen: l) }
    }

    private func content(_ t: StationTile) -> some View {
        let listen = t.prefs.listen
        let activated = listen.mode == .activated
        let _ = model.historyVersion
        return Page(tr("station_settings_of", t.name)) {
            Card {
                SectionTitle(tr("listen_mode"))
                Segmented(options: [(ListenMode.activated, tr("mode_sound")), (.continuous, tr("mode_always"))], selected: listen.mode) { m in
                    setListen({ $0.mode = m }, t)
                }
                CalibrationChart(points: model.history(id), thresholdDb: threshold, activated: activated) { db in
                    threshold = clampThreshold(db)
                    setListen({ $0.thresholdDbfs = threshold }, t)
                }
                Caption(tr(activated ? "calibrate_hint" : "calibrate_continuous"))
                HStack {
                    Text(tr("threshold")).font(NWFont.body).foregroundColor(c.text1)
                    Spacer()
                    Text(tr("threshold_value", Int(threshold))).font(NWFont.body).foregroundColor(c.honey)
                }
                Slider(value: $threshold, in: thresholdMin...thresholdMax, step: 1) { editing in
                    editingThreshold = editing
                    if !editing { setListen({ $0.thresholdDbfs = clampThreshold(threshold) }, t) }
                }
                .tint(c.honey)
                .accessibilityLabel(Text(tr("threshold")))
                .accessibilityValue(Text(tr("threshold_value", Int(threshold))))
                if activated {
                    Text(tr("hold_time")).font(NWFont.body).foregroundColor(c.text1)
                    let nearest = holdChoices.min { abs(Int($0) - Int(listen.holdMs)) < abs(Int($1) - Int(listen.holdMs)) } ?? 8_000
                    Segmented(options: holdChoices.map { ($0, "\($0 / 1000) s") }, selected: nearest) { ms in
                        setListen({ $0.holdMs = ms }, t)
                    }
                    Caption(trp("seconds", Int(listen.holdMs / 1000)))
                }
            }
            Card {
                HStack {
                    Text(tr("volume")).font(NWFont.body).foregroundColor(c.text1)
                    Spacer()
                    Text(tr("percent", Int((volume * 100).rounded()))).font(NWFont.body).foregroundColor(c.text2)
                }
                Slider(value: $volume, in: 0...1) { editing in
                    editingVolume = editing
                    if !editing { model.withListener { try $0.setVolume(stationId: id, volume: volume) } }
                }
                .tint(c.lavender)
                .accessibilityLabel(Text(tr("volume")))
                SwitchRow(label: tr("mute"), isOn: t.prefs.muted) { m in model.withListener { try $0.setMuted(stationId: id, muted: m) } }
                let a = t.prefs.alerts
                SwitchRow(label: tr("alert_sound_toggle"), isOn: a.sound) { v in setAlerts(AlertSettings(sound: v, unreachable: a.unreachable, battery: a.battery)) }
                SwitchRow(label: tr("alert_unreachable_toggle"), isOn: a.unreachable) { v in setAlerts(AlertSettings(sound: a.sound, unreachable: v, battery: a.battery)) }
                SwitchRow(label: tr("alert_battery_toggle"), isOn: a.battery) { v in setAlerts(AlertSettings(sound: a.sound, unreachable: a.unreachable, battery: v)) }
                Caption(tr("alert_time_sensitive"))
            }
            Card {
                if case .live = t.connection, let r = t.route { Caption(tr("connected_via", r)) }
                let trimmed = address.trimmingCharacters(in: .whitespaces)
                let valid = !trimmed.isEmpty && isValidRoute(input: trimmed)
                Field(
                    label: tr("address_field"),
                    text: $address,
                    hint: !trimmed.isEmpty && !valid ? tr("err_invalid_address") : tr("address_hint"),
                    error: !trimmed.isEmpty && !valid,
                    keyboard: .URL
                )
                .onChange(of: address) { _ in added = false }
                SecondaryButton(tr("add_address"), enabled: valid) {
                    model.withListener { try $0.addRoute(stationId: id, route: trimmed) }
                    address = ""
                    added = true
                }
                if added { Caption(tr("address_added"), color: c.sage) }
            }
            HStack(spacing: 12) {
                if t.prefs.follow {
                    SecondaryButton(tr("pause")) { model.withListener { try $0.unfollow(stationId: id) } }
                } else {
                    SecondaryButton(tr("resume")) { model.withListener { try $0.follow(stationId: id) } }
                }
                DangerButton(tr("forget")) { forgetting = true }
            }
        }
        .onAppear {
            threshold = listen.thresholdDbfs
            volume = t.prefs.volume
        }
        .onChange(of: listen.thresholdDbfs) { v in if !editingThreshold { threshold = v } }
        .onChange(of: t.prefs.volume) { v in if !editingVolume { volume = v } }
        .alert(tr("forget"), isPresented: $forgetting) {
            Button(tr("forget_yes"), role: .destructive) {
                model.withListener { try $0.forget(stationId: id) }
                nav.show(.listener)
            }
            Button(tr("cancel"), role: .cancel) {}
        } message: {
            Text(tr("forget_confirm"))
        }
    }

    private func setAlerts(_ a: AlertSettings) {
        model.withListener { try $0.setAlerts(stationId: id, alerts: a) }
    }
}
