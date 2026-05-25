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
            "Workspace & Bots"
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
            "person.2.wave.2.fill"
        case .skills:
            "wand.and.stars"
        case .commands:
            "command"
        case .mcp:
            "point.3.connected.trianglepath.dotted"
        case .autoResearch:
            "atom"
        case .workspaceBots:
            "folder"
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

public enum GaryxMobileLeadingEdgeAction: Equatable, Sendable {
    case openSidebar
    case mainPanelBack
    case settingsOverview
    case workspaceBotsOverview
}

public enum GaryxMobilePanelOpenSource: Equatable, Sendable {
    case current
    case sidebar
    case replace
}

public struct GaryxMobilePanelRoute: Equatable, Sendable {
    public let panel: GaryxMobilePanel
    public let settingsTab: GaryxMobileSettingsTab

    public init(panel: GaryxMobilePanel, settingsTab: GaryxMobileSettingsTab) {
        self.panel = panel
        self.settingsTab = settingsTab
    }
}

public struct GaryxMobileNavigationState: Equatable, Sendable {
    public private(set) var activePanel: GaryxMobilePanel
    public var activeSettingsTab: GaryxMobileSettingsTab
    public var workspaceBotsDrilldownActive: Bool
    public var workspaceBotsBackRequest: Int
    public private(set) var mainPanelBackStack: [GaryxMobilePanelRoute]

    public init(
        activePanel: GaryxMobilePanel = .chat,
        activeSettingsTab: GaryxMobileSettingsTab = .manage,
        workspaceBotsDrilldownActive: Bool = false,
        workspaceBotsBackRequest: Int = 0,
        mainPanelBackStack: [GaryxMobilePanelRoute] = []
    ) {
        self.activePanel = activePanel
        self.activeSettingsTab = activeSettingsTab
        self.workspaceBotsDrilldownActive = workspaceBotsDrilldownActive
        self.workspaceBotsBackRequest = workspaceBotsBackRequest
        self.mainPanelBackStack = mainPanelBackStack
    }

    public var currentRoute: GaryxMobilePanelRoute {
        GaryxMobilePanelRoute(panel: activePanel, settingsTab: activeSettingsTab)
    }

    public var leadingEdgeAction: GaryxMobileLeadingEdgeAction {
        if activePanel == .workspaceBots, workspaceBotsDrilldownActive {
            return .workspaceBotsOverview
        }
        if activePanel == .settings, activeSettingsTab != .manage {
            return .settingsOverview
        }
        if !mainPanelBackStack.isEmpty {
            return .mainPanelBack
        }
        return .openSidebar
    }

    public mutating func setActivePanel(_ panel: GaryxMobilePanel) {
        guard activePanel != panel else { return }
        activePanel = panel
        mainPanelBackStack.removeAll()
        if panel != .workspaceBots {
            workspaceBotsDrilldownActive = false
        }
    }

    public mutating func setWorkspaceBotsDrilldownActive(_ active: Bool) {
        workspaceBotsDrilldownActive = active
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
            if previousRoute != route, mainPanelBackStack.last != previousRoute {
                mainPanelBackStack.append(previousRoute)
            }
        case .sidebar, .replace:
            mainPanelBackStack.removeAll()
        }

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

    public mutating func requestWorkspaceBotsOverview() {
        workspaceBotsBackRequest &+= 1
    }

    private mutating func apply(_ route: GaryxMobilePanelRoute) {
        activePanel = route.panel
        activeSettingsTab = route.panel == .settings ? route.settingsTab : .manage
        if route.panel != .workspaceBots {
            workspaceBotsDrilldownActive = false
        }
    }

    private func resolvedPanel(
        _ panel: GaryxMobilePanel,
        dreamsAutoScanEnabled: Bool
    ) -> GaryxMobilePanel {
        switch panel {
        case .bots, .workspaces:
            .workspaceBots
        case .dreams where !dreamsAutoScanEnabled:
            .chat
        default:
            panel
        }
    }
}
