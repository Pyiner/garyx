import XCTest
@testable import GaryxMobileCore

final class GaryxGatewaySwitcherPresentationTests: XCTestCase {
    func testIdentityFallsBackToGaryxWhenUnconfigured() {
        let identity = GaryxGatewaySwitcherPresentation.identity(
            gatewayURL: "   ",
            profileLabel: nil,
            connectionState: .disconnected
        )

        XCTAssertEqual(identity.title, "Garyx")
        XCTAssertNil(identity.subtitle)
        XCTAssertEqual(identity.status, .notConnected)
        XCTAssertFalse(identity.isInteractive)
    }

    func testIdentityUsesProfileLabelWithHostAndStatusSubtitle() {
        let identity = GaryxGatewaySwitcherPresentation.identity(
            gatewayURL: "https://gateway.example.test/",
            profileLabel: "Home Mac mini",
            connectionState: .ready(version: "1.2.3")
        )

        XCTAssertEqual(identity.title, "Home Mac mini")
        XCTAssertEqual(identity.subtitle, "gateway.example.test · Connected")
        XCTAssertEqual(identity.status, .connected)
        XCTAssertTrue(identity.isInteractive)
    }

    func testIdentityWithoutProfileLabelUsesHostTitleAndStatusOnlySubtitle() {
        let identity = GaryxGatewaySwitcherPresentation.identity(
            gatewayURL: "http://127.0.0.1:31337",
            profileLabel: "  ",
            connectionState: .checking
        )

        XCTAssertEqual(identity.title, "127.0.0.1:31337")
        XCTAssertEqual(identity.subtitle, "Connecting")
        XCTAssertEqual(identity.status, .connecting)
        XCTAssertTrue(identity.isInteractive)
    }

    func testStatusMappingCoversAllConnectionStates() {
        XCTAssertEqual(GaryxGatewaySwitcherPresentation.status(for: .ready(version: nil)), .connected)
        XCTAssertEqual(GaryxGatewaySwitcherPresentation.status(for: .checking), .connecting)
        XCTAssertEqual(GaryxGatewaySwitcherPresentation.status(for: .failed("boom")), .failed)
        XCTAssertEqual(GaryxGatewaySwitcherPresentation.status(for: .disconnected), .notConnected)

        XCTAssertEqual(GaryxGatewaySwitcherPresentation.statusLabel(for: .ready(version: nil)), "Connected")
        XCTAssertEqual(GaryxGatewaySwitcherPresentation.statusLabel(for: .checking), "Connecting")
        XCTAssertEqual(GaryxGatewaySwitcherPresentation.statusLabel(for: .failed("boom")), "Connection failed")
        XCTAssertEqual(GaryxGatewaySwitcherPresentation.statusLabel(for: .disconnected), "Not connected")
    }

    func testRowsMoveCurrentProfileToFront() {
        let rows = GaryxGatewaySwitcherPresentation.rows(
            profiles: [
                makeProfile(label: "Office", gatewayUrl: "https://office.example.test"),
                makeProfile(label: "Home", gatewayUrl: "https://home.example.test"),
            ],
            currentGatewayURL: "https://home.example.test/"
        )

        XCTAssertEqual(rows.map(\.title), ["Home", "Office"])
        XCTAssertEqual(rows.map(\.isCurrent), [true, false])
        XCTAssertNotNil(rows[0].profileId)
    }

    func testRowsInsertSyntheticCurrentRowForUnsavedGateway() {
        let rows = GaryxGatewaySwitcherPresentation.rows(
            profiles: [
                makeProfile(label: "Office", gatewayUrl: "https://office.example.test"),
            ],
            currentGatewayURL: "http://10.0.0.42:31337"
        )

        XCTAssertEqual(rows.count, 2)
        XCTAssertTrue(rows[0].isCurrent)
        XCTAssertNil(rows[0].profileId)
        XCTAssertEqual(rows[0].title, "10.0.0.42:31337")
        XCTAssertEqual(rows[0].subtitle, "http://10.0.0.42:31337")
        XCTAssertEqual(rows[1].title, "Office")
        XCTAssertFalse(rows[1].isCurrent)
    }

    func testRowsWithoutCurrentGatewayKeepProfileOrder() {
        let rows = GaryxGatewaySwitcherPresentation.rows(
            profiles: [
                makeProfile(label: "Office", gatewayUrl: "https://office.example.test"),
                makeProfile(label: "Home", gatewayUrl: "https://home.example.test"),
            ],
            currentGatewayURL: ""
        )

        XCTAssertEqual(rows.map(\.title), ["Office", "Home"])
        XCTAssertEqual(rows.map(\.isCurrent), [false, false])
    }

    private func makeProfile(label: String, gatewayUrl: String) -> GaryxGatewayProfile {
        GaryxGatewayProfile(
            id: GaryxGatewayProfileStorage.stableId(for: gatewayUrl),
            label: label,
            gatewayUrl: gatewayUrl,
            updatedAt: Date(timeIntervalSince1970: 1000),
            hasToken: false
        )
    }
}
