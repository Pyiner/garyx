import SwiftUI

struct GaryxWorkspaceThreadListDrilldown: View {
    let model: GaryxMobileModel
    let path: String
    @ObservedObject var store: GaryxThreadListStore
    @State private var showsRenameDialog = false
    @State private var renameDraft = ""
    @State private var showsRemoveConfirm = false

    private var workspaceSummary: GaryxWorkspaceSummary? {
        model.workspaceCatalog.summary(forPath: path)
    }

    private var displayName: String {
        workspaceSummary?.name
            ?? (path.garyxLastPathComponent.isEmpty ? path : path.garyxLastPathComponent)
    }

    var body: some View {
        GaryxListPanelScaffold(
            title: displayName,
            onRefresh: {
                await model.refreshWorkspaces()
                await model.refreshWorkspaceThreadList(path: path)
            }
        ) {
            workspaceHeaderCard
            GaryxThreadListRowsSection(
                model: model,
                store: store,
                emptyTitle: "No threads yet",
                onRetry: { await model.refreshWorkspaceThreadList(path: path) },
                onLoadMore: { trigger in
                    await model.loadMoreWorkspaceThreadList(path: path, trigger: trigger)
                },
                onRetryLoadMore: {
                    await model.retryLoadMoreWorkspaceThreadList(path: path)
                },
                onArchive: { thread, _ in
                    Task { await model.archiveThread(thread) }
                }
            )
        } actions: {
            workspaceActionsMenu
        }
        // One first-page request per resident scope instance. A gateway reset
        // replaces the store object, which re-arms the task even when the
        // visible route key itself did not change.
        .task(id: ObjectIdentifier(store)) {
            await model.refreshWorkspaceThreadList(path: path)
        }
        .task {
            if model.workspaceGitStatuses[path] == nil {
                await model.refreshWorkspaceGitStatus(for: path)
            }
        }
        .alert("Rename Workspace", isPresented: $showsRenameDialog) {
            TextField("Name", text: $renameDraft)
            Button("Cancel", role: .cancel) {}
            Button("Rename") {
                let name = renameDraft
                Task { await model.renameUserWorkspace(path: path, name: name) }
            }
        }
        .confirmationDialog(
            "Remove this workspace from the list? Files on the gateway machine are not touched.",
            isPresented: $showsRemoveConfirm,
            titleVisibility: .visible
        ) {
            Button("Remove Workspace", role: .destructive) {
                Task { await model.removeUserWorkspace(path: path) }
            }
            Button("Cancel", role: .cancel) {}
        }
        .onChange(of: model.workspaceCatalogState.phase) { _, phase in
            // A gateway switch resets the catalog; management dialogs from
            // the previous universe must not survive it.
            if phase == .idle {
                showsRenameDialog = false
                showsRemoveConfirm = false
            }
        }
    }

    /// The iOS adaptation of the desktop workspace hover card: name, pin
    /// state, thread count, `~`-abbreviated path (tap to copy), git branch.
    private var workspaceHeaderCard: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                if workspaceSummary?.pinned == true {
                    Label("Pinned", systemImage: "pin.fill")
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                }
                if let count = workspaceSummary?.threadCount {
                    Text(count == 1 ? "1 thread" : "\(count) threads")
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                }
                if let branch = model.workspaceGitStatuses[path]?.currentBranch,
                   !branch.isEmpty {
                    Label(branch, systemImage: "arrow.triangle.branch")
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .garyxReadingLineLimit()
                }
                Spacer(minLength: 0)
            }
            Button {
                UIPasteboard.general.string = path
            } label: {
                HStack(spacing: 5) {
                    Text(abbreviatedPath)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .garyxReadingLineLimit()
                        .truncationMode(.middle)
                    Image(systemName: "doc.on.doc")
                        .font(GaryxFont.fixedSystem(size: 10, weight: .regular))
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Copy workspace path")
        }
        .padding(.horizontal, 16)
        .padding(.top, 2)
        .padding(.bottom, 8)
    }

    private var abbreviatedPath: String {
        GaryxMobileWorkspacePresentation.abbreviatedPath(
            path,
            gatewayHome: model.gatewayHomePath
        )
    }

    private var workspaceActionsMenu: some View {
        Menu {
            Button {
                let pinned = workspaceSummary?.pinned == true
                Task { await model.setWorkspacePinned(path: path, pinned: !pinned) }
            } label: {
                if workspaceSummary?.pinned == true {
                    Label("Unpin", systemImage: "pin.slash")
                } else {
                    Label("Pin", systemImage: "pin")
                }
            }
            Button {
                renameDraft = displayName
                showsRenameDialog = true
            } label: {
                Label("Rename…", systemImage: "pencil")
            }
            Button {
                model.selectDraftWorkspace(path)
                model.openNewThreadDraft()
            } label: {
                Label("New Thread", systemImage: "square.and.pencil")
            }
            Button {
                UIPasteboard.general.string = path
            } label: {
                Label("Copy Path", systemImage: "doc.on.doc")
            }
            Divider()
            Button(role: .destructive) {
                showsRemoveConfirm = true
            } label: {
                Label("Remove", systemImage: "trash")
            }
        } label: {
            GaryxToolbarIcon(systemName: "ellipsis")
        }
        .accessibilityLabel("Workspace actions")
    }
}

struct GaryxAutomationThreadListDrilldown: View {
    let model: GaryxMobileModel
    let automation: GaryxAutomationSummary
    @ObservedObject var store: GaryxThreadListStore

    var body: some View {
        GaryxListPanelScaffold(
            title: automation.label,
            onRefresh: {
                await model.refreshAutomationThreadList(automationId: automation.id)
            }
        ) {
            GaryxThreadListRowsSection(
                model: model,
                store: store,
                emptyTitle: "No triggered threads yet",
                onRetry: {
                    await model.refreshAutomationThreadList(automationId: automation.id)
                },
                onLoadMore: { trigger in
                    await model.loadMoreAutomationThreadList(
                        automationId: automation.id,
                        trigger: trigger
                    )
                },
                onRetryLoadMore: {
                    await model.retryLoadMoreAutomationThreadList(automationId: automation.id)
                },
                onArchive: { thread, _ in
                    Task { await model.archiveThread(thread) }
                }
            )
        }
        .task(id: ObjectIdentifier(store)) {
            await model.refreshAutomationThreadList(automationId: automation.id)
        }
    }
}

struct GaryxBotThreadListDrilldown: View {
    let model: GaryxMobileModel
    let group: GaryxMobileBotGroup
    @ObservedObject var store: GaryxThreadListStore

    var body: some View {
        GaryxListPanelScaffold(
            title: group.title,
            onRefresh: {
                await model.refreshRemoteState()
                refreshCurrentGroup()
            }
        ) {
            GaryxThreadListRowsSection(
                model: model,
                store: store,
                emptyTitle: "No threads yet",
                onRetry: {
                    await model.refreshRemoteState()
                    refreshCurrentGroup()
                },
                onLoadMore: { _ in },
                onRetryLoadMore: {},
                onArchive: { thread, strategy in
                    guard strategy == .botEndpoint,
                          let entry = entriesByThreadId[thread.id] else {
                        Task { await model.archiveThread(thread) }
                        return
                    }
                    Task { await model.archiveBotConversationEndpoint(entry.endpoint) }
                }
            )
        }
        .task(id: ObjectIdentifier(store)) {
            model.refreshBotThreadList(group: group)
        }
        .onChange(of: group) { _, updatedGroup in
            // Catalog refreshes can change endpoint membership while this
            // narrow store remains mounted. Re-apply only this bot scope.
            model.refreshBotThreadList(group: updatedGroup)
        }
    }

    private var entriesByThreadId: [String: GaryxBotSidebarConversationEntry] {
        Dictionary(
            uniqueKeysWithValues: group.sidebarChildConversationEntries().compactMap { entry in
                guard let threadId = entry.threadId?.garyxTrimmedNilIfEmpty else { return nil }
                return (threadId, entry)
            }
        )
    }

    private func refreshCurrentGroup() {
        guard let current = model.mobileBotGroups.first(where: { $0.id == group.id }) else {
            return
        }
        model.refreshBotThreadList(group: current)
    }
}

private struct GaryxThreadListRowsSection: View {
    let model: GaryxMobileModel
    @ObservedObject var store: GaryxThreadListStore
    let emptyTitle: String
    let onRetry: () async -> Void
    let onLoadMore: (GaryxThreadListLoadMoreTrigger) async -> Void
    let onRetryLoadMore: () async -> Void
    let onArchive: (GaryxThreadSummary, GaryxThreadArchiveStrategy) -> Void

    var body: some View {
        switch store.snapshot.availability {
        case .unsupportedGateway:
            stateRow(
                icon: "exclamationmark.triangle",
                title: "网关版本过旧，请升级",
                message: ""
            )
        case .failed(let message):
            retryRow(message: message)
            if !store.snapshot.rows.isEmpty {
                threadRows
                footerRow
            }
        case .ready:
            if !store.snapshot.isPrimed && store.snapshot.isRefreshing {
                loadingRow
            } else if store.snapshot.rows.isEmpty {
                stateRow(icon: "bubble.left.and.text.bubble.right", title: emptyTitle, message: "")
            } else {
                if store.snapshot.headFailure {
                    retryRow(message: "The last refresh failed. Showing saved threads.")
                }
                threadRows
                footerRow
            }
        }
    }

    @ViewBuilder
    private var threadRows: some View {
        let prefetchId = GaryxThreadListPageMerge.prefetchTriggerRowId(
            recentIds: store.snapshot.rows.map(\.id)
        )
        ForEach(Array(store.snapshot.rows.enumerated()), id: \.element.id) { index, thread in
            let capabilities = store.snapshot.capabilitiesById[thread.id]
                ?? GaryxThreadRowCapabilityDeriver.capabilities(
                    for: nil,
                    context: GaryxThreadRowCapabilityContext(openable: false)
                )
            GaryxThreadListRowButton(
                input: GaryxThreadListRowInput(
                    thread: thread,
                    presentation: GaryxSidebarThreadRowPresentation(
                        thread: thread,
                        isSelected: store.snapshot.selectedThreadId == thread.id,
                        isPinned: store.snapshot.pinnedStateThreadIds.contains(thread.id),
                        isFavorite: store.snapshot.favoriteThreadIds.contains(thread.id),
                        trailingTimestamp: nil
                    ),
                    avatar: avatar(for: thread),
                    timestampValue: thread.updatedAt ?? thread.createdAt,
                    capabilities: capabilities,
                    motion: store.snapshot.motionById[thread.id] ?? .stable,
                    showsDivider: index > 0,
                    openSource: .current
                ),
                onOpenThread: { thread, source in
                    Task { await model.openThread(thread, source: source) }
                },
                onSetPinned: { threadId, desired in
                    if desired {
                        guard !model.isThreadPinned(threadId) else { return }
                        model.togglePinnedThread(threadId)
                    } else {
                        model.unpinThread(threadId)
                    }
                },
                onSetFavorite: { threadId, desired in
                    model.setThreadFavorite(threadId, desired: desired)
                },
                onArchive: onArchive
            )
            .equatable()
            .onAppear {
                if thread.id == prefetchId {
                    Task { await onLoadMore(.nearTail) }
                }
            }
        }
    }

    @ViewBuilder
    private var footerRow: some View {
        switch store.snapshot.footerState {
        case .hidden:
            EmptyView()
        case .idle:
            Color.clear
                .frame(height: 44)
                .onAppear { Task { await onLoadMore(.footer) } }
        case .loading:
            HStack(spacing: 8) {
                ProgressView().scaleEffect(0.72)
                Text("Loading more")
                    .font(GaryxFont.caption(weight: .medium))
            }
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, minHeight: 44)
        case .failed:
            Button {
                Task { await onRetryLoadMore() }
            } label: {
                Label("Couldn't load more · Tap to retry", systemImage: "arrow.clockwise")
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 44)
            }
            .buttonStyle(GaryxPressableRowStyle())
        }
    }

    private var loadingRow: some View {
        HStack(spacing: 8) {
            ProgressView().scaleEffect(0.72)
            Text("Loading threads")
                .font(GaryxFont.caption(weight: .medium))
        }
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, minHeight: 64)
    }

    private func retryRow(message: String) -> some View {
        Button {
            Task { await onRetry() }
        } label: {
            stateRow(
                icon: "arrow.clockwise",
                title: "Could not load threads",
                message: message
            )
        }
        .buttonStyle(GaryxPressableRowStyle())
    }

    private func stateRow(icon: String, title: String, message: String) -> some View {
        GaryxEmptyPanelView(icon: icon, title: title, text: message)
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
    }

    private func avatar(for thread: GaryxThreadSummary) -> GaryxSidebarThreadRowAvatar {
        let identity = model.widgetAgentIdentity(for: thread)
        return GaryxSidebarThreadRowAvatar(
            agentId: identity.id ?? "",
            avatarDataUrl: identity.avatarDataUrl ?? "",
            label: identity.name ?? thread.title,
            providerType: identity.providerType ?? "",
            builtIn: identity.builtIn
        )
    }
}

enum GaryxWorkspaceRowAction {
    case togglePin
    case rename
    case newThread
    case copyPath
    case remove
}

struct GaryxWorkspaceRootSection: View {
    let groups: [GaryxSidebarWorkspaceThreadGroup]
    let onSelect: (String) -> Void
    var onAction: ((GaryxWorkspaceRowAction, GaryxSidebarWorkspaceThreadGroup) -> Void)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if groups.isEmpty {
                GaryxEmptyPanelView(icon: "folder", title: "No workspaces yet", text: "")
                    .padding(.horizontal, 16)
            } else {
                GaryxSidebarSectionHeader(title: "Workspaces", systemImage: "folder.fill")
                    .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                    .padding(.bottom, 4)
                ForEach(groups) { group in
                    GaryxDisclosureListRow(
                        title: group.name,
                        systemImage: group.pinned ? "pin.fill" : "folder",
                        selectedSystemImage: group.pinned ? "pin.fill" : "folder.fill",
                        iconFrame: GaryxSidebarMetrics.iconFrame,
                        horizontalPadding: GaryxSidebarMetrics.rowInnerHorizontalPadding,
                        verticalPadding: 0,
                        minHeight: GaryxSidebarMetrics.rowHeight,
                        titleWeight: .medium,
                        action: { onSelect(group.path) }
                    )
                    .accessibilityIdentifier("workspace-row-\(group.path)")
                    .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
                    .contextMenu {
                        if let onAction {
                            workspaceRowMenu(group: group, onAction: onAction)
                        }
                    }
                }
            }
        }
        .padding(.bottom, 10)
    }

    @ViewBuilder
    private func workspaceRowMenu(
        group: GaryxSidebarWorkspaceThreadGroup,
        onAction: @escaping (GaryxWorkspaceRowAction, GaryxSidebarWorkspaceThreadGroup) -> Void
    ) -> some View {
        Button {
            onAction(.togglePin, group)
        } label: {
            if group.pinned {
                Label("Unpin", systemImage: "pin.slash")
            } else {
                Label("Pin", systemImage: "pin")
            }
        }
        Button {
            onAction(.rename, group)
        } label: {
            Label("Rename…", systemImage: "pencil")
        }
        Button {
            onAction(.newThread, group)
        } label: {
            Label("New Thread", systemImage: "square.and.pencil")
        }
        Button {
            onAction(.copyPath, group)
        } label: {
            Label("Copy Path", systemImage: "doc.on.doc")
        }
        Divider()
        Button(role: .destructive) {
            onAction(.remove, group)
        } label: {
            Label("Remove", systemImage: "trash")
        }
    }
}
