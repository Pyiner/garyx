import XCTest

@testable import GaryxMobileCore

// Captured shape of GET /api/tasks/forest?anchor_thread_id= from the new
// gateway: thread root first, DFS pre-order, per-node depth, page-level
// active_count, done leaf retained. Synthetic data only.
private let anchoredForestFixture = """
{
  "tasks": [
    {
      "kind": "thread",
      "node_id": "thread-root:thread::origin",
      "thread_id": "thread::origin",
      "title": "Origin conversation",
      "thread_type": "chat",
      "provider_type": "codex",
      "agent_id": "codex",
      "message_count": 8,
      "last_message_preview": "Spawned task work",
      "active_run_id": null,
      "run_state": "idle",
      "updated_at": "2026-01-01T00:00:00.500Z",
      "last_active_at": "2026-01-01T00:00:00.500Z",
      "depth": 0,
      "unknown_field": true
    },
    {
      "kind": "task",
      "node_id": "task:thread::root-task",
      "parent_node_id": "thread-root:thread::origin",
      "thread_id": "thread::root-task",
      "task_id": "#TASK-40",
      "number": 40,
      "title": "Root task",
      "status": "in_progress",
      "creator": {"kind": "agent", "agent_id": "test-agent"},
      "assignee": {"kind": "agent", "agent_id": "test-agent"},
      "updated_at": "2026-01-01T00:00:02Z",
      "updated_by": {"kind": "agent", "agent_id": "test-agent"},
      "runtime_agent_id": "test-agent",
      "reply_count": 0,
      "parent_task_number": null,
      "parent_thread_id": "thread::origin",
      "active_run_id": "run::root",
      "run_state": "running",
      "last_active_at": null,
      "depth": 0
    },
    {
      "kind": "task",
      "node_id": "task:thread::review-child",
      "parent_node_id": "task:thread::root-task",
      "thread_id": "thread::review-child",
      "task_id": "#TASK-42",
      "number": 42,
      "title": "Review child",
      "status": "in_review",
      "creator": {"kind": "agent", "agent_id": "test-agent"},
      "updated_at": "2026-01-01T00:00:04Z",
      "updated_by": {"kind": "agent", "agent_id": "test-agent"},
      "runtime_agent_id": "test-agent",
      "reply_count": 0,
      "parent_task_number": 40,
      "parent_thread_id": "thread::root-task",
      "active_run_id": null,
      "run_state": "idle",
      "last_active_at": null,
      "depth": 1
    },
    {
      "kind": "task",
      "node_id": "task:thread::done-leaf",
      "parent_node_id": "task:thread::root-task",
      "thread_id": "thread::done-leaf",
      "task_id": "#TASK-41",
      "number": 41,
      "title": "Done leaf",
      "status": "done",
      "creator": {"kind": "agent", "agent_id": "test-agent"},
      "assignee": {"kind": "agent", "agent_id": "test-agent"},
      "updated_at": "2026-01-01T00:00:03Z",
      "updated_by": {"kind": "agent", "agent_id": "test-agent"},
      "runtime_agent_id": "test-agent",
      "reply_count": 0,
      "parent_task_number": 40,
      "parent_thread_id": "thread::root-task",
      "active_run_id": null,
      "run_state": "idle",
      "last_active_at": null,
      "depth": 1
    }
  ],
  "total": 4,
  "active_count": 2,
  "root_thread_ids": ["thread::origin"],
  "skipped_pinned_thread_ids": []
}
"""

final class GaryxTaskTreeSidebarTests: XCTestCase {
    private func decodedFixturePage() throws -> GaryxTaskForestPage {
        try JSONDecoder().decode(
            GaryxTaskForestPage.self,
            from: Data(anchoredForestFixture.utf8)
        )
    }

    private func pageWithoutServerLayout(_ page: GaryxTaskForestPage) -> GaryxTaskForestPage {
        var stripped = page
        stripped.activeCount = nil
        stripped.nodes = page.nodes.map { node in
            switch node {
            case .thread(var thread):
                thread.depth = nil
                return .thread(thread)
            case .task(var task):
                task.depth = nil
                return .task(task)
            }
        }
        return stripped
    }

    func testFixtureDecodesWithKindDispatchAndUnknownFieldsIgnored() throws {
        let page = try decodedFixturePage()
        XCTAssertEqual(page.total, 4)
        XCTAssertEqual(page.activeCount, 2)
        XCTAssertEqual(page.rootThreadIds, ["thread::origin"])
        XCTAssertEqual(page.nodes.count, 4)

        guard case .thread(let root) = page.nodes[0] else {
            XCTFail("first node must be the thread root")
            return
        }
        XCTAssertEqual(root.threadId, "thread::origin")
        XCTAssertEqual(root.title, "Origin conversation")
        XCTAssertEqual(root.agentId, "codex")
        XCTAssertEqual(root.depth, 0)

        guard case .task(let rootTask) = page.nodes[1] else {
            XCTFail("second node must be the root task")
            return
        }
        XCTAssertEqual(rootTask.task.id, "#TASK-40")
        XCTAssertEqual(rootTask.task.status, .inProgress)
        XCTAssertEqual(rootTask.parentNodeId, "thread-root:thread::origin")
        XCTAssertEqual(rootTask.runState, "running")
        XCTAssertEqual(rootTask.depth, 0)
        XCTAssertEqual(page.nodes[3].taskNode?.task.status, .done)
    }

    func testMissingDepthAndActiveCountToleratedForOldGateways() throws {
        let legacy = """
        {
          "tasks": [
            {
              "kind": "task",
              "node_id": "task:thread::standalone",
              "thread_id": "thread::standalone",
              "task_id": "#TASK-7",
              "number": 7,
              "title": "Standalone",
              "status": "todo",
              "runtime_agent_id": "",
              "reply_count": 0,
              "run_state": "idle"
            }
          ],
          "total": 1,
                  "root_thread_ids": ["thread::standalone"],
          "skipped_pinned_thread_ids": []
        }
        """
        let page = try JSONDecoder().decode(
            GaryxTaskForestPage.self,
            from: Data(legacy.utf8)
        )
        XCTAssertNil(page.activeCount)
        XCTAssertNil(page.nodes.first?.depth)
    }

    func testServerLayoutRendersWireOrderWithIndents() throws {
        let page = try decodedFixturePage()
        let rows = GaryxTaskTreeSidebarPresentation.rows(
            page: page,
            currentThreadId: "thread::review-child"
        )

        XCTAssertEqual(rows.map(\.id), [
            "thread-root:thread::origin",
            "task:thread::root-task",
            "task:thread::review-child",
            "task:thread::done-leaf",
        ])
        XCTAssertEqual(rows.map(\.indentLevel), [0, 0, 1, 1])
        XCTAssertEqual(rows[0].kind, .sourceThread)
        XCTAssertNil(rows[0].taskDisplayId)
        XCTAssertEqual(rows[1].taskDisplayId, "#TASK-40")
        XCTAssertTrue(rows[1].isRunning)
        XCTAssertTrue(rows[2].isCurrent)
    }

    func testIndentClampsAtFourForDeepServerLayouts() throws {
        var page = try decodedFixturePage()
        page.nodes = page.nodes.map { node in
            guard case .task(var task) = node, task.nodeId == "task:thread::done-leaf" else {
                return node
            }
            task.depth = 9
            return .task(task)
        }
        let rows = GaryxTaskTreeSidebarPresentation.rows(page: page, currentThreadId: nil)
        let deep = rows.first { $0.id == "task:thread::done-leaf" }
        XCTAssertEqual(deep?.indentLevel, 4)
    }

    func testFallbackLayoutWithoutDepthMatchesServerLayout() throws {
        let page = try decodedFixturePage()
        let serverRows = GaryxTaskTreeSidebarPresentation.rows(
            page: page,
            currentThreadId: "thread::origin"
        )
        let fallbackRows = GaryxTaskTreeSidebarPresentation.rows(
            page: pageWithoutServerLayout(page),
            currentThreadId: "thread::origin"
        )
        XCTAssertEqual(fallbackRows, serverRows)
    }

    func testFallbackLayoutTreatsOrphanParentsAsRoots() throws {
        var page = try decodedFixturePage()
        page.nodes = page.nodes.compactMap { node -> GaryxTaskForestNode? in
            guard case .task(var task) = node else { return nil }
            task.depth = nil
            if task.nodeId == "task:thread::root-task" {
                task.parentNodeId = "thread-root:thread::missing"
            }
            return .task(task)
        }
        page.activeCount = nil
        let rows = GaryxTaskTreeSidebarPresentation.rows(page: page, currentThreadId: nil)
        XCTAssertEqual(rows.map(\.id), [
            "task:thread::root-task",
            "task:thread::review-child",
            "task:thread::done-leaf",
        ])
        XCTAssertEqual(rows.map(\.indentLevel), [0, 1, 1])
    }

    func testCurrentHighlightAppliesToThreadRootAndDoneRows() throws {
        let page = try decodedFixturePage()

        let rootCurrent = GaryxTaskTreeSidebarPresentation.rows(
            page: page,
            currentThreadId: "thread::origin"
        )
        XCTAssertTrue(rootCurrent[0].isCurrent)
        XCTAssertTrue(rootCurrent.dropFirst().allSatisfy { !$0.isCurrent })

        let doneCurrent = GaryxTaskTreeSidebarPresentation.rows(
            page: page,
            currentThreadId: "thread::done-leaf"
        )
        let doneRow = doneCurrent.first { $0.id == "task:thread::done-leaf" }
        XCTAssertEqual(doneRow?.isCurrent, true)

        let evicted = GaryxTaskTreeSidebarPresentation.rows(
            page: page,
            currentThreadId: "thread::not-in-tree"
        )
        XCTAssertTrue(evicted.allSatisfy { !$0.isCurrent })
    }

    func testTreeCacheKeyPrefersOriginThenAnchor() throws {
        let page = try decodedFixturePage()
        // Origin-rooted tree: the key is the origin thread for every anchor,
        // so all threads of one tree share one cached snapshot.
        XCTAssertEqual(
            GaryxTaskTreeSidebarPresentation.treeCacheKey(
                page: page,
                anchorThreadId: "thread::done-leaf"
            ),
            "thread::origin"
        )

        var taskOnly = page
        taskOnly.rootThreadIds = ["thread::root-task"]
        XCTAssertEqual(
            GaryxTaskTreeSidebarPresentation.treeCacheKey(
                page: taskOnly,
                anchorThreadId: "thread::done-leaf"
            ),
            "thread::root-task"
        )

        var empty = page
        empty.rootThreadIds = []
        XCTAssertEqual(
            GaryxTaskTreeSidebarPresentation.treeCacheKey(
                page: empty,
                anchorThreadId: "thread::anchor"
            ),
            "thread::anchor"
        )
    }

    func testBadgeEqualsWireActiveCountAndLocalRecountFallback() throws {
        let page = try decodedFixturePage()
        XCTAssertEqual(GaryxTaskTreeSidebarPresentation.activeBadgeCount(page: page), 2)

        var withoutServerCount = page
        withoutServerCount.activeCount = nil
        XCTAssertEqual(
            GaryxTaskTreeSidebarPresentation.activeBadgeCount(page: withoutServerCount),
            2,
            "local recount matches the server count for the same fixture"
        )
    }

    func testTapPolicyCurrentClosesOnlyOthersNavigate() {
        XCTAssertFalse(GaryxTaskTreeSidebarPresentation.shouldNavigate(
            currentThreadId: "thread::a",
            targetThreadId: "thread::a"
        ))
        XCTAssertTrue(GaryxTaskTreeSidebarPresentation.shouldNavigate(
            currentThreadId: "thread::a",
            targetThreadId: "thread::b"
        ))
        XCTAssertFalse(GaryxTaskTreeSidebarPresentation.shouldNavigate(
            currentThreadId: nil,
            targetThreadId: ""
        ))
    }

    func testAvailabilityHidesEmptyAndTaskFreePages() throws {
        let page = try decodedFixturePage()
        XCTAssertTrue(GaryxTaskTreeSidebarPresentation.isSidebarAvailable(page: page))

        let empty = GaryxTaskForestPage()
        XCTAssertFalse(GaryxTaskTreeSidebarPresentation.isSidebarAvailable(page: empty))
        XCTAssertTrue(
            GaryxTaskTreeSidebarPresentation.rows(page: empty, currentThreadId: nil).isEmpty
        )

        var threadOnly = page
        threadOnly.nodes = page.nodes.filter { $0.taskNode == nil }
        XCTAssertFalse(GaryxTaskTreeSidebarPresentation.isSidebarAvailable(page: threadOnly))
    }

    func testRequestGateRejectsStaleGenerationsAnchorsAndGateways() {
        var gate = GaryxTaskTreeRequestGate()
        let first = gate.begin(gatewayKey: "gw-1", anchorThreadId: "thread::a")
        XCTAssertTrue(gate.accepts(token: first, gatewayKey: "gw-1", anchorThreadId: "thread::a"))

        // Anchor changed: the old token must be rejected.
        let second = gate.begin(gatewayKey: "gw-1", anchorThreadId: "thread::b")
        XCTAssertFalse(gate.accepts(token: first, gatewayKey: "gw-1", anchorThreadId: "thread::a"))
        XCTAssertTrue(gate.accepts(token: second, gatewayKey: "gw-1", anchorThreadId: "thread::b"))

        // Gateway switched: same anchor, new generation required.
        let third = gate.begin(gatewayKey: "gw-2", anchorThreadId: "thread::b")
        XCTAssertFalse(gate.accepts(token: second, gatewayKey: "gw-1", anchorThreadId: "thread::b"))
        XCTAssertTrue(gate.accepts(token: third, gatewayKey: "gw-2", anchorThreadId: "thread::b"))
    }

    func testTaskIdentityPrefersExecutorThenAssigneeThenRuntimeAgent() throws {
        var page = try decodedFixturePage()
        page.nodes = page.nodes.map { node in
            guard case .task(var task) = node, task.nodeId == "task:thread::root-task" else {
                return node
            }
            task.task.executor = GaryxTaskExecutor(type: "agent", agentId: "reviewer")
            return .task(task)
        }
        let rows = GaryxTaskTreeSidebarPresentation.rows(page: page, currentThreadId: nil)
        let rootTask = rows.first { $0.id == "task:thread::root-task" }
        XCTAssertEqual(rootTask?.identityAgentId, "reviewer")

        let assigneeBacked = rows.first { $0.id == "task:thread::done-leaf" }
        XCTAssertEqual(assigneeBacked?.identityAgentId, "test-agent")

        // No executor/assignee: falls back to the runtime agent.
        let runtimeBacked = rows.first { $0.id == "task:thread::review-child" }
        XCTAssertEqual(runtimeBacked?.identityAgentId, "test-agent")

        let threadRow = rows.first { $0.kind == .sourceThread }
        XCTAssertEqual(threadRow?.identityAgentId, "codex")
        XCTAssertEqual(threadRow?.providerType, "codex")
    }
}
