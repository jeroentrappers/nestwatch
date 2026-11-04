// SPDX-License-Identifier: AGPL-3.0-or-later
import XCTest
@testable import Nestwatch

final class LogicTests: XCTestCase {
    func testLevelMappingMatchesUi1() {
        XCTAssertEqual(levelPosition(-90), 0, accuracy: 1e-6)
        XCTAssertEqual(levelPosition(-120), 0, accuracy: 1e-6)
        XCTAssertEqual(levelPosition(-10), 1, accuracy: 1e-6)
        XCTAssertEqual(levelPosition(0), 1, accuracy: 1e-6)
        XCTAssertEqual(levelPosition(-50), 0.5, accuracy: 1e-6)
        XCTAssertEqual(positionToDb(0.5), -50, accuracy: 1e-4)
        XCTAssertEqual(levelPosition(.nan), 0)
        // The core's mapping is the same (UI-1).
        XCTAssertEqual(levelFraction(dbfs: -50), levelPosition(-50), accuracy: 1e-6)
    }

    func testThresholdsAreWholeDecibelsInRange() {
        XCTAssertEqual(clampThreshold(-99), -85)
        XCTAssertEqual(clampThreshold(-3), -10)
        XCTAssertEqual(clampThreshold(-42.4), -42)
    }

    func testHistoryKeepsPeakPerBucketForOneMinute() {
        var h = LevelHistory()
        h.add(nowMs: 1_000, dbfs: -60)
        h.add(nowMs: 1_050, dbfs: -40)
        h.add(nowMs: 1_100, dbfs: -70)
        XCTAssertEqual(h.count, 1)
        XCTAssertEqual(h.range?.max, -40)
        h.add(nowMs: 1_200, dbfs: -55)
        XCTAssertEqual(h.count, 2)
        h.add(nowMs: 62_000, dbfs: -80)
        let s = h.series(nowMs: 62_000)
        XCTAssertTrue(s.allSatisfy { (0...1).contains($0.x) })
        XCTAssertEqual(s.last!.x, 1, accuracy: 1e-6)
        h.add(nowMs: 200_000, dbfs: -30)
        XCTAssertEqual(h.count, 1)
        h.add(nowMs: 200_100, dbfs: .nan)
        XCTAssertEqual(h.count, 1)
    }

    func testCountdownClock() {
        XCTAssertEqual(clockText(ms: 600_000), "10:00")
        XCTAssertEqual(clockText(ms: 5_400), "0:05")
        XCTAssertEqual(clockText(ms: -5), "0:00")
    }

    func testAgo() {
        let now: UInt64 = 10_000_000_000
        XCTAssertEqual(agoText(unixMs: now, nowMs: now), "just now")
        XCTAssertEqual(agoText(unixMs: now - 5 * 60_000, nowMs: now), "5 min ago")
        XCTAssertEqual(agoText(unixMs: now - 3 * 3_600_000, nowMs: now), "3 h ago")
        XCTAssertEqual(agoText(unixMs: now - 30 * 3_600_000, nowMs: now), "yesterday")
        XCTAssertEqual(agoText(unixMs: now - 72 * 3_600_000, nowMs: now), "3 days ago")
        // A clock that went backwards reads as now.
        XCTAssertEqual(agoText(unixMs: now + 1_000, nowMs: now), "just now")
    }

    func testStringsAndPlurals() {
        XCTAssertEqual(trp("listening_count", 1), "1 listening")
        XCTAssertEqual(trp("pair_devices_left", 1), "1 more device")
        XCTAssertEqual(trp("pair_devices_left", 5), "up to 5 devices")
        XCTAssertEqual(tr("alert_battery", "Nursery", 12), "Nursery: battery low (12%)")
        XCTAssertEqual(tr("frame_info", 640, 480, 10), "640×480 · 10 fps")
        XCTAssertNotEqual(tr("station_background_note"), "station_background_note")
    }

    func testInviteUrls() {
        let u = URL(string: "nestwatch://invite?v=1&k=abc&n=Nursery")!
        XCTAssertEqual(invitation(from: u), u.absoluteString)
        XCTAssertNil(invitation(from: URL(string: "https://example.org/invite")!))
        XCTAssertNil(invitation(from: URL(string: "nestwatch://other")!))
    }

    func testPathLabelsCoverEveryPath() {
        // FR-LS-3, rendezvous.md FR-RV-4.
        XCTAssertEqual(pathLabel(.lan), "home network")
        XCTAssertEqual(pathLabel(.vpn), "via VPN")
        XCTAssertEqual(pathLabel(.direct), "direct (internet)")
        XCTAssertEqual(pathLabel(.relayed), "relayed")
    }

    func testRelayStatusPills() {
        XCTAssertEqual(relayLook(.off), .off)
        XCTAssertEqual(relayLook(.connecting), .connecting)
        XCTAssertEqual(relayLook(.online(observed: "203.0.113.9:4000")), .online)
        XCTAssertEqual(relayLook(.error(message: "offline", permanent: false)), .error)
        XCTAssertEqual(relayPillKind(.online), .calm)
        XCTAssertEqual(relayPillKind(.error), .sound)
    }

    private func tile(_ connection: Connection, mode: ListenMode = .activated, open: Bool = false, talk: TalkState = .idle, talker: String? = nil) -> StationTile {
        StationTile(
            id: "s", name: "Nursery", connection: connection, path: .lan, route: nil, levelDbfs: -60, open: open,
            battery: nil, charging: nil, stationListeners: nil, cameraOn: false, talker: talker, talk: talk,
            video: .off, rttMs: nil, canTalk: true, canVideo: true,
            prefs: StationPrefs(
                listen: ListenSettingsInfo(mode: mode, thresholdDbfs: -50, holdMs: 8_000, prerollMs: 1_000),
                volume: 1, muted: false, alerts: AlertSettings(sound: true, unreachable: true, battery: true), follow: true
            ),
            audioFrames: 0
        )
    }

    func testTileStatus() {
        XCTAssertEqual(tileStatus(tile(.live)).kind, .calm)
        XCTAssertEqual(tileStatus(tile(.live)).text, "Quiet")
        XCTAssertEqual(tileStatus(tile(.live, open: true)).kind, .sound)
        XCTAssertTrue(tileHasSound(tile(.live, open: true)))
        XCTAssertFalse(tileHasSound(tile(.live, mode: .continuous, open: true)))
        XCTAssertEqual(tileStatus(tile(.live, mode: .continuous)).text, "Listening")
        XCTAssertEqual(tileStatus(tile(.live, talk: .granted)).text, "You are talking")
        XCTAssertEqual(tileStatus(tile(.live, talker: "Dad")).text, "Dad talking")
        XCTAssertEqual(tileStatus(tile(.reconnecting(attempt: 2, tried: []))).kind, .warn)
        XCTAssertEqual(tileStatus(tile(.needsPairing)).kind, .warn)
        XCTAssertEqual(tileStatus(tile(.idle)).kind, .neutral)
        XCTAssertEqual(talkLabel(.busy(by: "Mum")), "Mum is talking")
        XCTAssertEqual(talkLabel(.busy(by: nil)), "Someone else is talking")
    }

    func testErrorTexts() {
        XCTAssertEqual(errorText(NestwatchError.Unreachable(reason: "x")), tr("err_unreachable"))
        XCTAssertEqual(errorText(NestwatchError.Invalid(reason: "station id")), tr("err_invalid_id"))
        XCTAssertEqual(errorText(NestwatchError.Invalid(reason: "code"), invalid: "err_invalid_code"), tr("err_invalid_code"))
        XCTAssertEqual(errorText(NestwatchError.Stopped), tr("err_generic"))
    }
}

final class MediaTests: XCTestCase {
    func testPcmRoundTrip() {
        let samples: [Float] = [0, 0.5, -0.5, 1, -1, 2]
        let data = samples.withUnsafeBufferPointer(pcm16(from:))
        XCTAssertEqual(data.count, 12)
        var back = [Float](repeating: 9, count: 8)
        back.withUnsafeMutableBufferPointer { floats(fromPCM16: data, into: $0) }
        XCTAssertEqual(back[0], 0)
        XCTAssertEqual(back[1], 0.5, accuracy: 1e-3)
        XCTAssertEqual(back[2], -0.5, accuracy: 1e-3)
        XCTAssertEqual(back[3], 1, accuracy: 1e-3)
        XCTAssertEqual(back[5], 1, accuracy: 1e-3, "clamped")
        XCTAssertEqual(back[6], 0, "padded with silence")
        XCTAssertEqual(back[7], 0)
        // Little-endian on the wire (FFI-1).
        XCTAssertEqual([UInt8](data.prefix(4)), [0, 0, 0xFF, 0x3F])
    }

    func testBgraToRgba() {
        // 2×1 image, rows padded to 12 bytes: blue, then red.
        let bgra: [UInt8] = [255, 0, 0, 255, 0, 0, 255, 255, 7, 7, 7, 7]
        let out = bgra.withUnsafeBytes { rgba(fromBGRA: $0.baseAddress!, width: 2, height: 1, stride: 12) }
        XCTAssertEqual([UInt8](out), [0, 0, 255, 255, 255, 0, 0, 255])
    }

    func testQrMatrixFromCore() {
        let qr = qrCode(text: "nestwatch://invite?v=1")
        XCTAssertNotNil(qr)
        XCTAssertEqual(Int(qr!.size * qr!.size), qr!.modules.count)
    }

    func testInvitationParsingRejectsJunk() {
        XCTAssertThrowsError(try parseInvitation(input: "https://example.org"))
        XCTAssertFalse(isValidRoute(input: "not an address"))
        XCTAssertTrue(isValidRoute(input: "192.168.1.20:47100"))
    }
}

final class StorageTests: XCTestCase {
    func testKeychainRoundTrip() throws {
        let defaults = UserDefaults(suiteName: "nestwatch-tests")!
        defaults.removePersistentDomain(forName: "nestwatch-tests")
        let s = KeychainStorage(service: "org.nestwatch.Nestwatch.tests", defaults: defaults)
        try s.delete(key: "device-key")
        XCTAssertNil(try s.get(key: "device-key"))
        try s.put(key: "device-key", value: Data([1, 2, 3]))
        XCTAssertEqual(try s.get(key: "device-key"), Data([1, 2, 3]))
        try s.put(key: "device-key", value: Data([4]))
        XCTAssertEqual(try s.get(key: "device-key"), Data([4]))
        try s.delete(key: "device-key")
        XCTAssertNil(try s.get(key: "device-key"))
        try s.delete(key: "device-key")
        s.removeAll()
    }

    func testPrefsPersist() {
        let defaults = UserDefaults(suiteName: "nestwatch-prefs-tests")!
        defaults.removePersistentDomain(forName: "nestwatch-prefs-tests")
        let p = Prefs(defaults: defaults)
        XCTAssertEqual(p.value.role, .none)
        p.update {
            $0.role = .listener
            $0.theme = .night
            $0.relayAsked = true
        }
        let q = Prefs(defaults: defaults)
        XCTAssertEqual(q.value.role, .listener)
        XCTAssertEqual(q.value.theme, .night)
        XCTAssertTrue(q.value.relayAsked)
    }
}
