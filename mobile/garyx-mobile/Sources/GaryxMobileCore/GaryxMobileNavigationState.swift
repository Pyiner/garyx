import Combine
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
    case workspaces
    case automations
    case capsules
    case agents
    case skills
    case commands
    case mcp
    case workspaceBots
    case bots
    case settings

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .chat:
            "Chat"
        case .workspaces:
            "Workspaces"
        case .automations:
            "Automation"
        case .capsules:
            "Capsules"
        case .agents:
            "Agents"
        case .skills:
            "Skills"
        case .commands:
            "Commands"
        case .mcp:
            "MCP"
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
        case .workspaces:
            "folder"
        case .automations:
            "clock.arrow.circlepath"
        case .capsules:
            "capsule.fill"
        case .agents:
            "person.2.fill"
        case .skills:
            "wand.and.stars"
        case .commands:
            "command"
        case .mcp:
            "point.3.connected.trianglepath.dotted"
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

struct GaryxShellChromeSnapshot: Equatable, Sendable {
    var sidebarVisible: Bool

    init(sidebarVisible: Bool = false) {
        self.sidebarVisible = sidebarVisible
    }
}

@MainActor
final class GaryxShellChromeStore: ObservableObject {
    @Published private(set) var snapshot: GaryxShellChromeSnapshot
    private(set) var publishCount = 0

    init(snapshot: GaryxShellChromeSnapshot = .init()) {
        self.snapshot = snapshot
    }

    @discardableResult
    func apply(_ nextSnapshot: GaryxShellChromeSnapshot) -> Bool {
        guard snapshot != nextSnapshot else { return false }
        snapshot = nextSnapshot
        publishCount += 1
        return true
    }
}

struct GaryxNavigationDrawerWorkspaceRow: Identifiable, Equatable, Sendable {
    var path: String
    var name: String

    var id: String { path }
}

struct GaryxNavigationDrawerSnapshot: Equatable, Sendable {
    var activePanel: GaryxMobilePanel
    var gatewayIdentity: GaryxGatewaySwitcherIdentity
    var gatewayRows: [GaryxGatewaySwitcherRow]
    var botGroups: [GaryxMobileBotGroup]
    var workspaceRows: [GaryxNavigationDrawerWorkspaceRow]

    init(
        activePanel: GaryxMobilePanel = .chat,
        gatewayIdentity: GaryxGatewaySwitcherIdentity = GaryxGatewaySwitcherIdentity(
            title: GaryxGatewaySwitcherPresentation.unconfiguredTitle,
            subtitle: nil,
            status: .notConnected,
            isInteractive: false
        ),
        gatewayRows: [GaryxGatewaySwitcherRow] = [],
        botGroups: [GaryxMobileBotGroup] = [],
        workspaceRows: [GaryxNavigationDrawerWorkspaceRow] = []
    ) {
        self.activePanel = activePanel
        self.gatewayIdentity = gatewayIdentity
        self.gatewayRows = gatewayRows
        self.botGroups = botGroups
        self.workspaceRows = workspaceRows
    }
}

@MainActor
final class GaryxNavigationDrawerStore: ObservableObject {
    @Published private(set) var snapshot: GaryxNavigationDrawerSnapshot
    private(set) var publishCount = 0

    init(snapshot: GaryxNavigationDrawerSnapshot = .init()) {
        self.snapshot = snapshot
    }

    @discardableResult
    func apply(_ nextSnapshot: GaryxNavigationDrawerSnapshot) -> Bool {
        guard snapshot != nextSnapshot else { return false }
        snapshot = nextSnapshot
        publishCount += 1
        return true
    }
}

public enum GaryxMobilePanelOpenSource: Equatable, Sendable {
    case current
    case sidebar
    case replace
    /// External links replace the complete occurrence chain without an
    /// animated intermediate route, including when home is currently visible.
    case deepLink
    /// Present a focused detail above the current conversation and dismiss back
    /// to it, instead of switching to that detail's management panel. Used by
    /// in-transcript capsule cards so opening a capsule never lands the user on
    /// the Capsules overview (mobile-ui: drilldowns never back to an overview).
    /// The capsule route handles this before `openRoute`; this case maps to a
    /// fresh content stack only for exhaustiveness.
    case conversation
}

public enum GaryxWorkspaceBotsDrilldown: Equatable, Sendable {
    case bot(String)
    case workspace(String)
    case automationThreads(String)
}

/// Read-only product projection of the canonical route stack.
///
/// The occurrence path is the only navigation truth. This projection exists
/// solely for feature code that needs to react to the currently visible
/// module (stream ownership, drawer selection, and restoration policy); it
/// never stores predecessors and cannot mutate navigation.
public struct GaryxMobileNavigationState: Equatable, Sendable {
    public private(set) var activePanel: GaryxMobilePanel
    public private(set) var activeSettingsTab: GaryxMobileSettingsTab
    public private(set) var workspaceBotsDrilldown: GaryxWorkspaceBotsDrilldown?
    public private(set) var presentsContent: Bool

    public init(
        activePanel: GaryxMobilePanel = .chat,
        activeSettingsTab: GaryxMobileSettingsTab = .manage,
        workspaceBotsDrilldown: GaryxWorkspaceBotsDrilldown? = nil,
        presentsContent: Bool = false
    ) {
        self.activePanel = activePanel
        self.activeSettingsTab = activeSettingsTab
        self.workspaceBotsDrilldown = workspaceBotsDrilldown
        self.presentsContent = presentsContent
    }

    public init(projecting path: [GaryxRouteEntry]) {
        self.init()
        guard let destination = path.last?.destination else { return }
        presentsContent = true
        switch destination {
        case .conversation, .conversationDraft:
            activePanel = .chat
        case .panel(let rawPanel):
            activePanel = GaryxMobilePanel(rawValue: rawPanel) ?? .chat
        case .settingsDetail(let rawTab):
            activePanel = .settings
            activeSettingsTab = GaryxMobileSettingsTab(rawValue: rawTab) ?? .manage
        case .workspaceDrilldown(let identity):
            activePanel = .workspaceBots
            switch identity {
            case .workspace(let path):
                workspaceBotsDrilldown = .workspace(path)
            case .bot(let accountID):
                workspaceBotsDrilldown = .bot(accountID)
            case .automationThreads(let automationID):
                workspaceBotsDrilldown = .automationThreads(automationID)
            }
        }
    }
}
