// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI

/// Welcome: name, promise, disclaimer and the role choice (README §5.1, UI-6).
struct WelcomeScreen: View {
    let onRole: (Role) -> Void
    @Environment(\.palette) private var c

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                Spacer(minLength: 24)
                Logo(size: 72)
                Text(tr("app_name")).font(NWFont.headline).foregroundColor(c.text1).accessibilityAddTraits(.isHeader)
                Text(tr("promise")).font(NWFont.body).foregroundColor(c.text2).multilineTextAlignment(.center)
                Card {
                    Text(tr("disclaimer")).font(NWFont.bodySmall).foregroundColor(c.amber)
                    Caption(tr("disclaimer_ack"))
                }
                RoleChoice(title: tr("role_station"), hint: tr("role_station_hint")) { onRole(.station) }
                RoleChoice(title: tr("role_listener"), hint: tr("role_listener_hint")) { onRole(.listener) }
            }
            .frame(maxWidth: 520)
            .padding(24)
            .frame(maxWidth: .infinity)
        }
        .background(c.ink0.ignoresSafeArea())
    }
}

private struct RoleChoice: View {
    let title: String
    let hint: String
    let action: () -> Void
    @Environment(\.palette) private var c

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(NWFont.subtitle).foregroundColor(c.honey)
                Text(hint).font(NWFont.bodySmall).foregroundColor(c.text2).multilineTextAlignment(.leading)
            }
            .padding(16)
            .frame(maxWidth: .infinity, minHeight: 72, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: Radius.large).fill(c.ink1))
            .overlay(RoundedRectangle(cornerRadius: Radius.large).strokeBorder(c.ink3))
        }
        .buttonStyle(.plain)
    }
}

/// About: privacy promise, disclaimer, version, device ID, licence.
struct AboutScreen: View {
    @EnvironmentObject private var model: Nestwatch
    @Environment(\.palette) private var c

    var body: some View {
        Page(tr("about")) {
            Card {
                Logo(size: 48)
                Text(tr("about_body")).font(NWFont.body).foregroundColor(c.text1)
                Text(tr("disclaimer")).font(NWFont.bodySmall).foregroundColor(c.amber)
            }
            Card {
                Caption(tr("version"))
                Text("\(appVersion) · \(coreVersion())").font(NWFont.body).foregroundColor(c.text1)
                if let id = model.deviceId {
                    Caption(tr("device_id"))
                    Text(groupedId(id: id)).font(NWFont.mono(15)).tracking(2).foregroundColor(c.text1).textSelection(.enabled)
                }
                Caption(tr("license"))
            }
        }
    }

    private var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "?"
    }
}
