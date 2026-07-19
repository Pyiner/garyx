import Foundation
import SwiftUI

/// Root content column owned exclusively by the UIKit occurrence stack.
struct GaryxRootNavigationView: View, Equatable {
    @ObservedObject var routeStore: GaryxProductionRouteStore
    @ObservedObject var routeNotFoundStore: GaryxRouteNotFoundStore
    @ObservedObject var homeListStore: GaryxHomeThreadListStore
    let model: GaryxMobileModel
    let isSidebarDragActive: Bool
    let onOpenDrawer: () -> Void
    let onRefreshAll: () async -> Void
    let onRefreshSidebarThreads: () async -> Void
    let onLoadMoreThreads: (GaryxThreadListLoadMoreTrigger) async -> Void
    let onRetryLoadMoreThreads: () async -> Void
    let onSelectRecentFilter: (GaryxRecentThreadFilter) -> Void
    let onStartNewChat: () -> Void
    let onOpenThread: (GaryxThreadSummary, GaryxMobilePanelOpenSource) -> Void
    let onTogglePinnedThread: (String) -> Void
    let onToggleFavoriteThread: (String) -> Void
    let onUnpinThread: (String) -> Void
    let onBeginPinnedOrderDrag: () -> Void
    let onPreviewPinnedOrderDrag: ([String]) -> Void
    let onAcceptPinnedOrderDrop: () -> Void
    let onCancelPinnedOrderDrag: () -> Void
    let onArchiveThread: (GaryxThreadSummary) async -> Void

    static func == (lhs: GaryxRootNavigationView, rhs: GaryxRootNavigationView) -> Bool {
        lhs.routeStore === rhs.routeStore
            && lhs.routeNotFoundStore === rhs.routeNotFoundStore
            && lhs.homeListStore === rhs.homeListStore
            && lhs.isSidebarDragActive == rhs.isSidebarDragActive
    }

    var body: some View {
        #if DEBUG
        let _ = GaryxHomeScrollPerformanceProbe.shared.markRootBody()
        #endif
        GaryxProductionRouteStack(
            store: routeStore,
            model: model,
            homeContent: AnyView(homeContent),
            routeContent: { node in
                guard case .entry(let entry) = node else {
                    return AnyView(EmptyView())
                }
                return AnyView(GaryxRootRouteContentView(destination: entry.destination))
            },
            onOpenDrawer: onOpenDrawer
        )
        .garyxPageBackground()
        .garyxFullScreenCover(item: $routeNotFoundStore.selection) { state in
            GaryxFormSheet(title: state.title) {
                GaryxRouteNotFoundCard(state: state)
            }
        }
    }

    private var homeContent: some View {
        GaryxHomeThreadListView(
            homeListStore: homeListStore,
            isSidebarDragActive: isSidebarDragActive,
            onOpenDrawer: onOpenDrawer,
            onRefreshAll: onRefreshAll,
            onRefreshSidebarThreads: onRefreshSidebarThreads,
            onLoadMoreThreads: onLoadMoreThreads,
            onRetryLoadMoreThreads: onRetryLoadMoreThreads,
            onSelectRecentFilter: onSelectRecentFilter,
            onStartNewChat: onStartNewChat,
            onOpenThread: onOpenThread,
            onTogglePinnedThread: onTogglePinnedThread,
            onToggleFavoriteThread: onToggleFavoriteThread,
            onUnpinThread: onUnpinThread,
            onBeginPinnedOrderDrag: onBeginPinnedOrderDrag,
            onPreviewPinnedOrderDrag: onPreviewPinnedOrderDrag,
            onAcceptPinnedOrderDrop: onAcceptPinnedOrderDrop,
            onCancelPinnedOrderDrag: onCancelPinnedOrderDrag,
            onArchiveThread: onArchiveThread
        )
        .equatable()
        .toolbar(.hidden, for: .navigationBar)
    }
}

private struct GaryxRootRouteContentView: View {
    let destination: GaryxRouteDestination

    var body: some View {
        switch destination {
        case .conversation, .conversationDraft:
            GaryxConversationView(destination: destination)
        case .panel(let rawPanel):
            panelContent(for: GaryxMobilePanel(rawValue: rawPanel) ?? .chat)
        case .settingsDetail(let rawTab):
            GaryxMobileSettingsPanel(
                tab: GaryxMobileSettingsTab(rawValue: rawTab) ?? .manage
            )
        case .workspaceDrilldown(let identity):
            GaryxWorkspaceBotsView(drilldown: identity.drilldown)
        }
    }

    @ViewBuilder
    private func panelContent(for panel: GaryxMobilePanel) -> some View {
        switch panel {
        case .chat:
            EmptyView()
        case .workspaces:
            GaryxWorkspacesView()
        case .automations:
            GaryxAutomationsView()
        case .capsules:
            GaryxCapsulesView()
        case .workspaceBots, .bots:
            GaryxWorkspaceBotsView()
        case .agents:
            GaryxAgentsView()
        case .skills:
            GaryxSkillsView()
        case .commands:
            GaryxCommandsView()
        case .mcp:
            GaryxMcpServersView()
        case .settings:
            GaryxMobileSettingsPanel(tab: .manage)
        }
    }
}

private struct GaryxRouteNotFoundCard: View {
    let state: GaryxMobileRouteNotFound

    var body: some View {
        GaryxEmptyPanelView(
            icon: "magnifyingglass",
            title: state.title,
            text: state.message
        )
    }
}

enum GaryxSidebarMetrics {
    static let outerHorizontalPadding: CGFloat = 16
    static let sectionHorizontalPadding: CGFloat = 24
    static let rowOuterPadding: CGFloat = 18
    static let rowInnerHorizontalPadding: CGFloat = 7
    static let rowHeight: CGFloat = 52
    static let threadRowMinHeight: CGFloat = 50
    static let rowCornerRadius: CGFloat = 12
    static let selectedThreadCornerRadius: CGFloat = 12
    static let iconFrame: CGFloat = 28
}

struct GaryxHomeThreadListView: View, Equatable {
    @ObservedObject var homeListStore: GaryxHomeThreadListStore
    @Environment(\.garyxMotion) private var motion
    @StateObject private var pinnedDragLifecycle = GaryxPinnedDragLifecycleController()
    @State private var threadMenuDismissToken = 0
    @State private var completedDropHapticTrigger = 0
    #if DEBUG
    @ObservedObject private var performanceProbe = GaryxHomeScrollPerformanceProbe.shared
    @State private var dragBaselineOrder: [String] = []
    @State private var dragPreviewOrder: [String]?
    @State private var spikeCommittedOrder: [String]?
    @State private var debugInjectedServerOrder: [String]?
    @State private var spikeCommitCount = 0
    @State private var spikeRemoteMutationCount = 0
    @State private var midLiftSnapshotStayedFrozen = false
    @State private var debugPinMoveCount = 0
    #endif
    let isSidebarDragActive: Bool
    let onOpenDrawer: () -> Void
    let onRefreshAll: () async -> Void
    let onRefreshSidebarThreads: () async -> Void
    let onLoadMoreThreads: (GaryxThreadListLoadMoreTrigger) async -> Void
    let onRetryLoadMoreThreads: () async -> Void
    let onSelectRecentFilter: (GaryxRecentThreadFilter) -> Void
    let onStartNewChat: () -> Void
    let onOpenThread: (GaryxThreadSummary, GaryxMobilePanelOpenSource) -> Void
    let onTogglePinnedThread: (String) -> Void
    let onToggleFavoriteThread: (String) -> Void
    let onUnpinThread: (String) -> Void
    let onBeginPinnedOrderDrag: () -> Void
    let onPreviewPinnedOrderDrag: ([String]) -> Void
    let onAcceptPinnedOrderDrop: () -> Void
    let onCancelPinnedOrderDrag: () -> Void
    let onArchiveThread: (GaryxThreadSummary) async -> Void
    private let silentRefreshIntervalNanos: UInt64 = 10_000_000_000

    static func == (lhs: GaryxHomeThreadListView, rhs: GaryxHomeThreadListView) -> Bool {
        lhs.homeListStore === rhs.homeListStore
            && lhs.isSidebarDragActive == rhs.isSidebarDragActive
    }

    var body: some View {
        #if DEBUG
        let _ = GaryxHomeScrollPerformanceProbe.shared.markHomeBody()
        #endif
        threadListWithBottomBar
            .frame(maxHeight: .infinity)
            .garyxPageBackground()
            .garyxFloatingBottomChrome {
                GaryxHomeNewThreadFab(action: onStartNewChat)
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .padding(.trailing, 20)
                    .padding(.bottom, 8)
            }
            .garyxAdaptiveTopBar {
                GaryxHomeHeaderView(
                    selectedRecentFilter: homeListStore.presentationSnapshot.selectedRecentFilter,
                    onOpenDrawer: onOpenDrawer,
                    onSelectRecentFilter: onSelectRecentFilter
                )
            }
            .task(id: homeListStore.snapshot.isHomeVisible) {
                await runSilentSidebarRefreshLoop()
            }
            .overlay {
                pinnedDragLifecycleAdapter
            }
            .sensoryFeedback(.selection, trigger: completedDropHapticTrigger)
            .onAppear {
                configurePinnedDragLifecycle()
            }
            #if DEBUG
            .overlay(alignment: .bottomLeading) {
                debugPerformanceProbeControls
            }
            .overlay(alignment: .topTrailing) {
                debugPinnedReorderControls
            }
            #endif
            .garyxThreadActionMenuHost(bottomInset: 88)
    }

    private var threadListWithBottomBar: some View {
        // Native List backs onto UICollectionView, so off-screen rows are truly
        // recycled and scrolling emits real UIScrollView signals (Instruments
        // Animation Hitches). Headers/spacers/footer are flat rows (no Section)
        // to keep the non-sticky parity of the old LazyVStack.
        List {
            sidebarThreadRows
        }
        .listStyle(.plain)
        .environment(\.defaultMinListRowHeight, 0)
        .scrollContentBackground(.hidden)
        // The List and its rows intentionally hide their UIKit backgrounds.
        // Keep one opaque SwiftUI backing layer so Reduce Transparency never
        // exposes the hosting window's clear surface between recycled cells.
        .background(GaryxTheme.background)
        .scrollDisabled(isSidebarDragActive)
        .scrollDismissesKeyboard(.interactively)
        .refreshable {
            await refreshAll()
        }
    }

    @ViewBuilder
    private var sidebarThreadRows: some View {
        Group {
            spacerRow(height: 4)
            sidebarThreadSections
        }
        // Rows carry their own padding/background; strip List chrome so the page
        // background shows through and custom dividers are the only separators.
        .listRowSeparator(.hidden)
        .listRowInsets(EdgeInsets())
        .listRowBackground(Color.clear)
    }

    private func spacerRow(height: CGFloat) -> some View {
        Color.clear
            .frame(height: height)
            .accessibilityHidden(true)
    }

    // Section headers and thread rows are emitted as flat List rows. Each thread
    // is exactly one row (its leading divider is folded into the row), so
    // row.id == thread.id stays stable for correct cell reuse.
    @ViewBuilder
    private var sidebarThreadSections: some View {
        let snapshot = homeListStore.presentationSnapshot
        let items = pinnedReorderItems(
            GaryxHomeThreadListLayout.primaryItems(for: snapshot)
        )
        let prefetchTriggerRowId = GaryxThreadListPageMerge.prefetchTriggerRowId(
            recentIds: snapshot.sections.recent.map(\.id)
        )

        ForEach(items) { item in
            Group {
                switch item {
                case .pinnedHeader:
                    GaryxSidebarSectionHeader(
                        title: "Pinned",
                        systemImage: "pin.fill",
                        statusLabel: homeListStore.pinnedOrderSyncStatusLabel
                    )
                        .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                        .padding(.bottom, 4)

                case let .thread(row, region):
                    GaryxThreadListRowButton(
                        input: GaryxThreadListRowInput(
                            thread: row.thread,
                            presentation: row.presentation,
                            avatar: row.avatar,
                            timestampValue: row.timestampValue,
                            capabilities: row.capabilities,
                            motion: homeListStore.rowMotion(threadId: row.id),
                            showsDivider: row.showsDivider,
                            menuDismissToken: pinnedMenuDismissToken(for: region),
                            menuMovementSuppression: pinnedMenuMovementSuppression(for: region),
                            openSource: .replace
                        ),
                        onOpenThread: onOpenThread,
                        onSetPinned: { threadId, desired in
                            if desired {
                                onTogglePinnedThread(threadId)
                            } else {
                                onUnpinThread(threadId)
                            }
                        },
                        onSetFavorite: { threadId, _ in
                            onToggleFavoriteThread(threadId)
                        },
                        onArchive: { thread, _ in
                            Task { await onArchiveThread(thread) }
                        }
                    )
                    .equatable()
                    .onAppear {
                        if region == .recent, row.id == prefetchTriggerRowId {
                            Task { await onLoadMoreThreads(.nearTail) }
                        }
                    }

                case .pinnedSpacer:
                    spacerRow(height: 10)

                case .recentHeader:
                    GaryxSidebarSectionHeader(
                        title: "Recent",
                        systemImage: "clock.fill",
                        statusLabel: snapshot.selectedRecentFilter.activeStatusLabel
                    )
                        .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                        .padding(.bottom, 4)

                case let .recentPlaceholder(placeholder):
                    recentPlaceholder(placeholder, selectedFilter: snapshot.selectedRecentFilter)
                }
            }
            .moveDisabled(pinnedMoveIsDisabled(for: item))
        }
        .onMove(perform: pinnedMoveAction)
        .animation(motion.spatialAnimation(.threadListMutation), value: items.map(\.id))

        spacerRow(height: 10)

        if snapshot.recentFeedPresentation.headFailure,
           snapshot.recentFeedPresentation.isPrimed {
            Button {
                Task { await onRefreshSidebarThreads() }
            } label: {
                GaryxSidebarEmptyRow(title: "Couldn't refresh · Tap to retry")
            }
            .buttonStyle(.plain)
        }

        GaryxSidebarThreadAutoLoadFooter()
            .environment(\.garyxLoadMoreThreads, onLoadMoreThreads)
            .environment(\.garyxRetryLoadMoreThreads, onRetryLoadMoreThreads)
    }

    @ViewBuilder
    private func recentPlaceholder(
        _ placeholder: GaryxHomeRecentPlaceholder,
        selectedFilter: GaryxRecentThreadFilter
    ) -> some View {
        switch placeholder {
        case .loadingSkeleton(let rowCount):
            GaryxSidebarSkeletonRows(rowCount: rowCount)
        case .empty:
            GaryxSidebarEmptyRow(
                title: {
                    switch selectedFilter {
                    case .all: return "No recent threads"
                    case .nonTask: return "No recent chats"
                    case .favorites: return "No favorite threads"
                    }
                }()
            )
        case .unavailable:
            Button {
                Task { await onRefreshSidebarThreads() }
            } label: {
                GaryxSidebarEmptyRow(title: "Recent threads unavailable · Tap to retry")
            }
            .buttonStyle(.plain)
        case .none:
            EmptyView()
        }
    }

    private func refreshAll() async {
        await onRefreshAll()
    }

    private func runSilentSidebarRefreshLoop() async {
        guard shouldRefreshSidebarThreads else { return }
        // Let the drawer-open animation settle before the first refresh so
        // response handling does not contend with the opening transition.
        try? await Task.sleep(for: .seconds(GaryxMotion.drawerRefreshDeferral))
        guard !Task.isCancelled, shouldRefreshSidebarThreads else { return }
        await refreshSidebarThreads()
        while !Task.isCancelled {
            try? await Task.sleep(nanoseconds: silentRefreshIntervalNanos)
            guard !Task.isCancelled, shouldRefreshSidebarThreads else { return }
            await refreshSidebarThreads()
        }
    }

    private func refreshSidebarThreads() async {
        guard shouldRefreshSidebarThreads else { return }
        // Concurrent refreshes coalesce inside the pager; no extra gate.
        await onRefreshSidebarThreads()
    }

    private var shouldRefreshSidebarThreads: Bool {
        homeListStore.presentationSnapshot.isHomeVisible
    }

    private func pinnedReorderItems(
        _ items: [GaryxHomeThreadListItem]
    ) -> [GaryxHomeThreadListItem] {
        #if DEBUG
        if GaryxPinnedThreadReorderRuntimeGate.isArchitectureSpikeEnabled {
            let serverItems = applyingPinnedOrder(debugInjectedServerOrder, to: items)
            return applyingPinnedOrder(dragPreviewOrder ?? spikeCommittedOrder, to: serverItems)
        }
        #endif
        guard GaryxPinnedThreadReorderRuntimeGate.isFeatureEnabled else { return items }
        return applyingPinnedOrder(homeListStore.pinnedOrderState.presentedOrder, to: items)
    }

    private func applyingPinnedOrder(
        _ order: [String]?,
        to items: [GaryxHomeThreadListItem]
    ) -> [GaryxHomeThreadListItem] {
        guard let order else { return items }
        let pinned = items.compactMap { item -> GaryxHomeThreadListItem? in
            guard case let .thread(_, region) = item, region == .pinned else { return nil }
            return item
        }
        let byId = Dictionary(uniqueKeysWithValues: pinned.map { ($0.id, $0) })
        var seen = Set<String>()
        var reordered = order.compactMap { id -> GaryxHomeThreadListItem? in
            let itemId = "thread:\(id)"
            guard seen.insert(itemId).inserted else { return nil }
            return byId[itemId]
        }
        reordered += pinned.filter { seen.insert($0.id).inserted }
        var iterator = reordered.makeIterator()
        return items.map { item in
            guard case let .thread(_, region) = item, region == .pinned else { return item }
            return iterator.next() ?? item
        }
    }

    private func pinnedMoveIsDisabled(for item: GaryxHomeThreadListItem) -> Bool {
        guard GaryxPinnedThreadReorderRuntimeGate.isFeatureEnabled else { return true }
        if case let .thread(_, region) = item, region == .pinned { return false }
        return true
    }

    private var pinnedMoveAction: ((IndexSet, Int) -> Void)? {
        guard GaryxPinnedThreadReorderRuntimeGate.isFeatureEnabled else { return nil }
        return handlePinnedMove
    }

    private func pinnedMenuDismissToken(for region: GaryxHomeThreadListRegion) -> Int {
        if region == .pinned, GaryxPinnedThreadReorderRuntimeGate.isFeatureEnabled {
            return threadMenuDismissToken
        }
        return 0
    }

    private func pinnedMenuMovementSuppression(for region: GaryxHomeThreadListRegion) -> Bool {
        return region == .pinned
            && GaryxPinnedThreadReorderRuntimeGate.isFeatureEnabled
    }

    private func configurePinnedDragLifecycle() {
        guard GaryxPinnedThreadReorderRuntimeGate.isFeatureEnabled else { return }
        pinnedDragLifecycle.configure(
            callbacks: .init(
                began: beginPinnedDragSession,
                moved: pinnedDragSessionDidMove,
                accepted: acceptPinnedDragSession,
                cancelled: cancelPinnedDragSession
            )
        )
    }

    private func beginPinnedDragSession() {
        #if DEBUG
        if GaryxPinnedThreadReorderRuntimeGate.isArchitectureSpikeEnabled {
            beginArchitectureSpikePinnedDragSession()
            return
        }
        #endif
        onBeginPinnedOrderDrag()
    }

    private func pinnedDragSessionDidMove() {
        // The stationary menu recognizer and native reorder lift are armed
        // together. Once movement establishes drag ownership, invalidate any
        // menu that managed to present during the hold.
        threadMenuDismissToken &+= 1
    }

    private func handlePinnedMove(sourceOffsets: IndexSet, destination: Int) {
        let items = pinnedReorderItems(
            GaryxHomeThreadListLayout.primaryItems(for: homeListStore.presentationSnapshot)
        )
        guard let move = GaryxPinnedListMoveTranslator.translate(
            items: items,
            sourceOffsets: sourceOffsets,
            destination: destination
        ) else { return }
        pinnedDragLifecycle.notePreviewMove(move.order)
        #if DEBUG
        if GaryxPinnedThreadReorderRuntimeGate.isArchitectureSpikeEnabled {
            dragPreviewOrder = move.order
            return
        }
        #endif
        onPreviewPinnedOrderDrag(move.order)
    }

    private func acceptPinnedDragSession(previewOrder: [String]) {
        #if DEBUG
        if GaryxPinnedThreadReorderRuntimeGate.isArchitectureSpikeEnabled {
            spikeCommittedOrder = previewOrder
            dragPreviewOrder = nil
            dragBaselineOrder = []
            spikeCommitCount += 1
            completedDropHapticTrigger &+= 1
            return
        }
        #endif
        // Fold the controller's terminal preview before accepting in case the
        // final SwiftUI onMove arrived on the deferred classification turn.
        onPreviewPinnedOrderDrag(previewOrder)
        onAcceptPinnedOrderDrop()
        completedDropHapticTrigger &+= 1
    }

    private func cancelPinnedDragSession() {
        #if DEBUG
        if GaryxPinnedThreadReorderRuntimeGate.isArchitectureSpikeEnabled {
            dragPreviewOrder = nil
            dragBaselineOrder = []
            return
        }
        #endif
        onCancelPinnedOrderDrag()
    }

    @ViewBuilder
    private var pinnedDragLifecycleAdapter: some View {
        if GaryxPinnedThreadReorderRuntimeGate.isFeatureEnabled {
            GaryxPinnedDragLifecycleAdapter(controller: pinnedDragLifecycle)
                .allowsHitTesting(false)
        }
    }

    #if DEBUG
    private func beginArchitectureSpikePinnedDragSession() {
        let order = renderedPinnedOrder
        dragBaselineOrder = order
        dragPreviewOrder = order
        midLiftSnapshotStayedFrozen = false

        guard ProcessInfo.processInfo.environment["GARYX_MOBILE_PIN_REORDER_INJECT_MIDLIFT"] == "1"
        else { return }
        debugInjectedServerOrder = Array(order.reversed())
        DispatchQueue.main.async {
            midLiftSnapshotStayedFrozen = renderedPinnedOrder == order
        }
    }

    private var renderedPinnedOrder: [String] {
        pinnedReorderItems(
            GaryxHomeThreadListLayout.primaryItems(for: homeListStore.presentationSnapshot)
        ).compactMap { item in
            guard case let .thread(row, region) = item, region == .pinned else { return nil }
            return row.id
        }
    }

    @ViewBuilder
    private var debugPinnedReorderControls: some View {
        if GaryxPinnedThreadReorderRuntimeGate.isArchitectureSpikeEnabled {
            VStack(spacing: 0) {
                Button {
                    dragBaselineOrder = []
                    dragPreviewOrder = nil
                    spikeCommittedOrder = nil
                    debugInjectedServerOrder = nil
                    spikeCommitCount = 0
                    spikeRemoteMutationCount = 0
                    midLiftSnapshotStayedFrozen = false
                } label: {
                    Color.primary.opacity(0.02)
                        .frame(width: 44, height: 36)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Reset pinned reorder spike")
                .accessibilityIdentifier("pinned-reorder-debug-reset")

                Button {
                    guard let first = homeListStore.presentationSnapshot.sections.pinned.first else { return }
                    _ = homeListStore.beginPinTransition(
                        threadId: first.id,
                        pinned: false,
                        originalPinned: true,
                        recentIndex: 0
                    )
                    debugPinMoveCount += 1
                } label: {
                    Color.primary.opacity(0.02)
                        .frame(width: 44, height: 36)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Inject pin move")
                .accessibilityIdentifier("pinned-reorder-debug-pin-move")

                Text("Pinned reorder lifecycle")
                    .accessibilityIdentifier("pinned-reorder-lifecycle")
                    .accessibilityValue(pinnedDragLifecycle.debugReport)
                    .frame(width: 1, height: 1)
                    .clipped()

                Text("Pinned reorder result")
                    .accessibilityIdentifier("pinned-reorder-result")
                    .accessibilityValue(
                        "commits=\(spikeCommitCount) remote_mutations=\(spikeRemoteMutationCount) midlift_frozen=\(midLiftSnapshotStayedFrozen ? 1 : 0) pin_moves=\(debugPinMoveCount) order=\(renderedPinnedOrder.joined(separator: ","))"
                    )
                    .frame(width: 1, height: 1)
                    .clipped()

                Text("Pinned reorder recognizers")
                    .accessibilityIdentifier("pinned-reorder-recognizers")
                    .accessibilityValue(pinnedDragLifecycle.observedRecognizerNames)
                    .frame(width: 1, height: 1)
                    .clipped()
            }
            .font(.system(size: 1))
            .padding(.top, 88)
            .padding(.trailing, 2)
        }
    }
    #endif

    #if DEBUG
    @ViewBuilder
    private var debugPerformanceProbeControls: some View {
        if ProcessInfo.processInfo.environment["GARYX_MOBILE_HOME_SCROLL_PROBE_MANUAL"] == "1" {
            VStack(spacing: 0) {
                Button {
                    performanceProbe.beginWindow(label: "home_scroll_ui_test")
                } label: {
                    Color.primary.opacity(0.02)
                        .frame(width: 44, height: 36)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Begin home scroll probe")
                .accessibilityIdentifier("home-scroll-probe-begin")

                Button {
                    _ = performanceProbe.endWindow()
                } label: {
                    Color.primary.opacity(0.02)
                        .frame(width: 44, height: 36)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("End home scroll probe")
                .accessibilityIdentifier("home-scroll-probe-end")

                Text("Home scroll probe state")
                    .accessibilityIdentifier("home-scroll-probe-state")
                    .accessibilityValue(performanceProbe.isRecording ? "recording" : "idle")
                    .frame(width: 1, height: 1)
                    .clipped()

                if let report = performanceProbe.latestReport {
                    Text("Home scroll probe report")
                        .accessibilityIdentifier("home-scroll-probe-report")
                        .accessibilityValue(report.machineReadableLine)
                        .frame(width: 1, height: 1)
                        .clipped()
                }
            }
            .padding(.leading, 2)
            .padding(.bottom, 92)
        }
    }
    #endif

}

struct GaryxHomeHeaderView: View {
    let selectedRecentFilter: GaryxRecentThreadFilter
    let onOpenDrawer: () -> Void
    let onSelectRecentFilter: (GaryxRecentThreadFilter) -> Void

    var body: some View {
        GaryxAdaptiveGlassContainer(spacing: 10) {
            HStack(alignment: .center, spacing: 12) {
                GaryxSidebarMenuButton(action: onOpenDrawer)

                Text("Garyx")
                    .font(GaryxFont.system(size: 26, weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.75)

                Spacer(minLength: 0)

                GaryxRecentThreadFilterMenu(
                    selection: selectedRecentFilter,
                    onSelect: onSelectRecentFilter
                )
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
        // Match the page color while giving Reduce Transparency an opaque
        // surface behind the title and the two glass controls.
        .background(GaryxTheme.header)
    }
}

/// Navigation drawer over the home thread list: module entries with Bots and
/// Workspaces expanding inline like the Mac sidebar rail, and Settings plus
/// the gateway identity bar at the bottom.
struct GaryxNavigationDrawerView: View {
    @ObservedObject var drawerStore: GaryxNavigationDrawerStore
    @Environment(\.garyxSidebarDragActive) private var sidebarDragActive
    let onOpenPanel: (GaryxMobilePanel) -> Void
    let onOpenBotGroup: (GaryxMobileBotGroup) -> Void
    let onOpenBotDrilldown: (String) -> Void
    let onOpenWorkspaceDrilldown: (String) -> Void
    let onOpenSettings: () -> Void
    let onSwitchGateway: (GaryxGatewaySwitcherRow) -> Void
    let onManageGateways: () -> Void
    @Binding var debugShowsGatewaySwitcher: Bool

    var body: some View {
        let snapshot = drawerStore.snapshot
        VStack(alignment: .leading, spacing: 0) {
            GaryxAdaptiveGlassContainer(spacing: 10) {
                HStack(alignment: .center, spacing: 12) {
                    // The gateway identity IS the drawer title.
                    GaryxSidebarGatewayIdentityControl(
                        identity: snapshot.gatewayIdentity,
                        rows: snapshot.gatewayRows,
                        onSwitch: onSwitchGateway,
                        onManageGateways: onManageGateways,
                        debugShowsGatewaySwitcher: $debugShowsGatewaySwitcher
                    )

                    Spacer(minLength: 0)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)

            ScrollView(.vertical, showsIndicators: false) {
                VStack(alignment: .leading, spacing: 6) {
                    GaryxSidebarNavigationRow(
                        panel: .automations,
                        isSelected: snapshot.activePanel == .automations,
                        action: { onOpenPanel(.automations) }
                    )

                    GaryxSidebarNavigationRow(
                        panel: .capsules,
                        isSelected: snapshot.activePanel == .capsules,
                        action: { onOpenPanel(.capsules) }
                    )

                    GaryxSidebarNavigationRow(
                        panel: .agents,
                        isSelected: snapshot.activePanel == .agents,
                        action: { onOpenPanel(.agents) }
                    )

                    if !snapshot.botGroups.isEmpty {
                        GaryxDrawerSectionLabel(title: GaryxMobilePanel.bots.label)
                        ForEach(snapshot.botGroups) { group in
                            GaryxDrawerChildRow(title: group.title) {
                                GaryxChannelLogoView(
                                    channel: group.channel,
                                    label: group.title,
                                    iconDataUrl: group.iconDataUrl,
                                    diameter: 22
                                )
                            } action: {
                                openBotGroup(group)
                            }
                        }
                    }

                    if !snapshot.workspaceRows.isEmpty {
                        GaryxDrawerSectionLabel(title: GaryxMobilePanel.workspaceBots.label)
                        ForEach(snapshot.workspaceRows) { row in
                            GaryxDrawerChildRow(title: row.name) {
                                Image(systemName: "folder")
                                    .font(GaryxFont.system(size: 15, weight: .semibold))
                                    .foregroundStyle(.secondary)
                                    .frame(width: 22, height: 22)
                            } action: {
                                onOpenWorkspaceDrilldown(row.path)
                            }
                        }
                    }

                }
                .padding(.horizontal, GaryxSidebarMetrics.outerHorizontalPadding)
                .padding(.top, 6)
                .padding(.bottom, 12)
            }
            .scrollDisabled(sidebarDragActive)

            Spacer(minLength: 0)

            // Settings keeps the floating glass pill treatment at the drawer
            // bottom.
            GaryxAdaptiveGlassContainer(spacing: 10) {
                HStack(spacing: 0) {
                    GaryxSidebarActionPill(
                        title: "Settings",
                        iconSystemName: "gearshape",
                        style: .glass,
                        action: onOpenSettings
                    )

                    Spacer(minLength: 0)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 6)
            .padding(.bottom, 10)
        }
        .frame(maxHeight: .infinity, alignment: .top)
        .garyxPageBackground()
    }

    private func openBotGroup(_ group: GaryxMobileBotGroup) {
        if group.rootCanOpen {
            onOpenBotGroup(group)
        } else {
            onOpenBotDrilldown(group.id)
        }
    }
}

/// Non-interactive caption above a flat drawer group, matching the home
/// list's Pinned/Recent section labels.
private struct GaryxDrawerSectionLabel: View {
    let title: String

    var body: some View {
        Text(title)
            .font(GaryxFont.caption(weight: .medium))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .padding(.horizontal, GaryxSidebarMetrics.rowInnerHorizontalPadding)
            .padding(.top, 14)
            .padding(.bottom, 2)
            .accessibilityAddTraits(.isHeader)
    }
}

/// Flat drawer row for a bot account or workspace folder.
private struct GaryxDrawerChildRow<Icon: View>: View {
    let title: String
    @ViewBuilder var icon: () -> Icon
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                icon()
                    .frame(width: 26, height: 26)

                Text(title)
                    .font(GaryxFont.callout())
                    .foregroundStyle(Color.primary.opacity(0.88))
                    .lineLimit(1)
                    .truncationMode(.middle)

                Spacer(minLength: 0)
            }
            .padding(.horizontal, GaryxSidebarMetrics.rowInnerHorizontalPadding)
            .frame(minHeight: 40)
            .contentShape(RoundedRectangle(cornerRadius: GaryxSidebarMetrics.rowCornerRadius, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
    }
}

struct GaryxSidebarNavigationRow: View {
    let panel: GaryxMobilePanel
    let isSelected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                GaryxPanelIconView(systemName: panel.iconName, size: 19)
                    .foregroundStyle(iconColor)
                    .frame(width: 26, height: 26)

                Text(panel.label)
                    .font(GaryxFont.callout(weight: .medium))
                    .foregroundStyle(textColor)
                    .lineLimit(1)

                Spacer(minLength: 0)
            }
            .padding(.horizontal, GaryxSidebarMetrics.rowInnerHorizontalPadding)
            .frame(minHeight: 44)
            .background(
                isSelected ? Color(.tertiarySystemFill).opacity(0.58) : Color.clear,
                in: RoundedRectangle(cornerRadius: GaryxSidebarMetrics.rowCornerRadius, style: .continuous)
            )
            .contentShape(RoundedRectangle(cornerRadius: GaryxSidebarMetrics.rowCornerRadius, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(panel.label)
    }

    private var iconColor: Color {
        isSelected ? .primary : Color.primary.opacity(0.78)
    }

    private var textColor: Color {
        isSelected ? .primary : Color.primary.opacity(0.88)
    }
}

struct GaryxSidebarWorkspaceThreadGroup: Identifiable {
    let path: String
    let name: String

    var id: String { path }
}

extension GaryxMobileModel {
    var sidebarWorkspaceThreadGroups: [GaryxSidebarWorkspaceThreadGroup] {
        let paths = userWorkspacePaths
        let duplicateNames = Dictionary(grouping: paths, by: { $0.garyxLastPathComponent })
            .filter { !$0.key.isEmpty && $0.value.count > 1 }
        return paths
            .map { path in
                let name = path.garyxLastPathComponent.isEmpty ? path : path.garyxLastPathComponent
                return GaryxSidebarWorkspaceThreadGroup(
                    path: path,
                    name: duplicateNames[name] == nil ? name : path.garyxDisambiguatedWorkspaceName
                )
            }
    }
}

/// Hairline between thread rows, inset to the text column so it reads like a
/// native chat list separator.
struct GaryxSidebarRowDivider: View {
    var body: some View {
        Rectangle()
            .fill(Color.primary.opacity(0.06))
            .frame(height: 1.0 / UIScreen.main.scale)
            // Outer row padding (rowOuterPadding - 4) + inner padding + avatar
            // diameter + avatar-text gap.
            .padding(.leading, (GaryxSidebarMetrics.rowOuterPadding - 4) + GaryxSidebarMetrics.rowInnerHorizontalPadding + 38 + 10)
            .padding(.trailing, GaryxSidebarMetrics.rowOuterPadding)
            .accessibilityHidden(true)
    }
}

private struct GaryxSidebarSkeletonRows: View {
    @Environment(\.garyxMotion) private var motion
    let rowCount: Int

    var body: some View {
        let count = max(0, rowCount)
        TimelineView(
            .animation(
                minimumInterval: GaryxMotion.timelineMinimumInterval,
                paused: motion.pausesContinuousMotion(.loadingShimmer)
            )
        ) { context in
            let shimmerDuration = motion.cycleDuration(.loadingShimmer)
            let normalized = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: shimmerDuration) / shimmerDuration
            let phase = CGFloat(normalized) * 2.0 - 0.5
            let fill = LinearGradient(
                colors: [
                    Color.primary.opacity(0.045),
                    Color.primary.opacity(0.105),
                    Color.primary.opacity(0.045),
                ],
                startPoint: UnitPoint(x: phase - 0.6, y: 0.35),
                endPoint: UnitPoint(x: phase + 0.6, y: 0.65)
            )

            VStack(spacing: 0) {
                ForEach(0..<count, id: \.self) { index in
                    GaryxSidebarSkeletonRow(index: index, fill: fill)
                    if index < count - 1 {
                        GaryxSidebarRowDivider()
                    }
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Loading recent threads")
    }
}

private struct GaryxSidebarSkeletonRow: View {
    let index: Int
    let fill: LinearGradient

    private var titleWidth: CGFloat {
        [154, 118, 142, 166, 132, 150][index % 6]
    }

    private var subtitleWidth: CGFloat {
        [202, 172, 190, 154, 214, 178][index % 6]
    }

    private var timestampWidth: CGFloat {
        [36, 44, 30, 50, 40, 34][index % 6]
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Circle()
                .fill(fill)
                .frame(width: 38, height: 38)

            VStack(alignment: .leading, spacing: 7) {
                RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .fill(fill)
                    .frame(width: titleWidth, height: 12)
                RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .fill(fill)
                    .frame(width: subtitleWidth, height: 10)
                RoundedRectangle(cornerRadius: 5, style: .continuous)
                    .fill(fill)
                    .frame(width: timestampWidth, height: 8)
            }
            .padding(.top, 2)

            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(height: 60)
        .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
        .accessibilityHidden(true)
    }
}

private struct GaryxSidebarEmptyRow: View {
    let title: String

    var body: some View {
        Text(title)
            .font(GaryxFont.caption(weight: .medium))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .frame(minHeight: 44)
            .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
    }
}

/// Auto-load footer rendered from the pager's projected state. The row
/// keeps a constant 44pt height across idle/loading/failed so page
/// completions never shift bottom content; only exhaustion removes it.
private struct GaryxSidebarThreadAutoLoadFooter: View {
    @Environment(GaryxHomeObservationStore.self) private var homeObservationStore
    @Environment(\.garyxLoadMoreThreads) private var loadMoreThreads
    @Environment(\.garyxRetryLoadMoreThreads) private var retryLoadMoreThreads

    var body: some View {
        let state = homeObservationStore.loadMoreFooterState
        if state != .hidden {
            ZStack {
                switch state {
                case .idle:
                    // Fallback trigger for short lists and fast flings; the
                    // near-tail row prefetch usually fires first. Rejected
                    // triggers cost nothing (pager-gated), and the row
                    // re-arms on every state change because the branch
                    // identity is keyed by `state`.
                    Color.clear
                        .onAppear {
                            Task { await loadMoreThreads(.footer) }
                        }
                case .loading:
                    HStack(spacing: 8) {
                        ProgressView()
                            .scaleEffect(0.68)
                        Text("Loading more")
                            .font(GaryxFont.caption(weight: .medium))
                    }
                    .foregroundStyle(.tertiary)
                case .failed:
                    Button {
                        Task { await retryLoadMoreThreads() }
                    } label: {
                        HStack(spacing: 6) {
                            Image(systemName: "arrow.clockwise")
                                .font(.system(size: 11, weight: .semibold))
                            Text("Couldn't load more · Tap to retry")
                                .font(GaryxFont.caption(weight: .medium))
                        }
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, minHeight: 44)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                case .hidden:
                    EmptyView()
                }
            }
            .frame(maxWidth: .infinity)
            .frame(height: 44)
            .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
            .padding(.bottom, 10)
        }
    }
}

struct GaryxSidebarSectionHeader: View {
    let title: String
    let systemImage: String
    var statusLabel: String? = nil

    var body: some View {
        HStack(spacing: 0) {
            Text(title)
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            if let statusLabel {
                Text(" · ")
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                Text(statusLabel)
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .layoutPriority(1)
                    .accessibilityIdentifier("\(title.lowercased())-section-status")
            }
        }
        .accessibilityElement(children: .combine)
    }
}

enum GaryxSidebarThreadRowDensity {
    case regular
    case compact

    var minHeight: CGFloat {
        switch self {
        case .regular:
            GaryxSidebarMetrics.threadRowMinHeight
        case .compact:
            38
        }
    }

    var titleWeight: Font.Weight {
        switch self {
        case .regular:
            .semibold
        case .compact:
            .regular
        }
    }

    var titleFont: Font {
        switch self {
        case .regular:
            GaryxFont.subheadline(weight: titleWeight)
        case .compact:
            GaryxFont.footnote(weight: titleWeight)
        }
    }

    var subtitleFont: Font {
        switch self {
        case .regular:
            GaryxFont.footnote()
        case .compact:
            GaryxFont.caption()
        }
    }

    var textSpacing: CGFloat {
        switch self {
        case .regular:
            3
        case .compact:
            2
        }
    }

    func verticalPadding(isFullBleed: Bool) -> CGFloat {
        switch self {
        case .regular:
            isFullBleed ? 10 : 8
        case .compact:
            4
        }
    }
}

enum GaryxSidebarThreadSelectionDisplay: Equatable {
    case sidebar
    case checkmark
    case none
}

/// Trailing relative timestamp that refreshes once a minute so the label
/// ("3m"/"2h") never freezes when its row stays equatable across true List cell
/// reuse. Only this leaf re-renders on the tick — the row body does not.
struct GaryxRelativeTimestampText: View {
    let timestampValue: String?

    var body: some View {
        TimelineView(.everyMinute) { context in
            Text(garyxFormattedTaskTimestamp(timestampValue, now: context.date))
                .font(GaryxFont.caption())
                .foregroundStyle(.tertiary)
                .lineLimit(1)
        }
    }
}

struct GaryxSidebarThreadRowView: View {
    let presentation: GaryxSidebarThreadRowPresentation
    var avatar: GaryxSidebarThreadRowAvatar?
    var isFullBleed = false
    var density: GaryxSidebarThreadRowDensity = .regular
    var selectionDisplay: GaryxSidebarThreadSelectionDisplay = .sidebar
    /// When set (home rows), render a self-refreshing relative timestamp from
    /// this raw ISO value instead of `presentation.trailingTimestamp`.
    var liveTimestampValue: String?
    /// Home rows let the long-press menu own one exclusive tap/long-press
    /// recognizer. Other sidebar rows keep their direct tap gesture.
    var usesExternalSelectionGesture = false
    var onSelect: (() -> Void)?
    var onUnpin: (() -> Void)?
    // `.onTapGesture` ignores `.disabled`, so honor the environment manually:
    // the drawer disables the sidebar while a horizontal drag is in flight to
    // keep mid-drag finger-ups from opening rows.
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            if let avatar {
                GaryxAgentAvatarView(
                    agentId: avatar.agentId,
                    avatarDataUrl: avatar.avatarDataUrl,
                    label: avatar.label,
                    providerType: avatar.providerType,
                    builtIn: avatar.builtIn,
                    diameter: 38
                )
                .overlay(alignment: .bottomTrailing) {
                    if presentation.isRunning {
                        GaryxAvatarTypingBadge()
                            .offset(x: 3, y: 3)
                    }
                }
            }

            VStack(alignment: .leading, spacing: density.textSpacing) {
                HStack(alignment: .center, spacing: 5) {
                    Text(presentation.title)
                        .font(density.titleFont)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(.primary)
                        .layoutPriority(1)

                    if presentation.isPinned {
                        if let onUnpin {
                            Button(action: onUnpin) {
                                Image(systemName: "pin.fill")
                                    .font(GaryxFont.system(size: 11, weight: .semibold))
                                    .foregroundStyle(.tertiary)
                                    .rotationEffect(.degrees(28))
                                    .frame(width: 20, height: 20)
                            }
                            .frame(width: 26, height: 22)
                            .contentShape(Rectangle())
                            .buttonStyle(.plain)
                            .accessibilityLabel("Unpin thread")
                        } else {
                            Image(systemName: "pin.fill")
                                .font(GaryxFont.system(size: 11, weight: .semibold))
                                .foregroundStyle(.tertiary)
                                .rotationEffect(.degrees(28))
                                .frame(width: 20, height: 20)
                                .accessibilityHidden(true)
                        }
                    }

                    Spacer(minLength: 8)

                    trailingMeta
                        .fixedSize(horizontal: true, vertical: false)
                }

                if let subtitle = presentation.subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(density.subtitleFont)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .contentShape(Rectangle())
        .onTapGesture {
            guard isEnabled, !usesExternalSelectionGesture else { return }
            onSelect?()
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(rowAccessibilityLabel)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction {
            onSelect?()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(minHeight: density.minHeight, alignment: .leading)
        .padding(.horizontal, isFullBleed ? GaryxSidebarMetrics.sectionHorizontalPadding : GaryxSidebarMetrics.rowInnerHorizontalPadding)
        .padding(.vertical, density.verticalPadding(isFullBleed: isFullBleed))
        .padding(.horizontal, isFullBleed ? 0 : GaryxSidebarMetrics.rowOuterPadding - 4)
    }
}

/// Running-state badge pinned to the avatar's bottom-right corner: a small
/// tinted bubble with an iMessage-style three-dot typing wave, ringed by the
/// page background so it sits cleanly on any avatar.
struct GaryxAvatarTypingBadge: View {
    @Environment(\.garyxMotion) private var motion
    var isPaused = false
    var scale: CGFloat = 1

    var body: some View {
        Group {
            if isPaused || motion.pausesContinuousMotion(.runningTyping) {
                badge(activeDot: -1)
            } else {
                // One looping PhaseAnimator drives the three-dot wave on the render
                // server: per-frame main-thread cost is zero (Core Animation
                // interpolates opacity), the phase advances only ~3x/sec, and with
                // no retained @State the loop restarts cleanly whenever a recycled
                // List cell reappears. Cost does not scale with visible running rows.
                PhaseAnimator([0, 1, 2]) { activeDot in
                    badge(activeDot: activeDot)
                } animation: { _ in
                    motion.continuousAnimation(.runningTyping)
                }
            }
        }
        .accessibilityLabel("Running")
    }

    private func badge(activeDot: Int) -> some View {
        HStack(spacing: 2.2 * scale) {
            ForEach(0..<3, id: \.self) { index in
                Circle()
                    .fill(Color(.systemGray))
                    .frame(width: 3.2 * scale, height: 3.2 * scale)
                    .opacity(index == activeDot ? 1.0 : 0.4)
            }
        }
        .frame(width: 22 * scale, height: 15 * scale)
        .background(Color(.systemGray5), in: Capsule())
        .overlay {
            Capsule()
                .stroke(GaryxTheme.background, lineWidth: max(1, 2 * scale))
        }
    }
}

private struct GaryxSidebarRunningIndicator: View {
    @Environment(\.garyxMotion) private var motion

    var body: some View {
        TimelineView(
            .animation(
                minimumInterval: GaryxMotion.timelineMinimumInterval,
                paused: motion.pausesContinuousMotion(.runningOrbit)
            )
        ) { context in
            let cycle = motion.cycleDuration(.runningOrbit)
            let progress = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: cycle) / cycle

            ZStack {
                Circle()
                    .stroke(Color.primary.opacity(0.08), lineWidth: 1)

                Circle()
                    .trim(from: 0.08, to: 0.78)
                    .stroke(
                        AngularGradient(
                            colors: [
                                Color.primary.opacity(0.05),
                                Color.primary.opacity(0.22),
                                Color.primary.opacity(0.62),
                                Color.primary.opacity(0.18),
                                Color.primary.opacity(0.04)
                            ],
                            center: .center
                        ),
                        style: StrokeStyle(lineWidth: 1.7, lineCap: .round)
                    )
                    .rotationEffect(.degrees(progress * 360))

                Circle()
                    .trim(from: 0.0, to: 0.18)
                    .stroke(
                        Color.primary.opacity(0.24),
                        style: StrokeStyle(lineWidth: 1.1, lineCap: .round)
                    )
                    .rotationEffect(.degrees(progress * -520 + 90))
                    .blur(radius: 0.15)
            }
        }
        .frame(width: 18, height: 18)
        .accessibilityLabel("Running")
    }
}

private extension GaryxSidebarThreadRowView {
    var rowAccessibilityLabel: String {
        [
            presentation.title,
            presentation.subtitle,
            presentation.isPinned ? "Pinned" : nil,
            presentation.isRunning ? "Running" : nil,
        ]
        .compactMap { value in
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return trimmed.isEmpty ? nil : trimmed
        }
        .joined(separator: ", ")
    }

    var trailingMeta: some View {
        HStack(spacing: 6) {
            if presentation.isRunning, avatar == nil {
                GaryxSidebarRunningIndicator()
            } else if presentation.isSelected, selectionDisplay == .checkmark {
                GaryxSelectionCheckmark(size: 13)
            } else if let liveTimestampValue, !liveTimestampValue.isEmpty {
                // Home rows: self-refreshing relative timestamp (never freezes
                // under equatable rows + true List cell reuse).
                GaryxRelativeTimestampText(timestampValue: liveTimestampValue)
            } else if let trailingTimestamp = presentation.trailingTimestamp, !trailingTimestamp.isEmpty {
                // The selected row already reads through its background fill;
                // no extra trailing marker.
                Text(trailingTimestamp)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
        }
    }
}

struct GaryxSidebarActionPill: View {
    enum Style {
        case glass
        case accent
    }

    let title: String
    let iconSystemName: String
    let style: Style
    var isEnabled = true
    var isLoading = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                if isLoading {
                    ProgressView()
                        .tint(foreground)
                        .scaleEffect(0.8)
                } else {
                    Image(systemName: iconSystemName)
                        .font(GaryxFont.system(size: 13, weight: .semibold))
                }

                Text(title)
                    .font(GaryxFont.subheadline(weight: .semibold))
            }
            .foregroundStyle(foreground)
            .padding(.horizontal, 16)
            .frame(minHeight: 44)
            .background(background, in: Capsule())
            .if(style == .glass) { view in
                view.garyxAdaptiveGlass(.regular, isInteractive: true, in: Capsule())
            }
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .opacity(isEnabled ? 1 : 0.45)
    }

    private var foreground: Color {
        switch style {
        case .glass:
            .primary
        case .accent:
            Color(.systemBackground)
        }
    }

    private var background: Color {
        switch style {
        case .glass:
            Color.clear
        case .accent:
            Color(.label)
        }
    }
}

struct GaryxWorkspaceBotsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxMotion) private var motion
    let drilldown: GaryxWorkspaceBotsDrilldown?
    @State private var showsAddWorkspace = false
    @State private var addWorkspacePath = ""

    init(drilldown: GaryxWorkspaceBotsDrilldown? = nil) {
        self.drilldown = drilldown
    }

    @ViewBuilder
    var body: some View {
        Group {
            switch drilldown {
            case .workspace(let path):
                GaryxWorkspaceThreadListDrilldown(
                    model: model,
                    path: path,
                    store: model.workspaceThreadListStore(path: path)
                )
            case .bot(let id):
                if let group = model.mobileBotGroups.first(where: { $0.id == id }) {
                    GaryxBotThreadListDrilldown(
                        model: model,
                        group: group,
                        store: model.botThreadListStore(group: group)
                    )
                } else {
                    missingDrilldown(
                        title: "Bot Not Found",
                        message: "Garyx could not find bot \(id)."
                    )
                }
            case .automationThreads(let id):
                if let automation = generatedAutomations.first(where: { $0.id == id }) {
                    GaryxAutomationThreadListDrilldown(
                        model: model,
                        automation: automation,
                        store: model.automationThreadListStore(automationId: id)
                    )
                } else {
                    missingDrilldown(
                        title: "Automation Not Found",
                        message: "Garyx could not find automation \(id)."
                    )
                }
            case nil:
                rootWorkspacePanel
            }
        }
    }

    private var rootWorkspacePanel: some View {
        GaryxPanelScaffold(
            title: "Workspaces",
            subtitle: "",
            onRefresh: { await model.refreshRemoteState() },
            leadingActionLabel: nil,
            leadingAction: nil,
            contentHorizontalPadding: 0
        ) {
            GaryxWorkspaceRootSection(groups: model.sidebarWorkspaceThreadGroups) { path in
                withAnimation(motion.spatialAnimation(.drilldown)) {
                    model.openWorkspaceBotsDrilldown(.workspace(path), source: .current)
                }
            }
        } actions: {
            if drilldown == nil {
                Button {
                    addWorkspacePath = ""
                    showsAddWorkspace = true
                } label: {
                    GaryxToolbarIcon(systemName: "plus")
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Add Workspace")
            }
        }
        .task {
            await model.refreshRemoteState()
        }
        .garyxSheet(isPresented: $showsAddWorkspace) {
            GaryxWorkspacePathPickerSheet(
                title: "Add Workspace",
                path: $addWorkspacePath
            )
        }
        .onChange(of: addWorkspacePath) { _, newValue in
            let path = newValue.trimmingCharacters(in: .whitespacesAndNewlines)
            guard garyxIsAbsoluteWorkspacePath(path) else { return }
            Task { await addWorkspace(path) }
        }
    }

    private var generatedAutomations: [GaryxAutomationSummary] {
        model.automations.filter(\.isGeneratedThreadMode)
    }

    private func addWorkspace(_ path: String) async {
        guard let addedPath = await model.addUserWorkspacePath(path) else { return }
        await model.selectWorkspace(addedPath)
        model.openWorkspaceBotsDrilldown(.workspace(addedPath), source: .current)
    }

    private func missingDrilldown(title: String, message: String) -> some View {
        GaryxListPanelScaffold(
            title: title
        ) {
            GaryxEmptyPanelView(icon: "magnifyingglass", title: title, text: message)
                .padding(.horizontal, 16)
        }
    }
}
