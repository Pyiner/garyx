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

struct GaryxGatewayConnectTimeoutError: LocalizedError {
    var errorDescription: String? {
        "Gateway did not respond within 5 seconds."
    }
}

struct GaryxMobileRouteNotFound: Identifiable, Equatable {
    let id = UUID()
    let title: String
    let message: String
}

@MainActor
final class GaryxMobileModel: ObservableObject {
    static let threadListPageLimit = 30
    static let threadHistoryPageLimit = 100
    // Open a thread by loading the most recent few user-query turns (with tool
    // messages) in a single request — no separate fast/no-tools pre-pass.
    static let threadHistoryUserQueryLimit = 3
    // Cap on forward `after_index` delta pages walked in one incremental open so a
    // far-behind or misbehaving cursor can't loop unbounded; the reconcile loop
    // catches up any remainder. 50 * 100 = 5000 committed rows per catch-up.
    static let threadHistoryMaxForwardPages = 50
    static let selectedThreadReconcileIntervalNanos: UInt64 = 1_500_000_000
    /// Coalescing window for streamed committed rows: a large catch-up replays many
    /// committed messages back-to-back, so visible run-state, render, and
    /// disk-persist fold into one update per interval instead of flickering the list.
    static let streamedCommittedFlushDelayNanos = GaryxStreamUpdateCadence.committedMessageBatchWindowNanos
    static let selectedThreadHistoryRetryLimit = 8

    struct MessageListSignature: Equatable {
        let count: Int
        let fingerprint: Int
        let sampled: Bool
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
    @Published var threads: [GaryxThreadSummary] = [] {
        didSet { refreshHomeThreadListSnapshot() }
    }
    @Published var selectedThread: GaryxThreadSummary? {
        didSet {
            applySelectedThreadStreamPolicy(previousThreadId: oldValue?.id, selectedThreadId: selectedThread?.id)
            refreshHomeThreadListSnapshot()
        }
    }
    @Published var messages: [GaryxMobileMessage] = [] {
        didSet {
            if let pendingSelectedMessagesSignature {
                selectedMessagesSignature = pendingSelectedMessagesSignature
                self.pendingSelectedMessagesSignature = nil
            } else {
                selectedMessagesSignature = Self.messageListSignature(for: messages)
            }
        }
    }
    /// Per-thread composer drafts. Not `@Published`: the composer view owns the
    /// live text and reloads on `composerContextVersion`, so persisting a single
    /// keystroke must not publish and re-render the transcript. Read the active
    /// context's text through `activeComposerDraft`.
    var composerDraftStore = GaryxComposerDraftStore()
    @Published var composerContextVersion = 0
    @Published var composerAttachments: [GaryxMobileComposerAttachment] = []
    @Published var isLoadingThreads = false {
        didSet { refreshHomeThreadListSnapshot() }
    }
    @Published var isLoadingMoreThreads = false
    @Published var hasMoreThreadSummaries = false
    @Published var isLoadingSelectedThreadHistory = false
    @Published var isLoadingOlderThreadHistory = false
    @Published var selectedThreadHasMoreHistoryBefore = false
    /// Conversation run/send lifecycle state. Owns what used to be the
    /// scattered `isSending` / `activeRunThreadId` /
    /// `pendingChatStartThreadIds` / `terminatedActiveRunIdsByThread` flags;
    /// see docs/agents/conversation-state.md.
    @Published var runTracker = GaryxConversationRunTracker() {
        didSet { refreshHomeThreadListSnapshot() }
    }
    /// Server run-state rebuilt from committed transcript control records.
    @Published var runStateByThread: [String: GaryxTranscriptRunState] = [:] {
        didSet { refreshHomeThreadListSnapshot() }
    }
    /// Server-rendered transcript snapshots keyed by thread. These snapshots own
    /// visible transcript rows; committed messages remain only the data pool they
    /// reference.
    @Published var renderSnapshotsByThread: [String: GaryxRenderSnapshot] = [:]
    /// Legacy-shaped read bridges over `runTracker`.
    var isSending: Bool { runTracker.hasLocalActiveRun }
    var activeRunThreadId: String? { runTracker.localActiveRunThreadId }
    var remoteBusyThreadIds: Set<String> {
        runTracker.busyThreadIds.union(
            Set(runStateByThread.compactMap { threadId, state in
                state.busy ? threadId : nil
            })
        )
    }
    @Published var navigationState = GaryxMobileNavigationState() {
        didSet {
            endThreadListInteractionIfHomeBecameHidden(previousNavigationState: oldValue)
            refreshHomeThreadListSnapshot()
        }
    }
    @Published var pendingMobileRoute: GaryxMobileRoute?
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
    @Published var pinnedThreadIds: [String] = [] {
        didSet { refreshHomeThreadListSnapshot() }
    }
    @Published var recentThreadIds: [String] = [] {
        didSet { refreshHomeThreadListSnapshot() }
    }
    @Published var dreams: [GaryxDreamTopic] = []
    @Published var latestDreamScan: GaryxDreamScan?
    @Published var isScanningDreams = false
    @Published var dreamsAutoScanEnabled = false
    @Published var isSavingDreamsSettings = false
    @Published var agents: [GaryxAgentSummary] = [] {
        didSet { refreshHomeThreadListSnapshot() }
    }
    @Published var teams: [GaryxTeamSummary] = [] {
        didSet { refreshHomeThreadListSnapshot() }
    }
    @Published var skills: [GaryxSkillSummary] = []
    @Published var tasks: [GaryxTaskSummary] = []
    @Published var tasksPanelState = GaryxMobileTasksPanelState()
    @Published var workflowRunPanelState = GaryxWorkflowRunPanelState()
    @Published var selectedWorkflowRunThread: GaryxThreadSummary?
    @Published var automations: [GaryxAutomationSummary] = [] {
        didSet { refreshHomeThreadListSnapshot() }
    }
    @Published var remoteStateLoadPhase: GaryxMobileLoadPhase = .idle
    @Published var agentTargetsLoadPhase: GaryxMobileLoadPhase = .idle
    @Published var selectedAgentTargetId: String
    @Published var newThreadWorkspace: String
    @Published var newThreadWorkspaceMode: String
    /// Per-thread overrides for the new-thread draft; empty means agent default.
    @Published var newThreadModelOverride = ""
    @Published var newThreadReasoningEffortOverride = ""
    @Published var newThreadServiceTierOverride = ""
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
    #if DEBUG
    @Published var debugShowsWorkspaceModeSheet = false
    @Published var debugShowsGatewaySwitcher = false
    #endif
    @Published var isUploadingWorkspaceFiles = false
    @Published var workspaceUploadStatus: String?
    @Published var slashCommands: [GaryxSlashCommand] = []
    @Published var mcpServers: [GaryxMcpServer] = []
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
    @Published var selectedTaskDetail: GaryxTaskSummary?
    @Published var selectedAutomationEditor: GaryxAutomationSummary?
    @Published var selectedAgentDetail: GaryxAgentSummary?
    @Published var selectedTeamDetail: GaryxTeamSummary?
    @Published var selectedRouteNotFound: GaryxMobileRouteNotFound?
    var skillEditorLoadRequestId: UUID?
    var skillFileLoadRequestId: UUID?
    @Published var draftThreadTitle = ""
    @Published var draftSkillId = ""
    @Published var draftSkillName = ""
    @Published var draftSkillDescription = ""
    @Published var draftSkillBody = ""
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
    let defaults: UserDefaults
    let keychain: GaryxMobileKeychain
    var backgroundCommittedRunReconcileTask: Task<Void, Never>?
    var selectedThreadReconcileTask: Task<Void, Never>?
    var selectedThreadReconcileThreadId: String?
    var selectedThreadActivitySignatures: [String: String] = [:]
    /// S5 resumable per-thread transcript stream for the open thread.
    var selectedThreadStreamTask: Task<Void, Never>?
    var selectedThreadStreamGeneration: UUID?
    var streamOwnedThreadId: String?
    /// Resume-cursor override for the next per-thread stream connection. Set when a
    /// live seq gap is detected (a dropped broadcast event) so the reconnect replays
    /// from the last contiguous seq and refills the hole, rather than from the cache
    /// head (which would skip it).
    var selectedThreadStreamResumeOverride: Int?
    /// Highest committed seq applied on the CURRENT stream connection (0 = none yet).
    /// Drives mid-stream seq-gap detection and per-connection progress; reset on each
    /// (re)connect.
    var selectedThreadStreamConnectionLastSeq: Int = 0
    /// Coalesces render + persist across a burst of streamed committed rows (a large
    /// catch-up). Each row merges into the in-memory window immediately; this task
    /// flushes the accumulated window to the view/disk once per interval.
    var selectedThreadStreamFlushTask: Task<Void, Never>?
    var messagesByThread: [String: [GaryxMobileMessage]] = [:]
    var messageSignaturesByThread: [String: MessageListSignature] = [:]
    /// Persistent committed-transcript cache (S2/S3): instant cold-start display
    /// and incremental (`after_index`) opens. `cachedTranscriptSnapshots` is the
    /// in-memory mirror of the on-disk window so the forward cursor is read
    /// without touching disk on every delta fetch.
    var transcriptCacheStore: GaryxTranscriptCacheStore = GaryxTranscriptFileCacheStore(
        directory: GaryxTranscriptFileCacheStore.defaultDirectory(),
        ttl: GaryxTranscriptFileCacheStore.defaultTTL
    )
    var cachedTranscriptSnapshots: [String: GaryxCachedTranscript] = [:]
    var selectedMessagesSignature = MessageListSignature(count: 0, fingerprint: 0, sampled: false)
    var pendingSelectedMessagesSignature: MessageListSignature?
    var activeAssistantMessageIdsByThread: [String: String] = [:]
    var pendingDirectFollowUpsByThread: [String: [(userId: String, assistantId: String)]] = [:]
    var pendingQueuedInputsByIntentId: [String: GaryxPendingQueuedInput] = [:]
    var pendingThreadArchives = GaryxPendingThreadArchiveState()
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
    var lastPersistedWidgetThreads: [GaryxMobileWidgetThread]?
    var hasDeferredRecentThreadsWidgetSnapshotPersistence = false
    let homeThreadListStore = GaryxHomeThreadListStore()
    var isThreadListInteracting = false
    var hasAttemptedLastOpenedThreadRestore = false
    var selectedThreadNextHistoryBeforeIndex: Int?
    var sceneRefreshTask: Task<Void, Never>?
    var pendingBotId: String?
    var pendingBotWorkspace: String?
    var pendingBotAgentId: String?
    var pendingBotDraftGeneration: UUID?
    var pendingNewThreadAgentTargetId: String?
    var pendingNewThreadAgentTargetGeneration: UUID?
    var selectedThreadDraftGeneration = UUID()
    var threadOpenState = GaryxMobileThreadOpenState()
    var threadRuntimeMutationIds: [String: UUID] = [:]
    var workflowRunPollTask: Task<Void, Never>?
    var workflowRunPollGeneration: UUID?
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
        refreshHomeThreadListSnapshot()
    }
}
