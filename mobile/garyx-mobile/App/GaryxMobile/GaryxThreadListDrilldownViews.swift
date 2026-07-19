import SwiftUI

struct GaryxWorkspaceThreadListDrilldown: View {
    let model: GaryxMobileModel
    let path: String
    @ObservedObject var store: GaryxThreadListStore

    var body: some View {
        GaryxListPanelScaffold(
            title: path.garyxLastPathComponent.isEmpty ? path : path.garyxLastPathComponent,
            onRefresh: { await model.refreshWorkspaceThreadList(path: path) }
        ) {
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
        }
        // One first-page request per resident scope instance. A gateway reset
        // replaces the store object, which re-arms the task even when the
        // visible route key itself did not change.
        .task(id: ObjectIdentifier(store)) {
            await model.refreshWorkspaceThreadList(path: path)
        }
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

struct GaryxWorkspaceRootSection: View {
    let groups: [GaryxSidebarWorkspaceThreadGroup]
    let onSelect: (String) -> Void

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
                        systemImage: "folder",
                        selectedSystemImage: "folder.fill",
                        iconFrame: GaryxSidebarMetrics.iconFrame,
                        horizontalPadding: GaryxSidebarMetrics.rowInnerHorizontalPadding,
                        verticalPadding: 0,
                        minHeight: GaryxSidebarMetrics.rowHeight,
                        titleWeight: .medium,
                        action: { onSelect(group.path) }
                    )
                    .accessibilityIdentifier("workspace-row-\(group.path)")
                    .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
                }
            }
        }
        .padding(.bottom, 10)
    }
}
