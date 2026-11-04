// SPDX-License-Identifier: AGPL-3.0-or-later
import AVFoundation
import UIKit

/// Copies a BGRA image (rows `stride` bytes apart) into tightly packed RGBA.
func rgba(fromBGRA src: UnsafeRawPointer, width: Int, height: Int, stride: Int) -> Data {
    var out = Data(count: width * height * 4)
    out.withUnsafeMutableBytes { raw in
        let dst = raw.bindMemory(to: UInt8.self)
        let s = src.assumingMemoryBound(to: UInt8.self)
        for y in 0..<height {
            let row = s + y * stride
            var d = y * width * 4
            for x in 0..<width {
                let p = row + x * 4
                dst[d] = p[2]
                dst[d + 1] = p[1]
                dst[d + 2] = p[0]
                dst[d + 3] = 255
                d += 4
            }
        }
    }
    return out
}

/// The station's on-demand camera (FR-CAM-1, media.md §5): an
/// `AVCaptureSession` with a BGRA video data output, throttled to the
/// requested frame rate and handed to `sink` upright as RGBA. It runs only
/// between [start] and [stop], i.e. while the engine reports the camera as
/// wanted and the app is in the foreground (platforms.md §2.3).
final class StationCamera: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    private let queue = DispatchQueue(label: "nestwatch.camera")
    private let session = AVCaptureSession()
    private var current: CameraRequest?
    private var minInterval: CFTimeInterval = 0.1
    private var lastFrame: CFTimeInterval = 0
    private let sink: (CameraImage) -> Void
    private let onUnavailable: () -> Void

    init(sink: @escaping (CameraImage) -> Void, onUnavailable: @escaping () -> Void) {
        self.sink = sink
        self.onUnavailable = onUnavailable
        super.init()
    }

    static var authorization: AVAuthorizationStatus { AVCaptureDevice.authorizationStatus(for: .video) }

    static var hasCamera: Bool {
        AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back) != nil
            || AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front) != nil
    }

    /// Starts or reconfigures the camera.
    func start(_ req: CameraRequest, orientation: AVCaptureVideoOrientation) {
        queue.async { [self] in
            minInterval = 1.0 / Double(min(max(req.maxFps, 1), 30))
            let prev = current
            current = req
            if session.isRunning, let prev, prev.facing == req.facing, prev.maxW == req.maxW, prev.maxH == req.maxH {
                return
            }
            configure(req, orientation: orientation)
        }
    }

    private func configure(_ req: CameraRequest, orientation: AVCaptureVideoOrientation) {
        session.beginConfiguration()
        session.inputs.forEach(session.removeInput)
        session.outputs.forEach(session.removeOutput)
        let wanted: AVCaptureDevice.Position = req.facing == .front ? .front : .back
        let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: wanted)
            ?? AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back)
            ?? AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .front)
        guard let device, let input = try? AVCaptureDeviceInput(device: device), session.canAddInput(input) else {
            session.commitConfiguration()
            onUnavailable()
            return
        }
        session.addInput(input)
        let largest = max(req.maxW, req.maxH)
        let preset: AVCaptureSession.Preset = largest > 640 ? .hd1280x720 : (largest > 352 ? .vga640x480 : .cif352x288)
        session.sessionPreset = session.canSetSessionPreset(preset) ? preset : .medium
        let output = AVCaptureVideoDataOutput()
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA]
        output.alwaysDiscardsLateVideoFrames = true
        output.setSampleBufferDelegate(self, queue: queue)
        guard session.canAddOutput(output) else {
            session.commitConfiguration()
            onUnavailable()
            return
        }
        session.addOutput(output)
        if let c = output.connection(with: .video) {
            // Upright buffers for the way the station stands.
            if c.isVideoOrientationSupported { c.videoOrientation = orientation }
            if c.isVideoMirroringSupported { c.isVideoMirrored = device.position == .front }
        }
        session.commitConfiguration()
        if !session.isRunning { session.startRunning() }
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        let now = CACurrentMediaTime()
        guard current != nil, now - lastFrame >= minInterval,
              let px = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        lastFrame = now
        CVPixelBufferLockBaseAddress(px, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(px, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(px) else { return }
        let w = CVPixelBufferGetWidth(px), h = CVPixelBufferGetHeight(px)
        let data = rgba(fromBGRA: base, width: w, height: h, stride: CVPixelBufferGetBytesPerRow(px))
        sink(CameraImage(width: UInt32(w), height: UInt32(h), format: .rgba, rotation: 0, data: data))
    }

    /// Releases the camera at once (FR-CAM-1).
    func stop() {
        queue.async { [self] in
            current = nil
            if session.isRunning { session.stopRunning() }
        }
    }
}

extension AVCaptureVideoOrientation {
    /// The capture orientation matching the interface orientation.
    init(_ o: UIInterfaceOrientation) {
        switch o {
        case .landscapeLeft: self = .landscapeLeft
        case .landscapeRight: self = .landscapeRight
        case .portraitUpsideDown: self = .portraitUpsideDown
        default: self = .portrait
        }
    }
}
