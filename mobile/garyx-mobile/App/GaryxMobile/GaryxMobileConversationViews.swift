import Foundation
import ImageIO
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

private let garyxHistoryPrefetchBoundaryRows = 3
private let garyxHistoryPrefetchMinDistance: CGFloat = 640
private let garyxHistoryPrefetchViewportMultiplier: CGFloat = 1.5

private func garyxDismissKeyboard() {
    UIApplication.shared.sendAction(
        #selector(UIResponder.resignFirstResponder),
        to: nil,
        from: nil,
        for: nil
    )
}

struct GaryxThreadHistoryLoadingView: View {
    var body: some View {
        HStack(spacing: 8) {
            ProgressView()
                .scaleEffect(0.76)
            Text("Loading thread")
                .font(GaryxFont.callout())
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 24)
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

private struct GaryxConversationViewportHeightKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

struct GaryxConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @FocusState private var isComposerFocused: Bool
    @State private var scrollPreservationThreadId: String?
    @State private var pendingHistoryPrefetchThreadId: String?
    @State private var conversationTopOffset: CGFloat?
    @State private var conversationBottomOffset: CGFloat = 0
    @State private var conversationViewportHeight: CGFloat = 0
    @State private var isNearConversationBottom = true
    @State private var hasMovedTowardOlderHistory = false
    @State private var showsScrollToBottomButton = false
    @State private var tailScrollRequestGeneration = 0

    var body: some View {
        ScrollViewReader { proxy in
            ZStack(alignment: .bottomTrailing) {
                messageScroll(proxy: proxy)
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                VStack(alignment: .trailing, spacing: 8) {
                    if showsScrollToBottomButton {
                        Button {
                            withAnimation(.easeOut(duration: 0.2)) {
                                scrollToConversationTail(proxy)
                            }
                            showsScrollToBottomButton = false
                        } label: {
                            Image(systemName: "arrow.down")
                                .font(GaryxFont.system(size: 15, weight: .semibold))
                                .foregroundStyle(.primary)
                                .frame(width: 42, height: 42)
                                .garyxAdaptiveGlass(
                                    .regular,
                                    isInteractive: true,
                                    fallbackMaterial: .ultraThinMaterial,
                                    in: Circle()
                                )
                                .shadow(color: Color.black.opacity(0.12), radius: 14, x: 0, y: 8)
                        }
                        .buttonStyle(.plain)
                        .padding(.trailing, 18)
                        .transition(.scale(scale: 0.88).combined(with: .opacity))
                        .accessibilityLabel("Scroll to latest message")
                    }

                    GaryxComposer(isFocused: $isComposerFocused)
                }
                .frame(maxWidth: .infinity, alignment: .trailing)
            }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onAppear {
                    isNearConversationBottom = true
                    hasMovedTowardOlderHistory = false
                    showsScrollToBottomButton = false
                    scheduleScrollToConversationTail(proxy, animated: false, retryLayout: true)
                }
                .onChange(of: model.selectedThread?.id) { _, _ in
                    scrollPreservationThreadId = model.selectedThread?.id
                    pendingHistoryPrefetchThreadId = nil
                    conversationTopOffset = nil
                    conversationBottomOffset = 0
                    isNearConversationBottom = true
                    hasMovedTowardOlderHistory = false
                    showsScrollToBottomButton = false
                    scheduleScrollToConversationTail(proxy, animated: false, retryLayout: true)
                }
                .onChange(of: model.messages) { oldValue, newValue in
                    defer {
                        prefetchOlderHistoryIfNeeded()
                    }
                    if shouldPreserveScrollForPrependedHistory(oldValue: oldValue, newValue: newValue) {
                        return
                    }
                    guard !newValue.isEmpty || model.showsTailThinkingIndicator else { return }
                    if oldValue.isEmpty || isNearConversationBottom {
                        scheduleScrollToConversationTail(proxy, animated: !oldValue.isEmpty, retryLayout: true)
                        showsScrollToBottomButton = false
                    } else {
                        showsScrollToBottomButton = true
                    }
                }
                .onChange(of: isComposerFocused) { _, isFocused in
                    guard isFocused else { return }
                    scheduleScrollToConversationTail(proxy, animated: true, retryLayout: true)
                }
        }
        .background(GaryxTheme.background)
        .garyxAdaptiveTopBar {
            GaryxConversationHeader()
        }
    }

    private func messageScroll(proxy: ScrollViewProxy) -> some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
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
                        .padding(.top, 96)
                } else if model.messages.isEmpty {
                    if model.showsTailThinkingIndicator {
                        GaryxThinkingLabel()
                            .padding(.top, 96)
                    } else if model.selectedThread != nil {
                        GaryxSelectedThreadEmptyConversationView()
                            .padding(.top, 96)
                    } else {
                        GaryxEmptyConversationView()
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
                        forceRunningLastTurn: model.isSelectedThreadSending,
                        prefetchBoundaryRowCount: garyxHistoryPrefetchBoundaryRows
                    ) {
                        prefetchOlderHistoryIfNeeded(ignoreDistance: true)
                    }
                    if model.showsTailThinkingIndicator {
                        GaryxThinkingLabel()
                            .id(tailThinkingAnchorId)
                    }
                }
                Color.clear
                    .frame(height: 1)
                    .id(conversationBottomAnchorId)
                    .background {
                        GeometryReader { geometry in
                            Color.clear.preference(
                                key: GaryxConversationBottomOffsetKey.self,
                                value: geometry.frame(in: .named("garyx-conversation-scroll")).maxY
                            )
                        }
                    }
            }
            .padding(.horizontal, 16)
            .padding(.top, 18)
            .padding(.bottom, 24)
        }
        .id(conversationScrollIdentity)
        .coordinateSpace(name: "garyx-conversation-scroll")
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background {
            GeometryReader { geometry in
                Color.clear.preference(
                    key: GaryxConversationViewportHeightKey.self,
                    value: geometry.size.height
                )
            }
        }
        .onPreferenceChange(GaryxConversationBottomOffsetKey.self) { value in
            conversationBottomOffset = value
            updateConversationBottomState()
            repairVisibleTailGapIfNeeded(proxy)
        }
        .onPreferenceChange(GaryxConversationTopOffsetKey.self) { value in
            conversationTopOffset = value
            if let value, value < -96 {
                hasMovedTowardOlderHistory = true
            }
            prefetchOlderHistoryIfNeeded()
            repairVisibleTailGapIfNeeded(proxy)
        }
        .onPreferenceChange(GaryxConversationViewportHeightKey.self) { value in
            conversationViewportHeight = value
            updateConversationBottomState()
            prefetchOlderHistoryIfNeeded()
            repairVisibleTailGapIfNeeded(proxy)
        }
        .scrollDisabled(isComposerFocused)
        .scrollDismissesKeyboard(.never)
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

    private func scrollToConversationTail(_ proxy: ScrollViewProxy) {
        if model.showsTailThinkingIndicator {
            proxy.scrollTo(tailThinkingAnchorId, anchor: .bottom)
        } else {
            proxy.scrollTo(conversationBottomAnchorId, anchor: .bottom)
        }
    }

    private func scheduleScrollToConversationTail(_ proxy: ScrollViewProxy, animated: Bool, retryLayout: Bool = false) {
        tailScrollRequestGeneration += 1
        let generation = tailScrollRequestGeneration
        let identity = conversationScrollIdentity
        let delays: [DispatchTimeInterval] = retryLayout
            ? [.milliseconds(0), .milliseconds(40), .milliseconds(140), .milliseconds(320)]
            : [.milliseconds(0)]

        for (index, delay) in delays.enumerated() {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                DispatchQueue.main.async {
                    guard generation == tailScrollRequestGeneration,
                          identity == conversationScrollIdentity,
                          index == 0 || isNearConversationBottom || shouldRepairVisibleTailGap else {
                        return
                    }
                    let shouldAnimate = animated && index == 0
                    if shouldAnimate {
                        withAnimation(.easeOut(duration: 0.2)) {
                            scrollToConversationTail(proxy)
                        }
                    } else {
                        scrollToConversationTail(proxy)
                    }
                    showsScrollToBottomButton = false
                }
            }
        }
    }

    private func repairVisibleTailGapIfNeeded(_ proxy: ScrollViewProxy) {
        guard shouldRepairVisibleTailGap else { return }
        scheduleScrollToConversationTail(proxy, animated: false, retryLayout: true)
    }

    private var shouldRepairVisibleTailGap: Bool {
        guard conversationViewportHeight > 0,
              let conversationTopOffset,
              isNearConversationBottom,
              !model.messages.isEmpty || model.showsTailThinkingIndicator else {
            return false
        }
        let contentBottomIsAboveViewportBottom = conversationBottomOffset < conversationViewportHeight - 96
        let contentTopIsScrolledAboveViewport = conversationTopOffset < -96
        return contentBottomIsAboveViewportBottom && contentTopIsScrolledAboveViewport
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

    private func updateConversationBottomState() {
        guard conversationViewportHeight > 0 else { return }
        let distanceFromBottom = conversationBottomOffset - conversationViewportHeight
        let isNearBottom = distanceFromBottom <= 96
        if !isNearBottom {
            hasMovedTowardOlderHistory = true
        }
        isNearConversationBottom = isNearBottom
        showsScrollToBottomButton = !isNearBottom && !model.messages.isEmpty
        prefetchOlderHistoryIfNeeded()
    }

    private func prefetchOlderHistoryIfNeeded(ignoreDistance: Bool = false) {
        guard let threadId = model.selectedThread?.id,
              model.selectedThreadHasMoreHistoryBefore,
              !model.isLoadingOlderThreadHistory,
              pendingHistoryPrefetchThreadId != threadId,
              hasMovedTowardOlderHistory,
              ignoreDistance || isNearLoadedHistoryStart else {
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

    private var isNearLoadedHistoryStart: Bool {
        guard let conversationTopOffset,
              conversationViewportHeight > 0 else {
            return false
        }
        let prefetchDistance = max(
            garyxHistoryPrefetchMinDistance,
            conversationViewportHeight * garyxHistoryPrefetchViewportMultiplier
        )
        return conversationTopOffset >= -prefetchDistance
    }

    private func shouldPreserveScrollForPrependedHistory(
        oldValue: [GaryxMobileMessage],
        newValue: [GaryxMobileMessage]
    ) -> Bool {
        let threadId = model.selectedThread?.id
        defer { scrollPreservationThreadId = threadId }
        guard threadId == scrollPreservationThreadId,
              newValue.count > oldValue.count,
              let oldFirstId = oldValue.first?.id,
              newValue.first?.id != oldFirstId,
              let oldFirstIndex = newValue.firstIndex(where: { $0.id == oldFirstId }) else {
            return false
        }
        return oldFirstIndex > 0
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

                Menu {
                    if let selectedThread = model.selectedThread {
                        Section("Bot") {
                            Button(
                                model.selectedThreadBotGroup == nil ? "Bind Bot" : "Change Bot",
                                systemImage: model.selectedThreadBotGroup == nil ? "link.badge.plus" : "arrow.triangle.2.circlepath"
                            ) {
                                botBindingThreadId = selectedThread.id
                                showsBotBindingSheet = true
                            }
                            .disabled(model.mobileBotGroups.isEmpty)

                            if let boundGroup = model.selectedThreadBotGroup,
                               let configuredBot = garyxConfiguredBot(for: boundGroup, in: model.configuredBots) {
                                Button("Unbind \(boundGroup.title)", systemImage: "link.badge.minus", role: .destructive) {
                                    Task { await model.unbindBot(configuredBot) }
                                }
                            }
                        }

                        Button(
                            model.isThreadPinned(selectedThread.id) ? "Unpin thread" : "Pin thread",
                            systemImage: model.isThreadPinned(selectedThread.id) ? "pin.slash" : "pin"
                        ) {
                            model.togglePinnedThread(selectedThread.id)
                        }
                        if model.selectedThreadTask == nil {
                            Button("Promote to Task", systemImage: "checklist") {
                                Task { await model.promoteSelectedThreadToTask() }
                            }
                        } else {
                            Button("View Task", systemImage: "checklist") {
                                model.openPanel(.tasks)
                            }
                        }
                        Button("Rename", systemImage: "pencil") {
                            openRenamePrompt()
                        }
                        Button("Archive", systemImage: "archivebox", role: .destructive) {
                            Task { await model.deleteSelectedThread() }
                        }
                    }
                } label: {
                    GaryxToolbarIcon(systemName: "ellipsis")
                }
                .buttonStyle(.plain)
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
    }

    private func openRenamePrompt() {
        renameDraftTitle = model.selectedThread?.title ?? model.draftThreadTitle
        showsRenamePrompt = true
    }

    private func openSidebar() {
        garyxDismissKeyboard()
        model.setSidebarVisible(true)
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
                            if let boundBot {
                                botOptionRow(
                                    title: "No bot",
                                    subtitle: "Unbind this thread from \(boundGroup?.title ?? "the current bot")",
                                    channel: boundBot.channel,
                                    iconDataUrl: nil,
                                    systemName: "link.badge.minus",
                                    isSelected: false,
                                    role: .destructive,
                                    isDestructive: true
                                ) {
                                    apply {
                                        await model.unbindBot(boundBot)
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
            }
            .scrollIndicators(.hidden)
        }
        .garyxBotBindingSheetStyle()
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
        role: ButtonRole? = nil,
        isDestructive: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(role: role, action: action) {
            HStack(spacing: 12) {
                if iconDataUrl?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false {
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
                    }

                    if !displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        GaryxMarkdownText(
                            text: displayText,
                            foreground: .primary,
                            codeBackground: userCodeBackground,
                            codeBorder: GaryxTheme.hairline,
                            fillsAvailableWidth: false,
                            onFileLinkTap: openMessageFileLink
                        )
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(userBubbleBackground, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
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
                }
                if message.isStreaming && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    if message.attachments.isEmpty {
                        GaryxThinkingLabel()
                    }
                } else if !displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    GaryxMarkdownText(
                        text: displayText,
                        foreground: .primary,
                        onFileLinkTap: openMessageFileLink
                    )
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case .system:
            GaryxMarkdownText(
                text: displayText,
                foreground: .secondary,
                fillsAvailableWidth: false,
                onFileLinkTap: openMessageFileLink
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
    let attachment: GaryxMobileMessageAttachment
    let isUser: Bool

    @State private var decodedImage: UIImage?
    @State private var decodedImageKey: String?
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
                    dataUrl: attachment.dataUrl,
                    remoteUrl: attachment.remoteUrl,
                    filePath: Self.localFilePath(from: attachment.path)
                )
            ) {
                showsPreview = false
            }
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
        return "\(attachment.id):\(raw.count):\(raw.hashValue)"
    }

    @MainActor
    private func updateDecodedImage() async {
        let key = dataUrlDecodeKey
        guard decodedImageKey != key else { return }
        decodedImage = nil
        decodedImageKey = key
        guard let raw = attachment.dataUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
              !raw.isEmpty else { return }
        let image = await Task.detached(priority: .utility) {
            GaryxImageDecoder.image(fromDataUrl: raw, maxPixelSize: 520)
        }.value
        guard !Task.isCancelled, decodedImageKey == key else { return }
        decodedImage = image
    }

    private var remoteURL: URL? {
        guard let raw = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
              raw.hasPrefix("http://") || raw.hasPrefix("https://") else {
            return nil
        }
        return URL(string: raw)
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
        .accessibilityLabel(attachment.name.isEmpty ? "File attachment" : attachment.name)
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
