import Foundation
import SwiftUI

enum GaryxSidebarDragAxis {
    case horizontal
    case vertical
}

/// Root content column: the home thread list with conversation and panel
/// pages pushed above it. Pushes originate from model navigation state; the
/// path binding only ever receives pops (system back swipe or back buttons).
struct GaryxRootNavigationView: View, Equatable {
    @ObservedObject var navigationStore: GaryxRootNavigationPathStore
    @ObservedObject var routeNotFoundStore: GaryxRouteNotFoundStore
    @ObservedObject var homeListStore: GaryxHomeThreadListStore
    let isSidebarDragActive: Bool
    let onOpenDrawer: () -> Void
    let applyRootNavigationPath: ([GaryxMobileRootRoute]) -> Void
    let onRefreshAll: () async -> Void
    let onRefreshSidebarThreads: () async -> Void
    let onLoadMoreThreads: (GaryxThreadListLoadMoreTrigger) async -> Void
    let onRetryLoadMoreThreads: () async -> Void
    let onStartNewChat: () -> Void
    let onOpenThread: (GaryxThreadSummary) -> Void
    let onTogglePinnedThread: (String) -> Void
    let onUnpinThread: (String) -> Void
    let onArchiveThread: (GaryxThreadSummary) async -> Void

    static func == (lhs: GaryxRootNavigationView, rhs: GaryxRootNavigationView) -> Bool {
        lhs.navigationStore === rhs.navigationStore
            && lhs.routeNotFoundStore === rhs.routeNotFoundStore
            && lhs.homeListStore === rhs.homeListStore
            && lhs.isSidebarDragActive == rhs.isSidebarDragActive
    }

    var body: some View {
        #if DEBUG
        let _ = GaryxHomeScrollPerformanceProbe.shared.markRootBody()
        #endif
        NavigationStack(path: rootPathBinding) {
            GaryxHomeThreadListView(
                homeListStore: homeListStore,
                isSidebarDragActive: isSidebarDragActive,
                onOpenDrawer: onOpenDrawer,
                onRefreshAll: onRefreshAll,
                onRefreshSidebarThreads: onRefreshSidebarThreads,
                onLoadMoreThreads: onLoadMoreThreads,
                onRetryLoadMoreThreads: onRetryLoadMoreThreads,
                onStartNewChat: onStartNewChat,
                onOpenThread: onOpenThread,
                onTogglePinnedThread: onTogglePinnedThread,
                onUnpinThread: onUnpinThread,
                onArchiveThread: onArchiveThread
            )
                .equatable()
                .toolbar(.hidden, for: .navigationBar)
                .navigationDestination(for: GaryxMobileRootRoute.self) { route in
                    GaryxRootRouteContentView(route: route)
                        .toolbar(.hidden, for: .navigationBar)
                }
        }
        .garyxPageBackground()
        .fullScreenCover(item: $routeNotFoundStore.selection) { state in
            GaryxFormSheet(title: state.title) {
                GaryxRouteNotFoundCard(state: state)
            }
        }
    }

    private var rootPathBinding: Binding<[GaryxMobileRootRoute]> {
        Binding(
            get: { navigationStore.path },
            set: { applyRootNavigationPath($0) }
        )
    }
}

private struct GaryxRootRouteContentView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let route: GaryxMobileRootRoute

    var body: some View {
        switch route {
        case .conversation:
            if model.showsWorkflowRunSurface {
                GaryxWorkflowRunView()
            } else {
                GaryxConversationView()
            }
        case .panel(let panel):
            panelContent(for: panel)
        }
    }

    @ViewBuilder
    private func panelContent(for panel: GaryxMobilePanel) -> some View {
        switch panel {
        case .chat:
            if model.showsWorkflowRunSurface {
                GaryxWorkflowRunView()
            } else {
                GaryxConversationView()
            }
        case .workspaces:
            GaryxWorkspacesView()
        case .automations:
            GaryxAutomationsView()
        case .capsules:
            GaryxCapsulesView()
        case .workspaceBots:
            GaryxWorkspaceBotsView()
        case .agents:
            GaryxAgentsView()
        case .skills:
            GaryxSkillsView()
        case .commands:
            GaryxCommandsView()
        case .mcp:
            GaryxMcpServersView()
        case .bots:
            GaryxWorkspaceBotsView()
        case .settings:
            GaryxMobileSettingsPanel()
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

private enum GaryxSidebarMetrics {
    static let outerHorizontalPadding: CGFloat = 16
    static let sectionHorizontalPadding: CGFloat = 24
    static let rowOuterPadding: CGFloat = 18
    static let rowInnerHorizontalPadding: CGFloat = 7
    static let rowHeight: CGFloat = 52
    static let threadRowMinHeight: CGFloat = 50
    static let rowCornerRadius: CGFloat = 12
    static let selectedThreadCornerRadius: CGFloat = 12
    static let iconFrame: CGFloat = 28
    static let bottomBarClearance: CGFloat = 28
}

struct GaryxHomeThreadListView: View, Equatable {
    @ObservedObject var homeListStore: GaryxHomeThreadListStore
    let isSidebarDragActive: Bool
    let onOpenDrawer: () -> Void
    let onRefreshAll: () async -> Void
    let onRefreshSidebarThreads: () async -> Void
    let onLoadMoreThreads: (GaryxThreadListLoadMoreTrigger) async -> Void
    let onRetryLoadMoreThreads: () async -> Void
    let onStartNewChat: () -> Void
    let onOpenThread: (GaryxThreadSummary) -> Void
    let onTogglePinnedThread: (String) -> Void
    let onUnpinThread: (String) -> Void
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
            .garyxAdaptiveTopBar {
                GaryxHomeHeaderView(
                    onOpenDrawer: onOpenDrawer,
                    onNewChat: { onStartNewChat() }
                )
            }
            .task(id: homeListStore.snapshot.isHomeVisible) {
                await runSilentSidebarRefreshLoop()
            }
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
            spacerRow(height: GaryxSidebarMetrics.bottomBarClearance)
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
        let snapshot = homeListStore.snapshot
        let sections = snapshot.sections
        if !sections.pinned.isEmpty {
            GaryxSidebarSectionHeader(title: "Pinned", systemImage: "pin.fill")
                .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                .padding(.bottom, 4)

            ForEach(sections.pinned) { row in
                GaryxHomeThreadButton(
                    row: row,
                    onOpenThread: onOpenThread,
                    onTogglePinnedThread: onTogglePinnedThread,
                    onUnpinThread: onUnpinThread,
                    onArchiveThread: onArchiveThread
                )
                .equatable()
            }

            spacerRow(height: 10)
        }

        GaryxSidebarSectionHeader(title: "Recent", systemImage: "clock.fill")
            .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
            .padding(.bottom, 4)

        switch snapshot.recentPlaceholder {
        case .loadingSkeleton(let rowCount):
            GaryxSidebarSkeletonRows(rowCount: rowCount)
        case .empty:
            GaryxSidebarEmptyRow(title: "No recent threads")
        case .none:
            // Near-tail prefetch: the row K places from the end starts the
            // next page before the user reaches the bottom. The trigger is
            // gated by the pager, so repeat appearances are free.
            let prefetchTriggerRowId = GaryxThreadListPageMerge.prefetchTriggerRowId(
                recentIds: sections.recent.map(\.id)
            )
            ForEach(sections.recent) { row in
                GaryxHomeThreadButton(
                    row: row,
                    onOpenThread: onOpenThread,
                    onTogglePinnedThread: onTogglePinnedThread,
                    onUnpinThread: onUnpinThread,
                    onArchiveThread: onArchiveThread
                )
                .equatable()
                .onAppear {
                    if row.id == prefetchTriggerRowId {
                        Task { await onLoadMoreThreads(.nearTail) }
                    }
                }
            }
        }

        spacerRow(height: 10)

        GaryxSidebarThreadAutoLoadFooter()
            .environment(\.garyxLoadMoreThreads, onLoadMoreThreads)
            .environment(\.garyxRetryLoadMoreThreads, onRetryLoadMoreThreads)
    }

    private func refreshAll() async {
        await onRefreshAll()
    }

    private func runSilentSidebarRefreshLoop() async {
        guard shouldRefreshSidebarThreads else { return }
        // Let the drawer-open animation settle before the first refresh so
        // response handling does not contend with the opening transition.
        try? await Task.sleep(nanoseconds: 300_000_000)
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
        homeListStore.snapshot.isHomeVisible
    }

}

private struct GaryxHomeThreadButton: View, Equatable {
    let row: GaryxHomeThreadRow
    let onOpenThread: (GaryxThreadSummary) -> Void
    let onTogglePinnedThread: (String) -> Void
    let onUnpinThread: (String) -> Void
    let onArchiveThread: (GaryxThreadSummary) async -> Void

    static func == (lhs: GaryxHomeThreadButton, rhs: GaryxHomeThreadButton) -> Bool {
        lhs.row == rhs.row
    }

    var body: some View {
        #if DEBUG
        let _ = GaryxHomeScrollPerformanceProbe.shared.markRowBody()
        #endif
        // Divider folded into the row so one thread == one List cell. The
        // timestamp is rendered live (self-refreshing) from the raw value rather
        // than baked here, so the body stays equatable for cell reuse.
        VStack(spacing: 0) {
            if row.showsDivider {
                GaryxSidebarRowDivider()
            }
            GaryxSidebarThreadRowView(
                presentation: row.presentation,
                avatar: row.avatar,
                liveTimestampValue: row.timestampValue,
                onSelect: {
                    onOpenThread(row.thread)
                },
                onUnpin: {
                    onUnpinThread(row.id)
                }
            )
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            if row.canArchive {
                Button(role: .destructive) {
                    Task { await onArchiveThread(row.thread) }
                } label: {
                    Label("Archive thread", systemImage: "archivebox")
                }
            }
            Button {
                onTogglePinnedThread(row.id)
            } label: {
                Label(
                    row.presentation.isPinned ? "Unpin thread" : "Pin thread",
                    systemImage: row.presentation.isPinned ? "pin.slash" : "pin"
                )
            }
            .tint(Color(.systemGray))
        }
    }
}

struct GaryxHomeHeaderView: View {
    let onOpenDrawer: () -> Void
    let onNewChat: () -> Void

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

                Button(action: onNewChat) {
                    // Mirrors the menu button's glass circle treatment.
                    Image(systemName: "plus.bubble")
                        .font(GaryxFont.system(size: 16, weight: .semibold))
                        .foregroundStyle(.primary)
                        .frame(width: 44, height: 44)
                        .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .ultraThinMaterial, in: Circle())
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("New chat")
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
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

private struct GaryxSidebarWorkspaceThreadGroup: Identifiable {
    let path: String
    let name: String
    let threads: [GaryxThreadSummary]

    var id: String { path }
}

private extension GaryxMobileModel {
    var sidebarWorkspaceThreadGroups: [GaryxSidebarWorkspaceThreadGroup] {
        let grouped = Dictionary(grouping: threads) { thread in
            thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        }
        let paths = userWorkspacePaths
        let duplicateNames = Dictionary(grouping: paths, by: { $0.garyxLastPathComponent })
            .filter { !$0.key.isEmpty && $0.value.count > 1 }
        return paths
            .map { path in
                let name = path.garyxLastPathComponent.isEmpty ? path : path.garyxLastPathComponent
                return GaryxSidebarWorkspaceThreadGroup(
                    path: path,
                    name: duplicateNames[name] == nil ? name : path.garyxDisambiguatedWorkspaceName,
                    threads: (grouped[path] ?? []).sorted(by: garyxThreadSort)
                )
            }
    }

    var sidebarVisibleThreadIds: Set<String> {
        Set(threads.map(\.id))
    }
}

private func garyxThreadSort(_ lhs: GaryxThreadSummary, _ rhs: GaryxThreadSummary) -> Bool {
    let left = garyxThreadDate(from: lhs.updatedAt ?? lhs.createdAt ?? "") ?? .distantPast
    let right = garyxThreadDate(from: rhs.updatedAt ?? rhs.createdAt ?? "") ?? .distantPast
    if left != right {
        return left > right
    }
    return lhs.title.localizedCaseInsensitiveCompare(rhs.title) == .orderedAscending
}

/// Hairline between thread rows, inset to the text column so it reads like a
/// native chat list separator.
private struct GaryxSidebarRowDivider: View {
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
    private static let shimmerDuration: Double = 2.4
    let rowCount: Int

    var body: some View {
        let count = max(0, rowCount)
        TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: false)) { context in
            let normalized = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: Self.shimmerDuration) / Self.shimmerDuration
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

private struct GaryxSidebarBotsSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var activeDrilldown: GaryxWorkspaceBotsDrilldown?

    private var groups: [GaryxMobileBotGroup] {
        model.mobileBotGroups
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if case let .bot(id) = activeDrilldown {
                if let selectedGroup {
                    GaryxBotThreadDetailSection(
                        group: selectedGroup
                    )
                } else {
                    GaryxWorkspaceBotsMissingDrilldownState(
                        title: "Bot Not Found",
                        message: "Garyx could not find bot \(id)."
                    )
                }
            } else {
                if !groups.isEmpty {
                    GaryxSidebarSectionHeader(title: "Bots", systemImage: "bubble.left.and.bubble.right")
                        .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                        .padding(.bottom, 4)

                    ForEach(groups) { group in
                        let childEntries = group.sidebarChildConversationEntries()
                        GaryxSidebarBotRow(
                            group: group,
                            canDrillDown: !group.rootCanOpen || childEntries.count > 1,
                            onSelect: {
                                withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                                    activeDrilldown = .bot(group.id)
                                }
                            },
                            onOpenRoot: {
                                Task { await model.openBotGroup(group) }
                            }
                        )
                    }
                }
            }
        }
        .padding(.bottom, 10)
    }

    private var selectedGroup: GaryxMobileBotGroup? {
        guard case let .bot(id) = activeDrilldown else { return nil }
        return groups.first { $0.id == id }
    }
}

private struct GaryxSidebarAutomationsSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var activeDrilldown: GaryxWorkspaceBotsDrilldown?

    private var generatedAutomations: [GaryxAutomationSummary] {
        model.automations
            .filter(\.isGeneratedThreadMode)
            .sorted { lhs, rhs in
                lhs.label.localizedCaseInsensitiveCompare(rhs.label) == .orderedAscending
            }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if case let .automationThreads(id) = activeDrilldown {
                GaryxAutomationThreadsDetailSection(
                    automationId: id,
                    automation: generatedAutomations.first { $0.id == id }
                )
            } else if !generatedAutomations.isEmpty {
                GaryxSidebarSectionHeader(title: "Scheduled automations", systemImage: "clock.arrow.circlepath")
                    .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                    .padding(.bottom, 4)

                ForEach(generatedAutomations) { automation in
                    GaryxDisclosureListRow(
                        title: automation.label,
                        subtitle: automation.workspacePath.isEmpty
                            ? nil
                            : URL(fileURLWithPath: automation.workspacePath).lastPathComponent,
                        systemImage: "clock.arrow.circlepath",
                        iconFrame: GaryxSidebarMetrics.iconFrame,
                        horizontalPadding: GaryxSidebarMetrics.rowInnerHorizontalPadding,
                        verticalPadding: 0,
                        minHeight: GaryxSidebarMetrics.rowHeight,
                        titleWeight: .medium,
                        action: {
                            withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                                activeDrilldown = .automationThreads(automation.id)
                            }
                        }
                    )
                    .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
                }
            }
        }
        .padding(.bottom, 10)
    }
}

private struct GaryxAutomationThreadsDetailSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let automationId: String
    let automation: GaryxAutomationSummary?

    @State private var page: GaryxAutomationThreadsPage?
    @State private var isLoading = false
    @State private var failureMessage: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            GaryxSidebarSectionHeader(title: "Threads", systemImage: "bubble.left.and.text.bubble.right.fill")
                .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                .padding(.bottom, 4)

            if isLoading && page == nil {
                HStack(spacing: 8) {
                    ProgressView()
                        .scaleEffect(0.72)
                    Text("Loading threads")
                        .font(GaryxFont.caption(weight: .medium))
                }
                .foregroundStyle(.secondary)
                .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                .padding(.vertical, 10)
            } else if let failureMessage {
                Text(failureMessage)
                    .font(GaryxFont.footnote())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                    .padding(.vertical, 8)
            } else if entries.isEmpty {
                Text("No triggered threads yet")
                    .font(GaryxFont.footnote())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                    .padding(.vertical, 8)
            } else {
                ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                    if index > 0 {
                        GaryxSidebarRowDivider()
                    }
                    if let thread = threadSummary(for: entry) {
                        GaryxSidebarThreadButton(
                            model: model,
                            thread: thread,
                            isSelected: model.selectedThread?.id == thread.id,
                            isPinned: model.isThreadPinned(thread.id),
                            trailingTimestamp: garyxFormattedTaskTimestamp(entry.finishedAt ?? entry.startedAt),
                            openSource: .current
                        )
                    } else {
                        GaryxAutomationThreadUnavailableRow(entry: entry)
                    }
                }
            }
        }
        .transition(.opacity)
        .task(id: automationId) {
            await load()
        }
    }

    private var entries: [GaryxAutomationThreadEntry] {
        page?.items ?? []
    }

    private func threadSummary(for entry: GaryxAutomationThreadEntry) -> GaryxThreadSummary? {
        entry.thread ?? model.sidebarThreadSummary(for: entry.threadId)
    }

    @MainActor
    private func load() async {
        guard !isLoading else { return }
        isLoading = true
        failureMessage = nil
        do {
            let client = try model.client()
            page = try await client.automationThreads(id: automationId)
        } catch {
            failureMessage = "Could not load triggered threads."
        }
        isLoading = false
    }
}

private struct GaryxAutomationThreadUnavailableRow: View {
    let entry: GaryxAutomationThreadEntry

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "exclamationmark.triangle")
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: GaryxSidebarMetrics.iconFrame, height: GaryxSidebarMetrics.iconFrame)

            VStack(alignment: .leading, spacing: 2) {
                Text("Thread unavailable")
                    .font(GaryxFont.subheadline(weight: .medium))
                    .foregroundStyle(.primary)
                    .lineLimit(1)

                Text(entry.threadId)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Spacer(minLength: 0)
        }
        .padding(.horizontal, GaryxSidebarMetrics.rowInnerHorizontalPadding)
        .frame(minHeight: GaryxSidebarMetrics.rowHeight)
        .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
    }
}

private struct GaryxSidebarBotRow: View {
    let group: GaryxMobileBotGroup
    let canDrillDown: Bool
    let onSelect: () -> Void
    let onOpenRoot: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            Button {
                if group.rootCanOpen {
                    onOpenRoot()
                } else if canDrillDown {
                    onSelect()
                }
            } label: {
                HStack(spacing: 10) {
                    GaryxChannelLogoView(
                        channel: group.channel,
                        label: group.title,
                        iconDataUrl: group.iconDataUrl,
                        diameter: 22
                    )

                    VStack(alignment: .leading, spacing: 2) {
                        Text(group.title)
                            .font(GaryxFont.subheadline(weight: .medium))
                            .foregroundStyle(.primary)
                            .lineLimit(1)

                        if !group.compactDetailLine.isEmpty {
                            Text(group.compactDetailLine)
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                    }

                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if canDrillDown {
                Button(action: onSelect) {
                    Image(systemName: "chevron.right")
                        .font(GaryxFont.system(size: 11, weight: .semibold))
                        .foregroundStyle(.tertiary)
                        .frame(width: 32, height: 32)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, GaryxSidebarMetrics.rowInnerHorizontalPadding)
        .frame(height: GaryxSidebarMetrics.rowHeight)
        .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
    }
}

private struct GaryxBotThreadDetailSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let group: GaryxMobileBotGroup

    private var entries: [GaryxBotSidebarConversationEntry] {
        group.sidebarChildConversationEntries()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            GaryxSidebarSectionHeader(title: "Threads", systemImage: "bubble.left.and.text.bubble.right.fill")
                .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                .padding(.bottom, 4)

            if entries.isEmpty {
                Text("No threads yet")
                    .font(GaryxFont.footnote())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                    .padding(.vertical, 8)
            } else {
                ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                    let timestamp = garyxFormattedTaskTimestamp(entry.latestActivity)
                    if index > 0 {
                        GaryxSidebarRowDivider()
                    }
                    if let thread = threadSummary(for: entry) {
                        GaryxSidebarThreadButton(
                            model: model,
                            thread: thread,
                            isSelected: model.selectedThread?.id == thread.id,
                            isPinned: model.isThreadPinned(thread.id),
                            trailingTimestamp: timestamp,
                            canArchive: canArchive(entry),
                            onSelect: {
                                guard entry.openable else { return }
                                Task { await model.openThread(thread, source: .current) }
                            },
                            onArchive: {
                                Task { await model.archiveBotConversationEndpoint(entry.endpoint) }
                            }
                        )
                    }
                }
            }
        }
        .transition(.opacity)
    }

    private func threadSummary(for entry: GaryxBotSidebarConversationEntry) -> GaryxThreadSummary? {
        guard let threadId = entry.threadId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            return nil
        }
        return model.sidebarThreadSummary(for: threadId)
            ?? entry.fallbackThreadSummary(workspacePath: group.workspaceDir)
    }

    private func canArchive(_ entry: GaryxBotSidebarConversationEntry) -> Bool {
        guard let threadId = entry.threadId,
              !threadId.isEmpty else {
            return false
        }
        return model.canArchiveThreadId(threadId)
    }
}

private struct GaryxWorkspaceThreadGroupsSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var activeDrilldown: GaryxWorkspaceBotsDrilldown?

    var body: some View {
        let groups = model.sidebarWorkspaceThreadGroups
        if !groups.isEmpty || isWorkspaceDrilldownActive {
            VStack(alignment: .leading, spacing: 0) {
                if case let .workspace(path) = activeDrilldown {
                    if let selectedGroup {
                        GaryxWorkspaceThreadDetailSection(
                            group: selectedGroup
                        )
                    } else {
                        GaryxWorkspaceBotsMissingDrilldownState(
                            title: "Workspace Not Found",
                            message: "Garyx could not find workspace \(path)."
                        )
                    }
                } else {
                    GaryxSidebarSectionHeader(title: "Workspaces", systemImage: "folder.fill")
                        .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                        .padding(.bottom, 4)

                    ForEach(groups) { group in
                        GaryxWorkspaceThreadGroupView(
                            group: group,
                            isSelected: false,
                            onSelect: {
                                withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                                    activeDrilldown = .workspace(group.path)
                                }
                            }
                        )
                    }
                }
            }
            .padding(.bottom, 10)
        }
    }

    private var selectedGroup: GaryxSidebarWorkspaceThreadGroup? {
        guard case let .workspace(path) = activeDrilldown else { return nil }
        return model.sidebarWorkspaceThreadGroups.first { $0.path == path }
    }

    private var isWorkspaceDrilldownActive: Bool {
        if case .workspace = activeDrilldown {
            return true
        }
        return false
    }
}

private struct GaryxWorkspaceBotsMissingDrilldownState: View {
    let title: String
    let message: String

    var body: some View {
        GaryxEmptyPanelView(
            icon: "magnifyingglass",
            title: title,
            text: message
        )
        .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
        .padding(.vertical, 12)
    }
}

private struct GaryxWorkspaceThreadGroupView: View {
    let group: GaryxSidebarWorkspaceThreadGroup
    let isSelected: Bool
    let onSelect: () -> Void

    var body: some View {
        GaryxDisclosureListRow(
            title: group.name,
            systemImage: "folder",
            selectedSystemImage: "folder.fill",
            isSelected: isSelected,
            iconFrame: GaryxSidebarMetrics.iconFrame,
            horizontalPadding: GaryxSidebarMetrics.rowInnerHorizontalPadding,
            verticalPadding: 0,
            minHeight: GaryxSidebarMetrics.rowHeight,
            titleWeight: .medium,
            action: onSelect
        )
        .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
    }
}

private struct GaryxWorkspaceThreadDetailSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let group: GaryxSidebarWorkspaceThreadGroup

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            GaryxSidebarSectionHeader(title: "Threads", systemImage: "bubble.left.and.text.bubble.right.fill")
                .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                .padding(.bottom, 4)

            if group.threads.isEmpty {
                Text("No threads yet")
                    .font(GaryxFont.footnote())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                    .padding(.vertical, 8)
            } else {
                ForEach(Array(group.threads.enumerated()), id: \.element.id) { index, thread in
                    if index > 0 {
                        GaryxSidebarRowDivider()
                    }
                    GaryxSidebarThreadButton(
                        model: model,
                        thread: thread,
                        isSelected: model.selectedThread?.id == thread.id,
                        isPinned: model.isThreadPinned(thread.id),
                        trailingTimestamp: garyxFormattedTaskTimestamp(thread.updatedAt ?? thread.createdAt),
                        openSource: .current
                    )
                }
            }
        }
        .transition(.opacity)
    }
}

private struct GaryxSidebarSectionHeader: View {
    let title: String
    let systemImage: String

    var body: some View {
        HStack(spacing: 0) {
            Text(title)
                .font(GaryxFont.caption(weight: .medium))
                .lineLimit(1)
        }
        .foregroundStyle(.secondary)
    }
}

private struct GaryxSidebarDisclosureRow: View {
    let title: String
    let systemName: String
    let action: () -> Void

    var body: some View {
        GaryxDisclosureListRow(
            title: title,
            systemImage: systemName,
            iconFrame: GaryxSidebarMetrics.iconFrame,
            horizontalPadding: GaryxSidebarMetrics.rowInnerHorizontalPadding,
            verticalPadding: 0,
            minHeight: GaryxSidebarMetrics.rowHeight,
            titleWeight: .medium,
            action: action
        )
    }
}

private struct GaryxSidebarThreadButton: View {
    // Plain reference on purpose: rows call model actions but must not each
    // subscribe to the whole observable model, which re-rendered every
    // materialized row on any model publish. Render state (`isSelected`,
    // `isPinned`) comes in as values from the observing parent section.
    let model: GaryxMobileModel
    let thread: GaryxThreadSummary
    var indent: CGFloat = 0
    var isSelected = false
    var isPinned = false
    var trailingTimestamp: String?
    var isFullBleed = false
    var canArchive: Bool?
    var openSource: GaryxMobilePanelOpenSource = .replace
    var onSelect: (() -> Void)?
    var onArchive: (() -> Void)?
    @State private var showsArchiveConfirmation = false

    var body: some View {
        GaryxSwipeActionRow(id: "thread:\(thread.id)", actions: threadSwipeActions) {
            GaryxSidebarThreadRowView(
                presentation: GaryxSidebarThreadRowPresentation(
                    thread: thread,
                    isSelected: isSelected,
                    isPinned: isPinned,
                    trailingTimestamp: trailingTimestamp
                ),
                avatar: rowAvatar,
                isFullBleed: isFullBleed,
                onSelect: {
                    if let onSelect {
                        onSelect()
                    } else {
                        Task { await model.openThread(thread, source: openSource) }
                    }
                },
                onUnpin: {
                    model.unpinThread(thread.id)
                }
            )
        }
        .onLongPressGesture {
            guard archiveAvailable else { return }
            showsArchiveConfirmation = true
        }
        .confirmationDialog("Archive thread", isPresented: $showsArchiveConfirmation, titleVisibility: .visible) {
            Button("Archive", role: .destructive) {
                archive()
            }
        }
        .padding(.leading, indent)
    }

    private var threadSwipeActions: [GaryxRowAction] {
        [
            GaryxRowAction(
                title: isPinned ? "Unpin thread" : "Pin thread",
                systemImage: isPinned ? "pin.slash" : "pin",
                tone: .neutral
            ) {
                model.togglePinnedThread(thread.id)
            },
            GaryxRowAction(
                title: "Archive thread",
                systemImage: "archivebox",
                tone: .destructive
            ) {
                archive()
            },
        ]
    }

    private var archiveAvailable: Bool {
        canArchive ?? model.canArchiveThread(thread)
    }

    // Same identity resolution as the recent-threads widget so sidebar rows
    // and widget rows show the same agent/team avatar for a thread.
    private var rowAvatar: GaryxSidebarThreadRowAvatar {
        let identity = model.widgetAgentIdentity(for: thread)
        return GaryxSidebarThreadRowAvatar(
            agentId: identity.id ?? "",
            avatarDataUrl: identity.avatarDataUrl ?? "",
            kind: identity.isTeam ? .team : .agent,
            label: identity.name ?? thread.title,
            providerType: identity.providerType ?? "",
            builtIn: identity.builtIn
        )
    }

    private func archive() {
        if let onArchive {
            onArchive()
        } else {
            Task { await model.archiveThread(thread) }
        }
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
                    kind: avatar.kind,
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
                        Button {
                            onUnpin?()
                        } label: {
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
            guard isEnabled else { return }
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
    var isPaused = false
    var scale: CGFloat = 1

    var body: some View {
        Group {
            if isPaused {
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
                    .easeInOut(duration: 0.34)
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
    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30.0)) { context in
            let cycle = 1.55
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

struct GaryxSidebarBottomActionBar: View {
    let isChatEnabled: Bool
    let isCreatingThread: Bool
    let onTapSettings: () -> Void
    let onTapChat: () -> Void

    var body: some View {
        GaryxAdaptiveGlassContainer(spacing: 10) {
            HStack(spacing: 10) {
                GaryxSidebarActionPill(
                    title: "Settings",
                    iconSystemName: "gearshape",
                    style: .glass,
                    action: onTapSettings
                )

                Spacer(minLength: 0)

                GaryxSidebarActionPill(
                    title: "Chat",
                    iconSystemName: "square.and.pencil",
                    style: .accent,
                    isEnabled: isChatEnabled,
                    isLoading: isCreatingThread,
                    action: onTapChat
                )
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 6)
        .padding(.bottom, 4)
        .frame(maxWidth: .infinity)
        // Absorb taps that land in the bar but miss the pills, so they do not
        // fall through to the thread rows scrolling behind this overlay bar.
        .contentShape(Rectangle())
        .onTapGesture {}
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

struct GaryxSidebarEmptyState: View {
    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: "bubble.left.and.text.bubble.right")
                .font(GaryxFont.title2(weight: .medium))
                .foregroundStyle(.secondary)
            Text("No threads yet")
                .font(GaryxFont.body(weight: .medium))
                .foregroundStyle(.primary)
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 24)
    }
}

struct GaryxWorkspaceBotsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsAddWorkspace = false
    @State private var addWorkspacePath = ""

    var body: some View {
        GaryxPanelScaffold(
            title: title,
            subtitle: "",
            onRefresh: { await refresh() },
            leadingActionLabel: nil,
            leadingAction: nil,
            // Thread and drilldown rows here are the home pinned+recent row
            // components; they own their horizontal geometry, so the page
            // must not add the default content inset on top of it.
            contentHorizontalPadding: 0
        ) {
            VStack(alignment: .leading, spacing: 0) {
                switch activeDrilldown {
                case .automationThreads:
                    GaryxSidebarAutomationsSection(activeDrilldown: activeDrilldownBinding)
                case .bot:
                    GaryxSidebarBotsSection(activeDrilldown: activeDrilldownBinding)
                case .workspace, nil:
                    // Root lists workspaces only; bots have their own page
                    // and automation threads open from the Automation page.
                    GaryxWorkspaceThreadGroupsSection(activeDrilldown: activeDrilldownBinding)
                    if activeDrilldown == nil, model.sidebarWorkspaceThreadGroups.isEmpty {
                        GaryxEmptyPanelView(
                            icon: "folder",
                            title: "No workspaces yet",
                            text: ""
                        )
                    }
                }
            }
        } actions: {
            if activeDrilldown == nil {
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
            await refresh()
        }
        .onDisappear {
            model.workspaceBotsDrilldown = nil
        }
        .sheet(isPresented: $showsAddWorkspace) {
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

    private var activeDrilldown: GaryxWorkspaceBotsDrilldown? {
        model.workspaceBotsDrilldown
    }

    private var activeDrilldownBinding: Binding<GaryxWorkspaceBotsDrilldown?> {
        Binding(
            get: { model.workspaceBotsDrilldown },
            set: { model.workspaceBotsDrilldown = $0 }
        )
    }

    private var title: String {
        switch activeDrilldown {
        case let .bot(id):
            model.mobileBotGroups.first { $0.id == id }?.title ?? "Bot"
        case let .workspace(path):
            model.sidebarWorkspaceThreadGroups.first { $0.path == path }?.name ?? "Workspace"
        case let .automationThreads(id):
            generatedAutomations.first { $0.id == id }?.label ?? "Automation Threads"
        case nil:
            "Workspaces"
        }
    }

    private var generatedAutomations: [GaryxAutomationSummary] {
        model.automations.filter(\.isGeneratedThreadMode)
    }

    private func refresh() async {
        await model.refreshRemoteState()
        await model.refreshWorkspaceAndBotThreads()
    }

    private func addWorkspace(_ path: String) async {
        guard let addedPath = await model.addUserWorkspacePath(path) else { return }
        await model.selectWorkspace(addedPath)
        await model.refreshWorkspaceAndBotThreads()
        model.workspaceBotsDrilldown = .workspace(addedPath)
    }
}
