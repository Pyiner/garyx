import XCTest
@testable import GaryxMobileCore

final class GaryxMobileResourceStateTests: XCTestCase {
    func testFirstRefreshUsesLoadingThenLoaded() {
        var state = GaryxMobileResourceState(value: [String]())

        state.beginRefresh()
        XCTAssertEqual(state.phase, .loading)
        XCTAssertTrue(state.isRefreshing)

        let date = Date(timeIntervalSince1970: 10)
        state.completeRefresh(["/workspace/garyx"], at: date)

        XCTAssertEqual(state.value, ["/workspace/garyx"])
        XCTAssertEqual(state.phase, .loaded)
        XCTAssertFalse(state.isRefreshing)
        XCTAssertEqual(state.lastUpdatedAt, date)
        XCTAssertNil(state.lastFailureMessage)
    }

    func testRefreshFailureKeepsStaleValueVisible() {
        var state = GaryxMobileResourceState(value: ["/workspace/garyx"], phase: .loaded)

        state.beginRefresh()
        XCTAssertEqual(state.phase, .loaded)
        XCTAssertTrue(state.isRefreshing)

        state.failRefresh("Gateway unavailable", keepingStaleValue: true)

        XCTAssertEqual(state.value, ["/workspace/garyx"])
        XCTAssertEqual(state.phase, .loaded)
        XCTAssertFalse(state.isRefreshing)
        XCTAssertEqual(state.lastFailureMessage, "Gateway unavailable")
    }

    func testRefreshFailureWithoutStaleValueFails() {
        var state = GaryxMobileResourceState(value: [String]())

        state.beginRefresh()
        state.failRefresh("Gateway unavailable", keepingStaleValue: false)

        XCTAssertEqual(state.value, [])
        XCTAssertEqual(state.phase, .failed("Gateway unavailable"))
        XCTAssertFalse(state.isRefreshing)
    }

    func testRetryAfterFailureReturnsToLoading() {
        var state = GaryxMobileResourceState(value: [String]())

        state.beginRefresh()
        state.failRefresh("Gateway unavailable", keepingStaleValue: false)
        state.beginRefresh()

        XCTAssertEqual(state.phase, .loading)
        XCTAssertTrue(state.isRefreshing)
        XCTAssertEqual(state.lastFailureMessage, "Gateway unavailable")
    }

    func testRestoreHydratesCachedValueWithoutFreshTimestamp() {
        var state = GaryxMobileResourceState(
            value: [String](),
            phase: .failed("Old failure"),
            lastUpdatedAt: Date(timeIntervalSince1970: 1),
            lastFailureMessage: "Old failure",
            isRefreshing: true
        )

        state.restore(["/workspace/garyx"])

        XCTAssertEqual(state.value, ["/workspace/garyx"])
        XCTAssertEqual(state.phase, .loaded)
        XCTAssertNil(state.lastUpdatedAt)
        XCTAssertNil(state.lastFailureMessage)
        XCTAssertFalse(state.isRefreshing)
    }

    func testReplaceRecordsDirectMutationTimestamp() {
        var state = GaryxMobileResourceState(value: ["/workspace/old"], phase: .loaded)
        let date = Date(timeIntervalSince1970: 20)

        state.replace(["/workspace/new"], at: date)

        XCTAssertEqual(state.value, ["/workspace/new"])
        XCTAssertEqual(state.phase, .loaded)
        XCTAssertEqual(state.lastUpdatedAt, date)
        XCTAssertNil(state.lastFailureMessage)
        XCTAssertFalse(state.isRefreshing)
    }

    func testResetClearsTransientState() {
        var state = GaryxMobileResourceState(
            value: ["/workspace/garyx"],
            phase: .loaded,
            lastUpdatedAt: Date(timeIntervalSince1970: 30),
            lastFailureMessage: "Old failure",
            isRefreshing: true
        )

        state.reset(to: [])

        XCTAssertEqual(state.value, [])
        XCTAssertEqual(state.phase, .idle)
        XCTAssertNil(state.lastUpdatedAt)
        XCTAssertNil(state.lastFailureMessage)
        XCTAssertFalse(state.isRefreshing)
    }

    func testCompleteRefreshClearsPreviousFailure() {
        var state = GaryxMobileResourceState(value: [String]())

        state.beginRefresh()
        state.failRefresh("Gateway unavailable", keepingStaleValue: false)
        state.beginRefresh()
        state.completeRefresh(["/workspace/garyx"], at: Date(timeIntervalSince1970: 40))

        XCTAssertEqual(state.value, ["/workspace/garyx"])
        XCTAssertEqual(state.phase, .loaded)
        XCTAssertNil(state.lastFailureMessage)
        XCTAssertFalse(state.isRefreshing)
    }
}
