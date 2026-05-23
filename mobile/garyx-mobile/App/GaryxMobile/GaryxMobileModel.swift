import Foundation
import SwiftUI
import UniformTypeIdentifiers

enum GaryxMobileConnectionState: Equatable {
    case disconnected
    case checking
    case ready(version: String?)
    case failed(String)

    var label: String {
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

enum GaryxMobilePanel: String, CaseIterable, Identifiable {
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

    var id: String { rawValue }

    var label: String {
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

    var iconName: String {
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

enum GaryxMobileSettingsTab: String, CaseIterable, Identifiable {
    case manage
    case gateway
    case provider
    case channels
    case commands
    case mcp

    var id: String { rawValue }

    var label: String {
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

    var iconName: String {
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

enum GaryxMobileLeadingEdgeAction {
    case openSidebar
    case settingsOverview
}

struct GaryxMobileAgentTarget: Identifiable, Equatable {
    enum Kind: Equatable {
        case agent
        case team
    }

    let id: String
    let title: String
    let subtitle: String
    let kind: Kind
    let avatarDataUrl: String
    let providerType: String
    let builtIn: Bool
}

struct GaryxMobileBotGroup: Identifiable, Equatable {
    let id: String
    let channel: String
    let accountId: String
    let title: String
    let subtitle: String
    let agentId: String?
    let rootBehavior: String
    let status: String
    let endpointCount: Int
    let boundEndpointCount: Int
    let workspaceDir: String?
    let mainThreadId: String?
    let defaultOpenThreadId: String?
    let endpoints: [GaryxChannelEndpoint]
    let conversationNodes: [GaryxBotConversationNode]
    let iconDataUrl: String?
}

private struct GaryxPendingUploadPreview {
    var name: String
    var mediaType: String
    var previewDataUrl: String?
}

private struct GaryxPendingQueuedInput {
    var threadId: String
    var text: String
    var attachments: [GaryxMobileComposerAttachment]
    var clientIntentId: String
}

struct GaryxGatewayProfile: Identifiable, Codable, Equatable {
    var id: String
    var label: String
    var gatewayUrl: String
    var updatedAt: Date
    var hasToken: Bool
}

@MainActor
final class GaryxMobileModel: ObservableObject {
    private static let threadListPageLimit = 80
    private static let threadHistoryPageLimit = 120
    private static let threadHistoryUserQueryLimit = 10
    private static let selectedThreadReconcileIntervalNanos: UInt64 = 1_500_000_000
    private static let assistantDeltaFlushDelayNanos: UInt64 = 50_000_000
    private static let gatewayReconnectInitialDelayNanos: UInt64 = 1_000_000_000
    private static let gatewayReconnectMaxDelayNanos: UInt64 = 10_000_000_000

    private struct MessageListSignature: Equatable {
        let count: Int
        let fingerprint: Int
    }

    private struct TurnRowsCacheKey: Equatable {
        let isRunning: Bool
        let messages: MessageListSignature
    }

    private struct PendingAssistantDelta {
        var targetId: String
        var text: String
    }

    @Published var gatewayURL: String
    @Published var gatewayAuthToken: String
    @Published var gatewayProfiles: [GaryxGatewayProfile]
    @Published var gatewaySettingsStatus: String?
    @Published var connectionState: GaryxMobileConnectionState = .disconnected
    @Published var threads: [GaryxThreadSummary] = []
    @Published var selectedThread: GaryxThreadSummary?
    @Published var messages: [GaryxMobileMessage] = [] {
        didSet {
            selectedMessagesSignature = Self.messageListSignature(for: messages)
            selectedThreadTurnRowsCacheKey = nil
        }
    }
    @Published var draft = ""
    @Published private(set) var composerContextVersion = 0
    @Published var composerAttachments: [GaryxMobileComposerAttachment] = []
    @Published var isLoadingThreads = false
    @Published var isLoadingMoreThreads = false
    @Published var hasMoreThreadSummaries = false
    @Published var isLoadingSelectedThreadHistory = false
    @Published var isLoadingOlderThreadHistory = false
    @Published var selectedThreadHasMoreHistoryBefore = false
    @Published var isSending = false
    @Published var activeRunThreadId: String?
    @Published private(set) var remoteBusyThreadIds: Set<String> = []
    @Published var activePanel: GaryxMobilePanel = .chat
    @Published var activeSettingsTab: GaryxMobileSettingsTab = .manage
    @Published private var storedLastError: String?
    var lastError: String? {
        get {
            storedLastError
        }
        set {
            storedLastError = Self.presentableErrorMessage(newValue)
        }
    }
    @Published var showsSettings = false
    @Published var sidebarVisible = false
    @Published var pinnedThreadIds: [String] = []
    @Published var recentThreadIds: [String] = []
    @Published var dreams: [GaryxDreamTopic] = []
    @Published var latestDreamScan: GaryxDreamScan?
    @Published var isScanningDreams = false
    @Published var dreamsAutoScanEnabled = false
    @Published var isSavingDreamsSettings = false
    @Published var agents: [GaryxAgentSummary] = []
    @Published var teams: [GaryxTeamSummary] = []
    @Published var skills: [GaryxSkillSummary] = []
    @Published var tasks: [GaryxTaskSummary] = []
    @Published var automations: [GaryxAutomationSummary] = []
    @Published var isLoadingRemoteState = false
    @Published var selectedAgentTargetId: String
    @Published var newThreadWorkspace: String
    @Published var newThreadWorkspaceMode: String
    @Published var draftTaskTitle = ""
    @Published var draftTaskBody = ""
    @Published var lastAutomationRun: GaryxAutomationActivityEntry?
    @Published var selectedWorkspacePath = ""
    @Published var selectedWorkspaceDirectory = ""
    @Published var draftWorkspacePath = ""
    @Published var workspaceListing: GaryxWorkspaceFileListing?
    @Published var workspacePreview: GaryxWorkspaceFilePreview?
    @Published var workspaceGitStatuses: [String: GaryxWorkspaceGitStatus] = [:]
    @Published var isUploadingWorkspaceFiles = false
    @Published var workspaceUploadStatus: String?
    @Published var slashCommands: [GaryxSlashCommand] = []
    @Published var mcpServers: [GaryxMcpServer] = []
    @Published var autoResearchRuns: [GaryxAutoResearchRun] = []
    @Published var channelEndpoints: [GaryxChannelEndpoint] = []
    @Published var configuredBots: [GaryxConfiguredBot] = []
    @Published var botConsoles: [GaryxBotConsoleSummary] = []
    @Published var botStatusesById: [String: GaryxBotBindingResult] = [:]
    @Published var channelPlugins: [GaryxChannelPluginCatalogEntry] = []
    @Published var providerModelsByType: [String: GaryxProviderModels] = [:]
    @Published var selectedSkillEditor: GaryxSkillEditorState?
    @Published var selectedSkillDocument: GaryxSkillFileDocument?
    @Published var selectedSkillFileContent = ""
    @Published var researchCandidatesByRunId: [String: GaryxAutoResearchCandidatesPage] = [:]
    @Published var autoResearchDetailsByRunId: [String: GaryxAutoResearchDetail] = [:]
    @Published var autoResearchIterationsByRunId: [String: [GaryxAutoResearchIteration]] = [:]
    @Published var draftThreadTitle = ""
    @Published var draftAutomationLabel = ""
    @Published var draftAutomationPrompt = ""
    @Published var draftAutomationIntervalHours = "24"
    @Published var draftAutomationTargetsExistingThread = false
    @Published var draftAutomationTargetThreadId = ""
    @Published var draftAgentId = ""
    @Published var draftAgentName = ""
    @Published var draftAgentProvider = "codex_app_server"
    @Published var draftAgentModel = ""
    @Published var draftAgentWorkspace = ""
    @Published var draftAgentPrompt = ""
    @Published var draftTeamId = ""
    @Published var draftTeamName = ""
    @Published var draftTeamLeaderId = ""
    @Published var draftTeamMemberIds = ""
    @Published var draftTeamWorkflow = ""
    @Published var draftSkillId = ""
    @Published var draftSkillName = ""
    @Published var draftSkillDescription = ""
    @Published var draftSkillBody = ""
    @Published var draftSkillEntryPath = ""
    @Published var draftSkillEntryType = "file"
    @Published var draftSlashName = ""
    @Published var draftSlashDescription = ""
    @Published var draftSlashPrompt = ""
    @Published var draftMcpName = ""
    @Published var draftMcpCommand = ""
    @Published var draftMcpArgs = ""
    @Published var draftMcpEnv = ""
    @Published var draftMcpWorkingDir = ""
    @Published var draftMcpUrl = ""
    @Published var draftMcpHeaders = ""
    @Published var draftAutoResearchGoal = ""
    @Published var draftAutoResearchIterations = "3"
    @Published var draftAutoResearchTimeBudgetMinutes = "15"

    private let defaults: UserDefaults
    private let keychain: GaryxMobileKeychain
    private var activeTask: URLSessionWebSocketTask?
    private var activeReaderTask: Task<Void, Never>?
    private var activeTasksByThread: [String: URLSessionWebSocketTask] = [:]
    private var activeReaderTasksByThread: [String: Task<Void, Never>] = [:]
    private var globalEventStreamTask: Task<Void, Never>?
    private var globalEventStreamGeneration: UUID?
    private var globalEventStreamActive = false
    private var gatewayReconnectTask: Task<Void, Never>?
    private var gatewayReconnectGeneration: UUID?
    private var selectedThreadReconcileTask: Task<Void, Never>?
    private var selectedThreadReconcileThreadId: String?
    private var selectedThreadActivitySignatures: [String: String] = [:]
    private var messagesByThread: [String: [GaryxMobileMessage]] = [:]
    private var messageSignaturesByThread: [String: MessageListSignature] = [:]
    private var selectedMessagesSignature = MessageListSignature(count: 0, fingerprint: 0)
    private var selectedThreadTurnRowsCacheKey: TurnRowsCacheKey?
    private var selectedThreadTurnRowsCache: [GaryxMobileTurnRow] = []
    private var activeAssistantMessageIdsByThread: [String: String] = [:]
    private var pendingAssistantDeltasByThread: [String: PendingAssistantDelta] = [:]
    private var assistantDeltaFlushTasksByThread: [String: Task<Void, Never>] = [:]
    private var pendingQueuedInputsByIntentId: [String: GaryxPendingQueuedInput] = [:]
    private var gatewayRuntimeGeneration = UUID()
    private var selectedThreadRecoveryTask: Task<Void, Never>?
    private var selectedThreadRecoveryThreadId: String?
    private var selectedThreadHistoryRequestId: UUID?
    private var nextThreadListOffset = 0
    private var selectedThreadNextHistoryBeforeIndex: Int?
    private var sceneRefreshTask: Task<Void, Never>?
    private var pendingBotId: String?
    private var pendingBotWorkspace: String?
    private var pendingBotAgentId: String?
    #if DEBUG
    private(set) var debugSnapshotActive = false
    #endif

    init(defaults: UserDefaults = .standard, keychain: GaryxMobileKeychain = .shared) {
        self.defaults = defaults
        self.keychain = keychain
        gatewayURL = Self.firstNonEmpty(
            defaults.string(forKey: GaryxMobileSettingsKeys.gatewayUrl),
            defaults.string(forKey: GaryxMobileSettingsKeys.legacyGatewayURL)
        ) ?? Self.defaultGatewayURL
        let storedToken = keychain.readGatewayAuthToken()
        let legacyToken = defaults.string(forKey: GaryxMobileSettingsKeys.legacyGatewayToken) ?? ""
        gatewayAuthToken = storedToken.isEmpty ? legacyToken : storedToken
        if !legacyToken.isEmpty && storedToken.isEmpty {
            keychain.saveGatewayAuthToken(legacyToken)
            defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayToken)
        }
        gatewayProfiles = Self.loadGatewayProfiles(defaults: defaults)
        selectedAgentTargetId = defaults.string(forKey: GaryxMobileSettingsKeys.selectedAgentTargetId) ?? "claude"
        newThreadWorkspace = defaults.string(forKey: GaryxMobileSettingsKeys.newThreadWorkspace) ?? ""
        newThreadWorkspaceMode = Self.normalizedWorkspaceMode(
            defaults.string(forKey: GaryxMobileSettingsKeys.newThreadWorkspaceMode)
        )
        loadGatewayScopedUserState(fallbackToLegacy: true)

        #if DEBUG
        let debugEnvironment = ProcessInfo.processInfo.environment
        if debugEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] == "1" {
            loadDebugSnapshot()
            applyDebugDestination(
                panelName: debugEnvironment["GARYX_MOBILE_DEBUG_PANEL"],
                tabName: debugEnvironment["GARYX_MOBILE_DEBUG_SETTINGS_TAB"],
                showSidebar: debugEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] == "1"
            )
        }
        #endif
    }

    private static var defaultGatewayURL: String {
        #if targetEnvironment(simulator)
        "http://127.0.0.1:31337"
        #else
        ""
        #endif
    }

    private static func firstNonEmpty(_ values: String?...) -> String? {
        values
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }
    }

    private static func normalizedWorkspaceMode(_ value: String?) -> String {
        let normalized = value?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        return normalized == "worktree" ? "worktree" : "local"
    }

    private static func loadGatewayProfiles(defaults: UserDefaults) -> [GaryxGatewayProfile] {
        guard let data = defaults.data(forKey: GaryxMobileSettingsKeys.gatewayProfiles) else {
            return []
        }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        guard let profiles = try? decoder.decode([GaryxGatewayProfile].self, from: data) else {
            return []
        }
        return normalizedGatewayProfiles(profiles)
    }

    private static func normalizedGatewayProfiles(_ profiles: [GaryxGatewayProfile]) -> [GaryxGatewayProfile] {
        var byKey: [String: GaryxGatewayProfile] = [:]
        for profile in profiles {
            let url = normalizedGatewayProfileURL(profile.gatewayUrl)
            guard !url.isEmpty else { continue }
            let key = url.lowercased()
            var normalized = profile
            normalized.gatewayUrl = url
            normalized.id = stableGatewayProfileId(for: url)
            normalized.label = profile.label.trimmingCharacters(in: .whitespacesAndNewlines)
            if normalized.label.isEmpty {
                normalized.label = gatewayProfileLabel(for: url)
            }
            if let current = byKey[key], current.updatedAt >= normalized.updatedAt {
                continue
            }
            byKey[key] = normalized
        }
        return byKey.values
            .sorted { $0.updatedAt > $1.updatedAt }
            .prefix(8)
            .map { $0 }
    }

    private static func normalizedGatewayProfileURL(_ value: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        return trimmed.replacingOccurrences(
            of: "/+$",
            with: "",
            options: .regularExpression
        )
    }

    private static func stableGatewayProfileId(for gatewayUrl: String) -> String {
        var hash: UInt64 = 14695981039346656037
        for byte in gatewayUrl.lowercased().utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 1099511628211
        }
        return String(format: "gateway::%016llx", hash)
    }

    private static func gatewayProfileLabel(for gatewayUrl: String) -> String {
        guard let url = URL(string: gatewayUrl) else {
            return gatewayUrl
        }
        if let host = url.host, let port = url.port {
            return "\(host):\(port)"
        }
        return url.host ?? gatewayUrl
    }

    private static func botGroupKey(channel: String, accountId: String) -> String {
        "\(channel.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())::\(accountId.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())"
    }

    private static func botSelectorId(channel: String, accountId: String) -> String {
        "\(channel.trimmingCharacters(in: .whitespacesAndNewlines)):\(accountId.trimmingCharacters(in: .whitespacesAndNewlines))"
    }

    private static func removeChannelAccount(
        from settings: inout [String: GaryxJSONValue],
        channel: String,
        accountId: String
    ) -> Bool {
        guard var channels = settings["channels"]?.objectValue else { return false }
        let channelKey = channels.keys.first {
            $0.caseInsensitiveCompare(channel.trimmingCharacters(in: .whitespacesAndNewlines)) == .orderedSame
        }
        guard let channelKey,
              var channelConfig = channels[channelKey]?.objectValue,
              var accounts = channelConfig["accounts"]?.objectValue,
              accounts.keys.contains(accountId) else {
            return false
        }

        accounts.removeValue(forKey: accountId)
        channelConfig["accounts"] = .object(accounts)
        channels[channelKey] = .object(channelConfig)
        settings["channels"] = .object(channels)
        return true
    }

    private static func channelDisplayName(_ channel: String) -> String {
        let normalized = channel.trimmingCharacters(in: .whitespacesAndNewlines)
        switch normalized.lowercased() {
        case "telegram":
            return "Telegram"
        case "feishu":
            return "Feishu"
        case "weixin":
            return "Weixin"
        case "discord":
            return "Discord"
        case "api":
            return "API"
        default:
            return normalized.isEmpty ? "Channel" : normalized
        }
    }

    nonisolated static func isVisibleMobileWorkspacePath(_ path: String) -> Bool {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        let normalized = trimmed.replacingOccurrences(of: "\\", with: "/")
        if normalized.contains("/.garyx/worktrees/") || normalized.contains("/.codex/worktrees/") {
            return false
        }
        return true
    }

    nonisolated private static func normalizedWorkspacePathKey(_ path: String) -> String {
        path.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\\", with: "/")
    }

    var hasGatewaySettings: Bool {
        !gatewayURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var canConnectGateway: Bool {
        parsedGatewayURL(from: gatewayURL) != nil
    }

    var currentGatewayProfile: GaryxGatewayProfile? {
        let currentURL = normalizedGatewayURL(gatewayURL).lowercased()
        return gatewayProfiles.first { $0.gatewayUrl.lowercased() == currentURL }
    }

    var canSend: Bool {
        canSendComposerPayload(text: draft, attachments: composerAttachments)
    }

    var hasComposerPayload: Bool {
        !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !composerAttachments.isEmpty
    }

    func canSendComposerPayload(text: String, attachments: [GaryxMobileComposerAttachment]) -> Bool {
        let hasPayload = !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty
        return hasPayload && (
            (!isSelectedThreadSending && !isSelectedThreadRemoteBusy)
                || canQueueSelectedThreadInput
        )
    }

    var canQueueSelectedThreadInput: Bool {
        guard let selectedThread else { return false }
        return isThreadBusy(selectedThread.id)
    }

    var isSelectedThreadSending: Bool {
        guard let selectedThread else {
            return false
        }
        return activeTasksByThread[selectedThread.id] != nil
            || (isSending && activeRunThreadId == selectedThread.id)
            || remoteBusyThreadIds.contains(selectedThread.id)
            || threads.contains { thread in
                thread.id == selectedThread.id
                    && !(thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true)
            }
    }

    var isSelectedThreadRemoteBusy: Bool {
        guard let selectedThread else { return false }
        return remoteBusyThreadIds.contains(selectedThread.id)
    }

    var showsTailThinkingIndicator: Bool {
        guard isSelectedThreadSending else { return false }
        if let last = messages.last,
           last.role == .assistant,
           last.isStreaming,
           last.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return false
        }
        if let last = messages.last,
           last.role == .tool,
           last.toolTraceGroup?.isActive == true {
            return false
        }
        return true
    }

    func isThreadBusy(_ threadId: String) -> Bool {
        activeRunThreadId == threadId
            || activeTasksByThread[threadId] != nil
            || remoteBusyThreadIds.contains(threadId)
            || threads.contains { thread in
                thread.id == threadId
                    && !(thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true)
            }
    }

    func canDeleteThread(_ thread: GaryxThreadSummary) -> Bool {
        guard !isThreadBusy(thread.id) else { return false }
        if automations.contains(where: { $0.threadId == thread.id }) {
            return false
        }
        let liveBotKeys = Set(
            configuredBots
                .filter(\.enabled)
                .map { "\($0.channel):\($0.accountId)" }
        )
        if channelEndpoints.contains(where: { endpoint in
            endpoint.threadId == thread.id && liveBotKeys.contains("\(endpoint.channel):\(endpoint.accountId)")
        }) {
            return false
        }
        return true
    }

    private func cachedMessages(for threadId: String) -> [GaryxMobileMessage] {
        messagesByThread[threadId] ?? []
    }

    func selectedThreadTurnRows() -> [GaryxMobileTurnRow] {
        let isRunning = isSelectedThreadSending
        let key = TurnRowsCacheKey(
            isRunning: isRunning,
            messages: selectedMessagesSignature
        )
        if selectedThreadTurnRowsCacheKey != key {
            selectedThreadTurnRowsCache = GaryxMobileTurnRenderer.buildTurnRows(
                messages: messages,
                isRunningThread: isRunning
            )
            selectedThreadTurnRowsCacheKey = key
        }
        return selectedThreadTurnRowsCache
    }

    private func setMessages(
        _ nextMessages: [GaryxMobileMessage],
        for threadId: String,
        reconcileActiveAssistant: Bool = false
    ) {
        var adjustedMessages = nextMessages
        if reconcileActiveAssistant {
            reconcileActiveAssistantMessageId(threadId: threadId, messages: &adjustedMessages)
        }
        let nextSignature = Self.messageListSignature(for: adjustedMessages)
        if messageSignaturesByThread[threadId] == nextSignature,
           (selectedThread?.id != threadId || selectedMessagesSignature == nextSignature) {
            return
        }
        messagesByThread[threadId] = adjustedMessages
        messageSignaturesByThread[threadId] = nextSignature
        if selectedThread?.id == threadId {
            messages = adjustedMessages
        }
    }

    private func reconcileActiveAssistantMessageId(threadId: String, messages: inout [GaryxMobileMessage]) {
        let isBusy = activeTasksByThread[threadId] != nil || remoteBusyThreadIds.contains(threadId)
        guard isBusy else {
            activeAssistantMessageIdsByThread[threadId] = nil
            return
        }
        if let activeId = activeAssistantMessageIdsByThread[threadId],
           let index = messages.firstIndex(where: { $0.id == activeId && $0.role == .assistant }) {
            messages[index].isStreaming = true
            return
        }
        if let index = messages.indices.last(where: { messages[$0].role == .assistant && messages[$0].isStreaming }) {
            messages[index].isStreaming = true
            activeAssistantMessageIdsByThread[threadId] = messages[index].id
        } else {
            activeAssistantMessageIdsByThread[threadId] = nil
        }
    }

    private func clearMessages(for threadId: String) {
        discardPendingAssistantDelta(for: threadId)
        messagesByThread[threadId] = []
        messageSignaturesByThread[threadId] = Self.messageListSignature(for: [])
        activeAssistantMessageIdsByThread[threadId] = nil
        if selectedThread?.id == threadId {
            messages = []
        }
    }

    private func resetSelectedThreadHistoryPagination() {
        isLoadingOlderThreadHistory = false
        selectedThreadHasMoreHistoryBefore = false
        selectedThreadNextHistoryBeforeIndex = nil
    }

    private func resetThreadListPagination() {
        isLoadingMoreThreads = false
        hasMoreThreadSummaries = false
        nextThreadListOffset = 0
    }

    private func syncVisibleMessages(for threadId: String) {
        if selectedThread?.id == threadId {
            messages = cachedMessages(for: threadId)
        }
    }

    private func mutateMessages(for threadId: String, _ update: (inout [GaryxMobileMessage]) -> Void) {
        var nextMessages = cachedMessages(for: threadId)
        update(&nextMessages)
        setMessages(nextMessages, for: threadId)
    }

    private func resetComposerDraft() {
        draft = ""
        composerAttachments = []
        composerContextVersion &+= 1
    }

    private static func messageListSignature(for messages: [GaryxMobileMessage]) -> MessageListSignature {
        var hasher = Hasher()
        for message in messages {
            hasher.combine(message.id)
            hasher.combine(Self.roleSignature(message.role))
            hasher.combine(message.text)
            hasher.combine(message.timestamp)
            hasher.combine(message.isStreaming)
            hasher.combine(message.statusText)
            hasher.combine(message.clientIntentId)
            hasher.combine(message.pendingInputId)
            hasher.combine(message.attachments.count)
            for attachment in message.attachments {
                hasher.combine(attachment.id)
                hasher.combine(attachment.kind)
                hasher.combine(attachment.name)
                hasher.combine(attachment.mediaType)
                hasher.combine(attachment.path)
                hasher.combine(attachment.dataUrl)
                hasher.combine(attachment.remoteUrl)
            }
            if let group = message.toolTraceGroup {
                hasher.combine(group.live)
                hasher.combine(group.entries.count)
                for entry in group.entries {
                    hasher.combine(entry.id)
                    hasher.combine(entry.toolUseId)
                    hasher.combine(entry.parentToolUseId)
                    hasher.combine(entry.toolName)
                    hasher.combine(entry.title)
                    hasher.combine(entry.inputLabel)
                    hasher.combine(entry.resultLabel)
                    hasher.combine(entry.summaryText)
                    hasher.combine(entry.status.rawValue)
                    hasher.combine(entry.isError)
                    hasher.combine(entry.timestamp)
                    hasher.combine(entry.primaryPathBadge)
                    hasher.combine(entry.inputText)
                    hasher.combine(entry.resultText)
                }
            }
        }
        return MessageListSignature(count: messages.count, fingerprint: hasher.finalize())
    }

    private static func roleSignature(_ role: GaryxMobileMessage.Role) -> String {
        switch role {
        case .user:
            "user"
        case .assistant:
            "assistant"
        case .system:
            "system"
        case .tool:
            "tool"
        }
    }

    var agentTargets: [GaryxMobileAgentTarget] {
        let agentItems = agents
            .filter { $0.standalone }
            .map {
                GaryxMobileAgentTarget(
                    id: $0.id,
                    title: $0.displayName.isEmpty ? $0.id : $0.displayName,
                    subtitle: "",
                    kind: .agent,
                    avatarDataUrl: $0.avatarDataUrl,
                    providerType: $0.providerType,
                    builtIn: $0.builtIn
                )
            }
        let teamItems = teams.map {
            GaryxMobileAgentTarget(
                id: $0.id,
                title: $0.displayName.isEmpty ? $0.id : $0.displayName,
                subtitle: "\($0.memberAgentIds.count) agents",
                kind: .team,
                avatarDataUrl: $0.avatarDataUrl,
                providerType: "",
                builtIn: false
            )
        }
        return agentItems + teamItems
    }

    var selectedAgentTarget: GaryxMobileAgentTarget? {
        agentTargets.first(where: { $0.id == selectedAgentTargetId })
    }

    var selectedAgentLabel: String {
        selectedAgentTarget?.title ?? selectedAgentTargetId
    }

    var selectedThreadAgentTarget: GaryxMobileAgentTarget? {
        guard let thread = selectedThread else {
            return selectedAgentTarget
        }
        if let teamId = thread.teamId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !teamId.isEmpty,
           let target = agentTargets.first(where: { $0.id == teamId }) {
            return target
        }
        if let agentId = thread.agentId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !agentId.isEmpty,
           let target = agentTargets.first(where: { $0.id == agentId }) {
            return target
        }
        return nil
    }

    var selectedThreadAgentLabel: String {
        if let target = selectedThreadAgentTarget {
            return target.title
        }
        if let teamName = selectedThread?.teamName?.trimmingCharacters(in: .whitespacesAndNewlines),
           !teamName.isEmpty {
            return teamName
        }
        if let agentId = selectedThread?.agentId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !agentId.isEmpty {
            return agentId
        }
        return selectedAgentLabel
    }

    var mobileBotGroups: [GaryxMobileBotGroup] {
        let endpointsByGroup = Dictionary(grouping: channelEndpoints) { endpoint in
            Self.botGroupKey(channel: endpoint.channel, accountId: endpoint.accountId)
        }
        var configuredByGroup: [String: GaryxConfiguredBot] = [:]
        var groups: [String: GaryxMobileBotGroup] = [:]
        var order: [String] = []
        var orderedKeys = Set<String>()

        func rememberOrder(_ key: String) {
            if orderedKeys.insert(key).inserted {
                order.append(key)
            }
        }

        for bot in configuredBots {
            let key = Self.botGroupKey(channel: bot.channel, accountId: bot.accountId)
            if configuredByGroup[key] == nil {
                configuredByGroup[key] = bot
            }
            rememberOrder(key)
        }

        func remember(_ group: GaryxMobileBotGroup) {
            let key = Self.botGroupKey(channel: group.channel, accountId: group.accountId)
            rememberOrder(key)
            groups[key] = group
        }

        func iconDataUrl(for channel: String) -> String? {
            GaryxChannelIconResolver.iconDataUrl(for: channel, plugins: channelPlugins)
        }

        func nonEmpty(_ value: String?) -> String? {
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return trimmed.isEmpty ? nil : trimmed
        }

        for console in botConsoles {
            let key = Self.botGroupKey(channel: console.channel, accountId: console.accountId)
            let endpoints = endpointsByGroup[key] ?? []
            let decodedEndpointCount = endpoints.count
            let decodedBoundCount = endpoints.filter { $0.threadId?.isEmpty == false }.count
            let configured = configuredByGroup[key]
            remember(
                GaryxMobileBotGroup(
                    id: console.id.isEmpty ? "\(console.channel)::\(console.accountId)" : console.id,
                    channel: console.channel,
                    accountId: console.accountId,
                    title: console.title,
                    subtitle: console.subtitle,
                    agentId: nonEmpty(console.agentId) ?? nonEmpty(configured?.agentId),
                    rootBehavior: console.rootBehavior,
                    status: console.status,
                    endpointCount: max(console.endpointCount, decodedEndpointCount),
                    boundEndpointCount: max(console.boundEndpointCount, decodedBoundCount),
                    workspaceDir: nonEmpty(console.workspaceDir) ?? nonEmpty(configured?.workspaceDir),
                    mainThreadId: nonEmpty(console.mainThreadId) ?? nonEmpty(configured?.mainThreadId),
                    defaultOpenThreadId: nonEmpty(console.defaultOpenThreadId)
                        ?? nonEmpty(configured?.defaultOpenThreadId)
                        ?? nonEmpty(configured?.mainThreadId),
                    endpoints: endpoints,
                    conversationNodes: console.conversationNodes,
                    iconDataUrl: iconDataUrl(for: console.channel)
                )
            )
        }

        for bot in configuredBots {
            let key = Self.botGroupKey(channel: bot.channel, accountId: bot.accountId)
            if groups[key] != nil {
                continue
            }
            let endpoints = endpointsByGroup[key] ?? []
            remember(
                GaryxMobileBotGroup(
                    id: "\(bot.channel)::\(bot.accountId)",
                    channel: bot.channel,
                    accountId: bot.accountId,
                    title: bot.displayName,
                    subtitle: "\(Self.channelDisplayName(bot.channel)) Bot · \(bot.accountId)",
                    agentId: nonEmpty(bot.agentId),
                    rootBehavior: bot.rootBehavior,
                    status: bot.enabled ? "idle" : "disabled",
                    endpointCount: endpoints.count,
                    boundEndpointCount: endpoints.filter { $0.threadId?.isEmpty == false }.count,
                    workspaceDir: nonEmpty(bot.workspaceDir),
                    mainThreadId: nonEmpty(bot.mainThreadId),
                    defaultOpenThreadId: nonEmpty(bot.defaultOpenThreadId) ?? nonEmpty(bot.mainThreadId),
                    endpoints: endpoints,
                    conversationNodes: [],
                    iconDataUrl: iconDataUrl(for: bot.channel)
                )
            )
        }

        if groups.isEmpty {
            for (key, endpoints) in endpointsByGroup.sorted(by: { $0.key < $1.key }) {
                guard let first = endpoints.first else { continue }
                remember(
                    GaryxMobileBotGroup(
                        id: key,
                        channel: first.channel,
                        accountId: first.accountId,
                        title: "\(Self.channelDisplayName(first.channel)) / \(first.accountId)",
                        subtitle: "\(Self.channelDisplayName(first.channel)) Bot · \(first.accountId)",
                        agentId: nil,
                        rootBehavior: "open_default",
                        status: "idle",
                        endpointCount: endpoints.count,
                        boundEndpointCount: endpoints.filter { $0.threadId?.isEmpty == false }.count,
                        workspaceDir: nil,
                        mainThreadId: nil,
                        defaultOpenThreadId: endpoints.first(where: { $0.threadId?.isEmpty == false })?.threadId,
                        endpoints: endpoints,
                        conversationNodes: [],
                        iconDataUrl: iconDataUrl(for: first.channel)
                    )
                )
            }
        }

        return order.compactMap { groups[$0] }
    }

    var selectedThreadBotGroup: GaryxMobileBotGroup? {
        guard let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            return nil
        }
        return mobileBotGroups.first { group in
            if group.mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) == threadId {
                return true
            }
            if group.defaultOpenThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) == threadId {
                return true
            }
            return group.endpoints.contains { endpoint in
                endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) == threadId
            }
        }
    }

    var activeTaskCount: Int {
        tasks.filter { $0.status != .done }.count
    }

    var selectedThreadTask: GaryxTaskSummary? {
        guard let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            return nil
        }
        return taskSummary(forThreadId: threadId)
    }

    var enabledAutomationCount: Int {
        automations.filter(\.enabled).count
    }

    var knownWorkspacePaths: [String] {
        var seen = Set<String>()
        let worktreePaths = Set(
            threads
                .compactMap(\.worktreePath)
                .map(Self.normalizedWorkspacePathKey)
        )
        let values = threads.compactMap(\.workspacePath)
            + automations.map(\.workspacePath)
            + autoResearchRuns.compactMap(\.workspaceDir)
            + [newThreadWorkspace, selectedWorkspacePath]
        return values
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .filter(Self.isVisibleMobileWorkspacePath)
            .filter { !worktreePaths.contains(Self.normalizedWorkspacePathKey($0)) }
            .filter { seen.insert($0).inserted }
            .sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    }

    var runningResearchCount: Int {
        autoResearchRuns.filter { run in
            !garyxAutoResearchIsTerminal(run.state)
        }.count
    }

    var pinnedThreads: [GaryxThreadSummary] {
        var byId: [String: GaryxThreadSummary] = [:]
        for thread in threads {
            byId[thread.id] = thread
        }
        return pinnedThreadIds.compactMap { byId[$0] }
    }

    var recentThreads: [GaryxThreadSummary] {
        var byId: [String: GaryxThreadSummary] = [:]
        for thread in threads {
            byId[thread.id] = thread
        }
        return recentThreadIds.compactMap { byId[$0] }
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

    func openPanel(_ panel: GaryxMobilePanel) {
        let targetPanel: GaryxMobilePanel = switch panel {
        case .bots, .workspaces:
            .workspaceBots
        default:
            panel
        }
        guard targetPanel != .dreams || dreamsAutoScanEnabled else {
            activePanel = .chat
            setSidebarVisible(false)
            return
        }
        activePanel = targetPanel
        setSidebarVisible(false)
    }

    func openSettings(tab: GaryxMobileSettingsTab = .manage) {
        activeSettingsTab = tab
        openPanel(.settings)
    }

    var mainPanelLeadingEdgeAction: GaryxMobileLeadingEdgeAction {
        if activePanel == .settings, activeSettingsTab != .manage {
            return .settingsOverview
        }
        return .openSidebar
    }

    func performMainPanelLeadingEdgeAction() {
        switch mainPanelLeadingEdgeAction {
        case .openSidebar:
            setSidebarVisible(true)
        case .settingsOverview:
            showSettingsOverview()
        }
    }

    func showSettingsOverview() {
        activeSettingsTab = .manage
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

    private func applyDebugDestination(panelName: String?, tabName: String?, showSidebar: Bool = false) {
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

    private func loadDebugSnapshot() {
        debugSnapshotActive = true
        cancelGlobalEventStream()
        cancelActiveSocket()

        gatewayURL = "http://127.0.0.1:31337"
        gatewayAuthToken = "debug-token"
        gatewayProfiles = [
            GaryxGatewayProfile(
                id: Self.stableGatewayProfileId(for: "http://127.0.0.1:31337"),
                label: "127.0.0.1:31337",
                gatewayUrl: "http://127.0.0.1:31337",
                updatedAt: Date(timeIntervalSince1970: 1_779_172_400),
                hasToken: true
            ),
            GaryxGatewayProfile(
                id: Self.stableGatewayProfileId(for: "http://10.0.0.2:31337"),
                label: "10.0.0.2:31337",
                gatewayUrl: "http://10.0.0.2:31337",
                updatedAt: Date(timeIntervalSince1970: 1_779_168_800),
                hasToken: false
            ),
        ]
        keychain.saveGatewayProfileToken(
            "debug-token",
            profileId: Self.stableGatewayProfileId(for: "http://127.0.0.1:31337")
        )
        gatewaySettingsStatus = nil
        connectionState = .ready(version: "debug")
        isSending = false
        isLoadingThreads = false
        resetThreadListPagination()
        isLoadingRemoteState = false
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

    private static func decodeDebugFixture<T: Decodable>(_ type: T.Type, from json: String) -> T? {
        try? JSONDecoder().decode(type, from: Data(json.utf8))
    }
    #endif

    func isThreadPinned(_ threadId: String) -> Bool {
        pinnedThreadIds.contains(threadId.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    func togglePinnedThread(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        let pinned = !isThreadPinned(normalizedId)
        Task { await setThreadPinned(normalizedId, pinned: pinned) }
    }

    func unpinThread(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        Task { await setThreadPinned(normalizedId, pinned: false) }
    }

    func setThreadPinned(_ threadId: String, pinned: Bool) async {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return }
        let previousIds = pinnedThreadIds
        pinnedThreadIds = Self.pinnedThreadIdsWith(
            pinnedThreadIds,
            threadId: normalizedId,
            pinned: pinned
        )
        do {
            let page = try await client().setThreadPinned(threadId: normalizedId, pinned: pinned)
            applyPinnedThreadIds(page.threadIds)
        } catch {
            pinnedThreadIds = previousIds
            lastError = displayMessage(for: error)
        }
    }

    private func applyPinnedThreadIds(_ ids: [String]) {
        pinnedThreadIds = Self.normalizedPinnedThreadIds(ids)
    }

    private func removePinnedThreadIdLocally(_ threadId: String) {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        pinnedThreadIds.removeAll { $0 == normalizedId }
    }

    private static func pinnedThreadIdsWith(
        _ ids: [String],
        threadId: String,
        pinned: Bool
    ) -> [String] {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return normalizedPinnedThreadIds(ids) }
        let remaining = normalizedPinnedThreadIds(ids).filter { $0 != normalizedId }
        return pinned ? [normalizedId] + remaining : remaining
    }

    private static func normalizedPinnedThreadIds(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        var normalized: [String] = []
        for rawId in ids {
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { continue }
            normalized.append(id)
        }
        return normalized
    }

    func saveGatewaySettings() {
        gatewaySettingsStatus = nil
        gatewayURL = normalizedGatewayURL(gatewayURL)
        gatewayAuthToken = gatewayAuthToken.trimmingCharacters(in: .whitespacesAndNewlines)
        defaults.set(gatewayURL, forKey: GaryxMobileSettingsKeys.gatewayUrl)
        defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayURL)
        defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayToken)
        saveGatewayScopedUserState()
        keychain.saveGatewayAuthToken(gatewayAuthToken)
    }

    func saveGatewaySettingsFromUI() {
        saveGatewaySettings()
        rememberCurrentGatewayProfile()
        gatewaySettingsStatus = "Saved"
    }

    private var currentGatewayScopeId: String {
        let normalized = normalizedGatewayURL(gatewayURL)
        guard !normalized.isEmpty else { return "unconfigured" }
        return Self.stableGatewayProfileId(for: normalized)
    }

    private func scopedSettingsKey(_ key: String) -> String {
        "\(key).\(currentGatewayScopeId)"
    }

    private func loadGatewayScopedUserState(fallbackToLegacy: Bool) {
        let agentKey = scopedSettingsKey(GaryxMobileSettingsKeys.selectedAgentTargetId)
        let workspaceKey = scopedSettingsKey(GaryxMobileSettingsKeys.newThreadWorkspace)
        let workspaceModeKey = scopedSettingsKey(GaryxMobileSettingsKeys.newThreadWorkspaceMode)
        selectedAgentTargetId = defaults.string(forKey: agentKey)
            ?? (fallbackToLegacy ? defaults.string(forKey: GaryxMobileSettingsKeys.selectedAgentTargetId) : nil)
            ?? "claude"
        newThreadWorkspace = defaults.string(forKey: workspaceKey)
            ?? (fallbackToLegacy ? defaults.string(forKey: GaryxMobileSettingsKeys.newThreadWorkspace) : nil)
            ?? ""
        newThreadWorkspaceMode = Self.normalizedWorkspaceMode(
            defaults.string(forKey: workspaceModeKey)
                ?? (fallbackToLegacy ? defaults.string(forKey: GaryxMobileSettingsKeys.newThreadWorkspaceMode) : nil)
        )
    }

    private func saveGatewayScopedUserState() {
        defaults.set(selectedAgentTargetId, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.selectedAgentTargetId))
        defaults.set(
            newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines),
            forKey: scopedSettingsKey(GaryxMobileSettingsKeys.newThreadWorkspace)
        )
        defaults.set(
            Self.normalizedWorkspaceMode(newThreadWorkspaceMode),
            forKey: scopedSettingsKey(GaryxMobileSettingsKeys.newThreadWorkspaceMode)
        )
    }

    private func resetGatewayRuntimeState() {
        gatewayRuntimeGeneration = UUID()
        selectedThreadRecoveryTask?.cancel()
        selectedThreadRecoveryTask = nil
        selectedThreadRecoveryThreadId = nil
        selectedThreadHistoryRequestId = nil
        resetSelectedThreadHistoryPagination()
        resetThreadListPagination()
        sceneRefreshTask?.cancel()
        sceneRefreshTask = nil
        gatewayReconnectTask?.cancel()
        gatewayReconnectTask = nil
        gatewayReconnectGeneration = nil
        cancelSelectedThreadReconcileLoop()
        selectedThreadActivitySignatures = [:]
        cancelGlobalEventStream()
        cancelActiveSocket()
        isSending = false
        remoteBusyThreadIds = []
        connectionState = .disconnected
        threads = []
        pinnedThreadIds = []
        recentThreadIds = []
        selectedThread = nil
        messages = []
        messagesByThread = [:]
        messageSignaturesByThread = [:]
        activeAssistantMessageIdsByThread = [:]
        pendingAssistantDeltasByThread = [:]
        assistantDeltaFlushTasksByThread.values.forEach { $0.cancel() }
        assistantDeltaFlushTasksByThread = [:]
        resetComposerDraft()
        draftThreadTitle = ""
        agents = []
        teams = []
        skills = []
        tasks = []
        automations = []
        slashCommands = []
        mcpServers = []
        autoResearchRuns = []
        channelEndpoints = []
        configuredBots = []
        botConsoles = []
        botStatusesById = [:]
        channelPlugins = []
        providerModelsByType = [:]
        selectedWorkspacePath = ""
        selectedWorkspaceDirectory = ""
        draftWorkspacePath = ""
        clearPendingBotDraft()
        workspaceListing = nil
        workspacePreview = nil
        workspaceGitStatuses = [:]
        isUploadingWorkspaceFiles = false
        workspaceUploadStatus = nil
        selectedSkillEditor = nil
        selectedSkillDocument = nil
        selectedSkillFileContent = ""
        researchCandidatesByRunId = [:]
        autoResearchDetailsByRunId = [:]
        autoResearchIterationsByRunId = [:]
        isLoadingThreads = false
        isLoadingRemoteState = false
        isLoadingSelectedThreadHistory = false
    }

    func selectGatewayProfile(_ profile: GaryxGatewayProfile) {
        saveGatewayScopedUserState()
        resetGatewayRuntimeState()
        gatewayURL = profile.gatewayUrl
        gatewayAuthToken = keychain.readGatewayProfileToken(profileId: profile.id)
        loadGatewayScopedUserState(fallbackToLegacy: false)
        gatewaySettingsStatus = "Selected \(profile.label)"
        lastError = nil
    }

    func activateGatewayProfile(_ profile: GaryxGatewayProfile) async {
        selectGatewayProfile(profile)
        await connectAndRefresh()
    }

    func gatewayProfileToken(_ profile: GaryxGatewayProfile) -> String {
        keychain.readGatewayProfileToken(profileId: profile.id)
    }

    @discardableResult
    func updateGatewayProfile(
        _ profile: GaryxGatewayProfile,
        label: String,
        gatewayUrl: String,
        token: String
    ) -> Bool {
        let normalizedURL = normalizedGatewayURL(gatewayUrl)
        guard parsedGatewayURL(from: normalizedURL) != nil else {
            lastError = "Invalid gateway URL"
            return false
        }
        let trimmedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextId = Self.stableGatewayProfileId(for: normalizedURL)
        let currentURL = normalizedGatewayURL(gatewayURL)
        let currentProfileId = currentGatewayProfile?.id
        let affectsCurrentProfile = currentProfileId == profile.id
            || currentProfileId == nextId
            || currentURL.lowercased() == normalizedURL.lowercased()
        let currentURLChanged = currentURL.lowercased() != normalizedURL.lowercased()
        let activeTokenChanged = gatewayAuthToken != trimmedToken
        var nextProfile = profile
        nextProfile.id = nextId
        nextProfile.label = trimmedLabel.isEmpty ? Self.gatewayProfileLabel(for: normalizedURL) : trimmedLabel
        nextProfile.gatewayUrl = normalizedURL
        nextProfile.updatedAt = Date()
        nextProfile.hasToken = !trimmedToken.isEmpty

        gatewayProfiles.removeAll { candidate in
            candidate.id == profile.id
                || candidate.gatewayUrl.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                    == normalizedURL.lowercased()
        }
        gatewayProfiles = Self.normalizedGatewayProfiles([nextProfile] + gatewayProfiles)
        persistGatewayProfiles()
        if profile.id != nextId {
            keychain.deleteGatewayProfileToken(profileId: profile.id)
        }
        keychain.saveGatewayProfileToken(trimmedToken, profileId: nextId)

        if affectsCurrentProfile {
            saveGatewayScopedUserState()
            if currentURLChanged || activeTokenChanged {
                resetGatewayRuntimeState()
            }
            gatewayURL = normalizedURL
            gatewayAuthToken = trimmedToken
            defaults.set(gatewayURL, forKey: GaryxMobileSettingsKeys.gatewayUrl)
            defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayURL)
            defaults.removeObject(forKey: GaryxMobileSettingsKeys.legacyGatewayToken)
            keychain.saveGatewayAuthToken(gatewayAuthToken)
            if currentURLChanged {
                loadGatewayScopedUserState(fallbackToLegacy: false)
            }
        }
        gatewaySettingsStatus = "Updated \(nextProfile.label)"
        lastError = nil
        return true
    }

    func removeGatewayProfile(_ profile: GaryxGatewayProfile) {
        gatewayProfiles.removeAll { $0.id == profile.id }
        persistGatewayProfiles()
        keychain.deleteGatewayProfileToken(profileId: profile.id)
        if currentGatewayProfile?.id == profile.id {
            gatewaySettingsStatus = nil
        }
    }

    private func clearPendingBotDraft() {
        pendingBotId = nil
        pendingBotWorkspace = nil
        pendingBotAgentId = nil
    }

    func handleScenePhase(_ phase: ScenePhase) {
        switch phase {
        case .active:
            sceneRefreshTask?.cancel()
            let selectedThreadId = selectedThread?.id
            sceneRefreshTask = Task { [weak self] in
                guard let self else { return }
                switch connectionState {
                case .ready:
                    startGlobalEventStream()
                    startSelectedThreadReconcileLoop()
                    await refreshThreads()
                    guard !Task.isCancelled else { return }
                    if let selectedThreadId, selectedThread?.id == selectedThreadId {
                        await loadSelectedThreadHistory()
                    }
                case .checking:
                    break
                case .disconnected, .failed:
                    startGatewayReconnectLoop(immediate: true)
                }
            }
        case .background:
            sceneRefreshTask?.cancel()
            sceneRefreshTask = nil
            gatewayReconnectTask?.cancel()
            gatewayReconnectTask = nil
            gatewayReconnectGeneration = nil
            cancelSelectedThreadReconcileLoop()
            cancelGlobalEventStream()
            let runningThreadIds = Array(activeTasksByThread.keys)
            if !runningThreadIds.isEmpty {
                for threadId in runningThreadIds {
                    let activeAssistantMessageId = suspendStreamingAssistantForBackground(threadId: threadId)
                    remoteBusyThreadIds.insert(threadId)
                    cancelActiveSocket(for: threadId)
                    if let activeAssistantMessageId,
                       cachedMessages(for: threadId).contains(where: { $0.id == activeAssistantMessageId }) {
                        activeAssistantMessageIdsByThread[threadId] = activeAssistantMessageId
                    }
                }
                isSending = false
            }
        default:
            break
        }
    }

    private func rememberCurrentGatewayProfile() {
        let url = normalizedGatewayURL(gatewayURL)
        guard !url.isEmpty else { return }
        let profile = GaryxGatewayProfile(
            id: Self.stableGatewayProfileId(for: url),
            label: Self.gatewayProfileLabel(for: url),
            gatewayUrl: url,
            updatedAt: Date(),
            hasToken: !gatewayAuthToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        )
        gatewayProfiles = Self.normalizedGatewayProfiles([profile] + gatewayProfiles)
        persistGatewayProfiles()
        keychain.saveGatewayProfileToken(gatewayAuthToken, profileId: profile.id)
    }

    private func persistGatewayProfiles() {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        if let data = try? encoder.encode(gatewayProfiles) {
            defaults.set(data, forKey: GaryxMobileSettingsKeys.gatewayProfiles)
        }
    }

    func applyMobileConnectLink(_ url: URL) async {
        guard let payload = GaryxMobileConnectLink.parse(url) else {
            return
        }
        saveGatewayScopedUserState()
        resetGatewayRuntimeState()
        gatewayURL = payload.gatewayUrl
        gatewayAuthToken = payload.gatewayAuthToken
        loadGatewayScopedUserState(fallbackToLegacy: false)
        await connectAndRefresh()
    }

    func connectAndRefresh() async {
        gatewayReconnectTask?.cancel()
        gatewayReconnectTask = nil
        gatewayReconnectGeneration = nil
        await connectAndRefresh(scheduleReconnectOnFailure: true)
    }

    private func connectAndRefresh(scheduleReconnectOnFailure: Bool) async {
        gatewayURL = normalizedGatewayURL(gatewayURL)
        gatewayAuthToken = gatewayAuthToken.trimmingCharacters(in: .whitespacesAndNewlines)
        connectionState = .checking
        lastError = nil
        gatewaySettingsStatus = nil
        do {
            let status = try await client().status()
            _ = try await client().chatHealth()
            saveGatewaySettings()
            rememberCurrentGatewayProfile()
            gatewaySettingsStatus = "Saved and connected"
            connectionState = .ready(version: status.version)
            startGlobalEventStream()
            await refreshThreads()
            await refreshRemoteState()
            startSelectedThreadReconcileLoop()
        } catch {
            cancelGlobalEventStream()
            cancelSelectedThreadReconcileLoop()
            let message = displayMessage(for: error)
            connectionState = .failed(message)
            if scheduleReconnectOnFailure {
                lastError = message
            } else {
                gatewaySettingsStatus = "Reconnecting gateway"
            }
            if scheduleReconnectOnFailure {
                startGatewayReconnectLoop()
            }
        }
    }

    func refreshRemoteState() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        isLoadingRemoteState = true
        defer {
            if runtimeGeneration == gatewayRuntimeGeneration {
                isLoadingRemoteState = false
            }
        }
        do {
            let gateway = try client()
            async let agentsResult = gateway.listAgents()
            async let teamsResult = gateway.listTeams()
            async let skillsResult = gateway.listSkills()
            async let tasksResult = gateway.listTasks(includeDone: true, limit: 120)
            async let dreamsResult = gateway.listDreams(sinceHours: 24, limit: 80)
            async let gatewaySettingsResult = gateway.gatewaySettings()
            async let automationsResult = gateway.listAutomations()
            async let slashCommandsResult = gateway.listSlashCommands()
            async let mcpServersResult = gateway.listMcpServers()
            async let autoResearchRunsResult = gateway.listAutoResearchRuns()
            async let channelEndpointsResult = gateway.listChannelEndpoints()
            async let configuredBotsResult = gateway.listConfiguredBots()
            async let botConsolesResult = gateway.listBotConsoles()
            async let channelPluginsResult = gateway.listChannelPlugins()

            let nextAgents = try? await agentsResult
            let nextTeams = try? await teamsResult
            let nextSkills = try? await skillsResult
            let nextTasksPage = try? await tasksResult
            let nextDreamsPage = try? await dreamsResult
            let nextGatewaySettings = try? await gatewaySettingsResult
            let nextAutomations = try? await automationsResult
            let nextSlashCommands = try? await slashCommandsResult
            let nextMcpServers = try? await mcpServersResult
            let nextAutoResearchRuns = try? await autoResearchRunsResult
            let nextChannelEndpoints = try? await channelEndpointsResult
            let nextConfiguredBots = try? await configuredBotsResult
            let nextBotConsoles = try? await botConsolesResult
            let nextChannelPlugins = try? await channelPluginsResult
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }

            agents = nextAgents ?? agents
            teams = nextTeams ?? teams
            skills = nextSkills ?? skills
            if let page = nextTasksPage {
                tasks = page.tasks
            }
            if let page = nextDreamsPage {
                dreams = page.dreams
                latestDreamScan = page.scan ?? page.latestScan
            }
            if let settings = nextGatewaySettings {
                applyGatewayRuntimeSettings(settings)
            }
            automations = nextAutomations ?? automations
            slashCommands = nextSlashCommands ?? slashCommands
            mcpServers = nextMcpServers ?? mcpServers
            autoResearchRuns = nextAutoResearchRuns ?? autoResearchRuns
            channelEndpoints = nextChannelEndpoints ?? channelEndpoints
            configuredBots = nextConfiguredBots ?? configuredBots
            botConsoles = nextBotConsoles ?? botConsoles
            channelPlugins = nextChannelPlugins ?? channelPlugins
            await mergeMissingSidebarRequiredThreads(
                using: gateway,
                extraThreadIds: [selectedThread?.id],
                runtimeGeneration: runtimeGeneration
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            ensureSelectedAgentTarget()
            ensureSelectedWorkspace()
            await refreshProviderModelsForVisibleAgents(runtimeGeneration: runtimeGeneration)
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    private func applyGatewayRuntimeSettings(_ settings: [String: GaryxJSONValue]) {
        dreamsAutoScanEnabled = settings
            .objectValue(forKeys: ["dreams"])?
            .boolValue(forKeys: ["enabled"]) ?? false
        if !dreamsAutoScanEnabled {
            dreams = []
            latestDreamScan = nil
            if activePanel == .dreams {
                activePanel = .chat
            }
        }
    }

    func refreshThreads() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        isLoadingThreads = true
        defer {
            if runtimeGeneration == gatewayRuntimeGeneration {
                isLoadingThreads = false
            }
        }
        do {
            let previousSelectedId = selectedThread?.id
            let gatewayClient = try client()
            async let threadsPage = gatewayClient.listRecentThreads(limit: Self.threadListPageLimit)
            async let threadPinsPage = gatewayClient.listThreadPins()
            let (page, pinsPage) = try await (threadsPage, threadPinsPage)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            applyPinnedThreadIds(pinsPage.threadIds)
            updateThreadListPagination(from: page)
            recentThreadIds = page.threads.map(\.id)
            var nextThreads = page.threads
            let requiredThreadIds = normalizedThreadIds(pinsPage.threadIds + [previousSelectedId])
            nextThreads += await fetchMissingThreadSummaries(
                using: gatewayClient,
                requiredThreadIds: requiredThreadIds,
                existingThreadIds: Set(nextThreads.map(\.id))
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            threads = Self.mergedThreadSummaries(nextThreads)
            refreshRemoteBusyIdsForVisibleThreads()
            if let previousSelectedId,
               let updatedSelection = threads.first(where: { $0.id == previousSelectedId }) {
                selectedThread = updatedSelection
                draftThreadTitle = updatedSelection.title
            } else if previousSelectedId != nil {
                selectedThread = nil
                draftThreadTitle = ""
                resetComposerDraft()
                messages = []
                cancelSelectedThreadReconcileLoop()
                resetSelectedThreadHistoryPagination()
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    @discardableResult
    private func applyThreadTitleUpdate(threadId: String, title: String) -> Bool {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextTitle = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty, !nextTitle.isEmpty else { return false }

        var changed = false
        threads = threads.map { thread in
            guard thread.id == normalizedThreadId, thread.title != nextTitle else {
                return thread
            }
            var updated = thread
            updated.title = nextTitle
            changed = true
            return updated
        }

        if selectedThread?.id == normalizedThreadId,
           selectedThread?.title != nextTitle {
            selectedThread?.title = nextTitle
            draftThreadTitle = nextTitle
            changed = true
        }

        return changed
    }

    func loadMoreThreads() async {
        guard hasGatewaySettings,
              hasMoreThreadSummaries,
              !isLoadingThreads,
              !isLoadingMoreThreads else {
            return
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        let offset = nextThreadListOffset
        guard offset > 0 else { return }
        isLoadingMoreThreads = true
        defer {
            if runtimeGeneration == gatewayRuntimeGeneration {
                isLoadingMoreThreads = false
            }
        }
        do {
            let page = try await client().listRecentThreads(limit: Self.threadListPageLimit, offset: offset)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            updateThreadListPagination(from: page)
            var seenRecentIds = Set(recentThreadIds)
            recentThreadIds += page.threads.compactMap { thread in
                seenRecentIds.insert(thread.id).inserted ? thread.id : nil
            }
            threads = Self.mergedThreadSummaries(threads + page.threads)
            refreshRemoteBusyIdsForVisibleThreads()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    private func updateThreadListPagination(from page: GaryxThreadsPage) {
        let returnedEnd = page.offset + page.count
        nextThreadListOffset = returnedEnd
        hasMoreThreadSummaries = returnedEnd < page.total
    }

    private func updateThreadListPagination(from page: GaryxRecentThreadsPage) {
        nextThreadListOffset = page.offset + page.count
        hasMoreThreadSummaries = page.hasMore
    }

    func refreshWorkspaceAndBotThreads() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let gatewayClient = try client()
            var offset = 0
            var allThreads: [GaryxThreadSummary] = []
            while true {
                let page = try await gatewayClient.listThreads(limit: 1000, offset: offset)
                allThreads += page.threads
                let nextOffset = page.offset + page.count
                if nextOffset >= page.total || page.count == 0 {
                    break
                }
                offset = nextOffset
            }
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            threads = Self.mergedThreadSummaries(threads + allThreads)
            await mergeMissingSidebarRequiredThreads(
                using: gatewayClient,
                extraThreadIds: [selectedThread?.id],
                runtimeGeneration: runtimeGeneration
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            refreshRemoteBusyIdsForVisibleThreads()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    private func refreshRemoteBusyIdsForVisibleThreads() {
        var refreshedBusyIds = remoteBusyThreadIds
        for thread in threads {
            if !(thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true) {
                if activeTasksByThread[thread.id] == nil {
                    refreshedBusyIds.insert(thread.id)
                } else {
                    refreshedBusyIds.remove(thread.id)
                }
            } else if activeTasksByThread[thread.id] == nil {
                refreshedBusyIds.remove(thread.id)
            }
        }
        remoteBusyThreadIds = refreshedBusyIds
    }

    func selectThread(_ thread: GaryxThreadSummary) async {
        let previousThreadId = selectedThread?.id
        if previousThreadId != thread.id {
            resetComposerDraft()
            selectedThreadRecoveryTask?.cancel()
            selectedThreadRecoveryTask = nil
            selectedThreadRecoveryThreadId = nil
            cancelSelectedThreadReconcileLoop()
            resetSelectedThreadHistoryPagination()
        }
        selectedThread = thread
        clearPendingBotDraft()
        draftThreadTitle = thread.title
        activePanel = .chat
        setSidebarVisible(false)
        if previousThreadId != thread.id {
            messages = cachedMessages(for: thread.id)
        }
        await loadSelectedThreadHistory()
        startSelectedThreadReconcileLoop()
    }

    func openNewThreadDraft() {
        selectedThreadRecoveryTask?.cancel()
        selectedThreadRecoveryTask = nil
        selectedThreadRecoveryThreadId = nil
        cancelSelectedThreadReconcileLoop()
        selectedThreadHistoryRequestId = nil
        isLoadingSelectedThreadHistory = false
        resetSelectedThreadHistoryPagination()
        clearPendingBotDraft()
        selectedThread = nil
        draftThreadTitle = ""
        resetComposerDraft()
        messages = []
        activePanel = .chat
        setSidebarVisible(false)
        lastError = nil
    }

    func createThread() async {
        clearPendingBotDraft()
        await createThread(workspaceOverride: nil)
    }

    func createThreadFromCurrentDraft() async {
        guard pendingBotId != nil else {
            await createThread()
            return
        }
        do {
            saveGatewaySettings()
            let existingThreadId = selectedThread?.id
            let thread = try await ensureSelectedThread()
            activePanel = .chat
            draftThreadTitle = thread.title
            if existingThreadId == nil {
                clearMessages(for: thread.id)
            }
            setSidebarVisible(false)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    private func createThread(workspaceOverride: String?, agentOverride: String? = nil) async {
        do {
            saveGatewaySettings()
            let workspace = (workspaceOverride ?? newThreadWorkspace).trimmingCharacters(in: .whitespacesAndNewlines)
            let agentId = (agentOverride ?? selectedAgentTargetId).trimmingCharacters(in: .whitespacesAndNewlines)
            let workspaceMode = workspaceModeForNewThread(workspace: workspace)
            let thread = try await client().createThread(
                GaryxCreateThreadRequest(
                    workspaceDir: workspace.isEmpty ? nil : workspace,
                    workspaceMode: workspaceMode,
                    agentId: agentId.isEmpty ? nil : agentId,
                    metadata: ["client": "garyx-mobile"]
                )
            )
            threads.insert(thread, at: 0)
            selectedThread = thread
            clearPendingBotDraft()
            resetComposerDraft()
            draftThreadTitle = thread.title
            activePanel = .chat
            clearMessages(for: thread.id)
            setSidebarVisible(false)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    private func workspaceModeForNewThread(workspace: String) -> String {
        let trimmedWorkspace = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedWorkspace.isEmpty else { return "local" }
        guard Self.normalizedWorkspaceMode(newThreadWorkspaceMode) == "worktree" else { return "local" }
        if let status = workspaceGitStatuses[trimmedWorkspace], !status.canUseWorktree {
            return "local"
        }
        return "worktree"
    }

    func createThread(inWorkspace workspacePath: String) async {
        clearPendingBotDraft()
        await createThread(workspaceOverride: workspacePath)
    }

    func openBotGroup(_ group: GaryxMobileBotGroup) async {
        let openThreadId = group.mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
            ?? group.defaultOpenThreadId?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
        if let openThreadId {
            await openThread(id: openThreadId)
            return
        }

        let workspace = group.workspaceDir?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let agentId = group.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        pendingBotId = Self.botSelectorId(channel: group.channel, accountId: group.accountId)
        pendingBotWorkspace = workspace.isEmpty ? nil : workspace
        pendingBotAgentId = agentId.isEmpty ? nil : agentId
        cancelSelectedThreadReconcileLoop()
        selectedThread = nil
        resetSelectedThreadHistoryPagination()
        draftThreadTitle = ""
        resetComposerDraft()
        messages = []
        activePanel = .chat
        setSidebarVisible(false)
        lastError = nil
    }

    func deleteSelectedThread() async {
        guard let selectedThread else { return }
        await deleteThread(selectedThread)
    }

    func deleteThread(_ thread: GaryxThreadSummary) async {
        guard canDeleteThread(thread) else {
            lastError = "This thread is active or managed by an automation or channel."
            return
        }
        do {
            _ = try await client().deleteThread(threadId: thread.id)
            removePinnedThreadIdLocally(thread.id)
            if selectedThread?.id == thread.id {
                self.selectedThread = nil
                draftThreadTitle = ""
                resetComposerDraft()
                messages = []
                cancelSelectedThreadReconcileLoop()
                resetSelectedThreadHistoryPagination()
            }
            discardPendingAssistantDelta(for: thread.id)
            messagesByThread[thread.id] = nil
            messageSignaturesByThread[thread.id] = nil
            activeAssistantMessageIdsByThread[thread.id] = nil
            await refreshThreads()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func renameSelectedThread(to proposedTitle: String? = nil) async {
        guard let selectedThread else { return }
        let title = (proposedTitle ?? draftThreadTitle).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, title != selectedThread.title else { return }
        do {
            let updated = try await client().updateThread(threadId: selectedThread.id, label: title)
            self.selectedThread = updated
            draftThreadTitle = updated.title
            if let index = threads.firstIndex(where: { $0.id == updated.id }) {
                threads[index] = updated
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func loadSelectedThreadHistory() async {
        guard let selectedThread else {
            messages = []
            selectedThreadHasMoreHistoryBefore = false
            selectedThreadNextHistoryBeforeIndex = nil
            isLoadingOlderThreadHistory = false
            return
        }
        let threadId = selectedThread.id
        let requestId = UUID()
        selectedThreadHistoryRequestId = requestId
        isLoadingSelectedThreadHistory = true
        defer {
            if selectedThreadHistoryRequestId == requestId {
                isLoadingSelectedThreadHistory = false
            }
        }
        do {
            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard self.selectedThread?.id == threadId, selectedThreadHistoryRequestId == requestId else { return }
            selectedThreadActivitySignatures[threadId] = GaryxThreadActivitySignature.make(from: transcript)
            updateThreadRuntimeState(threadId: threadId, transcript: transcript)
            updateSelectedThreadHistoryPagination(
                threadId: threadId,
                transcript: transcript,
                preservingLoadedOlderPages: true
            )
            let remoteMessages = mobileMessages(from: transcript, threadId: threadId, live: remoteBusyThreadIds.contains(threadId))
            setMessages(
                mergedMessages(
                    remoteMessages,
                    withLocal: cachedMessages(for: threadId),
                    preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript)
                ),
                for: threadId,
                reconcileActiveAssistant: true
            )
            scheduleSelectedThreadRecoveryIfNeeded(threadId: threadId)
            startSelectedThreadReconcileLoop()
        } catch {
            guard self.selectedThread?.id == threadId, selectedThreadHistoryRequestId == requestId else { return }
            if cachedMessages(for: threadId).isEmpty {
                messages = []
            }
            lastError = displayMessage(for: error)
        }
    }

    func loadOlderSelectedThreadHistory() async {
        guard let selectedThread,
              selectedThreadHasMoreHistoryBefore,
              !isLoadingOlderThreadHistory,
              let beforeIndex = selectedThreadNextHistoryBeforeIndex else {
            return
        }
        let threadId = selectedThread.id
        isLoadingOlderThreadHistory = true
        defer {
            if self.selectedThread?.id == threadId {
                isLoadingOlderThreadHistory = false
            }
        }
        do {
            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                beforeIndex: beforeIndex,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard self.selectedThread?.id == threadId else { return }
            updateSelectedThreadHistoryPagination(threadId: threadId, transcript: transcript)
            prependOlderMessages(
                mobileMessages(from: transcript.messages, live: false),
                for: threadId
            )
        } catch {
            guard self.selectedThread?.id == threadId else { return }
            lastError = displayMessage(for: error)
        }
    }

    private func updateThreadRuntimeState(threadId: String, transcript: GaryxThreadTranscript) {
        let hasActiveRun = transcript.threadRuntime?.activeRun != nil
        let hasActivePendingInput = transcript.pendingUserInputs.contains { input in
            input.active && (input.status ?? "awaiting_ack").lowercased() != "abandoned"
        }
        if hasActiveRun || hasActivePendingInput {
            if activeTasksByThread[threadId] == nil {
                remoteBusyThreadIds.insert(threadId)
            } else {
                remoteBusyThreadIds.remove(threadId)
            }
        } else if activeTasksByThread[threadId] == nil {
            remoteBusyThreadIds.remove(threadId)
        }
    }

    private func updateSelectedThreadHistoryPagination(
        threadId: String,
        transcript: GaryxThreadTranscript,
        preservingLoadedOlderPages: Bool = false
    ) {
        guard selectedThread?.id == threadId else { return }
        if preservingLoadedOlderPages,
           let oldestLoadedIndex = oldestLoadedHistoryIndex(for: threadId),
           let latestPageStartIndex = preserveRemoteBeforeIndex(from: transcript),
           oldestLoadedIndex < latestPageStartIndex {
            if oldestLoadedIndex > 0 {
                selectedThreadHasMoreHistoryBefore = true
                selectedThreadNextHistoryBeforeIndex = oldestLoadedIndex
            } else {
                selectedThreadHasMoreHistoryBefore = false
                selectedThreadNextHistoryBeforeIndex = nil
            }
            return
        }
        selectedThreadHasMoreHistoryBefore = transcript.pageInfo?.hasMoreBefore ?? false
        selectedThreadNextHistoryBeforeIndex = transcript.pageInfo?.nextBeforeIndex
    }

    private func oldestLoadedHistoryIndex(for threadId: String) -> Int? {
        cachedMessages(for: threadId)
            .compactMap { Self.historyIndex(fromMessageId: $0.id) }
            .min()
    }

    private func prependOlderMessages(_ olderMessages: [GaryxMobileMessage], for threadId: String) {
        guard !olderMessages.isEmpty else { return }
        let existingMessages = cachedMessages(for: threadId)
        let existingIds = Set(existingMessages.map(\.id))
        let dedupedOlderMessages = olderMessages.filter { !existingIds.contains($0.id) }
        guard !dedupedOlderMessages.isEmpty else { return }
        setMessages(dedupedOlderMessages + existingMessages, for: threadId)
    }

    private func scheduleSelectedThreadRecoveryIfNeeded(threadId: String) {
        guard selectedThread?.id == threadId,
              remoteBusyThreadIds.contains(threadId),
              activeTasksByThread[threadId] == nil,
              selectedThreadRecoveryTask == nil else {
            return
        }
        selectedThreadRecoveryThreadId = threadId
        selectedThreadRecoveryTask = Task { [weak self] in
            var delay: UInt64 = 1_200_000_000
            for _ in 0..<8 {
                try? await Task.sleep(nanoseconds: delay)
                guard !Task.isCancelled else { break }
                await self?.refreshSelectedThreadRuntimeSnapshot(threadId: threadId)
                let shouldContinue = self?.shouldContinueRecoveringSelectedThread(threadId: threadId) ?? false
                if !shouldContinue {
                    break
                }
                delay = min(delay * 2, 5_000_000_000)
            }
            self?.clearSelectedThreadRecoveryTask(threadId: threadId)
        }
    }

    private func shouldContinueRecoveringSelectedThread(threadId: String) -> Bool {
        selectedThread?.id == threadId
            && remoteBusyThreadIds.contains(threadId)
            && activeTasksByThread[threadId] == nil
    }

    private func clearSelectedThreadRecoveryTask(threadId: String) {
        if selectedThreadRecoveryThreadId == threadId {
            selectedThreadRecoveryTask = nil
            selectedThreadRecoveryThreadId = nil
        }
    }

    private func refreshSelectedThreadRuntimeSnapshot(threadId: String) async {
        guard selectedThread?.id == threadId else { return }
        let observedHistoryRequestId = selectedThreadHistoryRequestId
        do {
            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            selectedThreadActivitySignatures[threadId] = GaryxThreadActivitySignature.make(from: transcript)
            updateThreadRuntimeState(threadId: threadId, transcript: transcript)
            updateSelectedThreadHistoryPagination(
                threadId: threadId,
                transcript: transcript,
                preservingLoadedOlderPages: true
            )
            let remoteMessages = mobileMessages(from: transcript, threadId: threadId, live: remoteBusyThreadIds.contains(threadId))
            setMessages(
                mergedMessages(
                    remoteMessages,
                    withLocal: cachedMessages(for: threadId),
                    preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript)
                ),
                for: threadId,
                reconcileActiveAssistant: true
            )
            if !remoteBusyThreadIds.contains(threadId) {
                await refreshThreads()
            }
        } catch {
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            let message = displayMessage(for: error)
            if Self.isTransientGatewayErrorMessage(message) {
                gatewaySettingsStatus = "Waiting to sync with gateway"
            } else {
                lastError = message
            }
        }
    }

    private func startSelectedThreadReconcileLoop() {
        guard hasGatewaySettings,
              case .ready = connectionState,
              let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            cancelSelectedThreadReconcileLoop()
            return
        }
        if selectedThreadReconcileThreadId == threadId, selectedThreadReconcileTask != nil {
            return
        }
        cancelSelectedThreadReconcileLoop()
        selectedThreadReconcileThreadId = threadId
        selectedThreadReconcileTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: Self.selectedThreadReconcileIntervalNanos)
                if Task.isCancelled { break }
                await reconcileSelectedThreadFromGatewayIfChanged(threadId: threadId)
            }
        }
    }

    private func cancelSelectedThreadReconcileLoop() {
        selectedThreadReconcileTask?.cancel()
        selectedThreadReconcileTask = nil
        selectedThreadReconcileThreadId = nil
    }

    private func reconcileSelectedThreadFromGatewayIfChanged(threadId: String) async {
        guard selectedThread?.id == threadId,
              hasGatewaySettings,
              case .ready = connectionState,
              !isLoadingSelectedThreadHistory else {
            return
        }
        if activeTasksByThread[threadId] != nil {
            return
        }
        let observedHistoryRequestId = selectedThreadHistoryRequestId
        do {
            let transcript = try await client().threadHistory(
                threadId: threadId,
                limit: Self.threadHistoryPageLimit,
                userQueryLimit: Self.threadHistoryUserQueryLimit
            )
            guard selectedThread?.id == threadId,
                  selectedThreadHistoryRequestId == observedHistoryRequestId else { return }
            let signature = GaryxThreadActivitySignature.make(from: transcript)
            if selectedThreadActivitySignatures[threadId] == signature {
                updateThreadRuntimeState(threadId: threadId, transcript: transcript)
                return
            }
            selectedThreadActivitySignatures[threadId] = signature
            updateThreadRuntimeState(threadId: threadId, transcript: transcript)
            updateSelectedThreadHistoryPagination(
                threadId: threadId,
                transcript: transcript,
                preservingLoadedOlderPages: true
            )
            let remoteMessages = mobileMessages(from: transcript, threadId: threadId, live: remoteBusyThreadIds.contains(threadId))
            setMessages(
                mergedMessages(
                    remoteMessages,
                    withLocal: cachedMessages(for: threadId),
                    preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript)
                ),
                for: threadId,
                reconcileActiveAssistant: true
            )
            if !remoteBusyThreadIds.contains(threadId) {
                await refreshThreads()
            }
        } catch {
            guard selectedThread?.id == threadId else { return }
            let message = displayMessage(for: error)
            if Self.isTransientGatewayErrorMessage(message) {
                gatewaySettingsStatus = "Waiting to sync with gateway"
            } else {
                lastError = message
            }
        }
    }

    private func transcript(fromSnapshotPayload payload: [String: GaryxJSONValue]) throws -> GaryxThreadTranscript? {
        guard case let .object(snapshot)? = payload["payload"] else {
            return nil
        }
        let data = try JSONEncoder().encode(GaryxJSONValue.object(snapshot))
        return try JSONDecoder().decode(GaryxThreadTranscript.self, from: data)
    }

    func attachFiles(from urls: [URL]) async {
        guard !urls.isEmpty else { return }
        do {
            let localFiles = try urls.map { url in
                let didAccess = url.startAccessingSecurityScopedResource()
                defer {
                    if didAccess {
                        url.stopAccessingSecurityScopedResource()
                    }
                }
                let data = try Data(contentsOf: url)
                let resourceValues = try? url.resourceValues(forKeys: [.contentTypeKey])
                let mediaType = resourceValues?.contentType?.preferredMIMEType
                    ?? UTType(filenameExtension: url.pathExtension)?.preferredMIMEType
                    ?? "application/octet-stream"
                let kind = mediaType.hasPrefix("image/") ? "image" : "file"
                let encoded = data.base64EncodedString()
                let name = url.lastPathComponent.isEmpty ? "attachment" : url.lastPathComponent
                return (
                    blob: GaryxUploadChatAttachmentBlob(
                        kind: kind,
                        name: name,
                        mediaType: mediaType,
                        dataBase64: encoded
                    ),
                    preview: GaryxPendingUploadPreview(
                        name: name,
                        mediaType: mediaType,
                        previewDataUrl: kind == "image" ? Self.dataUrl(mediaType: mediaType, base64: encoded) : nil
                    )
                )
            }
            let uploaded = try await client().uploadChatAttachments(
                GaryxUploadChatAttachmentsRequest(files: localFiles.map(\.blob))
            )
            var previews = localFiles.map(\.preview)
            composerAttachments.append(
                contentsOf: uploaded.files.map { file in
                    let preview = Self.matchedUploadPreview(for: file, from: &previews)
                    return GaryxMobileComposerAttachment(
                        id: "\(file.path)-\(UUID().uuidString)",
                        kind: file.kind,
                        name: file.name,
                        mediaType: file.mediaType,
                        path: file.path,
                        previewDataUrl: preview?.previewDataUrl
                    )
                }
            )
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func attachImages(_ images: [GaryxMobileSelectedImage]) async {
        guard !images.isEmpty else { return }
        for image in images {
            do {
                let encoded = image.data.base64EncodedString()
                let uploaded = try await client().uploadChatAttachments(
                    GaryxUploadChatAttachmentsRequest(
                        files: [
                            GaryxUploadChatAttachmentBlob(
                                kind: "image",
                                name: image.name,
                                mediaType: image.mediaType,
                                dataBase64: encoded
                            ),
                        ]
                    )
                )
                guard let file = uploaded.files.first else {
                    throw GaryxGatewayError.encodingFailed("Gateway did not return an uploaded image.")
                }
                let fallbackMediaType = image.mediaType.isEmpty ? "image/jpeg" : image.mediaType
                composerAttachments.append(
                    GaryxMobileComposerAttachment(
                        id: "\(file.path)-\(UUID().uuidString)",
                        kind: file.kind.isEmpty ? "image" : file.kind,
                        name: file.name,
                        mediaType: file.mediaType.isEmpty ? fallbackMediaType : file.mediaType,
                        path: file.path,
                        previewDataUrl: Self.dataUrl(mediaType: fallbackMediaType, base64: encoded)
                    )
                )
            } catch {
                lastError = displayMessage(for: error)
                return
            }
        }
    }

    func removeComposerAttachment(_ attachment: GaryxMobileComposerAttachment) {
        composerAttachments.removeAll { $0.id == attachment.id }
    }

    @discardableResult
    func sendDraft() async -> Bool {
        await sendDraft(text: draft)
    }

    @discardableResult
    func sendDraft(text rawText: String) async -> Bool {
        let text = rawText.trimmingCharacters(in: .whitespacesAndNewlines)
        let attachments = composerAttachments
        guard canSendComposerPayload(text: text, attachments: attachments) else { return false }
        guard !text.isEmpty || !attachments.isEmpty else { return false }
        resetComposerDraft()
        await send(text, attachments: attachments)
        return true
    }

    func send(_ text: String, attachments: [GaryxMobileComposerAttachment] = []) async {
        if let selectedThread, activeTasksByThread[selectedThread.id] != nil {
            await queueInput(text, attachments: attachments, in: selectedThread)
            return
        }
        if let selectedThread, isThreadBusy(selectedThread.id) {
            await queueRemoteInput(text, attachments: attachments, in: selectedThread)
            return
        }

        let visibleUserText = Self.visibleUserText(text: text, attachments: attachments)
        let clientIntentId = "mobile-\(UUID().uuidString)"
        let userMessage = GaryxMobileMessage(
            id: "local-user-\(UUID().uuidString)",
            role: .user,
            text: visibleUserText,
            attachments: Self.messageAttachments(from: attachments),
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId
        )
        let assistantId = "local-assistant-\(UUID().uuidString)"
        let assistantMessage = GaryxMobileMessage(
            id: assistantId,
            role: .assistant,
            text: "",
            timestamp: nil,
            isStreaming: true
        )
        var optimisticThreadId = selectedThread?.id
        if let optimisticThreadId {
            mutateMessages(for: optimisticThreadId) { messages in
                messages.append(userMessage)
                messages.append(assistantMessage)
            }
            activeAssistantMessageIdsByThread[optimisticThreadId] = assistantId
        } else {
            messages = [userMessage, assistantMessage]
        }

        do {
            let thread = try await ensureSelectedThread()
            if optimisticThreadId == nil {
                optimisticThreadId = thread.id
                setMessages(messages, for: thread.id)
                activeAssistantMessageIdsByThread[thread.id] = assistantId
            }
            if activeTasksByThread[thread.id] != nil {
                await queueInput(text, attachments: attachments, in: thread)
                return
            }
            guard !remoteBusyThreadIds.contains(thread.id) else {
                markLatestLocalUserFailed(for: thread.id, message: "Thread is busy")
                markStreamingAssistantComplete(for: thread.id, removeEmpty: true)
                return
            }
            isSending = true
            activeRunThreadId = thread.id
            remoteBusyThreadIds.remove(thread.id)
            lastError = nil
            activeAssistantMessageIdsByThread[thread.id] = assistantId
            let task = try client().makeWebSocketTask()
            activeTask = task
            activeTasksByThread[thread.id] = task
            task.resume()
            let workspacePath = Self.firstNonEmpty(
                thread.workspacePath,
                newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            let command = try client().encodeWebSocketCommand(
                .start(
                    threadId: thread.id,
                    message: text,
                    fromId: "garyx-mobile",
                    workspacePath: workspacePath,
                    attachments: attachments.map(\.promptAttachment),
                    metadata: [
                        "client": "garyx-mobile",
                        "client_intent_id": clientIntentId,
                        "client_timestamp_local": Self.localChatTimestamp(),
                    ]
                )
            )
            try await task.send(.string(command))
            activeReaderTasksByThread[thread.id]?.cancel()
            let readerTask = Task { [weak self, weak task] in
                guard let self, let task else { return }
                await self.receiveEvents(from: task, threadId: thread.id, assistantMessageId: assistantId)
            }
            activeReaderTasksByThread[thread.id] = readerTask
            activeReaderTask = readerTask
        } catch {
            if let optimisticThreadId {
                markLatestLocalUserFailed(for: optimisticThreadId, message: displayMessage(for: error))
                markStreamingAssistantComplete(for: optimisticThreadId, removeEmpty: true)
            } else {
                messages.removeAll { $0.id == assistantId }
                if let index = messages.firstIndex(where: { $0.id == userMessage.id }) {
                    messages[index].statusText = displayMessage(for: error)
                }
            }
            if let optimisticThreadId {
                cancelActiveSocket(for: optimisticThreadId)
            }
            lastError = displayMessage(for: error)
        }
    }

    private func queueInput(
        _ text: String,
        attachments: [GaryxMobileComposerAttachment],
        in thread: GaryxThreadSummary
    ) async {
        guard let activeTask = activeTasksByThread[thread.id] else {
            await queueRemoteInput(text, attachments: attachments, in: thread)
            return
        }
        let clientIntentId = "mobile-\(UUID().uuidString)"
        let visibleUserText = Self.visibleUserText(text: text, attachments: attachments)
        let userMessage = GaryxMobileMessage(
            id: "local-user-\(UUID().uuidString)",
            role: .user,
            text: visibleUserText,
            attachments: Self.messageAttachments(from: attachments),
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId
        )
        mutateMessages(for: thread.id) { messages in
            messages.append(userMessage)
        }
        let queued = GaryxPendingQueuedInput(
            threadId: thread.id,
            text: text,
            attachments: attachments,
            clientIntentId: clientIntentId
        )
        pendingQueuedInputsByIntentId[clientIntentId] = queued
        do {
            let command = try client().encodeWebSocketCommand(
                .input(
                    threadId: thread.id,
                    message: text,
                    clientIntentId: clientIntentId,
                    attachments: attachments.map(\.promptAttachment)
                )
            )
            try await activeTask.send(.string(command))
        } catch {
            if let claimed = pendingQueuedInputsByIntentId.removeValue(forKey: clientIntentId) {
                cancelActiveSocket(for: thread.id)
                await submitQueuedInputViaGateway(claimed)
            } else {
                markLatestLocalUserFailed(for: thread.id, message: displayMessage(for: error))
                lastError = displayMessage(for: error)
            }
        }
    }

    private func queueRemoteInput(
        _ text: String,
        attachments: [GaryxMobileComposerAttachment],
        in thread: GaryxThreadSummary
    ) async {
        let clientIntentId = "mobile-\(UUID().uuidString)"
        let visibleUserText = Self.visibleUserText(text: text, attachments: attachments)
        let userMessage = GaryxMobileMessage(
            id: "local-user-\(UUID().uuidString)",
            role: .user,
            text: visibleUserText,
            attachments: Self.messageAttachments(from: attachments),
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId
        )
        mutateMessages(for: thread.id) { messages in
            messages.append(userMessage)
        }
        let queued = GaryxPendingQueuedInput(
            threadId: thread.id,
            text: text,
            attachments: attachments,
            clientIntentId: clientIntentId
        )
        pendingQueuedInputsByIntentId[clientIntentId] = queued
        await submitQueuedInputViaGateway(queued)
    }

    private func submitQueuedInputViaGateway(_ queued: GaryxPendingQueuedInput) async {
        pendingQueuedInputsByIntentId[queued.clientIntentId] = queued
        do {
            let result = try await client().streamInput(
                GaryxStreamInputRequest(
                    threadId: queued.threadId,
                    clientIntentId: queued.clientIntentId,
                    message: queued.text,
                    attachments: queued.attachments.map(\.promptAttachment)
                )
            )
            if Self.isSuccessfulStreamInputStatus(result.status) {
                bindLocalPendingInput(
                    threadId: queued.threadId,
                    clientIntentId: result.clientIntentId ?? queued.clientIntentId,
                    pendingInputId: result.pendingInputId
                )
                remoteBusyThreadIds.insert(queued.threadId)
            } else if Self.shouldFallbackStreamInputStatus(result.status) {
                if let claimed = pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId) {
                    await dispatchQueuedInputFallback(claimed)
                }
            } else {
                pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId)
                let failureMessage = result.status.isEmpty ? "Input was not queued" : result.status
                let markedInput = markLocalInputFailed(
                    threadId: queued.threadId,
                    clientIntentId: result.clientIntentId ?? queued.clientIntentId,
                    pendingInputId: result.pendingInputId,
                    message: failureMessage
                )
                if !markedInput {
                    lastError = failureMessage
                }
            }
        } catch {
            if pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId) != nil {
                let message = displayMessage(for: error)
                markLocalInputFailed(
                    threadId: queued.threadId,
                    clientIntentId: queued.clientIntentId,
                    pendingInputId: nil,
                    message: message
                )
                lastError = message
            }
        }
    }

    private func dispatchQueuedInputFallback(_ queued: GaryxPendingQueuedInput) async {
        let fallbackSelectedThread = selectedThread?.id == queued.threadId ? selectedThread : nil
        guard let thread = threads.first(where: { $0.id == queued.threadId }) ?? fallbackSelectedThread else {
            markLocalInputFailed(
                threadId: queued.threadId,
                clientIntentId: queued.clientIntentId,
                pendingInputId: nil,
                message: "Input was not queued"
            )
            return
        }
        if activeTasksByThread[queued.threadId] != nil {
            cancelActiveSocket(for: queued.threadId)
        }
        remoteBusyThreadIds.remove(queued.threadId)
        clearLocalInputStatus(threadId: queued.threadId, clientIntentId: queued.clientIntentId)

        let assistantId = "stream-assistant-\(queued.threadId)-\(UUID().uuidString)"
        mutateMessages(for: queued.threadId) { messages in
            let assistantMessage = GaryxMobileMessage(
                id: assistantId,
                role: .assistant,
                text: "",
                timestamp: nil,
                isStreaming: true
            )
            if let userIndex = messages.indices.last(where: { index in
                messages[index].role == .user && messages[index].clientIntentId == queued.clientIntentId
            }) {
                let insertIndex = messages.index(after: userIndex)
                messages.insert(assistantMessage, at: insertIndex)
            } else {
                messages.append(assistantMessage)
            }
        }

        do {
            isSending = true
            activeRunThreadId = queued.threadId
            activeAssistantMessageIdsByThread[queued.threadId] = assistantId
            let task = try client().makeWebSocketTask()
            activeTask = task
            activeTasksByThread[queued.threadId] = task
            task.resume()
            let workspacePath = Self.firstNonEmpty(
                thread.workspacePath,
                newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            let command = try client().encodeWebSocketCommand(
                .start(
                    threadId: queued.threadId,
                    message: queued.text,
                    fromId: "garyx-mobile",
                    workspacePath: workspacePath,
                    attachments: queued.attachments.map(\.promptAttachment),
                    metadata: [
                        "client": "garyx-mobile",
                        "client_intent_id": queued.clientIntentId,
                        "client_timestamp_local": Self.localChatTimestamp(),
                    ]
                )
            )
            try await task.send(.string(command))
            activeReaderTasksByThread[queued.threadId]?.cancel()
            let readerTask = Task { [weak self, weak task] in
                guard let self, let task else { return }
                await self.receiveEvents(from: task, threadId: queued.threadId, assistantMessageId: assistantId)
            }
            activeReaderTasksByThread[queued.threadId] = readerTask
            activeReaderTask = readerTask
        } catch {
            markLocalInputFailed(
                threadId: queued.threadId,
                clientIntentId: queued.clientIntentId,
                pendingInputId: nil,
                message: displayMessage(for: error)
            )
            markStreamingAssistantComplete(for: queued.threadId, removeEmpty: true)
            cancelActiveSocket(for: queued.threadId)
            lastError = displayMessage(for: error)
        }
    }

    func interruptActiveRun() async {
        guard let threadId = selectedThread?.id ?? activeRunThreadId else { return }
        let hadLocalTask = activeTasksByThread[threadId] != nil
        var sentLocalInterrupt = false
        var sentGatewayInterrupt = false
        if let activeTask = activeTasksByThread[threadId] {
            do {
                let command = try client().encodeWebSocketCommand(.interrupt(threadId: threadId))
                try await activeTask.send(.string(command))
                sentLocalInterrupt = true
            } catch {
                // Continue to the gateway-backed interrupt below; the local socket may be stale.
            }
        }
        do {
            _ = try await client().interruptThread(threadId: threadId)
            sentGatewayInterrupt = true
        } catch {
            if !sentLocalInterrupt {
                lastError = displayMessage(for: error)
            }
        }
        if hadLocalTask {
            cancelActiveSocket(for: threadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        }
        guard sentLocalInterrupt || sentGatewayInterrupt || hadLocalTask else {
            return
        }
        remoteBusyThreadIds.remove(threadId)
        await refreshThreads()
        if selectedThread?.id == threadId {
            await loadSelectedThreadHistory()
        }
    }

    func createTaskFromDraft(
        start: Bool = true,
        notificationTarget: GaryxTaskNotificationTargetRequest = .none
    ) async {
        let title = draftTaskTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        let body = draftTaskBody.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty || !body.isEmpty else { return }
        do {
            saveGatewaySettings()
            let target = selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
            let workspace = newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
            let task = try await client().createTask(
                GaryxTaskCreateRequest(
                    title: title.isEmpty ? nil : title,
                    body: body.isEmpty ? nil : body,
                    assignee: start && !target.isEmpty ? .agent(target) : nil,
                    start: start,
                    runtime: GaryxTaskRuntimeRequest(
                        agentId: start && !target.isEmpty ? target : nil,
                        workspaceDir: workspace.isEmpty ? nil : workspace
                    ),
                    notificationTarget: notificationTarget
                )
            )
            draftTaskTitle = ""
            draftTaskBody = ""
            upsertTask(task)
            if !task.threadId.isEmpty {
                await openThread(id: task.threadId)
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func promoteSelectedThreadToTask() async {
        guard let thread = selectedThread else { return }
        let title = thread.title.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            var task = try await client().promoteTask(
                GaryxTaskPromoteRequest(
                    threadId: thread.id,
                    title: title.isEmpty ? nil : title
                )
            )
            if task.threadId.isEmpty {
                task.threadId = thread.id
            }
            upsertTask(task)
            activePanel = .tasks
        } catch {
            if Self.isAlreadyTaskError(error),
               await reconcileTaskForThread(thread.id) != nil {
                activePanel = .tasks
                return
            }
            lastError = displayMessage(for: error)
        }
    }

    func updateTask(_ task: GaryxTaskSummary, to status: GaryxTaskStatus) async {
        do {
            _ = try await client().updateTaskStatus(
                taskId: task.id,
                request: GaryxTaskUpdateStatusRequest(to: status)
            )
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateTaskTitle(_ task: GaryxTaskSummary, title: String) async {
        let nextTitle = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextTitle.isEmpty else { return }
        do {
            _ = try await client().updateTaskTitle(taskId: task.id, title: nextTitle)
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func assignTask(_ task: GaryxTaskSummary, agentId: String) async {
        let target = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        do {
            _ = try await client().assignTask(taskId: task.id, request: GaryxTaskAssignRequest(to: .agent(target)))
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func unassignTask(_ task: GaryxTaskSummary) async {
        do {
            _ = try await client().unassignTask(taskId: task.id)
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func stopTask(_ task: GaryxTaskSummary) async {
        do {
            _ = try await client().stopTask(taskId: task.id)
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteTask(_ task: GaryxTaskSummary) async {
        do {
            _ = try await client().deleteTask(taskId: task.id)
            tasks.removeAll { $0.id == task.id }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    private func upsertTask(_ task: GaryxTaskSummary) {
        if let index = tasks.firstIndex(where: { $0.id == task.id }) {
            tasks[index] = task
        } else {
            tasks.insert(task, at: 0)
        }
    }

    private func taskSummary(forThreadId threadId: String) -> GaryxTaskSummary? {
        let threadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else { return nil }
        return tasks.first { $0.threadId == threadId }
    }

    private func reconcileTaskForThread(_ threadId: String) async -> GaryxTaskSummary? {
        if let task = taskSummary(forThreadId: threadId) {
            return task
        }
        do {
            let page = try await client().listTasks(includeDone: true, limit: 200)
            tasks = page.tasks
            return taskSummary(forThreadId: threadId)
        } catch {
            lastError = displayMessage(for: error)
            return nil
        }
    }

    private static func isAlreadyTaskError(_ error: Error) -> Bool {
        if case let GaryxGatewayError.httpStatus(_, body) = error {
            return body.contains("\"code\":\"AlreadyATask\"")
                || body.contains("\"code\": \"AlreadyATask\"")
                || body.contains("AlreadyATask")
        }
        return false
    }

    func refreshDreams() async {
        guard hasGatewaySettings, dreamsAutoScanEnabled else {
            dreams = []
            latestDreamScan = nil
            return
        }
        do {
            let page = try await client().listDreams(sinceHours: 24, limit: 80)
            dreams = page.dreams
            latestDreamScan = page.scan ?? page.latestScan
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func scanDreams() async {
        guard hasGatewaySettings, dreamsAutoScanEnabled, !isScanningDreams else { return }
        isScanningDreams = true
        defer { isScanningDreams = false }
        do {
            let page = try await client().scanDreams(
                request: GaryxDreamScanRequest(sinceHours: 24, mode: "auto", limit: 600)
            )
            dreams = page.dreams
            latestDreamScan = page.scan ?? page.latestScan
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func setDreamsAutoScanEnabled(_ enabled: Bool) async {
        guard hasGatewaySettings, dreamsAutoScanEnabled != enabled, !isSavingDreamsSettings else {
            return
        }
        let previous = dreamsAutoScanEnabled
        dreamsAutoScanEnabled = enabled
        isSavingDreamsSettings = true
        defer { isSavingDreamsSettings = false }
        do {
            _ = try await client().saveGatewaySettings([
                "dreams": .object([
                    "enabled": .bool(enabled)
                ])
            ])
            gatewaySettingsStatus = "Saved"
            if !enabled {
                dreams = []
                latestDreamScan = nil
                if activePanel == .dreams {
                    activePanel = .chat
                }
            } else {
                await refreshDreams()
            }
        } catch {
            dreamsAutoScanEnabled = previous
            lastError = displayMessage(for: error)
        }
    }

    func openDreamSpan(_ span: GaryxDreamSpan) async {
        await openThread(id: span.threadId)
    }

    func runAutomation(_ automation: GaryxAutomationSummary) async {
        do {
            let run = try await client().runAutomationNow(id: automation.id)
            lastAutomationRun = run
            await refreshRemoteState()
            if !run.threadId.isEmpty {
                await openThread(id: run.threadId)
            } else if let targetThreadId = automation.targetThreadId, !targetThreadId.isEmpty {
                await openThread(id: targetThreadId)
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func toggleAutomation(_ automation: GaryxAutomationSummary) async {
        do {
            _ = try await client().updateAutomationEnabled(
                id: automation.id,
                enabled: !automation.enabled
            )
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateAutomation(
        _ automation: GaryxAutomationSummary,
        label: String,
        prompt: String,
        intervalHours: String,
        targetsExistingThread: Bool,
        targetThreadId: String,
        workspacePath: String
    ) async {
        let nextLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextPrompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextTargetThreadId = targetsExistingThread
            ? targetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            : ""
        let nextWorkspacePath = workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        let hours = max(1, Int(intervalHours.trimmingCharacters(in: .whitespacesAndNewlines)) ?? automation.schedule.hours ?? 24)
        let nextSchedule: GaryxAutomationSchedule? = automation.schedule.kind == .interval
            ? .interval(hours: hours)
            : nil
        guard !nextLabel.isEmpty, !nextPrompt.isEmpty else { return }
        if targetsExistingThread {
            guard !nextTargetThreadId.isEmpty else { return }
        } else {
            guard !nextWorkspacePath.isEmpty else { return }
        }
        do {
            let updated = try await client().updateAutomation(
                id: automation.id,
                request: GaryxAutomationUpdateRequest(
                    label: nextLabel,
                    prompt: nextPrompt,
                    workspaceDir: nextTargetThreadId.isEmpty ? nextWorkspacePath : nil,
                    targetThreadId: nextTargetThreadId.isEmpty ? nil : nextTargetThreadId,
                    clearsTargetThreadId: nextTargetThreadId.isEmpty,
                    schedule: nextSchedule
                )
            )
            replaceAutomation(updated)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func openThread(id: String) async {
        if let thread = threads.first(where: { $0.id == id }) {
            await selectThread(thread)
            activePanel = .chat
            return
        }
        await refreshThreads()
        if let thread = threads.first(where: { $0.id == id }) {
            await selectThread(thread)
            activePanel = .chat
            return
        }
        do {
            let thread = try await client().getThread(threadId: id)
            threads = Self.mergedThreadSummaries(threads + [thread])
            await selectThread(thread)
            activePanel = .chat
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func setSelectedAgentTarget(_ id: String) {
        selectedAgentTargetId = id
        defaults.set(id, forKey: scopedSettingsKey(GaryxMobileSettingsKeys.selectedAgentTargetId))
    }

    func setNewThreadWorkspace(_ path: String) {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        newThreadWorkspace = trimmed
        if trimmed.isEmpty {
            newThreadWorkspaceMode = "local"
        }
        saveGatewayScopedUserState()
        if !trimmed.isEmpty, workspaceGitStatuses[trimmed] == nil {
            Task { await refreshWorkspaceGitStatus(for: trimmed) }
        }
    }

    func setNewThreadWorkspaceMode(_ mode: String) {
        let normalized = Self.normalizedWorkspaceMode(mode)
        newThreadWorkspaceMode = normalized
        if newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            newThreadWorkspaceMode = "local"
        }
        saveGatewayScopedUserState()
    }

    var newThreadWorkspaceLabel: String {
        let workspace = newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = (workspace as NSString).lastPathComponent
        return workspace.isEmpty ? "No workspace" : (name.isEmpty ? workspace : name)
    }

    var newThreadWorkspaceCanUseWorktree: Bool {
        let workspace = newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty else { return false }
        return workspaceGitStatuses[workspace]?.canUseWorktree ?? true
    }

    var newThreadUsesWorktree: Bool {
        Self.normalizedWorkspaceMode(newThreadWorkspaceMode) == "worktree" && newThreadWorkspaceCanUseWorktree
    }

    func refreshWorkspaceGitStatus(for path: String) async {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            let status = try await client().workspaceGitStatus(workspaceDir: trimmed)
            workspaceGitStatuses[trimmed] = status
            if !status.canUseWorktree, newThreadWorkspace == trimmed {
                setNewThreadWorkspaceMode("local")
            }
        } catch {
            // Workspace status is an affordance for the mode selector; keep chat usable if it fails.
        }
    }

    func createAgentFromDraft() async -> Bool {
        let agentId = draftAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let displayName = draftAgentName.trimmingCharacters(in: .whitespacesAndNewlines)
        let provider = draftAgentProvider.trimmingCharacters(in: .whitespacesAndNewlines)
        let model = draftAgentModel.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = draftAgentWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = draftAgentPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !agentId.isEmpty, !displayName.isEmpty, !provider.isEmpty else { return false }
        do {
            let agent = try await client().createAgent(
                GaryxCustomAgentRequest(
                    agentId: agentId,
                    displayName: displayName,
                    providerType: provider,
                    model: model.isEmpty ? nil : model,
                    defaultWorkspaceDir: workspace.isEmpty ? nil : workspace,
                    systemPrompt: prompt
                )
            )
            draftAgentId = ""
            draftAgentName = ""
            draftAgentModel = ""
            draftAgentWorkspace = ""
            draftAgentPrompt = ""
            replaceAgent(agent)
            setSelectedAgentTarget(agent.id)
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func updateAgent(
        _ agent: GaryxAgentSummary,
        agentId: String,
        displayName: String,
        providerType: String,
        modelName: String,
        workspace: String,
        systemPrompt: String
    ) async {
        let nextAgentId = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextDisplayName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextProviderType = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextModelName = modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextWorkspace = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextAgentId.isEmpty, !nextDisplayName.isEmpty, !nextProviderType.isEmpty else { return }
        do {
            let updated = try await client().updateAgent(
                agentId: agent.id,
                request: GaryxCustomAgentRequest(
                    agentId: nextAgentId,
                    displayName: nextDisplayName,
                    providerType: nextProviderType,
                    model: nextModelName.isEmpty ? nil : nextModelName,
                    modelReasoningEffort: agent.modelReasoningEffort.isEmpty ? nil : agent.modelReasoningEffort,
                    modelServiceTier: agent.modelServiceTier.isEmpty ? nil : agent.modelServiceTier,
                    providerEnv: agent.providerEnv.isEmpty ? nil : agent.providerEnv,
                    authSource: agent.authSource.isEmpty ? nil : agent.authSource,
                    baseUrl: agent.baseUrl.isEmpty ? nil : agent.baseUrl,
                    codexHome: agent.codexHome.isEmpty ? nil : agent.codexHome,
                    maxToolIterations: agent.maxToolIterations,
                    requestTimeoutSeconds: agent.requestTimeoutSeconds,
                    defaultWorkspaceDir: nextWorkspace.isEmpty ? nil : nextWorkspace,
                    avatarDataUrl: agent.avatarDataUrl.isEmpty ? nil : agent.avatarDataUrl,
                    systemPrompt: systemPrompt
                )
            )
            replaceAgent(updated)
            setSelectedAgentTarget(updated.id)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateTeam(
        _ team: GaryxTeamSummary,
        teamId: String,
        displayName: String,
        leaderAgentId: String,
        memberAgentIds: String,
        workflowText: String
    ) async {
        let nextTeamId = teamId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextDisplayName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextLeader = leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextMembers = Self.normalizedTeamMemberIds(memberAgentIds, leaderAgentId: nextLeader)
        let nextWorkflow = workflowText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextTeamId.isEmpty, !nextDisplayName.isEmpty, !nextLeader.isEmpty else { return }
        do {
            let updated = try await client().updateTeam(
                teamId: team.id,
                request: GaryxTeamRequest(
                    teamId: nextTeamId,
                    displayName: nextDisplayName,
                    leaderAgentId: nextLeader,
                    memberAgentIds: nextMembers,
                    workflowText: nextWorkflow,
                    avatarDataUrl: team.avatarDataUrl.isEmpty ? nil : team.avatarDataUrl
                )
            )
            replaceTeam(updated)
            setSelectedAgentTarget(updated.id)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteAgent(_ agent: GaryxAgentSummary) async {
        guard !agent.builtIn else { return }
        do {
            _ = try await client().deleteAgent(agentId: agent.id)
            agents.removeAll { $0.id == agent.id }
            ensureSelectedAgentTarget()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func loadProviderModels(providerType: String, runtimeGeneration: UUID? = nil) async {
        let provider = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !provider.isEmpty else { return }
        let observedGeneration = runtimeGeneration ?? gatewayRuntimeGeneration
        do {
            let models = try await client().providerModels(providerType: provider)
            guard observedGeneration == gatewayRuntimeGeneration else { return }
            providerModelsByType[provider] = models
        } catch {
            guard observedGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func createTeamFromDraft() async -> Bool {
        let teamId = draftTeamId.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = draftTeamName.trimmingCharacters(in: .whitespacesAndNewlines)
        let leader = draftTeamLeaderId.trimmingCharacters(in: .whitespacesAndNewlines)
        let members = Self.normalizedTeamMemberIds(draftTeamMemberIds, leaderAgentId: leader)
        let workflow = draftTeamWorkflow.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !teamId.isEmpty, !name.isEmpty, !leader.isEmpty else { return false }
        do {
            let team = try await client().createTeam(
                GaryxTeamRequest(
                    teamId: teamId,
                    displayName: name,
                    leaderAgentId: leader,
                    memberAgentIds: members,
                    workflowText: workflow
                )
            )
            draftTeamId = ""
            draftTeamName = ""
            draftTeamLeaderId = ""
            draftTeamMemberIds = ""
            draftTeamWorkflow = ""
            replaceTeam(team)
            setSelectedAgentTarget(team.id)
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func deleteTeam(_ team: GaryxTeamSummary) async {
        do {
            _ = try await client().deleteTeam(teamId: team.id)
            teams.removeAll { $0.id == team.id }
            ensureSelectedAgentTarget()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func selectWorkspace(_ path: String) async {
        selectedWorkspacePath = path
        draftWorkspacePath = path
        selectedWorkspaceDirectory = ""
        workspaceListing = nil
        workspacePreview = nil
        workspaceUploadStatus = nil
        await refreshSelectedWorkspace()
    }

    func prepareWorkspaceBrowser() async {
        ensureSelectedWorkspace()
        guard !selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        await refreshSelectedWorkspace()
    }

    func selectDraftWorkspace() async {
        let path = draftWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else { return }
        await selectWorkspace(path)
    }

    func refreshSelectedWorkspace() async {
        let path = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else { return }
        let directory = selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let gateway = try client()
            async let listingResult = gateway.listWorkspaceFiles(
                workspaceDir: path,
                directoryPath: directory.isEmpty ? nil : directory
            )
            async let gitStatusResult = gateway.workspaceGitStatus(workspaceDir: path)
            let listing = try await listingResult
            guard isCurrentWorkspaceRequest(
                workspace: path,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            workspaceListing = listing
            if let status = try? await gitStatusResult {
                guard isCurrentWorkspaceRequest(
                    workspace: path,
                    directory: directory,
                    runtimeGeneration: runtimeGeneration
                ) else { return }
                workspaceGitStatuses[path] = status
            }
        } catch {
            guard isCurrentWorkspaceRequest(
                workspace: path,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            lastError = displayMessage(for: error)
        }
    }

    func openWorkspaceEntry(_ entry: GaryxWorkspaceFileEntry) async {
        guard !selectedWorkspacePath.isEmpty else { return }
        if entry.entryType == "directory" {
            selectedWorkspaceDirectory = entry.path
            workspaceListing = nil
            workspacePreview = nil
            await refreshSelectedWorkspace()
            return
        }
        let workspace = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        let directory = selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let preview = try await client().previewWorkspaceFile(
                workspaceDir: workspace,
                path: entry.path
            )
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            workspacePreview = preview
        } catch {
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            lastError = displayMessage(for: error)
        }
    }

    func goUpWorkspaceDirectory() async {
        guard !selectedWorkspaceDirectory.isEmpty else { return }
        let parent = (selectedWorkspaceDirectory as NSString).deletingLastPathComponent
        selectedWorkspaceDirectory = parent == "." ? "" : parent
        workspaceListing = nil
        workspacePreview = nil
        await refreshSelectedWorkspace()
    }

    func uploadFilesToSelectedWorkspace(from urls: [URL]) async {
        let workspace = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty, !urls.isEmpty else { return }
        let directory = selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        let runtimeGeneration = gatewayRuntimeGeneration
        isUploadingWorkspaceFiles = true
        workspaceUploadStatus = nil
        defer { isUploadingWorkspaceFiles = false }
        do {
            var files: [GaryxUploadFileBlob] = []
            for url in urls {
                let didStartAccess = url.startAccessingSecurityScopedResource()
                defer {
                    if didStartAccess {
                        url.stopAccessingSecurityScopedResource()
                    }
                }
                let values = try url.resourceValues(forKeys: [.isDirectoryKey, .nameKey, .contentTypeKey])
                if values.isDirectory == true {
                    continue
                }
                let data = try Data(contentsOf: url)
                let name = (values.name ?? url.lastPathComponent).trimmingCharacters(in: .whitespacesAndNewlines)
                guard !name.isEmpty else { continue }
                let mediaType = values.contentType?.preferredMIMEType
                    ?? UTType(filenameExtension: (name as NSString).pathExtension)?.preferredMIMEType
                files.append(
                    GaryxUploadFileBlob(
                        name: name,
                        mediaType: mediaType,
                        dataBase64: data.base64EncodedString()
                    )
                )
            }
            guard !files.isEmpty else {
                guard isCurrentWorkspaceRequest(
                    workspace: workspace,
                    directory: directory,
                    runtimeGeneration: runtimeGeneration
                ) else { return }
                workspaceUploadStatus = "No files selected"
                return
            }
            let result = try await client().uploadWorkspaceFiles(
                GaryxUploadWorkspaceFilesRequest(
                    workspaceDir: workspace,
                    path: directory.isEmpty ? nil : directory,
                    files: files
                )
            )
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            workspaceUploadStatus = files.count == 1 ? "Uploaded \(files[0].name)" : "Uploaded \(files.count) files"
            await refreshSelectedWorkspace()
            if let firstPath = result.uploadedPaths.first?.trimmingCharacters(in: .whitespacesAndNewlines),
               !firstPath.isEmpty {
                let preview = try? await client().previewWorkspaceFile(workspaceDir: workspace, path: firstPath)
                guard isCurrentWorkspaceRequest(
                    workspace: workspace,
                    directory: directory,
                    runtimeGeneration: runtimeGeneration
                ) else { return }
                workspacePreview = preview
            }
        } catch {
            guard isCurrentWorkspaceRequest(
                workspace: workspace,
                directory: directory,
                runtimeGeneration: runtimeGeneration
            ) else { return }
            workspaceUploadStatus = nil
            lastError = displayMessage(for: error)
        }
    }

    private func isCurrentWorkspaceRequest(
        workspace: String,
        directory: String,
        runtimeGeneration: UUID
    ) -> Bool {
        runtimeGeneration == gatewayRuntimeGeneration
            && selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines) == workspace
            && selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines) == directory
    }

    func createAutomationFromDraft() async -> Bool {
        let label = draftAutomationLabel.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = draftAutomationPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        let targetThreadId = draftAutomationTargetsExistingThread
            ? draftAutomationTargetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            : ""
        guard !label.isEmpty, !prompt.isEmpty else { return false }
        guard !targetThreadId.isEmpty || !workspace.isEmpty else { return false }
        let trimmedHours = draftAutomationIntervalHours.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let hours = Int(trimmedHours), hours > 0 else { return false }
        do {
            let automation = try await client().createAutomation(
                GaryxAutomationCreateRequest(
                    label: label,
                    prompt: prompt,
                    agentId: selectedAgentTargetId,
                    workspaceDir: targetThreadId.isEmpty && !workspace.isEmpty ? workspace : nil,
                    targetThreadId: targetThreadId.isEmpty ? nil : targetThreadId,
                    schedule: .interval(hours: hours),
                    enabled: true
                )
            )
            draftAutomationLabel = ""
            draftAutomationPrompt = ""
            draftAutomationTargetThreadId = ""
            draftAutomationTargetsExistingThread = false
            automations.insert(automation, at: 0)
            activePanel = .automations
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func deleteAutomation(_ automation: GaryxAutomationSummary) async {
        do {
            _ = try await client().deleteAutomation(id: automation.id)
            automations.removeAll { $0.id == automation.id }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func createSkillFromDraft() async -> Bool {
        let id = draftSkillId.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = draftSkillName.trimmingCharacters(in: .whitespacesAndNewlines)
        let description = draftSkillDescription.trimmingCharacters(in: .whitespacesAndNewlines)
        let body = draftSkillBody.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty, !name.isEmpty else { return false }
        do {
            let skill = try await client().createSkill(
                GaryxCreateSkillRequest(
                    id: id,
                    name: name,
                    description: description,
                    body: body.isEmpty ? "" : body
                )
            )
            draftSkillId = ""
            draftSkillName = ""
            draftSkillDescription = ""
            draftSkillBody = ""
            skills.insert(skill, at: 0)
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func toggleSkill(_ skill: GaryxSkillSummary) async {
        do {
            let updated = try await client().toggleSkill(skillId: skill.id)
            replaceSkill(updated)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteSkill(_ skill: GaryxSkillSummary) async {
        do {
            _ = try await client().deleteSkill(skillId: skill.id)
            skills.removeAll { $0.id == skill.id }
            if selectedSkillEditor?.skill.id == skill.id {
                selectedSkillEditor = nil
                selectedSkillDocument = nil
                selectedSkillFileContent = ""
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateSkill(_ skill: GaryxSkillSummary, name: String, description: String) async {
        do {
            let updated = try await client().updateSkill(
                skillId: skill.id,
                request: GaryxUpdateSkillRequest(name: name, description: description)
            )
            replaceSkill(updated)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func openSkillEditor(_ skill: GaryxSkillSummary) async {
        do {
            selectedSkillEditor = try await client().skillEditor(skillId: skill.id)
            selectedSkillDocument = nil
            selectedSkillFileContent = ""
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func openSkillFile(skillId: String, path: String) async {
        do {
            let document = try await client().readSkillFile(skillId: skillId, path: path)
            selectedSkillDocument = document
            selectedSkillFileContent = document.content
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func saveSelectedSkillFile() async {
        guard let document = selectedSkillDocument else { return }
        do {
            let saved = try await client().saveSkillFile(
                skillId: document.skill.id,
                request: GaryxSkillFileWriteRequest(path: document.path, content: selectedSkillFileContent)
            )
            selectedSkillDocument = saved
            selectedSkillFileContent = saved.content
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func createSkillEntry() async {
        guard let editor = selectedSkillEditor else { return }
        let path = draftSkillEntryPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else { return }
        do {
            selectedSkillEditor = try await client().createSkillEntry(
                skillId: editor.skill.id,
                request: GaryxSkillEntryCreateRequest(path: path, entryType: draftSkillEntryType)
            )
            draftSkillEntryPath = ""
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteSkillEntry(skillId: String, path: String) async {
        do {
            selectedSkillEditor = try await client().deleteSkillEntry(skillId: skillId, path: path)
            if selectedSkillDocument?.path == path {
                selectedSkillDocument = nil
                selectedSkillFileContent = ""
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func createSlashCommandFromDraft() async -> Bool {
        let name = draftSlashName.trimmingCharacters(in: .whitespacesAndNewlines)
        let description = draftSlashDescription.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = draftSlashPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, !description.isEmpty, !prompt.isEmpty else { return false }
        do {
            let command = try await client().createSlashCommand(
                GaryxSlashCommandRequest(name: name, description: description, prompt: prompt)
            )
            draftSlashName = ""
            draftSlashDescription = ""
            draftSlashPrompt = ""
            slashCommands.append(command)
            slashCommands.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func deleteSlashCommand(_ command: GaryxSlashCommand) async {
        do {
            _ = try await client().deleteSlashCommand(name: command.name)
            slashCommands.removeAll { $0.name == command.name }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateSlashCommand(_ command: GaryxSlashCommand, name: String, description: String, prompt: String) async {
        let nextName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextDescription = description.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextPrompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextName.isEmpty, !nextDescription.isEmpty else { return }
        do {
            let updated = try await client().updateSlashCommand(
                currentName: command.name,
                request: GaryxSlashCommandRequest(
                    name: nextName,
                    description: nextDescription,
                    prompt: nextPrompt.isEmpty ? nil : nextPrompt
                )
            )
            replaceSlashCommand(updated, previousName: command.name)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func createMcpServerFromDraft() async -> Bool {
        let name = draftMcpName.trimmingCharacters(in: .whitespacesAndNewlines)
        let command = draftMcpCommand.trimmingCharacters(in: .whitespacesAndNewlines)
        let url = draftMcpUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, !command.isEmpty || !url.isEmpty else { return false }
        do {
            let request = GaryxMcpServerRequest(
                name: name,
                transport: url.isEmpty ? "stdio" : "streamable_http",
                command: command,
                args: splitShellLikeList(draftMcpArgs),
                env: keyValueDictionary(from: draftMcpEnv),
                enabled: true,
                workingDir: draftMcpWorkingDir.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty,
                url: url.isEmpty ? nil : url,
                headers: keyValueDictionary(from: draftMcpHeaders)
            )
            let server = try await client().createMcpServer(request)
            draftMcpName = ""
            draftMcpCommand = ""
            draftMcpArgs = ""
            draftMcpEnv = ""
            draftMcpWorkingDir = ""
            draftMcpUrl = ""
            draftMcpHeaders = ""
            mcpServers.append(server)
            mcpServers.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func toggleMcpServer(_ server: GaryxMcpServer) async {
        do {
            let updated = try await client().toggleMcpServer(name: server.name, enabled: !server.enabled)
            replaceMcpServer(updated)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteMcpServer(_ server: GaryxMcpServer) async {
        do {
            _ = try await client().deleteMcpServer(name: server.name)
            mcpServers.removeAll { $0.name == server.name }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateMcpServer(
        _ server: GaryxMcpServer,
        name: String,
        command: String,
        argsText: String,
        envText: String,
        workingDir: String,
        url: String,
        headersText: String
    ) async {
        let nextName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextCommand = command.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextUrl = url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextName.isEmpty, !nextCommand.isEmpty || !nextUrl.isEmpty else { return }
        do {
            let updated = try await client().updateMcpServer(
                currentName: server.name,
                request: GaryxMcpServerRequest(
                    name: nextName,
                    transport: nextUrl.isEmpty ? "stdio" : "streamable_http",
                    command: nextCommand,
                    args: splitShellLikeList(argsText),
                    env: keyValueDictionary(from: envText),
                    enabled: server.enabled,
                    workingDir: workingDir.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty,
                    url: nextUrl.isEmpty ? nil : nextUrl,
                    headers: keyValueDictionary(from: headersText)
                )
            )
            replaceMcpServer(updated, previousName: server.name)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func createAutoResearchRunFromDraft() async -> Bool {
        let goal = draftAutoResearchGoal.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        let iterationsText = draftAutoResearchIterations.trimmingCharacters(in: .whitespacesAndNewlines)
        let timeBudgetText = draftAutoResearchTimeBudgetMinutes.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !goal.isEmpty,
              !workspace.isEmpty,
              let iterations = Int(iterationsText), iterations > 0,
              let timeBudgetMinutes = Int(timeBudgetText),
              timeBudgetMinutes > 0,
              timeBudgetMinutes <= Int.max / 60 else {
            return false
        }
        let timeBudgetSecs = timeBudgetMinutes * 60
        do {
            let run = try await client().createAutoResearchRun(
                GaryxAutoResearchCreateRequest(
                    goal: goal,
                    workspaceDir: workspace,
                    maxIterations: iterations,
                    timeBudgetSecs: timeBudgetSecs
                )
            )
            draftAutoResearchGoal = ""
            autoResearchRuns.insert(run, at: 0)
            activePanel = .autoResearch
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func stopAutoResearchRun(_ run: GaryxAutoResearchRun) async {
        do {
            let updated = try await client().stopAutoResearchRun(runId: run.runId, reason: "user_requested")
            replaceAutoResearchRun(updated)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func loadAutoResearchDetail(_ run: GaryxAutoResearchRun) async {
        await loadAutoResearchDetail(runId: run.runId)
    }

    func loadAutoResearchDetail(runId: String) async {
        let runId = runId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !runId.isEmpty else { return }
        do {
            let gateway = try client()
            async let detailResult = gateway.getAutoResearchRun(runId: runId)
            async let iterationsResult = gateway.listAutoResearchIterations(runId: runId)
            let detail = try await detailResult
            let iterations = try await iterationsResult
            autoResearchDetailsByRunId[runId] = detail
            autoResearchIterationsByRunId[runId] = iterations
            replaceAutoResearchRun(detail.run)
            if let page = try? await gateway.listAutoResearchCandidates(runId: runId) {
                researchCandidatesByRunId[runId] = page
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func loadAutoResearchCandidates(_ run: GaryxAutoResearchRun) async {
        do {
            researchCandidatesByRunId[run.runId] = try await client().listAutoResearchCandidates(runId: run.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func selectAutoResearchCandidate(run: GaryxAutoResearchRun, candidate: GaryxResearchCandidate) async {
        do {
            let updated = try await client().selectAutoResearchCandidate(
                runId: run.runId,
                candidateId: candidate.candidateId
            )
            replaceAutoResearchRun(updated)
            await loadAutoResearchDetail(runId: updated.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func reverifyAutoResearchCandidate(run: GaryxAutoResearchRun, candidate: GaryxResearchCandidate) async {
        do {
            let updated = try await client().reverifyAutoResearchCandidate(
                runId: run.runId,
                request: GaryxAutoResearchReverifyRequest(candidateId: candidate.candidateId)
            )
            replaceAutoResearchRun(updated)
            await loadAutoResearchDetail(runId: updated.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func sendAutoResearchFeedback(
        run: GaryxAutoResearchRun,
        candidate: GaryxResearchCandidate?,
        feedback: String
    ) async {
        let feedback = feedback.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !feedback.isEmpty else { return }
        do {
            let message: String
            if let candidate {
                message = "Candidate \(candidate.iteration): \(feedback)"
            } else {
                message = feedback
            }
            let updated = try await client().sendAutoResearchFeedback(
                runId: run.runId,
                request: GaryxAutoResearchFeedbackRequest(message: message)
            )
            replaceAutoResearchRun(updated)
            await loadAutoResearchDetail(runId: updated.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteAutoResearchRun(_ run: GaryxAutoResearchRun) async {
        do {
            _ = try await client().deleteAutoResearchRun(runId: run.runId)
            autoResearchRuns.removeAll { $0.runId == run.runId }
            researchCandidatesByRunId.removeValue(forKey: run.runId)
            autoResearchDetailsByRunId.removeValue(forKey: run.runId)
            autoResearchIterationsByRunId.removeValue(forKey: run.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func openBotThread(_ threadId: String?) async {
        guard let threadId, !threadId.isEmpty else { return }
        await openThread(id: threadId)
    }

    func loadBotStatus(_ bot: GaryxConfiguredBot) async {
        do {
            botStatusesById[bot.id] = try await client().botStatus(botId: bot.id)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func bindBotToSelectedThread(_ bot: GaryxConfiguredBot) async {
        guard let threadId = selectedThread?.id else { return }
        do {
            botStatusesById[bot.id] = try await client().bindBot(botId: bot.id, threadId: threadId)
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func unbindBot(_ bot: GaryxConfiguredBot) async {
        do {
            botStatusesById[bot.id] = try await client().unbindBot(botId: bot.id)
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteConfiguredBotAccount(_ bot: GaryxConfiguredBot) async {
        do {
            var settings = try await client().gatewaySettings()
            guard Self.removeChannelAccount(
                from: &settings,
                channel: bot.channel,
                accountId: bot.accountId
            ) else {
                lastError = "Bot account not found"
                return
            }
            _ = try await client().saveGatewaySettings(settings, merge: false)
            configuredBots.removeAll { $0.id == bot.id }
            channelEndpoints.removeAll { endpoint in
                endpoint.channel.caseInsensitiveCompare(bot.channel) == .orderedSame
                    && endpoint.accountId == bot.accountId
            }
            botConsoles.removeAll {
                $0.channel.caseInsensitiveCompare(bot.channel) == .orderedSame
                    && $0.accountId == bot.accountId
            }
            botStatusesById.removeValue(forKey: bot.id)
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func bindEndpointToSelectedThread(_ endpoint: GaryxChannelEndpoint) async {
        guard let threadId = selectedThread?.id else { return }
        do {
            _ = try await client().bindChannelEndpoint(endpointKey: endpoint.endpointKey, threadId: threadId)
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func detachEndpoint(_ endpoint: GaryxChannelEndpoint) async {
        do {
            _ = try await client().detachChannelEndpoint(endpointKey: endpoint.endpointKey)
            await refreshRemoteState()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func archiveBotConversationEndpoint(_ endpoint: GaryxChannelEndpoint) async {
        let threadId = endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !threadId.isEmpty else { return }
        guard !isThreadBusy(threadId) else {
            lastError = "This thread is active."
            return
        }
        guard !automations.contains(where: { $0.threadId == threadId }) else {
            lastError = "Delete this automation first."
            return
        }

        var endpointKeys = Set(
            channelEndpoints
                .filter { $0.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) == threadId }
                .map(\.endpointKey)
                .filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        )
        let currentEndpointKey = endpoint.endpointKey.trimmingCharacters(in: .whitespacesAndNewlines)
        if !currentEndpointKey.isEmpty {
            endpointKeys.insert(currentEndpointKey)
        }

        do {
            for endpointKey in endpointKeys {
                _ = try await client().detachChannelEndpoint(endpointKey: endpointKey)
            }
            _ = try await client().deleteThread(threadId: threadId)
            unpinThread(threadId)
            if selectedThread?.id == threadId {
                selectedThread = nil
                draftThreadTitle = ""
                resetComposerDraft()
                messages = []
                cancelSelectedThreadReconcileLoop()
                resetSelectedThreadHistoryPagination()
            }
            discardPendingAssistantDelta(for: threadId)
            messagesByThread[threadId] = nil
            messageSignaturesByThread[threadId] = nil
            activeAssistantMessageIdsByThread[threadId] = nil
            await refreshRemoteState()
            await refreshThreads()
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    private func receiveEvents(from task: URLSessionWebSocketTask, threadId: String, assistantMessageId: String) async {
        while !Task.isCancelled {
            do {
                let message = try await task.receive()
                let text: String
                switch message {
                case .string(let value):
                    text = value
                case .data(let data):
                    text = String(data: data, encoding: .utf8) ?? ""
                @unknown default:
                    text = ""
                }
                guard !text.isEmpty else { continue }
                let event = try client().decodeStreamEvent(text)
                let eventThreadId = Self.threadId(from: event)
                let affectsActiveRun = eventThreadId == threadId
                    || (eventThreadId.isEmpty && activeTasksByThread[threadId] === task)
                if !affectsActiveRun {
                    updateRemoteBusyState(from: event)
                    continue
                }
                let isVisibleThread = selectedThread?.id == threadId
                if !isVisibleThread {
                    handle(event, threadId: threadId, assistantMessageId: assistantMessageId, affectsActiveRun: affectsActiveRun)
                    if case .done = event {
                        task.cancel(with: .normalClosure, reason: nil)
                        clearActiveRun(task: task, threadId: threadId)
                        await refreshThreads()
                        return
                    }
                    if case .runComplete = event {
                        task.cancel(with: .normalClosure, reason: nil)
                        clearActiveRun(task: task, threadId: threadId)
                        await refreshThreads()
                        return
                    }
                    if case let .error(_, _, error) = event {
                        if Self.isTransientGatewayErrorMessage(error) {
                            remoteBusyThreadIds.insert(threadId)
                            await refreshThreads()
                        }
                        task.cancel(with: .goingAway, reason: nil)
                        clearActiveRun(task: task, threadId: threadId)
                        return
                    }
                    if case .interrupt = event {
                        clearActiveRun(task: task, threadId: threadId)
                        return
                    }
                    continue
                }
                handle(event, threadId: threadId, assistantMessageId: assistantMessageId, affectsActiveRun: affectsActiveRun)
                if case .done = event, affectsActiveRun {
                    task.cancel(with: .normalClosure, reason: nil)
                    clearActiveRun(task: task, threadId: threadId)
                    await refreshThreads()
                    if selectedThread?.id == threadId {
                        await loadSelectedThreadHistory()
                    }
                    return
                }
                if case .runComplete = event, affectsActiveRun {
                    task.cancel(with: .normalClosure, reason: nil)
                    clearActiveRun(task: task, threadId: threadId)
                    await refreshThreads()
                    if selectedThread?.id == threadId {
                        await loadSelectedThreadHistory()
                    }
                    return
                }
                if case let .error(_, _, error) = event, affectsActiveRun {
                    task.cancel(with: .goingAway, reason: nil)
                    clearActiveRun(task: task, threadId: threadId)
                    if Self.isTransientGatewayErrorMessage(error), selectedThread?.id == threadId {
                        await loadSelectedThreadHistory()
                    }
                    return
                }
                if case .interrupt = event, affectsActiveRun {
                    task.cancel(with: .normalClosure, reason: nil)
                    clearActiveRun(task: task, threadId: threadId)
                    await refreshThreads()
                    if selectedThread?.id == threadId {
                        await loadSelectedThreadHistory()
                    }
                    return
                }
            } catch {
                guard activeTasksByThread[threadId] === task else {
                    return
                }
                let message = displayMessage(for: error)
                let isTransient = Self.isTransientGatewayErrorMessage(message)
                if isTransient {
                    remoteBusyThreadIds.insert(threadId)
                    gatewaySettingsStatus = "Waiting to sync with gateway"
                } else if isSending {
                    lastError = message
                }
                clearActiveRun(task: task, threadId: threadId)
                if selectedThread?.id == threadId {
                    if isTransient {
                        await loadSelectedThreadHistory()
                    } else {
                        markStreamingAssistantComplete(for: threadId, removeEmpty: true)
                    }
                }
                return
            }
        }
    }

    private func updateRemoteBusyState(from event: GaryxChatStreamEvent) {
        let threadId = Self.threadId(from: event)
        guard !threadId.isEmpty else { return }
        switch event {
        case .accepted,
             .userMessage,
             .assistantDelta,
             .assistantBoundary,
             .userAck,
             .toolUse,
             .toolResult:
            if activeTasksByThread[threadId] == nil {
                remoteBusyThreadIds.insert(threadId)
            } else {
                remoteBusyThreadIds.remove(threadId)
            }
        case .streamInput(let status, _, _, _):
            if Self.isSuccessfulStreamInputStatus(status) {
                if activeTasksByThread[threadId] == nil {
                    remoteBusyThreadIds.insert(threadId)
                } else {
                    remoteBusyThreadIds.remove(threadId)
                }
            } else if activeTasksByThread[threadId] == nil {
                remoteBusyThreadIds.remove(threadId)
            }
        case .done, .runComplete, .error, .interrupt:
            remoteBusyThreadIds.remove(threadId)
        default:
            break
        }
    }

    private func handle(_ event: GaryxChatStreamEvent, threadId: String, assistantMessageId: String, affectsActiveRun: Bool) {
        let eventThreadId = Self.threadId(from: event)
        updateRemoteBusyState(from: event)
        if !Self.isAssistantDeltaEvent(event) {
            flushPendingAssistantDelta(for: threadId)
        }
        switch event {
        case .userMessage(let runId, _, let text, let imageCount):
            appendRemoteUserMessage(
                runId: runId,
                threadId: threadId,
                text: text,
                imageCount: imageCount
            )
        case .assistantDelta(_, _, let delta, _):
            appendAssistantDelta(delta, threadId: threadId, assistantMessageId: assistantMessageId)
        case .assistantBoundary:
            appendAssistantBoundary(threadId: threadId, assistantMessageId: assistantMessageId)
        case .toolUse(_, _, let message):
            appendToolTraceEvent(.toolUse, threadId: threadId, message: message)
        case .toolResult(_, _, let message):
            appendToolTraceEvent(.toolResult, threadId: threadId, message: message)
        case .userAck where affectsActiveRun:
            markActiveAssistantSegmentComplete(for: threadId)
            activeAssistantMessageIdsByThread[threadId] = nil
            if selectedThread?.id == eventThreadId,
               activeTasksByThread[eventThreadId] == nil {
                Task { await loadSelectedThreadHistory() }
            }
        case .streamInput(let status, _, let clientIntentId, let pendingInputId):
            if Self.isSuccessfulStreamInputStatus(status) {
                bindLocalPendingInput(threadId: threadId, clientIntentId: clientIntentId, pendingInputId: pendingInputId)
            } else {
                let normalizedClientIntentId = clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                if Self.shouldFallbackStreamInputStatus(status),
                   !normalizedClientIntentId.isEmpty,
                   let queued = pendingQueuedInputsByIntentId.removeValue(forKey: normalizedClientIntentId) {
                    Task { await dispatchQueuedInputFallback(queued) }
                    break
                }
                let failureMessage = status.isEmpty ? "Input was not queued" : status
                let markedInput = markLocalInputFailed(
                    threadId: threadId,
                    clientIntentId: clientIntentId,
                    pendingInputId: pendingInputId,
                    message: failureMessage
                )
                if !markedInput {
                    lastError = failureMessage
                }
            }
        case .threadTitleUpdated(_, let threadId, let title):
            applyThreadTitleUpdate(threadId: threadId, title: title)
        case .done where affectsActiveRun:
            if !eventThreadId.isEmpty {
                remoteBusyThreadIds.remove(eventThreadId)
            }
            clearActiveRun(task: nil, threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        case .runComplete where affectsActiveRun:
            if !eventThreadId.isEmpty {
                remoteBusyThreadIds.remove(eventThreadId)
            }
            clearActiveRun(task: nil, threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        case .error(_, _, let error) where affectsActiveRun:
            if Self.isTransientGatewayErrorMessage(error) {
                if !eventThreadId.isEmpty {
                    remoteBusyThreadIds.insert(eventThreadId)
                }
                gatewaySettingsStatus = "Waiting to sync with gateway"
                markStreamingAssistantComplete(for: threadId, removeEmpty: true)
            } else {
                if !eventThreadId.isEmpty {
                    remoteBusyThreadIds.remove(eventThreadId)
                }
                lastError = error
                markLatestLocalUserFailed(for: threadId, message: error)
                markStreamingAssistantComplete(for: threadId, removeEmpty: true)
            }
            clearActiveRun(task: nil, threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
        case .interrupt where affectsActiveRun:
            if !eventThreadId.isEmpty {
                remoteBusyThreadIds.remove(eventThreadId)
            }
            clearActiveRun(task: nil, threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        case .snapshot(let threadId, let payload):
            guard selectedThread?.id == threadId,
                  let transcript = try? transcript(fromSnapshotPayload: payload) else {
                return
            }
            selectedThreadActivitySignatures[threadId] = GaryxThreadActivitySignature.make(from: transcript)
            updateThreadRuntimeState(threadId: threadId, transcript: transcript)
            scheduleSelectedThreadRecoveryIfNeeded(threadId: threadId)
            if activeTasksByThread[threadId] != nil {
                return
            }
            let remoteMessages = mobileMessages(from: transcript, threadId: threadId, live: remoteBusyThreadIds.contains(threadId))
            setMessages(
                mergedMessages(
                    remoteMessages,
                    withLocal: cachedMessages(for: threadId),
                    preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript)
                ),
                for: threadId,
                reconcileActiveAssistant: true
            )
        default:
            break
        }
    }

    private func appendAssistantDelta(_ delta: String, threadId: String, assistantMessageId: String) {
        guard !delta.isEmpty else { return }
        let targetId = activeAssistantMessageIdsByThread[threadId]
            ?? "stream-assistant-\(threadId)-\(UUID().uuidString)"
        activeAssistantMessageIdsByThread[threadId] = targetId
        if var pending = pendingAssistantDeltasByThread[threadId],
           pending.targetId == targetId {
            pending.text += delta
            pendingAssistantDeltasByThread[threadId] = pending
        } else {
            pendingAssistantDeltasByThread[threadId] = PendingAssistantDelta(targetId: targetId, text: delta)
        }
        scheduleAssistantDeltaFlush(for: threadId)
    }

    private func scheduleAssistantDeltaFlush(for threadId: String) {
        guard assistantDeltaFlushTasksByThread[threadId] == nil else { return }
        assistantDeltaFlushTasksByThread[threadId] = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.assistantDeltaFlushDelayNanos)
            await MainActor.run {
                self?.flushPendingAssistantDelta(for: threadId)
            }
        }
    }

    private func flushPendingAssistantDelta(for threadId: String) {
        assistantDeltaFlushTasksByThread[threadId]?.cancel()
        assistantDeltaFlushTasksByThread[threadId] = nil
        guard let pending = pendingAssistantDeltasByThread.removeValue(forKey: threadId),
              !pending.text.isEmpty else {
            return
        }
        let targetId = pending.targetId
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { $0.id == targetId }) else {
                activeAssistantMessageIdsByThread[threadId] = targetId
                messages.append(
                    GaryxMobileMessage(
                        id: targetId,
                        role: .assistant,
                        text: pending.text,
                        timestamp: nil,
                        isStreaming: true
                    )
                )
                return
            }
            messages[index].text += pending.text
            messages[index].isStreaming = true
        }
    }

    private func discardPendingAssistantDelta(for threadId: String) {
        assistantDeltaFlushTasksByThread[threadId]?.cancel()
        assistantDeltaFlushTasksByThread[threadId] = nil
        pendingAssistantDeltasByThread[threadId] = nil
    }

    private static func isAssistantDeltaEvent(_ event: GaryxChatStreamEvent) -> Bool {
        if case .assistantDelta = event {
            return true
        }
        return false
    }

    private func appendRemoteUserMessage(runId: String, threadId: String, text: String, imageCount: Int) {
        let messageId = runId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "remote-user-\(threadId)-\(UUID().uuidString)"
            : "remote-user-\(runId)"
        let visibleText = Self.remoteUserMessageText(text: text, imageCount: imageCount)
        mutateMessages(for: threadId) { messages in
            if messages.contains(where: { $0.id == messageId }) {
                return
            }
            if let localIndex = messages.firstIndex(where: { message in
                message.role == .user
                    && (message.id.hasPrefix("local-user-") || message.id.hasPrefix("pending-user:"))
                    && Self.normalizedMergeText(message.text) == Self.normalizedMergeText(visibleText)
            }) {
                let local = messages[localIndex]
                let remoteMessage = GaryxMobileMessage(
                    id: messageId,
                    role: .user,
                    text: visibleText,
                    attachments: local.attachments,
                    timestamp: local.timestamp,
                    isStreaming: false,
                    statusText: local.statusText,
                    clientIntentId: local.clientIntentId,
                    pendingInputId: local.pendingInputId
                )
                messages[localIndex] = remoteMessage
                return
            }
            messages.append(
                GaryxMobileMessage(
                    id: messageId,
                    role: .user,
                    text: visibleText,
                    timestamp: nil,
                    isStreaming: false
                )
            )
        }
    }

    private func appendAssistantBoundary(threadId: String, assistantMessageId: String) {
        guard let targetId = activeAssistantMessageIdsByThread[threadId] else {
            return
        }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { $0.id == targetId }) else {
                return
            }
            let hasText = !messages[index].text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            guard hasText else { return }
            messages[index].text += "\n\n"
            messages[index].isStreaming = true
            activeAssistantMessageIdsByThread[threadId] = messages[index].id
        }
    }

    private func markStreamingAssistantComplete(for threadId: String, removeEmpty: Bool = false) {
        mutateMessages(for: threadId) { messages in
            if removeEmpty {
                messages.removeAll { message in
                    message.role == .assistant
                        && message.isStreaming
                        && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                }
            }
            for index in messages.indices where messages[index].isStreaming {
                messages[index].isStreaming = false
                messages[index].toolTraceGroup?.live = false
            }
        }
        activeAssistantMessageIdsByThread[threadId] = nil
    }

    private func markActiveAssistantSegmentComplete(for threadId: String) {
        guard let activeAssistantMessageId = activeAssistantMessageIdsByThread[threadId] else { return }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { $0.id == activeAssistantMessageId }),
                  messages[index].role == .assistant else {
                return
            }
            messages[index].isStreaming = false
        }
    }

    private func suspendStreamingAssistantForBackground(threadId: String) -> String? {
        flushPendingAssistantDelta(for: threadId)
        let activeAssistantMessageId = activeAssistantMessageIdsByThread[threadId]
        var preservedAssistantId: String?
        mutateMessages(for: threadId) { messages in
            messages.removeAll { message in
                message.role == .assistant
                    && message.isStreaming
                    && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            }
            if let activeAssistantMessageId,
               let index = messages.firstIndex(where: { $0.id == activeAssistantMessageId }),
               messages[index].role == .assistant {
                messages[index].isStreaming = true
                preservedAssistantId = activeAssistantMessageId
            }
        }
        return preservedAssistantId
    }

    private func markLatestLocalUserFailed(for threadId: String, message: String) {
        mutateMessages(for: threadId) { messages in
            guard let index = messages.indices.last(where: { index in
                messages[index].role == .user
            }) else {
                return
            }
            messages[index].statusText = message
        }
    }

    @discardableResult
    private func markLocalInputFailed(
        threadId: String,
        clientIntentId: String?,
        pendingInputId: String?,
        message: String
    ) -> Bool {
        let normalizedClientIntentId = clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let normalizedPendingInputId = pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        var didMark = false
        mutateMessages(for: threadId) { messages in
            let preciseIndex = messages.indices.last(where: { index in
                guard messages[index].role == .user else { return false }
                if !normalizedClientIntentId.isEmpty, messages[index].clientIntentId == normalizedClientIntentId {
                    return true
                }
                if !normalizedPendingInputId.isEmpty, messages[index].pendingInputId == normalizedPendingInputId {
                    return true
                }
                return false
            })
            let fallbackIndex: Int?
            if normalizedClientIntentId.isEmpty && normalizedPendingInputId.isEmpty {
                fallbackIndex = messages.indices.last(where: { index in
                    messages[index].role == .user && messages[index].id.hasPrefix("local-user-")
                })
            } else {
                fallbackIndex = nil
            }
            guard let index = preciseIndex ?? fallbackIndex else {
                return
            }
            messages[index].statusText = message
            didMark = true
        }
        return didMark
    }

    private func clearLocalInputStatus(threadId: String, clientIntentId: String) {
        let normalizedClientIntentId = clientIntentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedClientIntentId.isEmpty else { return }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.indices.last(where: { index in
                messages[index].role == .user && messages[index].clientIntentId == normalizedClientIntentId
            }) else {
                return
            }
            messages[index].statusText = nil
        }
    }

    private func bindLocalPendingInput(
        threadId: String,
        clientIntentId: String?,
        pendingInputId: String?
    ) {
        let normalizedClientIntentId = clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let normalizedPendingInputId = pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !normalizedClientIntentId.isEmpty || !normalizedPendingInputId.isEmpty else { return }
        if !normalizedClientIntentId.isEmpty {
            pendingQueuedInputsByIntentId.removeValue(forKey: normalizedClientIntentId)
        }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.indices.last(where: { index in
                messages[index].role == .user
                    && (messages[index].clientIntentId == normalizedClientIntentId
                        || messages[index].pendingInputId == normalizedPendingInputId)
            }) else {
                return
            }
            if !normalizedClientIntentId.isEmpty {
                messages[index].clientIntentId = normalizedClientIntentId
            }
            if !normalizedPendingInputId.isEmpty {
                messages[index].pendingInputId = normalizedPendingInputId
            }
            messages[index].statusText = nil
        }
    }

    private func mergedMessages(
        _ remoteMessages: [GaryxMobileMessage],
        withLocal localMessages: [GaryxMobileMessage],
        preserveRemoteBeforeIndex: Int? = nil
    ) -> [GaryxMobileMessage] {
        guard !remoteMessages.isEmpty else {
            return localMessages
        }

        var merged = remoteMessages
        var preservedOlderRemoteMessages: [GaryxMobileMessage] = []
        var preservedOlderRemoteIds = Set<String>()
        var remoteUserTextCounts = Dictionary(
            grouping: remoteMessages.filter { $0.role == .user }.map(Self.userMergeKey),
            by: { $0 }
        )
        .mapValues(\.count)
        for localRemoteUserText in localMessages
            .filter({ $0.role == .user && !$0.id.hasPrefix("local-user-") })
            .map(Self.userMergeKey) {
            if let count = remoteUserTextCounts[localRemoteUserText], count > 0 {
                remoteUserTextCounts[localRemoteUserText] = count - 1
            }
        }
        let currentTurnRemoteAssistantTexts = Self.currentTurnAssistantTexts(in: remoteMessages)
        let remoteClientIntentIds = Set(remoteMessages.compactMap { $0.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty })
        let remotePendingInputIds = Set(remoteMessages.compactMap { $0.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty })

        var isAfterUnmaterializedLocalUser = false
        for local in localMessages {
            if let remoteIndex = merged.firstIndex(where: { $0.id == local.id }) {
                if local.role == .assistant,
                   local.isStreaming,
                   merged[remoteIndex].role == .assistant,
                   Self.normalizedMergeText(local.text).count > Self.normalizedMergeText(merged[remoteIndex].text).count {
                    merged[remoteIndex] = local
                }
                continue
            }
            if let preserveRemoteBeforeIndex,
               let historyIndex = Self.historyIndex(fromMessageId: local.id),
               historyIndex < preserveRemoteBeforeIndex,
               preservedOlderRemoteIds.insert(local.id).inserted {
                preservedOlderRemoteMessages.append(local)
                continue
            }
            let localClientIntentId = local.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let localPendingInputId = local.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if !localClientIntentId.isEmpty,
               remoteClientIntentIds.contains(localClientIntentId) {
                isAfterUnmaterializedLocalUser = false
                continue
            }
            if !localPendingInputId.isEmpty,
               remotePendingInputIds.contains(localPendingInputId) {
                isAfterUnmaterializedLocalUser = false
                continue
            }
            let normalizedText = Self.normalizedMergeText(local.text)
            switch local.role {
            case .user:
                if local.id.hasPrefix("local-user-") {
                    let mergeKey = Self.userMergeKey(local)
                    if let count = remoteUserTextCounts[mergeKey],
                       count > 0 {
                        remoteUserTextCounts[mergeKey] = count - 1
                        isAfterUnmaterializedLocalUser = false
                        continue
                    }
                    merged.append(local)
                    isAfterUnmaterializedLocalUser = true
                } else {
                    isAfterUnmaterializedLocalUser = false
                }
            case .assistant:
                if local.isStreaming || local.id.hasPrefix("local-assistant-") {
                    if isAfterUnmaterializedLocalUser {
                        merged.append(local)
                        continue
                    }
                    let alreadyMaterialized = currentTurnRemoteAssistantTexts.contains { remoteText in
                        !normalizedText.isEmpty
                            && !remoteText.isEmpty
                            && remoteText.count >= normalizedText.count
                            && remoteText.hasPrefix(normalizedText)
                    }
                    if !alreadyMaterialized {
                        merged.append(local)
                    }
                }
            case .tool:
                if local.isStreaming || local.toolTraceGroup?.isActive == true {
                    if let localGroup = local.toolTraceGroup,
                       let remoteIndex = merged.indices.first(where: { remoteIndex in
                           let remote = merged[remoteIndex]
                           guard let remoteGroup = remote.toolTraceGroup else { return false }
                           return Self.toolTraceGroupsOverlap(
                               remoteGroup,
                               localGroup,
                               allowFingerprint: Self.isInCurrentTurn(index: remoteIndex, messages: merged)
                           )
                       }) {
                        if var remoteGroup = merged[remoteIndex].toolTraceGroup {
                            remoteGroup = Self.mergedToolTraceGroup(remoteGroup, with: localGroup)
                            merged[remoteIndex].toolTraceGroup = remoteGroup
                            merged[remoteIndex].text = remoteGroup.summary
                            merged[remoteIndex].isStreaming = remoteGroup.isActive
                        }
                        continue
                    }
                    merged.append(local)
                }
            case .system:
                if local.statusText != nil || local.id.hasPrefix("local-") {
                    merged.append(local)
                }
            }
        }

        if !preservedOlderRemoteMessages.isEmpty {
            merged = preservedOlderRemoteMessages + merged
        }
        return merged
    }

    private static func historyIndex(fromMessageId id: String) -> Int? {
        guard let range = id.range(of: "history:") else { return nil }
        let suffix = id[range.upperBound...]
        let digits = suffix.prefix { $0.isNumber }
        guard !digits.isEmpty else { return nil }
        return Int(digits)
    }

    private static func currentTurnAssistantTexts(in messages: [GaryxMobileMessage]) -> [String] {
        let startIndex: Array<GaryxMobileMessage>.Index
        if let lastUserIndex = messages.lastIndex(where: { $0.role == .user }) {
            startIndex = messages.index(after: lastUserIndex)
        } else {
            startIndex = messages.startIndex
        }
        return messages[startIndex...]
            .filter { $0.role == .assistant }
            .map { Self.normalizedMergeText($0.text) }
    }

    private static func normalizedMergeText(_ text: String) -> String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\r\n", with: "\n")
    }

    private static func toolTraceGroupsOverlap(
        _ left: GaryxMobileToolTraceGroup,
        _ right: GaryxMobileToolTraceGroup,
        allowFingerprint: Bool
    ) -> Bool {
        let leftKeys = Set(left.entries.compactMap { toolTraceMergeKey($0, includeFingerprint: allowFingerprint) })
        let rightKeys = Set(right.entries.compactMap { toolTraceMergeKey($0, includeFingerprint: allowFingerprint) })
        return !leftKeys.isDisjoint(with: rightKeys)
    }

    private static func isInCurrentTurn(index: Int, messages: [GaryxMobileMessage]) -> Bool {
        guard let lastUserIndex = messages.lastIndex(where: { $0.role == .user }) else {
            return true
        }
        return index > lastUserIndex
    }

    private static func mergedToolTraceGroup(
        _ remote: GaryxMobileToolTraceGroup,
        with local: GaryxMobileToolTraceGroup
    ) -> GaryxMobileToolTraceGroup {
        var merged = remote
        merged.live = remote.live || local.live
        for localEntry in local.entries {
            if let localKey = toolTraceMergeKey(localEntry),
               let index = merged.entries.firstIndex(where: { toolTraceMergeKey($0) == localKey }) {
                if localEntry.status != .running {
                    merged.entries[index].absorb(result: localEntry)
                }
                continue
            }
            merged.entries.append(localEntry)
        }
        return merged
    }

    private static func toolTraceMergeKey(
        _ entry: GaryxMobileToolTraceEntry,
        includeFingerprint: Bool = true
    ) -> String? {
        if let toolUseId = entry.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !toolUseId.isEmpty {
            return "id:\(toolUseId)"
        }
        guard includeFingerprint else {
            return nil
        }
        let normalizedTool = entry.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let input = entry.inputText?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let summary = entry.summaryText?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !normalizedTool.isEmpty, !input.isEmpty || !summary.isEmpty else {
            return nil
        }
        return "fp:\(normalizedTool):\(input):\(summary):\(entry.isError)"
    }

    private static func userMergeKey(_ message: GaryxMobileMessage) -> String {
        GaryxStructuredContentRenderer.userMergeKey(
            text: message.text,
            attachments: message.attachments.map(\.contentDescriptor)
        )
    }

    private static func attachmentSummary(from attachments: [GaryxMobileMessageAttachment]) -> String? {
        GaryxStructuredContentRenderer.attachmentSummary(
            from: attachments.map(\.contentDescriptor)
        )
    }

    private static func remoteUserMessageText(text: String, imageCount: Int) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            return trimmed
        }
        if imageCount == 1 {
            return "[1 image]"
        }
        if imageCount > 1 {
            return "[\(imageCount) images]"
        }
        return "User message"
    }

    private static func visibleUserText(text: String, attachments: [GaryxMobileComposerAttachment]) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.isEmpty else {
            return text
        }
        let imageCount = attachments.filter { $0.kind == "image" || $0.mediaType.hasPrefix("image/") }.count
        let fileCount = max(attachments.count - imageCount, 0)
        var parts: [String] = []
        if imageCount > 0 {
            parts.append("\(imageCount) image\(imageCount == 1 ? "" : "s")")
        }
        if fileCount > 0 {
            parts.append("\(fileCount) file\(fileCount == 1 ? "" : "s")")
        }
        if parts.isEmpty {
            return "User message"
        }
        return "[\(parts.joined(separator: ", "))]"
    }

    private static func pendingUserInputText(
        _ input: GaryxPendingUserInput,
        attachments: [GaryxMobileMessageAttachment] = []
    ) -> String {
        let trimmed = input.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            return input.text
        }
        if !attachments.isEmpty {
            return input.content.flatMap { GaryxStructuredContentRenderer.text(from: $0) } ?? ""
        }
        if let contentSummary = input.content.flatMap({ GaryxStructuredContentRenderer.summaryText(from: $0) }),
           !contentSummary.isEmpty {
            return contentSummary
        }
        return "User message"
    }

    private static func dataUrl(mediaType: String, base64: String) -> String {
        let normalizedType = mediaType.trimmingCharacters(in: .whitespacesAndNewlines)
        let type = normalizedType.isEmpty ? "application/octet-stream" : normalizedType
        return "data:\(type);base64,\(base64)"
    }

    private static func matchedUploadPreview(
        for file: GaryxUploadedChatAttachment,
        from previews: inout [GaryxPendingUploadPreview]
    ) -> GaryxPendingUploadPreview? {
        let fileName = file.name.trimmingCharacters(in: .whitespacesAndNewlines)
        let fileMediaType = file.mediaType.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        let exactMatches = previews.indices.filter { index in
            previews[index].name == fileName
                && (fileMediaType.isEmpty || previews[index].mediaType.lowercased() == fileMediaType)
        }
        if exactMatches.count == 1 {
            return previews.remove(at: exactMatches[0])
        }

        let nameMatches = previews.indices.filter { previews[$0].name == fileName }
        if nameMatches.count == 1 {
            return previews.remove(at: nameMatches[0])
        }

        return nil
    }

    private static func messageAttachments(from attachments: [GaryxMobileComposerAttachment]) -> [GaryxMobileMessageAttachment] {
        attachments.map { attachment in
            GaryxMobileMessageAttachment(
                id: attachment.id,
                kind: attachment.kind,
                name: attachment.name,
                mediaType: attachment.mediaType,
                path: attachment.path,
                dataUrl: attachment.previewDataUrl,
                remoteUrl: nil
            )
        }
    }

    private static func transcriptStructuredContent(_ item: GaryxTranscriptMessage) -> GaryxJSONValue? {
        if let messageContent = item.message?.jsonStringDecodedIfNeeded.objectValue?["content"] {
            return messageContent.jsonStringDecodedIfNeeded
        }
        return item.content?.jsonStringDecodedIfNeeded
    }

    private static func transcriptMessageText(
        _ item: GaryxTranscriptMessage,
        attachments: [GaryxMobileMessageAttachment]
    ) -> String {
        if item.role == .user,
           !attachments.isEmpty,
           let content = transcriptStructuredContent(item) {
            return GaryxStructuredContentRenderer.text(from: content) ?? ""
        }
        return item.text
    }

    private static func messageAttachments(fromTranscript item: GaryxTranscriptMessage) -> [GaryxMobileMessageAttachment] {
        guard let content = transcriptStructuredContent(item) else { return [] }
        return messageAttachments(fromStructuredContent: content)
    }

    private static func messageAttachments(fromStructuredContent content: GaryxJSONValue?) -> [GaryxMobileMessageAttachment] {
        GaryxStructuredContentRenderer.attachments(from: content).map { attachment in
            GaryxMobileMessageAttachment(
                id: attachment.id,
                kind: attachment.kind,
                name: attachment.name,
                mediaType: attachment.mediaType,
                path: attachment.path,
                dataUrl: attachment.dataUrl,
                remoteUrl: attachment.remoteUrl
            )
        }
    }

    private func preserveRemoteBeforeIndex(from transcript: GaryxThreadTranscript) -> Int? {
        transcript.pageInfo?.returnedStartIndex ?? transcript.messages.compactMap(\.index).min()
    }

    private func mobileMessages(from transcript: GaryxThreadTranscript, threadId: String, live: Bool = false) -> [GaryxMobileMessage] {
        var rendered = mobileMessages(from: transcript.messages, live: live)
        var existingPendingIds = Set(
            rendered
                .compactMap { $0.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
        )
        for input in transcript.pendingUserInputs {
            let pendingId = input.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard input.active,
                  !pendingId.isEmpty,
                  (input.status ?? "awaiting_ack").lowercased() != "abandoned",
                  !existingPendingIds.contains(pendingId) else {
                continue
            }
            existingPendingIds.insert(pendingId)
            let attachments = Self.messageAttachments(fromStructuredContent: input.content)
            rendered.append(
                GaryxMobileMessage(
                    id: "pending-user:\(pendingId)",
                    role: .user,
                    text: Self.pendingUserInputText(input, attachments: attachments),
                    attachments: attachments,
                    timestamp: input.timestamp,
                    isStreaming: false,
                    pendingInputId: pendingId
                )
            )
        }

        if live,
           activeTasksByThread[threadId] == nil,
           let activeRun = transcript.threadRuntime?.activeRun,
           let assistantText = activeRun.assistantResponse?.trimmingCharacters(in: .whitespacesAndNewlines),
           !assistantText.isEmpty {
            let normalizedAssistantText = Self.normalizedMergeText(assistantText)
            let alreadyRendered = Self.currentTurnAssistantTexts(in: rendered).contains { normalizedExisting in
                !normalizedExisting.isEmpty
                    && (normalizedExisting.contains(normalizedAssistantText)
                        || normalizedAssistantText.contains(normalizedExisting))
            }
            if !alreadyRendered {
                let runId = activeRun.runId?.trimmingCharacters(in: .whitespacesAndNewlines)
                let stableRunId = runId.flatMap { $0.isEmpty ? nil : $0 } ?? "active"
                let assistantId = activeAssistantMessageIdsByThread[threadId]
                    ?? "stream-assistant-\(threadId)-\(stableRunId)"
                activeAssistantMessageIdsByThread[threadId] = assistantId
                rendered.append(
                    GaryxMobileMessage(
                        id: assistantId,
                        role: .assistant,
                        text: assistantText,
                        timestamp: activeRun.updatedAt,
                        isStreaming: true
                    )
                )
            }
        }

        return rendered
    }

    private func mobileMessages(from transcript: [GaryxTranscriptMessage], live: Bool = false) -> [GaryxMobileMessage] {
        var rendered: [GaryxMobileMessage] = []
        var pendingToolGroup: GaryxMobileToolTraceGroup?

        func flushToolGroup() {
            guard let group = pendingToolGroup, !group.entries.isEmpty else {
                pendingToolGroup = nil
                return
            }
            let firstEntry = group.entries[0]
            rendered.append(
                GaryxMobileMessage(
                    id: "tool-group:\(firstEntry.id)",
                    role: .tool,
                    text: group.summary,
                    timestamp: firstEntry.timestamp,
                    isStreaming: live && group.entries.contains { $0.status == .running },
                    toolTraceGroup: GaryxMobileToolTraceGroup(
                        entries: group.entries,
                        live: live && group.entries.contains { $0.status == .running }
                    )
                )
            )
            pendingToolGroup = nil
        }

        for item in transcript {
            if item.role == .toolUse || item.role == .toolResult {
                guard let entry = GaryxMobileToolTraceEntry(transcript: item) else {
                    continue
                }
                var group = pendingToolGroup ?? GaryxMobileToolTraceGroup(entries: [], live: false)
                if item.role == .toolResult, mergeToolResult(entry, into: &group) {
                    pendingToolGroup = group
                    continue
                }
                group.entries.append(entry)
                pendingToolGroup = group
                continue
            }

            flushToolGroup()

            let attachments = Self.messageAttachments(fromTranscript: item)
            let displayText = Self.transcriptMessageText(item, attachments: attachments)
            let trimmed = displayText.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty, attachments.isEmpty, item.role != .user, item.role != .assistant {
                continue
            }
            rendered.append(
                GaryxMobileMessage(
                    id: item.id,
                    role: mobileRole(for: item.role),
                    text: displayText,
                    attachments: attachments,
                    timestamp: item.timestamp,
                    isStreaming: false
                )
            )
        }

        flushToolGroup()
        return rendered
    }

    private func appendToolTraceEvent(_ eventKind: GaryxMobileToolTraceEventKind, threadId: String, message: GaryxJSONValue?) {
        removeEmptyActiveAssistantPlaceholder(for: threadId)
        activeAssistantMessageIdsByThread[threadId] = nil
        guard let entry = GaryxMobileToolTraceEntry(eventKind: eventKind, value: message) else {
            return
        }

        mutateMessages(for: threadId) { messages in
            if eventKind == .toolResult {
                for index in messages.indices.reversed() {
                    if messages[index].role == .user {
                        break
                    }
                    guard var group = messages[index].toolTraceGroup else { continue }
                    if mergeToolResult(entry, into: &group) {
                        messages[index].toolTraceGroup = group
                        messages[index].text = group.summary
                        messages[index].isStreaming = group.isActive
                        return
                    }
                }
            }

            if let index = messages.indices.last, messages[index].role == .tool, var group = messages[index].toolTraceGroup {
                group.live = true
                group.entries.append(entry)
                messages[index].toolTraceGroup = group
                messages[index].text = group.summary
                messages[index].isStreaming = group.isActive
                return
            }

            let group = GaryxMobileToolTraceGroup(entries: [entry], live: true)
            messages.append(
                GaryxMobileMessage(
                    id: "tool-group:\(entry.id)",
                    role: .tool,
                    text: group.summary,
                    timestamp: entry.timestamp,
                    isStreaming: group.isActive,
                    toolTraceGroup: group
                )
            )
        }
    }

    private func removeEmptyActiveAssistantPlaceholder(for threadId: String) {
        guard let activeAssistantMessageId = activeAssistantMessageIdsByThread[threadId] else {
            return
        }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { $0.id == activeAssistantMessageId }),
                  messages[index].role == .assistant,
                  messages[index].text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                return
            }
            messages.remove(at: index)
        }
    }

    private func mergeToolResult(
        _ result: GaryxMobileToolTraceEntry,
        into group: inout GaryxMobileToolTraceGroup
    ) -> Bool {
        if let toolUseId = result.toolUseId,
           let match = group.entries.lastIndex(where: { $0.toolUseId == toolUseId && $0.resultText == nil }) {
            group.entries[match].absorb(result: result)
            return true
        }
        if result.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false {
            return false
        }

        let fallbackMatches = group.entries.indices.filter {
            canMergeToolResultFallback(result, into: group.entries[$0])
        }
        if let match = fallbackMatches.last {
            group.entries[match].absorb(result: result)
            return true
        }

        return false
    }

    private func canMergeToolResultFallback(
        _ result: GaryxMobileToolTraceEntry,
        into candidate: GaryxMobileToolTraceEntry
    ) -> Bool {
        guard candidate.status == .running, candidate.resultText == nil else {
            return false
        }
        if let resultToolUseId = result.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !resultToolUseId.isEmpty,
           let candidateToolUseId = candidate.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !candidateToolUseId.isEmpty,
           resultToolUseId != candidateToolUseId {
            return false
        }
        let resultTool = result.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let candidateTool = candidate.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if !resultTool.isEmpty, resultTool == candidateTool {
            return true
        }
        if candidateTool == "tool" || resultTool == "tool" {
            return true
        }
        if result.title.caseInsensitiveCompare(candidate.title) == .orderedSame {
            return true
        }
        if let resultSummary = result.summaryText,
           let candidateSummary = candidate.summaryText,
           resultSummary == candidateSummary {
            return true
        }
        return false
    }

    private func ensureSelectedThread() async throws -> GaryxThreadSummary {
        if let selectedThread {
            return selectedThread
        }
        let pendingWorkspace = pendingBotWorkspace?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let pendingAgentId = pendingBotAgentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let workspace = pendingWorkspace.isEmpty
            ? newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
            : pendingWorkspace
        let agentId = pendingAgentId.isEmpty
            ? selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
            : pendingAgentId
        let workspaceMode = pendingWorkspace.isEmpty ? workspaceModeForNewThread(workspace: workspace) : "local"
        let thread = try await client().createThread(
            GaryxCreateThreadRequest(
                workspaceDir: workspace.isEmpty ? nil : workspace,
                workspaceMode: workspaceMode,
                agentId: agentId.isEmpty ? nil : agentId,
                metadata: ["client": "garyx-mobile"]
            )
        )
        selectedThread = thread
        draftThreadTitle = thread.title
        threads.insert(thread, at: 0)
        if let pendingBotId = pendingBotId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !pendingBotId.isEmpty {
            _ = try await client().bindBot(botId: pendingBotId, threadId: thread.id)
            clearPendingBotDraft()
            await refreshRemoteState()
        }
        return thread
    }

    private func startGlobalEventStream() {
        guard hasGatewaySettings, canConnectGateway else { return }
        globalEventStreamTask?.cancel()
        let generation = UUID()
        globalEventStreamGeneration = generation
        globalEventStreamActive = false
        globalEventStreamTask = Task { [weak self] in
            guard let self else { return }
            await self.runGlobalEventStream(generation: generation)
        }
    }

    private func cancelGlobalEventStream() {
        globalEventStreamTask?.cancel()
        globalEventStreamTask = nil
        globalEventStreamGeneration = nil
        globalEventStreamActive = false
    }

    private func startGatewayReconnectLoop(immediate: Bool = false) {
        guard hasGatewaySettings, canConnectGateway else { return }
        guard gatewayReconnectTask == nil else { return }
        let generation = UUID()
        gatewayReconnectGeneration = generation
        gatewayReconnectTask = Task { [weak self] in
            guard let self else { return }
            var delay = immediate ? UInt64(0) : Self.gatewayReconnectInitialDelayNanos
            while !Task.isCancelled {
                if delay > 0 {
                    try? await Task.sleep(nanoseconds: delay)
                }
                if Task.isCancelled { break }
                guard gatewayReconnectGeneration == generation,
                      hasGatewaySettings,
                      canConnectGateway else { break }
                if case .ready = connectionState {
                    break
                }
                await connectAndRefresh(scheduleReconnectOnFailure: false)
                guard gatewayReconnectGeneration == generation else { break }
                if case .ready = connectionState {
                    break
                }
                delay = delay == 0
                    ? Self.gatewayReconnectInitialDelayNanos
                    : min(delay * 2, Self.gatewayReconnectMaxDelayNanos)
            }
            if gatewayReconnectGeneration == generation {
                gatewayReconnectTask = nil
                gatewayReconnectGeneration = nil
            }
        }
    }

    private func runGlobalEventStream(generation: UUID) async {
        var retryDelay: UInt64 = 1_000_000_000
        while !Task.isCancelled, hasGatewaySettings {
            guard globalEventStreamGeneration == generation else { break }
            do {
                let request = try client().eventStreamRequest(historyLimit: 50)
                let (bytes, response) = try await URLSession.shared.bytes(for: request)
                guard let http = response as? HTTPURLResponse,
                      (200..<300).contains(http.statusCode) else {
                    throw GaryxGatewayError.invalidHTTPResponse
                }
                guard globalEventStreamGeneration == generation else { break }
                globalEventStreamActive = true
                retryDelay = 1_000_000_000
                var dataLines: [String] = []
                for try await line in bytes.lines {
                    if Task.isCancelled { break }
                    guard globalEventStreamGeneration == generation else { break }
                    if line.isEmpty {
                        if !dataLines.isEmpty {
                            await handleGlobalEventStreamPayload(dataLines.joined(separator: "\n"))
                            dataLines.removeAll()
                        }
                        continue
                    }
                    if line.hasPrefix(":") {
                        continue
                    }
                    guard line.hasPrefix("data:") else {
                        continue
                    }
                    var value = String(line.dropFirst(5))
                    if value.hasPrefix(" ") {
                        value.removeFirst()
                    }
                    dataLines.append(value)
                }
            } catch {
                if !Task.isCancelled, globalEventStreamGeneration == generation {
                    globalEventStreamActive = false
                    if case .ready = connectionState {
                        gatewaySettingsStatus = "Live updates disconnected"
                    }
                    await refreshThreads()
                    if selectedThread != nil {
                        await loadSelectedThreadHistory()
                    }
                }
            }
            if globalEventStreamGeneration == generation {
                globalEventStreamActive = false
            }
            if Task.isCancelled { break }
            try? await Task.sleep(nanoseconds: retryDelay)
            retryDelay = min(retryDelay * 2, 10_000_000_000)
            if globalEventStreamGeneration == generation, case .ready = connectionState {
                gatewaySettingsStatus = nil
            }
        }
        if globalEventStreamGeneration == generation {
            globalEventStreamActive = false
        }
    }

    private func handleGlobalEventStreamPayload(_ payload: String, replay: Bool = false) async {
        let trimmed = payload.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if let data = trimmed.data(using: .utf8),
           let object = try? JSONSerialization.jsonObject(with: data),
           let dictionary = object as? [String: Any],
            let type = dictionary["type"] as? String {
            if type == "history" {
                let events = dictionary["events"] as? [String] ?? []
                var shouldReloadSelectedHistory = false
                var titleUpdates: [(threadId: String, title: String)] = []
                for eventPayload in events {
                    if let event = try? client().decodeStreamEvent(eventPayload) {
                        if case .threadTitleUpdated(_, let threadId, let title) = event {
                            titleUpdates.append((threadId: threadId, title: title))
                        }
                        if selectedThread?.id == Self.threadId(from: event) {
                            switch event {
                            case .done, .runComplete, .error, .interrupt:
                                shouldReloadSelectedHistory = true
                            default:
                                break
                            }
                        }
                    }
                    await handleGlobalEventStreamPayload(eventPayload, replay: true)
                }
                await refreshThreads()
                for update in titleUpdates {
                    applyThreadTitleUpdate(threadId: update.threadId, title: update.title)
                }
                if shouldReloadSelectedHistory {
                    await loadSelectedThreadHistory()
                }
                return
            }
            if type == "snapshot", dictionary["thread_id"] == nil, dictionary["threadId"] == nil {
                return
            }
        }
        guard let event = try? client().decodeStreamEvent(trimmed) else {
            return
        }
        await handleGlobalStreamEvent(event, replay: replay)
    }

    private func handleGlobalStreamEvent(_ event: GaryxChatStreamEvent, replay: Bool = false) async {
        let threadId = Self.threadId(from: event)
        guard !threadId.isEmpty else { return }

        if replay {
            updateRemoteBusyState(from: event)
            switch event {
            case .threadTitleUpdated(_, let threadId, let title):
                applyThreadTitleUpdate(threadId: threadId, title: title)
            default:
                break
            }
            return
        }

        if activeTasksByThread[threadId] != nil {
            updateRemoteBusyState(from: event)
            if case .threadTitleUpdated(_, let eventThreadId, let title) = event {
                applyThreadTitleUpdate(threadId: eventThreadId, title: title)
            }
            return
        }

        let assistantMessageId = activeAssistantMessageIdsByThread[threadId]
            ?? "stream-assistant-\(threadId)-\(UUID().uuidString)"
        handle(event, threadId: threadId, assistantMessageId: assistantMessageId, affectsActiveRun: true)

        switch event {
        case .threadTitleUpdated(_, let threadId, let title):
            applyThreadTitleUpdate(threadId: threadId, title: title)
        case .done, .runComplete:
            await refreshThreads()
            if selectedThread?.id == threadId {
                await loadSelectedThreadHistory()
            }
        case .error(_, _, let error):
            if Self.isTransientGatewayErrorMessage(error), selectedThread?.id == threadId {
                await loadSelectedThreadHistory()
            }
        default:
            break
        }
    }

    private func cancelActiveSocket() {
        for threadId in Array(pendingAssistantDeltasByThread.keys) {
            flushPendingAssistantDelta(for: threadId)
        }
        for task in activeReaderTasksByThread.values {
            task.cancel()
        }
        for task in activeTasksByThread.values {
            task.cancel(with: .goingAway, reason: nil)
        }
        activeReaderTasksByThread = [:]
        activeTasksByThread = [:]
        activeTask = nil
        activeReaderTask = nil
        if let activeRunThreadId {
            activeAssistantMessageIdsByThread[activeRunThreadId] = nil
        }
        activeRunThreadId = nil
        isSending = false
    }

    private func cancelActiveSocket(for threadId: String) {
        flushPendingAssistantDelta(for: threadId)
        activeReaderTasksByThread[threadId]?.cancel()
        activeReaderTasksByThread[threadId] = nil
        activeTasksByThread[threadId]?.cancel(with: .goingAway, reason: nil)
        activeTasksByThread[threadId] = nil
        activeAssistantMessageIdsByThread[threadId] = nil
        if activeRunThreadId == threadId {
            activeRunThreadId = activeTasksByThread.keys.first
        }
        activeTask = activeRunThreadId.flatMap { activeTasksByThread[$0] }
        activeReaderTask = activeRunThreadId.flatMap { activeReaderTasksByThread[$0] }
        isSending = !activeTasksByThread.isEmpty
    }

    private func clearActiveRun(task: URLSessionWebSocketTask?, threadId: String?) {
        let resolvedThreadId: String?
        if let threadId {
            resolvedThreadId = threadId
        } else if let task {
            resolvedThreadId = activeTasksByThread.first(where: { $0.value === task })?.key
        } else {
            resolvedThreadId = nil
        }

        guard let resolvedThreadId else {
            cancelActiveSocket()
            return
        }

        if let task,
           let current = activeTasksByThread[resolvedThreadId],
           current !== task {
            return
        }

        flushPendingAssistantDelta(for: resolvedThreadId)
        activeReaderTasksByThread[resolvedThreadId]?.cancel()
        activeReaderTasksByThread[resolvedThreadId] = nil
        activeTasksByThread[resolvedThreadId] = nil
        activeAssistantMessageIdsByThread[resolvedThreadId] = nil
        if activeRunThreadId == resolvedThreadId {
            activeRunThreadId = activeTasksByThread.keys.first
        }
        activeTask = activeRunThreadId.flatMap { activeTasksByThread[$0] }
        activeReaderTask = activeRunThreadId.flatMap { activeReaderTasksByThread[$0] }
        isSending = !activeTasksByThread.isEmpty
        cancelSelectedThreadRecoveryIfNeeded(threadId: resolvedThreadId)
    }

    private func cancelSelectedThreadRecoveryIfNeeded(threadId: String) {
        guard selectedThreadRecoveryThreadId == threadId else { return }
        selectedThreadRecoveryTask?.cancel()
        selectedThreadRecoveryTask = nil
        selectedThreadRecoveryThreadId = nil
    }

    private static func localChatTimestamp() -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: Date())
    }

    private static func isTransientGatewayErrorMessage(_ message: String) -> Bool {
        let normalized = message.lowercased()
        return normalized.contains("timed out")
            || normalized.contains("timeout")
            || normalized.contains("network connection was lost")
            || normalized.contains("not connected to the internet")
            || normalized.contains("connection reset")
            || normalized.contains("connection closed")
            || normalized.contains("websocket")
            || normalized.contains("socket")
            || normalized.contains("gateway unavailable")
            || normalized.contains("bad gateway")
            || normalized.contains("service unavailable")
    }

    private static func isSuccessfulStreamInputStatus(_ status: String) -> Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "queued"
            || normalized == "accepted"
            || normalized == "ok"
            || normalized == "success"
    }

    private static func shouldFallbackStreamInputStatus(_ status: String) -> Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "no_active_session"
            || normalized == "no active session"
            || normalized == "inactive"
            || normalized == "closed"
            || normalized == "not_found"
    }

    private static func threadId(from event: GaryxChatStreamEvent) -> String {
        switch event {
        case .accepted(_, let threadId),
             .assistantDelta(_, let threadId, _, _),
             .assistantBoundary(_, let threadId),
             .toolUse(_, let threadId, _),
             .toolResult(_, let threadId, _),
             .userMessage(_, let threadId, _, _),
             .userAck(_, let threadId, _),
             .threadTitleUpdated(_, let threadId, _),
             .done(_, let threadId),
             .runComplete(_, let threadId),
             .streamInput(_, let threadId, _, _),
             .interrupt(_, let threadId, _),
             .snapshot(let threadId, _),
             .error(_, let threadId, _):
            return threadId
        case .ping, .unknown:
            return ""
        }
    }

    private func ensureSelectedAgentTarget() {
        let targets = agentTargets
        if targets.contains(where: { $0.id == selectedAgentTargetId }) {
            return
        }
        if let first = targets.first {
            setSelectedAgentTarget(first.id)
        }
    }

    private func ensureSelectedWorkspace() {
        let paths = knownWorkspacePaths
        if !selectedWorkspacePath.isEmpty, paths.contains(selectedWorkspacePath) {
            draftWorkspacePath = selectedWorkspacePath
            return
        }
        selectedWorkspacePath = paths.first ?? ""
        draftWorkspacePath = selectedWorkspacePath
    }

    private func mergeMissingSidebarRequiredThreads(
        using gatewayClient: GaryxGatewayClient,
        extraThreadIds: [String?] = [],
        runtimeGeneration: UUID? = nil
    ) async {
        let observedGeneration = runtimeGeneration ?? gatewayRuntimeGeneration
        let requiredThreadIds = sidebarRequiredThreadIds(
            pinnedThreadIds: pinnedThreadIds,
            extraThreadIds: extraThreadIds
        )
        let missingThreads = await fetchMissingThreadSummaries(
            using: gatewayClient,
            requiredThreadIds: requiredThreadIds,
            existingThreadIds: Set(threads.map(\.id))
        )
        guard observedGeneration == gatewayRuntimeGeneration else { return }
        if !missingThreads.isEmpty {
            threads = Self.mergedThreadSummaries(threads + missingThreads)
        }
    }

    private func fetchMissingThreadSummaries(
        using gatewayClient: GaryxGatewayClient,
        requiredThreadIds: [String],
        existingThreadIds: Set<String>
    ) async -> [GaryxThreadSummary] {
        var visibleThreadIds = existingThreadIds
        var missingThreads: [GaryxThreadSummary] = []
        for threadId in requiredThreadIds where !visibleThreadIds.contains(threadId) {
            if let thread = try? await gatewayClient.getThread(threadId: threadId) {
                missingThreads.append(thread)
                visibleThreadIds.insert(thread.id)
            }
        }
        return missingThreads
    }

    private func normalizedThreadIds(_ values: [String?]) -> [String] {
        var seen = Set<String>()
        return values.compactMap { value -> String? in
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard !trimmed.isEmpty, seen.insert(trimmed).inserted else { return nil }
            return trimmed
        }
    }

    private func sidebarRequiredThreadIds(
        pinnedThreadIds: [String],
        extraThreadIds: [String?] = []
    ) -> [String] {
        var seen = Set<String>()
        var ids: [String] = []

        func append(_ value: String?) {
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard !trimmed.isEmpty, seen.insert(trimmed).inserted else { return }
            ids.append(trimmed)
        }

        pinnedThreadIds.forEach { append($0) }
        extraThreadIds.forEach { append($0) }
        channelEndpoints.forEach { append($0.threadId) }
        configuredBots.forEach { bot in
            append(bot.mainThreadId)
            append(bot.defaultOpenThreadId)
        }
        botConsoles.forEach { console in
            append(console.mainThreadId)
            append(console.defaultOpenThreadId)
            console.conversationNodes.forEach { append($0.endpoint.threadId) }
        }

        return ids
    }

    private static func mergedThreadSummaries(_ values: [GaryxThreadSummary]) -> [GaryxThreadSummary] {
        var indexesById: [String: Int] = [:]
        var merged: [GaryxThreadSummary] = []
        for value in values {
            guard !value.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                continue
            }
            if let index = indexesById[value.id] {
                merged[index] = value
            } else {
                indexesById[value.id] = merged.count
                merged.append(value)
            }
        }
        return merged
    }

    private func refreshProviderModelsForVisibleAgents(runtimeGeneration: UUID? = nil) async {
        let providerTypes = Set(agents.map(\.providerType).filter { !$0.isEmpty })
        for providerType in providerTypes where providerModelsByType[providerType] == nil {
            await loadProviderModels(providerType: providerType, runtimeGeneration: runtimeGeneration)
        }
    }

    private func replaceAgent(_ agent: GaryxAgentSummary) {
        if let index = agents.firstIndex(where: { $0.id == agent.id }) {
            agents[index] = agent
        } else {
            agents.insert(agent, at: 0)
        }
    }

    private func replaceTeam(_ team: GaryxTeamSummary) {
        if let index = teams.firstIndex(where: { $0.id == team.id }) {
            teams[index] = team
        } else {
            teams.insert(team, at: 0)
        }
    }

    private static func normalizedTeamMemberIds(_ rawValue: String, leaderAgentId: String) -> [String] {
        let leader = leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        var ids: [String] = leader.isEmpty ? [] : [leader]
        for token in rawValue.split(whereSeparator: { $0 == "," || $0 == "\n" || $0 == " " }) {
            let id = String(token).trimmingCharacters(in: .whitespacesAndNewlines)
            if !id.isEmpty, !ids.contains(id) {
                ids.append(id)
            }
        }
        return ids
    }

    private func replaceAutomation(_ automation: GaryxAutomationSummary) {
        if let index = automations.firstIndex(where: { $0.id == automation.id }) {
            automations[index] = automation
        } else {
            automations.insert(automation, at: 0)
        }
    }

    private func replaceSkill(_ skill: GaryxSkillSummary) {
        if let index = skills.firstIndex(where: { $0.id == skill.id }) {
            skills[index] = skill
        } else {
            skills.insert(skill, at: 0)
        }
    }

    private func replaceSlashCommand(_ command: GaryxSlashCommand, previousName: String? = nil) {
        if let previousName, previousName != command.name {
            slashCommands.removeAll { $0.name == previousName }
        }
        if let index = slashCommands.firstIndex(where: { $0.name == command.name }) {
            slashCommands[index] = command
        } else {
            slashCommands.append(command)
        }
        slashCommands.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private func replaceMcpServer(_ server: GaryxMcpServer, previousName: String? = nil) {
        if let previousName, previousName != server.name {
            mcpServers.removeAll { $0.name == previousName }
        }
        if let index = mcpServers.firstIndex(where: { $0.name == server.name }) {
            mcpServers[index] = server
        } else {
            mcpServers.append(server)
        }
        mcpServers.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    private func replaceAutoResearchRun(_ run: GaryxAutoResearchRun) {
        if let index = autoResearchRuns.firstIndex(where: { $0.runId == run.runId }) {
            autoResearchRuns[index] = run
        } else {
            autoResearchRuns.insert(run, at: 0)
        }
        if var detail = autoResearchDetailsByRunId[run.runId] {
            detail.run = run
            autoResearchDetailsByRunId[run.runId] = detail
        }
    }

    private func splitShellLikeList(_ value: String) -> [String] {
        value
            .split { $0 == "," || $0 == "\n" }
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    private func keyValueDictionary(from value: String) -> [String: String] {
        var result: [String: String] = [:]
        for line in value.split(whereSeparator: \.isNewline) {
            let text = String(line).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { continue }
            let parts = text.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            guard let key = parts.first.map(String.init)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !key.isEmpty else {
                continue
            }
            let rawValue = parts.dropFirst().first.map(String.init) ?? ""
            result[key] = rawValue.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return result
    }

    private func client() throws -> GaryxGatewayClient {
        let normalized = normalizedGatewayURL(gatewayURL)
        guard let url = parsedGatewayURL(from: normalized) else {
            throw GaryxGatewayError.invalidURL(normalized)
        }
        return GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: url,
                authToken: gatewayAuthToken
            )
        )
    }

    private func parsedGatewayURL(from value: String) -> URL? {
        let normalized = normalizedGatewayURL(value)
        guard !normalized.isEmpty else { return nil }
        guard
            let components = URLComponents(string: normalized),
            let scheme = components.scheme?.lowercased(),
            scheme == "http" || scheme == "https",
            let host = components.host,
            !host.isEmpty,
            let url = components.url
        else {
            return nil
        }
        return url
    }

    private func normalizedGatewayURL(_ value: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return trimmed }
        if trimmed.hasPrefix("http://") || trimmed.hasPrefix("https://") {
            return trimmed.replacingOccurrences(
                of: "/+$",
                with: "",
                options: .regularExpression
            )
        }
        let withoutTrailingSlash = trimmed.replacingOccurrences(
            of: "/+$",
            with: "",
            options: .regularExpression
        )
        return "http://\(withoutTrailingSlash)"
    }

    private func mobileRole(for role: GaryxTranscriptRole) -> GaryxMobileMessage.Role {
        switch role {
        case .assistant:
            .assistant
        case .user:
            .user
        case .toolUse, .toolResult:
            .tool
        case .system, .unknown:
            .system
        }
    }

    private func displayMessage(for error: Error) -> String {
        if Self.isCancellationError(error) {
            return ""
        }
        if let localized = (error as? LocalizedError)?.errorDescription, !localized.isEmpty {
            return localized
        }
        return error.localizedDescription
    }

    private static func presentableErrorMessage(_ message: String?) -> String? {
        let trimmed = message?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !trimmed.isEmpty else { return nil }
        guard !isCancellationMessage(trimmed) else { return nil }
        return trimmed
    }

    private static func isCancellationError(_ error: Error) -> Bool {
        if error is CancellationError {
            return true
        }
        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain && nsError.code == NSURLErrorCancelled {
            return true
        }
        return isCancellationMessage(error.localizedDescription)
    }

    private static func isCancellationMessage(_ message: String) -> Bool {
        let normalized = message
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        return normalized == "cancel"
            || normalized == "cancel."
            || normalized == "cancelled"
            || normalized == "canceled"
            || normalized == "cancelled."
            || normalized == "canceled."
            || normalized == "the operation was cancelled."
            || normalized == "the operation was canceled."
            || normalized == "the operation couldn’t be completed. (nsurlerrordomain error -999.)"
            || normalized == "the operation could not be completed. (nsurlerrordomain error -999.)"
    }
}

private enum GaryxMobileToolTraceEventKind {
    case toolUse
    case toolResult
}

private struct GaryxMobileToolTracePayload {
    var toolUseId: String?
    var parentToolUseId: String?
    var toolName: String?
    var contentText: String?
    var summaryText: String?
    var timestamp: String?
    var primaryPathBadge: String?
    var source: String?
    var itemType: String?
    var isError: Bool

    static func fromEvent(_ value: GaryxJSONValue?, eventKind: GaryxMobileToolTraceEventKind) -> GaryxMobileToolTracePayload {
        from(value: value, eventKind: eventKind, fallbackText: nil, fallbackToolName: nil, fallbackTimestamp: nil)
    }

    static func fromTranscript(_ message: GaryxTranscriptMessage) -> GaryxMobileToolTracePayload {
        let eventKind: GaryxMobileToolTraceEventKind = message.role == .toolResult ? .toolResult : .toolUse
        return from(
            value: message.message ?? message.content ?? GaryxJSONValue.decoded(from: message.text),
            eventKind: eventKind,
            fallbackText: message.text,
            fallbackToolName: message.kind,
            fallbackTimestamp: message.timestamp
        )
    }

    private static func from(
        value: GaryxJSONValue?,
        eventKind: GaryxMobileToolTraceEventKind,
        fallbackText: String?,
        fallbackToolName: String?,
        fallbackTimestamp: String?
    ) -> GaryxMobileToolTracePayload {
        let decodedValue = value?.jsonStringDecodedIfNeeded
        guard let object = decodedValue?.objectValue else {
            return GaryxMobileToolTracePayload(
                toolUseId: nil,
                parentToolUseId: nil,
                toolName: fallbackToolName?.garyxTrimmedNilIfEmpty,
                contentText: fallbackText?.garyxTrimmedNilIfEmpty,
                summaryText: fallbackText?.garyxSafeToolSummary,
                timestamp: fallbackTimestamp,
                primaryPathBadge: nil,
                source: nil,
                itemType: fallbackToolName?.garyxTrimmedNilIfEmpty,
                isError: false
            )
        }

        let payloadValue = object.unwrappedToolPayloadValue ?? decodedValue ?? .object(object)
        let payloadObject = payloadValue.objectValue
        let nestedContent = payloadObject ?? object.objectValue(forKeys: ["content", "message", "payload"])
        let metadata = object.objectValue(forKeys: ["metadata"])
            ?? payloadObject?.objectValue(forKeys: ["metadata"])
            ?? nestedContent?.objectValue(forKeys: ["metadata"])
        let source = metadata?.stringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
            ?? object.stringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
            ?? payloadObject?.stringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
            ?? nestedContent?.stringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
        let toolUseId = object.stringValue(forKeys: ["toolUseId", "tool_use_id", "id"])
            ?? payloadObject?.stringValue(forKeys: ["toolUseId", "tool_use_id", "id"])
            ?? nestedContent?.stringValue(forKeys: ["toolUseId", "tool_use_id", "id"])
        let parentToolUseId = object.stringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? payloadObject?.stringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? nestedContent?.stringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? metadata?.stringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
        let toolName = object.stringValue(forKeys: ["toolName", "tool_name", "name", "tool", "title"])
            ?? payloadObject?.stringValue(forKeys: ["toolName", "tool_name", "name", "tool", "title", "type"])
            ?? nestedContent?.stringValue(forKeys: ["toolName", "tool_name", "name", "tool", "title"])
            ?? fallbackToolName?.garyxTrimmedNilIfEmpty
        let itemType = object.stringValue(forKeys: ["type", "item_type", "itemType"])
            ?? payloadObject?.stringValue(forKeys: ["type", "item_type", "itemType"])
            ?? nestedContent?.stringValue(forKeys: ["type", "item_type", "itemType"])
            ?? metadata?.stringValue(forKeys: ["type", "item_type", "itemType"])
            ?? toolName
        let detailKeys = eventKind == .toolUse
            ? ["input", "arguments", "params", "content", "command", "path", "file_path", "text"]
            : ["result", "output", "content", "stdout", "stderr", "text", "message"]
        let content = payloadObject?.detailText(forKeys: detailKeys)
            ?? object.detailText(forKeys: detailKeys)
            ?? fallbackText?.garyxTrimmedNilIfEmpty
        let summary = Self.summaryText(
            toolName: toolName,
            payload: payloadObject,
            payloadValue: payloadValue,
            eventKind: eventKind
        ) ?? fallbackText?.garyxSafeToolSummary
        let timestamp = object.stringValue(forKeys: ["timestamp", "createdAt", "created_at"]) ?? fallbackTimestamp
        let primaryPathBadge = Self.primaryPathBadge(
            payload: payloadObject,
            nestedContent: nestedContent
        )
        let isError = object.boolValue(forKeys: ["isError", "is_error", "error"])
            ?? payloadObject?.boolValue(forKeys: ["isError", "is_error", "error"])
            ?? nestedContent?.boolValue(forKeys: ["isError", "is_error", "error"])
            ?? false

        return GaryxMobileToolTracePayload(
            toolUseId: toolUseId,
            parentToolUseId: parentToolUseId,
            toolName: toolName,
            contentText: content,
            summaryText: summary,
            timestamp: timestamp,
            primaryPathBadge: primaryPathBadge,
            source: source,
            itemType: itemType,
            isError: isError
        )
    }

    private static func primaryPathBadge(
        payload: [String: GaryxJSONValue]?,
        nestedContent: [String: GaryxJSONValue]?
    ) -> String? {
        let input = payload?.objectValue(forKeys: ["input", "arguments", "params"])
            ?? nestedContent?.objectValue(forKeys: ["input", "arguments", "params"])
            ?? payload
            ?? nestedContent
        return input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"])
            .map { $0.garyxPathTail }
    }

    private static func summaryText(
        toolName: String?,
        payload: [String: GaryxJSONValue]?,
        payloadValue: GaryxJSONValue,
        eventKind: GaryxMobileToolTraceEventKind
    ) -> String? {
        let normalizedTool = toolName?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        let input = payload?.objectValue(forKeys: ["input", "arguments", "params"]) ?? payload
        let result = payload?.objectValue(forKeys: ["result", "output"]) ?? payload

        if eventKind == .toolResult {
            let text = result?.stringValue(forKeys: ["summary", "message", "text", "stdout", "stderr"])
                ?? payload?.stringValue(forKeys: ["summary", "message", "text", "stdout", "stderr"])
            return text?.garyxSafeToolSummary
        }

        switch normalizedTool {
        case "bash", "shell", "exec_command", "command", "commandexecution":
            return input?.stringValue(forKeys: ["description"])
                ?? input?.stringValue(forKeys: ["command", "cmd"])
                    .map { $0.garyxShellSummary }
        case "read", "view", "open", "cat":
            return input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"])
                .map { "read \($0.garyxPathTail)" }
        case "write", "create":
            return input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"])
                .map { "write \($0.garyxPathTail)" }
        case "edit", "multiedit", "apply_patch":
            return input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"])
                .map { "edit \($0.garyxPathTail)" }
        case "grep", "search", "rg":
            let pattern = input?.stringValue(forKeys: ["pattern", "query"])
            let path = input?.stringValue(forKeys: ["path", "include", "glob"])
            if let pattern, let path {
                return "search \(pattern) in \(path.garyxPathTail)"
            }
            return pattern.map { "search \($0)" }
        case "glob", "find":
            return input?.stringValue(forKeys: ["pattern", "path"])
                .map { "find \($0.garyxPathTail)" }
        case "ls", "list":
            return input?.stringValue(forKeys: ["path", "directory"])
                .map { "list \($0.garyxPathTail)" } ?? "list files"
        case "todowrite", "todo_write":
            if let todos = input?["todos"]?.arrayValue, !todos.isEmpty {
                return "\(todos.count) todo items"
            }
            return nil
        case "webfetch", "web_fetch":
            return input?.stringValue(forKeys: ["url"])
                .flatMap { URL(string: $0)?.host }
                .map { "fetch \($0)" }
        case "websearch", "web_search":
            return input?.stringValue(forKeys: ["query"]).map { "search web for \($0)" }
        default:
            if let path = input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"]) {
                return path.garyxPathTail
            }
            if let command = input?.stringValue(forKeys: ["command", "cmd"]) {
                return command.garyxShellSummary
            }
            if case .string(let text) = payloadValue {
                return text.garyxSafeToolSummary
            }
            return nil
        }
    }
}

private extension GaryxMobileToolTraceEntry {
    init?(transcript message: GaryxTranscriptMessage) {
        let payload = GaryxMobileToolTracePayload.fromTranscript(message)
        guard payload.shouldRender else {
            return nil
        }
        let eventKind: GaryxMobileToolTraceEventKind = message.role == .toolResult ? .toolResult : .toolUse
        self.init(
            id: "\(message.id):\(eventKind.idSuffix)",
            toolUseId: payload.toolUseId,
            parentToolUseId: payload.parentToolUseId,
            toolName: payload.normalizedToolName,
            title: GaryxMobileToolTraceEntry.title(for: payload.normalizedToolName),
            inputText: eventKind == .toolUse ? payload.contentText : nil,
            resultText: eventKind == .toolResult ? payload.contentText : nil,
            summaryText: payload.summaryText,
            inputLabel: "Call",
            resultLabel: "Result",
            status: eventKind == .toolResult ? (payload.isError ? .failed : .completed) : .running,
            isError: payload.isError,
            timestamp: payload.timestamp,
            primaryPathBadge: payload.primaryPathBadge
        )
    }

    init?(eventKind: GaryxMobileToolTraceEventKind, value: GaryxJSONValue?) {
        let payload = GaryxMobileToolTracePayload.fromEvent(value, eventKind: eventKind)
        guard payload.shouldRender else {
            return nil
        }
        let generatedId = payload.toolUseId ?? UUID().uuidString
        self.init(
            id: "\(eventKind.idSuffix):\(generatedId):\(UUID().uuidString)",
            toolUseId: payload.toolUseId,
            parentToolUseId: payload.parentToolUseId,
            toolName: payload.normalizedToolName,
            title: GaryxMobileToolTraceEntry.title(for: payload.normalizedToolName),
            inputText: eventKind == .toolUse ? payload.contentText : nil,
            resultText: eventKind == .toolResult ? payload.contentText : nil,
            summaryText: payload.summaryText,
            inputLabel: "Call",
            resultLabel: "Result",
            status: eventKind == .toolUse ? .running : (payload.isError ? .failed : .completed),
            isError: payload.isError,
            timestamp: payload.timestamp,
            primaryPathBadge: payload.primaryPathBadge
        )
    }

    static func title(for toolName: String) -> String {
        switch toolName.lowercased() {
        case "exec_command", "command":
            return "Command"
        case "write_stdin":
            return "Input"
        case "apply_patch":
            return "Edit"
        case "view_image":
            return "Image"
        case "read_mcp_resource":
            return "MCP resource"
        case "list_mcp_resources":
            return "MCP resources"
        default:
            let words = toolName
                .replacingOccurrences(of: "-", with: "_")
                .split(separator: "_")
                .map { $0.capitalized }
            return words.isEmpty ? "Tool" : words.joined(separator: " ")
        }
    }
}

private extension GaryxMobileToolTracePayload {
    var shouldRender: Bool {
        let normalizedSource = source?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let isCodex = normalizedSource == "codex" || normalizedSource == "codex_app_server"
        let normalizedItemType = itemType?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let normalizedToolName = toolName?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return !(isCodex && (normalizedItemType == "reasoning" || normalizedToolName == "reasoning"))
    }

    var normalizedToolName: String {
        toolName?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased().nilIfEmpty ?? "tool"
    }
}

private extension GaryxMobileToolTraceEventKind {
    var idSuffix: String {
        switch self {
        case .toolUse:
            "tool-use"
        case .toolResult:
            "tool-result"
        }
    }
}

private extension GaryxJSONValue {
    static func decoded(from text: String) -> GaryxJSONValue? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{") || trimmed.hasPrefix("[") else { return nil }
        return try? JSONDecoder().decode(GaryxJSONValue.self, from: Data(trimmed.utf8))
    }

    var objectValue: [String: GaryxJSONValue]? {
        if case .object(let value) = self {
            return value
        }
        return nil
    }

    var arrayValue: [GaryxJSONValue]? {
        if case .array(let value) = self {
            return value
        }
        return nil
    }

    var jsonStringDecodedIfNeeded: GaryxJSONValue {
        if case .string(let value) = self,
           let decoded = GaryxJSONValue.decoded(from: value) {
            return decoded
        }
        return self
    }

    var stringValue: String? {
        switch self {
        case .string(let value):
            return value.garyxTrimmedNilIfEmpty
        case .number(let value):
            if value.rounded() == value {
                return String(Int(value))
            }
            return String(value).garyxTrimmedNilIfEmpty
        case .bool(let value):
            return value ? "true" : "false"
        case .null:
            return nil
        case .array, .object:
            return prettyPrinted
        }
    }

    var boolValue: Bool? {
        switch self {
        case .bool(let value):
            return value
        case .string(let value):
            let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if ["true", "yes", "1"].contains(normalized) {
                return true
            }
            if ["false", "no", "0"].contains(normalized) {
                return false
            }
            return nil
        default:
            return nil
        }
    }

    var prettyPrinted: String {
        if case .string(let value) = self {
            return value
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(self),
              let text = String(data: data, encoding: .utf8) else {
            return ""
        }
        return text
    }

    var isMeaningful: Bool {
        switch self {
        case .null:
            false
        case .string(let value):
            !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        case .array(let values):
            !values.isEmpty
        case .object(let values):
            !values.isEmpty
        case .number, .bool:
            true
        }
    }
}

private extension Dictionary where Key == String, Value == GaryxJSONValue {
    var unwrappedToolPayloadValue: GaryxJSONValue? {
        guard let content = self["content"]?.jsonStringDecodedIfNeeded else { return nil }
        let hasEnvelopeMarkers = self["toolName"] != nil
            || self["tool_name"] != nil
            || self["toolUseId"] != nil
            || self["tool_use_id"] != nil
            || self["metadata"] != nil
            || self["role"] != nil
        return hasEnvelopeMarkers ? content : nil
    }

    func stringValue(forKeys keys: [String]) -> String? {
        for key in keys {
            if let value = self[key]?.stringValue?.garyxTrimmedNilIfEmpty {
                return value
            }
        }
        return nil
    }

    func boolValue(forKeys keys: [String]) -> Bool? {
        for key in keys {
            if let value = self[key]?.boolValue {
                return value
            }
        }
        return nil
    }

    func objectValue(forKeys keys: [String]) -> [String: GaryxJSONValue]? {
        for key in keys {
            if let value = self[key]?.objectValue {
                return value
            }
        }
        return nil
    }

    func detailText(forKeys keys: [String]) -> String? {
        for key in keys {
            guard let value = self[key], value.isMeaningful else { continue }
            if key == "message", value.objectValue != nil {
                continue
            }
            if let text = value.stringValue?.garyxTrimmedNilIfEmpty {
                return text
            }
        }
        return nil
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }

    var garyxTrimmedNilIfEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    func garyxSingleLineTruncated(limit: Int) -> String {
        let normalized = replacingOccurrences(of: "\r", with: "\n")
            .split(whereSeparator: \.isNewline)
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty } ?? trimmingCharacters(in: .whitespacesAndNewlines)
        guard normalized.count > limit else { return normalized }
        let end = normalized.index(normalized.startIndex, offsetBy: max(0, limit - 1))
        return "\(normalized[..<end])…"
    }

    var garyxSafeToolSummary: String? {
        let summary = garyxSingleLineTruncated(limit: 120)
        guard !summary.isEmpty, summary != "{", summary != "[", !summary.hasPrefix("{\"") else {
            return nil
        }
        return summary
    }

    var garyxPathTail: String {
        let normalized = replacingOccurrences(of: "\\", with: "/")
        let parts = normalized.split(separator: "/").map(String.init)
        guard parts.count > 2 else { return normalized }
        return parts.suffix(2).joined(separator: "/")
    }

    var garyxShellSummary: String {
        var normalized = trimmingCharacters(in: .whitespacesAndNewlines)
        let launchers = [
            "/bin/bash -lc ",
            "bash -lc ",
            "/bin/sh -lc ",
            "sh -lc ",
            "/bin/zsh -lc ",
            "zsh -lc ",
        ]
        for launcher in launchers where normalized.hasPrefix(launcher) {
            normalized = String(normalized.dropFirst(launcher.count)).garyxUnwrappedQuotes
            break
        }
        normalized = normalized
            .replacingOccurrences(of: #" 2>&1\b"#, with: "", options: .regularExpression)
            .replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return normalized.garyxSingleLineTruncated(limit: 112)
    }

    private var garyxUnwrappedQuotes: String {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count >= 2,
              let first = trimmed.first,
              let last = trimmed.last,
              (first == "\"" || first == "'"),
              first == last else {
            return trimmed
        }
        return String(trimmed.dropFirst().dropLast()).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
