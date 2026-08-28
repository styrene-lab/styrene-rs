import XCTest

final class CrossPlatformHubUITests: XCTestCase {
    func testIOSQueuesCorrelatedMessageForAndroid() throws {
        let message = try XCTUnwrap(ProcessInfo.processInfo.environment["STYRENE_INTEGRATION_MESSAGE"])
        let profile = try XCTUnwrap(ProcessInfo.processInfo.environment["STYRENE_IOS_PROFILE"])
        let app = XCUIApplication()
        app.launchArguments = [
            "--styrene-integration-profile", profile,
            "--styrene-hub-address", "127.0.0.1:4242",
            "--styrene-display-name", "iOS A",
            "--styrene-reset-state",
        ]
        app.launch()

        XCTAssertTrue(app.staticTexts["Transport active"].waitForExistence(timeout: 20))
        app.tabBars.buttons["Network"].tap()
        app.buttons["Announce"].tap()
        app.tabBars.buttons["People"].tap()
        app.buttons["Discovered"].tap()

        let android = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Android A")).firstMatch
        XCTAssertTrue(android.waitForExistence(timeout: 30), "Android peer was not discovered")
        android.tap()
        let messageButton = app.buttons["Message"]
        XCTAssertTrue(messageButton.waitForExistence(timeout: 5))
        messageButton.tap()

        let composer = app.textFields["messages.composer"]
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        composer.tap()
        composer.typeText(message)
        app.buttons["messages.send"].tap()

        XCTAssertTrue(app.staticTexts[message].waitForExistence(timeout: 15))
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "ios-message-queued"
        attachment.lifetime = .keepAlways
        add(attachment)
        print("STYRENE_EVIDENCE ios_message_queued=\(message)")
    }
}
