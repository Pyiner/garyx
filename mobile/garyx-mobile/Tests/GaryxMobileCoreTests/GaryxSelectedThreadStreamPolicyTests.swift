import XCTest
@testable import GaryxMobileCore

final class GaryxSelectedThreadStreamPolicyTests: XCTestCase {
    func testNewSelectedThreadStartsPerThreadStream() {
        XCTAssertEqual(
            GaryxSelectedThreadStreamPolicy.action(previousThreadId: nil, selectedThreadId: "thread-new"),
            .start("thread-new")
        )
    }
}
