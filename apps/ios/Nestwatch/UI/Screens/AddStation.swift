// SPDX-License-Identifier: AGPL-3.0-or-later
import AVFoundation
import SwiftUI

/// Add a station (README §5.5): QR scan, nearby, code, link, manual address,
/// then the verification number check (FR-PAIR-5).
struct AddStationScreen: View {
    let initialInvite: String?
    @EnvironmentObject private var model: Nestwatch
    @EnvironmentObject private var nav: Nav
    @Environment(\.palette) private var c
    @State private var scanning = false
    @State private var busy = false
    @State private var error: String?
    @State private var result: PairResult?
    @State private var removed = false
    @State private var invite = ""
    @State private var code = ""
    @State private var stationId = ""
    @State private var chosen: NearbyStation?
    @State private var address = ""
    @State private var byLink = false
    @State private var cameraNote: String?
    @State private var started = false

    var body: some View {
        Page(tr("add_station")) {
            Card {
                PrimaryButton(tr("scan_qr"), action: scan)
                Caption(cameraNote ?? tr("scan_hint"), color: cameraNote == nil ? nil : c.amber)
            }
            nearby
            Card {
                Segmented(options: [(false, tr("by_code")), (true, tr("by_link"))], selected: byLink) { byLink = $0 }
                if byLink {
                    Field(label: tr("invite_field"), text: $invite, hint: tr("invite_hint"), keyboard: .URL)
                } else {
                    Field(
                        label: chosen.map { tr("code_field_for", $0.name) } ?? tr("code_field"),
                        text: $code,
                        mono: true,
                        keyboard: .asciiCapable,
                        capitalization: .characters
                    )
                    if chosen == nil {
                        Field(label: tr("station_id_field"), text: $stationId, keyboard: .asciiCapable)
                    }
                }
                let addr = address.trimmingCharacters(in: .whitespaces)
                let addrValid = addr.isEmpty || isValidRoute(input: addr)
                Field(
                    label: tr("address_field"),
                    text: $address,
                    hint: addrValid ? tr("address_hint") : tr("err_invalid_address"),
                    error: !addrValid,
                    keyboard: .URL
                )
                if busy {
                    HStack(spacing: 8) {
                        ProgressView().tint(c.honey)
                        Text(tr("pairing")).font(NWFont.body).foregroundColor(c.text2)
                    }
                } else if byLink {
                    PrimaryButton(tr("invite_submit"), enabled: !invite.trimmingCharacters(in: .whitespaces).isEmpty && addrValid) {
                        if (try? parseInvitation(input: invite.trimmingCharacters(in: .whitespacesAndNewlines))) == nil {
                            error = tr("err_invalid_invite")
                        } else {
                            pairInvite(invite)
                        }
                    }
                } else {
                    PrimaryButton(
                        tr("code_submit"),
                        enabled: !code.isEmpty && (chosen != nil || !stationId.isEmpty) && addrValid,
                        action: pairCode
                    )
                }
                if let error { Text(error).font(NWFont.bodySmall).foregroundColor(c.coral) }
                if removed { Text(tr("verify_removed")).font(NWFont.bodySmall).foregroundColor(c.amber) }
            }
        }
        .onAppear {
            guard !started else { return }
            started = true
            if let i = initialInvite, !i.isEmpty {
                invite = i
                byLink = true
                pairInvite(i)
            }
        }
        .fullScreenCover(isPresented: $scanning) {
            QrScannerView(onResult: { text in
                scanning = false
                invite = text
                byLink = true
                if (try? parseInvitation(input: text)) == nil {
                    error = tr("scan_not_nestwatch")
                } else {
                    pairInvite(text)
                }
            }, onCancel: { scanning = false })
        }
        .sheet(item: Binding(get: { result.map(IdentifiedResult.init) }, set: { if $0 == nil { result = nil } })) { r in
            VerifySheet(result: r.result, onSame: {
                result = nil
                nav.show(.listener)
            }, onDifferent: {
                model.withListener { try $0.forget(stationId: r.result.stationId) }
                result = nil
                removed = true
            })
            .environment(\.palette, c)
        }
    }

    @ViewBuilder private var nearby: some View {
        Card {
            SectionTitle(tr("nearby"))
            let list = model.listenerState?.nearby ?? []
            if list.isEmpty {
                HStack(spacing: 8) {
                    ProgressView().tint(c.text3)
                    Caption(tr("nearby_searching"))
                }
            }
            ForEach(list, id: \.id) { n in
                Button {
                    chosen = n
                    stationId = n.id
                    byLink = false
                } label: {
                    HStack {
                        Text(n.name).font(NWFont.body).foregroundColor(c.text1)
                        Spacer()
                        if n.known {
                            Pill(text: tr("already_paired"), kind: .neutral)
                        } else if n.pairing {
                            Pill(text: tr("pairing_open"), kind: .calm)
                        } else {
                            Pill(text: tr("not_pairing"), kind: .neutral)
                        }
                    }
                    .frame(minHeight: 44)
                    .background(chosen?.id == n.id ? c.lavenderSoft : Color.clear)
                }
                .buttonStyle(.plain)
                .disabled(!n.pairing || n.known)
            }
        }
    }

    private var routes: [String] {
        let a = address.trimmingCharacters(in: .whitespaces)
        return a.isEmpty ? [] : [a]
    }

    private func scan() {
        cameraNote = nil
        guard AVCaptureDevice.default(for: .video) != nil else {
            cameraNote = tr("no_camera_scan")
            return
        }
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized: scanning = true
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { ok in
                DispatchQueue.main.async {
                    if ok { scanning = true } else { cameraNote = tr("perm_denied_camera") }
                }
            }
        default: cameraNote = tr("perm_denied_camera")
        }
    }

    private func pairInvite(_ text: String) {
        guard let l = model.listener() else { return }
        busy = true
        error = nil
        let r = routes
        Task { @MainActor in
            defer { busy = false }
            do {
                result = try await l.pairInvitation(input: text.trimmingCharacters(in: .whitespacesAndNewlines), routes: r)
            } catch {
                self.error = errorText(error, invalid: "err_invalid_invite")
            }
        }
    }

    private func pairCode() {
        guard let l = model.listener() else { return }
        busy = true
        error = nil
        let c = code.trimmingCharacters(in: .whitespaces).uppercased()
        let manual = routes
        let picked = chosen
        let typed = stationId
        Task { @MainActor in
            defer { busy = false }
            do {
                let id = try picked?.id ?? parseDeviceId(input: typed)
                let r = manual.isEmpty ? (picked?.endpoints ?? []) : manual
                result = try await l.pairCode(code: c, stationId: id, routes: r)
            } catch {
                self.error = errorText(error, invalid: "err_invalid_code")
            }
        }
    }
}

private struct IdentifiedResult: Identifiable {
    let result: PairResult
    var id: String { result.stationId }
}

/// "Check that the station shows the same number" (FR-PAIR-5).
private struct VerifySheet: View {
    let result: PairResult
    let onSame: () -> Void
    let onDifferent: () -> Void
    @Environment(\.palette) private var c

    var body: some View {
        VStack(spacing: 16) {
            Text(tr("verify_prompt")).font(NWFont.title).foregroundColor(c.text1).multilineTextAlignment(.center)
            Text(result.name).font(NWFont.body).foregroundColor(c.text2)
            CodeBox(text: result.verification, size: 32)
            PrimaryButton(tr("verify_same"), action: onSame)
            DangerButton(tr("verify_different"), action: onDifferent)
        }
        .padding(24)
        .frame(maxWidth: 480)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(c.ink1.ignoresSafeArea())
        .interactiveDismissDisabled()
    }
}
