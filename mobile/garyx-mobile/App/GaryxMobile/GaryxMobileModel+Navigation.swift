import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    var activePanel: GaryxMobilePanel {
        get { navigationState.activePanel }
        set {
            setActivePanel(newValue)
        }
    }

    var activeSettingsTab: GaryxMobileSettingsTab {
        get { navigationState.activeSettingsTab }
        set {
            guard navigationState.activeSettingsTab != newValue else { return }
            invalidatePendingThreadOpen()
            var nextState = navigationState
            nextState.activeSettingsTab = newValue
            navigationState = nextState
        }
    }

    var workspaceBotsDrilldown: GaryxWorkspaceBotsDrilldown? {
        get { navigationState.workspaceBotsDrilldown }
        set {
            guard navigationState.workspaceBotsDrilldown != newValue else { return }
            invalidatePendingThreadOpen()
            var nextState = navigationState
            nextState.setWorkspaceBotsDrilldown(newValue)
            navigationState = nextState
        }
    }

    var mainPanelBackStack: [GaryxMobilePanelRoute] {
        navigationState.mainPanelBackStack
    }

    var rootNavigationPath: [GaryxMobileRootRoute] {
        navigationState.rootNavigationPath
    }

    var isHomeVisible: Bool {
        !navigationState.presentsContent
    }

    /// Receives NavigationStack path writes. The system only pops (back
    /// swipe / back button); pushes always originate from the model.
    func applyRootNavigationPath(_ newPath: [GaryxMobileRootRoute]) {
        guard newPath.isEmpty, navigationState.presentsContent else { return }
        performMainPanelLeadingEdgeAction()
    }

    func popToHome() {
        guard navigationState.presentsContent else { return }
        invalidatePendingThreadOpen()
        stopSelectedThreadStreamForHome()
        cancelSelectedThreadReconcileLoop()
        var nextState = navigationState
        nextState.popToHome()
        navigationState = nextState
    }

    func setSidebarVisible(_ visible: Bool, animated: Bool = true) {
        guard sidebarVisible != visible else { return }
        // A light tick on open and close so the drawer state change is felt.
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        if animated {
            withAnimation(GaryxMobileMotion.sidebar) {
                sidebarVisible = visible
            }
        } else {
            sidebarVisible = visible
        }
    }

    func setActivePanel(
        _ panel: GaryxMobilePanel,
        invalidatesPendingThreadOpen: Bool = true
    ) {
        guard navigationState.activePanel != panel else {
            // Same panel, but it may not be presented above the home list
            // yet (for example reopening the conversation after a pop).
            if !navigationState.presentsContent {
                var nextState = navigationState
                nextState.setActivePanel(panel)
                navigationState = nextState
                if panel == .chat {
                    ensureSelectedThreadStreamForVisibleConversation()
                }
            }
            return
        }
        if invalidatesPendingThreadOpen {
            invalidatePendingThreadOpen()
        }
        var nextState = navigationState
        nextState.setActivePanel(panel)
        navigationState = nextState
    }

    func openConversation(
        source: GaryxMobilePanelOpenSource = .replace,
        invalidatesPendingThreadOpen: Bool = true
    ) {
        if invalidatesPendingThreadOpen {
            invalidatePendingThreadOpen()
        }
        var nextState = navigationState
        nextState.openConversation(source: source)
        navigationState = nextState
        ensureSelectedThreadStreamForVisibleConversation()
        setSidebarVisible(false)
    }

    func openPanel(_ panel: GaryxMobilePanel, source: GaryxMobilePanelOpenSource = .current) {
        invalidatePendingThreadOpen()
        var nextState = navigationState
        nextState.openPanel(panel, source: source)
        navigationState = nextState
        setSidebarVisible(false)
    }

    func openSettings(tab: GaryxMobileSettingsTab = .manage, source: GaryxMobilePanelOpenSource = .sidebar) {
        invalidatePendingThreadOpen()
        var nextState = navigationState
        nextState.openSettings(tab: tab, source: source)
        navigationState = nextState
        setSidebarVisible(false)
    }

    func openWorkspaceBotsDrilldown(
        _ drilldown: GaryxWorkspaceBotsDrilldown,
        source: GaryxMobilePanelOpenSource = .current
    ) {
        invalidatePendingThreadOpen()
        var nextState = navigationState
        nextState.openRoute(
            GaryxMobilePanelRoute(
                panel: .workspaceBots,
                settingsTab: .manage,
                workspaceBotsDrilldown: drilldown
            ),
            source: source
        )
        navigationState = nextState
        setSidebarVisible(false)
    }

    func openWorkspaceFilesPanel(source: GaryxMobilePanelOpenSource = .current) {
        invalidatePendingThreadOpen()
        var nextState = navigationState
        nextState.openRoute(
            GaryxMobilePanelRoute(panel: .workspaces, settingsTab: .manage),
            source: source
        )
        navigationState = nextState
        setSidebarVisible(false)
    }

    func queuePendingMobileRoute(_ route: GaryxMobileRoute) {
        pendingMobileRoute = route
        if case let .thread(threadId) = route {
            queuePendingThreadLink(threadId)
        }
    }

    func openPendingMobileRouteIfNeeded() async {
        guard let route = pendingMobileRoute else {
            await openPendingThreadLinkIfNeeded()
            return
        }
        guard case .ready = connectionState else { return }
        pendingMobileRoute = nil
        await openMobileRoute(route, source: .replace)
    }

    func openMobileRouteFromLink(_ route: GaryxMobileRoute) async {
        queuePendingMobileRoute(route)
        if case .ready = connectionState {
            await openPendingMobileRouteIfNeeded()
        } else if canConnectGateway, case .checking = connectionState {
            return
        } else if canConnectGateway {
            await connectAndRefresh()
        }
    }

    func openMobileRoute(
        _ route: GaryxMobileRoute,
        source: GaryxMobilePanelOpenSource = .replace
    ) async {
        clearRouteDrivenDetailState()
        switch route {
        case .chat:
            openNewThreadDraft()
        case let .thread(threadId):
            await openThread(id: threadId, source: source)
        case let .settings(tab):
            openSettings(tab: tab, source: source)
        case let .panel(panel):
            openPanel(panel, source: source)
        case let .automation(id):
            await openAutomationRoute(id, source: source)
        case let .automationThreads(id):
            await openAutomationThreadsRoute(id, source: source)
        case let .capsule(id):
            await openCapsuleRoute(id, source: source)
        case let .agent(id):
            await openAgentRoute(id, source: source)
        case let .skill(id):
            await openSkillRoute(id, source: source)
        case let .skillFile(skillId, path):
            await openSkillFileRoute(skillId: skillId, path: path, source: source)
        case let .workspace(path):
            await openWorkspaceRoute(path, source: source)
        case let .bot(channel, accountId):
            await openBotRoute(channel: channel, accountId: accountId, source: source)
        case let .workspaceFile(workspaceDir, path):
            await openWorkspaceFilePreview(
                GaryxMobileWorkspaceFileTarget(workspaceDir: workspaceDir, path: path),
                source: source
            )
        }
    }

    private func openAutomationRoute(_ id: String, source: GaryxMobilePanelOpenSource) async {
        let automationId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !automationId.isEmpty else { return }
        openPanel(.automations, source: source)
        await refreshRemoteState()
        guard let automation = automations.first(where: { $0.id == automationId }) else {
            showRouteNotFound(kind: "Automation", id: automationId)
            return
        }
        selectedAutomationEditor = automation
    }

    private func openAutomationThreadsRoute(_ id: String, source: GaryxMobilePanelOpenSource) async {
        let automationId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !automationId.isEmpty else { return }
        openWorkspaceBotsDrilldown(.automationThreads(automationId), source: source)
        if automations.isEmpty {
            await refreshRemoteState()
        }
    }

    private func openCapsuleRoute(_ id: String, source: GaryxMobilePanelOpenSource) async {
        let capsuleId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !capsuleId.isEmpty else { return }
        // In-transcript capsule cards present over the conversation and dismiss
        // back to it, instead of switching to the Capsules overview.
        if source == .conversation {
            await presentConversationCapsulePreview(capsuleId)
            return
        }
        openPanel(.capsules, source: source)
        await refreshCapsules()
        guard let capsule = capsules.first(where: { $0.id == capsuleId }) else {
            showRouteNotFound(kind: "Capsule", id: capsuleId)
            return
        }
        galleryFocusedCapsule = GaryxCapsulePreviewSelection(capsule: capsule)
    }

    private func openAgentRoute(_ id: String, source: GaryxMobilePanelOpenSource) async {
        let agentId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !agentId.isEmpty else { return }
        openPanel(.agents, source: source)
        await refreshRemoteState()
        guard let agent = agents.first(where: { $0.id == agentId }) else {
            showRouteNotFound(kind: "Agent", id: agentId)
            return
        }
        selectedAgentDetail = agent
    }

    private func openSkillRoute(_ id: String, source: GaryxMobilePanelOpenSource) async {
        let skillId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !skillId.isEmpty else { return }
        openPanel(.skills, source: source)
        await refreshRemoteState()
        guard let skill = skills.first(where: { $0.id == skillId }) else {
            showRouteNotFound(kind: "Skill", id: skillId)
            return
        }
        await openSkillEditor(skill)
    }

    private func openSkillFileRoute(
        skillId: String,
        path: String,
        source: GaryxMobilePanelOpenSource
    ) async {
        let normalizedSkillId = skillId.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedPath = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedSkillId.isEmpty, !normalizedPath.isEmpty else { return }
        openPanel(.skills, source: source)
        await refreshRemoteState()
        guard let skill = skills.first(where: { $0.id == normalizedSkillId }) else {
            showRouteNotFound(kind: "Skill", id: normalizedSkillId)
            return
        }
        await openSkillEditor(skill, selecting: normalizedPath)
    }

    private func openWorkspaceRoute(_ path: String, source: GaryxMobilePanelOpenSource) async {
        let workspacePath = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspacePath.isEmpty else { return }
        await selectWorkspace(workspacePath)
        openWorkspaceBotsDrilldown(.workspace(workspacePath), source: source)
    }

    private func openBotRoute(
        channel: String,
        accountId: String,
        source: GaryxMobilePanelOpenSource
    ) async {
        let normalizedChannel = channel.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedAccountId = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedChannel.isEmpty, !normalizedAccountId.isEmpty else { return }
        await refreshRemoteState()
        let groupId = mobileBotGroups.first { group in
            group.channel.caseInsensitiveCompare(normalizedChannel) == .orderedSame
                && group.accountId.caseInsensitiveCompare(normalizedAccountId) == .orderedSame
        }?.id ?? "\(normalizedChannel)::\(normalizedAccountId)"
        openWorkspaceBotsDrilldown(.bot(groupId), source: source)
    }

    private func clearRouteDrivenDetailState() {
        selectedAutomationEditor = nil
        selectedAgentDetail = nil
        galleryFocusedCapsule = nil
        conversationCapsulePreview = nil
        routeNotFoundStore.selection = nil
        closeSkillDetail()
    }

    private func showRouteNotFound(kind: String, id: String) {
        let target = id.trimmingCharacters(in: .whitespacesAndNewlines)
        routeNotFoundStore.selection = GaryxMobileRouteNotFound(
            title: "\(kind) Not Found",
            message: target.isEmpty
                ? "Garyx could not find the requested \(kind.lowercased())."
                : "Garyx could not find \(kind.lowercased()) \(target)."
        )
    }

    var mainPanelLeadingEdgeAction: GaryxMobileLeadingEdgeAction {
        navigationState.leadingEdgeAction
    }

    var mainPanelLeadingEdgeActionLabel: String {
        switch mainPanelLeadingEdgeAction {
        case .openSidebar:
            "Open menu"
        case .popToHome, .mainPanelBack:
            "Back"
        case .settingsOverview:
            "All Settings"
        case .workspaceBotsOverview:
            "Threads"
        }
    }

    func performMainPanelLeadingEdgeAction() {
        // While the task-tree sidebar is open, a leading-edge swipe must not
        // back-navigate underneath the panel; the swipe only closes the
        // sidebar via its scrim gesture.
        guard !isTaskTreeSidebarOpen else { return }
        switch mainPanelLeadingEdgeAction {
        case .openSidebar:
            setSidebarVisible(true)
        case .popToHome:
            popToHome()
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
        invalidatePendingThreadOpen()
        var nextState = navigationState
        nextState.showSettingsOverview()
        navigationState = nextState
    }

    func goBackInMainPanel() {
        invalidatePendingThreadOpen()
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
            loadDebugSnapshot(recentFilter: .all)
        }

        applyDebugDestination(
            panelName: queryValue("panel"),
            tabName: queryValue("tab"),
            showSidebar: url.path == "/sidebar" || queryValue("panel") == "sidebar"
        )
        if queryValue("drawer") == "1" {
            setSidebarVisible(true, animated: false)
        }
        let shouldShowWorkspaceModeSheet =
            queryValue("sheet") == "workspaceMode"
            || queryValue("workspaceModeSheet") == "1"
        if shouldShowWorkspaceModeSheet || queryValue("draft") == "1" {
            openNewThreadDraft()
        }
        debugShowsWorkspaceModeSheet = shouldShowWorkspaceModeSheet
        debugShowsGatewaySwitcher = queryValue("sheet") == "gatewaySwitcher"
        return true
    }

    func applyDebugDestination(panelName: String?, tabName: String?, showSidebar: Bool = false) {
        if showSidebar {
            // The thread list is the home root now; the legacy debug sidebar
            // route lands there instead of opening the navigation drawer.
            popToHome()
            setSidebarVisible(false, animated: false)
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
            activePanel = targetPanel
            setSidebarVisible(false, animated: false)
            return
        }

        activePanel = .chat
        setSidebarVisible(false, animated: false)
    }

    func loadDebugSnapshot(recentFilter: GaryxRecentThreadFilter) {
        debugSnapshotActive = true
        cancelBackgroundCommittedRunReconcileLoop()
        clearActiveRunState()

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
        debugShowsWorkspaceModeSheet = false
        debugShowsGatewaySwitcher = false
        recentThreadFeeds.select(recentFilter)
        resetThreadListPagination()
        remoteStateLoadPhase = .loaded
        agentTargetsLoadPhase = .loaded
        resetSelectedThreadHistoryPagination()
        lastError = nil
        showsSettings = false
        messagesByThread = [:]
        messageSignaturesByThread = [:]
        activeAssistantMessageIdsByThread = [:]
        pendingDirectFollowUpsByThread = [:]

        let fixtureThreads = Self.decodeDebugFixture([GaryxThreadSummary].self, from: """
        [
          {
            "thread_id": "thread-history",
            "title": "Thread History",
            "updated_at": "2026-05-19T08:30:00Z",
            "last_user_message": "Review markdown, tool folding, and sidebar hierarchy",
            "workspace_dir": "/workspace/garyx",
            "message_count": 36,
            "agent_id": "codex"
          },
          {
            "thread_id": "thread-task-board",
            "title": "Tasks",
            "updated_at": "2026-05-19T07:15:00Z",
            "last_assistant_message": "Task fields now match the desktop surface.",
            "workspace_dir": "/workspace/garyx",
            "message_count": 18,
            "agent_id": "codex"
          },
          {
            "thread_id": "thread-automations",
            "title": "Gateway automation smoke",
            "updated_at": "2026-05-18T21:40:00Z",
            "last_assistant_message": "The synthetic run completed successfully.",
            "workspace_dir": "/workspace/garyx-gateway",
            "message_count": 12,
            "agent_id": "claude"
          },
          {
            "thread_id": "thread-root-chat",
            "title": "Quick root chat",
            "updated_at": "2026-05-18T19:10:00Z",
            "last_user_message": "Draft a compact release note",
            "message_count": 7,
            "agent_id": "codex"
          }
        ]
        """) ?? []
        seedThreadSummariesForTesting(
            fixtureThreads,
            recentThreadIds: fixtureThreads.map(\.id)
        )
        selectedThread = fixtureThreads.first
        draftThreadTitle = selectedThread?.title ?? ""
        pinnedThreadIds = ["thread-task-board"]
        selectedAgentTargetId = nil
        gatewayDefaultAgentId = "codex"
        effectiveDefaultAgentId = "codex"
        newThreadWorkspace = "/workspace/garyx"
        newThreadWorkspaceMode = "local"
        replaceWorkspaceCatalogPaths(["/workspace/garyx"])
        seedWorkspaceThreadListForTesting(
            path: "/workspace/garyx",
            summaries: fixtureThreads
        )
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
        workspaceGitStatuses = [
            "/workspace/garyx": GaryxWorkspaceGitStatus(
                workspaceDir: "/workspace/garyx",
                isGitRepo: true,
                repoRoot: "/workspace/garyx",
                currentBranch: "main",
                isDirty: false
            )
        ]
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
            messageSignaturesByThread[selectedThread.id] = GaryxMessageListSignature.make(for: messages)
        }

        agents = Self.decodeDebugFixture(GaryxAgentsPage.self, from: """
        {
          "agents": [
            {
              "agent_id": "codex",
              "display_name": "Codex",
              "provider_type": "codex_app_server",
              "model": "gpt-5.3-codex",
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
              "default_workspace_dir": "/workspace/garyx",
              "avatar_data_url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR42mOM8VjwnwEPYGIgAIaHAgBXtgJTMAef0wAAAABJRU5ErkJggg==",
              "built_in": false,
              "standalone": true
            }
          ]
        }
        """)?.agents ?? []
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
        skillEditorLoadRequestId = nil
        skillFileLoadRequestId = nil
        selectedSkillEditor = nil
        selectedSkillDocument = nil
    }

    static func decodeDebugFixture<T: Decodable>(_ type: T.Type, from json: String) -> T? {
        try? JSONDecoder().decode(type, from: Data(json.utf8))
    }
    #endif
}
