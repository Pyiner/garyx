import Foundation

public enum GaryxMobileConnectionState: Equatable, Sendable {
    case disconnected
    case checking
    case ready(version: String?)
    case failed(String)

    public var label: String {
        switch self {
        case .disconnected:
            "Disconnected"
        case .checking:
            "Checking"
        case .ready:
            "Connected"
        case .failed:
            "Offline"
        }
    }
}

public enum GaryxMobilePanel: String, CaseIterable, Identifiable, Sendable {
    case chat
    case dreams
    case tasks
    case workspaces
    case automations
    case agents
    case skills
    case commands
    case mcp
    case autoResearch
    case workspaceBots
    case bots
    case settings

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .chat:
            "Chat"
        case .dreams:
            "Dreams"
        case .tasks:
            "Tasks"
        case .workspaces:
            "Workspaces"
        case .automations:
            "Automation"
        case .agents:
            "Agents"
        case .skills:
            "Skills"
        case .commands:
            "Commands"
        case .mcp:
            "MCP"
        case .autoResearch:
            "Auto Research"
        case .workspaceBots:
            "Workspaces"
        case .bots:
            "Bots"
        case .settings:
            "Settings"
        }
    }

    public var iconName: String {
        switch self {
        case .chat:
            "bubble.left.and.text.bubble.right.fill"
        case .dreams:
            "moon.stars.fill"
        case .tasks:
            "checklist.checked"
        case .workspaces:
            "folder"
        case .automations:
            "clock.arrow.circlepath"
        case .agents:
            "person.2.fill"
        case .skills:
            "wand.and.stars"
        case .commands:
            "command"
        case .mcp:
            "point.3.connected.trianglepath.dotted"
        case .autoResearch:
            "atom"
        case .workspaceBots:
            "folder.fill"
        case .bots:
            "bubble.left.and.bubble.right"
        case .settings:
            "gearshape"
        }
    }
}

public enum GaryxMobileSettingsTab: String, CaseIterable, Identifiable, Sendable {
    case manage
    case gateway
    case provider
    case channels
    case commands
    case mcp

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .manage:
            "All Settings"
        case .gateway:
            "Gateway"
        case .provider:
            "Provider"
        case .channels:
            "Channels"
        case .commands:
            "Commands"
        case .mcp:
            "MCP"
        }
    }

    public var iconName: String {
        switch self {
        case .manage:
            "list.bullet"
        case .gateway:
            "server.rack"
        case .provider:
            "sparkles"
        case .channels:
            "bubble.left.and.bubble.right.fill"
        case .commands:
            "command"
        case .mcp:
            "point.3.connected.trianglepath.dotted"
        }
    }
}

public enum GaryxMobileLoadPhase: Equatable, Sendable {
    case idle
    case loading
    case loaded
    case failed(String)

    public var isLoading: Bool {
        if case .loading = self {
            return true
        }
        return false
    }

    public var hasResolved: Bool {
        switch self {
        case .loaded, .failed:
            return true
        case .idle, .loading:
            return false
        }
    }

    public var failureMessage: String? {
        if case .failed(let message) = self {
            return message
        }
        return nil
    }
}

public struct GaryxMobileResourceState<Value: Equatable & Sendable>: Equatable, Sendable {
    public private(set) var value: Value
    public private(set) var phase: GaryxMobileLoadPhase
    public private(set) var lastUpdatedAt: Date?
    public private(set) var lastFailureMessage: String?
    public private(set) var isRefreshing: Bool

    public init(
        value: Value,
        phase: GaryxMobileLoadPhase = .idle,
        lastUpdatedAt: Date? = nil,
        lastFailureMessage: String? = nil,
        isRefreshing: Bool = false
    ) {
        self.value = value
        self.phase = phase
        self.lastUpdatedAt = lastUpdatedAt
        self.lastFailureMessage = lastFailureMessage
        self.isRefreshing = isRefreshing
    }

    public mutating func reset(to value: Value) {
        self.value = value
        phase = .idle
        lastUpdatedAt = nil
        lastFailureMessage = nil
        isRefreshing = false
    }

    /// Hydrates display state from a local cache without implying a fresh network result.
    public mutating func restore(_ value: Value, at date: Date? = nil) {
        self.value = value
        phase = .loaded
        lastUpdatedAt = date
        lastFailureMessage = nil
        isRefreshing = false
    }

    public mutating func beginRefresh() {
        isRefreshing = true
        switch phase {
        case .idle, .failed:
            phase = .loading
        case .loading, .loaded:
            break
        }
    }

    /// Applies a successful async refresh result and records it as freshly updated.
    public mutating func completeRefresh(_ value: Value, at date: Date = Date()) {
        self.value = value
        phase = .loaded
        lastUpdatedAt = date
        lastFailureMessage = nil
        isRefreshing = false
    }

    public mutating func failRefresh(_ message: String, keepingStaleValue: Bool) {
        lastFailureMessage = message
        isRefreshing = false
        phase = keepingStaleValue ? .loaded : .failed(message)
    }

    /// Applies a direct local mutation, such as an add/delete response already accepted by the backend.
    public mutating func replace(_ value: Value, at date: Date = Date()) {
        self.value = value
        phase = .loaded
        lastUpdatedAt = date
        lastFailureMessage = nil
        isRefreshing = false
    }
}

public enum GaryxMobileThreadOpenSource: Equatable, Sendable {
    case url
    case direct
}

public struct GaryxMobileThreadOpenState: Equatable, Sendable {
    public private(set) var requestId: UUID
    public private(set) var pendingThreadId: String?
    public private(set) var pendingSource: GaryxMobileThreadOpenSource?
    public private(set) var shownThreadId: String?

    public init(requestId: UUID = UUID()) {
        self.requestId = requestId
    }

    public var hasPendingIntent: Bool {
        pendingThreadId != nil
    }

    public mutating func queue(
        threadId: String,
        source: GaryxMobileThreadOpenSource,
        requestId: UUID = UUID()
    ) -> UUID? {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty else { return nil }
        self.requestId = requestId
        pendingThreadId = normalizedThreadId
        pendingSource = source
        shownThreadId = nil
        return requestId
    }

    public mutating func beginDirectOpen(requestId: UUID = UUID()) -> UUID {
        self.requestId = requestId
        pendingThreadId = nil
        pendingSource = .direct
        shownThreadId = nil
        return requestId
    }

    public mutating func invalidate(requestId: UUID = UUID()) {
        self.requestId = requestId
        pendingThreadId = nil
        pendingSource = nil
        shownThreadId = nil
    }

    public func isCurrent(_ requestId: UUID) -> Bool {
        self.requestId == requestId
    }

    @discardableResult
    public mutating func markShown(threadId: String, requestId: UUID) -> Bool {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard isCurrent(requestId), pendingThreadId == normalizedThreadId else {
            return false
        }
        shownThreadId = normalizedThreadId
        return true
    }

    @discardableResult
    public mutating func complete(threadId: String, requestId: UUID? = nil) -> Bool {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        if let requestId, !isCurrent(requestId) {
            return false
        }
        guard pendingThreadId == normalizedThreadId else {
            return false
        }
        pendingThreadId = nil
        pendingSource = nil
        shownThreadId = nil
        return true
    }
}

public enum GaryxMobileLeadingEdgeAction: Equatable, Sendable {
    case openSidebar
    case popToHome
    case mainPanelBack
    case settingsOverview
    case workspaceBotsOverview
}

/// Top-level pushed route over the home thread list. Routes are stable
/// tokens; pushed pages read their detail state (settings tab, drilldowns)
/// from the navigation state so in-page navigation never re-pushes.
public enum GaryxMobileRootRoute: Hashable, Sendable {
    case conversation
    case panel(GaryxMobilePanel)
}

public enum GaryxMobilePanelOpenSource: Equatable, Sendable {
    case current
    case sidebar
    case replace
}

public enum GaryxWorkspaceBotsDrilldown: Equatable, Sendable {
    case bot(String)
    case workspace(String)
    case automationThreads(String)
}

public struct GaryxMobilePanelRoute: Equatable, Sendable {
    public let panel: GaryxMobilePanel
    public let settingsTab: GaryxMobileSettingsTab
    public let workspaceBotsDrilldown: GaryxWorkspaceBotsDrilldown?

    public init(
        panel: GaryxMobilePanel,
        settingsTab: GaryxMobileSettingsTab,
        workspaceBotsDrilldown: GaryxWorkspaceBotsDrilldown? = nil
    ) {
        self.panel = panel
        self.settingsTab = settingsTab
        self.workspaceBotsDrilldown = workspaceBotsDrilldown
    }
}

public struct GaryxMobileNavigationState: Equatable, Sendable {
    public private(set) var activePanel: GaryxMobilePanel
    public var activeSettingsTab: GaryxMobileSettingsTab
    public var workspaceBotsDrilldown: GaryxWorkspaceBotsDrilldown?
    public private(set) var mainPanelBackStack: [GaryxMobilePanelRoute]
    /// False while the home thread list is the visible root; true while a
    /// conversation or panel page is pushed above it.
    public private(set) var presentsContent: Bool

    public init(
        activePanel: GaryxMobilePanel = .chat,
        activeSettingsTab: GaryxMobileSettingsTab = .manage,
        workspaceBotsDrilldown: GaryxWorkspaceBotsDrilldown? = nil,
        mainPanelBackStack: [GaryxMobilePanelRoute] = [],
        presentsContent: Bool = false
    ) {
        self.activePanel = activePanel
        self.activeSettingsTab = activeSettingsTab
        self.workspaceBotsDrilldown = workspaceBotsDrilldown
        self.mainPanelBackStack = mainPanelBackStack
        self.presentsContent = presentsContent
    }

    /// NavigationStack path over the home thread list.
    public var rootNavigationPath: [GaryxMobileRootRoute] {
        guard presentsContent else { return [] }
        return activePanel == .chat ? [.conversation] : [.panel(activePanel)]
    }

    public mutating func popToHome() {
        presentsContent = false
        mainPanelBackStack.removeAll()
        workspaceBotsDrilldown = nil
        activeSettingsTab = .manage
    }

    public var currentRoute: GaryxMobilePanelRoute {
        GaryxMobilePanelRoute(
            panel: activePanel,
            settingsTab: activeSettingsTab,
            workspaceBotsDrilldown: activePanel == .workspaceBots ? workspaceBotsDrilldown : nil
        )
    }

    public var leadingEdgeAction: GaryxMobileLeadingEdgeAction {
        if activePanel == .workspaceBots, workspaceBotsDrilldown != nil {
            return .workspaceBotsOverview
        }
        if activePanel == .settings, activeSettingsTab != .manage {
            return .settingsOverview
        }
        if !mainPanelBackStack.isEmpty {
            return .mainPanelBack
        }
        return presentsContent ? .popToHome : .openSidebar
    }

    public mutating func setActivePanel(_ panel: GaryxMobilePanel) {
        guard activePanel != panel else {
            presentsContent = true
            return
        }
        activePanel = panel
        presentsContent = true
        mainPanelBackStack.removeAll()
        if panel != .workspaceBots {
            workspaceBotsDrilldown = nil
        }
    }

    public mutating func setWorkspaceBotsDrilldown(_ drilldown: GaryxWorkspaceBotsDrilldown?) {
        workspaceBotsDrilldown = drilldown
    }

    public mutating func openPanel(
        _ panel: GaryxMobilePanel,
        dreamsAutoScanEnabled: Bool,
        source: GaryxMobilePanelOpenSource
    ) {
        let targetPanel = resolvedPanel(panel, dreamsAutoScanEnabled: dreamsAutoScanEnabled)
        let route = GaryxMobilePanelRoute(
            panel: targetPanel,
            settingsTab: targetPanel == .settings ? activeSettingsTab : .manage
        )
        let resolvedSource: GaryxMobilePanelOpenSource = panel == .dreams && targetPanel == .chat
            ? .replace
            : source
        openRoute(route, source: resolvedSource)
    }

    public mutating func openSettings(
        tab: GaryxMobileSettingsTab = .manage,
        source: GaryxMobilePanelOpenSource
    ) {
        openRoute(GaryxMobilePanelRoute(panel: .settings, settingsTab: tab), source: source)
    }

    public mutating func openRoute(_ route: GaryxMobilePanelRoute, source: GaryxMobilePanelOpenSource) {
        let previousRoute = currentRoute
        switch source {
        case .current:
            // Only an already-presented page can be a back target; opening
            // from the home list starts a fresh content stack.
            if presentsContent, previousRoute != route, mainPanelBackStack.last != previousRoute {
                mainPanelBackStack.append(previousRoute)
            }
        case .sidebar, .replace:
            mainPanelBackStack.removeAll()
        }

        presentsContent = true
        apply(route)
    }

    public mutating func showSettingsOverview() {
        activeSettingsTab = .manage
    }

    @discardableResult
    public mutating func goBackInMainPanel() -> Bool {
        guard let previousRoute = mainPanelBackStack.popLast() else {
            return false
        }
        apply(previousRoute)
        return true
    }

    public mutating func showWorkspaceBotsOverview() {
        workspaceBotsDrilldown = nil
    }

    private mutating func apply(_ route: GaryxMobilePanelRoute) {
        activePanel = route.panel
        activeSettingsTab = route.panel == .settings ? route.settingsTab : .manage
        workspaceBotsDrilldown = route.panel == .workspaceBots ? route.workspaceBotsDrilldown : nil
    }

    private func resolvedPanel(
        _ panel: GaryxMobilePanel,
        dreamsAutoScanEnabled: Bool
    ) -> GaryxMobilePanel {
        switch panel {
        case .workspaces:
            // Legacy workspace links land on the workspace-threads page; the
            // .workspaces panel itself is the file browser.
            .workspaceBots
        case .dreams where !dreamsAutoScanEnabled:
            .chat
        default:
            panel
        }
    }
}
