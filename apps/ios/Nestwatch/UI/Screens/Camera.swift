// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI

/// Camera view (README §5.8, FR-CAM-1, FR-CAM-4): live image, night,
/// switch, talk. The view is open exactly while this screen is shown.
struct CameraScreen: View {
    let id: String
    @EnvironmentObject private var model: Nestwatch
    @ObservedObject private var frames = Nestwatch.shared.frames
    @Environment(\.palette) private var c
    @State private var facing: CameraFacing = .back
    @State private var night = false

    var body: some View {
        let t = model.listenerState?.stations.first { $0.id == id }
        Page(tr("camera_of", t?.name ?? "")) {
            ZStack(alignment: .topLeading) {
                Color.black
                if let img = frames.images[id], case .streaming = t?.video {
                    Image(uiImage: img).resizable().aspectRatio(contentMode: .fit)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .accessibilityLabel(Text(tr("camera_alt", t?.name ?? "")))
                    Pill(text: tr("camera_live"), kind: .cam)
                        .background(Capsule().fill(Color.black.opacity(0.7)))
                        .padding(8)
                } else {
                    Text(stateText(t?.video)).font(NWFont.body).foregroundColor(Color(hex: 0xB9B4C8))
                        .padding(16)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .aspectRatio(4 / 3, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: Radius.medium))
            if case let .streaming(w, h, fps) = t?.video {
                Caption(tr("frame_info", Int(w), Int(h), Int(fps)))
            }
            SwitchRow(label: tr("night_image"), isOn: night) { n in
                night = n
                model.withListener { try $0.setVideoOptions(stationId: id, facing: facing, night: n) }
            }
            SecondaryButton(tr("switch_camera")) {
                facing = facing == .back ? .front : .back
                model.withListener { try $0.setVideoOptions(stationId: id, facing: facing, night: night) }
            }
            if let t, case .live = t.connection, t.canTalk {
                let granted: Bool = { if case .granted = t.talk { return true }; return false }()
                let enabled: Bool = {
                    switch t.talk {
                    case .disabled, .busy: return false
                    default: return true
                    }
                }()
                HoldToTalk(active: granted, enabled: enabled, label: talkLabel(t.talk)) { on in
                    guard !on || AudioSessionConfig.micGranted else {
                        AudioSessionConfig.requestMic { _ in }
                        return
                    }
                    model.withListener { try $0.talk(stationId: id, on: on) }
                }
            }
        }
        .onAppear { model.withListener { try $0.requestVideo(stationId: id, maxW: 640, maxH: 480, maxFps: 10) } }
        .onDisappear { model.withListener { try $0.stopVideo(stationId: id) } }
    }

    private func stateText(_ v: VideoState?) -> String {
        switch v {
        case .disabled: return tr("camera_disabled")
        case .unavailable: return tr("camera_unavailable")
        case .denied: return tr("camera_denied")
        case .stopped: return tr("camera_stopped")
        default: return tr("camera_starting")
        }
    }
}
