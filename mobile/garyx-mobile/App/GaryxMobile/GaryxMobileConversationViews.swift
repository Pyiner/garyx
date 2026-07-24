import Foundation
import ImageIO
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

private let garyxHistoryPrefetchBoundaryRows = 3

private func garyxDismissKeyboard() {
    UIApplication.shared.sendAction(
        #selector(UIResponder.resignFirstResponder),
        to: nil,
        from: nil,
        for: nil
    )
}

/// Transcript content-plane background shared by startup prewarming and the
/// live message region. Its owner supplies the resolved content size; placing
/// it behind row content makes blank taps explicit without participating in
/// link, long-press, or disclosure hit testing.
struct GaryxTranscriptBlankSpaceTapLayer: View {
    let action: () -> Void

    var body: some View {
        Color.clear
            .contentShape(Rectangle())
            .onTapGesture(perform: action)
    }
}

/// Single preference key carrying all transcript content edges. The top
/// sentinel, intrinsic tail, and bottom anchor contribute one atomic value
/// that SwiftUI reduces within a layout pass. Do not split the edges back
/// into separate keys: multiple
/// callbacks make every scroll step look like a content-height change and
/// permanently reset the state machine's upward-travel accumulator
/// (#TASK-2073 P2).
private struct GaryxConversationContentEdgesKey: PreferenceKey {
    static var defaultValue = GaryxConversationContentEdges()

    static func reduce(
        value: inout GaryxConversationContentEdges,
        nextValue: () -> GaryxConversationContentEdges
    ) {
        value = value.merging(nextValue())
    }
}

/// Plain (non-observable) holder for the conversation scroll state machine.
/// Scroll measurements mutate it on every frame; keeping it out of SwiftUI
/// state means that churn never re-evaluates the transcript body.
private final class GaryxConversationScrollStateBox {
    var state = GaryxConversationScrollState()
}

/// Plain holder for retry-chain arbitration. Scheduling a scroll must not
/// invalidate the conversation body just to advance an internal token.
private final class GaryxConversationScrollSchedulerBox {
    var state = GaryxConversationScrollScheduler()
}

/// Live route to the UIScrollView hosting the conversation transcript.
///
/// Deliberately NOT a cached weak scroll-view reference: SwiftUI can replace
/// its hosting scroll view without moving the content to a new window, which
/// zeroes a stored weak handle with no re-resolve trigger (#TASK-2088 round
/// 2 — the compensation silently never ran). Instead the box keeps the
/// resolver view that lives INSIDE the scroll content — it is reparented
/// together with the content — and walks its superview chain at every use,
/// so the walk always lands on the current host.
private final class GaryxConversationHostScrollViewBox {
    weak var resolver: UIView?

    func currentScrollView() -> UIScrollView? {
        var candidate: UIView? = resolver?.superview
        while let view = candidate, !(view is UIScrollView) {
            candidate = view.superview
        }
        return candidate as? UIScrollView
    }
}

/// Plain (non-observable) store of every turn row's minY in the transcript
/// content coordinate space, fed by `GaryxMobileTurnRowsView`. Content-space
/// positions are scroll-invariant, so scrolling writes nothing here; only
/// layout changes do. Older-history prepend compensation reads the anchor
/// row's displacement out of this box — the exact height inserted above it.
private final class GaryxTurnRowGeometryBox {
    private var minYByRowId: [String: CGFloat] = [:]
    private(set) var intrinsicTailMinY: CGFloat?

    func record(_ rowId: String, minY: CGFloat) {
        minYByRowId[rowId] = minY
    }

    func minY(of rowId: String) -> CGFloat? {
        minYByRowId[rowId]
    }

    func recordIntrinsicTail(minY: CGFloat) {
        intrinsicTailMinY = minY
    }

    func contentBelowAnchorHeight(anchorRowId: String) -> CGFloat? {
        guard let anchorMinY = minY(of: anchorRowId),
              let intrinsicTailMinY else {
            return nil
        }
        return max(0, intrinsicTailMinY - anchorMinY)
    }

    func bottommostRow() -> (id: String, minY: CGFloat)? {
        minYByRowId.max { lhs, rhs in lhs.value < rhs.value }
            .map { (id: $0.key, minY: $0.value) }
    }

    /// Drop rows that left the transcript so a long session cannot grow the
    /// map without bound.
    func retain(only rowIds: Set<String>) {
        minYByRowId = minYByRowId.filter { rowIds.contains($0.key) }
    }
}

/// Invisible bridge view registering itself into
/// `GaryxConversationHostScrollViewBox` so the box can walk the live
/// superview chain on demand. It carries no scroll-view state of its own.
private struct GaryxEnclosingScrollViewReader: UIViewRepresentable {
    let box: GaryxConversationHostScrollViewBox

    /// Registration happens on window ENTRY, never from make/update: SwiftUI
    /// tears the representable down and up when the scroll view's identity
    /// changes (thread open), and the dying instance's final `updateUIView`
    /// would otherwise overwrite the box with a view that deallocates a beat
    /// later, zeroing the weak handle for good (observed on-device,
    /// #TASK-2088 round 3). Entry/exit callbacks converge in either order:
    /// the incoming view registers itself; the outgoing view only clears the
    /// box when it is still the registered one.
    final class ResolverView: UIView {
        weak var box: GaryxConversationHostScrollViewBox?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            if window != nil {
                box?.resolver = self
            } else if box?.resolver === self {
                box?.resolver = nil
            }
        }
    }

    func makeUIView(context: Context) -> ResolverView {
        let view = ResolverView()
        view.isUserInteractionEnabled = false
        view.isAccessibilityElement = false
        view.box = box
        return view
    }

    func updateUIView(_ uiView: ResolverView, context: Context) {
        uiView.box = box
    }
}

struct GaryxMessageBubbleActions {
    var model: GaryxMobileModel?
    var localFilePreview: @MainActor (_ target: String, _ reportsError: Bool) async -> GaryxWorkspaceFilePreview?
    var retryFailedUserMessage: @MainActor (_ messageId: String) async -> Bool
    var selectTaskNotification: @MainActor (GaryxTaskNotificationSelection) -> Void

    static let empty = GaryxMessageBubbleActions(
        model: nil,
        localFilePreview: { _, _ in nil },
        retryFailedUserMessage: { _ in false },
        selectTaskNotification: { _ in }
    )
}

private struct GaryxMessageBubbleActionsKey: EnvironmentKey {
    static let defaultValue = GaryxMessageBubbleActions.empty
}

extension EnvironmentValues {
    var garyxMessageBubbleActions: GaryxMessageBubbleActions {
        get { self[GaryxMessageBubbleActionsKey.self] }
        set { self[GaryxMessageBubbleActionsKey.self] = newValue }
    }
}

struct GaryxConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxRouteContext) private var routeContext
    @Environment(\.garyxSidebarDragActive) private var sidebarDragActive
    @Environment(\.garyxMotion) private var motion
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @FocusState private var isComposerFocused: Bool
    private let liveStore: GaryxConversationLiveStore
    private let transcriptStaging: GaryxConversationTranscriptStaging?
    /// Unified scroll state machine (GaryxMobileCore). The view feeds it
    /// events and executes the tail-scroll requests it returns; UI such as
    /// the scroll-to-bottom control reads its projections.
    // The scroll state machine lives in a plain reference box so the
    // per-frame scroll measurements feeding it never invalidate the
    // conversation body; `showsScrollToBottomButton` is the only scroll
    // fact the body reads, mirrored into SwiftUI state when it flips.
    @State private var scrollStateBox = GaryxConversationScrollStateBox()
    @State private var hostScrollViewBox = GaryxConversationHostScrollViewBox()
    @State private var rowGeometryBox = GaryxTurnRowGeometryBox()
    @State private var showsScrollToBottomButton = false
    @State private var pendingHistoryPrefetchThreadId: String?
    @State private var bottomChromeHeight: CGFloat = 0
    @State private var scrollSchedulerBox = GaryxConversationScrollSchedulerBox()
    @State private var sendAnchorFillerState = GaryxSendAnchorFillerState()
    @State private var sendAnchorFillerHeight: CGFloat = 0
    /// Mirror of `scrollStateBox.state.isSendAnchored`, mirrored only on
    /// flips (like the scroll-to-bottom button) so per-frame measurement
    /// churn never re-evaluates the body. Suspends the size-change bottom
    /// anchor during a send-anchor session; flipping OFF is the single
    /// owner of filler collapse (v2.1) — every session exit (gesture,
    /// exhaustion, scroll-to-bottom, thread switch, rollback) funnels
    /// through it.
    @State private var sendAnchorSessionActive = false
    @State private var readingAnchorRestoreGeneration = 0
    @State private var tailThinkingPresentationState = GaryxTailThinkingPresentationState()
    @State private var showsDebouncedTailThinking = false
    @State private var tailThinkingDebounceGeneration = 0
    @State private var taskNotificationSelectionState = GaryxTaskNotificationSelectionState()
    /// Runtime-panel morph state machine. `Presented` mounts the overlay
    /// surface (collapsed, exactly over the top-bar capsule) and hides the
    /// in-bar capsule; `Expanded` drives the spring morph. Close animates
    /// `Expanded` back first, then unmounts on completion.
    @State private var runtimePanelPresented = false
    @State private var runtimePanelExpanded = false
    init(
        destination: GaryxRouteDestination,
        transcriptStaging: GaryxConversationTranscriptStaging? = nil
    ) {
        liveStore = GaryxConversationLiveStore(destination: destination)
        self.transcriptStaging = transcriptStaging
    }

    var body: some View {
        ScrollViewReader { proxy in
            let turnRows = routeTurnRows
            let presentationInput = transcriptPresentationInput(turnRows: turnRows)
            let transcriptPresentation = transcriptPresentation(for: presentationInput)

            ZStack(alignment: .bottom) {
                if mountsLiveTranscript(for: transcriptPresentation) {
                    liveTranscript(
                        proxy: proxy,
                        treatment: presentationInput.treatment,
                        turnRows: turnRows
                    )
                    .allowsHitTesting(
                        allowsTranscriptInteraction(for: transcriptPresentation)
                    )
                    .accessibilityHidden(transcriptPresentation.showsOpeningCover)
                }

                if case .openingCover(let cover) = transcriptPresentation,
                   let transcriptStaging
                {
                    GaryxConversationOpeningTranscriptView(
                        cover: cover,
                        snapshotThreadID: transcriptStaging.snapshotThreadID
                    )
                    .allowsHitTesting(true)
                    .accessibilityHidden(true)
                }

                // The new-thread empty state lives outside the transcript
                // scroll so it stays anchored between the header and the
                // composer. The bottom padding lifts the visual center above
                // the true midpoint (by half the padding) so the title and
                // workspace picker sit slightly high on the page.
                if showsNewThreadEmptyState {
                    GaryxEmptyConversationView()
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
                        .padding(.bottom, 100)
                }
            }
            // Floating long-press menus render here, outside the transcript
            // scroll, so panels are never clipped and the pressed message
            // itself stays untouched.
            .garyxMessageMenuHost(
                bottomInset: bottomChromeHeight,
                dismissToken: messageMenuDismissToken
            )
            // Scroll-to-bottom hovers directly above the composer. It lives
            // INSIDE the bottom chrome: hosting it in a content overlay made
            // the safe-area inset shift its visuals without its hit-test
            // region, so taps fell through to the transcript rows behind it.
            .garyxFloatingBottomChrome(onHeightChange: { height in
                bottomChromeHeight = height
            }) {
                VStack(spacing: 12) {
                    if showsScrollToBottomButton {
                        Button {
                            updateScrollState(proxy: proxy) { $0.scrollToBottomTapped() }
                        } label: {
                            Image(systemName: "arrow.down")
                                .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                                .foregroundStyle(.primary)
                                .frame(width: 42, height: 42)
                                // Glass is decoration only: an iOS 26
                                // glassEffect applied to this button gets no
                                // working hit-test region inside the bottom
                                // chrome, so taps fell through to transcript
                                // rows (verified by tap bisection on 26.2).
                                // The tap target is the explicit content
                                // shape; the glass circle never hit-tests.
                                .background {
                                    Circle()
                                        .fill(Color.clear)
                                        .garyxAdaptiveGlass(
                                            .regular,
                                            isInteractive: false,
                                            in: Circle()
                                        )
                                        .allowsHitTesting(false)
                                }
                                .contentShape(Circle())
                                .shadow(color: Color.black.opacity(0.12), radius: 14, x: 0, y: 8)
                        }
                        .buttonStyle(GaryxPressableRowStyle())
                        .garyxMaterializeTransition(
                            .scrollLatest,
                            anchor: .bottom
                        )
                        .accessibilityLabel("Scroll to latest message")
                    }

                    if model.isNewThreadAgentBindingUnavailable {
                        Label("Enable an agent to start a new thread", systemImage: "exclamationmark.circle")
                            .font(GaryxFont.caption(weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 18)
                            .accessibilityIdentifier("new-thread-agent-unavailable")
                    }

                    GaryxComposer(
                        payload: model.composerPayloadCoordinator,
                        isFocused: $isComposerFocused
                    )
                        .disabled(model.isNewThreadAgentBindingUnavailable)
                }
                .frame(maxWidth: .infinity)
                .animation(motion.animation(.scrollLatest), value: showsScrollToBottomButton)
            }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onAppear {
                    reportTranscriptPresentationInput(presentationInput)
                }
                .onChange(of: presentationInput) { _, input in
                    reportTranscriptPresentationInput(input)
                }
        }
        .garyxPageBackground()
        .garyxAdaptiveTopBar {
            GaryxConversationHeader(
                liveStore: liveStore,
                stagedMetadata: stagedHeaderMetadata,
                showsStagedLoading: showsStagedHeaderLoading,
                preparesRuntimeModels: preparesHeaderRuntimeModels,
                isRuntimePanelPresented: runtimePanelPresented,
                onToggleRuntimePanel: {
                    setRuntimePanelVisible(!runtimePanelPresented)
                },
                onDismissRuntimePanel: {
                    setRuntimePanelVisible(false)
                }
            )
        }
        .overlayPreferenceValue(GaryxThreadRuntimeChromeAnchorKey.self) { anchor in
            runtimeSettingsOverlay(anchor: anchor)
        }
        // Task-tree sidebar overlays the whole conversation surface, header
        // included, so the scrim blocks every control behind the open panel.
        .garyxTaskTreeSidebarSurface()
        .environment(\.garyxMessageBubbleActions, messageBubbleActions)
        .onChange(of: taskNotificationPresentationScope) { _, scope in
            _ = taskNotificationSelectionState.synchronize(scope: scope)
        }
        .garyxFullScreenCover(item: taskNotificationSelectionBinding) { selection in
            GaryxTaskNotificationFullScreenView(notification: selection.notification) {
                taskNotificationSelectionState.dismiss()
            }
        }
        // Capsule card tapped in the transcript: present the focused preview
        // above this conversation and dismiss back to it (never switch to the
        // Capsules overview).
        .garyxFullScreenCover(item: $model.conversationCapsulePreview) { selection in
            GaryxCapsuleFocusedPreviewView(selection: selection)
        }
        // Route-time deletion validation: re-fires when the thread changes and
        // when capsule cards first appear (history can arrive after the thread is
        // selected, so a one-shot check on thread id alone would miss them).
        // Refreshing the capsules list prunes a remotely-deleted capsule's cached
        // preview HTML and bumps the cache epoch, so mounted chat thumbnails
        // re-validate to "deleted".
        .task(
            id: "\(liveStore.routeIdentity):\(mountsLiveTranscript):"
                + "\(liveStore.hasCapsuleCards(in: model))"
        ) {
            guard mountsLiveTranscript,
                  liveStore.hasCapsuleCards(in: model) else { return }
            await model.refreshCapsules()
        }
    }

    private var mountsLiveTranscript: Bool {
        transcriptStaging?.mountsLiveTranscript ?? true
    }

    private func transcriptPresentationInput(
        turnRows: [GaryxMobileTurnRow]
    ) -> GaryxConversationTranscriptPresentationInput {
        let hasTranscriptSnapshotPixels = liveStore.threadID.map {
            GaryxConversationTranscriptSnapshotCache.shared.hasSnapshot(for: $0)
        } ?? false
        let treatment = GaryxConversationTranscriptTreatmentPolicy.treatment(
            localRenderableRowCount: turnRows.count,
            hasRenderedSnapshot: liveStore.hasRenderedSnapshot(in: model),
            hasTranscriptSnapshotPixels: hasTranscriptSnapshotPixels,
            isAwaitingInitialHistory: liveStore.isAwaitingInitialHistory(
                in: model,
                isCanonicalTop: routeContext.isCanonicalTop
            )
        )
        return GaryxConversationTranscriptPresentationInput(
            treatment: treatment,
            hasTranscriptSnapshotPixels: hasTranscriptSnapshotPixels
        )
    }

    private func transcriptPresentation(
        for input: GaryxConversationTranscriptPresentationInput
    ) -> GaryxConversationTranscriptPresentation {
        guard let transcriptStaging else {
            return .live(input.treatment)
        }
        return GaryxConversationTranscriptPresentationPolicy.presentation(
            renderPhase: transcriptStaging.renderPhase,
            input: input
        )
    }

    private func mountsLiveTranscript(
        for presentation: GaryxConversationTranscriptPresentation
    ) -> Bool {
        switch presentation {
        case .live:
            true
        case .openingCover:
            mountsLiveTranscript
        }
    }

    private func allowsTranscriptInteraction(
        for presentation: GaryxConversationTranscriptPresentation
    ) -> Bool {
        guard case .live = presentation else { return false }
        return transcriptStaging?.allowsTranscriptInteraction ?? true
    }

    private func reportTranscriptPresentationInput(
        _ input: GaryxConversationTranscriptPresentationInput
    ) {
        transcriptStaging?.presentationInputDidChange(input)
    }

    private var stagedHeaderMetadata: GaryxConversationOpeningMetadata? {
        // The model-free metadata protects the moving destination only. Move
        // the production runtime-backed header into place as soon as the
        // terminal materialization window opens so reveal itself is only a
        // transcript compositor handoff.
        guard transcriptStaging?.renderPhase == .openingPage else { return nil }
        return transcriptStaging?.metadata
    }

    private var showsStagedHeaderLoading: Bool {
        transcriptStaging?.renderPhase == .openingPage && transcriptStaging != nil
    }

    private var preparesHeaderRuntimeModels: Bool {
        transcriptStaging?.renderPhase != .openingPage
    }

    private func liveTranscript(
        proxy: ScrollViewProxy,
        treatment: GaryxConversationTranscriptTreatment,
        turnRows: [GaryxMobileTurnRow]
    ) -> some View {
        messageScroll(
            proxy: proxy,
            treatment: treatment,
            turnRows: turnRows
        )
            .onAppear {
                GaryxConversationSendJitterProbe.shared?.attach(
                    routeIdentity: liveStore.routeIdentity,
                    scrollView: { [hostScrollViewBox] in
                        hostScrollViewBox.currentScrollView()
                    },
                    bottommostRow: { [rowGeometryBox] in
                        rowGeometryBox.bottommostRow()
                    },
                    rowMinY: { [rowGeometryBox] rowID in
                        rowGeometryBox.minY(of: rowID)
                    }
                )
                resetSendAnchorFiller()
                updateScrollState(proxy: proxy) { $0.threadOpened() }
                if isComposerFocused {
                    updateScrollState(proxy: proxy) { $0.composerFocused() }
                }
                resetTailThinkingPresentation(proxy: proxy)
                scheduleTranscriptSnapshot(rowIDs: routeTurnRows.map(\.id))
            }
            .onDisappear {
                GaryxConversationSendJitterProbe.shared?.detach(
                    routeIdentity: liveStore.routeIdentity
                )
            }
            .onChange(of: conversationScrollIdentity) { _, _ in
                setRuntimePanelVisible(false)
                pendingHistoryPrefetchThreadId = nil
                resetSendAnchorFiller()
                updateScrollState(proxy: proxy) { $0.threadOpened() }
                resetTailThinkingPresentation(proxy: proxy)
            }
            .onChange(of: messageScrollObservation) { oldValue, newValue in
                defer {
                    prefetchOlderHistoryIfNeeded()
                }
                if let localSend = newValue.localSendPresentation,
                   localSend != oldValue.localSendPresentation,
                   localSend.scopeIdentity == newValue.scopeIdentity {
                    beginSendAnchorFiller(anchorRowId: localSend.anchorRowId)
                    updateScrollState(proxy: proxy) {
                        $0.localSendPresented(anchorRowId: localSend.anchorRowId)
                    }
                } else if let cancelledSend = oldValue.localSendPresentation,
                          newValue.localSendPresentation == nil,
                          scrollStateBox.state.sendAnchorRowId == cancelledSend.anchorRowId {
                    // The send ended without a run to anchor for: durable
                    // rollback (the send never existed) or a terminal
                    // dispatch failure (busy / network / auth — the failed
                    // row stays with its error state). Remove the run space
                    // and restore ordinary opening ownership. A reader who
                    // already scrolled away keeps their position; their run
                    // space retires through the scroll-to-bottom control.
                    resetSendAnchorFiller()
                    updateScrollState(proxy: proxy) { $0.threadOpened() }
                }
                updateScrollState(proxy: proxy) {
                    $0.messagesChanged(
                        previous: oldValue.value,
                        current: newValue.value,
                        id: \.id,
                        previousScopeIdentity: oldValue.scopeIdentity,
                        currentScopeIdentity: newValue.scopeIdentity,
                        hasTailContent: !newValue.value.isEmpty || showsPresentedTailThinking
                    )
                }
                scheduleTranscriptSnapshot(rowIDs: routeTurnRows.map(\.id))
            }
            .onChange(of: renderRowScrollObservation) { oldValue, newValue in
                let restore = scrollStateBox.state.renderRowsChanged(
                    previousIds: oldValue.value,
                    currentIds: newValue.value,
                    previousScopeIdentity: oldValue.scopeIdentity,
                    currentScopeIdentity: newValue.scopeIdentity,
                    hasTailContent: !newValue.value.isEmpty || showsPresentedTailThinking
                )
                if let restore {
                    // Captured BEFORE the new rows lay out: the geometry box
                    // still holds the anchor row's pre-prepend content-space
                    // position.
                    scheduleReadingAnchorRestore(
                        restore,
                        capturedAnchorMinY: rowGeometryBox.minY(of: restore.anchorRowId),
                        proxy: proxy
                    )
                }
                rowGeometryBox.retain(only: Set(newValue.value))
                scheduleTranscriptSnapshot(rowIDs: newValue.value)
            }
            .onChange(of: liveStore.tailThinkingPresentationMode(in: model)) { _, _ in
                syncTailThinkingPresentation(proxy: proxy)
            }
            .onChange(of: isComposerFocused) { _, isFocused in
                guard isFocused else { return }
                updateScrollState(proxy: proxy) { $0.composerFocused() }
            }
            .onChange(of: bottomChromeHeight) { _, _ in
                reconcileSendAnchorFiller(proxy: proxy)
                updateScrollState(proxy: proxy) { $0.bottomChromeChanged() }
            }
    }

    @ViewBuilder
    private func runtimeSettingsOverlay(anchor: Anchor<CGRect>?) -> some View {
        if runtimePanelPresented, let anchor {
            GeometryReader { geometry in
                ZStack(alignment: .topLeading) {
                    Color.black.opacity(runtimePanelExpanded ? 0.10 : 0)
                        .ignoresSafeArea()
                        .contentShape(Rectangle())
                        .onTapGesture {
                            setRuntimePanelVisible(false)
                        }
                        .accessibilityLabel("Close thread settings")
                        .accessibilityAddTraits(.isButton)

                    // One glass surface morphs from the capsule's anchor rect
                    // to the wide panel — Dynamic Island style.
                    GaryxThreadRuntimeMorphSurface(
                        isExpanded: runtimePanelExpanded,
                        anchorRect: geometry[anchor],
                        containerSize: geometry.size,
                        onClose: {
                            setRuntimePanelVisible(false)
                        }
                    )
                    .environmentObject(model)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
        }
    }

    private func setRuntimePanelVisible(_ visible: Bool) {
        if visible {
            guard !runtimePanelPresented else { return }
            garyxDismissKeyboard()
            // Mount the surface collapsed, exactly over the capsule…
            runtimePanelPresented = true
            guard let animation = motion.animation(.morphOpen) else {
                runtimePanelExpanded = true
                return
            }
            // …then spring it open on the next main-actor turn so the morph
            // visibly starts from the capsule rect.
            Task { @MainActor in
                withAnimation(animation) {
                    runtimePanelExpanded = true
                }
            }
        } else {
            guard runtimePanelPresented else { return }
            guard let animation = motion.animation(.morphClose), runtimePanelExpanded else {
                runtimePanelExpanded = false
                runtimePanelPresented = false
                return
            }
            withAnimation(
                animation,
                completionCriteria: .logicallyComplete
            ) {
                runtimePanelExpanded = false
            } completion: {
                runtimePanelPresented = false
            }
        }
    }

    private var messageBubbleActions: GaryxMessageBubbleActions {
        GaryxMessageBubbleActions(
            model: model,
            localFilePreview: { target, reportsError in
                await model.localFilePreview(target, reportsError: reportsError)
            },
            retryFailedUserMessage: { messageId in
                await model.retryFailedUserMessage(
                    messageId,
                    presentationScopeIdentity: conversationScrollIdentity
                )
            },
            selectTaskNotification: { selection in
                taskNotificationSelectionState.present(
                    selection,
                    scope: taskNotificationPresentationScope
                )
            }
        )
    }

    private var taskNotificationPresentationScope: GaryxTaskNotificationPresentationScope {
        GaryxTaskNotificationPresentationScope(
            threadIdentity: liveStore.threadID ?? liveStore.routeIdentity,
            gatewayIdentity: model.currentGatewayScopeId,
            occurrenceIdentity: conversationScrollIdentity
        )
    }

    private var taskNotificationSelectionBinding: Binding<GaryxTaskNotificationSelection?> {
        Binding(
            get: { taskNotificationSelectionState.selection },
            set: { selection in
                if selection == nil {
                    taskNotificationSelectionState.dismiss()
                }
            }
        )
    }

    private func messageScroll(
        proxy: ScrollViewProxy,
        treatment: GaryxConversationTranscriptTreatment,
        turnRows: [GaryxMobileTurnRow]
    ) -> some View {
        ScrollView {
            ZStack(alignment: .topLeading) {
                // Give short transcripts a viewport-height content plane. The
                // gesture owner is attached after this ZStack resolves, so for
                // long transcripts it expands to the full scroll content height
                // instead of remaining behind only the first visible page.
                Color.clear
                    .containerRelativeFrame(.vertical) { length, _ in length }
                    .allowsHitTesting(false)

                VStack(alignment: .leading) {
                    // Deliberately an eager VStack: LazyVStack's estimated row
                    // heights put the synthetic bottom anchor below the real
                    // content end, so scroll-to-tail landed in blank phantom space
                    // that the anchor-based metrics could not detect. Long-thread
                    // scroll cost is controlled by keeping per-frame measurements
                    // out of SwiftUI state (`scrollStateBox`) instead.
                    VStack(alignment: .leading, spacing: 14) {
                        Color.clear
                            .frame(height: 1)
                            .background {
                                GeometryReader { geometry in
                                    Color.clear.preference(
                                        key: GaryxConversationContentEdgesKey.self,
                                        value: GaryxConversationContentEdges(
                                            top: geometry.frame(in: .named("garyx-conversation-scroll")).minY
                                        )
                                    )
                                }
                            }

                        switch treatment {
                        case .skeleton:
                            GaryxThreadHistoryLoadingView()
                                .padding(.top, 12)
                                .onAppear {
                                    GaryxRoutePushPerformanceProbe.shared?.markConversationMessageLoading()
                                }
                        case .content:
                            if turnRows.isEmpty {
                                if liveStore.isThinking(in: model) {
                                    if showsPresentedTailThinking {
                                        GaryxThinkingLabel()
                                            .padding(.top, 96)
                                            .transition(.opacity)
                                    }
                                } else if liveStore.threadID != nil {
                                    GaryxSelectedThreadEmptyConversationView()
                                        .padding(.top, 96)
                                }
                            } else {
                                if liveStore.hasMoreRenderableHistory(
                                    in: model,
                                    isCanonicalTop: routeContext.isCanonicalTop
                                ) {
                                    // Older history loads automatically as the reader nears
                                    // the top (two-stage: reveal window-hidden in-memory rows
                                    // first, then page the network — TASK-1751 P3). This row
                                    // is the top boundary sentinel plus the only loading
                                    // affordance; there is no manual load button.
                                    GaryxEarlierHistoryLoadingIndicator(
                                        isLoading: model.isLoadingOlderThreadHistory
                                    )
                                    .onAppear {
                                        prefetchOlderHistoryIfNeeded()
                                    }
                                }
                                GaryxMobileTurnRowsView(
                                    rows: turnRows,
                                    prefetchBoundaryRowCount: garyxHistoryPrefetchBoundaryRows,
                                    onNearHistoryBoundary: {
                                        prefetchOlderHistoryIfNeeded()
                                    },
                                    onRowContentMinYChange: { rowId, minY in
                                        // Plain box write: content-space geometry never
                                        // changes from scrolling, so this only fires on
                                        // layout changes and never invalidates the body.
                                        rowGeometryBox.record(rowId, minY: minY)
                                        reconcileSendAnchorFiller(proxy: proxy)
                                    }
                                )
                                .onAppear {
                                    GaryxRoutePushPerformanceProbe.shared?
                                        .markConversationLocalMessages()
                                }
                                if showsPresentedTailThinking {
                                    GaryxThinkingLabel()
                                        .id(tailThinkingAnchorId)
                                        .transition(.opacity)
                                }
                                if let rateLimit = liveStore.rateLimit(in: model),
                                   let threadId = liveStore.threadID
                                {
                                    GaryxRateLimitBanner(rateLimit: rateLimit) {
                                        try await model.retryThreadQuotaRecovery(threadId: threadId)
                                    }
                                    .transition(motion.transition(.transcriptAppear))
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 18)
                    .padding(.bottom, 24)
                    .garyxVerticalScrollContentWidth(alignment: .topLeading)
                    // Resolve the hosting UIScrollView for exact older-history
                    // prepend offset compensation. Must sit on content INSIDE the
                    // scroll view so the superview walk reaches it.
                    .background(GaryxEnclosingScrollViewReader(box: hostScrollViewBox))
                    // Do not attach a count-driven animation to the transcript
                    // container. A send changes the message count, composer height,
                    // spacer, and bottom anchor in the same layout pass; animating the
                    // whole stack makes the scroll view visibly wobble.

                    // Intrinsic transcript tail BEFORE send-anchor run space.
                    // It serves two independent pure-state inputs: content
                    // below the anchored row (filler reconciliation), and
                    // actual reply overflow (scroll-to-bottom visibility).
                    Color.clear
                        .frame(height: 0)
                        .accessibilityHidden(true)
                        .onGeometryChange(for: CGFloat.self) { geometry in
                            geometry.frame(
                                in: .named(garyxConversationContentSpaceName)
                            ).minY
                        } action: { minY in
                            rowGeometryBox.recordIntrinsicTail(minY: minY)
                            reconcileSendAnchorFiller(proxy: proxy)
                        }
                        .background {
                            GeometryReader { geometry in
                                Color.clear.preference(
                                    key: GaryxConversationContentEdgesKey.self,
                                    value: GaryxConversationContentEdges(
                                        tail: geometry.frame(
                                            in: .named("garyx-conversation-scroll")
                                        ).minY
                                    )
                                )
                            }
                        }

                    Color.clear
                        .frame(height: sendAnchorFillerHeight)
                        .accessibilityHidden(true)

                    Color.clear
                        .frame(height: conversationBottomChromeClearance)
                        .accessibilityHidden(true)

                    Color.clear
                        .frame(height: 1)
                        .id(conversationBottomAnchorId)
                        .accessibilityHidden(true)
                        .background {
                            GeometryReader { geometry in
                                Color.clear.preference(
                                    key: GaryxConversationContentEdgesKey.self,
                                    value: GaryxConversationContentEdges(
                                        bottom: geometry.frame(in: .named("garyx-conversation-scroll")).maxY
                                    )
                                )
                            }
                        }
                }
                // Scroll-invariant ruler shared by row geometry and the
                // intrinsic-tail sentinel. Moving it from the row stack to
                // this parent keeps all existing prepend differences exact
                // while allowing the filler sibling to stay outside the
                // measured intrinsic content.
                .coordinateSpace(name: garyxConversationContentSpaceName)
            }
            // Resolve the blank-space owner from the complete content plane:
            // max(viewport height, intrinsic transcript height). Row links,
            // long presses, and disclosures remain above this background.
            .background {
                GaryxTranscriptBlankSpaceTapLayer(action: dismissComposerKeyboard)
            }
        }
        .id(conversationScrollIdentity)
        .accessibilityIdentifier("garyx-conversation-transcript")
        // System-level anchoring stays on (v2): `.initialOffset` opens the
        // transcript already at the bottom with no programmatic jump, and
        // `.sizeChanges` keeps a reader positioned at the bottom pinned
        // through streaming growth. Only a send-anchor session suspends the
        // size-change role — its zero-auto-scroll contract is then the plain
        // UIScrollView default (below-viewport growth never moves the
        // offset). v1 removed both roles entirely, which regressed thread
        // opening and tail following everywhere.
        .garyxBottomAnchoredTranscript(sizeChangeAnchorSuspended: sendAnchorSessionActive)
        .coordinateSpace(name: "garyx-conversation-scroll")
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onGeometryChange(for: CGFloat.self) { geometry in
            geometry.size.height
        } action: { height in
            var metrics = scrollStateBox.state.metrics
            metrics.viewportHeight = height
            applyMetrics(metrics, proxy: proxy)
        }
        .onPreferenceChange(GaryxConversationContentEdgesKey.self) { edges in
            // One atomic frame per layout pass: both edges arrive together,
            // so the state machine never sees a phantom content-height change
            // mid-scroll (see GaryxConversationContentEdgesKey).
            var metrics = scrollStateBox.state.metrics
            if let top = edges.top {
                metrics.contentTopOffset = top
            }
            if let bottom = edges.bottom {
                metrics.contentBottomOffset = bottom
            }
            if let tail = edges.tail {
                metrics.contentTailOffset = tail
            }
            applyMetrics(metrics, proxy: proxy)
        }
        .scrollDisabled(sidebarDragActive)
        .scrollDismissesKeyboard(.interactively)
        .garyxUserScrollInteraction { isInteracting in
            updateScrollState(proxy: proxy) {
                $0.userScrollInteractionChanged(isInteracting: isInteracting)
            }
        }
        // Deliberately no `.refreshable`: the transcript is live (SSE +
        // automatic open/cold-start loading), so a top rubber-band pull has
        // exactly one meaning here — reach for older history (isPulledPastTop).
        // Keeping pull-to-refresh would bind two conflicting intents to the
        // same gesture.
    }

    private var showsNewThreadEmptyState: Bool {
        liveStore.threadID == nil
            && liveStore.messages(in: model).isEmpty
            && !liveStore.isThinking(in: model)
            && !model.isLoadingSelectedThreadHistory
            && !model.isSelectedThreadAwaitingInitialHistory
    }

    private var routeTurnRows: [GaryxMobileTurnRow] {
        liveStore.turnRows(in: model, isCanonicalTop: routeContext.isCanonicalTop)
    }

    private var messageScrollObservation: GaryxConversationScrollObservation<[GaryxMobileMessageGeometry]> {
        GaryxConversationScrollObservation(
            scopeIdentity: conversationScrollIdentity,
            value: liveStore.messages(in: model).map(GaryxMobileMessageGeometry.init),
            localSendPresentation: model.conversationLocalSendPresentation
        )
    }

    private var renderRowScrollObservation: GaryxConversationScrollObservation<[String]> {
        GaryxConversationScrollObservation(
            scopeIdentity: conversationScrollIdentity,
            value: routeTurnRows.map(\.id)
        )
    }

    private var showsPresentedTailThinking: Bool {
        liveStore.tailThinkingPresentationMode(in: model) == .immediate
            || showsDebouncedTailThinking
    }

    private func scheduleTranscriptSnapshot(rowIDs: [String]) {
        guard let threadID = liveStore.threadID, !rowIDs.isEmpty else { return }
        let messages = liveStore.messages(in: model)
        let tail = messages.last
        let revision = [
            String(rowIDs.count),
            rowIDs.first ?? "",
            rowIDs.last ?? "",
            String(messages.count),
            tail?.id ?? "",
            String(tail?.text.utf8.count ?? 0),
            String(tail?.isStreaming == true),
        ].joined(separator: ":")
        GaryxConversationTranscriptSnapshotCache.shared.scheduleCapture(
            threadID: threadID,
            revision: revision,
            scrollView: { [hostScrollViewBox] in
                hostScrollViewBox.currentScrollView()
            }
        )
    }

    /// Breathing room between the viewport top and an anchored user row so
    /// the message clears the floating title capsule (v2.1, boss feedback:
    /// the anchored position sat too high).
    private var conversationSendAnchorTopInset: CGFloat {
        16
    }

    private var conversationBottomChromeClearance: CGFloat {
        // The floating composer is attached with `safeAreaInset(.bottom)`, which already
        // reserves its full height above the transcript. This spacer only needs to add a
        // small breathing margin above that — adding the chrome height again double-counted
        // it and pushed the latest message a whole composer-height away from the input.
        24
    }

    private func beginSendAnchorFiller(anchorRowId: String) {
        let contentBelowAnchorHeight =
            rowGeometryBox.contentBelowAnchorHeight(anchorRowId: anchorRowId)
            ?? 0
        let height = sendAnchorFillerState.begin(
            anchorRowId: anchorRowId,
            viewportHeight: scrollStateBox.state.metrics.viewportHeight,
            bottomChromeClearance: conversationBottomChromeClearance,
            anchorTopInset: conversationSendAnchorTopInset,
            contentBelowAnchorHeight: contentBelowAnchorHeight
        )
        if sendAnchorFillerHeight != height {
            sendAnchorFillerHeight = height
        }
    }

    private func reconcileSendAnchorFiller(proxy: ScrollViewProxy) {
        guard let anchorRowId = sendAnchorFillerState.anchorRowId,
              let contentBelowAnchorHeight =
                  rowGeometryBox.contentBelowAnchorHeight(anchorRowId: anchorRowId) else {
            return
        }
        let height = sendAnchorFillerState.reconcile(
            viewportHeight: scrollStateBox.state.metrics.viewportHeight,
            bottomChromeClearance: conversationBottomChromeClearance,
            anchorTopInset: conversationSendAnchorTopInset,
            contentBelowAnchorHeight: contentBelowAnchorHeight
        )
        if sendAnchorFillerHeight != height {
            sendAnchorFillerHeight = height
        }
        if sendAnchorFillerState.isExhausted {
            // The reply grew below the screen: the run space is used up
            // (filler already zero), so end the session and hand off to
            // tail following (product decision 2026-07-24 — a reply longer
            // than one screen is followed, not parked). Filler collapse
            // happens in the session-exit mirror inside updateScrollState.
            updateScrollState(proxy: proxy) { $0.sendRunSpaceExhausted() }
        }
    }

    private func resetSendAnchorFiller() {
        sendAnchorFillerState.reset()
        if sendAnchorFillerHeight != 0 {
            sendAnchorFillerHeight = 0
        }
    }

    /// Feed a measurement update into the scroll state machine and run the
    /// follow-up work every metrics change shares.
    private func applyMetrics(_ metrics: GaryxConversationLayoutMetrics, proxy: ScrollViewProxy) {
        updateScrollState(proxy: proxy) {
            $0.metricsChanged(
                metrics,
                hasTailContent: !liveStore.messages(in: model).isEmpty
                    || showsPresentedTailThinking
            )
        }
        reconcileSendAnchorFiller(proxy: proxy)
        prefetchOlderHistoryIfNeeded()
    }

    /// Run a scroll state-machine event, mirror the UI projection into
    /// SwiftUI state only when it flipped, and execute the returned scroll
    /// request. Routing every event through here keeps the per-frame
    /// measurement churn from re-evaluating the conversation body.
    private func updateScrollState(
        proxy: ScrollViewProxy,
        _ event: (inout GaryxConversationScrollState) -> GaryxConversationScrollState.ScrollRequest?
    ) {
        let request = event(&scrollStateBox.state)
        let showsButton = scrollStateBox.state.showsScrollToBottomButton
        if showsScrollToBottomButton != showsButton {
            showsScrollToBottomButton = showsButton
        }
        let sessionActive = scrollStateBox.state.isSendAnchored
        if sendAnchorSessionActive != sessionActive {
            sendAnchorSessionActive = sessionActive
            if !sessionActive {
                // Single owner of run-space collapse: leaving the anchored
                // session (any exit path) removes the blank filler in the
                // same update. The filler sits below the viewport, so the
                // collapse is invisible; if the reader was inside the blank,
                // the clamp lands them on the real content bottom.
                resetSendAnchorFiller()
            }
        }
        apply(request, proxy: proxy)
    }

    /// Execute a target-bearing request produced by the scroll state machine.
    private func apply(
        _ request: GaryxConversationScrollState.ScrollRequest?,
        proxy: ScrollViewProxy
    ) {
        guard let request else { return }
        scheduleConversationScroll(proxy, request: request)
    }

    /// Pin the reading position through an older-history prepend
    /// (#TASK-2088): once the new rows have laid out, the anchor row's
    /// displacement in the transcript CONTENT coordinate space is exactly
    /// the height inserted above it (`historyPrependTopGrowth`). Shifting the
    /// CURRENT scroll offset by that displacement keeps the content under
    /// the reader perfectly still — concurrent tail streaming and concurrent
    /// reader scrolling both cancel out structurally, and no `scrollTo` is
    /// involved (its application timing is asynchronous and fights SwiftUI's
    /// own size-change repositioning; measured on-device).
    ///
    /// The attempts retry until the anchor row's post-prepend geometry is
    /// observable, apply exactly once (the generation bump kills the rest of
    /// the chain), and degrade to a coarse anchor-top `scrollTo` on the last
    /// attempt when geometry or the hosting scroll view never materialized.
    private func scheduleReadingAnchorRestore(
        _ restore: GaryxConversationScrollState.ReadingAnchorRestore,
        capturedAnchorMinY: CGFloat?,
        proxy: ScrollViewProxy
    ) {
        readingAnchorRestoreGeneration += 1
        let generation = readingAnchorRestoreGeneration
        let identity = conversationScrollIdentity
        let delays: [DispatchTimeInterval] = [
            .milliseconds(0), .milliseconds(16), .milliseconds(60), .milliseconds(140),
        ]

        for (index, delay) in delays.enumerated() {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                DispatchQueue.main.async {
                    guard generation == readingAnchorRestoreGeneration,
                          identity == conversationScrollIdentity else {
                        return
                    }
                    if let scrollView = hostScrollViewBox.currentScrollView(),
                       let growth = GaryxConversationScrollState.historyPrependTopGrowth(
                           capturedAnchorMinY: capturedAnchorMinY,
                           currentAnchorMinY: rowGeometryBox.minY(of: restore.anchorRowId)
                       ) {
                        let minOffset = -scrollView.adjustedContentInset.top
                        let maxOffset = max(
                            minOffset,
                            scrollView.contentSize.height - scrollView.bounds.height
                                + scrollView.adjustedContentInset.bottom
                        )
                        scrollView.setContentOffset(
                            CGPoint(
                                x: scrollView.contentOffset.x,
                                y: min(max(scrollView.contentOffset.y + growth, minOffset), maxOffset)
                            ),
                            animated: false
                        )
                        // Applied exactly once: kill the remaining attempts
                        // of this chain (a newer prepend has already re-armed
                        // with its own generation).
                        readingAnchorRestoreGeneration &+= 1
                        return
                    }
                    // Geometry (or the platform scroll view) never became
                    // observable: last attempt falls back to the coarse
                    // anchor-top scroll — still strictly better than parking
                    // the viewport over the just-loaded oldest rows. Never
                    // while a reader gesture drives the scroll view.
                    if index == delays.count - 1,
                       !scrollStateBox.state.isUserScrollInteracting {
                        var transaction = Transaction()
                        transaction.disablesAnimations = true
                        withTransaction(transaction) {
                            proxy.scrollTo(restore.anchorRowId, anchor: .top)
                        }
                    }
                }
            }
        }
    }

    private func resetTailThinkingPresentation(proxy: ScrollViewProxy) {
        tailThinkingPresentationState = GaryxTailThinkingPresentationState()
        setDebouncedTailThinking(false, proxy: proxy)
        syncTailThinkingPresentation(proxy: proxy)
    }

    private func syncTailThinkingPresentation(proxy: ScrollViewProxy) {
        tailThinkingDebounceGeneration += 1
        let generation = tailThinkingDebounceGeneration
        refreshTailThinkingPresentation(proxy: proxy, generation: generation)
    }

    private func refreshTailThinkingPresentation(proxy: ScrollViewProxy, generation: Int) {
        let now = Date().timeIntervalSinceReferenceDate
        let mode = liveStore.tailThinkingPresentationMode(in: model)
        let visible = tailThinkingPresentationState.update(
            mode: mode,
            now: now
        )
        setDebouncedTailThinking(
            visible,
            notifiesScrollState: mode == .debounced,
            proxy: proxy
        )
        if let delay = tailThinkingPresentationState.nextVisibilityCheck(now: now) {
            scheduleTailThinkingVisibilityCheck(delay: delay, proxy: proxy, generation: generation)
        }
    }

    private func scheduleTailThinkingVisibilityCheck(
        delay: TimeInterval,
        proxy: ScrollViewProxy,
        generation: Int
    ) {
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            DispatchQueue.main.async {
                guard generation == tailThinkingDebounceGeneration else { return }
                refreshTailThinkingPresentation(proxy: proxy, generation: generation)
            }
        }
    }

    private func setDebouncedTailThinking(
        _ visible: Bool,
        notifiesScrollState: Bool = false,
        proxy: ScrollViewProxy
    ) {
        guard showsDebouncedTailThinking != visible else { return }
        let update = {
            showsDebouncedTailThinking = visible
        }
        withAnimation(motion.animation(.tailThinking), update)
        if visible, notifiesScrollState {
            updateScrollState(proxy: proxy) { $0.thinkingIndicatorShown() }
        }
    }

    private func executeConversationScroll(
        _ proxy: ScrollViewProxy,
        request: GaryxConversationScrollState.ScrollRequest,
        animated: Bool = false
    ) {
        // A `.scrollPosition` binding is deliberately avoided here: binding a
        // ScrollPosition disables ScrollViewReader.scrollTo, and positioning
        // by `edge: .bottom` makes the scroll view stick to the bottom on
        // every content change, which fights the reader while a run streams.
        // The explicit target plus the scheduled retry chain is reliable.
        let targetId: String
        switch request.target {
        case .transcriptTail:
            targetId = conversationBottomAnchorId
        case .row(let id):
            // Row targets position exactly: row top at viewport top plus the
            // anchor inset (breathing room under the floating title capsule,
            // v2.1). `proxy.scrollTo(anchor: .top)` cannot express the inset,
            // so the primary path writes the host scroll view's offset
            // directly, mirroring the prepend-restore pattern.
            if let scrollView = hostScrollViewBox.currentScrollView(),
               let rowMinY = rowGeometryBox.minY(of: id) {
                let topInset = scrollView.adjustedContentInset.top
                let proposed = rowMinY - conversationSendAnchorTopInset - topInset
                let maxOffset = max(
                    -topInset,
                    scrollView.contentSize.height
                        + scrollView.adjustedContentInset.bottom
                        - scrollView.bounds.height
                )
                let target = CGPoint(
                    x: scrollView.contentOffset.x,
                    y: min(max(proposed, -topInset), maxOffset)
                )
                scrollView.setContentOffset(target, animated: animated)
                return
            }
            targetId = id
        }
        let anchor: UnitPoint = request.alignment == .top ? .top : .bottom
        if animated {
            withAnimation(motion.spatialAnimation(.scrollToTail)) {
                proxy.scrollTo(targetId, anchor: anchor)
            }
        } else {
            proxy.scrollTo(targetId, anchor: anchor)
        }
    }

    /// Run one target-bearing request across its Core-owned settlement clock.
    /// A local send uses the same long geometry horizon as opening, but its
    /// stable row target settles after the first observed top placement.
    private func scheduleConversationScroll(
        _ proxy: ScrollViewProxy,
        request: GaryxConversationScrollState.ScrollRequest
    ) {
        let token = scrollSchedulerBox.state.schedule(request: request)
        let identity = conversationScrollIdentity
        // Long transcripts re-layout while scrolling, so a single scrollTo
        // can land short; the later attempts converge on the true bottom.
        let delays = request.reason.retryDelayMilliseconds.map(
            DispatchTimeInterval.milliseconds
        )

        for (index, delay) in delays.enumerated() {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                DispatchQueue.main.async {
                    guard identity == conversationScrollIdentity else {
                        return
                    }
                    let input = scrollStateBox.state.scrollAttemptInput(
                        index: index,
                        request: request,
                        rowTargetViewportOffset: rowTargetViewportOffset(
                            for: request
                        )
                    )
                    // First authorized write of the chain: this is the moment
                    // the transcript actually moves. The send animation and
                    // its haptic key off it, so a missed zero-delay attempt
                    // (row not laid out yet) still animates on the retry that
                    // really lands instead of snapping (v2.1 alignment fix).
                    let isFirstWrite =
                        scrollSchedulerBox.state.lifecycle(of: token) == .requested
                    guard scrollSchedulerBox.state.authorizeAttempt(
                        token,
                        input: input
                    ) else {
                        return
                    }
                    if isFirstWrite, request.reason == .localSend {
                        GaryxMobileHaptics.shared.play(.messageSendCommitted)
                    }
                    executeConversationScroll(
                        proxy,
                        request: request,
                        animated: request.animated && isFirstWrite
                    )
                }
            }
        }
    }

    private func rowTargetViewportOffset(
        for request: GaryxConversationScrollState.ScrollRequest
    ) -> CGFloat? {
        guard case .row(let rowId) = request.target,
              let rowMinY = rowGeometryBox.minY(of: rowId),
              let scrollView = hostScrollViewBox.currentScrollView() else {
            return nil
        }
        let visibleContentTop =
            scrollView.contentOffset.y + scrollView.adjustedContentInset.top
        // Distance from the DESIRED placement (row top sitting exactly
        // `conversationSendAnchorTopInset` below the viewport top); zero
        // means satisfied.
        return rowMinY - visibleContentTop - conversationSendAnchorTopInset
    }

    private var conversationScrollIdentity: String {
        // Occurrence identity survives draft promotion because promotion only
        // changes the route payload revision, never the host identity.
        routeContext.occurrenceID?.rawValue ?? liveStore.routeIdentity
    }

    private var conversationBottomAnchorId: String {
        "conversation-bottom-anchor-\(conversationScrollIdentity)"
    }

    private var tailThinkingAnchorId: String {
        "tail-thinking-\(conversationScrollIdentity)"
    }

    private var messageMenuDismissToken: String {
        [
            conversationScrollIdentity,
            model.activePanel.rawValue,
            model.sidebarVisible ? "sidebar" : "content",
            model.showsSettings ? "settings" : "main",
            runtimePanelPresented ? "runtime-panel" : "runtime-closed",
        ].joined(separator: "|")
    }

    private func prefetchOlderHistoryIfNeeded() {
        guard routeContext.isCanonicalTop,
              let threadId = liveStore.threadID,
              model.selectedThread?.id == threadId,
              scrollStateBox.state.shouldPrefetchOlderHistory(
                // Reaching the top of the *rendered* content (the window floor)
                // reveals window-hidden rows first, then pages the network — so
                // the gate fires whenever more renderable history remains
                // (TASK-1751 P3).
                hasMoreHistoryBefore: liveStore.hasMoreRenderableHistory(
                    in: model,
                    isCanonicalTop: true
                ),
                isLoadingOlderHistory: model.isLoadingOlderThreadHistory,
                hasPendingPrefetch: pendingHistoryPrefetchThreadId == threadId
              ) else {
            return
        }
        pendingHistoryPrefetchThreadId = threadId
        Task {
            await model.advanceSelectedThreadHistoryBoundary()
            await MainActor.run {
                if pendingHistoryPrefetchThreadId == threadId {
                    pendingHistoryPrefetchThreadId = nil
                }
            }
        }
    }

    private func dismissComposerKeyboard() {
        isComposerFocused = false
        garyxDismissKeyboard()
    }

}

struct GaryxConversationHeader: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let liveStore: GaryxConversationLiveStore
    let stagedMetadata: GaryxConversationOpeningMetadata?
    let showsStagedLoading: Bool
    let preparesRuntimeModels: Bool
    let isRuntimePanelPresented: Bool
    let onToggleRuntimePanel: () -> Void
    let onDismissRuntimePanel: () -> Void

    @State private var showsRenamePrompt = false
    @State private var renameDraftTitle = ""
    @State private var showsBotBindingSheet = false
    @State private var botBindingThreadId: String?

    var body: some View {
        GaryxAdaptiveGlassContainer(spacing: 10) {
            HStack(spacing: 12) {
                Button(action: goHome) {
                    GaryxToolbarIcon(systemName: "chevron.left")
                }
                .buttonStyle(GaryxPressableRowStyle())
                .accessibilityLabel("Back")

                if liveStore.threadID == nil {
                    GaryxHeaderAgentControl()
                        .layoutPriority(1)
                } else {
                    GaryxThreadRuntimeHeaderControl(
                        routeSummary: liveStore.summary(in: model),
                        presentationTitle: stagedMetadata?.title,
                        presentationTarget: stagedMetadata?.agentTarget,
                        preparesRuntimeModels: preparesRuntimeModels,
                        isHidden: isRuntimePanelPresented,
                        onToggle: onToggleRuntimePanel
                    )
                    .layoutPriority(1)
                }

                Spacer(minLength: 0)

                if let selectedThread = liveStore.summary(in: model) {
                    Menu {
                        Section("Bot") {
                            Button {
                                botBindingThreadId = selectedThread.id
                                showsBotBindingSheet = true
                            } label: {
                                threadBotMenuLabel
                            }
                            .disabled(model.mobileBotGroups.isEmpty)
                        }

                        Button(
                            model.isThreadPinned(selectedThread.id) ? "Unpin thread" : "Pin thread",
                            systemImage: model.isThreadPinned(selectedThread.id) ? "pin.slash" : "pin"
                        ) {
                            model.togglePinnedThread(selectedThread.id)
                        }
                        Button(
                            model.threadIsFavorite(selectedThread.id)
                                ? "Unfavorite thread"
                                : "Favorite thread",
                            systemImage: model.threadIsFavorite(selectedThread.id)
                                ? "star.slash"
                                : "star"
                        ) {
                            model.toggleThreadFavorite(selectedThread.id)
                        }
                        Button("Rename", systemImage: "pencil") {
                            openRenamePrompt()
                        }
                        Button("Archive", systemImage: "archivebox", role: .destructive) {
                            Task { await model.deleteSelectedThread() }
                        }
                    } label: {
                        if showsHeaderLoading {
                            GaryxToolbarIcon {
                                GaryxInkSpinner()
                            }
                        } else {
                            GaryxToolbarIcon(systemName: "ellipsis")
                        }
                    }
                    .buttonStyle(GaryxPressableRowStyle(prepares: .threadPinChanged))
                    .accessibilityLabel(
                        showsHeaderLoading ? "Loading thread" : "Thread actions"
                    )
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
        .garyxAlert("Rename Thread", isPresented: $showsRenamePrompt) {
            TextField("Thread title", text: $renameDraftTitle)
            Button("Cancel", role: .cancel) {}
            Button("Save") {
                Task {
                    await model.renameSelectedThread(to: renameDraftTitle)
                }
            }
        }
        .garyxSheet(isPresented: $showsBotBindingSheet, onDismiss: {
            botBindingThreadId = nil
        }) {
            if let botBindingThreadId {
                GaryxThreadBotBindingSheet(threadId: botBindingThreadId)
            }
        }
        .onChange(of: liveStore.routeIdentity) { _, _ in
            dismissThreadPresentations()
        }
        .onChange(of: model.sidebarVisible) { _, visible in
            if visible {
                dismissThreadPresentations()
            }
        }
        .onChange(of: model.activePanel) { _, panel in
            if panel != .chat {
                dismissThreadPresentations()
            }
        }
        .onChange(of: model.showsSettings) { _, visible in
            if visible {
                dismissThreadPresentations()
            }
        }
    }

    private var showsHeaderLoading: Bool {
        showsStagedLoading
            || liveStore.isLoadingInitialHistory(in: model, isCanonicalTop: true)
    }

    @ViewBuilder
    private var threadBotMenuLabel: some View {
        if let group = model.selectedThreadBotGroup {
            GaryxBotGroupMenuSelectionLabel(group: group, selected: false)
        } else {
            Label("Bind Bot", systemImage: "link.badge.plus")
        }
    }

    private func openRenamePrompt() {
        renameDraftTitle = liveStore.summary(in: model)?.title ?? model.draftThreadTitle
        showsRenamePrompt = true
    }

    private func goHome() {
        garyxDismissKeyboard()
        dismissThreadPresentations()
        model.returnHome()
    }

    private func dismissThreadPresentations() {
        onDismissRuntimePanel()
        showsRenamePrompt = false
        showsBotBindingSheet = false
        botBindingThreadId = nil
    }
}

private struct GaryxThreadRuntimeHeaderControl: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let routeSummary: GaryxThreadSummary?
    let presentationTitle: String?
    let presentationTarget: GaryxMobileAgentTarget?
    let preparesRuntimeModels: Bool
    /// While the morph surface is presented it renders this control's twin
    /// at the same anchor rect, so the in-bar original hides without
    /// leaving layout (keeping the anchor alive for the collapse morph).
    let isHidden: Bool
    let onToggle: () -> Void

    private var selectedThread: GaryxThreadSummary? { routeSummary }
    private var runtime: GaryxThreadRuntimeSummary? { selectedThread?.threadRuntime }
    private var title: String {
        normalized(presentationTitle) ?? selectedThread?.title ?? model.draftThreadTitle
    }

    private var target: GaryxMobileAgentTarget? {
        presentationTarget ?? model.selectedThreadAgentTarget
    }

    private var providerType: String {
        normalized(runtime?.providerType)
            ?? normalized(selectedThread?.providerType)
            ?? normalized(target?.providerType)
            ?? ""
    }

    var body: some View {
        Button(action: onToggle) {
            // Glass is applied directly to the row content. Inside the top
            // bar's GlassEffectContainer a glass background shape gets
            // hoisted into the container's shared pass and draws over the
            // title/avatar (iOS 26), so the surface must never live in a
            // `.background` here.
            GaryxThreadRuntimeCompactContentRow(
                title: title,
                target: target
            )
                .garyxAdaptiveGlass(
                    .regular,
                    isInteractive: false,
                    in: Capsule(),
                    isEnabled: !isHidden
                )
                // The glass surface itself has no hit-test region on iOS 26
                // (taps between the glyphs fall through to the transcript),
                // so the label declares the full capsule as its tap target —
                // same pattern as GaryxToolbarIcon.
                .contentShape(Capsule())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .opacity(isHidden ? 0 : 1)
        .allowsHitTesting(!isHidden)
        .accessibilityLabel("\(title), thread settings")
        .accessibilityHidden(isHidden)
        .anchorPreference(key: GaryxThreadRuntimeChromeAnchorKey.self, value: .bounds) { $0 }
        .layoutPriority(1)
        .task(id: providerType) {
            guard preparesRuntimeModels,
                  !providerType.isEmpty,
                  model.providerModelsByType[providerType] == nil else {
                return
            }
            await model.loadProviderModels(providerType: providerType)
        }
    }

    private func normalized(_ value: String?) -> String? {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty else {
            return nil
        }
        return value
    }
}

private extension View {
    /// System-level transcript anchoring (v2 send-anchor design).
    ///
    /// `.initialOffset` opens the transcript anchored to its bottom from the
    /// very first layout pass — no post-load programmatic scroll-down. The
    /// alignment role is deliberately not anchored so short conversations
    /// keep starting at the top.
    ///
    /// `.sizeChanges` keeps the tail pinned through content growth while the
    /// reader is positioned at the bottom. A send-anchor session suspends
    /// exactly this role (`nil` anchor): with it off, plain UIScrollView
    /// behavior — below-viewport growth never moves the offset — provides
    /// the session's zero-auto-scroll contract. The role resumes when the
    /// session's run space is retired (scroll-to-bottom or thread switch);
    /// changing the anchor value never rebuilds the scroll view or moves
    /// its current offset by itself.
    func garyxBottomAnchoredTranscript(sizeChangeAnchorSuspended: Bool) -> some View {
        self
            .defaultScrollAnchor(.bottom, for: .initialOffset)
            .defaultScrollAnchor(
                sizeChangeAnchorSuspended ? nil : .bottom,
                for: .sizeChanges
            )
    }

    /// Reports whether the reader's gesture currently drives the scroll
    /// view (finger down or fling decelerating). Programmatic phases do not
    /// count.
    func garyxUserScrollInteraction(_ onChange: @escaping (Bool) -> Void) -> some View {
        onScrollPhaseChange { _, newPhase in
            switch newPhase {
            case .tracking, .interacting, .decelerating:
                onChange(true)
            case .idle, .animating:
                onChange(false)
            @unknown default:
                onChange(false)
            }
        }
    }
}

private struct GaryxHeaderAgentControl: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        if model.selectedThread == nil {
            GaryxAgentTargetPickerControl(
                selectedAgentTargetId: selectedAgentTargetBinding,
                style: .prominent,
                showsConfigure: true,
                showsThreadModelOverride: true,
                onConfigure: { model.openPanel(.agents) }
            )
            .accessibilityLabel("Agent")
        } else {
            GaryxAgentPickerLabel(
                target: model.selectedThreadAgentTarget,
                title: model.selectedThreadAgentLabel,
                showsChevron: false,
                style: .compact
            )
            .accessibilityLabel("Agent")
        }
    }

    private var selectedAgentTargetBinding: Binding<String> {
        Binding {
            model.newThreadAgentTargetId()
        } set: { value in
            model.setNewThreadAgentTarget(value)
        }
    }
}
