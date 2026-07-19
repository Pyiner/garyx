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

/// Single preference key carrying BOTH transcript content edges. The top
/// sentinel and the bottom anchor each contribute their half and SwiftUI
/// reduces them within one layout pass, so `onPreferenceChange` delivers an
/// atomic frame. Do not split the edges back into separate keys: two
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

    func record(_ rowId: String, minY: CGFloat) {
        minYByRowId[rowId] = minY
    }

    func minY(of rowId: String) -> CGFloat? {
        minYByRowId[rowId]
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

    static let empty = GaryxMessageBubbleActions(
        model: nil,
        localFilePreview: { _, _ in nil },
        retryFailedUserMessage: { _ in false }
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.layoutDirection) private var layoutDirection
    @FocusState private var isComposerFocused: Bool
    private let liveStore: GaryxConversationLiveStore
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
    @State private var scrollPreservationThreadId: String?
    @State private var rowScrollPreservationThreadId: String?
    @State private var pendingHistoryPrefetchThreadId: String?
    @State private var bottomChromeHeight: CGFloat = 0
    @State private var tailScrollRequestGeneration = 0
    @State private var readingAnchorRestoreGeneration = 0
    @State private var tailThinkingPresentationState = GaryxTailThinkingPresentationState()
    @State private var showsDebouncedTailThinking = false
    @State private var tailThinkingDebounceGeneration = 0
    /// Runtime-panel morph state machine. `Presented` mounts the overlay
    /// surface (collapsed, exactly over the top-bar capsule) and hides the
    /// in-bar capsule; `Expanded` drives the spring morph. Close animates
    /// `Expanded` back first, then unmounts on completion.
    @State private var runtimePanelPresented = false
    @State private var runtimePanelExpanded = false

    init(destination: GaryxRouteDestination) {
        liveStore = GaryxConversationLiveStore(destination: destination)
    }

    var body: some View {
        ScrollViewReader { proxy in
            ZStack(alignment: .bottom) {
                messageScroll(proxy: proxy)

                // The new-thread empty state lives outside the transcript
                // scroll so it stays centered between the header and the
                // composer.
                if showsNewThreadEmptyState {
                    GaryxEmptyConversationView()
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
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
                                .font(GaryxFont.system(size: 15, weight: .semibold))
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
                                            fallbackMaterial: .ultraThinMaterial,
                                            in: Circle()
                                        )
                                        .allowsHitTesting(false)
                                }
                                .contentShape(Circle())
                                .shadow(color: Color.black.opacity(0.12), radius: 14, x: 0, y: 8)
                        }
                        .buttonStyle(.plain)
                        .transition(.scale(scale: 0.88).combined(with: .opacity))
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
                .animation(.easeOut(duration: 0.18), value: showsScrollToBottomButton)
            }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onAppear {
                    updateScrollState(proxy: proxy) { $0.threadOpened() }
                    resetTailThinkingPresentation(proxy: proxy)
                }
                .onChange(of: liveStore.routeIdentity) { _, _ in
                    setRuntimePanelVisible(false)
                    scrollPreservationThreadId = liveStore.threadID
                    rowScrollPreservationThreadId = liveStore.threadID
                    pendingHistoryPrefetchThreadId = nil
                    updateScrollState(proxy: proxy) { $0.threadOpened() }
                    resetTailThinkingPresentation(proxy: proxy)
                }
                .onChange(of: liveStore.messages(in: model)) { oldValue, newValue in
                    defer {
                        prefetchOlderHistoryIfNeeded()
                    }
                    let threadUnchanged = liveStore.threadID == scrollPreservationThreadId
                    scrollPreservationThreadId = liveStore.threadID
                    let isHistoryPrepend = GaryxConversationScrollState.preservesScrollForPrependedHistory(
                        previousIds: oldValue.map(\.id),
                        currentIds: newValue.map(\.id),
                        threadUnchanged: threadUnchanged
                    )
                    updateScrollState(proxy: proxy) {
                        $0.contentChanged(
                            isInitialLoad: oldValue.isEmpty,
                            isHistoryPrepend: isHistoryPrepend,
                            hasTailContent: !newValue.isEmpty || showsDebouncedTailThinking
                        )
                    }
                }
                .onChange(of: routeTurnRows.map(\.id)) { oldValue, newValue in
                    let threadUnchanged = liveStore.threadID == rowScrollPreservationThreadId
                    rowScrollPreservationThreadId = liveStore.threadID
                    let restore = scrollStateBox.state.renderRowsChanged(
                        previousIds: oldValue,
                        currentIds: newValue,
                        threadUnchanged: threadUnchanged,
                        hasTailContent: !newValue.isEmpty || showsDebouncedTailThinking
                    )
                    if let restore {
                        // Captured BEFORE the new rows lay out: the geometry
                        // box still holds the anchor row's pre-prepend
                        // content-space position.
                        scheduleReadingAnchorRestore(
                            restore,
                            capturedAnchorMinY: rowGeometryBox.minY(of: restore.anchorRowId),
                            proxy: proxy
                        )
                    }
                    rowGeometryBox.retain(only: Set(newValue))
                }
                .onChange(of: liveStore.isThinking(in: model)) { _, _ in
                    syncTailThinkingPresentation(proxy: proxy)
                }
                .onChange(of: isComposerFocused) { _, isFocused in
                    guard isFocused else { return }
                    updateScrollState(proxy: proxy) { $0.composerFocused() }
                }
                .onChange(of: bottomChromeHeight) { _, _ in
                    updateScrollState(proxy: proxy) { $0.bottomChromeChanged() }
                }
        }
        .garyxPageBackground()
        .garyxAdaptiveTopBar {
            GaryxConversationHeader(
                liveStore: liveStore,
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
        .task(id: "\(liveStore.routeIdentity):\(liveStore.hasCapsuleCards(in: model))") {
            guard liveStore.hasCapsuleCards(in: model) else { return }
            await model.refreshCapsules()
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
            guard !reduceMotion else {
                runtimePanelExpanded = true
                return
            }
            // …then spring it open on the next main-actor turn so the morph
            // visibly starts from the capsule rect.
            Task { @MainActor in
                withAnimation(GaryxThreadRuntimeMorph.openAnimation) {
                    runtimePanelExpanded = true
                }
            }
        } else {
            guard runtimePanelPresented else { return }
            guard !reduceMotion, runtimePanelExpanded else {
                runtimePanelExpanded = false
                runtimePanelPresented = false
                return
            }
            withAnimation(
                GaryxThreadRuntimeMorph.closeAnimation,
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
                await model.retryFailedUserMessage(messageId)
            }
        )
    }

    private func messageScroll(proxy: ScrollViewProxy) -> some View {
        ScrollView {
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

                let turnRows = routeTurnRows
                if turnRows.isEmpty,
                   liveStore.isLoadingInitialHistory(
                       in: model,
                       isCanonicalTop: routeContext.isCanonicalTop
                   ) {
                    GaryxThreadHistoryLoadingView()
                        .padding(.top, 12)
                } else if turnRows.isEmpty {
                    if liveStore.isThinking(in: model) {
                        if showsDebouncedTailThinking {
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
                        }
                    )
                    if showsDebouncedTailThinking {
                        GaryxThinkingLabel()
                            .id(tailThinkingAnchorId)
                            .transition(.opacity)
                    }
                    if let rateLimit = liveStore.rateLimit(in: model) {
                        GaryxRateLimitBanner(rateLimit: rateLimit) {
                            await model.send("continue")
                        }
                        .transition(.garyxTranscriptAppear)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 18)
            .padding(.bottom, 24)
            // Content coordinate space for row geometry: scroll-invariant, so
            // a row's minY here only moves when the layout itself changes —
            // the ruler for exact prepend compensation.
            .coordinateSpace(name: garyxConversationContentSpaceName)
            .garyxVerticalScrollContentWidth(alignment: .topLeading)
            // Resolve the hosting UIScrollView for exact older-history
            // prepend offset compensation. Must sit on content INSIDE the
            // scroll view so the superview walk reaches it.
            .background(GaryxEnclosingScrollViewReader(box: hostScrollViewBox))
            // Do not attach a count-driven animation to the transcript
            // container. A send changes the message count, composer height,
            // spacer, and bottom anchor in the same layout pass; animating the
            // whole stack makes the scroll view visibly wobble.

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
        .id(conversationScrollIdentity)
        .garyxBottomAnchoredTranscript()
        // The transcript is laid out top-down: short conversations start at
        // the top of the viewport. Tail anchoring is driven explicitly by the
        // scroll state machine instead of a bottom default anchor.
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
            applyMetrics(metrics, proxy: proxy)
        }
        .scrollDisabled(isComposerFocused || sidebarDragActive)
        .scrollDismissesKeyboard(.never)
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
        .overlay {
            if isComposerFocused {
                GeometryReader { geometry in
                    Color.clear
                        .contentShape(Rectangle())
                        .onTapGesture {
                            dismissComposerKeyboard()
                        }
                        .gesture(
                            DragGesture(minimumDistance: 6, coordinateSpace: .local)
                                .onChanged { value in
                                    guard !startsInLeadingNavigationEdge(
                                        x: value.startLocation.x,
                                        width: geometry.size.width
                                    ) else { return }
                                    dismissComposerKeyboard()
                                }
                        )
                }
            }
        }
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

    private var conversationBottomChromeClearance: CGFloat {
        // The floating composer is attached with `safeAreaInset(.bottom)`, which already
        // reserves its full height above the transcript. This spacer only needs to add a
        // small breathing margin above that — adding the chrome height again double-counted
        // it and pushed the latest message a whole composer-height away from the input.
        24
    }

    /// Feed a measurement update into the scroll state machine and run the
    /// follow-up work every metrics change shares.
    private func applyMetrics(_ metrics: GaryxConversationLayoutMetrics, proxy: ScrollViewProxy) {
        updateScrollState(proxy: proxy) {
            $0.metricsChanged(
                metrics,
                hasTailContent: !liveStore.messages(in: model).isEmpty || showsDebouncedTailThinking
            )
        }
        prefetchOlderHistoryIfNeeded()
    }

    /// Run a scroll state machine event, mirror the UI projection into
    /// SwiftUI state only when it flipped, and execute the returned scroll
    /// request. Routing every event through here keeps the per-frame
    /// measurement churn from re-evaluating the conversation body.
    private func updateScrollState(
        proxy: ScrollViewProxy,
        _ event: (inout GaryxConversationScrollState) -> GaryxConversationScrollState.TailScrollRequest?
    ) {
        let request = event(&scrollStateBox.state)
        let showsButton = scrollStateBox.state.showsScrollToBottomButton
        if showsScrollToBottomButton != showsButton {
            showsScrollToBottomButton = showsButton
        }
        apply(request, proxy: proxy)
    }

    /// Execute a tail-scroll request produced by the scroll state machine.
    private func apply(
        _ request: GaryxConversationScrollState.TailScrollRequest?,
        proxy: ScrollViewProxy
    ) {
        guard let request else { return }
        scheduleScrollToConversationTail(proxy, request: request)
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
        let visible = tailThinkingPresentationState.update(
            isThinking: liveStore.isThinking(in: model),
            now: now
        )
        setDebouncedTailThinking(visible, proxy: proxy)
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

    private func setDebouncedTailThinking(_ visible: Bool, proxy: ScrollViewProxy) {
        guard showsDebouncedTailThinking != visible else { return }
        let update = {
            showsDebouncedTailThinking = visible
        }
        if reduceMotion {
            update()
        } else {
            withAnimation(.easeOut(duration: 0.15), update)
        }
        if visible {
            updateScrollState(proxy: proxy) { $0.thinkingIndicatorShown() }
        }
    }

    private func scrollToConversationTail(_ proxy: ScrollViewProxy) {
        // A `.scrollPosition` binding is deliberately avoided here: binding a
        // ScrollPosition disables ScrollViewReader.scrollTo, and positioning
        // by `edge: .bottom` makes the scroll view stick to the bottom on
        // every content change, which fights the reader while a run streams.
        // The anchor jump plus the scheduled retry chain is reliable.
        proxy.scrollTo(conversationBottomAnchorId, anchor: .bottom)
    }

    /// Run a tail scroll now and retry across the next layout passes, so the
    /// scroll lands even when row content (markdown, images, tool traces) is
    /// still settling. The state machine decides whether late retries should
    /// still run.
    private func scheduleScrollToConversationTail(
        _ proxy: ScrollViewProxy,
        request: GaryxConversationScrollState.TailScrollRequest
    ) {
        tailScrollRequestGeneration += 1
        let generation = tailScrollRequestGeneration
        let identity = conversationScrollIdentity
        // Long transcripts re-layout while scrolling, so a single scrollTo
        // can land short; the later attempts converge on the true bottom.
        let delays = tailScrollRetryDelays(for: request.reason)

        for (index, delay) in delays.enumerated() {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                DispatchQueue.main.async {
                    guard generation == tailScrollRequestGeneration,
                          identity == conversationScrollIdentity,
                          scrollStateBox.state.shouldRunTailScrollAttempt(index: index, reason: request.reason) else {
                        return
                    }
                    if request.animated && index == 0 {
                        withAnimation(.easeOut(duration: 0.2)) {
                            scrollToConversationTail(proxy)
                        }
                    } else {
                        scrollToConversationTail(proxy)
                    }
                }
            }
        }
    }

    private func tailScrollRetryDelays(
        for reason: GaryxConversationScrollState.TailScrollReason
    ) -> [DispatchTimeInterval] {
        switch reason {
        case .tailUpdate:
            // Ordinary tail growth during send/streaming should stay pinned,
            // but long retry chains make the transcript visibly wobble while
            // the composer and bottom spacer are also settling.
            return [.milliseconds(0), .milliseconds(40), .milliseconds(140)]
        case .openingThread, .manual, .repair:
            return [
                .milliseconds(0), .milliseconds(16), .milliseconds(40), .milliseconds(140),
                .milliseconds(320), .milliseconds(650), .milliseconds(1_000),
            ]
        }
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
        guard isComposerFocused else { return }
        isComposerFocused = false
        garyxDismissKeyboard()
    }

    private func startsInLeadingNavigationEdge(x: CGFloat, width: CGFloat) -> Bool {
        let leadingInset = layoutDirection == .rightToLeft ? width - x : x
        return leadingInset <= GaryxRouteTransitionCalibration.edgeZoneWidth
    }
}

struct GaryxConversationHeader: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let liveStore: GaryxConversationLiveStore
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
                .buttonStyle(.plain)
                .accessibilityLabel("Back")

                if liveStore.threadID == nil {
                    GaryxHeaderAgentControl()
                        .layoutPriority(1)
                } else {
                    GaryxThreadRuntimeHeaderControl(
                        routeSummary: liveStore.summary(in: model),
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
                        if liveStore.isLoadingInitialHistory(in: model, isCanonicalTop: true) {
                            GaryxToolbarIcon {
                                GaryxInkSpinner()
                            }
                        } else {
                            GaryxToolbarIcon(systemName: "ellipsis")
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(
                        liveStore.isLoadingInitialHistory(in: model, isCanonicalTop: true)
                            ? "Loading thread"
                            : "Thread actions"
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
    /// While the morph surface is presented it renders this control's twin
    /// at the same anchor rect, so the in-bar original hides without
    /// leaving layout (keeping the anchor alive for the collapse morph).
    let isHidden: Bool
    let onToggle: () -> Void

    private var selectedThread: GaryxThreadSummary? { routeSummary }
    private var runtime: GaryxThreadRuntimeSummary? { selectedThread?.threadRuntime }
    private var title: String { selectedThread?.title ?? model.draftThreadTitle }

    private var providerType: String {
        normalized(runtime?.providerType)
            ?? normalized(selectedThread?.providerType)
            ?? normalized(model.selectedThreadAgentTarget?.providerType)
            ?? ""
    }

    var body: some View {
        Button(action: onToggle) {
            // Glass is applied directly to the row content. Inside the top
            // bar's GlassEffectContainer a glass background shape gets
            // hoisted into the container's shared pass and draws over the
            // title/avatar (iOS 26), so the surface must never live in a
            // `.background` here.
            GaryxThreadRuntimeCompactRow()
                .garyxAdaptiveGlass(
                    .regular,
                    isInteractive: false,
                    fallbackMaterial: .ultraThinMaterial,
                    in: Capsule(),
                    isEnabled: !isHidden
                )
                // The glass surface itself has no hit-test region on iOS 26
                // (taps between the glyphs fall through to the transcript),
                // so the label declares the full capsule as its tap target —
                // same pattern as GaryxToolbarIcon.
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .opacity(isHidden ? 0 : 1)
        .allowsHitTesting(!isHidden)
        .accessibilityLabel("\(title), thread settings")
        .accessibilityHidden(isHidden)
        .anchorPreference(key: GaryxThreadRuntimeChromeAnchorKey.self, value: .bounds) { $0 }
        .layoutPriority(1)
        .task(id: providerType) {
            guard !providerType.isEmpty,
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
    /// Opens the transcript anchored to its bottom from the very first
    /// layout pass and keeps the tail pinned through content growth while
    /// positioned there — no post-load programmatic scroll-down. The
    /// alignment role is deliberately not anchored so short conversations
    /// keep starting at the top. Before iOS 18 the scroll state machine's
    /// retry chain remains the only mechanism.
    @ViewBuilder
    func garyxBottomAnchoredTranscript() -> some View {
        if #available(iOS 18.0, *) {
            self
                .defaultScrollAnchor(.bottom, for: .initialOffset)
                .defaultScrollAnchor(.bottom, for: .sizeChanges)
        } else {
            self
        }
    }

    /// Reports whether the reader's gesture currently drives the scroll
    /// view (finger down or fling decelerating). Programmatic phases do not
    /// count. No-op before iOS 18, where the scroll phase API is missing.
    @ViewBuilder
    func garyxUserScrollInteraction(_ onChange: @escaping (Bool) -> Void) -> some View {
        if #available(iOS 18.0, *) {
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
        } else {
            self
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
