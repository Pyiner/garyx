import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers
import WidgetKit

extension GaryxWorkspaceBotsDrilldown {
    var routeIdentity: GaryxWorkspaceDrilldownIdentity {
        switch self {
        case .workspace(let path):
            .workspace(path: path)
        case .bot(let accountID):
            .bot(accountID: accountID)
        case .automationThreads(let automationID):
            .automationThreads(automationID: automationID)
        }
    }
}

extension GaryxWorkspaceDrilldownIdentity {
    var drilldown: GaryxWorkspaceBotsDrilldown {
        switch self {
        case .workspace(let path):
            .workspace(path)
        case .bot(let accountID):
            .bot(accountID)
        case .automationThreads(let automationID):
            .automationThreads(automationID)
        }
    }
}

extension GaryxMobileModel {
    /// The UIKit occurrence path is the navigation truth. Module selection and
    /// selectedThread are read-only projections for existing feature stores.
    func applyCanonicalRouteProjection(_ path: [GaryxRouteEntry]) {
        forceTerminalGlobalRevealInteractions(.routeInvalidated)
        let projectedNavigation = GaryxMobileNavigationState(projecting: path)
        if navigationState != projectedNavigation {
            navigationState = projectedNavigation
        }

        guard let top = path.last else {
            cancelConversationContentActivation()
            stopSelectedThreadStreamForHome()
            cancelSelectedThreadReconcileLoop()
            selectedThread = nil
            return
        }
        switch top.destination {
        case .conversation(let threadID):
            let summary = cachedThreadSummary(for: threadID)
                ?? Self.placeholderThreadSummary(id: threadID)
            applySelectedThreadRouteProjection(summary, preparesContent: false)
        case .conversationDraft:
            cancelConversationContentActivation()
            if selectedThread != nil {
                resetSelectedTurnRowsWindow()
            }
            stopSelectedThreadStream()
            cancelSelectedThreadReconcileLoop()
            selectedThread = nil
            messages = []
            draftThreadTitle = ""
        case .panel, .settingsDetail, .workspaceDrilldown:
            cancelConversationContentActivation()
            stopSelectedThreadStreamForHome()
            cancelSelectedThreadReconcileLoop()
            selectedThread = nil
        }
    }

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
            openSettings(tab: newValue, source: .current)
        }
    }

    var workspaceBotsDrilldown: GaryxWorkspaceBotsDrilldown? {
        navigationState.workspaceBotsDrilldown
    }

    var isHomeVisible: Bool {
        productionRouteStore.path.isEmpty
    }

    func returnHome() {
        guard !productionRouteStore.path.isEmpty else { return }
        invalidatePendingThreadOpen()
        cancelConversationContentActivation()
        stopSelectedThreadStreamForHome()
        cancelSelectedThreadReconcileLoop()
        productionRouteStore.resetToHome()
    }

    func setSidebarVisible(_ visible: Bool, animated: Bool = true) {
        guard sidebarVisible != visible else { return }
        drawerRevealInteraction.setTarget(
            visible ? .open : .closed,
            animated: animated
        )
        sidebarVisible = visible
        if animated {
            GaryxMobileHaptics.shared.play(.drawerVisibilityCommitted)
        }
    }

    func setActivePanel(
        _ panel: GaryxMobilePanel,
        invalidatesPendingThreadOpen: Bool = true
    ) {
        if invalidatesPendingThreadOpen {
            invalidatePendingThreadOpen()
        }
        let resolved = panel == .bots ? GaryxMobilePanel.workspaceBots : panel
        if resolved != .chat {
            cancelConversationContentActivation()
        }
        let destination: GaryxRouteDestination
        if resolved == .chat {
            destination = selectedThread.map {
                .conversation(threadID: $0.id)
            } ?? .conversationDraft(draftID: newThreadComposerPayloadKey.draftRouteID)
        } else if resolved == .settings {
            destination = .settingsDetail(activeSettingsTab.rawValue)
        } else {
            destination = .panel(resolved.rawValue)
        }

        if productionRouteStore.path.last?.destination != destination {
            _ = productionRouteStore.open(destination, source: .replace)
        }
        if !productionRouteStore.isAttached {
            applyCanonicalRouteProjection(productionRouteStore.path)
        }
        if resolved == .chat, !productionRouteStore.isAttached {
            ensureSelectedThreadStreamForVisibleConversation()
        }
    }

    func openConversation(
        source: GaryxMobilePanelOpenSource = .replace,
        invalidatesPendingThreadOpen: Bool = true
    ) {
        if invalidatesPendingThreadOpen {
            invalidatePendingThreadOpen()
        }
        let destination: GaryxRouteDestination = selectedThread.map {
            .conversation(threadID: $0.id)
        } ?? .conversationDraft(draftID: newThreadComposerPayloadKey.draftRouteID)
        _ = productionRouteStore.open(destination, source: source)
        if !productionRouteStore.isAttached {
            applyCanonicalRouteProjection(productionRouteStore.path)
        }
        if !productionRouteStore.isAttached {
            ensureSelectedThreadStreamForVisibleConversation()
        }
        setSidebarVisible(false)
    }

    func openPanel(_ panel: GaryxMobilePanel, source: GaryxMobilePanelOpenSource = .current) {
        invalidatePendingThreadOpen()
        cancelConversationContentActivation()
        let resolved = panel == .bots ? GaryxMobilePanel.workspaceBots : panel
        _ = productionRouteStore.open(.panel(resolved.rawValue), source: source)
        if !productionRouteStore.isAttached {
            applyCanonicalRouteProjection(productionRouteStore.path)
        }
        setSidebarVisible(false)
    }

    func openSettings(tab: GaryxMobileSettingsTab = .manage, source: GaryxMobilePanelOpenSource = .sidebar) {
        invalidatePendingThreadOpen()
        cancelConversationContentActivation()
        let overview = GaryxRouteDestination.settingsDetail(
            GaryxMobileSettingsTab.manage.rawValue
        )
        if tab != .manage {
            if productionRouteStore.path.last?.destination == overview {
                _ = productionRouteStore.open(
                    .settingsDetail(tab.rawValue),
                    source: .current
                )
            } else {
                _ = productionRouteStore.open(
                    [overview, .settingsDetail(tab.rawValue)],
                    source: source
                )
            }
        } else {
            _ = productionRouteStore.open(.settingsDetail(tab.rawValue), source: source)
        }
        if !productionRouteStore.isAttached {
            applyCanonicalRouteProjection(productionRouteStore.path)
        }
        setSidebarVisible(false)
    }

    func openWorkspaceBotsDrilldown(
        _ drilldown: GaryxWorkspaceBotsDrilldown,
        source: GaryxMobilePanelOpenSource = .current
    ) {
        invalidatePendingThreadOpen()
        _ = productionRouteStore.open(
            .workspaceDrilldown(drilldown.routeIdentity),
            source: source
        )
        if !productionRouteStore.isAttached {
            applyCanonicalRouteProjection(productionRouteStore.path)
        }
        setSidebarVisible(false)
    }

    func openWorkspaceFilesPanel(source: GaryxMobilePanelOpenSource = .current) {
        openPanel(.workspaces, source: source)
    }

    func queuePendingMobileRoute(_ route: GaryxMobileRoute) {
        pendingMobileRoute = route
    }

    func openPendingMobileRouteIfNeeded() async {
        guard let route = pendingMobileRoute else {
            await openPendingThreadLinkIfNeeded()
            return
        }
        guard case .ready = connectionState else { return }
        pendingMobileRoute = nil
        await openMobileRoute(route, source: .deepLink)
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
        // In-transcript capsule cards present over the conversation and dismiss
        // back to it, instead of switching to the Capsules overview.
        if source == .conversation, case .capsule(let id) = route {
            let capsuleId = id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !capsuleId.isEmpty else { return }
            await presentConversationCapsulePreview(capsuleId)
            return
        }

        let preparation = productionRouteStore.beginNavigationPreparation(
            source: source,
            scopes: gatewayScopeRegistry
        )
        let outcome = await prepareMobileRoute(route)
        let routeOutcome = outcome.map(\.destinations)
        let prepared = outcome.preparedValue
        let submission = productionRouteStore.completeNavigationPreparation(
            preparation,
            outcome: routeOutcome,
            scopes: gatewayScopeRegistry,
            onVisible: { [weak self] in
                guard let self, let prepared else { return }
                activatePreparedMobileRoute(prepared)
            }
        )
        handleMobileRouteSubmission(submission.result, requestedRoute: route)
    }

    private func prepareMobileRoute(
        _ route: GaryxMobileRoute
    ) async -> GaryxPrepareOutcome<GaryxPreparedMobileRoute> {
        guard case .ready = connectionState else { return .authenticationRequired }
        let currentDraftID = newThreadComposerPayloadKey.draftRouteID

        func prepared(
            _ activation: GaryxPreparedMobileRouteActivation,
            botGroupID: String? = nil,
            draftID: String? = nil
        ) -> GaryxPrepareOutcome<GaryxPreparedMobileRoute> {
            .ready(
                GaryxPreparedMobileRoute(
                    destinations: GaryxMobileRoutePlan.destinations(
                        for: route,
                        draftID: draftID ?? currentDraftID,
                        resolvedBotGroupID: botGroupID
                    ),
                    activation: activation
                )
            )
        }

        do {
            switch route {
            case .chat:
                let targetID = preparedNewThreadAgentTargetID()
                return prepared(
                    .newThreadDraft(agentTargetID: targetID),
                    draftID: newThreadDraftID(agentTargetID: targetID)
                )
            case .panel(.chat):
                let targetID = preparedNewThreadAgentTargetID()
                return prepared(
                    .newThreadDraft(agentTargetID: targetID),
                    draftID: newThreadDraftID(agentTargetID: targetID)
                )
            case .settings, .panel:
                return prepared(.none)
            case .thread(let requestedID):
                let id = requestedID.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !id.isEmpty else { return .userVisibleNotFound }
                let thread: GaryxThreadSummary
                if let cached = cachedThreadSummary(for: id) {
                    thread = cached
                } else {
                    thread = try await client().getThread(threadId: id)
                }
                return prepared(.thread(thread))
            case .automation(let requestedID):
                let id = requestedID.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !id.isEmpty else { return .userVisibleNotFound }
                let catalog = try await client().listAutomations()
                guard let automation = catalog.first(where: { $0.id == id }) else {
                    return .userVisibleNotFound
                }
                return prepared(.automation(automation))
            case .automationThreads(let requestedID):
                let id = requestedID.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !id.isEmpty else { return .userVisibleNotFound }
                let catalog = try await client().listAutomations()
                guard let automation = catalog.first(where: { $0.id == id }) else {
                    return .userVisibleNotFound
                }
                return prepared(.automationThreads(automation))
            case .capsule(let requestedID):
                let id = requestedID.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !id.isEmpty else { return .userVisibleNotFound }
                let catalog = try await client().listCapsules()
                guard let capsule = catalog.first(where: { $0.id == id }) else {
                    return .userVisibleNotFound
                }
                return prepared(.capsule(.init(capsule: capsule)))
            case .agent(let requestedID):
                let id = requestedID.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !id.isEmpty else { return .userVisibleNotFound }
                let agent = try await client().getAgent(agentId: id)
                return prepared(.agent(agent))
            case .skill(let requestedID):
                let id = requestedID.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !id.isEmpty else { return .userVisibleNotFound }
                let gateway = try client()
                let editor = try await gateway.skillEditor(skillId: id)
                let document: GaryxSkillFileDocument?
                if let preferredPath = Self.preferredSkillFilePath(in: editor.entries) {
                    do {
                        document = try await gateway.readSkillFile(
                            skillId: id,
                            path: preferredPath
                        )
                    } catch GaryxGatewayError.httpStatus(let status, _, _) where status == 404 {
                        // The skill itself is a valid target even when its tree
                        // races a removed preferred file.
                        document = nil
                    }
                } else {
                    document = nil
                }
                return prepared(.skill(editor: editor, document: document))
            case .skillFile(let requestedID, let requestedPath):
                let id = requestedID.trimmingCharacters(in: .whitespacesAndNewlines)
                let path = requestedPath.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !id.isEmpty, !path.isEmpty else { return .userVisibleNotFound }
                let gateway = try client()
                async let editor = gateway.skillEditor(skillId: id)
                async let document = gateway.readSkillFile(skillId: id, path: path)
                let (resolvedEditor, resolvedDocument) = try await (editor, document)
                return prepared(
                    .skill(editor: resolvedEditor, document: resolvedDocument)
                )
            case .workspace(let requestedPath):
                let path = requestedPath.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !path.isEmpty else { return .userVisibleNotFound }
                let catalog = try await client().listWorkspaces()
                guard catalog.contains(where: { $0.path == path }) else {
                    return .userVisibleNotFound
                }
                return prepared(.none)
            case .bot(let requestedChannel, let requestedAccountID):
                let channel = requestedChannel.trimmingCharacters(in: .whitespacesAndNewlines)
                let accountID = requestedAccountID.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !channel.isEmpty, !accountID.isEmpty else {
                    return .userVisibleNotFound
                }
                let catalog = try await client().listConfiguredBots()
                guard let bot = catalog.first(where: {
                    $0.channel.caseInsensitiveCompare(channel) == .orderedSame
                        && $0.accountId.caseInsensitiveCompare(accountID) == .orderedSame
                }) else {
                    return .userVisibleNotFound
                }
                return prepared(.bot(bot), botGroupID: "\(bot.channel)::\(bot.accountId)")
            case .workspaceFile(let requestedWorkspace, let requestedPath):
                let workspace = requestedWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
                let path = requestedPath.trimmingCharacters(in: .whitespacesAndNewlines)
                let target = GaryxMobileWorkspaceFileTarget(
                    workspaceDir: workspace,
                    path: path
                )
                guard !target.workspaceDir.isEmpty, !target.path.isEmpty else {
                    return .userVisibleNotFound
                }
                let gateway = try client()
                async let listing: GaryxWorkspaceFileListing? = try? gateway.listWorkspaceFiles(
                    workspaceDir: target.workspaceDir,
                    directoryPath: (target.path as NSString).deletingLastPathComponent
                )
                let preview = try await gateway.previewWorkspaceFile(
                    workspaceDir: target.workspaceDir,
                    path: target.path
                )
                return prepared(
                    .workspaceFile(
                        target: target,
                        preview: preview,
                        listing: await listing
                    )
                )
            }
        } catch is CancellationError {
            return .cancelledOrStale
        } catch GaryxGatewayError.httpStatus(let status, _, _)
            where status == 401 || status == 403 {
            return .authenticationRequired
        } catch GaryxGatewayError.httpStatus(let status, _, _) where status == 404 {
            return .userVisibleNotFound
        } catch {
            return .retryableFailure(message: displayMessage(for: error))
        }
    }

    private func activatePreparedMobileRoute(_ prepared: GaryxPreparedMobileRoute) {
        invalidatePendingThreadOpen()
        clearRouteDrivenDetailState()
        switch prepared.activation {
        case .none:
            break
        case .newThreadDraft(let agentTargetID):
            activatePreparedNewThreadDraft(
                agentTargetOverride: agentTargetID,
                freezesAgentTarget: true
            )
        case .thread(let thread):
            Task { @MainActor [weak self] in
                await self?.activatePreparedThread(thread)
            }
        case .automation(let automation):
            replaceAutomation(automation)
            selectedAutomationEditor = automation
        case .automationThreads(let automation):
            replaceAutomation(automation)
        case .capsule(let selection):
            galleryFocusedCapsule = selection
        case .agent(let agent):
            selectedAgentDetail = agent
        case .skill(let editor, let document):
            skillEditorLoadRequestId = nil
            skillFileLoadRequestId = nil
            selectedSkillEditor = editor
            selectedSkillDocument = document
        case .bot(let bot):
            if let index = configuredBots.firstIndex(where: {
                $0.channel.caseInsensitiveCompare(bot.channel) == .orderedSame
                    && $0.accountId.caseInsensitiveCompare(bot.accountId) == .orderedSame
            }) {
                configuredBots[index] = bot
            } else {
                configuredBots.append(bot)
            }
        case .workspaceFile(let target, let preview, let listing):
            activatePreparedWorkspaceFilePreview(
                target: target,
                preview: preview,
                listing: listing
            )
        }
    }

    /// A new-chat link starts a fresh draft, so it must not inherit the bot or
    /// one-draft agent selection currently projected by the visible route.
    /// Freeze the gateway default into the prepared payload so a queued intent
    /// cannot expose a different composer key when it finally becomes visible.
    private func preparedNewThreadAgentTargetID() -> String {
        GaryxNewThreadAgentSelection.agentId(
            draftOverrideAgentId: nil,
            effectiveDefaultAgentId: effectiveDefaultAgentId
        ) ?? ""
    }

    private func newThreadDraftID(agentTargetID: String) -> String {
        let targetID = agentTargetID.trimmingCharacters(in: .whitespacesAndNewlines)
        return targetID.isEmpty ? "new-thread" : "new-thread:\(targetID)"
    }

    private func handleMobileRouteSubmission(
        _ result: GaryxNavigationQueueResult,
        requestedRoute: GaryxMobileRoute
    ) {
        switch result {
        case .userVisibleNotFound:
            let descriptor = requestedRoute.notFoundDescriptor
            showRouteNotFound(kind: descriptor.kind, id: descriptor.id)
        case .retryableFailure(let message):
            lastError = message
        case .authenticationRequired:
            pendingMobileRoute = requestedRoute
        case .internalFault(let code):
            lastError = "Garyx could not open this destination (\(code))."
        case .admittedImmediately, .queued, .presentationDismissalRequired,
             .cancelledOrStale, .stalePreparation, .reprepareRequired,
             .dependencyDiscarded:
            break
        }
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

    func dismissCurrentRoute() {
        guard !isTaskTreeSidebarOpen else { return }
        invalidatePendingThreadOpen()
        productionRouteStore.popOne()
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
        if queryValue("sheet") == "automationEditor" {
            selectedAutomationEditor = automations.first
        }
        return true
    }

    func applyDebugDestination(panelName: String?, tabName: String?, showSidebar: Bool = false) {
        if showSidebar {
            // The thread list is the home root now; the legacy debug sidebar
            // route lands there instead of opening the navigation drawer.
            returnHome()
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
            case .bots:
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
        sceneRefreshTask?.cancel()
        sceneRefreshTask = nil
        cancelBackgroundCommittedRunReconcileLoop()
        cancelSelectedThreadReconcileLoop()
        stopSelectedThreadStream()
        selectedThreadHistoryRequestId = nil
        selectedThreadHistoryRetryTask?.cancel()
        selectedThreadHistoryRetryTask = nil
        selectedThreadHistoryRetryThreadId = nil
        selectedThreadHistoryRetryCount = 0
        isLoadingSelectedThreadHistory = false
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
        renderSnapshotsByThread = [:]
        threadHistoryLoadedIds = []
        selectedThreadRenderFloorByThread = [:]
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
                text: "Type check",
                timestamp: "08:24",
                isStreaming: false
            ),
            GaryxMobileMessage(
                id: "debug-assistant-1",
                role: .assistant,
                text: """
                **Result**
                Wraps cleanly.
                """,
                timestamp: "08:25",
                isStreaming: false
            )
        ]
        if ProcessInfo.processInfo.environment["GARYX_MOBILE_ROUTE_PUSH_FIXTURE"] == "long" {
            messages = (0..<24).flatMap { turn in
                [
                    GaryxMobileMessage(
                        id: "route-push-user-\(turn)",
                        role: .user,
                        text: "Review route transition sample \(turn) and keep the transcript responsive.",
                        timestamp: "08:\(String(format: "%02d", turn))",
                        isStreaming: false
                    ),
                    GaryxMobileMessage(
                        id: "route-push-assistant-\(turn)",
                        role: .assistant,
                        text: """
                        Sample \(turn) is ready

                        - The cached row is deterministic.
                        - **Markdown** and `inline code` exercise the production renderer.
                        - The route-entry frame budget remains independent of transcript length.
                        """,
                        timestamp: "08:\(String(format: "%02d", turn))",
                        isStreaming: false
                    ),
                ]
            }
        }
        if let selectedThread {
            messagesByThread[selectedThread.id] = messages
            messageSignaturesByThread[selectedThread.id] = GaryxMessageListSignature.make(for: messages)
            renderSnapshotsByThread[selectedThread.id] = GaryxRenderSnapshot(
                basedOnSeq: 2,
                rows: [
                    .userTurn(GaryxRenderUserTurnRow(
                        id: "debug-turn-1",
                        user: GaryxRenderMessageRef(id: "debug-user-1", seq: 1, role: "user"),
                        activity: [
                            .assistantReply(GaryxRenderAssistantReplyRow(
                                id: "debug-assistant-row-1",
                                message: GaryxRenderMessageRef(
                                    id: "debug-assistant-1",
                                    seq: 2,
                                    role: "assistant"
                                )
                            )),
                        ]
                    )),
                ]
            )
            threadHistoryLoadedIds.insert(selectedThread.id)
            resetSelectedTurnRowsWindow()
            lockSelectedTurnRowsWindowFloorIfNeeded()
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
        seedDebugTaskTreeGestureFixtureIfRequested()
    }

    static func decodeDebugFixture<T: Decodable>(_ type: T.Type, from json: String) -> T? {
        try? JSONDecoder().decode(type, from: Data(json.utf8))
    }
    #endif
}

extension GaryxComposerKey {
    var draftRouteID: String {
        switch self {
        case .draft(let draftID):
            draftID
        case .thread(let threadID):
            "new-thread:\(threadID)"
        }
    }
}

private extension GaryxMobileRoute {
    var notFoundDescriptor: (kind: String, id: String) {
        switch self {
        case .chat:
            ("Destination", "chat")
        case .thread(let id):
            ("Thread", id)
        case .settings(let tab):
            ("Settings Page", tab.rawValue)
        case .panel(let panel):
            ("Page", panel.label)
        case .automation(let id), .automationThreads(let id):
            ("Automation", id)
        case .capsule(let id):
            ("Capsule", id)
        case .agent(let id):
            ("Agent", id)
        case .skill(let id):
            ("Skill", id)
        case .skillFile(let skillID, let path):
            ("Skill File", "\(skillID)/\(path)")
        case .workspace(let path):
            ("Workspace", path)
        case .bot(let channel, let accountID):
            ("Bot", "\(channel)/\(accountID)")
        case .workspaceFile(let workspace, let path):
            ("Workspace File", "\(workspace)/\(path)")
        }
    }
}
