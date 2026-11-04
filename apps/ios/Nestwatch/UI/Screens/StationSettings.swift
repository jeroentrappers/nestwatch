// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI

/// Station settings (FR-ST-7, FR-ST-8, FR-TB-3, FR-AU-5) and app settings.
struct StationSettingsScreen: View {
    let onSwitchRole: () -> Void
    @EnvironmentObject private var model: Nestwatch
    @Environment(\.palette) private var c

    var body: some View {
        Page(tr("station_settings")) {
            if let s = model.station.snapshot {
                StationCard(s: s)
                RelayCard(s: s)
                TrustedListeners()
                if !s.endpoints.isEmpty {
                    Card {
                        SectionTitle(tr("addresses"))
                        ForEach(s.endpoints, id: \.self) { e in
                            Text(e).font(NWFont.mono(13)).foregroundColor(c.text2).textSelection(.enabled)
                        }
                    }
                }
            }
            ThemeCard()
            ChangeRoleCard(onSwitchRole: onSwitchRole)
        }
    }
}

private struct StationCard: View {
    let s: StationSnapshot
    @EnvironmentObject private var model: Nestwatch
    @Environment(\.palette) private var c
    @State private var name = ""
    @State private var volume: Float = 1
    @State private var editingVolume = false

    var body: some View {
        Card {
            Field(label: tr("station_name"), text: $name, onSubmit: rename)
            if name.trimmingCharacters(in: .whitespaces) != s.name && !name.trimmingCharacters(in: .whitespaces).isEmpty {
                SecondaryButton(tr("save"), action: rename)
            }
            SwitchRow(label: tr("camera_allowed"), isOn: s.settings.cameraEnabled) { on in
                model.withStation { $0.setCameraEnabled(enabled: on) }
            }
            switch s.cameraAvailability {
            case .unavailable:
                Caption(tr(StationCamera.hasCamera ? "camera_background_here" : "camera_unavailable_here"))
            case .denied:
                Caption(tr("camera_denied_here"), color: c.amber)
                LinkButton(tr("open_settings"), color: c.lavender, action: openAppSettings)
            case .available:
                EmptyView()
            }
            SwitchRow(label: tr("talk_allowed"), isOn: s.settings.talkEnabled) { on in
                model.withStation { $0.setTalkEnabled(enabled: on) }
            }
            Text(tr("talkback_volume")).font(NWFont.body).foregroundColor(c.text1)
            Slider(value: $volume, in: 0...1) { editing in
                editingVolume = editing
                if !editing { model.withStation { $0.setTalkbackVolume(volume: volume) } }
            }
            .tint(c.lavender)
            .accessibilityValue(Text(tr("percent", Int((volume * 100).rounded()))))
            Caption(tr("percent", Int((volume * 100).rounded())))
            Text(tr("mic_sensitivity")).font(NWFont.body).foregroundColor(c.text1)
            Segmented(
                options: [(Gain.auto, tr("gain_auto")), (.db0, "0 dB"), (.db6, "+6"), (.db12, "+12"), (.db18, "+18")],
                selected: s.settings.gain
            ) { g in model.withStation { $0.setGain(gain: g) } }
            if let b = s.battery { Caption(tr("battery_here", Int(b))) }
        }
        .onAppear {
            name = s.name
            volume = s.settings.talkbackVolume
        }
        .onChange(of: s.settings.talkbackVolume) { v in if !editingVolume { volume = v } }
        .onChange(of: s.name) { name = $0 }
    }

    private func rename() {
        let n = name.trimmingCharacters(in: .whitespaces)
        guard !n.isEmpty, n != s.name else { return }
        model.withStation { try $0.rename(name: n) }
    }
}

/// Relay settings (FR-RV-1, FR-RV-6): the toggle with the explanation, the
/// server address, the status and "reset relay identity".
private struct RelayCard: View {
    let s: StationSnapshot
    @EnvironmentObject private var model: Nestwatch
    @Environment(\.palette) private var c
    @State private var server = ""
    @State private var error = false
    @State private var confirmReset = false

    var body: some View {
        Card {
            SwitchRow(label: tr("relay_title"), isOn: s.settings.relayEnabled) { on in
                model.withStation { try $0.setRelayEnabled(enabled: on) }
            }
            Caption(relayExplanation(s.settings.relayServer), color: c.text2)
            let look = relayLook(s.relay)
            Pill(text: tr("relay_pill", relayLabel(look)), kind: relayPillKind(look))
            if case let .error(message, _) = s.relay { Caption(message, color: c.amber) }
            Field(
                label: tr("relay_server"),
                text: $server,
                hint: tr(error ? "relay_server_invalid" : "relay_server_hint"),
                error: error,
                keyboard: .URL,
                onSubmit: save
            )
            .onChange(of: server) { _ in error = false }
            if server.trimmingCharacters(in: .whitespaces) != s.settings.relayServer && !server.isEmpty {
                SecondaryButton(tr("save"), action: save)
            }
            LinkButton(tr("relay_reset"), color: c.coral) { confirmReset = true }
        }
        .onAppear { server = s.settings.relayServer }
        .onChange(of: s.settings.relayServer) { server = $0 }
        .alert(tr("relay_reset"), isPresented: $confirmReset) {
            Button(tr("relay_reset_do"), role: .destructive) { model.withStation { try $0.resetRelayIdentity() } }
            Button(tr("cancel"), role: .cancel) {}
        } message: {
            Text(tr("relay_reset_confirm"))
        }
    }

    private func save() {
        guard let st = model.stationEngine else { return }
        do {
            try st.setRelayServer(origin: server.trimmingCharacters(in: .whitespaces))
            error = false
        } catch {
            self.error = true
        }
    }
}

private struct TrustedListeners: View {
    @EnvironmentObject private var model: Nestwatch
    @Environment(\.palette) private var c
    @State private var confirm: TrustedListenerInfo?

    var body: some View {
        // Re-read whenever the station state changes.
        let trusted = model.station.snapshot.flatMap { _ in model.stationEngine?.trustedListeners() } ?? []
        Card {
            SectionTitle(tr("trusted_listeners"))
            if trusted.isEmpty { Caption(tr("no_trusted")) }
            ForEach(trusted, id: \.id) { t in
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(t.name.isEmpty ? tr("unnamed") : t.name).font(NWFont.body).foregroundColor(c.text1)
                        Caption(tr("paired_seen", agoText(unixMs: t.pairedUnixMs), agoText(unixMs: t.lastSeenUnixMs)))
                    }
                    Spacer()
                    LinkButton(tr("revoke"), color: c.coral) { confirm = t }
                }
            }
        }
        .alert(item: Binding(get: { confirm.map(IdentifiedListener.init) }, set: { confirm = $0?.info })) { w in
            Alert(
                title: Text(tr("revoke_confirm", w.info.name)),
                primaryButton: .destructive(Text(tr("revoke"))) { model.withStation { _ = $0.revoke(listenerId: w.info.id) } },
                secondaryButton: .cancel(Text(tr("cancel")))
            )
        }
    }
}

private struct IdentifiedListener: Identifiable {
    let info: TrustedListenerInfo
    var id: String { info.id }
}
