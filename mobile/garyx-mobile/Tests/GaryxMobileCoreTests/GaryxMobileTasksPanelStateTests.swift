import XCTest
@testable import GaryxMobileCore

final class GaryxMobileTasksPanelStateTests: XCTestCase {
    func testSourceFilterSuccessPopulatesRowsAndLoadedPhase() {
        var state = GaryxMobileTasksPanelState()

        state.setSourceFilter(threadId: " thread::source ")
        XCTAssertEqual(state.sourceThreadFilterId, "thread::source")
        XCTAssertEqual(state.sourceThreadFilterLoadPhase, .loading)
        XCTAssertTrue(state.beginSourceFilterRefresh(threadId: "thread::source"))

        let task = task(id: "#TASK-1", sourceThreadId: "thread::source")
        XCTAssertTrue(state.applySourceFilterResult(threadId: "thread::source", tasks: [task]))

        XCTAssertEqual(state.sourceThreadFilteredTasks, [task])
        XCTAssertEqual(state.sourceThreadFilterLoadPhase, .loaded)
    }

    func testSourceFilterFailureSetsFailedPhaseAndClearsRows() {
        var state = GaryxMobileTasksPanelState(
            sourceThreadFilterId: "thread::source",
            sourceThreadFilteredTasks: [task(id: "#TASK-1", sourceThreadId: "thread::source")],
            sourceThreadFilterLoadPhase: .loaded
        )

        XCTAssertTrue(state.beginSourceFilterRefresh(threadId: "thread::source"))
        XCTAssertTrue(state.applySourceFilterFailure(threadId: "thread::source", message: "network failed"))

        XCTAssertEqual(state.sourceThreadFilteredTasks, [])
        XCTAssertEqual(state.sourceThreadFilterLoadPhase, .failed("network failed"))
    }

    func testStaleSourceFilterResponseIsDroppedAfterFilterSwitch() {
        var state = GaryxMobileTasksPanelState()
        state.setSourceFilter(threadId: "thread::old")
        XCTAssertTrue(state.beginSourceFilterRefresh(threadId: "thread::old"))

        state.setSourceFilter(threadId: "thread::new")

        XCTAssertFalse(
            state.applySourceFilterResult(
                threadId: "thread::old",
                tasks: [task(id: "#TASK-1", sourceThreadId: "thread::old")]
            )
        )
        XCTAssertEqual(state.sourceThreadFilterId, "thread::new")
        XCTAssertEqual(state.sourceThreadFilteredTasks, [])
        XCTAssertEqual(state.sourceThreadFilterLoadPhase, .loading)
    }

    func testClearSourceFilterRestoresAllTasksVisibility() {
        let allTasks = [
            task(id: "#TASK-1", sourceThreadId: "thread::source"),
            task(id: "#TASK-2", sourceThreadId: "thread::other"),
        ]
        var state = GaryxMobileTasksPanelState()
        state.setSourceFilter(threadId: "thread::source")
        _ = state.applySourceFilterResult(threadId: "thread::source", tasks: [allTasks[0]])

        XCTAssertEqual(state.visibleTasks(from: allTasks), [allTasks[0]])

        state.clearSourceFilter()

        XCTAssertNil(state.sourceThreadFilterId)
        XCTAssertEqual(state.sourceThreadFilteredTasks, [])
        XCTAssertEqual(state.sourceThreadFilterLoadPhase, .idle)
        XCTAssertEqual(state.visibleTasks(from: allTasks), allTasks)
    }

    func testApplyDeletionRemovesMatchingFilteredTaskOnly() {
        let first = task(id: "#TASK-1", sourceThreadId: "thread::source")
        let second = task(id: "#TASK-2", sourceThreadId: "thread::source")
        var state = GaryxMobileTasksPanelState(
            sourceThreadFilterId: "thread::source",
            sourceThreadFilteredTasks: [first, second],
            sourceThreadFilterLoadPhase: .loaded
        )

        state.applyDeletion(taskId: "#TASK-missing")
        XCTAssertEqual(state.sourceThreadFilteredTasks, [first, second])

        state.applyDeletion(taskId: " #TASK-1 ")
        XCTAssertEqual(state.sourceThreadFilteredTasks, [second])
    }

    func testSourceThreadTaskHelperFiltersBySourceThreadId() {
        let sourceTask = task(id: "#TASK-1", sourceThreadId: "thread::source")
        let otherTask = task(id: "#TASK-2", sourceThreadId: "thread::other")
        let noSourceTask = task(id: "#TASK-3", sourceThreadId: nil)
        let tasks = [sourceTask, otherTask, noSourceTask]

        XCTAssertEqual(
            GaryxMobileTasksPanelState.sourceThreadTasks(tasks, sourceThreadId: " thread::source "),
            [sourceTask]
        )
        XCTAssertEqual(GaryxMobileTasksPanelState.sourceThreadTasks(tasks, sourceThreadId: nil), tasks)
    }

    func testViewTasksMenuTitleIncludesCount() {
        XCTAssertEqual(GaryxMobileTasksPanelState.viewTasksMenuTitle(count: 0), "View Tasks (0)")
        XCTAssertEqual(GaryxMobileTasksPanelState.viewTasksMenuTitle(count: 3), "View Tasks (3)")
        XCTAssertEqual(GaryxMobileTasksPanelState.viewTasksMenuTitle(count: -1), "View Tasks (0)")
    }

    func testMergedTasksUsesServerRowsAndPreservesUnrelatedRows() {
        let existingFirst = task(id: "#TASK-1", title: "Old", sourceThreadId: "thread::source")
        let existingSecond = task(id: "#TASK-2", title: "Keep", sourceThreadId: "thread::other")
        let incomingFirst = task(id: "#TASK-1", title: "New", sourceThreadId: "thread::source")
        let incomingThird = task(id: "#TASK-3", title: "Discovered", sourceThreadId: "thread::source")

        let merged = GaryxMobileTasksPanelState.mergedTasks(
            existing: [existingFirst, existingSecond],
            incoming: [incomingFirst, incomingThird]
        )

        XCTAssertEqual(merged.map(\.id), ["#TASK-3", "#TASK-1", "#TASK-2"])
        XCTAssertEqual(merged.first(where: { $0.id == "#TASK-1" })?.title, "New")
        XCTAssertEqual(merged.first(where: { $0.id == "#TASK-2" })?.title, "Keep")
    }

    private func task(
        id: String,
        title: String = "Task",
        sourceThreadId: String?
    ) -> GaryxTaskSummary {
        GaryxTaskSummary(
            id: id,
            threadId: "thread::\(id)",
            number: 1,
            title: title,
            status: .todo,
            source: sourceThreadId.map { GaryxTaskSource(threadId: $0) }
        )
    }
}
