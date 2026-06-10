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

/// Transcript loading placeholder: a chat-shaped skeleton (user pill on the
/// trailing edge, assistant text lines on the leading edge) swept by the same
/// soft shimmer treatment as `GaryxShimmerText`, instead of a bare spinner.
struct GaryxThreadHistoryLoadingView: View {
    private static let shimmerDuration: Double = 2.4

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: false)) { context in
            let normalized = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: Self.shimmerDuration) / Self.shimmerDuration
            let phase = CGFloat(normalized) * 2.0 - 0.5
            let fill = LinearGradient(
                colors: [
                    Color.primary.opacity(0.05),
                    Color.primary.opacity(0.11),
                    Color.primary.opacity(0.05),
                ],
                startPoint: UnitPoint(x: phase - 0.6, y: 0.35),
                endPoint: UnitPoint(x: phase + 0.6, y: 0.65)
            )

            VStack(alignment: .leading, spacing: 18) {
                userBubble(width: 168, fill: fill)
                assistantLines(trailingInsets: [24, 64, 148], fill: fill)
                userBubble(width: 122, fill: fill)
                assistantLines(trailingInsets: [40, 96], fill: fill)

                GaryxShimmerText(
                    text: "Loading thread",
                    font: GaryxFont.footnote(weight: .medium)
                )
                .frame(maxWidth: .infinity)
                .padding(.top, 10)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Loading thread")
    }

    private func userBubble(width: CGFloat, fill: LinearGradient) -> some View {
        RoundedRectangle(cornerRadius: 19, style: .continuous)
            .fill(fill)
            .frame(width: width, height: 38)
            .frame(maxWidth: .infinity, alignment: .trailing)
    }

    private func assistantLines(trailingInsets: [CGFloat], fill: LinearGradient) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            ForEach(Array(trailingInsets.enumerated()), id: \.offset) { _, inset in
                RoundedRectangle(cornerRadius: 7, style: .continuous)
                    .fill(fill)
                    .frame(height: 14)
                    .padding(.trailing, inset)
            }
        }
    }
}

struct GaryxLoadEarlierHistoryButton: View {
    let isLoading: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                if isLoading {
                    ProgressView()
                        .scaleEffect(0.68)
                } else {
                    Image(systemName: "chevron.up")
                        .font(GaryxFont.system(size: 12, weight: .semibold))
                }
                Text(isLoading ? "Loading earlier" : "Load Earlier")
                    .font(GaryxFont.caption(weight: .semibold))
            }
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
        }
        .buttonStyle(.plain)
        .disabled(isLoading)
    }
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

struct GaryxConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxSidebarDragActive) private var sidebarDragActive
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
    @State private var pendingHistoryPrefetchThreadId: String?
    @State private var bottomChromeHeight: CGFloat = 0
    @State private var tailScrollRequestGeneration = 0

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
                }
                .onChange(of: model.selectedThread?.id) { _, _ in
                    scrollPreservationThreadId = model.selectedThread?.id
                    pendingHistoryPrefetchThreadId = nil
                    updateScrollState(proxy: proxy) { $0.threadOpened() }
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
                            hasTailContent: !newValue.isEmpty || model.showsTailThinkingIndicator
                        )
                    }
                }
                .onChange(of: model.showsTailThinkingIndicator) { _, visible in
                    guard visible else { return }
                    updateScrollState(proxy: proxy) { $0.thinkingIndicatorShown() }
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

                if model.messages.isEmpty,
                   model.isLoadingSelectedThreadHistory || model.isSelectedThreadAwaitingInitialHistory {
                    GaryxThreadHistoryLoadingView()
                        .padding(.top, 12)
                } else if model.messages.isEmpty {
                    if model.showsTailThinkingIndicator {
                        GaryxThinkingLabel()
                            .padding(.top, 96)
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
                            prefetchOlderHistoryIfNeeded(ignoreDistance: true)
                        }
                    }
                    GaryxMobileTurnRowsView(
                        rows: model.selectedThreadTurnRows(),
                        // The run-level sending flag, not per-block pending
                        // state: a lull between steps must not flip the
                        // trailing turn to "Worked" and auto-collapse it
                        // while the run is still going.
                        forceRunningLastTurn: model.isSelectedThreadSending,
                        prefetchBoundaryRowCount: garyxHistoryPrefetchBoundaryRows
                    ) {
                        prefetchOlderHistoryIfNeeded(ignoreDistance: true)
                    }
                    if model.showsTailThinkingIndicator {
                        GaryxThinkingLabel()
                            .id(tailThinkingAnchorId)
                            .transition(.garyxTranscriptAppear)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 18)
            .padding(.bottom, 24)
            .garyxVerticalScrollContentWidth(alignment: .topLeading)
            // A short entrance animation keyed to cheap insertion signals
            // (message count, indicator visibility), so new bubbles and tool
            // rows ease in instead of popping. Streaming text growth and
            // scroll measurements never re-key it.
            .animation(.easeOut(duration: 0.2), value: model.messages.count)
            .animation(.easeOut(duration: 0.2), value: model.showsTailThinkingIndicator)

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
                hasTailContent: !model.messages.isEmpty || model.showsTailThinkingIndicator
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
        let delays: [DispatchTimeInterval] = [
            .milliseconds(0), .milliseconds(16), .milliseconds(40), .milliseconds(140),
            .milliseconds(320), .milliseconds(650), .milliseconds(1_000),
        ]

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

    private var conversationScrollIdentity: String {
        model.selectedThread?.id ?? "garyx-draft-thread"
    }

    private var conversationBottomAnchorId: String {
        "conversation-bottom-anchor-\(conversationScrollIdentity)"
    }

    private var tailThinkingAnchorId: String {
        "tail-thinking-\(conversationScrollIdentity)"
    }

    private func prefetchOlderHistoryIfNeeded(ignoreDistance: Bool = false) {
        guard let threadId = model.selectedThread?.id,
              scrollStateBox.state.shouldPrefetchOlderHistory(
                hasMoreHistoryBefore: model.selectedThreadHasMoreHistoryBefore,
                isLoadingOlderHistory: model.isLoadingOlderThreadHistory,
                hasPendingPrefetch: pendingHistoryPrefetchThreadId == threadId,
                ignoreDistance: ignoreDistance
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
                GaryxSidebarMenuButton(action: openSidebar)

                if model.selectedThread == nil {
                    GaryxHeaderAgentControl()
                        .layoutPriority(1)
                } else {
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

                        Text(model.selectedThread?.title ?? model.draftThreadTitle)
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
                    .layoutPriority(1)
                }

                Spacer(minLength: 0)

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
                        GaryxToolbarIcon(systemName: "ellipsis")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Thread actions")
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

    private func openSidebar() {
        garyxDismissKeyboard()
        dismissThreadPresentations()
        model.setSidebarVisible(true)
    }

    private func dismissThreadPresentations() {
        showsRenamePrompt = false
        showsBotBindingSheet = false
        botBindingThreadId = nil
    }
}

private struct GaryxThreadBotBindingSheet: View {
    let threadId: String

    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var isApplying = false

    private var boundGroup: GaryxMobileBotGroup? {
        GaryxMobileBotGroupBuilder.selectedGroup(
            threadId: threadId,
            groups: model.mobileBotGroups
        )
    }

    private var boundBot: GaryxConfiguredBot? {
        guard let boundGroup else { return nil }
        return garyxConfiguredBot(for: boundGroup, in: model.configuredBots)
    }

    private var selectableGroups: [GaryxMobileBotGroup] {
        model.mobileBotGroups.filter {
            garyxConfiguredBot(for: $0, in: model.configuredBots) != nil
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            botBindingSheetHeader

            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
                        VStack(spacing: 0) {
                            if !selectableGroups.isEmpty || boundBot != nil {
                                botOptionRow(
                                    title: "No bot",
                                    subtitle: "Do not bind this thread to any bot",
                                    channel: boundBot?.channel ?? "",
                                    iconDataUrl: nil,
                                    systemName: "link.slash",
                                    isSelected: boundGroup == nil,
                                    usesBotLogo: false
                                ) {
                                    if let boundBot {
                                        apply {
                                            await model.unbindBot(boundBot)
                                        }
                                    } else {
                                        dismiss()
                                    }
                                }

                                if !selectableGroups.isEmpty {
                                    Divider().padding(.leading, 56)
                                }
                            }

                            if selectableGroups.isEmpty {
                                emptyState
                            } else {
                                ForEach(Array(selectableGroups.enumerated()), id: \.element.id) { index, group in
                                    if let bot = garyxConfiguredBot(for: group, in: model.configuredBots) {
                                        botOptionRow(
                                            title: group.title,
                                            subtitle: group.subtitle,
                                            channel: group.channel,
                                            iconDataUrl: group.iconDataUrl,
                                            systemName: "bubble.left.and.bubble.right",
                                            isSelected: group.id == boundGroup?.id
                                        ) {
                                            guard group.id != boundGroup?.id else {
                                                dismiss()
                                                return
                                            }
                                            apply {
                                                await model.bindBot(bot, toThreadId: threadId)
                                            }
                                        }
                                        if index < selectableGroups.count - 1 {
                                            Divider().padding(.leading, 56)
                                        }
                                    }
                                }
                            }
                        }
                        .padding(.horizontal, 10)
                        .padding(.vertical, 8)
                    }
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
                .garyxVerticalScrollContentWidth()
            }
            .scrollIndicators(.hidden)
        }
        .garyxBotBindingSheetStyle()
        .onChange(of: model.selectedThread?.id) { _, nextThreadId in
            if nextThreadId != threadId {
                dismiss()
            }
        }
        .onChange(of: model.sidebarVisible) { _, visible in
            if visible {
                dismiss()
            }
        }
        .onChange(of: model.activePanel) { _, panel in
            if panel != .chat {
                dismiss()
            }
        }
    }

    private var botBindingSheetHeader: some View {
        HStack(alignment: .center, spacing: 12) {
            Text("Thread Bot")
                .font(GaryxFont.callout(weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
            Spacer(minLength: 0)
            Button {
                dismiss()
            } label: {
                GaryxCompactGlassIcon(systemName: "xmark")
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Close")
        }
        .padding(.horizontal, 22)
        .padding(.top, 22)
        .padding(.bottom, 14)
    }

    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("No bots configured")
                .font(GaryxFont.subheadline(weight: .semibold))
                .foregroundStyle(.primary)
            Text("Add a bot in Settings before binding one to this thread.")
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 14)
    }

    private func botOptionRow(
        title: String,
        subtitle: String,
        channel: String,
        iconDataUrl: String?,
        systemName: String,
        isSelected: Bool,
        usesBotLogo: Bool = true,
        role: ButtonRole? = nil,
        isDestructive: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(role: role, action: action) {
            HStack(spacing: 12) {
                if usesBotLogo {
                    GaryxChannelLogoView(
                        channel: channel,
                        label: title,
                        iconDataUrl: iconDataUrl,
                        diameter: 34
                    )
                } else {
                    Image(systemName: systemName)
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(isDestructive ? .red : .secondary)
                        .frame(width: 34, height: 34)
                        .background(Color(.secondarySystemFill).opacity(0.72), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(isDestructive ? .red : .primary)
                        .lineLimit(1)
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                Spacer(minLength: 0)
                if isSelected {
                    GaryxSelectionCheckmark(size: 12)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, minHeight: 54, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(isApplying)
        .opacity(isApplying ? 0.62 : 1)
    }

    private func apply(_ operation: @escaping () async -> Void) {
        guard !isApplying else { return }
        isApplying = true
        dismiss()
        Task {
            await operation()
            await MainActor.run {
                isApplying = false
            }
        }
    }
}

private func garyxConfiguredBot(
    for group: GaryxMobileBotGroup,
    in configuredBots: [GaryxConfiguredBot]
) -> GaryxConfiguredBot? {
    configuredBots.first {
        $0.channel.caseInsensitiveCompare(group.channel) == .orderedSame
            && $0.accountId == group.accountId
    }
}

private extension View {
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

    func garyxBotBindingSheetStyle() -> some View {
        self
            .background {
                Rectangle()
                    .fill(Color(.systemBackground).opacity(0.98))
                    .overlay {
                        LinearGradient(
                            colors: [
                                Color.white.opacity(0.28),
                                Color.white.opacity(0.10)
                            ],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    }
                    .ignoresSafeArea()
            }
            .presentationBackground(.clear)
            .presentationBackgroundInteraction(.enabled)
            .presentationDetents([.fraction(0.93), .large])
            .presentationDragIndicator(.hidden)
            .presentationCornerRadius(38)
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

struct GaryxEmptyConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsWorkspacePicker = false

    var body: some View {
        VStack(spacing: 18) {
            Text("New Thread")
                .font(GaryxFont.title3(weight: .semibold))
                .foregroundStyle(.primary)

            workspacePicker
        }
        .frame(maxWidth: 300)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 28)
        .sheet(isPresented: $showsWorkspacePicker) {
            GaryxWorkspaceSelectSheet(
                title: "Workspace",
                path: newThreadWorkspaceBinding,
                workspacePaths: model.userWorkspacePaths,
                placeholder: "No workspace",
                allowsEmpty: true
            )
        }
        // Prefetch the catalog so the agent picker's override section is ready.
        .task(id: model.newThreadAgentTarget?.id) {
            await model.ensureNewThreadProviderModelsLoaded()
        }
        .onChange(of: model.sidebarVisible) { _, visible in
            if visible {
                showsWorkspacePicker = false
            }
        }
        .onChange(of: model.selectedThread?.id) { _, threadId in
            if threadId != nil {
                showsWorkspacePicker = false
            }
        }
        .onChange(of: model.activePanel) { _, panel in
            if panel != .chat {
                showsWorkspacePicker = false
            }
        }
    }

    private var workspacePicker: some View {
        Button {
            showsWorkspacePicker = true
        } label: {
            HStack(spacing: 10) {
                Text(model.newThreadWorkspaceLabel)
                    .font(GaryxFont.body(weight: .semibold))
                    .lineLimit(1)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.system(size: 10, weight: .bold))
            }
            .foregroundStyle(Color(.systemBackground))
            .padding(.horizontal, 18)
            .frame(height: 46)
            .background(Color(.label), in: Capsule())
        }
        .buttonStyle(.plain)
    }

    private var newThreadWorkspaceBinding: Binding<String> {
        Binding {
            model.newThreadWorkspace
        } set: { value in
            model.setNewThreadWorkspace(value)
        }
    }

}

private struct GaryxSelectedThreadEmptyConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(spacing: 14) {
            Text(model.selectedThread?.title ?? "Thread")
                .font(GaryxFont.title3(weight: .semibold))
                .foregroundStyle(.primary)
                .multilineTextAlignment(.center)
                .lineLimit(2)

            Text("No messages yet")
                .font(GaryxFont.callout())
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: 300)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 28)
    }
}

struct GaryxMessageBubble: View {
    let message: GaryxMobileMessage
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var retrying = false
    @State private var filePreviewSheet: GaryxMessageFilePreviewSheet?

    var body: some View {
        Group {
            if let group = message.toolTraceGroup {
                GaryxToolTraceGroupView(group: group)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                messageRow
            }
        }
        .fullScreenCover(item: $filePreviewSheet) { sheet in
            GaryxFullscreenWorkspaceFilePreview(preview: sheet.preview) {
                filePreviewSheet = nil
            }
            .environmentObject(model)
        }
    }

    @ViewBuilder
    private var messageRow: some View {
        switch message.role {
        case .user:
            HStack(alignment: .bottom) {
                Spacer(minLength: 60)
                VStack(alignment: .trailing, spacing: 4) {
                    if !message.attachments.isEmpty {
                        GaryxMessageAttachmentStack(attachments: message.attachments, isUser: true)
                            .garyxMessageCopyContext(text: messageCopyText)
                    }

                    if !displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        GaryxMarkdownText(
                            text: displayText,
                            foreground: .primary,
                            codeBackground: userCodeBackground,
                            codeBorder: GaryxTheme.hairline,
                            fillsAvailableWidth: false,
                            allowsTextSelection: false,
                            onFileLinkTap: openMessageFileLink,
                            onImageFilePreview: messageImageFilePreview
                        )
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(userBubbleBackground, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                        .garyxMessageInteraction(text: displayText)
                    }

                    if let statusText = message.statusText, !statusText.isEmpty {
                        failureStatusRow(statusText: statusText)
                    }
                }
                .frame(maxWidth: UIScreen.main.bounds.width * 0.77, alignment: .trailing)
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        case .assistant:
            VStack(alignment: .leading, spacing: 8) {
                if !message.attachments.isEmpty {
                    GaryxMessageAttachmentStack(attachments: message.attachments, isUser: false)
                        .garyxMessageCopyContext(text: messageCopyText)
                }
                if message.isStreaming && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    if message.attachments.isEmpty {
                        GaryxThinkingLabel()
                    }
                } else if !displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    GaryxMarkdownText(
                        text: displayText,
                        foreground: .primary,
                        allowsTextSelection: false,
                        onFileLinkTap: openMessageFileLink,
                        onImageFilePreview: messageImageFilePreview
                    )
                    .garyxMessageInteraction(text: displayText)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // Smooth the height growth while this bubble streams. Settled
            // bubbles compare their (storage-shared) text in O(1) and never
            // animate, so long transcripts pay nothing.
            .animation(message.isStreaming ? .easeOut(duration: 0.16) : nil, value: message.text)
        case .system:
            GaryxMarkdownText(
                text: displayText,
                foreground: .secondary,
                fillsAvailableWidth: false,
                allowsTextSelection: false,
                onFileLinkTap: openMessageFileLink,
                onImageFilePreview: messageImageFilePreview
            )
                .font(GaryxFont.footnote())
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .stroke(GaryxTheme.hairline, style: StrokeStyle(lineWidth: 1, dash: [4, 4]))
                }
                .frame(maxWidth: 720, alignment: .center)
                .frame(maxWidth: .infinity, alignment: .center)
                .garyxMessageInteraction(text: displayText)
        case .tool:
            EmptyView()
        }
    }

    private var displayText: String {
        if message.text.isEmpty, message.isStreaming { return "Thinking" }
        if !message.attachments.isEmpty,
           let summary = GaryxStructuredContentRenderer.attachmentSummary(
            from: message.attachments.map(\.contentDescriptor)
           ),
           message.text == summary {
            return ""
        }
        return message.text
    }

    private var messageCopyText: String {
        var parts: [String] = []
        if !displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            parts.append(displayText)
        }
        let attachmentText = message.attachments
            .compactMap(Self.copyTextLine(for:))
            .joined(separator: "\n")
        if !attachmentText.isEmpty {
            parts.append(attachmentText)
        }
        return parts.joined(separator: "\n\n")
    }

    private static func copyTextLine(for attachment: GaryxMobileMessageAttachment) -> String? {
        let title = attachment.name.trimmingCharacters(in: .whitespacesAndNewlines)
        let fallback = attachment.isImage ? "Image" : "Attachment"
        let label = title.isEmpty ? fallback : title
        if let path = attachment.path?.trimmingCharacters(in: .whitespacesAndNewlines),
           !path.isEmpty {
            return "\(label): \(path)"
        }
        if let remoteUrl = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
           !remoteUrl.isEmpty {
            return "\(label): \(remoteUrl)"
        }
        if attachment.dataUrl?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false {
            return "\(label): inline \(attachment.isImage ? "image" : "attachment")"
        }
        return title.isEmpty ? nil : label
    }

    private var userBubbleBackground: Color {
        (colorScheme == .dark ? Color.white.opacity(0.12) : Color.black.opacity(0.05))
    }

    private var userCodeBackground: Color {
        colorScheme == .dark ? Color.white.opacity(0.08) : Color.black.opacity(0.055)
    }

    private func openMessageFileLink(_ target: String) {
        Task {
            guard let preview = await model.localFilePreview(target) else { return }
            filePreviewSheet = GaryxMessageFilePreviewSheet(preview: preview)
        }
    }

    @MainActor
    private func messageImageFilePreview(_ target: String) async -> GaryxWorkspaceFilePreview? {
        await model.localFilePreview(target, reportsError: false)
    }

    @ViewBuilder
    private func failureStatusRow(statusText: String) -> some View {
        let canRetry = message.id.hasPrefix("local-user-")
            || message.id.hasPrefix("pending-user:")
        if canRetry {
            Button {
                guard !retrying else { return }
                retrying = true
                Task {
                    _ = await model.retryFailedUserMessage(message.id)
                    retrying = false
                }
            } label: {
                HStack(spacing: 6) {
                    if retrying {
                        ProgressView()
                            .controlSize(.mini)
                    } else {
                        Image(systemName: "arrow.clockwise")
                            .font(GaryxFont.system(size: 11, weight: .semibold))
                    }
                    Text(retrying ? "Retrying…" : statusText)
                        .font(GaryxFont.caption())
                        .lineLimit(2)
                        .multilineTextAlignment(.trailing)
                }
                .foregroundStyle(Color(.systemRed))
            }
            .buttonStyle(.plain)
            .disabled(retrying)
            .accessibilityLabel(Text("Retry message"))
            .accessibilityHint(Text(statusText))
        } else {
            Text(statusText)
                .font(GaryxFont.caption())
                .foregroundStyle(Color(.systemRed))
                .lineLimit(2)
                .multilineTextAlignment(.trailing)
        }
    }
}

private struct GaryxMessageFilePreviewSheet: Identifiable {
    let id = UUID()
    let preview: GaryxWorkspaceFilePreview
}

private struct GaryxMessageCopyContextModifier: ViewModifier {
    let text: String
    var title = "Copy Message"

    private var copyableText: String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func body(content: Content) -> some View {
        content
            .contextMenu {
                if !copyableText.isEmpty {
                    Button {
                        GaryxClipboard.copyString(text)
                    } label: {
                        Label(title, systemImage: "doc.on.doc")
                    }
                }
            }
            .accessibilityAction(named: Text(title)) {
                guard !copyableText.isEmpty else { return }
                GaryxClipboard.copyString(text)
            }
    }
}

private extension View {
    func garyxMessageCopyContext(text: String, title: String = "Copy Message") -> some View {
        modifier(GaryxMessageCopyContextModifier(text: text, title: title))
    }

    func garyxMessageInteraction(text: String) -> some View {
        modifier(GaryxMessageInteractionModifier(text: text))
    }
}

/// Long-press surface for message bubbles: copy the whole message, open the
/// drag-handle selection sheet, or share. This replaces SwiftUI text
/// selection inside the transcript, which could not select ranges and fought
/// the long-press gesture.
private struct GaryxMessageInteractionModifier: ViewModifier {
    let text: String

    @State private var showsTextSelection = false

    private var copyableText: String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func body(content: Content) -> some View {
        content
            .contextMenu {
                if !copyableText.isEmpty {
                    Button {
                        GaryxClipboard.copyString(text)
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                    Button {
                        showsTextSelection = true
                    } label: {
                        Label("Select Text", systemImage: "character.cursor.ibeam")
                    }
                    ShareLink(item: text) {
                        Label("Share", systemImage: "square.and.arrow.up")
                    }
                }
            }
            .sheet(isPresented: $showsTextSelection) {
                GaryxMessageTextSelectionSheet(text: text)
            }
    }
}

struct GaryxMessageAttachmentStack: View {
    let attachments: [GaryxMobileMessageAttachment]
    let isUser: Bool

    private var images: [GaryxMobileMessageAttachment] {
        attachments.filter(\.isImage)
    }

    private var files: [GaryxMobileMessageAttachment] {
        attachments.filter { !$0.isImage }
    }

    var body: some View {
        VStack(alignment: isUser ? .trailing : .leading, spacing: 6) {
            ForEach(images) { attachment in
                GaryxMessageImageAttachmentView(attachment: attachment, isUser: isUser)
            }
            ForEach(files) { attachment in
                GaryxMessageFileAttachmentView(attachment: attachment, isUser: isUser)
            }
        }
    }
}

struct GaryxMessageImageAttachmentView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    let attachment: GaryxMobileMessageAttachment
    let isUser: Bool

    @State private var decodedImage: UIImage?
    @State private var decodedImageKey: String?
    @State private var gatewayPreviewDataUrl: String?
    @State private var showsPreview = false

    var body: some View {
        Button {
            showsPreview = true
        } label: {
            ZStack {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .fill(Color(.secondarySystemFill))

                if let image = decodedImage {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFill()
                } else if let remoteURL {
                    AsyncImage(url: remoteURL) { phase in
                        if let image = phase.image {
                            image
                                .resizable()
                                .scaledToFill()
                        } else {
                            fallback
                        }
                    }
                } else {
                    fallback
                }
            }
            .frame(width: 218, height: 154)
            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(Color.primary.opacity(0.08), lineWidth: 1)
            }
        }
        .buttonStyle(.plain)
        .fullScreenCover(isPresented: $showsPreview) {
            GaryxFullscreenImagePreview(
                source: GaryxImagePreviewSource(
                    title: attachment.name.isEmpty ? "Image" : attachment.name,
                    dataUrl: attachment.dataUrl ?? gatewayPreviewDataUrl,
                    remoteUrl: attachment.remoteUrl,
                    filePath: gatewayPreviewDataUrl == nil ? Self.localFilePath(from: attachment.path) : nil
                )
            ) {
                showsPreview = false
            }
        }
        .contextMenu {
            if let decodedImage {
                Button {
                    GaryxClipboard.copyImage(decodedImage)
                } label: {
                    Label("Copy Image", systemImage: "photo.on.rectangle")
                }
            }
            if let sourceText = imageSourceText {
                Button {
                    GaryxClipboard.copyString(sourceText)
                } label: {
                    Label("Copy Image Source", systemImage: "link")
                }
            }
            if !attachment.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Button {
                    GaryxClipboard.copyString(attachment.name)
                } label: {
                    Label("Copy Name", systemImage: "text.cursor")
                }
            }
        }
        .accessibilityAction(named: Text("Copy Image Source")) {
            guard let imageSourceText else { return }
            GaryxClipboard.copyString(imageSourceText)
        }
        .accessibilityLabel(attachment.name.isEmpty ? "Image attachment" : attachment.name)
        .accessibilityHint("Opens full screen preview")
        .task(id: dataUrlDecodeKey) {
            await updateDecodedImage()
        }
    }

    @ViewBuilder
    private var fallback: some View {
        VStack(spacing: 6) {
            Image(systemName: "photo")
                .font(GaryxFont.title3(weight: .medium))
            Text(attachment.name.isEmpty ? "Image" : attachment.name)
                .font(GaryxFont.caption(weight: .medium))
                .lineLimit(1)
                .truncationMode(.middle)
                .padding(.horizontal, 10)
        }
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var dataUrlDecodeKey: String {
        let raw = attachment.dataUrl ?? ""
        let path = attachment.path ?? ""
        return "\(attachment.id):\(raw.count):\(raw.hashValue):\(path.hashValue)"
    }

    @MainActor
    private func updateDecodedImage() async {
        let key = dataUrlDecodeKey
        guard decodedImageKey != key else { return }
        decodedImage = nil
        gatewayPreviewDataUrl = nil
        decodedImageKey = key
        if let raw = attachment.dataUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
           !raw.isEmpty {
            let image = await Task.detached(priority: .utility) {
                GaryxImageDecoder.image(fromDataUrl: raw, maxPixelSize: 520)
            }.value
            guard !Task.isCancelled, decodedImageKey == key else { return }
            decodedImage = image
            return
        }
        guard let path = attachment.path?.trimmingCharacters(in: .whitespacesAndNewlines),
              !path.isEmpty,
              let preview = await model.localFilePreview(path, reportsError: false),
              preview.previewKind == "image",
              let dataUrl = preview.dataBase64?.trimmingCharacters(in: .whitespacesAndNewlines),
              !dataUrl.isEmpty else {
            return
        }
        let image = await Task.detached(priority: .utility) {
            GaryxImageDecoder.image(fromDataUrl: dataUrl, maxPixelSize: 520)
        }.value
        guard !Task.isCancelled, decodedImageKey == key else { return }
        gatewayPreviewDataUrl = dataUrl
        decodedImage = image
    }

    private var remoteURL: URL? {
        guard let raw = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
              raw.hasPrefix("http://") || raw.hasPrefix("https://") else {
            return nil
        }
        return URL(string: raw)
    }

    private var imageSourceText: String? {
        if let remoteUrl = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
           !remoteUrl.isEmpty {
            return remoteUrl
        }
        if let path = attachment.path?.trimmingCharacters(in: .whitespacesAndNewlines),
           !path.isEmpty {
            return path
        }
        return nil
    }

    private static func localFilePath(from value: String?) -> String? {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty else { return nil }
        if value.hasPrefix("file://") {
            return URL(string: value)?.path
        }
        if value.hasPrefix("/") {
            return value
        }
        return nil
    }
}

struct GaryxMessageFileAttachmentView: View {
    let attachment: GaryxMobileMessageAttachment
    let isUser: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "doc")
                .font(GaryxFont.footnote(weight: .semibold))
                .frame(width: 18, height: 18)
            Text(attachment.name.isEmpty ? "Attachment" : attachment.name)
                .font(GaryxFont.footnote(weight: .medium))
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .foregroundStyle(.primary)
        .padding(.horizontal, 11)
        .frame(height: 34)
        .background(
            isUser ? Color.black.opacity(0.06) : Color(.secondarySystemFill),
            in: Capsule()
        )
        .contextMenu {
            if let sourceText {
                Button {
                    GaryxClipboard.copyString(sourceText)
                } label: {
                    Label("Copy File Path", systemImage: "doc.on.doc")
                }
            }
            if !attachment.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Button {
                    GaryxClipboard.copyString(attachment.name)
                } label: {
                    Label("Copy Name", systemImage: "text.cursor")
                }
            }
        }
        .accessibilityAction(named: Text("Copy File Path")) {
            guard let sourceText else { return }
            GaryxClipboard.copyString(sourceText)
        }
        .accessibilityLabel(attachment.name.isEmpty ? "File attachment" : attachment.name)
    }

    private var sourceText: String? {
        if let path = attachment.path?.trimmingCharacters(in: .whitespacesAndNewlines),
           !path.isEmpty {
            return path
        }
        if let remoteUrl = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
           !remoteUrl.isEmpty {
            return remoteUrl
        }
        return nil
    }
}

struct GaryxThinkingLabel: View {
    var body: some View {
        GaryxShimmerText(text: "Thinking", font: GaryxFont.body())
            .frame(minHeight: 22)
    }
}

struct GaryxShimmerText: View {
    let text: String
    var font: Font = GaryxFont.body()
    var baseColor: Color = GaryxTheme.secondaryText
    var peakColor: Color = Color(.label)
    var duration: Double = 2.6

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: false)) { context in
            let normalized = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: duration) / duration
            let phase = CGFloat(normalized) * 2.0 - 0.5

            Text(text)
                .font(font)
                .foregroundStyle(
                    LinearGradient(
                        colors: [baseColor, peakColor, baseColor],
                        startPoint: UnitPoint(x: phase - 0.5, y: 0.5),
                        endPoint: UnitPoint(x: phase + 0.5, y: 0.5)
                    )
                )
        }
        .accessibilityLabel(text)
    }
}
