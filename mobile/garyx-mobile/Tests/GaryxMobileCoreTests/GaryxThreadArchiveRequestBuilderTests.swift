import XCTest
@testable import GaryxMobileCore

final class GaryxThreadArchiveRequestBuilderTests: XCTestCase {
    func testEndpointKeysUseMatchingThreadEndpointsAndAdditionalEndpoint() {
        let endpoints = [
            GaryxChannelEndpoint(
                endpointKey: " telegram::main::1000000001 ",
                channel: "telegram",
                accountId: "main",
                displayLabel: "Test User",
                threadId: " thread::archive "
            ),
            GaryxChannelEndpoint(
                endpointKey: "telegram::main::1000000002",
                channel: "telegram",
                accountId: "main",
                displayLabel: "Other User",
                threadId: "thread::other"
            ),
            GaryxChannelEndpoint(
                endpointKey: "api::main::loop",
                channel: "api",
                accountId: "main",
                displayLabel: "Loop",
                threadId: "thread::archive"
            )
        ]

        let keys = GaryxThreadArchiveRequestBuilder.endpointKeys(
            threadId: "thread::archive",
            endpoints: endpoints,
            additionalEndpointKey: "api::main::loop"
        )

        XCTAssertEqual(keys, [
            "api::main::loop",
            "telegram::main::1000000001"
        ])
    }
}
