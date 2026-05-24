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
                    Text("Gary X")
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
            model.openPanel(panel)
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

enum GaryxSidebarDrilldown: Equatable {
    case bot(String)
    case workspace(String)
}

private extension GaryxMobileModel {
    var sidebarWorkspaceThreadGroups: [GaryxSidebarWorkspaceThreadGroup] {
        let grouped = Dictionary(grouping: threads) { thread in
            thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        }
        let paths = knownWorkspacePaths
            .filter(GaryxMobileModel.isVisibleMobileWorkspacePath)
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
    @Binding var activeDrilldown: GaryxSidebarDrilldown?

    private var groups: [GaryxMobileBotGroup] {
        model.mobileBotGroups
    }

    var body: some View {
        let visibleThreadIds = model.sidebarVisibleThreadIds
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
                        GaryxSidebarBotRow(
                            group: group,
                            canDrillDown: !group.sidebarChildConversationEntries(visibleThreadIds: visibleThreadIds).isEmpty,
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

    private var rootCanOpen: Bool {
        let mainThreadId = group.mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let defaultOpenThreadId = group.defaultOpenThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return group.rootBehavior != "expand_only" || !mainThreadId.isEmpty || !defaultOpenThreadId.isEmpty
    }

    var body: some View {
        HStack(spacing: 0) {
            Button {
                if rootCanOpen {
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

                    Text(group.title)
                        .font(GaryxFont.subheadline(weight: .medium))
                        .foregroundStyle(.primary)
                        .lineLimit(1)

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

private struct GaryxBotSidebarConversationEntry: Identifiable, Equatable {
    let id: String
    let title: String
    let subtitle: String?
    let threadId: String?
    let latestActivity: String?
    let openable: Bool
    let endpoint: GaryxChannelEndpoint
}

extension GaryxMobileBotGroup {
    var compactDetailLine: String {
        let channelName = garyxBotChannelDisplayName(channel)
        let account = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        let botId = account.isEmpty ? channelName : "\(channelName) · \(account)"
        let agent = agentId?.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = workspaceDir?.trimmingCharacters(in: .whitespacesAndNewlines)
        return [
            botId,
            agent.flatMap { $0.isEmpty ? nil : $0 },
            workspace.flatMap { $0.isEmpty ? nil : $0.garyxLastPathComponent },
        ]
        .compactMap { $0 }
        .joined(separator: " / ")
    }

    fileprivate func sidebarChildConversationEntries(visibleThreadIds: Set<String>) -> [GaryxBotSidebarConversationEntry] {
        var entries: [GaryxBotSidebarConversationEntry] = []
        var seenThreadIds = Set<String>()
        let rootThreadId = mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

        if !conversationNodes.isEmpty {
            for node in conversationNodes {
                let threadId = node.endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines)
                guard let threadId, !threadId.isEmpty, visibleThreadIds.contains(threadId) else {
                    continue
                }
                if threadId == rootThreadId {
                    continue
                }
                if seenThreadIds.contains(threadId) {
                    continue
                }
                seenThreadIds.insert(threadId)
                entries.append(
                    GaryxBotSidebarConversationEntry(
                        id: node.id.isEmpty ? node.endpoint.endpointKey : node.id,
                        title: node.title.isEmpty ? node.endpoint.displayLabel : node.title,
                        subtitle: node.badge ?? node.endpoint.conversationLabel ?? node.endpoint.threadLabel,
                        threadId: threadId,
                        latestActivity: node.latestActivity,
                        openable: node.openable,
                        endpoint: node.endpoint
                    )
                )
            }
            return entries.sorted(by: garyxBotConversationEntrySort)
        }

        for endpoint in endpoints {
            let threadId = endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines)
            let conversationKind = endpoint.conversationKind?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
            guard let threadId, !threadId.isEmpty, conversationKind == "group" || conversationKind == "topic" else {
                continue
            }
            guard visibleThreadIds.contains(threadId) else {
                continue
            }
            if threadId == rootThreadId {
                continue
            }
            if seenThreadIds.contains(threadId) {
                continue
            }
            seenThreadIds.insert(threadId)
            entries.append(
                GaryxBotSidebarConversationEntry(
                    id: endpoint.endpointKey,
                    title: endpoint.displayLabel.isEmpty ? (endpoint.threadLabel ?? "Thread") : endpoint.displayLabel,
                    subtitle: endpoint.conversationLabel ?? endpoint.threadLabel ?? endpoint.workspaceDir?.garyxLastPathComponent,
                    threadId: threadId,
                    latestActivity: endpoint.lastInboundAt ?? endpoint.lastDeliveryAt,
                    openable: true,
                    endpoint: endpoint
                )
            )
        }

        return entries.sorted(by: garyxBotConversationEntrySort)
    }
}

private func garyxBotConversationEntrySort(
    _ lhs: GaryxBotSidebarConversationEntry,
    _ rhs: GaryxBotSidebarConversationEntry
) -> Bool {
    let titleOrder = lhs.title.localizedCaseInsensitiveCompare(rhs.title)
    if titleOrder != .orderedSame {
        return titleOrder == .orderedAscending
    }
    return lhs.id.localizedCaseInsensitiveCompare(rhs.id) == .orderedAscending
}

private func garyxBotChannelDisplayName(_ channel: String) -> String {
    let normalized = channel.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    switch normalized {
    case "telegram":
        return "Telegram"
    case "discord":
        return "Discord"
    case "api":
        return "API"
    default:
        return normalized.isEmpty ? "Channel" : normalized.replacingOccurrences(of: "_", with: " ").capitalized
    }
}

private struct GaryxBotThreadDetailSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let group: GaryxMobileBotGroup

    private var entries: [GaryxBotSidebarConversationEntry] {
        group.sidebarChildConversationEntries(visibleThreadIds: model.sidebarVisibleThreadIds)
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
                    let isSelected = entry.threadId.map { $0 == model.selectedThread?.id } ?? false
                    let timestamp = garyxFormattedTaskTimestamp(entry.latestActivity)
                    GaryxSwipeActionRow(actions: conversationActions(for: entry)) {
                        Button {
                            guard let threadId = entry.threadId, entry.openable else { return }
                            Task { await model.openBotThread(threadId) }
                        } label: {
                            HStack(alignment: .center, spacing: 8) {
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(entry.title)
                                        .font(GaryxFont.subheadline(weight: .medium))
                                        .foregroundStyle(.primary)
                                        .lineLimit(1)
                                        .truncationMode(.tail)

                                    if let subtitle = entry.subtitle, !subtitle.isEmpty {
                                        Text(subtitle)
                                            .font(GaryxFont.caption())
                                            .foregroundStyle(.secondary)
                                            .lineLimit(1)
                                            .truncationMode(.tail)
                                    }
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)

                                if isSelected {
                                    Circle()
                                        .fill(GaryxTheme.accent)
                                        .frame(width: 7, height: 7)
                                } else if !timestamp.isEmpty {
                                    Text(timestamp)
                                        .font(GaryxFont.caption())
                                        .foregroundStyle(.tertiary)
                                        .lineLimit(1)
                                }
                            }
                            .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                            .padding(.vertical, 10)
                            .background {
                                if isSelected {
                                    Color(.secondarySystemGroupedBackground)
                                }
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(!entry.openable)
                    }
                }
            }
        }
        .transition(.opacity)
    }

    private func conversationActions(for entry: GaryxBotSidebarConversationEntry) -> [GaryxSwipeAction] {
        guard let threadId = entry.threadId,
              !threadId.isEmpty,
              !model.isThreadBusy(threadId) else {
            return []
        }
        return [
            GaryxSwipeAction(title: "Archive", systemImage: "archivebox", tone: .destructive) {
                Task { await model.archiveBotConversationEndpoint(entry.endpoint) }
            }
        ]
    }
}

private struct GaryxWorkspaceThreadGroupsSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var activeDrilldown: GaryxSidebarDrilldown?

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
        Button(action: onSelect) {
            HStack(spacing: 10) {
                Image(systemName: isSelected ? "folder.fill" : "folder")
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(isSelected ? .primary : .secondary)
                    .frame(width: GaryxSidebarMetrics.iconFrame, height: GaryxSidebarMetrics.iconFrame)

                Text(group.name)
                    .font(GaryxFont.subheadline(weight: .medium))
                    .foregroundStyle(.primary)
                    .lineLimit(1)

                Spacer(minLength: 0)

                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, GaryxSidebarMetrics.rowInnerHorizontalPadding)
            .frame(height: GaryxSidebarMetrics.rowHeight)
            .background {
                if isSelected {
                    Color(.tertiarySystemFill).opacity(0.56)
                        .clipShape(
                            RoundedRectangle(
                                cornerRadius: GaryxSidebarMetrics.rowCornerRadius,
                                style: .continuous
                            )
                        )
                }
            }
            .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
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
                        showsWorkspaceMeta: false,
                        trailingTimestamp: garyxFormattedTaskTimestamp(thread.updatedAt ?? thread.createdAt),
                        isFullBleed: true
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
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: systemName)
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: GaryxSidebarMetrics.iconFrame, height: GaryxSidebarMetrics.iconFrame)

                Text(title)
                    .font(GaryxFont.subheadline(weight: .medium))
                    .foregroundStyle(.primary)
                    .lineLimit(1)

                Spacer(minLength: 0)

                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, GaryxSidebarMetrics.rowInnerHorizontalPadding)
            .frame(height: GaryxSidebarMetrics.rowHeight)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
    }
}

private struct GaryxSidebarThreadButton: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let thread: GaryxThreadSummary
    var indent: CGFloat = 0
    var showsPinnedMarker = false
    var showsWorkspaceMeta = true
    var trailingTimestamp: String?
    var isFullBleed = false

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
                Task { await model.selectThread(thread) }
            },
            onUnpin: {
                model.unpinThread(thread.id)
            }
        )
        .padding(.leading, indent)
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
            42
        }
    }

    func verticalPadding(isFullBleed: Bool) -> CGFloat {
        switch self {
        case .regular:
            isFullBleed ? 10 : 8
        case .compact:
            6
        }
    }
}

struct GaryxSidebarThreadRowView: View {
    let model: GaryxSidebarThreadRowPresentation
    var isFullBleed = false
    var density: GaryxSidebarThreadRowDensity = .regular
    var onSelect: (() -> Void)?
    var onUnpin: (() -> Void)?

    var body: some View {
        HStack(alignment: .center, spacing: 8) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Text(model.title)
                        .font(GaryxFont.subheadline(weight: .medium))
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
                        .font(GaryxFont.caption())
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
            if model.isSelected {
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
                Circle()
                    .fill(.secondary)
                    .frame(width: 7, height: 7)
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
    @State private var activeDrilldown: GaryxSidebarDrilldown?

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
        }
        .task {
            await refresh()
        }
        .onAppear {
            syncWorkspaceBotsDrilldownState()
        }
        .onChange(of: activeDrilldown) { _, _ in
            syncWorkspaceBotsDrilldownState()
        }
        .onChange(of: model.workspaceBotsBackRequest) { _, _ in
            guard activeDrilldown != nil else { return }
            goBack()
        }
        .onDisappear {
            model.workspaceBotsDrilldownActive = false
        }
    }

    private var activeDrilldownBinding: Binding<GaryxSidebarDrilldown?> {
        Binding(
            get: { activeDrilldown },
            set: { activeDrilldown = $0 }
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

    private func goBack() {
        if activeDrilldown != nil {
            withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                activeDrilldown = nil
            }
        }
    }

    private func syncWorkspaceBotsDrilldownState() {
        model.workspaceBotsDrilldownActive = activeDrilldown != nil
    }
}
