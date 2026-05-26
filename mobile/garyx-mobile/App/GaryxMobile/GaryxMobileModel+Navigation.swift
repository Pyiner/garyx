import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    var activePanel: GaryxMobilePanel {
        get { navigationState.activePanel }
        set {
            guard navigationState.activePanel != newValue else { return }
            var nextState = navigationState
            nextState.setActivePanel(newValue)
            navigationState = nextState
        }
    }

    var activeSettingsTab: GaryxMobileSettingsTab {
        get { navigationState.activeSettingsTab }
        set {
            guard navigationState.activeSettingsTab != newValue else { return }
            var nextState = navigationState
            nextState.activeSettingsTab = newValue
            navigationState = nextState
        }
    }

    var workspaceBotsDrilldown: GaryxWorkspaceBotsDrilldown? {
        get { navigationState.workspaceBotsDrilldown }
        set {
            guard navigationState.workspaceBotsDrilldown != newValue else { return }
            var nextState = navigationState
            nextState.setWorkspaceBotsDrilldown(newValue)
            navigationState = nextState
        }
    }

    var mainPanelBackStack: [GaryxMobilePanelRoute] {
        navigationState.mainPanelBackStack
    }

    func setSidebarVisible(_ visible: Bool, animated: Bool = true) {
        guard sidebarVisible != visible else { return }
        if animated {
            withAnimation(GaryxMobileMotion.sidebar) {
                sidebarVisible = visible
            }
        } else {
            sidebarVisible = visible
        }
    }

    func openPanel(_ panel: GaryxMobilePanel, source: GaryxMobilePanelOpenSource = .current) {
        var nextState = navigationState
        nextState.openPanel(panel, dreamsAutoScanEnabled: dreamsAutoScanEnabled, source: source)
        navigationState = nextState
        setSidebarVisible(false)
    }

    func openSettings(tab: GaryxMobileSettingsTab = .manage, source: GaryxMobilePanelOpenSource = .sidebar) {
        var nextState = navigationState
        nextState.openSettings(tab: tab, source: source)
        navigationState = nextState
        setSidebarVisible(false)
    }

    var mainPanelLeadingEdgeAction: GaryxMobileLeadingEdgeAction {
        navigationState.leadingEdgeAction
    }

    var mainPanelLeadingEdgeActionLabel: String {
        switch mainPanelLeadingEdgeAction {
        case .openSidebar:
            "Open menu"
        case .mainPanelBack:
            "Back"
        case .settingsOverview:
            "All Settings"
        case .workspaceBotsOverview:
            "Workspace & Bots"
        }
    }

    func performMainPanelLeadingEdgeAction() {
        switch mainPanelLeadingEdgeAction {
        case .openSidebar:
            setSidebarVisible(true)
        case .mainPanelBack:
            goBackInMainPanel()
        case .settingsOverview:
            showSettingsOverview()
        case .workspaceBotsOverview:
            var nextState = navigationState
            nextState.showWorkspaceBotsOverview()
            navigationState = nextState
        }
    }

    func showSettingsOverview() {
        var nextState = navigationState
        nextState.showSettingsOverview()
        navigationState = nextState
    }

    func goBackInMainPanel() {
        var nextState = navigationState
        guard nextState.goBackInMainPanel() else {
            setSidebarVisible(true)
            return
        }
        navigationState = nextState
        setSidebarVisible(false)
    }

    #if DEBUG
    @discardableResult
    func applyDebugURL(_ url: URL) -> Bool {
        guard url.scheme == "garyx", url.host == "debug" else {
            return false
        }

        let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        func queryValue(_ name: String) -> String? {
            components?.queryItems?.first(where: { $0.name == name })?.value
        }

        let usesLiveGateway = url.path == "/live" || queryValue("snapshot") == "0"
        if usesLiveGateway {
            debugSnapshotActive = false
        } else {
            loadDebugSnapshot()
        }

        applyDebugDestination(
            panelName: queryValue("panel"),
            tabName: queryValue("tab"),
            showSidebar: url.path == "/sidebar" || queryValue("panel") == "sidebar"
        )
        return true
    }

    func applyDebugDestination(panelName: String?, tabName: String?, showSidebar: Bool = false) {
        if showSidebar {
            activePanel = .chat
            setSidebarVisible(true, animated: false)
            return
        }

        if tabName == "general" {
            activeSettingsTab = .gateway
            activePanel = .settings
            setSidebarVisible(false, animated: false)
            return
        }

        if let tabName, let tab = GaryxMobileSettingsTab(rawValue: tabName) {
            activeSettingsTab = tab
            activePanel = .settings
            setSidebarVisible(false, animated: false)
            return
        }

        if let panelName, let panel = GaryxMobilePanel(rawValue: panelName) {
            let targetPanel: GaryxMobilePanel = switch panel {
            case .bots, .workspaces:
                .workspaceBots
            default:
                panel
            }
            activePanel = targetPanel == .dreams && !dreamsAutoScanEnabled ? .chat : targetPanel
            setSidebarVisible(false, animated: false)
            return
        }

        activePanel = .chat
        setSidebarVisible(false, animated: false)
    }

    func loadDebugSnapshot() {
        debugSnapshotActive = true
        cancelGlobalEventStream()
        cancelActiveSocket()

        gatewayURL = "http://127.0.0.1:31337"
        gatewayAuthToken = "debug-token"
        gatewayProfiles = [
            GaryxGatewayProfile(
                id: GaryxGatewayProfileStorage.stableId(for: "http://127.0.0.1:31337"),
                label: "127.0.0.1:31337",
                gatewayUrl: "http://127.0.0.1:31337",
                updatedAt: Date(timeIntervalSince1970: 1_779_172_400),
                hasToken: true
            ),
            GaryxGatewayProfile(
                id: GaryxGatewayProfileStorage.stableId(for: "http://10.0.0.2:31337"),
                label: "10.0.0.2:31337",
                gatewayUrl: "http://10.0.0.2:31337",
                updatedAt: Date(timeIntervalSince1970: 1_779_168_800),
                hasToken: false
            ),
        ]
        keychain.saveGatewayProfileToken(
            "debug-token",
            profileId: GaryxGatewayProfileStorage.stableId(for: "http://127.0.0.1:31337")
        )
        gatewaySettingsStatus = nil
        connectionState = .ready(version: "debug")
        isSending = false
        isLoadingThreads = false
        resetThreadListPagination()
        remoteStateLoadPhase = .loaded
        agentTargetsLoadPhase = .loaded
        resetSelectedThreadHistoryPagination()
        lastError = nil
        showsSettings = false
        messagesByThread = [:]
        messageSignaturesByThread = [:]
        activeAssistantMessageIdsByThread = [:]
        pendingAssistantDeltasByThread = [:]
        assistantDeltaFlushTasksByThread.values.forEach { $0.cancel() }
        assistantDeltaFlushTasksByThread = [:]

        threads = Self.decodeDebugFixture([GaryxThreadSummary].self, from: """
        [
          {
            "thread_id": "thread-history",
            "label": "Thread History",
            "updated_at": "2026-05-19T08:30:00Z",
            "last_user_message": "Review markdown, tool folding, and sidebar hierarchy",
            "workspace_dir": "/workspace/garyx",
            "message_count": 36,
            "agent_id": "codex"
          },
          {
            "thread_id": "thread-task-board",
            "label": "Tasks",
            "updated_at": "2026-05-19T07:15:00Z",
            "last_assistant_message": "Task fields now match the desktop surface.",
            "workspace_dir": "/workspace/garyx",
            "message_count": 18,
            "agent_id": "codex"
          },
          {
            "thread_id": "thread-automations",
            "label": "Gateway automation smoke",
            "updated_at": "2026-05-18T21:40:00Z",
            "last_assistant_message": "The synthetic run completed successfully.",
            "workspace_dir": "/workspace/garyx-gateway",
            "message_count": 12,
            "agent_id": "claude"
          },
          {
            "thread_id": "thread-root-chat",
            "label": "Quick root chat",
            "updated_at": "2026-05-18T19:10:00Z",
            "last_user_message": "Draft a compact release note",
            "message_count": 7,
            "agent_id": "codex"
          }
        ]
        """) ?? []
        selectedThread = threads.first
        draftThreadTitle = selectedThread?.title ?? ""
        pinnedThreadIds = ["thread-task-board"]
        selectedAgentTargetId = "codex"
        newThreadWorkspace = "/workspace/garyx"
        newThreadWorkspaceMode = "local"
        selectedWorkspacePath = "/workspace/garyx"
        selectedWorkspaceDirectory = ""
        draftWorkspacePath = ""
        workspaceListing = Self.decodeDebugFixture(GaryxWorkspaceFileListing.self, from: """
        {
          "workspace_dir": "/workspace/garyx",
          "directory_path": "",
          "entries": [
            { "path": "desktop", "name": "desktop", "entry_type": "directory", "has_children": true },
            { "path": "mobile", "name": "mobile", "entry_type": "directory", "has_children": true },
            { "path": "AGENTS.md", "name": "AGENTS.md", "entry_type": "file", "size": 4212, "media_type": "text/markdown" }
          ]
        }
        """)
        workspacePreview = nil
        workspaceGitStatuses = [:]
        messages = [
            GaryxMobileMessage(
                id: "debug-user-1",
                role: .user,
                text: "Please check markdown rendering, tool folding, and the sidebar hierarchy.",
                timestamp: "08:24",
                isStreaming: false
            ),
            GaryxMobileMessage(
                id: "debug-tools-1",
                role: .tool,
                text: "",
                timestamp: "08:25",
                isStreaming: false,
                toolTraceGroup: GaryxMobileToolTraceGroup(
                    entries: [
                        GaryxMobileToolTraceEntry(
                            id: "debug-tool-read",
                            toolUseId: "toolu-read",
                            parentToolUseId: nil,
                            toolName: "Read",
                            title: "Read",
                            inputText: "{ \"file\": \"mobile/garyx-mobile/App/GaryxMobile/GaryxMobileViews.swift\" }",
                            resultText: "Loaded the SwiftUI surface.",
                            summaryText: "Loaded SwiftUI surface",
                            inputLabel: "input",
                            resultLabel: "result",
                            status: .completed,
                            isError: false,
                            timestamp: "08:25",
                            primaryPathBadge: "GaryxMobileViews.swift"
                        ),
                        GaryxMobileToolTraceEntry(
                            id: "debug-tool-build",
                            toolUseId: "toolu-build",
                            parentToolUseId: nil,
                            toolName: "exec_command",
                            title: "Bash",
                            inputText: "swift test",
                            resultText: "Test Suite passed.",
                            summaryText: "swift test passed",
                            inputLabel: "command",
                            resultLabel: "output",
                            status: .completed,
                            isError: false,
                            timestamp: "08:26",
                            primaryPathBadge: nil
                        )
                    ]
                )
            ),
            GaryxMobileMessage(
                id: "debug-assistant-1",
                role: .assistant,
                text: """
                Sync complete

                **Result**
                - 477 buckets synced
                - 9 sessions reviewed
                - Dashboard: https://example.test/usage

                Code block rendering should stay compact and readable:

                ```bash
                swift test
                xcodebuild -scheme GaryxMobile build
                ```
                """,
                timestamp: "08:27",
                isStreaming: false
            )
        ]
        if let selectedThread {
            messagesByThread[selectedThread.id] = messages
            messageSignaturesByThread[selectedThread.id] = Self.messageListSignature(for: messages)
        }

        agents = Self.decodeDebugFixture(GaryxAgentsPage.self, from: """
        {
          "agents": [
            {
              "agent_id": "codex",
              "display_name": "Codex",
              "provider_type": "codex_app_server",
              "model": "gpt-5.3-codex",
              "auth_source": "mac_app",
              "default_workspace_dir": "/workspace/garyx",
              "avatar_data_url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR42mNUcLj0nwEPYGIgAIaHAgBE3AJBVcnK6gAAAABJRU5ErkJggg==",
              "built_in": true,
              "standalone": true
            },
            {
              "agent_id": "reviewer",
              "display_name": "Reviewer",
              "provider_type": "claude_code",
              "model": "sonnet",
              "auth_source": "mac_app",
              "default_workspace_dir": "/workspace/garyx",
              "avatar_data_url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR42mOM8VjwnwEPYGIgAIaHAgBXtgJTMAef0wAAAABJRU5ErkJggg==",
              "built_in": false,
              "standalone": true
            }
          ]
        }
        """)?.agents ?? []
        teams = Self.decodeDebugFixture(GaryxTeamsPage.self, from: """
        {
          "teams": [
            {
              "team_id": "qa-review",
              "display_name": "QA Review",
              "leader_agent_id": "codex",
              "member_agent_ids": ["codex", "reviewer"],
              "workflow_text": "Implement, review screenshots, then verify tests.",
              "avatar_data_url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR42mOUaYn5z4AHMDEQAMNDAQAOCgILqEOeygAAAABJRU5ErkJggg=="
            }
          ]
        }
        """)?.teams ?? []
        skills = Self.decodeDebugFixture([GaryxSkillSummary].self, from: """
        [
          {
            "id": "polish",
            "name": "Polish",
            "description": "Final UI quality pass for spacing, hierarchy, and details.",
            "installed": true,
            "enabled": true,
            "source_path": "/workspace/garyx/skills/polish"
          },
          {
            "id": "critique",
            "name": "Critique",
            "description": "Evaluate screens against product intent and visual quality.",
            "installed": true,
            "enabled": true,
            "source_path": "/workspace/garyx/skills/critique"
          }
        ]
        """) ?? []
        tasks = Self.decodeDebugFixture(GaryxTasksPage.self, from: """
        {
          "tasks": [
            {
              "task_id": "task-markdown",
              "thread_id": "thread-history",
              "number": 34,
              "title": "Fix markdown spacing and code blocks",
              "status": "in_progress",
              "assignee": { "kind": "agent", "agent_id": "codex" },
              "runtime_agent_id": "codex",
              "reply_count": 5,
              "updated_at": "2026-05-19T08:25:00Z"
            },
            {
              "task_id": "task-sidebar",
              "thread_id": "thread-history",
              "number": 35,
              "title": "Restore sidebar hierarchy",
              "status": "todo",
              "assignee": { "kind": "agent", "agent_id": "reviewer" },
              "runtime_agent_id": "reviewer",
              "reply_count": 2,
              "updated_at": "2026-05-19T08:10:00Z"
            },
            {
              "task_id": "task-shots",
              "thread_id": "thread-task-board",
              "number": 36,
              "title": "Capture every page",
              "status": "done",
              "assignee": { "kind": "agent", "agent_id": "codex" },
              "runtime_agent_id": "codex",
              "reply_count": 9,
              "updated_at": "2026-05-19T07:40:00Z"
            }
          ],
          "total": 3,
          "has_more": false
        }
        """)?.tasks ?? []
        automations = Self.decodeDebugFixture(GaryxAutomationsPage.self, from: """
        {
          "automations": [
            {
              "id": "automation-nightly-review",
              "label": "Nightly Review",
              "prompt": "Review open tasks and prepare a concise status.",
              "agent_id": "codex",
              "enabled": true,
              "workspace_dir": "/workspace/garyx",
              "next_run": "2026-05-20T01:00:00Z",
              "last_status": "success",
              "schedule": { "kind": "daily", "time": "09:00", "weekdays": ["mo", "tu", "we", "th", "fr"], "timezone": "Asia/Shanghai" }
            }
          ]
        }
        """)?.automations ?? []
        slashCommands = Self.decodeDebugFixture(GaryxSlashCommandsPage.self, from: """
        {
          "commands": [
            { "name": "ship-check", "description": "Run focused release checks.", "prompt": "Run tests, inspect UI screenshots, and summarize risk." },
            { "name": "qa-notes", "description": "Draft concise QA notes.", "prompt": "Summarize verified pages and open issues." }
          ]
        }
        """)?.commands ?? []
        mcpServers = Self.decodeDebugFixture(GaryxMcpServersPage.self, from: """
        {
          "servers": [
            { "name": "design", "transport": "stdio", "command": "design-mcp", "args": ["serve"], "env": {}, "enabled": true },
            { "name": "docs", "transport": "http", "url": "https://example.test/mcp", "headers": {}, "enabled": false }
          ]
        }
        """)?.servers ?? []
        autoResearchRuns = Self.decodeDebugFixture(GaryxAutoResearchRunsPage.self, from: """
        {
          "items": [
            {
              "run_id": "research-parity",
              "state": "running",
              "goal": "Compare navigation and transcript behavior.",
              "workspace_dir": "/workspace/garyx",
              "max_iterations": 3,
              "iterations_used": 2,
              "created_at": "2026-05-19T07:50:00Z",
              "updated_at": "2026-05-19T08:22:00Z"
            }
          ]
        }
        """)?.items ?? []
        channelEndpoints = Self.decodeDebugFixture(GaryxChannelEndpointsPage.self, from: """
        {
          "endpoints": [
            {
              "endpoint_key": "api:demo-thread",
              "channel": "api",
              "account_id": "demo-account",
              "display_label": "Demo API Thread",
              "thread_id": "thread-history",
              "thread_label": "Thread History",
              "workspace_dir": "/workspace/garyx",
              "conversation_kind": "thread",
              "conversation_label": "QA"
            }
          ]
        }
        """)?.endpoints ?? []
        configuredBots = Self.decodeDebugFixture(GaryxConfiguredBotsPage.self, from: """
        {
          "bots": [
            {
              "channel": "api",
              "account_id": "demo-account",
              "display_name": "Demo Bot",
              "enabled": true,
              "agent_id": "codex",
              "workspace_dir": "/workspace/garyx",
              "root_behavior": "open_default",
              "main_endpoint_status": "bound",
              "default_open_thread_id": "thread-history"
            }
          ]
        }
        """)?.bots ?? []
        botConsoles = Self.decodeDebugFixture(GaryxBotConsolesPage.self, from: """
        {
          "bots": [
            {
              "id": "api:demo-account",
              "channel": "api",
              "account_id": "demo-account",
              "title": "Demo Bot",
              "subtitle": "API channel",
              "agent_id": "codex",
              "root_behavior": "open_default",
              "status": "ready",
              "endpoint_count": 1,
              "bound_endpoint_count": 1,
              "workspace_dir": "/workspace/garyx",
              "default_open_thread_id": "thread-history"
            }
          ]
        }
        """)?.bots ?? []
        channelPlugins = []
        botStatusesById = [:]
        providerModelsByType = [:]
        selectedSkillEditor = nil
        selectedSkillDocument = nil
        selectedSkillFileContent = ""
        researchCandidatesByRunId = [:]
    }

    static func decodeDebugFixture<T: Decodable>(_ type: T.Type, from json: String) -> T? {
        try? JSONDecoder().decode(type, from: Data(json.utf8))
    }
    #endif
}
