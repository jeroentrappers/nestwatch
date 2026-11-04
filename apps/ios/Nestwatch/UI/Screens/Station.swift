// SPDX-License-Identifier: AGPL-3.0-or-later
import AVFoundation
import SwiftUI

/// An explanation shown before a system permission prompt (security.md PRIV-4).
struct PermissionAsk: Identifiable {
    let id = UUID()
    let title: String
    let body: String
    let proceed: () -> Void
    let cancel: () -> Void
}

/// The station screen (README §5.2, §5.3): state first, then actions.
struct StationHome: View {
    @Binding var dimmed: Bool
    let wide: Bool
    @EnvironmentObject private var model: Nestwatch
    @EnvironmentObject private var nav: Nav
    @Environment(\.palette) private var c
    @State private var ask: PermissionAsk?
    @State private var micDenied = false

    var body: some View {
        let run = model.station
        if dimmed, let s = run.snapshot {
            DimmedStation(s: s) { dimmed = false }
        } else {
            Page(run.snapshot?.name ?? tr("app_name"), back: false, maxWidth: wide ? 1100 : 760) {
                IconButton("info.circle", label: tr("about")) { nav.push(.about) }
                IconButton("gearshape", label: tr("settings")) { nav.push(.stationSettings) }
            } content: {
                switch run {
                case .stopped:
                    Card {
                        Text(tr("station_stopped")).font(NWFont.body).foregroundColor(c.text1)
                        if micDenied {
                            Text(tr("perm_denied_mic")).font(NWFont.bodySmall).foregroundColor(c.amber)
                            SecondaryButton(tr("open_settings"), action: openAppSettings)
                        }
                        PrimaryButton(tr("station_start"), action: start)
                    }
                case .starting:
                    HStack(spacing: 12) {
                        ProgressView().tint(c.honey)
                        Text(tr("station_starting")).font(NWFont.body).foregroundColor(c.text2)
                    }
                case let .failed(reason):
                    Card {
                        Text(tr("station_failed")).font(NWFont.body).foregroundColor(c.coral)
                        Caption(reason)
                        PrimaryButton(tr("retry"), action: start)
                    }
                case let .running(s):
                    RunningStation(s: s, wide: wide) { dimmed = true }
                }
            }
            .alert(item: $ask) { a in
                Alert(
                    title: Text(a.title),
                    message: Text(a.body),
                    primaryButton: .default(Text(tr("perm_continue")), action: a.proceed),
                    secondaryButton: .cancel(Text(tr("perm_not_now")), action: a.cancel)
                )
            }
        }
    }

    /// Microphone (required), then camera (optional), then start.
    private func start() {
        switch AVAudioSession.sharedInstance().recordPermission {
        case .granted:
            micDenied = false
            askCamera()
        case .denied:
            micDenied = true
        default:
            ask = PermissionAsk(title: tr("perm_mic_title"), body: tr("perm_mic_station"), proceed: {
                AudioSessionConfig.requestMic { ok in
                    micDenied = !ok
                    if ok { askCamera() }
                }
            }, cancel: { micDenied = true })
        }
    }

    private func askCamera() {
        guard StationCamera.hasCamera, StationCamera.authorization == .notDetermined else {
            StationHost.shared.start()
            return
        }
        // Asked now, while someone is at the device: an unattended station
        // cannot answer a prompt when a listener opens the camera (IOS-6).
        DispatchQueue.main.async {
            ask = PermissionAsk(title: tr("perm_camera_title"), body: tr("perm_camera_station"), proceed: {
                AVCaptureDevice.requestAccess(for: .video) { _ in
                    DispatchQueue.main.async { StationHost.shared.start() }
                }
            }, cancel: { StationHost.shared.start() })
        }
    }
}

private struct RunningStation: View {
    let s: StationSnapshot
    let wide: Bool
    let onDim: () -> Void
    @Environment(\.palette) private var c
    @EnvironmentObject private var model: Nestwatch
    @State private var pairing = false

    var body: some View {
        Group {
            if wide {
                HStack(alignment: .top, spacing: 24) {
                    VStack(alignment: .leading, spacing: 16) {
                        pills
                        ring
                        actions
                    }
                    .frame(maxWidth: .infinity)
                    VStack(alignment: .leading, spacing: 16) {
                        listeners
                        Caption(tr("station_background_note"))
                    }
                    .frame(maxWidth: .infinity)
                }
            } else {
                pills
                ring
                listeners
                actions
                Caption(tr("station_background_note"))
            }
        }
        .modifier(RelaySetupPrompt(s: s))
        .onReceive(model.$debugPairing) { if $0 { pairing = true } }
        .sheet(isPresented: $pairing) {
            PairingSheet(onClose: { pairing = false })
                .environment(\.palette, c)
                .environmentObject(Nestwatch.shared)
        }
    }

    private var pills: some View {
        var p: [(String, PillKind)] = []
        let n = s.listeners.count
        p.append((n == 0 ? tr("nobody_listening") : trp("listening_count", n), n == 0 ? .neutral : .calm))
        p.append((tr(s.cameraOn ? "camera_on" : "camera_off"), s.cameraOn ? .cam : .neutral))
        if let t = s.talker { p.append((tr("talking_through", t), .voice)) }
        if !s.settings.talkEnabled { p.append((tr("talk_off"), .neutral)) }
        if s.settings.relayEnabled {
            let look = relayLook(s.relay)
            p.append((tr("relay_pill", relayLabel(look)), relayPillKind(look)))
        }
        return PillRow(pills: p)
    }

    private var ring: some View {
        LevelRing(levelDb: s.levelDbfs, thresholdDb: s.strictestThresholdDbfs, sound: s.soundOpen)
            .frame(maxWidth: .infinity)
    }

    private var listeners: some View {
        Card {
            SectionTitle(tr("listeners_heading"))
            if s.listeners.isEmpty { Caption(tr("no_listeners")) }
            ForEach(s.listeners, id: \.session) { l in
                VStack(alignment: .leading, spacing: 2) {
                    Text(l.name.isEmpty ? tr("unnamed") : l.name).font(NWFont.body).foregroundColor(c.text1)
                    Caption(listenerBits(l).joined(separator: " · "))
                }
            }
        }
    }

    private var actions: some View {
        VStack(spacing: 12) {
            PrimaryButton(tr("pair_listener")) { pairing = true }
            HStack(spacing: 12) {
                SecondaryButton(tr("dim_screen"), action: onDim)
                DangerButton(tr("station_stop")) { StationHost.shared.stop() }
            }
            Caption(tr("station_keeps_running"))
        }
    }

    private func listenerBits(_ l: ConnectedListenerInfo) -> [String] {
        var bits = [tr(l.mode == .activated ? "mode_sound_activated" : "mode_always_on")]
        if l.web { bits.append(tr("via_browser")) }
        switch l.transport {
        case .relayed: bits.append(tr("path_relayed"))
        case .direct: bits.append(tr("path_direct"))
        default: break
        }
        if l.open { bits.append(tr("hearing")) }
        if l.talking { bits.append(tr("talking_now")) }
        if l.watching { bits.append(tr("watching_now")) }
        return bits
    }
}

func relayPillKind(_ look: RelayLook) -> PillKind {
    switch look {
    case .off: return .neutral
    case .connecting: return .warn
    case .online: return .calm
    case .error: return .sound
    }
}

/// The pairing sheet (README §5.4): QR, code, time left, verification numbers.
struct PairingSheet: View {
    let onClose: () -> Void
    @EnvironmentObject private var model: Nestwatch
    @Environment(\.palette) private var c
    @State private var failed = false
    @State private var seen = false

    var body: some View {
        let s = model.station.snapshot
        ScrollView {
            VStack(spacing: 12) {
                Text(tr("pair_listener")).font(NWFont.title).foregroundColor(c.text1).padding(.top, 16)
                if let s, let p = s.pairing {
                    Text(tr("pair_intro")).font(NWFont.body).foregroundColor(c.text2).multilineTextAlignment(.center)
                    if let qr = qrCode(text: p.inviteUri) {
                        QrImage(qr: qr, label: tr("qr_label")).frame(maxWidth: 280)
                    }
                    Caption(tr("pair_code_intro"))
                    CodeBox(text: p.code)
                    Caption(tr("pair_station_id"))
                    Text(groupedId(id: s.deviceId)).font(NWFont.mono(14)).tracking(2).foregroundColor(c.text2).textSelection(.enabled)
                    let left = max(Int(p.maxPairings) - Int(p.pairings), 0)
                    Caption(tr("pair_open_for", clockText(ms: Int64(p.remainingMs)), trp("pair_devices_left", left)))
                    if !model.justPaired.isEmpty {
                        Card {
                            Text(tr("pair_just_paired")).font(NWFont.body).foregroundColor(c.text1)
                            ForEach(model.justPaired) { j in
                                HStack {
                                    Text(j.name).font(NWFont.body).foregroundColor(c.text1)
                                    Spacer()
                                    CodeBox(text: j.verification)
                                }
                            }
                        }
                    }
                    DangerButton(tr("pair_close")) {
                        model.withStation { $0.closePairing() }
                        onClose()
                    }
                    .onAppear { seen = true }
                } else if failed {
                    Text(tr("err_generic")).foregroundColor(c.coral)
                } else if seen {
                    Text(tr("pair_closed")).font(NWFont.body).foregroundColor(c.text2)
                    PrimaryButton(tr("pair_listener")) { open() }
                } else {
                    Text(tr("pair_opening")).font(NWFont.body).foregroundColor(c.text2)
                }
                SecondaryButton(tr("close"), action: onClose).padding(.top, 8)
            }
            .padding(24)
            .frame(maxWidth: 520)
            .frame(maxWidth: .infinity)
        }
        .background(c.ink1.ignoresSafeArea())
        .onAppear { if model.station.snapshot?.pairing == nil { open() } }
    }

    private func open() {
        guard let st = model.stationEngine else { return }
        Task { @MainActor in
            do { _ = try await st.openPairing() } catch { failed = true }
        }
    }
}

/// Dimmed station (README §5.3): ring and listener count only; tap to brighten.
private struct DimmedStation: View {
    let s: StationSnapshot
    let onBrighten: () -> Void
    @Environment(\.palette) private var c

    var body: some View {
        ZStack {
            c.ink0.ignoresSafeArea()
            VStack(spacing: 16) {
                LevelRing(levelDb: s.levelDbfs, thresholdDb: s.strictestThresholdDbfs, sound: s.soundOpen, size: 160, showNumbers: false)
                let n = s.listeners.count
                Text(n == 0 ? tr("nobody_listening") : trp("listening_count", n)).font(NWFont.body).foregroundColor(c.text2)
                // The camera indicator stays visible while dimmed (FR-CAM-2).
                if s.cameraOn { Pill(text: tr("camera_on"), kind: .cam) }
                if let t = s.talker { Pill(text: tr("talking_through", t), kind: .voice) }
                Caption(tr("tap_to_brighten"))
            }
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: onBrighten)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(tr("dimmed_label")))
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { onBrighten() }
    }
}

/// Asked once when the station first runs (FR-RV-1): off unless the user opts in.
private struct RelaySetupPrompt: ViewModifier {
    let s: StationSnapshot
    @ObservedObject private var prefs = Nestwatch.shared.prefs

    func body(content: Content) -> some View {
        content.alert(tr("relay_setup_title"), isPresented: Binding(
            get: { !prefs.value.relayAsked },
            set: { if !$0 { done(false) } }
        )) {
            Button(tr("relay_setup_yes")) { done(true) }
            Button(tr("relay_setup_no"), role: .cancel) { done(false) }
        } message: {
            Text(relayExplanation(s.settings.relayServer) + "\n\n" + tr("relay_setup_later"))
        }
    }

    private func done(_ on: Bool) {
        guard !prefs.value.relayAsked else { return }
        if on { Nestwatch.shared.withStation { try $0.setRelayEnabled(enabled: true) } }
        prefs.update { $0.relayAsked = true }
    }
}

/// What the relay server can and cannot see (rendezvous.md §7).
func relayExplanation(_ server: String) -> String {
    let host = server.hasPrefix("https://") ? String(server.dropFirst(8)) : server
    return [tr("relay_server_is", host), tr("relay_explain"), tr("relay_can_see"), tr("relay_cannot_see")].joined(separator: "\n\n")
}
