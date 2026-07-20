import QuartzCore
import SwiftUI

/// Non-observable lifecycle fan-out owned by one route container. Conversation
/// occurrences subscribe by identity so terminal activation and delivered
/// frames never invalidate the moving destination through environment writes.
@MainActor
final class GaryxRouteLifecycleRegistry {
    typealias Observer = @MainActor (GaryxRouteHostLifecyclePhase) -> Void
    typealias ContentReadyObserver = @MainActor () -> Void
    typealias PresentedFrameDemand = @MainActor () -> Bool
    typealias PresentedFrameObserver = @MainActor (TimeInterval?) -> Void

    private struct PresentedFrameSubscription {
        let demand: PresentedFrameDemand
        let observer: PresentedFrameObserver
    }

    private var phaseByIdentity: [GaryxRoutePresentationIdentity: GaryxRouteHostLifecyclePhase] = [:]
    private var observers: [GaryxRoutePresentationIdentity: [UUID: Observer]] = [:]
    private var contentReadyIdentities = Set<GaryxRoutePresentationIdentity>()
    private var contentReadyObservers: [
        GaryxRoutePresentationIdentity: [UUID: ContentReadyObserver]
    ] = [:]
    private var presentedFrameSubscriptions: [
        GaryxRoutePresentationIdentity: [UUID: PresentedFrameSubscription]
    ] = [:]
    private var previousPresentedFrameTimestamp: CFTimeInterval?

    var presentedFrameDemandDidBecomeActive: (@MainActor () -> Void)? {
        didSet {
            if hasPresentedFrameDemand {
                presentedFrameDemandDidBecomeActive?()
            }
        }
    }
    var contentPreparationDidBegin: (@MainActor (GaryxRoutePresentationIdentity) -> Void)?

    func hostMounted(_ identity: GaryxRoutePresentationIdentity) {
        update(identity, lifecycle: .mounted)
    }

    func update(
        _ identity: GaryxRoutePresentationIdentity,
        lifecycle: GaryxRouteHostLifecyclePhase
    ) {
        guard phaseByIdentity[identity] != lifecycle else { return }
        phaseByIdentity[identity] = lifecycle
        let currentObservers = observers[identity].map { Array($0.values) } ?? []
        for observer in currentObservers {
            observer(lifecycle)
        }
        if hasPresentedFrameDemand {
            presentedFrameDemandDidBecomeActive?()
        }
    }

    func hostUnmounted(_ identity: GaryxRoutePresentationIdentity) {
        phaseByIdentity.removeValue(forKey: identity)
        let removedObservers = observers.removeValue(forKey: identity)
            .map { Array($0.values) } ?? []
        contentReadyIdentities.remove(identity)
        contentReadyObservers.removeValue(forKey: identity)
        presentedFrameSubscriptions.removeValue(forKey: identity)
        for observer in removedObservers {
            observer(.disappeared)
        }
    }

    func observe(
        _ identity: GaryxRoutePresentationIdentity,
        observer: @escaping Observer
    ) -> UUID {
        let token = UUID()
        observers[identity, default: [:]][token] = observer
        observer(phaseByIdentity[identity] ?? .mounted)
        return token
    }

    func removeObserver(_ token: UUID, for identity: GaryxRoutePresentationIdentity) {
        observers[identity]?[token] = nil
        if observers[identity]?.isEmpty == true {
            observers[identity] = nil
        }
    }

    func observeContentReady(
        _ identity: GaryxRoutePresentationIdentity,
        observer: @escaping ContentReadyObserver
    ) -> UUID {
        let token = UUID()
        contentReadyObservers[identity, default: [:]][token] = observer
        if contentReadyIdentities.contains(identity) {
            observer()
        }
        return token
    }

    func removeContentReadyObserver(
        _ token: UUID,
        for identity: GaryxRoutePresentationIdentity
    ) {
        contentReadyObservers[identity]?[token] = nil
        if contentReadyObservers[identity]?.isEmpty == true {
            contentReadyObservers[identity] = nil
        }
    }

    func contentDidBecomeReady(_ identity: GaryxRoutePresentationIdentity) {
        guard contentReadyIdentities.insert(identity).inserted else { return }
        let currentObservers = contentReadyObservers[identity]
            .map { Array($0.values) } ?? []
        for observer in currentObservers {
            observer()
        }
        if hasPresentedFrameDemand {
            previousPresentedFrameTimestamp = nil
            presentedFrameDemandDidBecomeActive?()
        }
    }

    func observePresentedFrames(
        _ identity: GaryxRoutePresentationIdentity,
        demand: @escaping PresentedFrameDemand,
        observer: @escaping PresentedFrameObserver
    ) -> UUID {
        let token = UUID()
        presentedFrameSubscriptions[identity, default: [:]][token] =
            PresentedFrameSubscription(demand: demand, observer: observer)
        if phaseByIdentity[identity] == .active, demand() {
            presentedFrameDemandDidBecomeActive?()
        }
        return token
    }

    func removePresentedFrameObserver(
        _ token: UUID,
        for identity: GaryxRoutePresentationIdentity
    ) {
        presentedFrameSubscriptions[identity]?[token] = nil
        if presentedFrameSubscriptions[identity]?.isEmpty == true {
            presentedFrameSubscriptions[identity] = nil
        }
    }

    var hasPresentedFrameDemand: Bool {
        for (identity, subscriptions) in presentedFrameSubscriptions
        where phaseByIdentity[identity] == .active {
            if subscriptions.values.contains(where: { $0.demand() }) {
                return true
            }
        }
        return false
    }

    func presentedFrame() {
        let timestamp = CACurrentMediaTime()
        let interval = previousPresentedFrameTimestamp.map { timestamp - $0 }
        previousPresentedFrameTimestamp = timestamp
        let activeSubscriptions = presentedFrameSubscriptions.compactMap {
            identity, subscriptions -> [PresentedFrameSubscription]? in
            guard phaseByIdentity[identity] == .active else { return nil }
            return Array(subscriptions.values)
        }.flatMap { $0 }
        for subscription in activeSubscriptions where subscription.demand() {
            subscription.observer(interval)
        }
        if !hasPresentedFrameDemand {
            previousPresentedFrameTimestamp = nil
        }
    }
}

private struct GaryxRouteLifecycleRegistryKey: EnvironmentKey {
    static let defaultValue: GaryxRouteLifecycleRegistry? = nil
}

extension EnvironmentValues {
    var garyxRouteLifecycleRegistry: GaryxRouteLifecycleRegistry? {
        get { self[GaryxRouteLifecycleRegistryKey.self] }
        set { self[GaryxRouteLifecycleRegistryKey.self] = newValue }
    }
}

struct GaryxConversationOpeningMetadata: Equatable {
    let title: String
    let agentTarget: GaryxMobileAgentTarget?
    let transcriptPresentation: GaryxConversationOpeningTranscriptPresentation
    let usesTranscriptSnapshot: Bool
    let localRows: [GaryxMobileTurnRow]

    static let newThread = GaryxConversationOpeningMetadata(
        title: "New Thread",
        agentTarget: nil,
        transcriptPresentation: .localMessages,
        usesTranscriptSnapshot: false,
        localRows: []
    )
    static let prewarmLoading = GaryxConversationOpeningMetadata(
        title: "Conversation",
        agentTarget: nil,
        transcriptPresentation: .loading,
        usesTranscriptSnapshot: false,
        localRows: []
    )
    static let prewarmLocal = GaryxConversationOpeningMetadata(
        title: "Conversation",
        agentTarget: nil,
        transcriptPresentation: .localMessages,
        usesTranscriptSnapshot: false,
        localRows: GaryxConversationRenderPrewarmFixture.representativeRows
    )

    /// Reports the production opening treatment for the current push. A
    /// touch-prepared route has already fired SwiftUI `onAppear` before the
    /// probe starts, so the navigation owner also invokes this synchronously
    /// after the push transaction is admitted and before its first frame.
    @MainActor
    func markPushPresentation() {
        let probe = GaryxRoutePushPerformanceProbe.shared
        probe?.openingConversationPageMounted()
        probe?.markConversationHeaderLoadingIndicator()
        if transcriptPresentation == .loading {
            probe?.markConversationMessageLoading()
        } else if usesTranscriptSnapshot || !localRows.isEmpty {
            probe?.markConversationLocalMessages()
        }
    }
}

/// Non-observable route metadata captured before the destination host mounts.
/// Reading it cannot subscribe the moving page to the large mobile model.
@MainActor
final class GaryxConversationRouteMetadataCache {
    static let shared = GaryxConversationRouteMetadataCache()

    private var metadataByThreadID: [String: GaryxConversationOpeningMetadata] = [:]
    private var insertionOrder: [String] = []
    private let capacity = 16

    private init() {}

    func store(
        _ thread: GaryxThreadSummary,
        agentTarget: GaryxMobileAgentTarget?,
        localRows: [GaryxMobileTurnRow]
    ) {
        let title = thread.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let usesTranscriptSnapshot = GaryxConversationTranscriptSnapshotCache.shared
            .hasSnapshot(for: thread.id)
        let metadata = GaryxConversationOpeningMetadata(
            title: title.isEmpty ? "Thread" : title,
            agentTarget: agentTarget,
            transcriptPresentation: GaryxConversationOpeningTranscriptPolicy.presentation(
                localRenderableRowCount: localRows.count,
                hasRenderedSnapshot: usesTranscriptSnapshot
            ),
            usesTranscriptSnapshot: usesTranscriptSnapshot,
            localRows: usesTranscriptSnapshot ? [] : Array(localRows.suffix(2))
        )
        if metadataByThreadID[thread.id] == nil {
            insertionOrder.append(thread.id)
        }
        metadataByThreadID[thread.id] = metadata
        while insertionOrder.count > capacity {
            metadataByThreadID[insertionOrder.removeFirst()] = nil
        }
    }

    func metadata(for destination: GaryxRouteDestination) -> GaryxConversationOpeningMetadata {
        switch destination {
        case .conversation(let threadID):
            return metadataByThreadID[threadID]
                ?? GaryxConversationOpeningMetadata(
                    title: "Thread",
                    agentTarget: nil,
                    transcriptPresentation: .loading,
                    usesTranscriptSnapshot: false,
                    localRows: []
                )
        case .conversationDraft:
            return .newThread
        default:
            return GaryxConversationOpeningMetadata(
                title: "Thread",
                agentTarget: nil,
                transcriptPresentation: .loading,
                usesTranscriptSnapshot: false,
                localRows: []
            )
        }
    }
}

/// Conversation destination whose first frame is already the complete thread
/// page. Before terminal it presents real page chrome and transcript-local
/// loading only. The heavier live graph mounts behind that page after terminal
/// and takes over after delivered frames prove it stable.
struct GaryxConversationRouteView: View {
    @Environment(\.garyxRouteLifecycleRegistry) private var lifecycleRegistry
    @StateObject private var presentationDriver = GaryxConversationRoutePresentationDriver()

    let destination: GaryxRouteDestination
    let occurrenceID: GaryxRouteInstanceID

    private var openingMetadata: GaryxConversationOpeningMetadata {
        GaryxConversationRouteMetadataCache.shared.metadata(for: destination)
    }

    private var openingSnapshotThreadID: String? {
        guard case .conversation(let threadID) = destination else { return nil }
        return threadID
    }

    var body: some View {
        ZStack {
            if presentationDriver.renderPhase != .openingPage {
                // The live page owns the same local-first rule as every
                // ordinary conversation open: cached rows stay visible while
                // history refreshes, and only an empty transcript chooses its
                // message-local skeleton.
                GaryxConversationView(destination: destination)
                .allowsHitTesting(presentationDriver.renderPhase == .live)
                .onAppear {
                    GaryxRoutePushPerformanceProbe.shared?.conversationSurfaceMounted()
                }
            }

            GaryxConversationOpeningPageView(
                metadata: openingMetadata,
                snapshotThreadID: openingSnapshotThreadID
            )
                // Keep the prepared live tree in the compositor while this
                // visually opaque page is on top. An alpha of exactly one lets
                // Core Animation cull that tree and moves its pipeline work to
                // the reveal frame. Keep this lightweight tree mounted after
                // reveal: changing a compositor opacity is cheap, while tearing
                // down the complete SwiftUI chrome can drop the handoff frame.
                .opacity(presentationDriver.renderPhase == .live ? 0 : 0.999)
                .allowsHitTesting(presentationDriver.renderPhase != .live)
                // This is a pixel-continuity layer. The real page owns the
                // route's semantics throughout their brief overlap.
                .accessibilityHidden(true)
        }
        .onAppear {
            presentationDriver.connect(
                to: lifecycleRegistry,
                identity: .entry(occurrenceID)
            )
        }
        .onDisappear {
            presentationDriver.disconnect()
        }
    }

}

/// The exact production loading language used before route staging existed:
/// shared glass chrome, the shared ink spinner, the shared composer surface,
/// and either real cached rows or the shared transcript skeleton. This view is
/// intentionally model-free so canonical projection cannot invalidate it
/// while the push is moving.
struct GaryxConversationOpeningPageView: View {
    @Environment(\.garyxRouteNavigationActions) private var routeNavigation
    let metadata: GaryxConversationOpeningMetadata
    let snapshotThreadID: String?

    init(
        metadata: GaryxConversationOpeningMetadata,
        snapshotThreadID: String? = nil
    ) {
        self.metadata = metadata
        self.snapshotThreadID = snapshotThreadID
    }

    var body: some View {
        Group {
            if metadata.usesTranscriptSnapshot, let snapshotThreadID {
                GaryxConversationTranscriptSnapshotView(threadID: snapshotThreadID)
                    .accessibilityHidden(true)
            } else {
                openingTranscript
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .garyxPageBackground()
        .garyxAdaptiveTopBar {
            openingHeader
        }
        .garyxFloatingBottomChrome {
            GaryxConversationOpeningComposerChrome()
        }
        .onAppear {
            metadata.markPushPresentation()
        }
    }

    private var openingTranscript: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Color.clear.frame(height: 1)

                switch metadata.transcriptPresentation {
                case .loading:
                    GaryxThreadHistoryLoadingView()
                        .padding(.top, 12)
                case .localMessages:
                    if !metadata.localRows.isEmpty {
                        GaryxMobileTurnRowsView(rows: metadata.localRows)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 18)
            .padding(.bottom, 24)
            .garyxVerticalScrollContentWidth(alignment: .topLeading)

            Color.clear
                .frame(height: 24)
                .accessibilityHidden(true)
        }
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .defaultScrollAnchor(.bottom, for: .sizeChanges)
        .scrollDisabled(true)
    }

    private var openingHeader: some View {
        GaryxAdaptiveGlassContainer(spacing: 10) {
            HStack(spacing: 12) {
                Button {
                    routeNavigation.dismiss?()
                } label: {
                    GaryxToolbarIcon(systemName: "chevron.left")
                }
                .buttonStyle(GaryxPressableRowStyle())
                .accessibilityLabel("Back")
                .accessibilityHidden(true)

                GaryxThreadRuntimeCompactContentRow(
                    title: metadata.title,
                    target: metadata.agentTarget
                )
                .garyxAdaptiveGlass(.regular, isInteractive: false, in: Capsule())
                .accessibilityElement(children: .combine)
                .accessibilityLabel("\(metadata.title), thread settings")
                .accessibilityHidden(true)

                Spacer(minLength: 0)

                GaryxToolbarIcon {
                    GaryxInkSpinner()
                }
                .accessibilityLabel("Loading thread")
                .accessibilityHidden(true)
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
    }
}

/// Non-rendering adapter from UIKit's delivered-frame clock to the Core
/// presentation policy. No display-link representable is mounted in the
/// moving SwiftUI destination.
@MainActor
private final class GaryxConversationRoutePresentationDriver: ObservableObject {
    @Published private(set) var renderPhase: GaryxConversationRouteRenderPhase = .openingPage

    private var presentation = GaryxConversationRoutePresentationState()

    private weak var lifecycleRegistry: GaryxRouteLifecycleRegistry?
    private var observedIdentity: GaryxRoutePresentationIdentity?
    private var observationToken: UUID?
    private var contentReadyObservationToken: UUID?
    private var frameObservationToken: UUID?
    private var fallbackFrameTask: Task<Void, Never>?
    private var awaitsPresentedLiveReveal = false

    func connect(
        to registry: GaryxRouteLifecycleRegistry?,
        identity: GaryxRoutePresentationIdentity
    ) {
        if lifecycleRegistry === registry,
           observedIdentity == identity,
           observationToken != nil,
           contentReadyObservationToken != nil,
           frameObservationToken != nil {
            return
        }
        let startsNewOccurrence = observedIdentity != nil && observedIdentity != identity
        disconnect()
        if startsNewOccurrence {
            presentation = GaryxConversationRoutePresentationState()
            renderPhase = .openingPage
        }
        guard let registry else {
            apply(lifecycle: .active)
            scheduleFallbackFrames()
            return
        }
        lifecycleRegistry = registry
        observedIdentity = identity
        observationToken = registry.observe(identity) { [weak self] lifecycle in
            self?.apply(lifecycle: lifecycle)
        }
        contentReadyObservationToken = registry.observeContentReady(identity) { [weak self] in
            self?.messageContentDidBecomeReady()
        }
        frameObservationToken = registry.observePresentedFrames(
            identity,
            demand: { [weak self] in
                guard let self else { return false }
                return self.presentation.needsPresentedFrameClock
                    || self.awaitsPresentedLiveReveal
            },
            observer: { [weak self] interval in
                self?.presentedFrame(interval: interval)
            }
        )
    }

    func disconnect() {
        if let lifecycleRegistry, let observedIdentity, let observationToken {
            lifecycleRegistry.removeObserver(observationToken, for: observedIdentity)
        }
        if let lifecycleRegistry, let observedIdentity, let frameObservationToken {
            lifecycleRegistry.removePresentedFrameObserver(
                frameObservationToken,
                for: observedIdentity
            )
        }
        if let lifecycleRegistry, let observedIdentity, let contentReadyObservationToken {
            lifecycleRegistry.removeContentReadyObserver(
                contentReadyObservationToken,
                for: observedIdentity
            )
        }
        lifecycleRegistry = nil
        observedIdentity = nil
        observationToken = nil
        contentReadyObservationToken = nil
        frameObservationToken = nil
        fallbackFrameTask?.cancel()
        fallbackFrameTask = nil
        awaitsPresentedLiveReveal = false
        guard !presentation.hasPresentedLiveConversation else { return }
        var next = presentation
        _ = next.apply(lifecycle: .disappeared)
        commit(next)
    }

    private func apply(lifecycle: GaryxRouteHostLifecyclePhase) {
        var next = presentation
        let action = next.apply(lifecycle: lifecycle)
        commit(next)
        perform(action)
        scheduleFallbackFrames()
    }

    private func messageContentDidBecomeReady() {
        var next = presentation
        next.messageContentDidBecomeReady()
        commit(next)
        GaryxRoutePushPerformanceProbe.shared?.messagePreparationCompleted()
    }

    private func presentedFrame(interval: TimeInterval?) {
        if awaitsPresentedLiveReveal {
            awaitsPresentedLiveReveal = false
            // `renderPhase = .live` schedules the compositor handoff; it does
            // not mean that handoff is already on screen. Mark reveal after
            // the following delivered frame, then begin the strict visible
            // sampling window on the next cadence.
            DispatchQueue.main.async {
                GaryxRoutePushPerformanceProbe.shared?.liveConversationRevealBegan()
            }
            return
        }
        let previousPhase = presentation.renderPhase
        let previousMessagePhase = presentation.messagePhase
        var next = presentation
        _ = next.presentedFrame(interval: interval)
        commit(next)
        if previousMessagePhase != next.messagePhase,
           next.messagePhase == .loading {
            perform(.beginMessagePreparation)
        }
        guard previousPhase != next.renderPhase else { return }
        switch next.renderPhase {
        case .openingPage:
            break
        case .materializingConversation:
            GaryxRoutePushPerformanceProbe.shared?.liveConversationMaterializationBegan()
        case .live:
            awaitsPresentedLiveReveal = true
        }
    }

    private func perform(_ action: GaryxConversationRoutePresentationAction) {
        guard action == .beginMessagePreparation else { return }
        GaryxRoutePushPerformanceProbe.shared?.messagePreparationBegan()
        if let lifecycleRegistry, let observedIdentity {
            lifecycleRegistry.contentPreparationDidBegin?(observedIdentity)
        }
    }

    private func scheduleFallbackFrames() {
        guard lifecycleRegistry == nil,
              presentation.needsPresentedFrameClock || awaitsPresentedLiveReveal,
              fallbackFrameTask == nil
        else { return }
        fallbackFrameTask = Task { @MainActor [weak self] in
            while let self,
                  !Task.isCancelled,
                  self.lifecycleRegistry == nil,
                  self.presentation.needsPresentedFrameClock
                    || self.awaitsPresentedLiveReveal {
                await Task.yield()
                guard !Task.isCancelled else { return }
                self.presentedFrame(interval: 1.0 / 60.0)
            }
            self?.fallbackFrameTask = nil
        }
    }

    private func commit(_ next: GaryxConversationRoutePresentationState) {
        guard next != presentation else { return }
        presentation = next
        // Delivered-frame counters and message readiness are policy internals.
        // Publishing them would reevaluate the complete opening page on every
        // terminal display-link callback and can itself drop the next frame.
        // SwiftUI only observes the two surface implementation handoffs.
        if renderPhase != next.renderPhase {
            renderPhase = next.renderPhase
        }
    }
}
