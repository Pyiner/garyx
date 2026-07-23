import QuartzCore
import SwiftUI

/// Non-observable lifecycle fan-out owned by one route container. Conversation
/// occurrences subscribe by identity so terminal activation and delivered
/// frames never invalidate the moving destination through environment writes.
@MainActor
final class GaryxRouteLifecycleRegistry {
    typealias Observer = @MainActor (GaryxRouteHostLifecyclePhase) -> Void
    typealias PresentedFrameDemand = @MainActor () -> Bool
    typealias PresentedFrameObserver = @MainActor (TimeInterval?) -> Void

    struct ObservationCounts: Equatable {
        var lifecycle = 0
        var presentedFrames = 0
    }

    private struct PresentedFrameSubscription {
        let demand: PresentedFrameDemand
        let observer: PresentedFrameObserver
    }

    private var phaseByIdentity: [GaryxRoutePresentationIdentity: GaryxRouteHostLifecyclePhase] = [:]
    private var observers: [GaryxRoutePresentationIdentity: [UUID: Observer]] = [:]
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

    /// A deterministic diagnostic for route-wiring contract tests. Draft
    /// occurrences have no staged presentation driver and therefore keep both
    /// counts at zero; gateway-thread occurrences own one of each.
    func observationCounts(
        for identity: GaryxRoutePresentationIdentity
    ) -> ObservationCounts {
        ObservationCounts(
            lifecycle: observers[identity]?.count ?? 0,
            presentedFrames: presentedFrameSubscriptions[identity]?.count ?? 0
        )
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

    func presentedFrame(at timestamp: TimeInterval = CACurrentMediaTime()) {
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

    static let prewarmLocal = GaryxConversationOpeningMetadata(
        title: "Conversation",
        agentTarget: nil
    )

    /// Reports the production opening chrome for the current push. Transcript
    /// treatment is deliberately absent: it is derived from live model inputs
    /// in the mounted conversation and must never be frozen in this cache.
    @MainActor
    func markPushPresentation() {
        let probe = GaryxRoutePushPerformanceProbe.shared
        probe?.openingConversationPageMounted()
        probe?.markConversationHeaderLoadingIndicator()
    }
}

/// Staging inputs for one existing-thread transcript. Production header and
/// composer chrome stay mounted outside this handoff; the conversation derives
/// one live treatment and reports that same value to the frame-clock driver.
struct GaryxConversationTranscriptStaging {
    let metadata: GaryxConversationOpeningMetadata
    let snapshotThreadID: String
    let renderPhase: GaryxConversationRouteRenderPhase
    let allowsTranscriptInteraction: Bool
    let presentationInputDidChange: @MainActor (
        GaryxConversationTranscriptPresentationInput
    ) -> Void

    var mountsLiveTranscript: Bool {
        renderPhase != .openingPage
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
        agentTarget: GaryxMobileAgentTarget?
    ) {
        let title = thread.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let metadata = GaryxConversationOpeningMetadata(
            title: title.isEmpty ? "Thread" : title,
            agentTarget: agentTarget
        )
        if metadataByThreadID[thread.id] == nil {
            insertionOrder.append(thread.id)
        }
        metadataByThreadID[thread.id] = metadata
        while insertionOrder.count > capacity {
            metadataByThreadID[insertionOrder.removeFirst()] = nil
        }
    }

    func metadata(forThreadID threadID: String) -> GaryxConversationOpeningMetadata {
        metadataByThreadID[threadID]
            ?? GaryxConversationOpeningMetadata(
                title: "Thread",
                agentTarget: nil
            )
    }
}

/// Selects the presentation pipeline once for one route occurrence. A local
/// draft mounts the final production graph directly; only an existing gateway
/// thread owns the staged opening/materialization driver. Keeping the plan in
/// occurrence state also keeps an in-place draft promotion on the direct path.
struct GaryxConversationRouteView: View {
    @State private var presentationPlan: GaryxConversationRoutePresentationPlan

    let destination: GaryxRouteDestination
    let occurrenceID: GaryxRouteInstanceID

    init(destination: GaryxRouteDestination, occurrenceID: GaryxRouteInstanceID) {
        guard let presentationPlan = GaryxConversationRoutePresentationPolicy.plan(
            for: destination
        ) else {
            preconditionFailure("conversation route requires a conversation destination")
        }
        self.destination = destination
        self.occurrenceID = occurrenceID
        _presentationPlan = State(initialValue: presentationPlan)
    }

    @ViewBuilder
    var body: some View {
        switch (presentationPlan, destination) {
        case (.directLocal, .conversation),
             (.directLocal, .conversationDraft),
             (.stagedGatewayThread, .conversationDraft):
            // The last combination is an invariant fallback: even if an
            // occurrence were ever rewritten back to a draft, a draft must
            // never enter the gateway-thread opening pipeline.
            GaryxConversationView(destination: destination)
                .onAppear {
                    GaryxRoutePushPerformanceProbe.shared?.conversationSurfaceMounted()
                }
        case (.stagedGatewayThread, .conversation(let threadID)):
            GaryxStagedConversationRouteView(
                threadID: threadID,
                occurrenceID: occurrenceID
            )
        case (_, .panel), (_, .settingsDetail), (_, .workspaceDrilldown):
            EmptyView()
        }
    }
}

/// Existing gateway conversation whose production header and composer are live
/// from its first frame. Only the transcript is staged: its cached/loading
/// cover stays above the delayed live transcript until delivered frames prove
/// the handoff stable.
private struct GaryxStagedConversationRouteView: View {
    @Environment(\.garyxRouteLifecycleRegistry) private var lifecycleRegistry
    @StateObject private var presentationDriver = GaryxConversationRoutePresentationDriver()

    let threadID: String
    let occurrenceID: GaryxRouteInstanceID

    private var destination: GaryxRouteDestination {
        .conversation(threadID: threadID)
    }

    private var openingMetadata: GaryxConversationOpeningMetadata {
        GaryxConversationRouteMetadataCache.shared.metadata(forThreadID: threadID)
    }

    var body: some View {
        GaryxConversationView(
            destination: destination,
            transcriptStaging: GaryxConversationTranscriptStaging(
                metadata: openingMetadata,
                snapshotThreadID: threadID,
                renderPhase: presentationDriver.renderPhase,
                allowsTranscriptInteraction: presentationDriver.allowsTranscriptInteraction,
                presentationInputDidChange: { input in
                    presentationDriver.updateTranscriptPresentation(input)
                }
            )
        )
        .onAppear {
            openingMetadata.markPushPresentation()
            presentationDriver.connect(
                to: lifecycleRegistry,
                identity: .entry(occurrenceID)
            )
            markLiveTranscriptMaterializationIfNeeded()
        }
        .onChange(of: presentationDriver.renderPhase) { oldPhase, newPhase in
            guard oldPhase == .openingPage, newPhase != .openingPage else { return }
            markLiveTranscriptMaterializationIfNeeded()
        }
        .onDisappear {
            presentationDriver.disconnect()
        }
    }

    private func markLiveTranscriptMaterializationIfNeeded() {
        guard presentationDriver.renderPhase != .openingPage else { return }
        GaryxRoutePushPerformanceProbe.shared?.conversationSurfaceMounted()
        // This marks local transcript-graph materialization, not the
        // independent network history refresh.
        GaryxRoutePushPerformanceProbe.shared?.messagePreparationCompleted()
    }
}

/// One opaque, model-free continuity surface for the transcript viewport. The
/// owning conversation supplies production chrome and derives the treatment;
/// this view only renders the Core-selected skeleton or cached-pixel cover.
struct GaryxConversationOpeningTranscriptView: View {
    let cover: GaryxConversationOpeningTranscriptCover
    let snapshotThreadID: String?

    init(
        cover: GaryxConversationOpeningTranscriptCover,
        snapshotThreadID: String? = nil
    ) {
        self.cover = cover
        self.snapshotThreadID = snapshotThreadID
    }

    var body: some View {
        ZStack {
            GaryxTheme.background

            switch cover {
            case .snapshotPixels:
                if let snapshotThreadID {
                    GaryxConversationTranscriptSnapshotView(threadID: snapshotThreadID)
                        .onAppear {
                            GaryxRoutePushPerformanceProbe.shared?
                                .markConversationLocalMessages()
                        }
                        .accessibilityHidden(true)
                }
            case .skeleton:
                loadingTranscript
                    .onAppear {
                        GaryxRoutePushPerformanceProbe.shared?
                            .markConversationMessageLoading()
                    }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var loadingTranscript: some View {
        ScrollView {
            ZStack(alignment: .topLeading) {
                Color.clear
                    .containerRelativeFrame(.vertical) { length, _ in length }
                    .allowsHitTesting(false)

                VStack(alignment: .leading) {
                    VStack(alignment: .leading, spacing: 14) {
                        Color.clear.frame(height: 1)

                        GaryxThreadHistoryLoadingView()
                            .padding(.top, 12)
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 18)
                    .padding(.bottom, 24)
                    .garyxVerticalScrollContentWidth(alignment: .topLeading)

                    Color.clear
                        .frame(height: 24)
                        .accessibilityHidden(true)
                }
            }
            .background {
                GaryxTranscriptBlankSpaceTapLayer(action: {})
            }
        }
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .defaultScrollAnchor(.bottom, for: .sizeChanges)
        // Prewarm the exact production transcript scroll dismissal pipeline;
        // this continuity surface never receives interaction itself.
        .scrollDismissesKeyboard(.interactively)
        .scrollDisabled(true)
    }
}

/// Non-rendering adapter from UIKit's delivered-frame clock to the Core
/// presentation policy. No display-link representable is mounted in the
/// moving SwiftUI destination.
@MainActor
private final class GaryxConversationRoutePresentationDriver: ObservableObject {
    @Published private(set) var renderPhase: GaryxConversationRouteRenderPhase = .openingPage

    private var presentation = GaryxConversationRoutePresentationState()

    var allowsTranscriptInteraction: Bool {
        presentation.allowsTranscriptInteraction
    }

    private weak var lifecycleRegistry: GaryxRouteLifecycleRegistry?
    private var observedIdentity: GaryxRoutePresentationIdentity?
    private var observationToken: UUID?
    private var frameObservationToken: UUID?
    private var fallbackFrameTask: Task<Void, Never>?
    private var awaitsPresentedLiveReveal = false
    private var latestTranscriptPresentationInput:
        GaryxConversationTranscriptPresentationInput?

    func updateTranscriptPresentation(
        _ input: GaryxConversationTranscriptPresentationInput
    ) {
        guard latestTranscriptPresentationInput != input else { return }
        latestTranscriptPresentationInput = input
        reconcileTranscriptPresentation()
        scheduleFallbackFrames()
    }

    func connect(
        to registry: GaryxRouteLifecycleRegistry?,
        identity: GaryxRoutePresentationIdentity
    ) {
        if lifecycleRegistry === registry,
           observedIdentity == identity,
           observationToken != nil,
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
        lifecycleRegistry = nil
        observedIdentity = nil
        observationToken = nil
        frameObservationToken = nil
        fallbackFrameTask?.cancel()
        fallbackFrameTask = nil
        awaitsPresentedLiveReveal = false
        guard !presentation.hasPresentedLiveTranscript else { return }
        var next = presentation
        next.apply(lifecycle: .disappeared)
        commit(next)
    }

    private func apply(lifecycle: GaryxRouteHostLifecyclePhase) {
        let previousPhase = presentation.renderPhase
        let hadBegunContentPreparation = presentation.hasBegunContentPreparation
        var next = presentation
        next.apply(lifecycle: lifecycle)
        if let latestTranscriptPresentationInput {
            _ = next.reconcileTranscriptPresentation(latestTranscriptPresentationInput)
        }
        commit(next)
        handleTransition(
            previousPhase: previousPhase,
            hadBegunContentPreparation: hadBegunContentPreparation,
            next: next
        )
        scheduleFallbackFrames()
    }

    private func reconcileTranscriptPresentation() {
        guard let latestTranscriptPresentationInput else { return }
        let previousPhase = presentation.renderPhase
        let hadBegunContentPreparation = presentation.hasBegunContentPreparation
        var next = presentation
        _ = next.reconcileTranscriptPresentation(latestTranscriptPresentationInput)
        commit(next)
        handleTransition(
            previousPhase: previousPhase,
            hadBegunContentPreparation: hadBegunContentPreparation,
            next: next
        )
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
        let hadBegunContentPreparation = presentation.hasBegunContentPreparation
        var next = presentation
        _ = next.presentedFrame(interval: interval)
        commit(next)
        handleTransition(
            previousPhase: previousPhase,
            hadBegunContentPreparation: hadBegunContentPreparation,
            next: next
        )
    }

    private func handleTransition(
        previousPhase: GaryxConversationRouteRenderPhase,
        hadBegunContentPreparation: Bool,
        next: GaryxConversationRoutePresentationState
    ) {
        if !hadBegunContentPreparation, next.hasBegunContentPreparation {
            beginContentPreparation()
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

    private func beginContentPreparation() {
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
        // Delivered-frame counters and the one-shot preparation flag are
        // policy internals.
        // Publishing them would reevaluate the complete opening page on every
        // terminal display-link callback and can itself drop the next frame.
        // SwiftUI only observes the two surface implementation handoffs.
        if renderPhase != next.renderPhase {
            renderPhase = next.renderPhase
        }
    }
}
