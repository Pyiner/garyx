import Foundation
import SwiftUI
import UIKit
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxAgentDetailPresentationTests: XCTestCase {
    override func tearDown() {
        GaryxAgentDetailURLProtocolStub.reset()
        super.tearDown()
    }

    func testAgentEditUsesSingleStableFullScreenOwnerAndRemainsPresented() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("garyx-agent-detail-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: directory) }

        let suiteName = "GaryxAgentDetailPresentation-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let agent = GaryxAgentSummary(
            id: "test-agent",
            displayName: "Test Agent",
            providerType: "codex_app_server",
            model: "test-model",
            providerEnv: ["SYNTHETIC": "value"],
            systemPrompt: "Synthetic test instructions.",
            updatedAt: "2026-07-22T00:00:00Z"
        )
        GaryxAgentDetailURLProtocolStub.configure(responseData: Data(
            #"{"agent_id":"test-agent","display_name":"Test Agent","provider_type":"codex_app_server","model":"test-model","provider_env":{"SYNTHETIC":"value"},"system_prompt":"Synthetic test instructions.","built_in":false,"standalone":true,"enabled":true,"updated_at":"2026-07-22T00:00:00Z"}"#.utf8
        ))
        let sessionConfiguration = URLSessionConfiguration.ephemeral
        sessionConfiguration.protocolClasses = [GaryxAgentDetailURLProtocolStub.self]
        let session = URLSession(configuration: sessionConfiguration)
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        let model = GaryxMobileModel(
            defaults: defaults,
            gatewayClientFactory: { configuration in
                GaryxGatewayClient(
                    configuration: configuration,
                    session: session,
                    retryPolicy: .disabled
                )
            },
            composerPayloadCoordinator: coordinator
        )
        model.agents = [agent]
        let navigation = GaryxAgentDetailNavigation()
        let controller = UIHostingController(
            rootView: GaryxAgentDetailPresentationReproRoot(
                model: model,
                navigation: navigation
            )
            .preferredColorScheme(.light)
        )
        let window = try makeAgentDetailTestWindow()
        window.rootViewController = controller
        window.isHidden = false
        defer {
            controller.dismiss(animated: false)
            window.isHidden = true
            window.rootViewController = nil
        }
        controller.view.frame = window.bounds
        window.layoutIfNeeded()

        model.selectedAgentDetail = agent
        let detailDidPresent = await waitUntil {
            window.layoutIfNeeded()
            return self.presentationDepth(from: controller) == 1
        }
        XCTAssertTrue(
            detailDidPresent,
            "Agent Detail must establish its sole full-screen presentation owner"
        )

        navigation.openEdit()
        try await Task.sleep(for: .seconds(5))
        window.layoutIfNeeded()

        let depth = presentationDepth(from: controller)
        print(
            "AGENT_DETAIL_EDIT_PRESENTATION selected=\(model.selectedAgentDetail?.id ?? "nil") "
                + "edit=\(navigation.showsEdit) presentationDepth=\(depth)"
        )
        XCTAssertEqual(model.selectedAgentDetail?.id, agent.id)
        XCTAssertTrue(navigation.showsEdit, "Edit must remain active after the former failure window")
        XCTAssertTrue(
            GaryxAgentDetailURLProtocolStub.requestedPaths.contains(
                "/api/custom-agents/test-agent"
            ),
            "the pushed production Edit page must mount and load its authoritative agent"
        )
        XCTAssertEqual(
            depth,
            1,
            "Detail and Edit must share one UIKit full-screen presentation owner"
        )

        navigation.returnToDetail()
        let editDidReturnToDetail = await waitUntil {
            window.layoutIfNeeded()
            return !navigation.showsEdit
                && model.selectedAgentDetail?.id == agent.id
                && self.presentationDepth(from: controller) == 1
        }
        XCTAssertTrue(
            editDidReturnToDetail,
            "Cancel/back must pop Edit without dismissing Agent Detail"
        )

        model.selectedAgentDetail = nil
        let detailDidDismiss = await waitUntil {
            window.layoutIfNeeded()
            return self.presentationDepth(from: controller) == 0
        }
        XCTAssertTrue(detailDidDismiss, "Done must dismiss the sole Detail presentation")
    }

    private func waitUntil(
        timeout: Duration = .seconds(5),
        condition: @escaping @MainActor () -> Bool
    ) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            if condition() { return true }
            await Task.yield()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return condition()
    }

    private func presentationDepth(from controller: UIViewController) -> Int {
        guard let presented = controller.presentedViewController else { return 0 }
        return 1 + presentationDepth(from: presented)
    }
}

private struct GaryxAgentDetailPresentationReproRoot: View {
    @ObservedObject var model: GaryxMobileModel
    let navigation: GaryxAgentDetailNavigation

    var body: some View {
        Color.white
            .garyxFullScreenCover(item: $model.selectedAgentDetail) { agent in
                GaryxAgentDetailPresentation(agent: agent, navigation: navigation)
            }
            .environmentObject(model)
    }
}

private final class GaryxAgentDetailURLProtocolStub: URLProtocol {
    private static let lock = NSLock()
    private static var responseData: Data?
    private static var paths: [String] = []

    static var requestedPaths: [String] {
        lock.withLock { paths }
    }

    static func configure(responseData: Data) {
        lock.withLock {
            self.responseData = responseData
            paths = []
        }
    }

    static func reset() {
        lock.withLock {
            responseData = nil
            paths = []
        }
    }

    override class func canInit(with request: URLRequest) -> Bool { true }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let url = request.url, let data = Self.record(request: request) else {
            client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
            return
        }
        let response = HTTPURLResponse(
            url: url,
            statusCode: 200,
            httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    private static func record(request: URLRequest) -> Data? {
        lock.withLock {
            if let path = request.url?.path {
                paths.append(path)
            }
            return responseData
        }
    }
}

@MainActor
private func makeAgentDetailTestWindow() throws -> UIWindow {
    let scene = try XCTUnwrap(
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first
    )
    let window = UIWindow(windowScene: scene)
    window.frame = CGRect(x: 0, y: 0, width: 393, height: 852)
    return window
}
