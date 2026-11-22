// SPDX-License-Identifier: AGPL-3.0-or-later
import XCTest

/// Captures the App Store screenshots (docs/releasing.md §6). Skipped unless
/// `NESTWATCH_SHOTS` is set; the listener test also needs one or more
/// invitations of running stations, e.g. from the CLI:
///
///     nestwatch-cli --data /tmp/st1 station --name Nursery --pair --tone --loopback
///     TEST_RUNNER_NESTWATCH_SHOTS=1 \
///     TEST_RUNNER_NESTWATCH_SHOT_INVITES='nestwatch://invite?… nestwatch://invite?…' \
///       xcodebuild test -only-testing:NestwatchUITests/StoreScreenshotTests …
///
/// The screenshots are kept as attachments of the result bundle, named
/// `NN-name`; `xcrun xcresulttool export attachments` writes them out.
final class StoreScreenshotTests: XCTestCase {
    private var env: [String: String] { ProcessInfo.processInfo.environment }

    override func setUpWithError() throws {
        guard env["NESTWATCH_SHOTS"] != nil else { throw XCTSkip("no NESTWATCH_SHOTS") }
        continueAfterFailure = false
        addUIInterruptionMonitor(withDescription: "permissions") { alert in
            for b in ["Allow", "OK", "Allow While Using App"] where alert.buttons[b].exists {
                alert.buttons[b].tap()
                return true
            }
            return false
        }
    }

    private func shot(_ name: String) {
        let a = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        a.name = name
        a.lifetime = .keepAlways
        add(a)
    }

    private func button(_ app: XCUIApplication, _ text: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", text)).firstMatch
    }

    private func allowSystemPrompts() {
        let sb = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        for _ in 0..<3 {
            var tapped = false
            for b in ["Allow", "Allow Once", "OK", "Allow While Using App"] where sb.buttons[b].firstMatch.waitForExistence(timeout: 2) {
                sb.buttons[b].firstMatch.tap()
                tapped = true
            }
            if !tapped { break }
        }
    }

    func testListenerScreens() throws {
        let invites = (env["NESTWATCH_SHOT_INVITES"] ?? "").split(separator: " ").map(String.init)
        guard !invites.isEmpty else { throw XCTSkip("no NESTWATCH_SHOT_INVITES") }
        let app = XCUIApplication()
        app.launchArguments += ["-role", "none", "-AppleLanguages", "(en)"]
        app.launch()
        XCTAssertTrue(button(app, "This device listens").waitForExistence(timeout: 10))
        shot("01-role")
        button(app, "This device listens").tap()
        allowSystemPrompts()
        for (i, invite) in invites.enumerated() {
            button(app, "Add a station").tap()
            allowSystemPrompts() // local network, on the first browse
            if i == 0 {
                XCTAssertTrue(button(app, "Use a link").waitForExistence(timeout: 5))
                shot("04-add-station")
            }
            button(app, "Use a link").tap()
            let field = app.textFields["Invitation link"]
            XCTAssertTrue(field.waitForExistence(timeout: 5))
            field.tap()
            field.typeText(invite)
            button(app, "Pair with link").tap()
            let same = button(app, "Same, continue")
            XCTAssertTrue(same.waitForExistence(timeout: 60), "verification number shown")
            same.tap()
        }
        let live = app.staticTexts.containing(NSPredicate(format: "label CONTAINS 'now'")).firstMatch
        XCTAssertTrue(live.waitForExistence(timeout: 60), "live level on a tile")
        sleep(UInt32(env["NESTWATCH_SHOT_SETTLE"].flatMap(UInt32.init) ?? 8))
        shot("02-listening")
    }

    func testStationScreens() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-role", "none", "-relay-asked", "YES", "-AppleLanguages", "(en)"]
        app.launch()
        button(app, "This device watches").tap()
        allowSystemPrompts()
        if button(app, "Start the station").waitForExistence(timeout: 5) { button(app, "Start the station").tap() }
        for t in ["Continue", "Continue"] where button(app, t).waitForExistence(timeout: 3) { button(app, t).tap() }
        allowSystemPrompts()
        if !button(app, "Pair a listener").waitForExistence(timeout: 30) { shot("99-debug") }
        XCTAssertTrue(button(app, "Pair a listener").exists)
        sleep(4)
        shot("05-station")
        button(app, "Pair a listener").tap()
        let code = app.staticTexts.matching(NSPredicate(format: "label MATCHES '^[A-Z0-9]{5}-[A-Z0-9]{5}$'")).firstMatch
        XCTAssertTrue(code.waitForExistence(timeout: 20))
        sleep(1)
        shot("03-pair")
    }
}
