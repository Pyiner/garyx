import Foundation
import os
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

/// App-side logging sink for the Core transcript cache store's diagnostics
/// (TASK-1751 P5). Kept here so `GaryxMobileCore` stays logging-free.
enum GaryxTranscriptCacheLog {
    static let logger = Logger(subsystem: "com.garyx.mobile", category: "transcript-cache")
}

struct GaryxPendingUploadPreview: Sendable {
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
final class GaryxRouteNotFoundStore: ObservableObject {
    @Published var selection: GaryxMobileRouteNotFound?
}

@MainActor
final class GaryxMobileModel: ObservableObject {
    static let threadListPageLimit = 30
    /// Load-more requests back off by this many rows and rely on id dedup,
    /// absorbing removal drift between offset pages (design §4).
    static let threadListPageOverlap = 5
    static let threadHistoryPageLimit = 100
    // Open a thread by loading the most recent few user-query turns (with tool
    // messages) in a single request — no separate fast/no-tools pre-pass.
    static let threadHistoryUserQueryLimit = 3
    // Cap on forward `after_index` delta pages walked in one incremental open so a
    // far-behind or misbehaving cursor can't loop unbounded; the reconcile loop
    // catches up any remainder. 50 * 100 = 5000 committed rows per catch-up.
    static let threadHistoryMaxForwardPages = 50
    static let selectedThreadReconcileIntervalNanos: UInt64 = 1_500_000_000
    static let backgroundCommittedRunReconcileIntervalNanos: UInt64 = 15_000_000_000
    static let backgroundCommittedRunThreadRefreshInterval: TimeInterval = 15
    /// Coalescing window for streamed committed rows: a large catch-up replays many
    /// committed messages back-to-back, so visible run-state, render, and
    /// disk-persist fold into one update per interval instead of flickering the list.
    static let streamedCommittedFlushDelayNanos = GaryxStreamUpdateCadence.committedMessageBatchWindowNanos
    static let selectedThreadHistoryRetryLimit = 8

    typealias MessageListSignature = GaryxMessageListSignature

    struct WidgetAgentIdentity {
        var id: String?
        var name: String?
        var avatarDataUrl: String?
        var providerType: String?
        var builtIn: Bool
    }

    @Published var gatewayURL: String {
        didSet {
            refreshNavigationDrawerSnapshot()
            refreshHomeObservationConnectionSnapshot()
        }
    }
    @Published var gatewayAuthToken: String
    @Published var gatewayHeaders: String
    @Published var gatewayProfiles: [GaryxGatewayProfile] {
        didSet { refreshNavigationDrawerSnapshot() }
    }
    @Published var gatewaySettingsStatus: String?
    @Published var connectionState: GaryxMobileConnectionState = .disconnected {
        didSet {
            refreshNavigationDrawerSnapshot()
            refreshHomeObservationConnectionSnapshot()
        }
    }
    @Published var threads: [GaryxThreadSummary] = [] {
        didSet {
            emitHomeProjectionSnapshot()
            refreshNavigationDrawerSnapshot()
        }
    }
    @Published var selectedThread: GaryxThreadSummary? {
        didSet {
            applySelectedThreadStreamPolicy(previousThreadId: oldValue?.id, selectedThreadId: selectedThread?.id)
            // Conversation identity: every real selection change resets the
            // scroll-container token, EXCEPT the draft-promotion write (the
            // just-created thread adopting the current draft) — the view's
            // .id() must stay continuous there or SwiftUI tears down and
            // rebuilds the whole transcript ("the list flashes" on the
            // first message of a new conversation).
            if adoptsDraftConversationToken {
                adoptsDraftConversationToken = false
            } else if oldValue?.id != selectedThread?.id {
                conversationSessionToken = UUID().uuidString
            }
            emitHomeProjectionSnapshot()
        }
    }
    @Published var messages: [GaryxMobileMessage] = [] {
        didSet {
            if let pendingSelectedMessagesSignature {
                selectedMessagesSignature = pendingSelectedMessagesSignature
                self.pendingSelectedMessagesSignature = nil
            } else {
                selectedMessagesSignature = GaryxMessageListSignature.make(for: messages)
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
    /// Filter-keyed Recent feeds. Each filter owns a pager/cursor/failure
    /// gate; All remains the canonical Widget/Automation feed while the
    /// selected feed alone drives Home presentation.
    var recentThreadFeeds = GaryxRecentThreadFeeds(
        pageLimit: GaryxMobileModel.threadListPageLimit,
        overlap: GaryxMobileModel.threadListPageOverlap
    ) {
        didSet {
            if oldValue != recentThreadFeeds {
                refreshHomeObservationPaginationSnapshot()
                emitHomeProjectionSnapshot()
            }
        }
    }
    var isLoadingThreads: Bool {
        recentThreadFeeds.selectedPresentation.showsInitialSkeleton
    }
    /// Identity for the conversation scroll container (see
    /// GaryxMobileConversationViews). Refreshed on real selection changes,
    /// preserved across draft promotion.
    @Published var conversationSessionToken = UUID().uuidString
    /// One-shot flag set by the draft-promotion write in ensureThread.
    var adoptsDraftConversationToken = false

    @Published var isLoadingSelectedThreadHistory = false
    @Published var isLoadingOlderThreadHistory = false
    @Published var selectedThreadHasMoreHistoryBefore = false
    /// Conversation run/send lifecycle state. Owns what used to be the
    /// scattered `isSending` / `activeRunThreadId` /
    /// `pendingChatStartThreadIds` / `terminatedActiveRunIdsByThread` flags;
    /// see docs/agents/conversation-state.md.
    @Published var runTracker = GaryxConversationRunTracker() {
        didSet { emitHomeProjectionSnapshot() }
    }
    /// Server run-state rebuilt from committed transcript control records.
    @Published var runStateByThread: [String: GaryxTranscriptRunState] = [:]
    /// Server-rendered transcript snapshots keyed by thread. These snapshots own
    /// visible transcript rows; committed messages remain only the data pool they
    /// reference.
    @Published var renderSnapshotsByThread: [String: GaryxRenderSnapshot] = [:]
    /// Bumped when the selected thread's render window (TASK-1751 P3) changes
    /// from an event handler (floor lock, expand, network-page extension, resume
    /// reset). The conversation body reads the windowed rows and this revision so
    /// a window change re-renders; the window state itself stays non-published so
    /// the pure body getter never publishes during a view update.
    @Published var selectedTurnRowsWindowRevision = 0
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
            rootNavigationPathStore.apply(navigationState: navigationState)
            refreshShellChromeSnapshot()
            refreshNavigationDrawerSnapshot()
            emitHomeProjectionSnapshot()
        }
    }
    @Published var pendingMobileRoute: GaryxMobileRoute?
    @Published var storedLastError: String?
    var lastError: String? {
        get {
            storedLastError
        }
        set {
            let message = Self.presentableErrorMessage(newValue)
            storedLastError = message
            homeObservationStore.setLastError(message)
        }
    }
    @Published var showsSettings = false {
        didSet { homeObservationStore.setShowsSettings(showsSettings) }
    }
    @Published var sidebarVisible = false {
        didSet { refreshShellChromeSnapshot() }
    }
    @Published var pinnedThreadIds: [String] = [] {
        didSet { emitHomeProjectionSnapshot() }
    }
    var allRecentThreadIds: [String] { recentThreadFeeds.allRecentThreadIds }
    var visibleRecentThreadIds: [String] { recentThreadFeeds.visibleRecentThreadIds }
    @Published var agents: [GaryxAgentSummary] = [] {
        didSet {
            predecodeAgentAvatarImages()
            emitHomeProjectionSnapshot()
        }
    }
    @Published var skills: [GaryxSkillSummary] = []
    /// Any capsules-list update (central catalog refresh, gallery refresh, local
    /// delete, gateway reset) prunes stale preview HTML so a remotely-deleted
    /// capsule's cached page cannot be served — and bumps the cache epoch so
    /// already-mounted thumbnails re-reconcile. See `pruneCapsuleHTMLCache`.
    @Published var capsules: [GaryxCapsuleSummary] = [] {
        didSet { pruneCapsuleHTMLCache(validCapsules: capsules) }
    }
    /// Focused capsule preview presented over the Capsules gallery (card tap or
    /// `garyx://mobile/capsule` deep link).
    @Published var galleryFocusedCapsule: GaryxCapsuleSummary?
    /// Focused capsule preview presented over the current conversation (chat
    /// capsule-card tap). Kept separate from the gallery cover so each surface
    /// hosts and dismisses its own preview.
    @Published var conversationCapsulePreview: GaryxCapsuleSummary?
    var capsuleHTMLCache: [GaryxCapsuleHTMLCacheKey: String] = [:]
    /// Bumped whenever cached preview HTML or a rendered thumbnail is evicted
    /// (prune or `/serve` 404), so `GaryxCapsulePreviewThumbnail` can include it
    /// in its `.task` identity and re-validate already-mounted thumbnails.
    @Published var capsuleHTMLCacheEpoch: Int = 0
    /// Rendered-thumbnail cache stack: the gallery and chat cards display a
    /// cached PNG (zero live `WKWebView`); a miss renders once via
    /// `GaryxCapsuleThumbnailRenderer` and writes through to disk + memory. This
    /// removes the live-render concurrency cap that starved gallery cards (A1)
    /// and pins a fixed 16:rendition cover crop (A2).
    let capsuleThumbnailStore = GaryxCapsuleThumbnailDiskStore()
    let capsuleThumbnailRenderer = GaryxCapsuleThumbnailRenderer()
    let capsuleThumbnailMemory = GaryxCapsuleThumbnailMemoryCache()
    /// Conversation task-tree sidebar: the trailing push-in panel on the chat
    /// surface. `taskTreeForestPage` is the anchored forest snapshot for the
    /// currently selected thread. Because the origin-rooted forest is
    /// anchor-independent, snapshots cache per *tree*
    /// (`taskTreeSnapshotsByOrigin`, keyed by gateway scope + tree cache key)
    /// with `taskTreeOriginKeyByAnchor` as the anchor→tree index; row-tap
    /// navigation pre-seeds that index so in-tree thread switches render
    /// instantly from cache while the live fetch revalidates in place.
    @Published var isTaskTreeSidebarOpen = false
    @Published var taskTreeForestPage: GaryxTaskForestPage?
    @Published var taskTreeLoadPhase: GaryxMobileLoadPhase = .idle
    var taskTreeRequestGate = GaryxTaskTreeRequestGate()
    var taskTreeOriginKeyByAnchor: [String: String] = [:]
    var taskTreeSnapshotsByOrigin: [String: GaryxTaskForestPage] = [:]
    /// Insertion order of `taskTreeSnapshotsByOrigin` keys for FIFO eviction.
    var taskTreeSnapshotOriginOrder: [String] = []
    @Published var automations: [GaryxAutomationSummary] = [] {
        didSet { emitHomeProjectionSnapshot() }
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
    @Published var workspaceCatalogState = GaryxMobileResourceState(value: [String]()) {
        didSet { refreshNavigationDrawerSnapshot() }
    }
    @Published var lastAutomationRun: GaryxAutomationActivityEntry?
    @Published var selectedWorkspacePath = ""
    @Published var selectedWorkspaceDirectory = ""
    @Published var draftWorkspacePath = ""
    @Published var workspaceListing: GaryxWorkspaceFileListing?
    @Published var workspacePreview: GaryxWorkspaceFilePreview?
    @Published var workspaceGitStatuses: [String: GaryxWorkspaceGitStatus] = [:]
    @Published var debugShowsWorkspaceModeSheet = false
    @Published var debugShowsGatewaySwitcher = false {
        didSet { homeObservationStore.setDebugShowsGatewaySwitcher(debugShowsGatewaySwitcher) }
    }
    @Published var isUploadingWorkspaceFiles = false
    @Published var workspaceUploadStatus: String?
    @Published var slashCommands: [GaryxSlashCommand] = []
    @Published var mcpServers: [GaryxMcpServer] = []
    @Published var channelEndpoints: [GaryxChannelEndpoint] = [] {
        didSet {
            predecodeChannelIconImages()
            refreshNavigationDrawerSnapshot()
        }
    }
    @Published var configuredBots: [GaryxConfiguredBot] = [] {
        didSet {
            predecodeChannelIconImages()
            refreshNavigationDrawerSnapshot()
        }
    }
    @Published var botConsoles: [GaryxBotConsoleSummary] = [] {
        didSet {
            predecodeChannelIconImages()
            refreshNavigationDrawerSnapshot()
        }
    }
    @Published var botStatusesById: [String: GaryxBotBindingResult] = [:]
    @Published var channelPlugins: [GaryxChannelPluginCatalogEntry] = [] {
        didSet {
            predecodeChannelIconImages()
            refreshNavigationDrawerSnapshot()
        }
    }
    @Published var gatewaySettingsDocument: [String: GaryxJSONValue] = [:]
    @Published var isSavingBotSettings = false
    @Published var providerModelsByType: [String: GaryxProviderModels] = [:]
    @Published var codingUsage: GaryxCodingUsage?
    @Published var claudeCodeAuthSession: GaryxClaudeCodeAuthSession?
    @Published var selectedSkillEditor: GaryxSkillEditorState?
    @Published var selectedSkillDocument: GaryxSkillFileDocument?
    @Published var selectedAutomationEditor: GaryxAutomationSummary?
    @Published var selectedAgentDetail: GaryxAgentSummary?
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
    let gatewayClientFactory: ((GaryxGatewayConfiguration) -> GaryxGatewayClient)?
    var backgroundCommittedRunReconcileTask: Task<Void, Never>?
    var selectedThreadReconcileTask: Task<Void, Never>?
    var selectedThreadReconcileThreadId: String?
    var selectedThreadActivitySignatures: [String: String] = [:]
    /// S5 resumable per-thread transcript stream for the open thread.
    var selectedThreadStreamTask: Task<Void, Never>?
    var selectedThreadStreamGeneration: UUID?
    var streamOwnedThreadId: String?
    /// Leading-edge throttle for stream flushes: the gate decides (per settled
    /// render frame) whether to flush immediately, coalesce into the open
    /// window, or skip a no-op frame; the task is the armed window timer.
    /// Invariant: the timer runs iff the gate is in its window state — both
    /// are driven together by settle/elapse/cancel helpers in ThreadStream.
    var selectedThreadStreamFlushGate = GaryxStreamFlushGate()
    var selectedThreadStreamFlushTask: Task<Void, Never>?
    var selectedThreadStreamDrainTask: Task<Void, Never>?
    var messagesByThread: [String: [GaryxMobileMessage]] = [:]
    var messageSignaturesByThread: [String: MessageListSignature] = [:]
    /// Persistent committed-transcript cache (S2/S3): instant cold-start display
    /// and incremental (`after_index`) opens. `cachedTranscriptSnapshots` is the
    /// in-memory mirror of the on-disk window so the forward cursor is read
    /// without touching disk on every delta fetch.
    var transcriptCacheStore: GaryxTranscriptCacheStore = GaryxTranscriptFileCacheStore(
        directory: GaryxTranscriptFileCacheStore.defaultDirectory(),
        ttl: GaryxTranscriptFileCacheStore.defaultTTL,
        diagnostics: { event in
            // TASK-1751 P5: surface persistent-cache write failures instead of
            // swallowing them. Core stays logging-free; the app owns the sink.
            switch event {
            case let .saveEncodeFailed(threadId):
                GaryxTranscriptCacheLog.logger.error(
                    "transcript cache encode failed thread=\(threadId, privacy: .public)"
                )
            case let .saveWriteFailed(threadId, reason):
                GaryxTranscriptCacheLog.logger.error(
                    "transcript cache write failed thread=\(threadId, privacy: .public) reason=\(reason, privacy: .public)"
                )
            }
        }
    )
    /// In-memory mirror of the on-disk committed window per thread. Wrapped so
    /// every mutation (set AND clear) bumps a per-thread generation the cold-open
    /// restore policy compares against — a write path can no longer bypass the
    /// freshness gate (TASK-1751 P1).
    var transcriptMirror = GaryxTranscriptMirrorStore()
    var transcriptCachePersistenceGenerations: [String: UInt64] = [:]
    /// Monotonic per-thread cold-open generation, bumped in `showSelectedThread`
    /// on a thread-id change; the async restore task captures it at spawn and
    /// aborts if it moved (switch-away-and-back). TASK-1751 P1.
    var selectedThreadColdOpenGeneration: UInt64 = 0
    /// LRU residency cap over the per-thread projections (TASK-1751 P4).
    var threadResidencyTracker = GaryxThreadResidencyTracker()
    /// Memoized full prepared turn rows for the selected thread (TASK-1751 P2);
    /// plain (non-published) — mutating it during a body read is invisible to
    /// SwiftUI, matching the scroll-state box pattern.
    var selectedTurnRowsCache = GaryxTurnRowsCache()
    /// Floor-anchored render window state for the selected thread (TASK-1751 P3);
    /// plain (non-published). The floor is only ever written from event handlers,
    /// never from the body getter.
    var selectedTurnRowsWindowState = GaryxTurnRowsWindowState()
    var selectedMessagesSignature = MessageListSignature(count: 0, fingerprint: 0, sampled: false)
    var pendingSelectedMessagesSignature: MessageListSignature?
    var activeAssistantMessageIdsByThread: [String: String] = [:]
    var pendingDirectFollowUpsByThread: [String: [(userId: String, assistantId: String)]] = [:]
    var pendingQueuedInputsByIntentId: [String: GaryxPendingQueuedInput] = [:]
    var pendingThreadArchives = GaryxPendingThreadArchiveState()
    var auxiliaryAllRecentThreadsRefreshTask: Task<Void, Never>?
    var auxiliaryAllRecentThreadsRefreshTaskId: UUID?
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
    let rootNavigationPathStore = GaryxRootNavigationPathStore()
    let routeNotFoundStore = GaryxRouteNotFoundStore()
    let homeObservationStore = GaryxHomeObservationStore()
    let homeThreadListStore = GaryxHomeThreadListStore()
    let homeProjectionGateway = HomeProjectionGateway()
    let shellChromeStore = GaryxShellChromeStore()
    let navigationDrawerStore = GaryxNavigationDrawerStore()
    let recentThreadsWidgetPersistenceQueue = GaryxRecentThreadsWidgetPersistenceQueue()
    let avatarStore: GaryxAvatarDiskStore
    let avatarImageProvider: GaryxAvatarImageProvider
    let backgroundCommittedRunReconcilePlanner = GaryxBackgroundCommittedRunReconcilePlanner(
        minimumRefreshInterval: GaryxMobileModel.backgroundCommittedRunThreadRefreshInterval
    )
    var recentThreadsWidgetPersistenceGeneration: UInt64 = 0
    var hasAttemptedLastOpenedThreadRestore = false
    var selectedThreadNextHistoryBeforeIndex: Int?
    var selectedThreadRenderFloorByThread: [String: Int] = [:]
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
    var claudeCodeAuthPollTask: Task<Void, Never>?
    var claudeCodeAuthPollGeneration: UUID?
    #if DEBUG
    var debugSnapshotActive = false
    #endif

    init(
        defaults: UserDefaults = .standard,
        keychain: GaryxMobileKeychain = .shared,
        gatewayClientFactory: ((GaryxGatewayConfiguration) -> GaryxGatewayClient)? = nil
    ) {
        self.defaults = defaults
        self.keychain = keychain
        self.gatewayClientFactory = gatewayClientFactory
        let avatarStore = GaryxAvatarDiskStore()
        self.avatarStore = avatarStore
        self.avatarImageProvider = GaryxAvatarImageProvider(
            store: avatarStore,
            validator: GaryxAvatarCGImageValidator()
        )
        gatewayURL = Self.firstNonEmpty(
            defaults.string(forKey: GaryxMobileSettingsKeys.gatewayUrl),
            defaults.string(forKey: GaryxMobileSettingsKeys.legacyGatewayURL)
        ) ?? Self.defaultGatewayURL
        let storedToken = keychain.readGatewayAuthToken()
        let legacyToken = defaults.string(forKey: GaryxMobileSettingsKeys.legacyGatewayToken) ?? ""
        gatewayAuthToken = storedToken.isEmpty ? legacyToken : storedToken
        gatewayHeaders = GaryxGatewayHeaders.normalizedBlock(
            defaults.string(forKey: GaryxMobileSettingsKeys.gatewayHeaders) ?? ""
        )
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
        rootNavigationPathStore.apply(navigationState: navigationState)
        homeProjectionGateway.setResultHandler { [weak self] result in
            self?.applyHomeProjectionResult(result)
        }
        refreshHomeObservationSnapshot()
        refreshShellChromeSnapshot()
        refreshNavigationDrawerSnapshot()
        emitHomeProjectionSnapshot()
        #if DEBUG
        GaryxHomeScrollPerformanceProbe.shared.attachModelObjectWillChange(objectWillChange)
        startHomeScrollPressureProbeIfRequested()
        #endif
        Task.detached(priority: .utility) {
            await avatarStore.warm()
        }
    }
}
