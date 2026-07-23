import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxPushNotificationsTests: XCTestCase {
    func testParsesManualPayloadAndMapsThreadRoute() {
        let payload = GaryxPushPayloadParser.parse(userInfo: [
            "aps": [
                "alert": ["title": "Done", "body": "Open it"],
                "thread-id": "thread::not-the-routing-source",
            ],
            "garyx": [
                "v": 1,
                "kind": "manual",
                "thread_id": " thread::synthetic ",
            ],
        ])

        XCTAssertEqual(
            payload,
            GaryxPushPayload(
                version: 1,
                kind: .manual,
                threadID: "thread::synthetic"
            )
        )
        XCTAssertEqual(
            payload.map(GaryxPushPayloadParser.route),
            .thread("thread::synthetic")
        )
    }

    func testPayloadWithoutThreadMapsToAppHome() {
        let payload = GaryxPushPayloadParser.parse(userInfo: [
            "garyx": ["v": NSNumber(value: 1), "kind": "manual"],
        ])
        XCTAssertEqual(payload.map(GaryxPushPayloadParser.route), .appHome)
    }

    func testRejectsUnknownVersionKindAndMissingGaryxContract() {
        XCTAssertNil(
            GaryxPushPayloadParser.parse(userInfo: [
                "garyx": ["v": 2, "kind": "manual"],
            ])
        )
        XCTAssertNil(
            GaryxPushPayloadParser.parse(userInfo: [
                "garyx": ["v": 1, "kind": "automatic"],
            ])
        )
        XCTAssertNil(
            GaryxPushPayloadParser.parse(userInfo: [
                "aps": ["thread-id": "thread::must-not-route"],
            ])
        )
    }

    func testForegroundSuppressesOnlyTheCurrentlyOpenThread() {
        let payload = GaryxPushPayload(
            version: 1,
            kind: .manual,
            threadID: "thread::open"
        )
        XCTAssertEqual(
            GaryxPushPayloadParser.foregroundPresentation(
                for: payload,
                openThreadID: "thread::open"
            ),
            .suppress
        )
        XCTAssertEqual(
            GaryxPushPayloadParser.foregroundPresentation(
                for: payload,
                openThreadID: "thread::other"
            ),
            .present
        )
        XCTAssertEqual(
            GaryxPushPayloadParser.foregroundPresentation(
                for: nil,
                openThreadID: "thread::open"
            ),
            .present
        )
    }

    func testBuildEnvironmentAndTokenHexArePure() {
        XCTAssertEqual(
            GaryxPushEnvironment.forBuild(isDebugBuild: true),
            .development
        )
        XCTAssertEqual(
            GaryxPushEnvironment.forBuild(isDebugBuild: false),
            .production
        )
        XCTAssertEqual(
            GaryxPushDeviceToken.hexadecimal(Data([0x00, 0x0f, 0xa5, 0xff])),
            "000fa5ff"
        )
    }

    func testRegistrationStartsWhenGatewayAndTokenAreBothKnown() {
        let device = testDevice()
        var state = GaryxPushRegistrationState()
        XCTAssertEqual(state.handle(.gatewayChanged("gateway-a")), [])
        let actions = state.handle(.deviceTokenReceived(device))
        XCTAssertEqual(
            actions,
            [.register(GaryxPushRegistrationKey(targetID: "gateway-a", device: device))]
        )
        XCTAssertEqual(
            state.handle(.deviceTokenReceived(device)),
            [],
            "an identical callback must not duplicate an in-flight upsert"
        )
    }

    func testFailureWaitsForForegroundAndSuccessStillRefreshesOnForeground() {
        let device = testDevice()
        let key = GaryxPushRegistrationKey(targetID: "gateway-a", device: device)
        var state = GaryxPushRegistrationState()
        _ = state.handle(.gatewayChanged("gateway-a"))
        _ = state.handle(.deviceTokenReceived(device))
        XCTAssertEqual(
            state.handle(.registrationFinished(key, succeeded: false)),
            []
        )
        XCTAssertEqual(state.handle(.foregrounded), [.register(key)])
        _ = state.handle(.registrationFinished(key, succeeded: true))
        XCTAssertEqual(state.lastSuccessfulRegistration, key)
        XCTAssertEqual(state.handle(.deviceTokenReceived(device)), [])
        XCTAssertEqual(state.handle(.foregrounded), [.register(key)])
    }

    func testGatewayAndTokenChangesBestEffortUnregisterOldValues() {
        let first = testDevice(token: "first-token")
        let second = testDevice(token: "second-token")
        var state = GaryxPushRegistrationState()
        _ = state.handle(.gatewayChanged("gateway-a"))
        _ = state.handle(.deviceTokenReceived(first))
        _ = state.handle(
            .registrationFinished(
                GaryxPushRegistrationKey(targetID: "gateway-a", device: first),
                succeeded: true
            )
        )

        XCTAssertEqual(
            state.handle(.gatewayChanged("gateway-b")),
            [
                .unregister(targetID: "gateway-a", token: "first-token"),
                .register(
                    GaryxPushRegistrationKey(targetID: "gateway-b", device: first)
                ),
            ]
        )
        XCTAssertEqual(
            state.handle(.deviceTokenReceived(second)),
            [
                .unregister(targetID: "gateway-b", token: "first-token"),
                .register(
                    GaryxPushRegistrationKey(targetID: "gateway-b", device: second)
                ),
            ]
        )
    }

    func testRequestUsesGatewaySnakeCaseContract() throws {
        let request = GaryxPushDeviceRegistrationRequest(
            device: testDevice()
        )
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(request))
                as? [String: Any]
        )
        XCTAssertEqual(object["token"] as? String, "synthetic-token")
        XCTAssertEqual(object["platform"] as? String, "ios")
        XCTAssertEqual(object["environment"] as? String, "development")
        XCTAssertEqual(object["bundle_id"] as? String, "com.garyx.mobile")
        XCTAssertEqual(object["device_name"] as? String, "Test iPhone")
    }

    private func testDevice(
        token: String = "synthetic-token"
    ) -> GaryxPushDeviceRegistration {
        GaryxPushDeviceRegistration(
            token: token,
            environment: .development,
            bundleID: "com.garyx.mobile",
            deviceName: "Test iPhone"
        )
    }
}
