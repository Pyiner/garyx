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
    var createDeliveryKey: GaryxCreateDeliveryKey? = nil
}

struct GaryxGatewayRuntimeIdentity: Equatable {
    var gatewayURL: String
    var authToken: String
    var headers: String
}

struct GaryxThreadRuntimeRollbackSnapshot {
    var selectedRuntime: GaryxThreadRuntimeSummary?
    var listRuntime: GaryxThreadRuntimeSummary?
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
    @Published var selectedThread: GaryxThreadSummary? {
        didSet {
            let selectionChanged = oldValue?.id != selectedThread?.id
            threadSummaryLeaseOwner.swapSelectedThread(selectedThread)
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
            if selectionChanged {
                refreshResidentThreadListStores()
            }
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
    /// Route-owned, scope-partitioned payload state. The composer observes this
    /// object directly so ordered keystrokes do not invalidate the transcript.
    let composerPayloadCoordinator: GaryxComposerPayloadCoordinator
    /// Filter-keyed Recent feeds. Each filter owns a pager/cursor/failure
    /// gate; All remains the canonical Widget/Automation feed while the
    /// selected feed alone drives Home presentation.
    var recentThreadFeeds: GaryxRecentThreadFeeds {
        didSet {
            if oldValue != recentThreadFeeds {
                refreshHomeObservationPaginationSnapshot()
                emitHomeProjectionSnapshot()
            }
        }
    }
    var threadFavoritesState: GaryxFavoritesState { threadFavoritesProvider.state }
    var isLoadingThreads: Bool {
        selectedRecentFeedPresentation.showsInitialSkeleton
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
        didSet {
            emitHomeProjectionSnapshot()
            if oldValue.busyThreadIds != runTracker.busyThreadIds {
                refreshResidentThreadListStores()
            }
        }
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
        didSet {
            refreshRecentThreadLeases()
            emitHomeProjectionSnapshot()
            refreshResidentThreadListStores()
        }
    }
    var allRecentThreadIds: [String] { recentThreadFeeds.allRecentThreadIds }
    var visibleRecentThreadIds: [String] {
        recentThreadFeeds.selectedFilter == .favorites
            ? pendingThreadArchives.visibleThreadIds(threadFavoritesState.presentedThreadIds)
            : recentThreadFeeds.visibleRecentThreadIds
    }
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
    var capsuleFavoriteState = GaryxCapsuleFavoriteReducerState()
    /// Focused capsule preview presented over the Capsules gallery (card tap or
    /// `garyx://mobile/capsule` deep link).
    @Published var galleryFocusedCapsule: GaryxCapsulePreviewSelection?
    /// Focused capsule preview presented over the current conversation (chat
    /// capsule-card tap). Kept separate from the gallery cover so each surface
    /// hosts and dismisses its own preview.
    @Published var conversationCapsulePreview: GaryxCapsulePreviewSelection?
    /// Scene notifications are versioned so every focused preview receives
    /// repeated inactive/background/active transitions, even when the enum case
    /// itself is unchanged.
    @Published var capsulePreviewSceneSignal = GaryxCapsulePreviewSceneSignal()
    var capsuleHTMLCache: [GaryxCapsuleHTMLCacheKey: String] = [:]
    /// All Capsule catalog reads converge here. The worker is single-flight;
    /// `requestedTicket > finishedTicket` is the trailing-refresh marker, and
    /// the committed ticket is an additional latest-wins defense.
    var capsuleCatalogRefreshTask: Task<Result<[GaryxCapsuleSummary], Error>, Never>?
    var capsuleCatalogRefreshTaskToken: UUID?
    var capsuleCatalogRequestedTicket: UInt64 = 0
    var capsuleCatalogFinishedTicket: UInt64 = 0
    var capsuleCatalogCommittedTicket: UInt64 = 0
    /// Focused `/serve` commits are accepted only for the latest (key, token).
    var focusedCapsuleHTMLRequestGate = GaryxFocusedCapsuleHTMLRequestGate()
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
        didSet {
            emitHomeProjectionSnapshot()
            let oldTargets = Set(oldValue.compactMap { automation in
                (automation.targetThreadId ?? "").garyxTrimmedNilIfEmpty
            })
            let newTargets = Set(automations.compactMap { automation in
                (automation.targetThreadId ?? "").garyxTrimmedNilIfEmpty
            })
            if oldTargets != newTargets {
                refreshResidentThreadListStores()
            }
        }
    }
    @Published var remoteStateLoadPhase: GaryxMobileLoadPhase = .idle
    @Published var agentTargetsLoadPhase: GaryxMobileLoadPhase = .idle
    /// One-off override for the current new-thread draft. Global default state
    /// is gateway-owned and cached separately below.
    @Published var selectedAgentTargetId: String?
    @Published var gatewayDefaultAgentId: String?
    @Published var effectiveDefaultAgentId: String?
    @Published var newThreadWorkspaceSelection: GaryxDraftWorkspaceSelection
    @Published var newThreadWorkspaceMode: String
    /// Per-thread overrides for the new-thread draft; empty means agent default.
    @Published var newThreadModelOverride = ""
    @Published var newThreadReasoningEffortOverride = ""
    @Published var newThreadServiceTierOverride = ""
    @Published var workspaceCatalogState = GaryxMobileResourceState(value: GaryxWorkspaceCatalog.empty) {
        didSet { refreshNavigationDrawerSnapshot() }
    }
    /// Monotonic gateway-scope generation for the workspace universe.
    /// Transient phase values can be coalesced inside one MainActor turn
    /// (loaded → idle → loaded), so scope-bound UI observes this counter —
    /// which only ever moves forward — instead of sniffing for `.idle`.
    @Published var workspaceCatalogScopeEpoch = 0
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
    @Published var claudeCodeAccounts: GaryxClaudeCodeAccounts?
    @Published var isLoadingClaudeCodeAccounts = false
    @Published var claudeCodeAccountsError: String?
    @Published var isMutatingClaudeCodeAccount = false
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
    /// Conversation runtime activation is occurrence-scoped and begins only
    /// when the Core route policy enters its masked preparation phase. Local
    /// activation and its waiters complete without waiting for the independent
    /// initial history refresh.
    var conversationContentActivationOccurrenceID: GaryxRouteInstanceID?
    var conversationInitialHistoryRefreshTask: Task<Void, Never>?
    var completedConversationContentActivationOccurrenceID: GaryxRouteInstanceID?
    var conversationContentActivationWaiters: [
        GaryxRouteInstanceID: [CheckedContinuation<Void, Never>]
    ] = [:]
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
    /// Deterministic test seam; production uses the Core policy delay.
    var lifecycleRetryDelayOverrideNanoseconds: UInt64?
    var auxiliaryAllRecentThreadsRefreshTask: Task<Void, Never>?
    var auxiliaryAllRecentThreadsRefreshTaskId: UUID?
    /// Durable scope + ephemeral activation CAS captured by every gateway
    /// request. Switching away and back to the same gateway creates a distinct
    /// activation while retaining that scope's composer partition.
    var gatewayRequestToken = GaryxGatewayRequestToken(
        scope: GaryxGatewayScope(identity: "unconfigured", epoch: 1),
        activationSequence: 1
    )
    var gatewayScopeRegistry = GaryxGatewayScopeRegistry()
    var gatewayScopeEpochByIdentity: [String: UInt64] = [:]
    var nextGatewayActivationSequence: UInt64 = 1
    var selectedThreadRecoveryTask: Task<Void, Never>?
    var selectedThreadRecoveryThreadId: String?
    var selectedThreadHistoryRequestId: UUID?
    var threadHistoryLoadedIds: Set<String> = []
    var selectedThreadHistoryRetryTask: Task<Void, Never>?
    var selectedThreadHistoryRetryThreadId: String?
    var selectedThreadHistoryRetryCount = 0
    var completedThreadHistoryHydrationTasks: [String: Task<Void, Never>] = [:]
    var activeGatewayScopeId = ""
    var activeGatewayRuntimeIdentity: GaryxGatewayRuntimeIdentity?
    var catalogSnapshotRestored = false
    var connectRefreshRequestId: UUID?
    var remoteStateRefreshRequestId: UUID?
    var agentTargetsRefreshRequestId: UUID?
    var agentTargetsStateRequestId: UUID?
    var workspaceRefreshRequestId: UUID?
    let productionRouteStore = GaryxProductionRouteStore()
    let routeNotFoundStore = GaryxRouteNotFoundStore()
    let homeObservationStore = GaryxHomeObservationStore()
    let threadSummaryCache: GaryxThreadSummaryCache
    let threadSummaryLeaseOwner: GaryxThreadSummaryLeaseOwner
    let threadMutationHubStore: GaryxThreadMutationHubStore
    let threadFavoritesProvider: GaryxFavoritesMembershipProvider
    let homeThreadListStore: GaryxHomeThreadListStore
    var threadFeedRegistry = GaryxThreadFeedRegistry()
    var workspaceThreadProviders: [String: GaryxThreadSummaryMembershipProvider] = [:]
    var workspaceThreadStores: [String: GaryxThreadListStore] = [:]
    var automationThreadProviders: [String: GaryxAutomationThreadMembershipProvider] = [:]
    var automationThreadStores: [String: GaryxThreadListStore] = [:]
    var botThreadProviders: [String: GaryxBotConversationMembershipProvider] = [:]
    var botThreadStores: [String: GaryxThreadListStore] = [:]
    var botThreadHydrationTasks: [String: [String: Task<Void, Never>]] = [:]
    var threadFavoritesSnapshotTask: Task<Void, Never>?
    var threadFavoritesSnapshotTaskToken: UUID?
    var threadSummaryRuntimeEpoch: UInt64 = 0
    var nextThreadMutationSequence: UInt64 = 1
    lazy var threadSummaryCapabilityStateMachine = GaryxThreadSummaryCapabilityStateMachine(
        runtimeEpoch: threadSummaryRuntimeEpoch
    ) { [weak self] in
        guard let self else { return .failed }
        return await self.probeThreadSummaryCapability()
    }
    let pinnedOrderOutboxStore: GaryxPinnedOrderUserDefaultsStore
    let homeProjectionGateway = HomeProjectionGateway()
    let shellChromeStore = GaryxShellChromeStore()
    let navigationDrawerStore = GaryxNavigationDrawerStore()
    let drawerRevealInteraction = GaryxHorizontalRevealInteractionStore(
        projection: .fullScreenNavigation,
        bindsToRootSurfaceHost: true
    )
    let taskTreeRevealInteraction = GaryxHorizontalRevealInteractionStore(
        projection: .fullScreenNavigation,
        bindsToRootSurfaceHost: true
    )
    let recentThreadsWidgetPersistenceQueue = GaryxRecentThreadsWidgetPersistenceQueue()
    let avatarStore: GaryxAvatarDiskStore
    let avatarImageProvider: GaryxAvatarImageProvider
    let backgroundCommittedRunReconcilePlanner = GaryxBackgroundCommittedRunReconcilePlanner(
        minimumRefreshInterval: GaryxMobileModel.backgroundCommittedRunThreadRefreshInterval
    )
    var recentThreadsWidgetPersistenceGeneration: UInt64 = 0
    var pinnedOrderReorderTask: Task<Void, Never>?
    var pinnedOrderReorderTaskToken: UInt64?
    var hasAttemptedLastOpenedThreadRestore = false
    var selectedThreadNextHistoryBeforeIndex: Int?
    var selectedThreadRenderFloorByThread: [String: Int] = [:]
    var sceneRefreshTask: Task<Void, Never>?
    var pendingBotId: String?
    var pendingBotWorkspace: String?
    var pendingBotAgentId: String?
    var pendingBotDraftGeneration: UUID?
    var pendingNewThreadAgentTargetGeneration: UUID?
    var frozenNewThreadAgentTargetGeneration: UUID?
    var selectedThreadDraftGeneration = UUID()
    var threadOpenState = GaryxMobileThreadOpenState()
    var threadRenameMutationIds: [String: GaryxThreadMutationID] = [:]
    var threadRenameRollbackSummaries: [String: GaryxThreadSummary] = [:]
    var threadRuntimeMutationIds: [String: GaryxThreadMutationID] = [:]
    var threadRuntimeRollbackSnapshots: [String: GaryxThreadRuntimeRollbackSnapshot] = [:]
    var claudeCodeAuthPollTask: Task<Void, Never>?
    var claudeCodeAuthPollGeneration: UUID?
    var claudeCodeAuthFlowGeneration = UUID()
    var claudeCodeAccountsLoadGeneration: UUID?
    var claudeCodeAccountMutationGeneration: UUID?
    #if DEBUG
    var debugSnapshotActive = false
    #endif

    init(
        defaults: UserDefaults = .standard,
        keychain: GaryxMobileKeychain = .shared,
        gatewayClientFactory: ((GaryxGatewayConfiguration) -> GaryxGatewayClient)? = nil,
        composerPayloadCoordinator: GaryxComposerPayloadCoordinator? = nil
    ) {
        let threadSummaryCache = GaryxThreadSummaryCache()
        let threadSummaryLeaseOwner = GaryxThreadSummaryLeaseOwner(cache: threadSummaryCache)
        let threadMutationHubStore = GaryxThreadMutationHubStore()
        self.threadSummaryCache = threadSummaryCache
        self.threadSummaryLeaseOwner = threadSummaryLeaseOwner
        self.threadMutationHubStore = threadMutationHubStore
        self.threadFavoritesProvider = GaryxFavoritesMembershipProvider(
            gatewayScope: "",
            cache: threadSummaryCache,
            leaseOwner: threadSummaryLeaseOwner
        )
        self.homeThreadListStore = GaryxHomeThreadListStore(
            mutationHubStore: threadMutationHubStore
        )
        self.composerPayloadCoordinator = composerPayloadCoordinator ?? .production()
        let restoredRecentThreadFilter = GaryxRecentThreadFilterStorage.load(
            defaults: defaults,
            key: GaryxMobileSettingsKeys.recentThreadFilter
        )
        self.defaults = defaults
        self.keychain = keychain
        self.gatewayClientFactory = gatewayClientFactory
        self.gatewayScopeEpochByIdentity = Self.loadGatewayScopeEpochs(defaults: defaults)
        self.pinnedOrderOutboxStore = GaryxPinnedOrderUserDefaultsStore(defaults: defaults)
        self.recentThreadFeeds = GaryxRecentThreadFeeds(
            pageLimit: Self.threadListPageLimit,
            overlap: Self.threadListPageOverlap,
            selectedFilter: restoredRecentThreadFilter
        )
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
        selectedAgentTargetId = nil
        gatewayDefaultAgentId = nil
        effectiveDefaultAgentId = nil
        newThreadWorkspaceSelection = GaryxDraftWorkspaceSelection.fromPersistedValue(
            defaults.string(forKey: GaryxMobileSettingsKeys.newThreadWorkspaceSelection)
        )
        newThreadWorkspaceMode = Self.normalizedWorkspaceMode(
            defaults.string(forKey: GaryxMobileSettingsKeys.newThreadWorkspaceMode)
        )
        loadGatewayScopedUserState(fallbackToLegacy: true)

        #if DEBUG
        let debugEnvironment = ProcessInfo.processInfo.environment
        if debugEnvironment["GARYX_MOBILE_DEBUG_SNAPSHOT"] == "1" {
            loadDebugSnapshot(recentFilter: .all)
            applyDebugDestination(
                panelName: debugEnvironment["GARYX_MOBILE_DEBUG_PANEL"],
                tabName: debugEnvironment["GARYX_MOBILE_DEBUG_SETTINGS_TAB"],
                showSidebar: debugEnvironment["GARYX_MOBILE_DEBUG_SIDEBAR"] == "1"
            )
        }
        #endif
        homeProjectionGateway.setResultHandler { [weak self] result in
            self?.applyHomeProjectionResult(result)
        }
        productionRouteStore.presentationBarrierActivated = { [weak self] in
            self?.forceTerminalGlobalRevealInteractions(.presentationBarrier)
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
