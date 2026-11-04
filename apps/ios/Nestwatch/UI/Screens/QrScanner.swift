// SPDX-License-Identifier: AGPL-3.0-or-later
import AVFoundation
import SwiftUI

/// Full-screen QR scanner for the station's invitation (FR-PAIR-1): an
/// `AVCaptureMetadataOutput` for QR codes on the back camera. Nothing is
/// recorded; the first QR text ends the scan.
struct QrScannerView: View {
    let onResult: (String) -> Void
    let onCancel: () -> Void

    var body: some View {
        ZStack(alignment: .bottom) {
            ScannerController(onResult: onResult).ignoresSafeArea()
            VStack(spacing: 12) {
                Text(tr("scan_hint")).font(NWFont.body).foregroundColor(.white).multilineTextAlignment(.center)
                Button(action: onCancel) {
                    Text(tr("scan_cancel")).font(NWFont.subtitle).foregroundColor(.white)
                        .frame(maxWidth: .infinity, minHeight: 48)
                        .background(RoundedRectangle(cornerRadius: Radius.medium).fill(Color.black.opacity(0.6)))
                }
                .buttonStyle(.plain)
            }
            .padding(24)
        }
        .background(Color.black.ignoresSafeArea())
    }
}

private struct ScannerController: UIViewControllerRepresentable {
    let onResult: (String) -> Void

    func makeUIViewController(context: Context) -> ScannerViewController {
        let vc = ScannerViewController()
        vc.onResult = onResult
        return vc
    }

    func updateUIViewController(_ vc: ScannerViewController, context: Context) {}
}

final class ScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var onResult: ((String) -> Void)?
    private let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "nestwatch.scanner")
    private var preview: AVCaptureVideoPreviewLayer?
    private var done = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back)
            ?? AVCaptureDevice.default(for: .video),
            let input = try? AVCaptureDeviceInput(device: device), session.canAddInput(input) else { return }
        session.addInput(input)
        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { return }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        if output.availableMetadataObjectTypes.contains(.qr) { output.metadataObjectTypes = [.qr] }
        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        view.layer.addSublayer(layer)
        preview = layer
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview?.frame = view.bounds
        if let c = preview?.connection, c.isVideoOrientationSupported {
            c.videoOrientation = AVCaptureVideoOrientation(view.window?.windowScene?.interfaceOrientation ?? .portrait)
        }
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        queue.async { [session] in if !session.isRunning { session.startRunning() } }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        queue.async { [session] in if session.isRunning { session.stopRunning() } }
    }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
        guard !done, let text = objects.compactMap({ ($0 as? AVMetadataMachineReadableCodeObject)?.stringValue }).first else { return }
        done = true
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        onResult?(text)
    }
}
