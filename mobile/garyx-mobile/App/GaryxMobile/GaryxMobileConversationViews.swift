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

private struct GaryxConversationBottomOffsetKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

private struct GaryxConversationTopOffsetKey: PreferenceKey {
    static var defaultValue: CGFloat?

    static func reduce(value: inout CGFloat?, nextValue: () -> CGFloat?) {
        value = nextValue() ?? value
    }
}

/// Plain (non-observable) holder for the conversation scroll state machine.
/// Scroll measurements mutate it on every frame; keeping it out of SwiftUI
/// state means that churn never re-evaluates the transcript body.
private final class GaryxConversationScrollStateBox {
    var state = GaryxConversationScrollState()
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
    @Environment(\.garyxSidebarDragActive) private var sidebarDragActive
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @FocusState private var isComposerFocused: Bool
    /// Unified scroll state machine (GaryxMobileCore). The view feeds it
    /// events and executes the tail-scroll requests it returns; UI such as
    /// the scroll-to-bottom control reads its projections.
    // The scroll state machine lives in a plain reference box so the
    // per-frame scroll measurements feeding it never invalidate the
    // conversation body; `showsScrollToBottomButton` is the only scroll
    // fact the body reads, mirrored into SwiftUI state when it flips.
    @State private var scrollStateBox = GaryxConversationScrollStateBox()
    @State private var showsScrollToBottomButton = false
    @State private var scrollPreservationThreadId: String?
    @State private var rowScrollPreservationThreadId: String?
    @State private var pendingHistoryPrefetchThreadId: String?
    @State private var bottomChromeHeight: CGFloat = 0
    @State private var tailScrollRequestGeneration = 0
    @State private var tailThinkingPresentationState = GaryxTailThinkingPresentationState()
    @State private var showsDebouncedTailThinking = false
    @State private var tailThinkingDebounceGeneration = 0

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

                    GaryxComposer(isFocused: $isComposerFocused)
                }
                .frame(maxWidth: .infinity)
                .animation(.easeOut(duration: 0.18), value: showsScrollToBottomButton)
            }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onAppear {
                    updateScrollState(proxy: proxy) { $0.threadOpened() }
                    resetTailThinkingPresentation(proxy: proxy)
                }
                .onChange(of: model.selectedThread?.id) { _, _ in
                    scrollPreservationThreadId = model.selectedThread?.id
                    rowScrollPreservationThreadId = model.selectedThread?.id
                    pendingHistoryPrefetchThreadId = nil
                    updateScrollState(proxy: proxy) { $0.threadOpened() }
                    resetTailThinkingPresentation(proxy: proxy)
                }
                .onChange(of: model.messages) { oldValue, newValue in
                    defer {
                        prefetchOlderHistoryIfNeeded()
                    }
                    let threadUnchanged = model.selectedThread?.id == scrollPreservationThreadId
                    scrollPreservationThreadId = model.selectedThread?.id
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
                .onChange(of: model.selectedThreadTurnRows().map(\.id)) { oldValue, newValue in
                    let threadUnchanged = model.selectedThread?.id == rowScrollPreservationThreadId
                    rowScrollPreservationThreadId = model.selectedThread?.id
                    updateScrollState(proxy: proxy) {
                        $0.renderRowsChanged(
                            previousIds: oldValue,
                            currentIds: newValue,
                            threadUnchanged: threadUnchanged,
                            hasTailContent: !newValue.isEmpty || showsDebouncedTailThinking
                        )
                    }
                }
                .onChange(of: model.showsTailThinkingIndicator) { _, _ in
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
            GaryxConversationHeader()
        }
        // Task-tree sidebar overlays the whole conversation surface, header
        // included, so the scrim blocks every control behind the open panel.
        .garyxTaskTreeSidebarSurface()
        .environment(\.garyxMessageBubbleActions, messageBubbleActions)
        // Capsule card tapped in the transcript: present the focused preview
        // above this conversation and dismiss back to it (never switch to the
        // Capsules overview).
        .fullScreenCover(item: $model.conversationCapsulePreview) { capsule in
            GaryxCapsuleFocusedPreviewView(capsule: capsule)
        }
        // Route-time deletion validation: re-fires when the thread changes and
        // when capsule cards first appear (history can arrive after the thread is
        // selected, so a one-shot check on thread id alone would miss them).
        // Refreshing the capsules list prunes a remotely-deleted capsule's cached
        // preview HTML and bumps the cache epoch, so mounted chat thumbnails
        // re-validate to "deleted".
        .task(id: "\(model.selectedThread?.id ?? ""):\(model.selectedThreadHasCapsuleCards)") {
            guard model.selectedThreadHasCapsuleCards else { return }
            await model.refreshCapsules()
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
                                key: GaryxConversationTopOffsetKey.self,
                                value: geometry.frame(in: .named("garyx-conversation-scroll")).minY
                            )
                        }
                    }

                let turnRows = model.selectedThreadTurnRows()
                if turnRows.isEmpty,
                   model.isSelectedThreadLoadingInitialHistory {
                    GaryxThreadHistoryLoadingView()
                        .padding(.top, 12)
                } else if turnRows.isEmpty {
                    if model.showsTailThinkingIndicator {
                        if showsDebouncedTailThinking {
                            GaryxThinkingLabel()
                                .padding(.top, 96)
                                .transition(.opacity)
                        }
                    } else if model.selectedThread != nil {
                        GaryxSelectedThreadEmptyConversationView()
                            .padding(.top, 96)
                    }
                } else {
                    if model.selectedThreadHasMoreHistoryBefore {
                        GaryxLoadEarlierHistoryButton(isLoading: model.isLoadingOlderThreadHistory) {
                            Task {
                                await model.loadOlderSelectedThreadHistory()
                            }
                        }
                        .onAppear {
                            prefetchOlderHistoryIfNeeded()
                        }
                    }
                    GaryxMobileTurnRowsView(
                        rows: turnRows,
                        prefetchBoundaryRowCount: garyxHistoryPrefetchBoundaryRows
                    ) {
                        prefetchOlderHistoryIfNeeded()
                    }
                    if showsDebouncedTailThinking {
                        GaryxThinkingLabel()
                            .id(tailThinkingAnchorId)
                            .transition(.opacity)
                    }
                    if let rateLimit = model.selectedThreadRateLimit {
                        GaryxRateLimitBanner(rateLimit: rateLimit)
                            .transition(.garyxTranscriptAppear)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 18)
            .padding(.bottom, 24)
            .garyxVerticalScrollContentWidth(alignment: .topLeading)
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
                            key: GaryxConversationBottomOffsetKey.self,
                            value: geometry.frame(in: .named("garyx-conversation-scroll")).maxY
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
        .onPreferenceChange(GaryxConversationBottomOffsetKey.self) { value in
            var metrics = scrollStateBox.state.metrics
            metrics.contentBottomOffset = value
            applyMetrics(metrics, proxy: proxy)
        }
        .onPreferenceChange(GaryxConversationTopOffsetKey.self) { value in
            var metrics = scrollStateBox.state.metrics
            metrics.contentTopOffset = value
            applyMetrics(metrics, proxy: proxy)
        }
        .scrollDisabled(isComposerFocused || sidebarDragActive)
        .scrollDismissesKeyboard(.never)
        .garyxUserScrollInteraction { isInteracting in
            updateScrollState(proxy: proxy) {
                $0.userScrollInteractionChanged(isInteracting: isInteracting)
            }
        }
        .refreshable {
            await model.loadSelectedThreadHistory()
        }
        .overlay {
            if isComposerFocused {
                Color.clear
                    .contentShape(Rectangle())
                    .onTapGesture {
                        dismissComposerKeyboard()
                    }
                    .gesture(
                        DragGesture(minimumDistance: 6, coordinateSpace: .local)
                            .onChanged { _ in
                                dismissComposerKeyboard()
                            }
                    )
            }
        }
    }

    private var showsNewThreadEmptyState: Bool {
        model.selectedThread == nil
            && model.messages.isEmpty
            && !model.showsTailThinkingIndicator
            && !model.isLoadingSelectedThreadHistory
            && !model.isSelectedThreadAwaitingInitialHistory
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
                hasTailContent: !model.messages.isEmpty || showsDebouncedTailThinking
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
            isThinking: model.showsTailThinkingIndicator,
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
        // Session token, not thread id: draft -> promoted-thread keeps one
        // token so the transcript view survives the first send instead of
        // being torn down and rebuilt (whole-list flash).
        model.conversationSessionToken
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
        ].joined(separator: "|")
    }

    private func prefetchOlderHistoryIfNeeded() {
        guard let threadId = model.selectedThread?.id,
              scrollStateBox.state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: model.selectedThreadHasMoreHistoryBefore,
                isLoadingOlderHistory: model.isLoadingOlderThreadHistory,
                hasPendingPrefetch: pendingHistoryPrefetchThreadId == threadId
              ) else {
            return
        }
        pendingHistoryPrefetchThreadId = threadId
        Task {
            await model.loadOlderSelectedThreadHistory()
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
}

struct GaryxConversationHeader: View {
    @EnvironmentObject private var model: GaryxMobileModel
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

                if model.selectedThread == nil {
                    GaryxHeaderAgentControl()
                        .layoutPriority(1)
                } else {
                    GaryxThreadRuntimeHeaderControl()
                    .layoutPriority(1)
                }

                Spacer(minLength: 0)

                if model.selectedThread != nil {
                    GaryxTaskTreeHeaderButton()
                }

                if let selectedThread = model.selectedThread {
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
                        Button(model.selectedThreadTasksMenuTitle, systemImage: "checklist") {
                            Task { await model.openSelectedThreadTasks() }
                        }
                        Button("Rename", systemImage: "pencil") {
                            openRenamePrompt()
                        }
                        Button("Archive", systemImage: "archivebox", role: .destructive) {
                            Task { await model.deleteSelectedThread() }
                        }
                    } label: {
                        if model.isSelectedThreadLoadingInitialHistory {
                            GaryxToolbarIcon {
                                GaryxInkSpinner()
                            }
                        } else {
                            GaryxToolbarIcon(systemName: "ellipsis")
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(
                        model.isSelectedThreadLoadingInitialHistory ? "Loading thread" : "Thread actions"
                    )
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
        .alert("Rename Thread", isPresented: $showsRenamePrompt) {
            TextField("Thread title", text: $renameDraftTitle)
            Button("Cancel", role: .cancel) {}
            Button("Save") {
                Task {
                    await model.renameSelectedThread(to: renameDraftTitle)
                }
            }
        }
        .sheet(isPresented: $showsBotBindingSheet, onDismiss: {
            botBindingThreadId = nil
        }) {
            if let botBindingThreadId {
                GaryxThreadBotBindingSheet(threadId: botBindingThreadId)
            }
        }
        .onChange(of: model.selectedThread?.id) { _, _ in
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
        renameDraftTitle = model.selectedThread?.title ?? model.draftThreadTitle
        showsRenamePrompt = true
    }

    private func goHome() {
        garyxDismissKeyboard()
        dismissThreadPresentations()
        model.popToHome()
    }

    private func dismissThreadPresentations() {
        showsRenamePrompt = false
        showsBotBindingSheet = false
        botBindingThreadId = nil
    }
}

private struct GaryxThreadRuntimeHeaderControl: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsRuntimePopover = false

    private var selectedThread: GaryxThreadSummary? { model.selectedThread }
    private var runtime: GaryxThreadRuntimeSummary? { selectedThread?.threadRuntime }
    private var title: String { selectedThread?.title ?? model.draftThreadTitle }

    private var providerType: String {
        normalized(runtime?.providerType)
            ?? normalized(selectedThread?.providerType)
            ?? normalized(model.selectedThreadAgentTarget?.providerType)
            ?? ""
    }

    var body: some View {
        Button {
            openRuntimePopover()
        } label: {
            HStack(spacing: 8) {
                if let target = model.selectedThreadAgentTarget {
                    GaryxAgentAvatarView(
                        agentId: target.id,
                        avatarDataUrl: target.avatarDataUrl,
                        kind: target.kind,
                        label: target.title,
                        providerType: target.providerType,
                        builtIn: target.builtIn,
                        diameter: 22
                    )
                }

                Text(title)
                    .font(GaryxFont.callout(weight: .medium))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(1)
            }
            .padding(.horizontal, 12)
            .frame(height: 44, alignment: .leading)
            .frame(maxWidth: 282, alignment: .leading)
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: false,
                fallbackMaterial: .ultraThinMaterial,
                in: Capsule()
            )
        }
        .buttonStyle(.plain)
        .contentShape(Capsule())
        .accessibilityLabel("\(title), thread settings")
        .layoutPriority(1)
        .task(id: providerType) {
            guard !providerType.isEmpty,
                  model.providerModelsByType[providerType] == nil else {
                return
            }
            await model.loadProviderModels(providerType: providerType)
        }
        .onChange(of: selectedThread?.id) { _, _ in
            showsRuntimePopover = false
        }
        .onChange(of: model.sidebarVisible) { _, visible in
            if visible {
                showsRuntimePopover = false
            }
        }
        .onChange(of: model.activePanel) { _, panel in
            if panel != .chat {
                showsRuntimePopover = false
            }
        }
        .onChange(of: model.showsSettings) { _, visible in
            if visible {
                showsRuntimePopover = false
            }
        }
        .onChange(of: showsRuntimePopover) { _, visible in
            if visible {
                garyxDismissKeyboard()
            }
        }
        .popover(
            isPresented: $showsRuntimePopover,
            attachmentAnchor: .rect(.bounds),
            arrowEdge: .top
        ) {
            GaryxThreadRuntimeSettingsSheet()
                .environmentObject(model)
                .presentationCompactAdaptation(.popover)
        }
    }

    private func openRuntimePopover() {
        garyxDismissKeyboard()
        showsRuntimePopover.toggle()
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
