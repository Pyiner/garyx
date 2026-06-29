import XCTest
@testable import GaryxMobileCore

/// Acceptance tests for #TASK-1449 symptom 2: returning to the foreground must
/// converge the open thread to the server's latest state — reconnecting first
/// when the connection dropped, and always resyncing + restarting the open
/// thread's stream.
final class GaryxForegroundSyncPlanTests: XCTestCase {
    func testReadyWithSelectedThreadResyncsAndRestartsStream() {
        XCTAssertEqual(
            GaryxForegroundSyncPlan.plan(connectionState: .ready(version: nil), selectedThreadId: "thread::T"),
            GaryxForegroundSyncPlan(reconnect: false, resyncOpenThread: true, restartStream: true)
        )
    }

    func testReadyWithoutSelectedThreadDoesNothingPerThread() {
        XCTAssertEqual(
            GaryxForegroundSyncPlan.plan(connectionState: .ready(version: "v"), selectedThreadId: nil),
            GaryxForegroundSyncPlan(reconnect: false, resyncOpenThread: false, restartStream: false)
        )
    }

    func testDisconnectedWithSelectedThreadReconnectsThenResyncs() {
        XCTAssertEqual(
            GaryxForegroundSyncPlan.plan(connectionState: .disconnected, selectedThreadId: "thread::T"),
            GaryxForegroundSyncPlan(reconnect: true, resyncOpenThread: true, restartStream: true)
        )
    }

    func testFailedWithSelectedThreadReconnectsThenResyncs() {
        XCTAssertEqual(
            GaryxForegroundSyncPlan.plan(connectionState: .failed("boom"), selectedThreadId: "thread::T"),
            GaryxForegroundSyncPlan(reconnect: true, resyncOpenThread: true, restartStream: true)
        )
    }

    func testDisconnectedWithoutThreadStillReconnects() {
        XCTAssertEqual(
            GaryxForegroundSyncPlan.plan(connectionState: .disconnected, selectedThreadId: nil),
            GaryxForegroundSyncPlan(reconnect: true, resyncOpenThread: false, restartStream: false)
        )
    }

    func testCheckingDefersToInFlightConnect() {
        XCTAssertEqual(
            GaryxForegroundSyncPlan.plan(connectionState: .checking, selectedThreadId: "thread::T"),
            GaryxForegroundSyncPlan(reconnect: false, resyncOpenThread: false, restartStream: false)
        )
    }

    func testBlankSelectedThreadIdTreatedAsNoThread() {
        XCTAssertEqual(
            GaryxForegroundSyncPlan.plan(connectionState: .ready(version: nil), selectedThreadId: "   "),
            GaryxForegroundSyncPlan(reconnect: false, resyncOpenThread: false, restartStream: false)
        )
    }
}
