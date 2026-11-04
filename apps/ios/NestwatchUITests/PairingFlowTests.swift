// SPDX-License-Identifier: AGPL-3.0-or-later
import XCTest

/// End-to-end flows against a real station, driven through the UI. They
/// run only when the station's details are given, e.g.
///
///     TEST_RUNNER_NESTWATCH_E2E_INVITE='nestwatch://invite?…' xcodebuild test \
///       -only-testing:NestwatchUITests …
///
/// and are skipped otherwise (CI). `NESTWATCH_E2E_HOLD` keeps the final
/// screens up for that many seconds (for screenshots).
final class PairingFlowTests: XCTestCase {
    private var env: [String: String] { ProcessInfo.processInfo.environment }
    private var hold: TimeInterval { TimeInterval(env["NESTWATCH_E2E_HOLD"] ?? "") ?? 0 }

    override func setUp() {
        continueAfterFailure = false
        // System prompts (notifications, microphone, local network).
        addUIInterruptionMonitor(withDescription: "permissions") { alert in
            for b in ["Allow", "OK", "Allow While Using App"] where alert.buttons[b].exists {
                alert.buttons[b].tap()
                return true
            }
            return false
        }
    }

    private func dismissSpringboardAlerts() {
        let sb = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        for b in ["Allow", "OK"] where sb.buttons[b].waitForExistence(timeout: 2) {
            sb.buttons[b].tap()
        }
    }

    /// The first button whose label starts with `text`.
    private func button(_ app: XCUIApplication, _ text: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", text)).firstMatch
    }

    func testListenerPairsByInvitationAndListens() throws {
        guard let invite = env["NESTWATCH_E2E_INVITE"] else { throw XCTSkip("no NESTWATCH_E2E_INVITE") }
        let app = XCUIApplication()
        app.launchArguments += ["-role", "none", "-AppleLanguages", "(en)"]
        app.launch()
        button(app, "This device listens").tap()
        dismissSpringboardAlerts()
        button(app, "Add a station").tap()
        button(app, "Use a link").tap()
        let field = app.textFields["Invitation link"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText(invite)
        button(app, "Pair with link").tap()
        let same = button(app, "Same, continue")
        XCTAssertTrue(same.waitForExistence(timeout: 60), "verification number shown")
        if hold > 0 { sleep(UInt32(min(hold, 15))) }
        same.tap()
        // The tile goes live with a path label and a level.
        let live = app.staticTexts.containing(NSPredicate(format: "label CONTAINS 'now'")).firstMatch
        XCTAssertTrue(live.waitForExistence(timeout: 60), "live level on the tile")
        if hold > 0 { sleep(UInt32(hold)) }
        // Talk-back: hold the button for two seconds.
        let talk = app.otherElements["Hold to talk"].exists ? app.otherElements["Hold to talk"] : app.buttons["Hold to talk"]
        if talk.waitForExistence(timeout: 5) {
            talk.press(forDuration: 2)
            dismissSpringboardAlerts()
        }
        // The on-demand camera.
        if button(app, "Camera").waitForExistence(timeout: 5) {
            button(app, "Camera").tap()
            XCTAssertTrue(app.staticTexts["Camera live"].waitForExistence(timeout: 30), "camera frames arrive")
            if hold > 0 { sleep(UInt32(hold)) }
            button(app, "Back").tap()
        }
    }
}

/// The app as a station: start it, opt in to the relay, open pairing and
/// print the code and station ID for a listener outside the test
/// (`NESTWATCH_E2E_STATION=1`), then wait for that listener.
final class StationFlowTests: XCTestCase {
    func testStationPairsAListener() throws {
        let env = ProcessInfo.processInfo.environment
        guard env["NESTWATCH_E2E_STATION"] != nil else { throw XCTSkip("no NESTWATCH_E2E_STATION") }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments += ["-role", "none", "-relay-asked", "NO", "-AppleLanguages", "(en)"]
        app.launch()
        func button(_ t: String) -> XCUIElement {
            app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", t)).firstMatch
        }
        func any(_ label: String) -> XCUIElement {
            app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", label)).firstMatch
        }
        button("This device watches").tap()
        button("Start the station").tap()
        // Our explanation, then the system prompt.
        for t in ["Continue", "Continue"] where button(t).waitForExistence(timeout: 3) { button(t).tap() }
        let sb = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        for _ in 0..<3 where sb.buttons["Allow"].waitForExistence(timeout: 3) { sb.buttons["Allow"].tap() }
        let relay = button("Make it reachable")
        XCTAssertTrue(relay.waitForExistence(timeout: 30), "relay offered once")
        relay.tap()
        XCTAssertTrue(any("Relay: Online").waitForExistence(timeout: 30), "relay online")
        button("Pair a listener").tap()
        let code = app.staticTexts.matching(NSPredicate(format: "label MATCHES '^[A-Z0-9]{5}-[A-Z0-9]{5}$'")).firstMatch
        XCTAssertTrue(code.waitForExistence(timeout: 20))
        let id = app.staticTexts.matching(NSPredicate(format: "label MATCHES '^[a-z0-9]{4}(-[a-z0-9]{4})+$'")).firstMatch
        XCTAssertTrue(id.waitForExistence(timeout: 5))
        print("E2E-STATION code=\(code.label) id=\(id.label)")
        let paired = app.staticTexts["Paired now. Check that the listener shows the same number:"]
        XCTAssertTrue(paired.waitForExistence(timeout: 180), "a listener paired")
        sleep(8)
        button("Close").tap()
        XCTAssertTrue(any("1 listening").waitForExistence(timeout: 60), "the listener is connected")
        sleep(UInt32(env["NESTWATCH_E2E_HOLD"].flatMap(UInt32.init) ?? 10))
    }
}
