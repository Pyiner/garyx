import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

struct GaryxPendingUploadPreview {
    var name: String
    var mediaType: String
    var previewDataUrl: String?
}

struct GaryxPendingQueuedInput {
    var threadId: String
    var text: String
    var attachments: [GaryxMobileComposerAttachment]
    var clientIntentId: String
}

struct GaryxEnsuredThread {
    var thread: GaryxThreadSummary
    var adoptedSelection: Bool
}

@MainActor
final class GaryxMobileModel: ObservableObject {
    static let threadListPageLimit = 30
    static let threadHistoryPageLimit = 120
    static let threadHistoryUserQueryLimit = 10
    static let selectedThreadReconcileIntervalNanos: UInt64 = 1_500_000_000
    static let assistantDeltaFlushDelayNanos: UInt64 = 50_000_000
    static let selectedThreadHistoryRetryLimit = 8

    struct MessageListSignature: Equatable {
        let count: Int
        let fingerprint: Int
        let sampled: Bool
    }

    struct TurnRowsCacheKey: Equatable {
        let isRunning: Bool
        let messages: MessageListSignature
    }

    struct PendingAssistantDelta {
        var targetId: String
        var text: String
    }

    struct WidgetAgentIdentity {
        var id: String?
        var name: String?
        var avatarDataUrl: String?
        var providerType: String?
        var isTeam: Bool
        var builtIn: Bool
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
            if let pendingSelectedMessagesSignature {
                selectedMessagesSignature = pendingSelectedMessagesSignature
                self.pendingSelectedMessagesSignature = nil
            } else {
                selectedMessagesSignature = Self.messageListSignature(for: messages)
            }
            selectedThreadTurnRowsCacheKey = nil
        }
    }
    @Published var draft = ""
    @Published var composerContextVersion = 0
    @Published var composerAttachments: [GaryxMobileComposerAttachment] = []
    @Published var isLoadingThreads = false
    @Published var isLoadingMoreThreads = false
    @Published var hasMoreThreadSummaries = false
    @Published var isLoadingSelectedThreadHistory = false
    @Published var isLoadingOlderThreadHistory = false
    @Published var selectedThreadHasMoreHistoryBefore = false
    @Published var isSending = false
    @Published var activeRunThreadId: String?
    @Published var remoteBusyThreadIds: Set<String> = []
    @Published var navigationState = GaryxMobileNavigationState()
    @Published var storedLastError: String?
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
    @Published var remoteStateLoadPhase: GaryxMobileLoadPhase = .idle
    @Published var agentTargetsLoadPhase: GaryxMobileLoadPhase = .idle
    @Published var selectedAgentTargetId: String
    @Published var newThreadWorkspace: String
    @Published var newThreadWorkspaceMode: String
    @Published var workspaceCatalogState = GaryxMobileResourceState(value: [String]())
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
    @Published var gatewaySettingsDocument: [String: GaryxJSONValue] = [:]
    @Published var isSavingBotSettings = false
    @Published var providerModelsByType: [String: GaryxProviderModels] = [:]
    @Published var selectedSkillEditor: GaryxSkillEditorState?
    @Published var selectedSkillDocument: GaryxSkillFileDocument?
    @Published var selectedSkillFileContent = ""
    @Published var researchCandidatesByRunId: [String: GaryxAutoResearchCandidatesPage] = [:]
    @Published var autoResearchDetailsByRunId: [String: GaryxAutoResearchDetail] = [:]
    @Published var autoResearchIterationsByRunId: [String: [GaryxAutoResearchIteration]] = [:]
    @Published var draftThreadTitle = ""
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

    let defaults: UserDefaults
    let keychain: GaryxMobileKeychain
    var activeTask: URLSessionWebSocketTask?
    var activeReaderTask: Task<Void, Never>?
    var activeTasksByThread: [String: URLSessionWebSocketTask] = [:]
    var activeReaderTasksByThread: [String: Task<Void, Never>] = [:]
    var globalEventStreamTask: Task<Void, Never>?
    var globalEventStreamGeneration: UUID?
    var globalEventStreamActive = false
    var selectedThreadReconcileTask: Task<Void, Never>?
    var selectedThreadReconcileThreadId: String?
    var selectedThreadActivitySignatures: [String: String] = [:]
    var messagesByThread: [String: [GaryxMobileMessage]] = [:]
    var messageSignaturesByThread: [String: MessageListSignature] = [:]
    var selectedMessagesSignature = MessageListSignature(count: 0, fingerprint: 0, sampled: false)
    var pendingSelectedMessagesSignature: MessageListSignature?
    var selectedThreadTurnRowsCacheKey: TurnRowsCacheKey?
    var selectedThreadTurnRowsCache: [GaryxMobileTurnRow] = []
    var activeAssistantMessageIdsByThread: [String: String] = [:]
    var pendingAssistantDeltasByThread: [String: PendingAssistantDelta] = [:]
    var assistantDeltaFlushTasksByThread: [String: Task<Void, Never>] = [:]
    var pendingQueuedInputsByIntentId: [String: GaryxPendingQueuedInput] = [:]
    var gatewayRuntimeGeneration = UUID()
    var selectedThreadRecoveryTask: Task<Void, Never>?
    var selectedThreadRecoveryThreadId: String?
    var selectedThreadHistoryRequestId: UUID?
    var threadHistoryLoadedIds: Set<String> = []
    var selectedThreadHistoryRetryTask: Task<Void, Never>?
    var selectedThreadHistoryRetryThreadId: String?
    var selectedThreadHistoryRetryCount = 0
    var completedThreadHistoryHydrationTasks: [String: Task<Void, Never>] = [:]
    var activeGatewayScopeId = ""
    var catalogSnapshotRestored = false
    var connectRefreshRequestId: UUID?
    var remoteStateRefreshRequestId: UUID?
    var agentTargetsRefreshRequestId: UUID?
    var agentTargetsStateRequestId: UUID?
    var workspaceRefreshRequestId: UUID?
    var nextThreadListOffset = 0
    var selectedThreadNextHistoryBeforeIndex: Int?
    var sceneRefreshTask: Task<Void, Never>?
    var pendingBotId: String?
    var pendingBotWorkspace: String?
    var pendingBotAgentId: String?
    var pendingBotDraftGeneration: UUID?
    var pendingNewThreadAgentTargetId: String?
    var pendingNewThreadAgentTargetGeneration: UUID?
    var selectedThreadDraftGeneration = UUID()
    var pendingThreadOpenRequestId = UUID()
    var pendingThreadLinkId: String?
    #if DEBUG
    var debugSnapshotActive = false
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
        gatewayProfiles = GaryxGatewayProfileStorage.load(defaults: defaults, key: GaryxMobileSettingsKeys.gatewayProfiles)
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
}
