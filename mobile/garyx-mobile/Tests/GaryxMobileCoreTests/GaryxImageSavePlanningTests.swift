import XCTest
@testable import GaryxMobileCore

final class GaryxImageSavePlanningTests: XCTestCase {
    private let png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01])
    private let jpeg = Data([0xFF, 0xD8, 0xFF, 0xE0, 0x01])

    func testCandidatePriorityMatchesPreviewLoadingOrder() {
        let request = GaryxImageSaveRequest(
            title: "diagram",
            dataURL: " inline ",
            filePath: " /local/image.png ",
            gatewayFilePath: " /gateway/image.png ",
            remoteURL: " https://example.test/image.png "
        )
        XCTAssertEqual(
            request.candidates,
            [
                .inlineData("inline"),
                .localFile("/local/image.png"),
                .gatewayFile("/gateway/image.png"),
                .remoteURL("https://example.test/image.png"),
            ]
        )
    }

    func testInlineDataURLPathProducesOriginalPNGBytes() async throws {
        let raw = "data:image/png;base64,\(png.base64EncodedString())"
        let payload = try await load(GaryxImageSaveRequest(title: "chart", dataURL: raw))
        XCTAssertEqual(payload.data, png)
        XCTAssertEqual(payload.originalFilename, "chart.png")
        XCTAssertEqual(payload.uniformTypeIdentifier, "public.png")
    }

    func testLocalFilePathUsesInjectedFileReader() async throws {
        let request = GaryxImageSaveRequest(title: "Image", filePath: "/tmp/source.png")
        let payload = try await GaryxImageSaveLoader.load(
            request,
            readFile: { path in
                XCTAssertEqual(path, "/tmp/source.png")
                return self.png
            },
            resolveGatewayDataURL: { _ in XCTFail("gateway must not be used"); return nil },
            fetchRemote: { _ in XCTFail("remote must not be used"); throw StubError.unexpected }
        )
        XCTAssertEqual(payload.originalFilename, "source.png")
        XCTAssertEqual(payload.data, png)
    }

    func testGatewayFilePathUsesResolvedDataURL() async throws {
        let request = GaryxImageSaveRequest(title: "gateway preview", gatewayFilePath: "/workspace/output.png")
        let payload = try await GaryxImageSaveLoader.load(
            request,
            readFile: { _ in XCTFail("file must not be used"); throw StubError.unexpected },
            resolveGatewayDataURL: { path in
                XCTAssertEqual(path, "/workspace/output.png")
                return "data:image/png;base64,\(self.png.base64EncodedString())"
            },
            fetchRemote: { _ in XCTFail("remote must not be used"); throw StubError.unexpected }
        )
        XCTAssertEqual(payload.originalFilename, "gateway preview.png")
        XCTAssertEqual(payload.data, png)
    }

    func testRemoteURLPathUsesResponseMetadataAndSniffedFormat() async throws {
        let request = GaryxImageSaveRequest(title: "Image", remoteURL: "https://example.test/assets/photo")
        let payload = try await GaryxImageSaveLoader.load(
            request,
            readFile: { _ in XCTFail("file must not be used"); throw StubError.unexpected },
            resolveGatewayDataURL: { _ in XCTFail("gateway must not be used"); return nil },
            fetchRemote: { url in
                XCTAssertEqual(url, "https://example.test/assets/photo")
                return GaryxImageSaveRemoteResource(
                    data: self.jpeg,
                    mimeType: "image/png",
                    suggestedFilename: "server-name.png"
                )
            }
        )
        XCTAssertEqual(payload.originalFilename, "server-name.jpg")
        XCTAssertEqual(payload.uniformTypeIdentifier, "public.jpeg")
        XCTAssertEqual(payload.data, jpeg)
    }

    func testUnavailableHigherPrioritySourceFallsThrough() async throws {
        let request = GaryxImageSaveRequest(
            title: "fallback",
            dataURL: "not-base64",
            filePath: "/missing.png",
            gatewayFilePath: "/gateway.png"
        )
        let payload = try await GaryxImageSaveLoader.load(
            request,
            readFile: { _ in throw StubError.unavailable },
            resolveGatewayDataURL: { _ in self.png.base64EncodedString() },
            fetchRemote: { _ in throw StubError.unexpected }
        )
        XCTAssertEqual(payload.data, png)
        XCTAssertEqual(payload.originalFilename, "fallback.png")
    }

    func testFilenameSanitizationAndMagicBytesOverrideMismatchedExtension() {
        let payload = GaryxImageSavePayloadFactory.makePayload(
            data: jpeg,
            title: "  report:final.png  ",
            sourceName: nil,
            mediaType: "image/png"
        )
        XCTAssertEqual(payload?.originalFilename, "report-final.jpg")
        XCTAssertEqual(payload?.uniformTypeIdentifier, "public.jpeg")
    }

    func testNonHTTPRemoteURLIsRejected() {
        let request = GaryxImageSaveRequest(title: "x", remoteURL: "file:///tmp/image.png")
        XCTAssertTrue(request.candidates.isEmpty)
    }

    private func load(_ request: GaryxImageSaveRequest) async throws -> GaryxImageSavePayload {
        try await GaryxImageSaveLoader.load(
            request,
            readFile: { _ in XCTFail("file must not be used"); throw StubError.unexpected },
            resolveGatewayDataURL: { _ in XCTFail("gateway must not be used"); return nil },
            fetchRemote: { _ in XCTFail("remote must not be used"); throw StubError.unexpected }
        )
    }

    private enum StubError: Error {
        case unavailable
        case unexpected
    }
}
