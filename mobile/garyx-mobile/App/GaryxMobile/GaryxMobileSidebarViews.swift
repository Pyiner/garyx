import Foundation
import SwiftUI

enum GaryxSidebarDragAxis {
    case horizontal
    case vertical
}

struct GaryxMainPanelView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        NavigationStack {
            Group {
                switch model.activePanel {
                case .chat:
                    GaryxConversationView()
                case .dreams:
                    if model.dreamsAutoScanEnabled {
                        GaryxDreamsView()
                    } else {
                        GaryxConversationView()
                    }
                case .tasks:
                    GaryxTasksView()
                case .workspaces:
                    GaryxWorkspacesView()
                case .automations:
                    GaryxAutomationsView()
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
                case .autoResearch:
                    GaryxAutoResearchView()
                case .bots:
                    GaryxBotsView()
                case .settings:
                    GaryxMobileSettingsPanel()
                }
            }
        }
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
    static let bottomBarClearance: CGFloat = 112
}

struct GaryxThreadSidebar: View {
    @EnvironmentObject private var model: GaryxMobileModel
    var showsInlineCloseButton: Bool
    private let silentRefreshIntervalNanos: UInt64 = 3_000_000_000

    var body: some View {
        threadListWithBottomBar
            .frame(maxHeight: .infinity)
            .background(GaryxTheme.background)
            .garyxAdaptiveTopBar {
                GaryxSidebarHeaderView(
                    drilldownContext: sidebarHeaderContext,
                    showsCloseButton: showsInlineCloseButton,
                    onBack: {},
                    onClose: { closeSidebar() }
                )
            }
            .task(id: model.sidebarVisible) {
                await runSilentSidebarRefreshLoop()
            }
    }

    private var threadListWithBottomBar: some View {
        ScrollView(.vertical, showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: 0) {
                GaryxSidebarNavigationList()
                    .padding(.horizontal, GaryxSidebarMetrics.outerHorizontalPadding)
                    .padding(.top, 6)
                    .padding(.bottom, 14)

                sidebarThreadSections

                Color.clear
                    .frame(height: GaryxSidebarMetrics.bottomBarClearance)
                    .accessibilityHidden(true)
            }
        }
        .scrollDismissesKeyboard(.interactively)
        .refreshable {
            await refreshAll()
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            GaryxSidebarBottomActionBar(
                isChatEnabled: model.hasGatewaySettings,
                isCreatingThread: false,
                onTapSettings: {
                    model.openSettings()
                },
                onTapChat: {
                    startNewChat()
                }
            )
        }
    }

    @ViewBuilder
    private var sidebarThreadSections: some View {
        GaryxPinnedThreadsSection()
        GaryxRecentThreadsSection()
        GaryxSidebarThreadAutoLoadFooter()
    }

    private var sidebarHeaderContext: GaryxSidebarHeaderContext? {
        nil
    }

    private func closeSidebar() {
        model.setSidebarVisible(false)
    }

    private func refreshAll() async {
        await model.refreshThreads(silent: true)
        await model.refreshRemoteState()
    }

    private func runSilentSidebarRefreshLoop() async {
        guard shouldRefreshSidebarThreads else { return }
        await refreshSidebarThreads(silent: true)
        while !Task.isCancelled {
            try? await Task.sleep(nanoseconds: silentRefreshIntervalNanos)
            guard !Task.isCancelled, shouldRefreshSidebarThreads else { return }
            await refreshSidebarThreads(silent: true)
        }
    }

    private func refreshSidebarThreads(silent: Bool = false) async {
        guard shouldRefreshSidebarThreads else { return }
        guard !model.isLoadingThreads, !model.isLoadingMoreThreads else { return }
        await model.refreshThreads(silent: silent)
    }

    private var shouldRefreshSidebarThreads: Bool {
        !(showsInlineCloseButton && !model.sidebarVisible)
    }

    private func startNewChat() {
        model.openNewThreadDraft()
    }
}

struct GaryxSidebarHeaderContext: Equatable {
    let title: String
    let subtitle: String?
}

struct GaryxSidebarHeaderView: View {
    let drilldownContext: GaryxSidebarHeaderContext?
    let showsCloseButton: Bool
    let onBack: () -> Void
    let onClose: () -> Void

    var body: some View {
        GaryxAdaptiveGlassContainer(spacing: 10) {
            HStack(alignment: .center, spacing: 12) {
                if let drilldownContext {
                    Button(action: onBack) {
                        GaryxToolbarIcon(systemName: "chevron.left")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Back")

                    GaryxPanelHeaderTitle(
                        title: drilldownContext.title,
                        subtitle: drilldownContext.subtitle ?? ""
                    )
                    .layoutPriority(1)
                } else {
                    Text("Garyx")
                        .font(GaryxFont.system(size: 26, weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.75)

                    Spacer(minLength: 0)
                }

                Spacer(minLength: 0)

                if showsCloseButton {
                    Button(action: onClose) {
                        GaryxToolbarIcon(systemName: "xmark")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Close menu")
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
    }
}

struct GaryxSidebarNavigationList: View {
    @EnvironmentObject private var model: GaryxMobileModel

    private let panels: [GaryxMobilePanel] = [
        .automations,
        .workspaceBots,
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(panels) { panel in
                GaryxSidebarNavigationRow(
                    panel: panel,
                    isSelected: model.activePanel == panel
                )
            }
        }
    }
}

struct GaryxSidebarNavigationRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let panel: GaryxMobilePanel
    let isSelected: Bool

    var body: some View {
        Button {
            model.openPanel(panel, source: .sidebar)
        } label: {
            HStack(spacing: 12) {
                Image(systemName: panel.iconName)
                    .font(GaryxFont.system(size: 19, weight: .regular))
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

private struct GaryxRecentThreadsSection: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        let threads = model.recentThreads.filter { !model.isThreadPinned($0.id) }
        VStack(alignment: .leading, spacing: 0) {
            GaryxSidebarSectionHeader(title: "Recent", systemImage: "clock.fill")
                .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                .padding(.bottom, 4)

            if threads.isEmpty {
                if model.isLoadingThreads {
                    GaryxSidebarLoadingRow(title: "Loading recent threads")
                } else {
                    GaryxSidebarEmptyRow(title: "No recent threads")
                }
            } else {
                ForEach(threads) { thread in
                    GaryxSidebarThreadButton(
                        thread: thread,
                        trailingTimestamp: garyxFormattedTaskTimestamp(thread.updatedAt ?? thread.createdAt)
                    )
                }
            }
        }
        .padding(.bottom, 10)
        .transition(.opacity)
    }
}

private struct GaryxPinnedThreadsSection: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        if !model.pinnedThreads.isEmpty {
            GaryxPinnedThreadsDetailSection()
                .padding(.bottom, 10)
        }
    }
}

private struct GaryxPinnedThreadsDetailSection: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            GaryxSidebarSectionHeader(title: "Pinned", systemImage: "pin.fill")
                .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                .padding(.bottom, 4)

            ForEach(model.pinnedThreads) { thread in
                GaryxSidebarThreadButton(
                    thread: thread,
                    showsPinnedMarker: true,
                    trailingTimestamp: garyxFormattedTaskTimestamp(thread.updatedAt ?? thread.createdAt)
                )
            }
        }
        .transition(.opacity)
    }
}

private struct GaryxSidebarLoadingRow: View {
    let title: String

    var body: some View {
        HStack(spacing: 8) {
            ProgressView()
                .scaleEffect(0.68)
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
        }
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity)
        .frame(minHeight: 44)
        .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
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

private struct GaryxSidebarThreadAutoLoadFooter: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        Group {
            if model.isLoadingMoreThreads {
                HStack(spacing: 8) {
                    ProgressView()
                        .scaleEffect(0.68)
                    Text("Loading more")
                        .font(GaryxFont.caption(weight: .medium))
                }
                .foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity)
                .frame(minHeight: 44)
            } else if model.hasMoreThreadSummaries {
                Color.clear
                    .frame(height: 1)
                    .onAppear {
                        Task { await model.loadMoreThreads() }
                    }
            }
        }
        .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
        .padding(.bottom, 10)
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
            if let selectedGroup {
                GaryxBotThreadDetailSection(
                    group: selectedGroup
                )
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
                ForEach(entries) { entry in
                    let timestamp = garyxFormattedTaskTimestamp(entry.latestActivity)
                    if let thread = threadSummary(for: entry) {
                        GaryxSidebarThreadButton(
                            thread: thread,
                            trailingTimestamp: timestamp,
                            canArchive: canArchive(entry),
                            onSelect: {
                                guard entry.openable else { return }
                                Task { await model.openThread(thread) }
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
        if !groups.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                if let selectedGroup {
                    GaryxWorkspaceThreadDetailSection(
                        group: selectedGroup
                    )
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
                ForEach(group.threads) { thread in
                    GaryxSidebarThreadButton(
                        thread: thread,
                        trailingTimestamp: garyxFormattedTaskTimestamp(thread.updatedAt ?? thread.createdAt)
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
    @EnvironmentObject private var model: GaryxMobileModel
    let thread: GaryxThreadSummary
    var indent: CGFloat = 0
    var showsPinnedMarker = false
    var trailingTimestamp: String?
    var isFullBleed = false
    var canArchive: Bool?
    var onSelect: (() -> Void)?
    var onArchive: (() -> Void)?
    @State private var showsArchiveConfirmation = false

    var body: some View {
        GaryxSidebarThreadRowView(
            model: GaryxSidebarThreadRowPresentation(
                thread: thread,
                isSelected: model.selectedThread?.id == thread.id,
                isPinned: showsPinnedMarker || model.isThreadPinned(thread.id),
                trailingTimestamp: trailingTimestamp
            ),
            isFullBleed: isFullBleed,
            onSelect: {
                if let onSelect {
                    onSelect()
                } else {
                    Task { await model.openThread(thread) }
                }
            },
            onUnpin: {
                model.unpinThread(thread.id)
            }
        )
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

    private var archiveAvailable: Bool {
        canArchive ?? model.canArchiveThread(thread)
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
            .medium
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
            GaryxFont.caption()
        case .compact:
            GaryxFont.caption()
        }
    }

    var textSpacing: CGFloat {
        switch self {
        case .regular:
            4
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

struct GaryxSidebarThreadRowView: View {
    let model: GaryxSidebarThreadRowPresentation
    var isFullBleed = false
    var density: GaryxSidebarThreadRowDensity = .regular
    var selectionDisplay: GaryxSidebarThreadSelectionDisplay = .sidebar
    var onSelect: (() -> Void)?
    var onUnpin: (() -> Void)?

    var body: some View {
        HStack(alignment: .center, spacing: 8) {
            VStack(alignment: .leading, spacing: density.textSpacing) {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Text(model.title)
                        .font(density.titleFont)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(.primary)
                        .layoutPriority(1)

                    if model.isPinned {
                        Button {
                            onUnpin?()
                        } label: {
                            Image(systemName: "pin.fill")
                                .font(GaryxFont.system(size: 13, weight: .semibold))
                                .foregroundStyle(.secondary)
                                .rotationEffect(.degrees(28))
                                .frame(width: 22, height: 22)
                        }
                        .frame(width: 30, height: 24)
                        .contentShape(Rectangle())
                        .buttonStyle(.plain)
                        .accessibilityLabel("Unpin thread")
                    }
                }

                if let subtitle = model.subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(density.subtitleFont)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            trailingMeta
                .fixedSize(horizontal: true, vertical: false)
        }
        .contentShape(Rectangle())
        .onTapGesture {
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
        .background {
            if model.isSelected, selectionDisplay == .sidebar {
                if isFullBleed {
                    Color(.secondarySystemGroupedBackground)
                } else {
                    Color(.tertiarySystemFill).opacity(0.5)
                        .clipShape(
                            RoundedRectangle(
                                cornerRadius: GaryxSidebarMetrics.selectedThreadCornerRadius,
                                style: .continuous
                            )
                        )
                }
            }
        }
        .padding(.horizontal, isFullBleed ? 0 : GaryxSidebarMetrics.rowOuterPadding - 4)
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
            model.title,
            model.subtitle,
            model.isPinned ? "Pinned" : nil,
            model.isRunning ? "Running" : nil,
        ]
        .compactMap { value in
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return trimmed.isEmpty ? nil : trimmed
        }
        .joined(separator: ", ")
    }

    var trailingMeta: some View {
        HStack(spacing: 6) {
            if model.isRunning {
                GaryxSidebarRunningIndicator()
            } else if model.isSelected {
                switch selectionDisplay {
                case .sidebar:
                    Circle()
                        .fill(.secondary)
                        .frame(width: 7, height: 7)
                case .checkmark:
                    GaryxSelectionCheckmark(size: 13)
                case .none:
                    EmptyView()
                }
            } else if let trailingTimestamp = model.trailingTimestamp, !trailingTimestamp.isEmpty {
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
            leadingActionLabel: activeDrilldown == nil ? nil : "Workspace & Bots",
            leadingAction: activeDrilldown == nil ? nil : { goBack() }
        ) {
            VStack(alignment: .leading, spacing: 16) {
                switch activeDrilldown {
                case .bot:
                    GaryxSidebarBotsSection(activeDrilldown: activeDrilldownBinding)
                case .workspace:
                    GaryxWorkspaceThreadGroupsSection(activeDrilldown: activeDrilldownBinding)
                case nil:
                    GaryxSidebarBotsSection(activeDrilldown: activeDrilldownBinding)
                    GaryxWorkspaceThreadGroupsSection(activeDrilldown: activeDrilldownBinding)
                    if model.mobileBotGroups.isEmpty && model.sidebarWorkspaceThreadGroups.isEmpty {
                        GaryxEmptyPanelView(
                            icon: "folder",
                            title: "No workspaces or bots yet",
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
        case nil:
            "Workspace & Bots"
        }
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

    private func goBack() {
        if activeDrilldown != nil {
            withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                model.workspaceBotsDrilldown = nil
            }
        }
    }
}
