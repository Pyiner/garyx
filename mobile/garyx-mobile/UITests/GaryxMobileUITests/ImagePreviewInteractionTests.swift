import XCTest

final class ImagePreviewInteractionTests: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testSingleImagePullDownDismissesAtFitScale() {
        let app = launchPreview(mode: "single")
        pullDownPreview(in: app)
        XCTAssertTrue(
            app.staticTexts["Image preview dismissed"].waitForExistence(timeout: 3),
            "a fit-to-screen single image should dismiss with a downward drag"
        )
    }

    func testGalleryPullDownDismissesAtFitScale() {
        let app = launchPreview(mode: "gallery")
        pullDownPreview(in: app)
        XCTAssertTrue(
            app.staticTexts["Image preview dismissed"].waitForExistence(timeout: 3),
            "a fit-to-screen gallery page should dismiss with the same downward drag"
        )
    }

    func testGalleryHorizontalPagingStillWorks() {
        let app = launchPreview(mode: "gallery")
        XCTAssertTrue(app.staticTexts["Image 1 of 2"].waitForExistence(timeout: 3))

        app.swipeLeft()

        XCTAssertTrue(
            app.staticTexts["Image 2 of 2"].waitForExistence(timeout: 3),
            "the dismiss recognizer must leave horizontal drags to the gallery pager"
        )
    }

    func testGalleryPullDownDismissesAfterZoomReturnsToFitScale() {
        let app = launchPreview(mode: "gallery")
        zoomInAndReturnToFitScale(in: app)
        pullDownPreview(in: app)
        XCTAssertTrue(
            app.staticTexts["Image preview dismissed"].waitForExistence(timeout: 3),
            "gallery pull-down dismissal should recover after zoom resets"
        )
    }

    func testSingleImagePullDownDismissesAfterZoomReturnsToFitScale() {
        let app = launchPreview(mode: "single")
        zoomInAndReturnToFitScale(in: app)
        pullDownPreview(in: app)
        XCTAssertTrue(
            app.staticTexts["Image preview dismissed"].waitForExistence(timeout: 3),
            "single-image pull-down dismissal should recover after zoom resets"
        )
    }

    func testSaveAddsInlineImageToPhotos() {
        let app = launchPreview(mode: "single")
        let saveButton = app.buttons["Save image to Photos"]
        XCTAssertTrue(saveButton.waitForExistence(timeout: 3))

        saveButton.tap()

        XCTAssertTrue(
            app.descendants(matching: .any)["Image saved to Photos"].waitForExistence(timeout: 5),
            "PhotoKit should report that the inline preview image was added"
        )
    }

    func testCancelledSlowGatewaySaveCannotOverwriteNextPageSaveState() {
        let app = launchPreview(mode: "cancellation-race-gallery")
        let saveButton = app.buttons["Save image to Photos"]
        XCTAssertTrue(saveButton.waitForExistence(timeout: 3))
        saveButton.tap()

        app.swipeLeft()
        XCTAssertTrue(app.staticTexts["Image 2 of 2"].waitForExistence(timeout: 3))
        saveButton.tap()
        XCTAssertTrue(
            app.descendants(matching: .any)["Image saved to Photos"].waitForExistence(timeout: 5)
        )

        XCTAssertFalse(app.alerts["Unable to Save Image"].exists)
        XCTAssertEqual(
            saveButton.value as? String,
            "Saved",
            "a cancelled save must not clear a newer save's state"
        )
    }

    func testGalleryFailureAlertBlocksPullDownDismissBridge() {
        let app = launchPreview(mode: "failing-gallery")
        let saveButton = app.buttons["Save image to Photos"]
        XCTAssertTrue(saveButton.waitForExistence(timeout: 3))
        saveButton.tap()

        let alert = app.alerts["Unable to Save Image"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5))
        pullDownPreview(in: app)

        XCTAssertTrue(alert.exists, "the save failure alert should remain presented")
        XCTAssertFalse(
            app.staticTexts["Image preview dismissed"].exists,
            "a pull-down over an alert must not dismiss the underlying preview"
        )
    }

    private func launchPreview(mode: String) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["GARYX_MOBILE_IMAGE_PREVIEW_FIXTURE"] = mode
        app.launch()
        XCTAssertTrue(app.images["Full screen image"].firstMatch.waitForExistence(timeout: 10))
        return app
    }

    private func pullDownPreview(in app: XCUIApplication) {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.38))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.76))
        start.press(forDuration: 0.1, thenDragTo: end)
    }

    private func zoomInAndReturnToFitScale(in app: XCUIApplication) {
        let image = app.images["Full screen image"].firstMatch
        image.doubleTap()
        image.doubleTap()
    }
}
