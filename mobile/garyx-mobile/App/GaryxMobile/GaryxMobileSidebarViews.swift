import Foundation
import SwiftUI

enum GaryxSidebarDragAxis {
    case horizontal
    case vertical
}

/// Root content column: the home thread list with conversation and panel
/// pages pushed above it. Pushes originate from model navigation state; the
/// path binding only ever receives pops (system back swipe or back buttons).
struct GaryxRootNavigationView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        NavigationStack(path: rootPathBinding) {
            GaryxHomeThreadListView()
                .toolbar(.hidden, for: .navigationBar)
                .navigationDestination(for: GaryxMobileRootRoute.self) { route in
                    GaryxRootRouteContentView(route: route)
                        .toolbar(.hidden, for: .navigationBar)
                }
        }
        .garyxPageBackground()
        .fullScreenCover(item: $model.selectedRouteNotFound) { state in
            GaryxFormSheet(title: state.title) {
                GaryxRouteNotFoundCard(state: state)
            }
        }
    }

    private var rootPathBinding: Binding<[GaryxMobileRootRoute]> {
        Binding(
            get: { model.rootNavigationPath },
            set: { model.applyRootNavigationPath($0) }
        )
    }
}

private struct GaryxRootRouteContentView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let route: GaryxMobileRootRoute

    var body: some View {
        switch route {
        case .conversation:
            GaryxConversationView()
        case .panel(let panel):
            panelContent(for: panel)
        }
    }

    @ViewBuilder
    private func panelContent(for panel: GaryxMobilePanel) -> some View {
        switch panel {
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

struct GaryxHomeThreadListView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxSidebarDragActive) private var sidebarDragActive
    @Environment(\.garyxOpenSidebar) private var openDrawer
    private let silentRefreshIntervalNanos: UInt64 = 3_000_000_000

    var body: some View {
        threadListWithBottomBar
            .frame(maxHeight: .infinity)
            .garyxPageBackground()
            .garyxAdaptiveTopBar {
                GaryxHomeHeaderView(
                    onOpenDrawer: { openDrawer() },
                    onNewChat: { startNewChat() }
                )
            }
            .task(id: model.isHomeVisible) {
                await runSilentSidebarRefreshLoop()
            }
    }

    private var threadListWithBottomBar: some View {
        ScrollView(.vertical, showsIndicators: false) {
            LazyVStack(alignment: .leading, spacing: 0) {
                Color.clear
                    .frame(height: 4)
                    .accessibilityHidden(true)

                sidebarThreadSections

                Color.clear
                    .frame(height: GaryxSidebarMetrics.bottomBarClearance)
                    .accessibilityHidden(true)
            }
        }
        .scrollDisabled(sidebarDragActive)
        .scrollDismissesKeyboard(.interactively)
        .refreshable {
            await refreshAll()
        }
    }

    // Section headers and thread rows are emitted directly into the enclosing
    // LazyVStack. Wrapping a section's ForEach in its own VStack would turn the
    // whole section into one eager lazy item and materialize every row at once.
    @ViewBuilder
    private var sidebarThreadSections: some View {
        let pinned = model.pinnedThreads
        if !pinned.isEmpty {
            GaryxSidebarSectionHeader(title: "Pinned", systemImage: "pin.fill")
                .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                .padding(.bottom, 4)

            ForEach(Array(pinned.enumerated()), id: \.element.id) { index, thread in
                if index > 0 {
                    GaryxSidebarRowDivider()
                }
                GaryxSidebarThreadButton(
                    model: model,
                    thread: thread,
                    isSelected: model.selectedThread?.id == thread.id,
                    isPinned: true,
                    trailingTimestamp: garyxFormattedTaskTimestamp(thread.updatedAt ?? thread.createdAt)
                )
            }

            Color.clear
                .frame(height: 10)
                .accessibilityHidden(true)
        }

        let recent = model.recentThreads.filter { !model.isThreadPinned($0.id) }
        GaryxSidebarSectionHeader(title: "Recent", systemImage: "clock.fill")
            .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
            .padding(.bottom, 4)

        if recent.isEmpty {
            if model.isLoadingThreads {
                GaryxSidebarLoadingRow(title: "Loading recent threads")
            } else {
                GaryxSidebarEmptyRow(title: "No recent threads")
            }
        } else {
            ForEach(Array(recent.enumerated()), id: \.element.id) { index, thread in
                if index > 0 {
                    GaryxSidebarRowDivider()
                }
                GaryxSidebarThreadButton(
                    model: model,
                    thread: thread,
                    isSelected: model.selectedThread?.id == thread.id,
                    isPinned: false,
                    trailingTimestamp: garyxFormattedTaskTimestamp(thread.updatedAt ?? thread.createdAt)
                )
            }
        }

        Color.clear
            .frame(height: 10)
            .accessibilityHidden(true)

        GaryxSidebarThreadAutoLoadFooter()
    }

    private func refreshAll() async {
        await model.refreshThreads(silent: true)
        await model.refreshRemoteState()
    }

    private func runSilentSidebarRefreshLoop() async {
        guard shouldRefreshSidebarThreads else { return }
        // Let the drawer-open animation settle before the first refresh so
        // response handling does not contend with the opening transition.
        try? await Task.sleep(nanoseconds: 300_000_000)
        guard !Task.isCancelled, shouldRefreshSidebarThreads else { return }
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
        model.isHomeVisible
    }

    private func startNewChat() {
        model.openNewThreadDraft()
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
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxSidebarDragActive) private var sidebarDragActive

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            GaryxAdaptiveGlassContainer(spacing: 10) {
                HStack(alignment: .center, spacing: 12) {
                    // The gateway identity IS the drawer title.
                    GaryxSidebarGatewayIdentityControl()

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
                        isSelected: model.activePanel == .automations
                    )

                    GaryxSidebarNavigationRow(
                        panel: .agents,
                        isSelected: model.activePanel == .agents
                    )

                    if !model.mobileBotGroups.isEmpty {
                        GaryxDrawerSectionLabel(title: GaryxMobilePanel.bots.label)
                        ForEach(model.mobileBotGroups) { group in
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

                    if !model.sidebarWorkspaceThreadGroups.isEmpty {
                        GaryxDrawerSectionLabel(title: GaryxMobilePanel.workspaceBots.label)
                        ForEach(model.sidebarWorkspaceThreadGroups) { group in
                            GaryxDrawerChildRow(title: group.name) {
                                Image(systemName: "folder")
                                    .font(GaryxFont.system(size: 15, weight: .semibold))
                                    .foregroundStyle(.secondary)
                                    .frame(width: 22, height: 22)
                            } action: {
                                model.openWorkspaceBotsDrilldown(.workspace(group.path), source: .sidebar)
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
                        action: { model.openSettings() }
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
            Task { await model.openBotGroup(group) }
            model.setSidebarVisible(false)
        } else {
            model.openWorkspaceBotsDrilldown(.bot(group.id), source: .sidebar)
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
                            trailingTimestamp: garyxFormattedTaskTimestamp(entry.finishedAt ?? entry.startedAt)
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
    var onSelect: (() -> Void)?
    var onArchive: (() -> Void)?
    @State private var showsArchiveConfirmation = false

    var body: some View {
        GaryxSidebarThreadRowView(
            model: GaryxSidebarThreadRowPresentation(
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

struct GaryxSidebarThreadRowAvatar: Equatable {
    let agentId: String
    let avatarDataUrl: String
    let kind: GaryxMobileAgentTarget.Kind
    let label: String
    let providerType: String
    let builtIn: Bool
}

struct GaryxSidebarThreadRowView: View {
    let model: GaryxSidebarThreadRowPresentation
    var avatar: GaryxSidebarThreadRowAvatar?
    var isFullBleed = false
    var density: GaryxSidebarThreadRowDensity = .regular
    var selectionDisplay: GaryxSidebarThreadSelectionDisplay = .sidebar
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
                    if model.isRunning {
                        GaryxAvatarTypingBadge()
                            .offset(x: 3, y: 3)
                    }
                }
            }

            VStack(alignment: .leading, spacing: density.textSpacing) {
                HStack(alignment: .center, spacing: 5) {
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

                if let subtitle = model.subtitle, !subtitle.isEmpty {
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

/// Running-state badge pinned to the avatar's bottom-right corner: a small
/// tinted bubble with an iMessage-style three-dot typing wave, ringed by the
/// page background so it sits cleanly on any avatar.
struct GaryxAvatarTypingBadge: View {
    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30.0)) { context in
            let cycle = 1.05
            let progress = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: cycle) / cycle

            HStack(spacing: 2.2) {
                ForEach(0..<3, id: \.self) { index in
                    Circle()
                        .fill(Color(.systemGray).opacity(dotOpacity(progress: progress, index: index)))
                        .frame(width: 3.2, height: 3.2)
                }
            }
            .frame(width: 22, height: 15)
            .background(Color(.systemGray5), in: Capsule())
            .overlay {
                Capsule()
                    .stroke(GaryxTheme.background, lineWidth: 2)
            }
        }
        .accessibilityLabel("Running")
    }

    private func dotOpacity(progress: Double, index: Int) -> Double {
        let phase = progress * 2 * .pi - Double(index) * (.pi / 4)
        return 0.35 + 0.65 * max(0, sin(phase))
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
            if model.isRunning, avatar == nil {
                GaryxSidebarRunningIndicator()
            } else if model.isSelected, selectionDisplay == .checkmark {
                GaryxSelectionCheckmark(size: 13)
            } else if let trailingTimestamp = model.trailingTimestamp, !trailingTimestamp.isEmpty {
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
            leadingAction: nil
        ) {
            VStack(alignment: .leading, spacing: 16) {
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
