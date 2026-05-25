import Foundation
import ImageIO
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

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
    @State private var conversationBottomOffset: CGFloat = 0
    @State private var conversationViewportHeight: CGFloat = 0
    @State private var isNearConversationBottom = true
    @State private var showsScrollToBottomButton = false

    var body: some View {
        ScrollViewReader { proxy in
            ZStack(alignment: .bottomTrailing) {
                messageScroll

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
                    .padding(.bottom, 14)
                    .transition(.scale(scale: 0.88).combined(with: .opacity))
                    .accessibilityLabel("Scroll to latest message")
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                GaryxComposer(isFocused: $isComposerFocused)
            }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onAppear {
                    isNearConversationBottom = true
                    showsScrollToBottomButton = false
                    scheduleScrollToConversationTail(proxy, animated: false)
                }
                .onChange(of: model.selectedThread?.id) { _, _ in
                    scrollPreservationThreadId = model.selectedThread?.id
                    isNearConversationBottom = true
                    showsScrollToBottomButton = false
                    scheduleScrollToConversationTail(proxy, animated: false)
                }
                .onChange(of: model.messages) { oldValue, newValue in
                    if shouldPreserveScrollForPrependedHistory(oldValue: oldValue, newValue: newValue) {
                        return
                    }
                    guard !newValue.isEmpty || model.showsTailThinkingIndicator else { return }
                    if oldValue.isEmpty || isNearConversationBottom {
                        withAnimation(.easeOut(duration: 0.2)) {
                            scrollToConversationTail(proxy)
                        }
                        showsScrollToBottomButton = false
                    } else {
                        showsScrollToBottomButton = true
                    }
                }
                .onChange(of: isComposerFocused) { _, isFocused in
                    guard isFocused else { return }
                    withAnimation(.easeOut(duration: 0.2)) {
                        scrollToConversationTail(proxy)
                    }
                }
        }
        .background(GaryxTheme.background)
        .garyxAdaptiveTopBar {
            GaryxConversationHeader()
        }
    }

    private var messageScroll: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                if model.messages.isEmpty, model.isLoadingSelectedThreadHistory {
                    GaryxThreadHistoryLoadingView()
                        .padding(.top, 96)
                } else if model.messages.isEmpty {
                    if model.showsTailThinkingIndicator {
                        GaryxThinkingLabel()
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
                    }
                    GaryxMobileTurnRowsView(
                        rows: model.selectedThreadTurnRows(),
                        forceRunningLastTurn: model.isSelectedThreadSending
                    )
                    if model.showsTailThinkingIndicator {
                        GaryxThinkingLabel()
                            .id("tail-thinking")
                    }
                }
                Color.clear
                    .frame(height: 1)
                    .id("conversation-bottom-anchor")
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
        }
        .onPreferenceChange(GaryxConversationViewportHeightKey.self) { value in
            conversationViewportHeight = value
            updateConversationBottomState()
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
            proxy.scrollTo("tail-thinking", anchor: .bottom)
        } else {
            proxy.scrollTo("conversation-bottom-anchor", anchor: .bottom)
        }
    }

    private func scheduleScrollToConversationTail(_ proxy: ScrollViewProxy, animated: Bool) {
        DispatchQueue.main.async {
            DispatchQueue.main.async {
                if animated {
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

    private func updateConversationBottomState() {
        guard conversationViewportHeight > 0 else { return }
        let distanceFromBottom = conversationBottomOffset - conversationViewportHeight
        let isNearBottom = distanceFromBottom <= 96
        isNearConversationBottom = isNearBottom
        showsScrollToBottomButton = !isNearBottom && !model.messages.isEmpty
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

private struct GaryxHeaderAgentControl: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsAgentPicker = false

    var body: some View {
        if model.selectedThread == nil {
            Button {
                showsAgentPicker = true
            } label: {
                GaryxAgentPickerLabel(
                    target: model.selectedAgentTarget,
                    title: model.selectedAgentLabel,
                    showsChevron: true,
                    style: .prominent
                )
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Agent")
            .popover(
                isPresented: $showsAgentPicker,
                attachmentAnchor: .rect(.bounds),
                arrowEdge: .top
            ) {
                GaryxAgentPickerPopover()
                    .environmentObject(model)
                    .presentationCompactAdaptation(.popover)
            }
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
}

private struct GaryxAgentPickerPopover: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if model.agentTargets.isEmpty {
                Text("No agents available")
                    .font(GaryxFont.callout())
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 18)
                    .padding(.vertical, 16)
            } else {
                Text("Latest")
                    .font(GaryxFont.footnote(weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 20)
                    .padding(.top, 16)
                    .padding(.bottom, 8)

                if model.agentTargets.count <= 5 {
                    ForEach(model.agentTargets) { target in
                        agentRow(for: target)
                    }
                } else {
                    if !agentTargets.isEmpty {
                        ForEach(agentTargets) { target in
                            agentRow(for: target)
                        }
                    }

                    if !teamTargets.isEmpty {
                        Divider()
                            .padding(.horizontal, 20)
                            .padding(.vertical, 8)

                        ForEach(teamTargets) { target in
                            agentRow(for: target)
                        }
                    }
                }
            }

            Divider()
                .padding(.horizontal, 18)
                .padding(.vertical, 8)

            Button {
                dismiss()
                model.openPanel(.agents)
            } label: {
                HStack(spacing: 14) {
                    Image(systemName: "slider.horizontal.3")
                        .font(GaryxFont.system(size: 17, weight: .semibold))
                        .frame(width: 30)

                    Text("Configure")
                        .font(GaryxFont.callout(weight: .medium))

                    Spacer(minLength: 0)
                }
                .foregroundStyle(.primary)
                .frame(height: 48)
                .padding(.horizontal, 20)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .frame(width: 308)
        .background(.regularMaterial)
    }

    private var agentTargets: [GaryxMobileAgentTarget] {
        model.agentTargets.filter { $0.kind == .agent }
    }

    private var teamTargets: [GaryxMobileAgentTarget] {
        model.agentTargets.filter { $0.kind == .team }
    }

    private func agentRow(for target: GaryxMobileAgentTarget) -> some View {
        Button {
            model.setSelectedAgentTarget(target.id)
            dismiss()
        } label: {
            HStack(spacing: 14) {
                Group {
                    if model.selectedAgentTargetId == target.id {
                        Image(systemName: "checkmark")
                            .font(GaryxFont.system(size: 18, weight: .semibold))
                            .foregroundStyle(.primary)
                    } else {
                        Color.clear
                    }
                }
                .frame(width: 30)

                GaryxAgentAvatarView(
                    agentId: target.id,
                    avatarDataUrl: target.avatarDataUrl,
                    kind: target.kind,
                    label: target.title,
                    providerType: target.providerType,
                    builtIn: target.builtIn,
                    diameter: 30
                )

                VStack(alignment: .leading, spacing: 2) {
                    Text(target.title)
                        .font(GaryxFont.callout(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)

                    if !target.subtitle.isEmpty {
                        Text(target.subtitle)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }

                Spacer(minLength: 0)
            }
            .frame(height: 54)
            .padding(.horizontal, 20)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct GaryxAgentPickerLabel: View {
    enum Style {
        case prominent
        case compact
    }

    let target: GaryxMobileAgentTarget?
    let title: String
    let showsChevron: Bool
    var style: Style = .prominent

    var body: some View {
        HStack(spacing: horizontalSpacing) {
            if let target {
                GaryxAgentAvatarView(
                    agentId: target.id,
                    avatarDataUrl: target.avatarDataUrl,
                    kind: target.kind,
                    label: target.title,
                    providerType: target.providerType,
                    builtIn: target.builtIn,
                    diameter: avatarDiameter
                )
            } else {
                Image(systemName: "person.crop.circle")
                    .font(GaryxFont.system(size: fallbackIconSize, weight: .semibold))
                    .foregroundStyle(.secondary)
            }

            Text(title.isEmpty ? "Agent" : title)
                .font(labelFont)
                .foregroundStyle(labelForeground)
                .lineLimit(1)
                .truncationMode(.tail)
                .minimumScaleFactor(0.8)
                .layoutPriority(1)

            if showsChevron {
                Image(systemName: "chevron.down")
                    .font(GaryxFont.system(size: chevronSize, weight: .bold))
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, horizontalPadding)
        .frame(height: labelHeight, alignment: .leading)
        .if(isProminent) { view in
            view.background {
                Capsule()
                    .fill(Color(.systemBackground).opacity(0.42))
                    .background(.ultraThinMaterial, in: Capsule())
            }
        }
        .overlay {
            Capsule()
                .stroke(Color.primary.opacity(isProminent ? 0.03 : 0), lineWidth: 1)
        }
        .contentShape(Capsule())
    }

    private var avatarDiameter: CGFloat {
        switch style {
        case .prominent:
            29
        case .compact:
            16
        }
    }

    private var fallbackIconSize: CGFloat {
        switch style {
        case .prominent:
            22
        case .compact:
            13
        }
    }

    private var chevronSize: CGFloat {
        switch style {
        case .prominent:
            10
        case .compact:
            8
        }
    }

    private var horizontalSpacing: CGFloat {
        switch style {
        case .prominent:
            8
        case .compact:
            6
        }
    }

    private var horizontalPadding: CGFloat {
        switch style {
        case .prominent:
            12
        case .compact:
            0
        }
    }

    private var labelHeight: CGFloat {
        switch style {
        case .prominent:
            44
        case .compact:
            19
        }
    }

    private var labelFont: Font {
        switch style {
        case .prominent:
            GaryxFont.body(weight: .semibold)
        case .compact:
            GaryxFont.caption(weight: .semibold)
        }
    }

    private var labelForeground: Color {
        switch style {
        case .prominent:
            .primary
        case .compact:
            .secondary
        }
    }

    private var isProminent: Bool {
        switch style {
        case .prominent:
            true
        case .compact:
            false
        }
    }
}

struct GaryxEmptyConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsWorkspaceEditor = false
    @State private var workspacePathDraft = ""

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
        .fullScreenCover(isPresented: $showsWorkspaceEditor) {
            GaryxFormSheet(title: "Workspace") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Directory")
                    TextField("Workspace path", text: $workspacePathDraft)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    Button {
                        model.setNewThreadWorkspace(workspacePathDraft)
                        showsWorkspaceEditor = false
                    } label: {
                        Label("Use Directory", systemImage: "folder")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
    }

    private var workspacePicker: some View {
        let workspaces = model.knownWorkspacePaths.filter(GaryxMobileModel.isVisibleMobileWorkspacePath)
        return Menu {
            Button {
                model.setNewThreadWorkspace("")
            } label: {
                Label("No workspace", systemImage: "minus.circle")
            }
            if !workspaces.isEmpty {
                Divider()
                ForEach(workspaces, id: \.self) { path in
                    Button {
                        model.setNewThreadWorkspace(path)
                    } label: {
                        Label(workspaceMenuLabel(for: path), systemImage: "folder")
                    }
                }
            }
            Divider()
            Button {
                workspacePathDraft = model.newThreadWorkspace
                showsWorkspaceEditor = true
            } label: {
                Label("Edit path", systemImage: "pencil")
            }
        } label: {
            HStack(spacing: 10) {
                Text(model.newThreadWorkspaceLabel)
                    .font(GaryxFont.body(weight: .semibold))
                    .lineLimit(1)
                if !model.newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(model.newThreadWorkspace)
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(Color(.systemBackground).opacity(0.76))
                        .lineLimit(1)
                }
                Image(systemName: "chevron.down")
                    .font(GaryxFont.system(size: 10, weight: .bold))
            }
            .foregroundStyle(Color(.systemBackground))
            .padding(.horizontal, 18)
            .frame(height: 46)
            .background(Color(.label), in: Capsule())
        }
        .buttonStyle(.plain)
    }

    private func workspaceMenuLabel(for path: String) -> String {
        let name = (path as NSString).lastPathComponent
        return name.isEmpty ? path : name
    }
}

struct GaryxMessageBubble: View {
    let message: GaryxMobileMessage
    @Environment(\.colorScheme) private var colorScheme
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var retrying = false

    var body: some View {
        Group {
            if let group = message.toolTraceGroup {
                GaryxToolTraceGroupView(group: group)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                messageRow
            }
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
                            fillsAvailableWidth: false
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
                    GaryxMarkdownText(text: displayText, foreground: .primary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case .system:
            GaryxMarkdownText(text: displayText, foreground: .secondary, fillsAvailableWidth: false)
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

    var body: some View {
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
        .accessibilityLabel(attachment.name.isEmpty ? "Image attachment" : attachment.name)
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
            Self.decodedImage(from: raw, maxPixelSize: 520)
        }.value
        guard !Task.isCancelled, decodedImageKey == key else { return }
        decodedImage = image
    }

    nonisolated private static func decodedImage(from raw: String, maxPixelSize: CGFloat) -> UIImage? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let encoded = trimmed.split(separator: ",", maxSplits: 1).last.map(String.init) ?? trimmed
        guard let data = Data(base64Encoded: encoded) else { return nil }
        let options = [kCGImageSourceShouldCache: false] as CFDictionary
        guard let source = CGImageSourceCreateWithData(data as CFData, options) else {
            return UIImage(data: data)
        }
        let thumbnailOptions: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: Int(maxPixelSize),
        ]
        let optionsDictionary = thumbnailOptions as CFDictionary
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, optionsDictionary) else {
            return UIImage(data: data)
        }
        return UIImage(cgImage: image)
    }

    private var remoteURL: URL? {
        guard let raw = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
              raw.hasPrefix("http://") || raw.hasPrefix("https://") else {
            return nil
        }
        return URL(string: raw)
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

struct GaryxMarkdownText: View {
    let text: String
    var foreground: Color = .primary
    var codeBackground: Color = GaryxTheme.surface
    var codeBorder: Color = GaryxTheme.hairline
    var fillsAvailableWidth = true

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(GaryxMarkdownRenderCache.shared.blocks(from: text)) { block in
                switch block.kind {
                case .markdown(let markdown):
                    GaryxMarkdownParagraphView(markdown: markdown, foreground: foreground)
                case .code(let language, let code):
                    GaryxCodeBlockView(
                        language: language,
                        code: code,
                        foreground: foreground,
                        background: codeBackground,
                        border: codeBorder,
                        fillsAvailableWidth: fillsAvailableWidth
                    )
                }
            }
        }
        .frame(maxWidth: fillsAvailableWidth ? .infinity : nil, alignment: .leading)
    }

    fileprivate static func attributedString(from markdown: String) -> AttributedString {
        GaryxMarkdownRenderCache.shared.attributedString(from: markdown)
    }
}

private struct GaryxMarkdownParagraphView: View {
    let markdown: String
    let foreground: Color

    private var lines: [String] {
        markdown.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                if line.trimmingCharacters(in: .whitespaces).isEmpty {
                    Color.clear.frame(height: 8)
                } else if let bullet = Self.bulletText(from: line) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Circle()
                            .fill(foreground)
                            .frame(width: 4, height: 4)
                            .offset(y: -2)

                        Text(GaryxMarkdownText.attributedString(from: bullet))
                            .font(GaryxFont.body())
                            .foregroundStyle(foreground)
                            .tint(GaryxTheme.accent)
                            .textSelection(.enabled)
                            .lineSpacing(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                } else if let numbered = Self.numberedList(from: line) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(numbered.label)
                            .font(GaryxFont.body(weight: .medium))
                            .foregroundStyle(foreground)
                            .textSelection(.enabled)

                        Text(GaryxMarkdownText.attributedString(from: numbered.text))
                            .font(GaryxFont.body())
                            .foregroundStyle(foreground)
                            .tint(GaryxTheme.accent)
                            .textSelection(.enabled)
                            .lineSpacing(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                } else {
                    Text(GaryxMarkdownText.attributedString(from: line))
                        .font(GaryxFont.body())
                        .foregroundStyle(foreground)
                        .tint(GaryxTheme.accent)
                        .textSelection(.enabled)
                        .lineSpacing(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    private static func bulletText(from line: String) -> String? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        if trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ") {
            return String(trimmed.dropFirst(2))
        }
        return nil
    }

    private static func numberedList(from line: String) -> (label: String, text: String)? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        var digitPrefix = ""
        var cursor = trimmed.startIndex
        while cursor < trimmed.endIndex, trimmed[cursor].isNumber {
            digitPrefix.append(trimmed[cursor])
            cursor = trimmed.index(after: cursor)
        }
        guard !digitPrefix.isEmpty, cursor < trimmed.endIndex, trimmed[cursor] == "." else {
            return nil
        }
        let afterDot = trimmed.index(after: cursor)
        guard afterDot < trimmed.endIndex, trimmed[afterDot] == " " else {
            return nil
        }
        let textStart = trimmed.index(after: afterDot)
        return ("\(digitPrefix).", String(trimmed[textStart...]))
    }
}

private struct GaryxCodeBlockView: View {
    let language: String?
    let code: String
    let foreground: Color
    let background: Color
    let border: Color
    let fillsAvailableWidth: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let language, !language.isEmpty {
                Text(language)
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 10)
                    .padding(.top, 8)
                    .padding(.bottom, 4)
            }

            ScrollView(.horizontal, showsIndicators: false) {
                Text(code.isEmpty ? " " : code)
                    .font(.system(size: 12.5, weight: .regular, design: .monospaced))
                    .foregroundStyle(foreground)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: true, vertical: true)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
            }
        }
        .frame(maxWidth: fillsAvailableWidth ? .infinity : nil, alignment: .leading)
        .background(background, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(border, lineWidth: 1)
        }
    }
}

private struct GaryxMarkdownBlock: Identifiable {
    enum Kind {
        case markdown(String)
        case code(language: String?, text: String)
    }

    let id: Int
    let kind: Kind

    static func blocks(from text: String) -> [GaryxMarkdownBlock] {
        var blocks: [GaryxMarkdownBlock] = []
        var markdownLines: [String] = []
        var codeLines: [String] = []
        var codeLanguage: String?
        var insideFence = false

        func appendMarkdown() {
            let value = markdownLines.joined(separator: "\n")
            markdownLines.removeAll(keepingCapacity: true)
            guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
            blocks.append(GaryxMarkdownBlock(id: blocks.count, kind: .markdown(value)))
        }

        func appendCode() {
            let value = codeLines.joined(separator: "\n")
            codeLines.removeAll(keepingCapacity: true)
            guard !value.isEmpty else { return }
            blocks.append(GaryxMarkdownBlock(id: blocks.count, kind: .code(language: codeLanguage, text: value)))
            codeLanguage = nil
        }

        for line in text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) {
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.hasPrefix("```") {
                if insideFence {
                    appendCode()
                    insideFence = false
                } else {
                    appendMarkdown()
                    insideFence = true
                    let language = String(trimmed.dropFirst(3)).trimmingCharacters(in: .whitespacesAndNewlines)
                    codeLanguage = language.isEmpty ? nil : language
                }
                continue
            }

            if insideFence {
                codeLines.append(line)
            } else {
                markdownLines.append(line)
            }
        }

        if insideFence {
            appendCode()
        }
        appendMarkdown()

        if blocks.isEmpty {
            blocks.append(GaryxMarkdownBlock(id: 0, kind: .markdown(text)))
        }
        return blocks
    }
}

private final class GaryxMarkdownRenderCache {
    static let shared = GaryxMarkdownRenderCache()

    private let maxCacheableBlockBytes = 16 * 1024
    private let maxCacheableAttributedBytes = 8 * 1024
    private let blockCache: NSCache<NSString, GaryxMarkdownBlockCacheEntry>
    private let attributedCache: NSCache<NSString, GaryxMarkdownAttributedCacheEntry>
    private let attributedOptions = AttributedString.MarkdownParsingOptions(
        interpretedSyntax: .full,
        failurePolicy: .returnPartiallyParsedIfPossible
    )

    private init() {
        let blockCache = NSCache<NSString, GaryxMarkdownBlockCacheEntry>()
        blockCache.countLimit = 256
        blockCache.totalCostLimit = 2 * 1024 * 1024
        self.blockCache = blockCache

        let attributedCache = NSCache<NSString, GaryxMarkdownAttributedCacheEntry>()
        attributedCache.countLimit = 1_024
        attributedCache.totalCostLimit = 4 * 1024 * 1024
        self.attributedCache = attributedCache
    }

    func blocks(from text: String) -> [GaryxMarkdownBlock] {
        let byteCount = text.utf8.count
        guard byteCount <= maxCacheableBlockBytes else {
            return GaryxMarkdownBlock.blocks(from: text)
        }
        let key = NSString(string: text)
        if let cached = blockCache.object(forKey: key) {
            return cached.blocks
        }
        let blocks = GaryxMarkdownBlock.blocks(from: text)
        blockCache.setObject(GaryxMarkdownBlockCacheEntry(blocks: blocks), forKey: key, cost: max(1, byteCount))
        return blocks
    }

    func attributedString(from markdown: String) -> AttributedString {
        let byteCount = markdown.utf8.count
        guard byteCount <= maxCacheableAttributedBytes else {
            return (try? AttributedString(markdown: markdown, options: attributedOptions)) ?? AttributedString(markdown)
        }
        let key = NSString(string: markdown)
        if let cached = attributedCache.object(forKey: key) {
            return cached.value
        }
        let value = (try? AttributedString(markdown: markdown, options: attributedOptions)) ?? AttributedString(markdown)
        attributedCache.setObject(GaryxMarkdownAttributedCacheEntry(value: value), forKey: key, cost: max(1, byteCount))
        return value
    }
}

private final class GaryxMarkdownBlockCacheEntry {
    let blocks: [GaryxMarkdownBlock]

    init(blocks: [GaryxMarkdownBlock]) {
        self.blocks = blocks
    }
}

private final class GaryxMarkdownAttributedCacheEntry {
    let value: AttributedString

    init(value: AttributedString) {
        self.value = value
    }
}

private enum GaryxComposerLayout {
    static let composerCornerRadius: CGFloat = 26
    static let composerSpacing: CGFloat = 6
    static let bottomBarSpacing: CGFloat = 12
    static let bottomBarHorizontalPadding: CGFloat = 8
    static let bottomBarTopPadding: CGFloat = 2
    static let bottomBarBottomPadding: CGFloat = 8
    static let bottomBarIconSide: CGFloat = 22
    static let inputHorizontalPadding: CGFloat = 16
    static let inputTopPadding: CGFloat = 12
    static let inputBottomPadding: CGFloat = 8
    static let draftFieldIdentity = "garyx-composer-draft-field"
}


struct GaryxComposer: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let isFocused: FocusState<Bool>.Binding
    @State private var draftText = ""
    @State private var draftContextVersion = 0
    @State private var isPickingAttachments = false
    @State private var isPickingPhotos = false
    @State private var selectedPhotoItems: [PhotosPickerItem] = []

    private var hasLocalPayload: Bool {
        !draftText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !model.composerAttachments.isEmpty
    }

    private var canSendLocalPayload: Bool {
        model.canSendComposerPayload(text: draftText, attachments: model.composerAttachments)
    }

    private var showsSendButton: Bool {
        !model.isSelectedThreadSending || hasLocalPayload
    }

    var body: some View {
        GaryxAdaptiveGlassContainer(spacing: GaryxComposerLayout.composerSpacing) {
            composerCard
        }
        .padding(.horizontal, 12)
        .padding(.top, 10)
        .padding(.bottom, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.clear)
        .animation(.spring(response: 0.24, dampingFraction: 0.88), value: model.composerAttachments)
        .fileImporter(
            isPresented: $isPickingAttachments,
            allowedContentTypes: [.item],
            allowsMultipleSelection: true
        ) { result in
            switch result {
            case .success(let urls):
                Task { await model.attachFiles(from: urls) }
            case .failure(let error):
                model.lastError = error.localizedDescription
            }
        }
        .photosPicker(
            isPresented: $isPickingPhotos,
            selection: $selectedPhotoItems,
            maxSelectionCount: 10,
            matching: .images
        )
        .onChange(of: selectedPhotoItems) { _, items in
            guard !items.isEmpty else { return }
            Task {
                await attachPhotos(items)
                selectedPhotoItems = []
            }
        }
        .onAppear {
            draftContextVersion = model.composerContextVersion
            draftText = model.draft
        }
        .onChange(of: model.composerContextVersion) { _, newValue in
            draftContextVersion = newValue
            draftText = model.draft
        }
        .onChange(of: model.draft) { _, newValue in
            guard newValue != draftText else { return }
            draftText = newValue
        }
        .onDisappear {
            guard draftContextVersion == model.composerContextVersion else { return }
            if model.draft != draftText {
                model.draft = draftText
            }
        }
    }

    private var composerCard: some View {
        VStack(spacing: 0) {
            if !model.composerAttachments.isEmpty {
                composerAttachmentsPreview
            }

            composerInput
            composerBottomBar
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .garyxAdaptiveGlass(
            .regular,
            in: RoundedRectangle(cornerRadius: GaryxComposerLayout.composerCornerRadius, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: GaryxComposerLayout.composerCornerRadius, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }

    private var composerAttachmentsPreview: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(model.composerAttachments) { attachment in
                    GaryxAttachmentChip(attachment: attachment)
                }
            }
            .padding(.horizontal, GaryxComposerLayout.inputHorizontalPadding)
            .padding(.top, 8)
            .padding(.bottom, 4)
        }
    }

    private var composerInput: some View {
        ZStack(alignment: .topLeading) {
            if draftText.isEmpty {
                Text(placeholderText)
                    .font(GaryxFont.subheadline())
                    .foregroundStyle(Color(.placeholderText))
                    .padding(.top, 2)
                    .allowsHitTesting(false)
            }

            TextField("", text: $draftText, axis: .vertical)
                .id(GaryxComposerLayout.draftFieldIdentity)
                .font(GaryxFont.subheadline())
                .foregroundStyle(.primary)
                .focused(isFocused)
                .lineLimit(1...4)
                .submitLabel(.send)
                .onSubmit {
                    Task { await sendLocalDraft() }
                }
        }
        .frame(maxWidth: .infinity, minHeight: 34, alignment: .topLeading)
        .padding(.horizontal, GaryxComposerLayout.inputHorizontalPadding)
        .padding(.top, model.composerAttachments.isEmpty ? GaryxComposerLayout.inputTopPadding : 6)
        .padding(.bottom, GaryxComposerLayout.inputBottomPadding)
        .contentShape(Rectangle())
        .onTapGesture {
            isFocused.wrappedValue = true
        }
    }

    private var placeholderText: String {
        model.selectedThread == nil ? "Ask Garyx anything..." : "Ask for follow-up changes"
    }

    private var composerBottomBar: some View {
        HStack(spacing: GaryxComposerLayout.bottomBarSpacing) {
            addMenuButton

            Spacer(minLength: 0)

            if model.isSelectedThreadSending {
                Button {
                    Task { await model.interruptActiveRun() }
                } label: {
                    GaryxCircleBadge(
                        systemName: "stop.fill",
                        foreground: Color(.systemBackground),
                        background: Color(.label)
                    )
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Stop current run")
            }

            if showsSendButton {
                Button {
                    Task { await sendLocalDraft() }
                } label: {
                    GaryxCircleBadge(
                        systemName: "arrow.up",
                        foreground: canSendLocalPayload ? Color(.systemBackground) : Color(.systemGray2),
                        background: canSendLocalPayload ? Color(.label) : Color(.systemGray5)
                    )
                }
                .buttonStyle(.plain)
                .disabled(!canSendLocalPayload)
                .accessibilityLabel("Send")
            }
        }
        .padding(.horizontal, GaryxComposerLayout.bottomBarHorizontalPadding)
        .padding(.top, GaryxComposerLayout.bottomBarTopPadding)
        .padding(.bottom, GaryxComposerLayout.bottomBarBottomPadding)
    }

    private var addMenuButton: some View {
        Menu {
            if !model.slashCommands.isEmpty {
                Section("Commands") {
                    ForEach(Array(model.slashCommands.prefix(6))) { command in
                        Button {
                            insertSlashCommand(command)
                        } label: {
                            Label(command.name, systemImage: "command")
                        }
                    }
                }
            }

            if model.selectedThread == nil {
                Section("New Thread") {
                    Button {
                        model.setNewThreadWorkspaceMode("local")
                    } label: {
                        Label("Local workspace", systemImage: model.newThreadUsesWorktree ? "laptopcomputer" : "checkmark")
                    }

                    Button {
                        model.setNewThreadWorkspaceMode("worktree")
                    } label: {
                        Label("Worktree", systemImage: model.newThreadUsesWorktree ? "checkmark" : "arrow.triangle.branch")
                    }
                    .disabled(!model.newThreadWorkspaceCanUseWorktree)
                }
            }

            Section("Attach") {
                Button {
                    DispatchQueue.main.async {
                        isPickingPhotos = true
                    }
                } label: {
                    Label("Photo library", systemImage: "photo")
                }

                Button {
                    DispatchQueue.main.async {
                        isPickingAttachments = true
                    }
                } label: {
                    Label("File", systemImage: "doc")
                }
            }

            if model.selectedThread != nil, !model.mobileBotGroups.isEmpty {
                Section("Bots") {
                    if let boundGroup = model.selectedThreadBotGroup,
                       let configuredBot = configuredBot(for: boundGroup) {
                        Button(role: .destructive) {
                            Task { await model.unbindBot(configuredBot) }
                        } label: {
                            Label("Unbind \(boundGroup.title)", systemImage: "link.badge.minus")
                        }
                    }

                    ForEach(model.mobileBotGroups) { group in
                        if let configuredBot = configuredBot(for: group) {
                            Button {
                                Task { await model.bindBotToSelectedThread(configuredBot) }
                            } label: {
                                botMenuLabel(for: group)
                            }
                        }
                    }
                }
            }
        } label: {
            Image(systemName: "plus")
                .font(GaryxFont.system(size: 22, weight: .regular))
                .foregroundStyle(.primary)
                .frame(
                    width: GaryxComposerLayout.bottomBarIconSide,
                    height: GaryxComposerLayout.bottomBarIconSide
                )
                .contentShape(Capsule())
        }
        .tint(.secondary)
        .buttonStyle(.plain)
        .accessibilityLabel("Composer options")
    }

    private func insertSlashCommand(_ command: GaryxSlashCommand) {
        let normalizedName = command.name.hasPrefix("/") ? command.name : "/\(command.name)"
        draftText = normalizedName + " "
        model.draft = draftText
        isFocused.wrappedValue = true
    }

    private func sendLocalDraft() async {
        guard canSendLocalPayload else { return }
        let text = draftText
        draftText = ""
        let sent = await model.sendDraft(text: text)
        if !sent {
            draftText = text
        }
    }

    private func configuredBot(for group: GaryxMobileBotGroup) -> GaryxConfiguredBot? {
        model.configuredBots.first {
            $0.channel.caseInsensitiveCompare(group.channel) == .orderedSame
                && $0.accountId == group.accountId
        }
    }

    @ViewBuilder
    private func botMenuLabel(for group: GaryxMobileBotGroup) -> some View {
        if let image = Self.decodedMenuIcon(from: group.iconDataUrl) {
            Label {
                Text(group.title)
            } icon: {
                Image(uiImage: image)
                    .renderingMode(.original)
                    .resizable()
                    .scaledToFit()
            }
        } else {
            Label(group.title, systemImage: "bubble.left.and.bubble.right")
        }
    }

    private static func decodedMenuIcon(from raw: String?) -> UIImage? {
        GaryxDataURLImageCache.image(from: raw)
    }

    private func attachPhotos(_ items: [PhotosPickerItem]) async {
        var images: [GaryxMobileSelectedImage] = []
        for (index, item) in items.enumerated() {
            do {
                guard let data = try await item.loadTransferable(type: Data.self) else {
                    continue
                }
                let contentType = item.supportedContentTypes.first { $0.conforms(to: .image) }
                    ?? item.supportedContentTypes.first
                let mediaType = contentType?.preferredMIMEType ?? "image/jpeg"
                let fileExtension = contentType?.preferredFilenameExtension ?? "jpg"
                guard let image = await Task.detached(priority: .utility, operation: {
                    Self.preparedPhotoUpload(
                        data: data,
                        index: index,
                        mediaType: mediaType,
                        fileExtension: fileExtension
                    )
                }).value else {
                    model.lastError = "That image is too large to prepare for upload."
                    continue
                }
                images.append(image)
            } catch {
                model.lastError = error.localizedDescription
            }
        }
        await model.attachImages(images)
    }

    nonisolated private static func preparedPhotoUpload(
        data: Data,
        index: Int,
        mediaType: String,
        fileExtension: String
    ) -> GaryxMobileSelectedImage? {
        if let jpegData = compressedJPEGPhotoData(from: data) {
            return GaryxMobileSelectedImage(
                name: "photo-\(index + 1).jpg",
                mediaType: "image/jpeg",
                data: jpegData
            )
        }
        guard data.count <= maxPreparedPhotoBytes else {
            return nil
        }
        let normalizedExtension = fileExtension.trimmingCharacters(in: .whitespacesAndNewlines)
        return GaryxMobileSelectedImage(
            name: "photo-\(index + 1).\(normalizedExtension.isEmpty ? "jpg" : normalizedExtension)",
            mediaType: mediaType.isEmpty ? "image/jpeg" : mediaType,
            data: data
        )
    }

    nonisolated private static func compressedJPEGPhotoData(from data: Data) -> Data? {
        for maxPixelSize in preparedPhotoPixelSizes {
            guard let image = thumbnailImage(from: data, maxPixelSize: maxPixelSize) else {
                continue
            }
            for quality in preparedPhotoJPEGQualities {
                guard let jpegData = image.jpegData(compressionQuality: quality) else {
                    continue
                }
                if jpegData.count <= maxPreparedPhotoBytes {
                    return jpegData
                }
            }
        }
        return nil
    }

    nonisolated private static func thumbnailImage(from data: Data, maxPixelSize: CGFloat) -> UIImage? {
        let options = [kCGImageSourceShouldCache: false] as CFDictionary
        if let source = CGImageSourceCreateWithData(data as CFData, options) {
            let thumbnailOptions: [CFString: Any] = [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceShouldCacheImmediately: true,
                kCGImageSourceThumbnailMaxPixelSize: Int(maxPixelSize),
            ]
            if let image = CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnailOptions as CFDictionary) {
                return UIImage(cgImage: image)
            }
        }

        guard let image = UIImage(data: data) else {
            return nil
        }
        let maxSide = max(image.size.width, image.size.height)
        guard maxSide > maxPixelSize else {
            return image
        }
        let scale = maxPixelSize / maxSide
        let targetSize = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let renderer = UIGraphicsImageRenderer(size: targetSize)
        return renderer.image { _ in
            image.draw(in: CGRect(origin: .zero, size: targetSize))
        }
    }

    nonisolated private static var preparedPhotoPixelSizes: [CGFloat] {
        [2048, 1600, 1280, 1024]
    }

    nonisolated private static var preparedPhotoJPEGQualities: [CGFloat] {
        [0.82, 0.72, 0.62, 0.52, 0.42, 0.34]
    }

    nonisolated private static var maxPreparedPhotoBytes: Int {
        1_350_000
    }
}

struct GaryxAttachmentChip: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let attachment: GaryxMobileComposerAttachment

    var body: some View {
        if attachment.kind == "image", let thumbnail = decodedThumbnail {
            imageChip(thumbnail: thumbnail)
        } else {
            fileChip
        }
    }

    private func imageChip(thumbnail: UIImage) -> some View {
        ZStack(alignment: .topTrailing) {
            Image(uiImage: thumbnail)
                .resizable()
                .scaledToFill()
                .frame(width: 56, height: 56)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .stroke(Color.primary.opacity(0.08), lineWidth: 1)
                }

            Button {
                model.removeComposerAttachment(attachment)
            } label: {
                Image(systemName: "xmark")
                    .font(GaryxFont.system(size: 9, weight: .bold))
                    .foregroundStyle(Color.white)
                    .padding(4)
                    .background(Color.black.opacity(0.65), in: Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove attachment")
            .padding(4)
        }
    }

    private var fileChip: some View {
        HStack(spacing: 7) {
            Image(systemName: "doc")
                .font(GaryxFont.caption(weight: .semibold))
            Text(attachment.name)
                .font(GaryxFont.caption(weight: .semibold))
                .lineLimit(1)
            Button {
                model.removeComposerAttachment(attachment)
            } label: {
                Image(systemName: "xmark")
                    .font(GaryxFont.caption(weight: .bold))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove attachment")
        }
        .foregroundStyle(.primary)
        .padding(.horizontal, 10)
        .frame(height: 30)
        .background(Color(.tertiarySystemFill), in: Capsule())
    }

    private var decodedThumbnail: UIImage? {
        GaryxDataURLImageCache.image(from: attachment.previewDataUrl)
    }
}
