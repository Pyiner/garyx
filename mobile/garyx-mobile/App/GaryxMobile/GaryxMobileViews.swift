import Foundation
import ImageIO
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

enum GaryxMobileMotion {
    static let sidebar = Animation.interactiveSpring(response: 0.28, dampingFraction: 0.92, blendDuration: 0.08)
    static let sidebarDrilldown = Animation.easeOut(duration: 0.16)
    static let rowSwipe = Animation.interactiveSpring(response: 0.22, dampingFraction: 0.92, blendDuration: 0.04)
}

struct GaryxRootView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        ZStack {
            GaryxTheme.background.ignoresSafeArea()

            if model.hasGatewaySettings, case .ready = model.connectionState {
                GaryxShellView()
            } else {
                GaryxGatewaySetupView()
            }
        }
        .overlay(alignment: .top) {
            GaryxGlobalErrorToastHost(topOffset: 72)
        }
        .task {
            #if DEBUG
            guard !model.debugSnapshotActive else { return }
            #endif
            if model.canConnectGateway {
                await model.connectAndRefresh()
            }
        }
        .onOpenURL { url in
            #if DEBUG
            if model.applyDebugURL(url) {
                return
            }
            #endif
            Task { await model.applyMobileConnectLink(url) }
        }
        .sheet(isPresented: $model.showsSettings) {
            GaryxGatewaySetupView(isSheet: true)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
        }
    }
}

struct GaryxGatewaySetupView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    var isSheet = false
    var startsEmpty = false
    @State private var draftGatewayURL = ""
    @State private var draftGatewayAuthToken = ""
    @State private var didInitializeDraft = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Spacer(minLength: 32)

                VStack(spacing: 20) {
                    GaryxAppLogo(size: 88)

                    GaryxConnectionPill(
                        label: setupStatusLabel,
                        color: setupStatusColor,
                        isBusy: setupIsBusy
                    )

                    VStack(spacing: 10) {
                        Text("Gary X")
                            .font(GaryxFont.largeTitle(weight: .semibold))
                            .foregroundStyle(.primary)

                        Text("Set the gateway address and token, then save. Saving verifies the gateway before continuing.")
                            .font(GaryxFont.callout())
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxWidth: 280)

                    VStack(spacing: 10) {
                        HStack(spacing: 8) {
                            TextField("Gateway URL", text: $draftGatewayURL)
                                .textContentType(.URL)
                                .keyboardType(.URL)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .garyxInputStyle()

                            GaryxGatewayProfileMenuButton { profile in
                                model.selectGatewayProfile(profile)
                                draftGatewayURL = model.gatewayURL
                                draftGatewayAuthToken = model.gatewayAuthToken
                            }
                        }

                        SecureField("Gateway Token", text: $draftGatewayAuthToken)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .garyxInputStyle()
                    }

                    GaryxPrimaryCapsuleButton(
                        title: setupIsBusy ? "Saving..." : "Save and Continue",
                        systemImage: setupIsBusy ? nil : "checkmark.circle.fill"
                    ) {
                        Task {
                            model.gatewayURL = draftGatewayURL
                            model.gatewayAuthToken = draftGatewayAuthToken
                            await model.connectAndRefresh()
                            if isSheet, case .ready = model.connectionState {
                                dismiss()
                            }
                        }
                    }
                    .disabled(!canSaveGateway || setupIsBusy)
                    .opacity(canSaveGateway && !setupIsBusy ? 1 : 0.45)
                }
                .frame(maxWidth: 320)
                .padding(.horizontal, 24)

                Spacer(minLength: 24)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(GaryxTheme.background)
            .navigationTitle("Gary X")
            .navigationBarTitleDisplayMode(.inline)
            .onAppear(perform: initializeDraft)
            .toolbar {
                if isSheet {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Done") {
                            model.showsSettings = false
                            dismiss()
                        }
                    }
                }
            }
            .overlay(alignment: .top) {
                if isSheet {
                    GaryxGlobalErrorToastHost(topOffset: 8)
                }
            }
        }
    }

    private var canSaveGateway: Bool {
        let trimmed = draftGatewayURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              ["http", "https"].contains(scheme),
              components.host != nil else {
            return false
        }
        return true
    }

    private func initializeDraft() {
        guard !didInitializeDraft else { return }
        draftGatewayURL = startsEmpty ? "" : model.gatewayURL
        draftGatewayAuthToken = startsEmpty ? "" : model.gatewayAuthToken
        didInitializeDraft = true
    }

    private var setupIsBusy: Bool {
        if case .checking = model.connectionState {
            return true
        }
        return false
    }

    private var setupStatusLabel: String {
        if startsEmpty && draftGatewayURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return "Add Gateway"
        }
        switch model.connectionState {
        case .disconnected:
            return "Not connected"
        case .checking:
            return "Connecting"
        case .ready:
            return "Connected"
        case .failed:
            return "Offline"
        }
    }

    private var setupStatusColor: Color {
        if startsEmpty && draftGatewayURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return Color(.tertiaryLabel)
        }
        switch model.connectionState {
        case .checking:
            return .orange
        case .ready:
            return .green
        case .disconnected, .failed:
            return Color(.tertiaryLabel)
        }
    }
}

struct GaryxShellView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.colorScheme) private var colorScheme

    @State private var sidebarDragOffset: CGFloat = 0

    private let sidebarWidth: CGFloat = 330
    private let sidebarEdgeGestureWidth: CGFloat = 64

    var body: some View {
        GeometryReader { proxy in
            let usePersistentSidebar = proxy.size.width > 760 && horizontalSizeClass != .compact
            let currentSidebarWidth = min(sidebarWidth, proxy.size.width)

            Group {
                if usePersistentSidebar {
                    HStack(spacing: 0) {
                        GaryxThreadSidebar(showsInlineCloseButton: false)
                            .frame(width: currentSidebarWidth)

                        GaryxMainPanelView()
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                    .background(GaryxTheme.background)
                } else {
                    drawerBody(width: drawerSidebarWidth(for: proxy.size), containerSize: proxy.size)
                }
            }
            .onChange(of: usePersistentSidebar) { _, isPersistent in
                sidebarDragOffset = 0
                if isPersistent {
                    model.setSidebarVisible(false, animated: false)
                }
            }
        }
        .onChange(of: horizontalSizeClass) { _, _ in
            sidebarDragOffset = 0
        }
    }

    private func drawerSidebarWidth(for containerSize: CGSize) -> CGFloat {
        if horizontalSizeClass == .compact {
            return containerSize.width
        }
        return min(sidebarWidth, containerSize.width * 0.92)
    }

    private func drawerBody(width: CGFloat, containerSize: CGSize) -> some View {
        let revealWidth = sidebarRevealWidth(for: width)
        let drawerOffset = revealWidth - width

        return ZStack(alignment: .topLeading) {
            GaryxMainPanelView()
                .frame(width: containerSize.width, height: containerSize.height)
                .contentShape(Rectangle())
                .simultaneousGesture(openingSidebarGesture(sidebarWidth: width))
                .zIndex(0)

            (colorScheme == .dark ? Color.white : Color.black)
                .opacity(contentDimOpacity(for: width))
                .frame(width: containerSize.width, height: containerSize.height)
                .ignoresSafeArea()
                .contentShape(Rectangle())
                .allowsHitTesting(revealWidth > 1)
                .onTapGesture { closeSidebar() }
                .gesture(closingSidebarGesture(sidebarWidth: width))
                .zIndex(1)

            GaryxThreadSidebar(showsInlineCloseButton: true)
                .frame(width: width)
                .frame(maxHeight: .infinity)
                .offset(x: drawerOffset)
                .allowsHitTesting(revealWidth > width * 0.82)
                .zIndex(2)

            if revealWidth > 1 {
                Color.clear
                    .frame(width: 28, height: containerSize.height)
                    .offset(x: revealWidth)
                    .contentShape(Rectangle())
                    .gesture(closingSidebarGesture(sidebarWidth: width))
                    .zIndex(3)
                    .accessibilityHidden(true)
            }

            if !model.sidebarVisible {
                Button {
                    finishGesture(open: true)
                } label: {
                    Rectangle()
                        .fill(Color.clear)
                        .frame(width: 78, height: 178)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .zIndex(4)
                .accessibilityHidden(true)
            }
        }
        .background(GaryxTheme.background)
    }

    private func sidebarRevealWidth(for width: CGFloat) -> CGFloat {
        if model.sidebarVisible {
            return max(0, min(width, width + sidebarDragOffset))
        }
        return max(0, min(width, sidebarDragOffset))
    }

    private func contentDimOpacity(for width: CGFloat) -> Double {
        guard width > 0 else { return 0 }
        return 0.12 * min(1, sidebarRevealWidth(for: width) / width)
    }

    private func openingSidebarGesture(sidebarWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 12, coordinateSpace: .global)
            .onChanged { value in
                guard !model.sidebarVisible, isOpeningSidebarGesture(value) else {
                    return
                }
                sidebarDragOffset = max(0, min(sidebarWidth, value.translation.width))
            }
            .onEnded { value in
                guard !model.sidebarVisible, isOpeningSidebarGesture(value) else {
                    resetSidebarDrag()
                    return
                }
                let shouldOpen = value.translation.width > sidebarWidth * 0.22
                    || value.predictedEndTranslation.width > sidebarWidth * 0.35
                finishGesture(open: shouldOpen)
            }
    }

    private func closingSidebarGesture(sidebarWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 12, coordinateSpace: .global)
            .onChanged { value in
                guard model.sidebarVisible, isClosingSidebarGesture(value) else {
                    return
                }
                sidebarDragOffset = min(0, max(-sidebarWidth, value.translation.width))
            }
            .onEnded { value in
                guard model.sidebarVisible, isClosingSidebarGesture(value) else {
                    resetSidebarDrag()
                    return
                }
                let shouldClose = -value.translation.width > sidebarWidth * 0.22
                    || -value.predictedEndTranslation.width > sidebarWidth * 0.35
                finishGesture(open: !shouldClose)
            }
    }

    private func isOpeningSidebarGesture(_ value: DragGesture.Value) -> Bool {
        let horizontal = value.translation.width
        let vertical = value.translation.height
        return value.startLocation.x <= sidebarEdgeGestureWidth
            && horizontal > 0
            && abs(horizontal) > abs(vertical) * 1.15
    }

    private func isClosingSidebarGesture(_ value: DragGesture.Value) -> Bool {
        let horizontal = value.translation.width
        let vertical = value.translation.height
        return horizontal < 0 && abs(horizontal) > abs(vertical) * 1.05
    }

    private func finishGesture(open: Bool) {
        hideKeyboard()
        withAnimation(GaryxMobileMotion.sidebar) {
            model.setSidebarVisible(open, animated: false)
            sidebarDragOffset = 0
        }
    }

    private func resetSidebarDrag() {
        withAnimation(GaryxMobileMotion.sidebar) {
            sidebarDragOffset = 0
        }
    }

    private func closeSidebar() {
        finishGesture(open: false)
    }

    private func hideKeyboard() {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil,
            from: nil,
            for: nil
        )
    }
}

struct GaryxMainPanelView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        NavigationStack {
            Group {
                switch model.activePanel {
                case .chat:
                    GaryxConversationView()
                case .tasks:
                    GaryxTasksView()
                case .automations:
                    GaryxAutomationsView()
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
    @State private var activeDrilldown: GaryxSidebarDrilldown?

    var body: some View {
        threadListWithBottomBar
            .frame(maxHeight: .infinity)
            .background(GaryxTheme.background)
            .garyxAdaptiveTopBar {
                GaryxSidebarHeaderView(
                    drilldownContext: sidebarHeaderContext,
                    showsCloseButton: showsInlineCloseButton,
                    onBack: { closeDrilldown() },
                    onClose: { closeSidebar() }
                )
                .modifier(GaryxSidebarHeaderBackdropModifier())
            }
            .task {
                if model.threads.isEmpty {
                    await model.refreshThreads()
                }
            }
            .onAppear {
                reconcileActiveDrilldown()
            }
            .onChange(of: model.sidebarUnscopedThreads.map(\.id)) { _, _ in
                reconcileActiveDrilldown()
            }
            .onChange(of: model.sidebarBotDrilldownFingerprints) { _, _ in
                reconcileActiveDrilldown()
            }
            .onChange(of: model.sidebarWorkspaceThreadGroups.map(\.path)) { _, _ in
                reconcileActiveDrilldown()
            }
    }

    private var threadListWithBottomBar: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                if activeDrilldown == nil {
                    GaryxSidebarNavigationList()
                        .padding(.horizontal, GaryxSidebarMetrics.outerHorizontalPadding)
                        .padding(.top, 6)
                        .padding(.bottom, 14)
                }

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
            .background(GaryxTheme.background.ignoresSafeArea(edges: .bottom))
        }
    }

    @ViewBuilder
    private var sidebarThreadSections: some View {
        switch activeDrilldown {
        case .unscopedThreads:
            GaryxUnscopedThreadsSection(activeDrilldown: $activeDrilldown)
        case .bot:
            GaryxSidebarBotsSection(activeDrilldown: $activeDrilldown)
        case .workspace:
            GaryxWorkspaceThreadGroupsSection(activeDrilldown: $activeDrilldown)
        case nil:
            GaryxPinnedThreadsSection()
            GaryxSidebarBotsSection(activeDrilldown: $activeDrilldown)
            GaryxWorkspaceThreadGroupsSection(activeDrilldown: $activeDrilldown)
        }
    }

    private var sidebarHeaderContext: GaryxSidebarHeaderContext? {
        switch activeDrilldown {
        case .unscopedThreads:
            GaryxSidebarHeaderContext(title: "Threads", subtitle: nil)
        case let .bot(id):
            model.mobileBotGroups
                .first { $0.id == id }
                .map { GaryxSidebarHeaderContext(title: $0.title, subtitle: $0.compactDetailLine) }
        case let .workspace(path):
            model.sidebarWorkspaceThreadGroups
                .first { $0.path == path }
                .map { GaryxSidebarHeaderContext(title: $0.name, subtitle: $0.path) }
        case nil:
            nil
        }
    }

    private func closeSidebar() {
        model.setSidebarVisible(false)
    }

    private func closeDrilldown() {
        withAnimation(GaryxMobileMotion.sidebarDrilldown) {
            activeDrilldown = nil
        }
    }

    private func reconcileActiveDrilldown() {
        switch activeDrilldown {
        case .unscopedThreads where model.sidebarUnscopedThreads.isEmpty:
            activeDrilldown = nil
        case let .bot(id):
            guard let group = model.mobileBotGroups.first(where: { $0.id == id }),
                  !group.sidebarChildConversationEntries(visibleThreadIds: model.sidebarVisibleThreadIds).isEmpty else {
                activeDrilldown = nil
                break
            }
        case let .workspace(path) where !model.sidebarWorkspaceThreadGroups.contains(where: { $0.path == path }):
            activeDrilldown = nil
        default:
            break
        }
    }

    private func refreshAll() async {
        await model.refreshThreads()
        await model.refreshRemoteState()
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
        HStack(alignment: .center, spacing: 10) {
            if let drilldownContext {
                Button(action: onBack) {
                    Image(systemName: "chevron.left")
                        .font(GaryxFont.system(size: 17, weight: .semibold))
                        .foregroundStyle(.primary)
                        .frame(width: 44, height: 44)
                        .background {
                            Circle()
                                .fill(Color(.systemBackground).opacity(0.42))
                                .background(.ultraThinMaterial, in: Circle())
                        }
                        .overlay {
                            Circle()
                                .stroke(Color.primary.opacity(0.032), lineWidth: 1)
                        }
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Back")

                VStack(alignment: .leading, spacing: 2) {
                    Text(drilldownContext.title)
                        .font(GaryxFont.system(size: 23, weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)

                    if let subtitle = drilldownContext.subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text("Gary X")
                    .font(GaryxFont.system(size: 26, weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.75)

                Spacer(minLength: 0)
            }

            if showsCloseButton {
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.primary)
                        .frame(width: 44, height: 44)
                        .background {
                            Circle()
                                .fill(Color(.systemBackground).opacity(0.42))
                                .background(.ultraThinMaterial, in: Circle())
                        }
                        .overlay {
                            Circle()
                                .stroke(Color.primary.opacity(0.032), lineWidth: 1)
                        }
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close menu")
            }
        }
        .padding(.horizontal, 26)
        .padding(.top, 6)
        .padding(.bottom, 14)
    }
}

struct GaryxSidebarNavigationList: View {
    @EnvironmentObject private var model: GaryxMobileModel

    private let panels: [GaryxMobilePanel] = [
        .automations,
        .tasks,
        .autoResearch,
        .agents,
        .skills,
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
            .frame(height: 39)
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

private enum GaryxSidebarDrilldown: Equatable {
    case unscopedThreads
    case bot(String)
    case workspace(String)
}

private extension GaryxMobileModel {
    var sidebarUnscopedThreads: [GaryxThreadSummary] {
        threads
            .filter { thread in
                let workspace = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                return workspace.isEmpty
            }
            .sorted(by: garyxThreadSort)
    }

    var sidebarWorkspaceThreadGroups: [GaryxSidebarWorkspaceThreadGroup] {
        let grouped = Dictionary(grouping: threads) { thread in
            thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        }
        let paths = knownWorkspacePaths
            .filter(GaryxMobileModel.isVisibleMobileWorkspacePath)
        let duplicateNames = Dictionary(grouping: paths, by: { $0.lastPathComponent })
            .filter { !$0.key.isEmpty && $0.value.count > 1 }
        return paths
            .map { path in
                let name = path.lastPathComponent.isEmpty ? path : path.lastPathComponent
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

    var sidebarBotDrilldownFingerprints: [String] {
        let visibleThreadIds = sidebarVisibleThreadIds
        return mobileBotGroups.map { group in
            let childIds = group.sidebarChildConversationEntries(visibleThreadIds: visibleThreadIds)
                .map(\.id)
                .joined(separator: ",")
            return "\(group.id):\(childIds)"
        }
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

private struct GaryxPinnedThreadsSection: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        if !model.pinnedThreads.isEmpty {
            GaryxPinnedThreadsDetailSection()
                .padding(.bottom, 10)
        }
    }
}

private struct GaryxUnscopedThreadsSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var activeDrilldown: GaryxSidebarDrilldown?

    var body: some View {
        if activeDrilldown == .unscopedThreads || !model.sidebarUnscopedThreads.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                if activeDrilldown == .unscopedThreads {
                    GaryxUnscopedThreadsDetailSection()
                } else {
                    GaryxSidebarDisclosureRow(
                        title: "Threads",
                        systemName: "bubble.left.and.text.bubble.right.fill"
                    ) {
                        withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                            activeDrilldown = .unscopedThreads
                        }
                    }
                }
            }
            .padding(.horizontal, GaryxSidebarMetrics.rowOuterPadding)
            .padding(.bottom, 10)
        }
    }
}

private struct GaryxUnscopedThreadsDetailSection: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            GaryxSidebarSectionHeader(title: "Threads", systemImage: "bubble.left.and.text.bubble.right.fill")
                .padding(.horizontal, GaryxSidebarMetrics.sectionHorizontalPadding)
                .padding(.bottom, 4)

            ForEach(model.sidebarUnscopedThreads) { thread in
                GaryxSidebarThreadButton(
                    thread: thread,
                    showsWorkspaceMeta: false,
                    trailingTimestamp: garyxFormattedTaskTimestamp(thread.updatedAt ?? thread.createdAt)
                )
            }
        }
        .transition(.opacity)
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

private struct GaryxSidebarBotsSection: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var activeDrilldown: GaryxSidebarDrilldown?

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
                        GaryxSidebarBotRow(
                            group: group,
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
    @EnvironmentObject private var model: GaryxMobileModel
    let group: GaryxMobileBotGroup
    let onSelect: () -> Void
    let onOpenRoot: () -> Void

    private var canDrillDown: Bool {
        !group.sidebarChildConversationEntries(visibleThreadIds: model.sidebarVisibleThreadIds).isEmpty
    }

    private var rootCanOpen: Bool {
        let mainThreadId = group.mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return group.rootBehavior != "expand_only" || !mainThreadId.isEmpty
    }

    private var rowCanOpen: Bool {
        canDrillDown || rootCanOpen
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
            .disabled(!rowCanOpen)

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

private extension GaryxMobileBotGroup {
    var compactDetailLine: String {
        let botId = "\(channel):\(accountId)"
        let agent = agentId?.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = workspaceDir?.trimmingCharacters(in: .whitespacesAndNewlines)
        return [
            botId,
            agent.flatMap { $0.isEmpty ? nil : $0 },
            workspace.flatMap { $0.isEmpty ? nil : $0.lastPathComponent },
        ]
        .compactMap { $0 }
        .joined(separator: " / ")
    }

    func sidebarChildConversationEntries(visibleThreadIds: Set<String>) -> [GaryxBotSidebarConversationEntry] {
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
            return entries
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
                    subtitle: endpoint.conversationLabel ?? endpoint.threadLabel ?? endpoint.workspaceDir?.lastPathComponent,
                    threadId: threadId,
                    latestActivity: endpoint.lastInboundAt ?? endpoint.lastDeliveryAt,
                    openable: true,
                    endpoint: endpoint
                )
            )
        }

        return entries
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
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
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
    @EnvironmentObject private var model: GaryxMobileModel
    let group: GaryxSidebarWorkspaceThreadGroup
    let isSelected: Bool
    let onSelect: () -> Void

    var body: some View {
        GaryxSwipeActionRow(actions: workspaceActions) {
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

    private var workspaceActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "New", systemImage: "square.and.pencil", tone: .accent) {
                Task { await model.createThread(inWorkspace: group.path) }
            }
        ]
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
                .font(GaryxFont.callout(weight: .semibold))
                .lineLimit(1)
        }
        .foregroundStyle(.primary.opacity(0.86))
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
        GaryxSwipeActionRow(actions: rowActions) {
            Button {
                Task { await model.selectThread(thread) }
            } label: {
                GaryxSidebarThreadRowView(
                    thread: thread,
                    isSelected: model.selectedThread?.id == thread.id,
                    isPinned: showsPinnedMarker || model.isThreadPinned(thread.id),
                    showsWorkspaceMeta: showsWorkspaceMeta,
                    trailingTimestamp: trailingTimestamp,
                    isFullBleed: isFullBleed
                )
                .padding(.leading, indent)
            }
            .buttonStyle(.plain)
        }
    }

    private var rowActions: [GaryxSwipeAction] {
        var actions = [
            GaryxSwipeAction(
                title: model.isThreadPinned(thread.id) ? "Unpin" : "Pin",
                systemImage: model.isThreadPinned(thread.id) ? "pin.slash" : "pin"
            ) {
                model.togglePinnedThread(thread.id)
            },
        ]
        if model.canDeleteThread(thread) {
            actions.append(
                GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                    Task { await model.deleteThread(thread) }
                }
            )
        }
        return actions
    }
}

struct GaryxSidebarThreadRowView: View {
    let thread: GaryxThreadSummary
    let isSelected: Bool
    var isPinned = false
    var showsWorkspaceMeta = true
    var trailingTimestamp: String?
    var isFullBleed = false

    var body: some View {
        HStack(alignment: .center, spacing: 8) {
            VStack(alignment: .leading, spacing: 4) {
                Text(thread.title.isEmpty ? "Untitled" : thread.title)
                    .font(GaryxFont.subheadline(weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .foregroundStyle(.primary)

                if let subtitle, !subtitle.isEmpty {
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
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(minHeight: GaryxSidebarMetrics.threadRowMinHeight, alignment: .leading)
        .contentShape(Rectangle())
        .padding(.horizontal, isFullBleed ? GaryxSidebarMetrics.sectionHorizontalPadding : GaryxSidebarMetrics.rowInnerHorizontalPadding)
        .padding(.vertical, isFullBleed ? 10 : 8)
        .background {
            if isSelected {
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

    private var subtitle: String? {
        if !thread.lastMessagePreview.isEmpty {
            return thread.lastMessagePreview
        }
        if let workspacePath = thread.workspacePath, !workspacePath.isEmpty {
            return workspacePath.lastPathComponent
        }
        if let teamName = thread.teamName, !teamName.isEmpty {
            return teamName
        }
        return thread.agentId
    }

    private var trailingMeta: some View {
        HStack(spacing: 6) {
            if isPinned {
                Image(systemName: "pin.fill")
                    .font(GaryxFont.system(size: 10, weight: .semibold))
                    .foregroundStyle(GaryxTheme.accent)
            }

            if showsWorkspaceMeta, let workspacePath = thread.workspacePath, !workspacePath.isEmpty {
                Text(workspacePath.lastPathComponent)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .frame(maxWidth: 72, alignment: .trailing)
            }

            if isSelected {
                Circle()
                    .fill(GaryxTheme.accent)
                    .frame(width: 7, height: 7)
            } else if let trailingTimestamp, !trailingTimestamp.isEmpty {
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
            .frame(height: 42)
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

struct GaryxConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        ScrollViewReader { proxy in
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
                        ForEach(model.messages) { message in
                            GaryxMessageBubble(message: message)
                                .id(message.id)
                        }
                        if model.showsTailThinkingIndicator {
                            GaryxThinkingLabel()
                                .id("tail-thinking")
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 18)
                .padding(.bottom, 12)
            }
            .refreshable {
                await model.loadSelectedThreadHistory()
            }
            .onChange(of: model.messages) { _, newValue in
                guard let last = newValue.last else { return }
                withAnimation(.easeOut(duration: 0.2)) {
                    proxy.scrollTo(last.id, anchor: .bottom)
                }
            }
        }
        .background(GaryxTheme.background)
        .garyxAdaptiveTopBar {
            GaryxConversationHeader()
                .modifier(GaryxSidebarHeaderBackdropModifier())
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            GaryxComposer()
                .background(Color.clear)
        }
        .garyxAdaptiveSoftScrollEdge(for: .top)
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
                    .background {
                        Capsule()
                            .fill(Color(.systemBackground).opacity(0.42))
                            .background(.ultraThinMaterial, in: Capsule())
                    }
                    .overlay {
                        Capsule()
                            .stroke(Color.primary.opacity(0.03), lineWidth: 1)
                    }
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
                    }
                    Button("Refresh", systemImage: "arrow.clockwise") {
                        Task { await model.loadSelectedThreadHistory() }
                    }
                    Button("Rename", systemImage: "pencil") {
                        openRenamePrompt()
                    }
                    Button("New Thread", systemImage: "square.and.pencil") {
                        model.openNewThreadDraft()
                    }
                    Button("Delete", systemImage: "trash", role: .destructive) {
                        Task { await model.deleteSelectedThread() }
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
        hideKeyboard()
        model.setSidebarVisible(true)
    }

    private func hideKeyboard() {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil,
            from: nil,
            for: nil
        )
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

private struct GaryxAgentPickerLabel: View {
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
                        Label(path.lastPathComponent.isEmpty ? path : path.lastPathComponent, systemImage: "folder")
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
}

struct GaryxMessageBubble: View {
    let message: GaryxMobileMessage
    @Environment(\.colorScheme) private var colorScheme

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
                        Text(statusText)
                            .font(GaryxFont.caption())
                            .foregroundStyle(Color(.systemRed))
                            .lineLimit(2)
                            .multilineTextAlignment(.trailing)
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
        message.text.isEmpty && message.isStreaming ? "Thinking" : message.text
    }

    private var userBubbleBackground: Color {
        (colorScheme == .dark ? Color.white.opacity(0.12) : Color.black.opacity(0.05))
    }

    private var userCodeBackground: Color {
        colorScheme == .dark ? Color.white.opacity(0.08) : Color.black.opacity(0.055)
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

struct GaryxToolTraceGroupView: View {
    let group: GaryxMobileToolTraceGroup

    @State private var expanded: Bool
    @State private var userControlled = false

    init(group: GaryxMobileToolTraceGroup) {
        self.group = group
        _expanded = State(initialValue: group.defaultExpanded)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                userControlled = true
                withAnimation(.easeOut(duration: 0.19)) {
                    expanded.toggle()
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "terminal")
                        .font(GaryxFont.system(size: 13, weight: .regular))
                        .frame(width: 16, height: 16)

                    Text(group.summary)
                        .font(GaryxFont.footnote())
                        .lineLimit(1)
                        .truncationMode(.tail)

                    if group.isActive {
                        ProgressView()
                            .scaleEffect(0.62)
                    }

                    Image(systemName: "chevron.down")
                        .font(GaryxFont.system(size: 10, weight: .semibold))
                        .rotationEffect(.degrees(expanded ? 0 : -90))
                        .opacity(0.74)
                }
                .foregroundStyle(group.isActive ? GaryxTheme.primaryText : GaryxTheme.secondaryText)
                .frame(minHeight: 22)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(expanded ? "Collapse tool calls" : "Expand tool calls")
            .accessibilityAddTraits(.isButton)

            if expanded {
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(group.entries) { entry in
                        GaryxToolTraceEntryView(entry: entry)
                    }
                }
                .padding(.top, 5)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onChange(of: group.defaultExpanded) { _, shouldExpand in
            guard !userControlled else { return }
            withAnimation(.easeOut(duration: 0.21)) {
                expanded = shouldExpand
            }
        }
    }
}

struct GaryxToolTraceEntryView: View {
    let entry: GaryxMobileToolTraceEntry

    @State private var expanded = false

    private var hasDetails: Bool {
        entry.inputText != nil || entry.resultText != nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            header

            if expanded && hasDetails {
                VStack(alignment: .leading, spacing: 4) {
                    if let inputText = entry.inputText {
                        GaryxToolTraceDetailSection(label: entry.inputLabel, text: inputText)
                    }
                    if let resultText = entry.resultText {
                        GaryxToolTraceDetailSection(label: entry.resultLabel, text: resultText)
                    }
                }
                .padding(.top, 1)
                .transition(.opacity)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .transaction { transaction in
            transaction.animation = nil
        }
    }

    @ViewBuilder
    private var header: some View {
        let content = HStack(spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: iconName)
                    .font(GaryxFont.system(size: 12, weight: .regular))
                    .foregroundStyle(GaryxTheme.secondaryText)
                    .frame(width: 16, height: 16)

                Text(entry.title)
                    .font(GaryxFont.footnote())
                    .foregroundStyle(GaryxTheme.secondaryText)
                    .lineLimit(1)

                if let previewText = entry.previewText {
                    Text(previewText)
                        .font(GaryxFont.system(size: 11))
                        .foregroundStyle(GaryxTheme.secondaryText)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Text(entry.status.label)
                .font(GaryxFont.system(size: 11))
                .foregroundStyle(statusColor)
                .textCase(.lowercase)

            if hasDetails {
                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 10, weight: .semibold))
                    .foregroundStyle(Color(.tertiaryLabel))
                    .rotationEffect(.degrees(expanded ? 90 : 0))
            }
        }
        .frame(minHeight: 20)

        if hasDetails {
            Button {
                withAnimation(.easeOut(duration: 0.16)) {
                    expanded.toggle()
                }
            } label: {
                content
            }
            .buttonStyle(.plain)
            .accessibilityLabel(expanded ? "Collapse tool details" : "Expand tool details")
        } else {
            content
        }
    }

    private var iconName: String {
        switch entry.status {
        case .running:
            "circle.dotted"
        case .completed:
            entry.isCommand ? "terminal" : "checkmark.circle"
        case .failed:
            "exclamationmark.triangle"
        }
    }

    private var statusColor: Color {
        switch entry.status {
        case .running:
            GaryxTheme.accent
        case .completed:
            GaryxTheme.secondaryText.opacity(0.5)
        case .failed:
            GaryxTheme.danger
        }
    }
}

struct GaryxToolTraceDetailSection: View {
    let label: String
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(GaryxTheme.secondaryText)
                .textCase(.uppercase)

            Text(text)
                .font(.system(size: 12, weight: .regular, design: .monospaced))
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .stroke(GaryxTheme.hairline, lineWidth: 1)
                }
        }
    }
}

struct GaryxThinkingLabel: View {
    var body: some View {
        HStack(spacing: 10) {
            ProgressView()
                .scaleEffect(0.72)
            Text("Thinking")
                .font(GaryxFont.body())
                .foregroundStyle(GaryxTheme.secondaryText)
        }
        .frame(minHeight: 22)
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
            ForEach(GaryxMarkdownBlock.blocks(from: text)) { block in
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
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .full,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        return (try? AttributedString(markdown: markdown, options: options)) ?? AttributedString(markdown)
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
    @FocusState private var isFocused: Bool
    @State private var isPickingAttachments = false
    @State private var isPickingPhotos = false
    @State private var selectedPhotoItems: [PhotosPickerItem] = []

    private var showsSendButton: Bool {
        !model.isSelectedThreadSending || model.hasComposerPayload
    }

    var body: some View {
        GaryxAdaptiveGlassContainer(spacing: GaryxComposerLayout.composerSpacing) {
            composerCard
        }
        .padding(.horizontal, 12)
        .padding(.top, 4)
        .padding(.bottom, 4)
        .frame(maxWidth: .infinity, alignment: .leading)
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
            if model.draft.isEmpty {
                Text(placeholderText)
                    .font(GaryxFont.subheadline())
                    .foregroundStyle(Color(.placeholderText))
                    .padding(.top, 2)
                    .allowsHitTesting(false)
            }

            TextField("", text: $model.draft, axis: .vertical)
                .id(GaryxComposerLayout.draftFieldIdentity)
                .font(GaryxFont.subheadline())
                .foregroundStyle(.primary)
                .focused($isFocused)
                .lineLimit(1...4)
                .submitLabel(.send)
                .onSubmit {
                    Task { await model.sendDraft() }
                }
        }
        .frame(maxWidth: .infinity, minHeight: 34, alignment: .topLeading)
        .padding(.horizontal, GaryxComposerLayout.inputHorizontalPadding)
        .padding(.top, model.composerAttachments.isEmpty ? GaryxComposerLayout.inputTopPadding : 6)
        .padding(.bottom, GaryxComposerLayout.inputBottomPadding)
        .contentShape(Rectangle())
        .onTapGesture {
            isFocused = true
        }
    }

    private var placeholderText: String {
        model.selectedThread == nil ? "Ask Gary X anything..." : "Ask for follow-up changes"
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
                    Task { await model.sendDraft() }
                } label: {
                    GaryxCircleBadge(
                        systemName: "arrow.up",
                        foreground: model.canSend ? Color(.systemBackground) : Color(.systemGray2),
                        background: model.canSend ? Color(.label) : Color(.systemGray5)
                    )
                }
                .buttonStyle(.plain)
                .disabled(!model.canSend)
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
                                Label(group.title, systemImage: "link")
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
        model.draft = normalizedName + " "
        isFocused = true
    }

    private func configuredBot(for group: GaryxMobileBotGroup) -> GaryxConfiguredBot? {
        model.configuredBots.first {
            $0.channel.caseInsensitiveCompare(group.channel) == .orderedSame
                && $0.accountId == group.accountId
        }
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
        HStack(spacing: 7) {
            Image(systemName: attachment.kind == "image" ? "photo" : "doc")
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
}

struct GaryxTasksView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxPanelScaffold(
            title: "Tasks",
            subtitle: "\(model.activeTaskCount) active / \(model.tasks.count) total",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                if model.tasks.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "checklist",
                        title: "No tasks yet.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Tasks") {
                        GaryxCompactListGroup {
                            GaryxTaskList(tasks: model.tasks)
                        }
                    }
                }
            }
        }
    }
}

struct GaryxTaskList: View {
    let tasks: [GaryxTaskSummary]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(tasks.enumerated()), id: \.element.id) { index, task in
                GaryxTaskListRow(task: task)
                if index < tasks.count - 1 {
                    GaryxCompactRowDivider()
                }
            }
        }
    }
}

struct GaryxTaskListRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let task: GaryxTaskSummary

    var body: some View {
        GaryxSwipeActionRow(actions: taskSwipeActions) {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Button {
                        Task { await model.openThread(id: task.threadId) }
                    } label: {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(task.title)
                                .font(GaryxFont.body(weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(2)
                                .multilineTextAlignment(.leading)
                            Text(task.displayId)
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                    .disabled(task.threadId.isEmpty)

                    GaryxStatusPill(text: task.status.label, tone: task.status.tone)
                }

                HStack(spacing: 8) {
                    Text(task.assigneeDisplayLabel)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: 8)
                    Text(task.formattedUpdatedAt)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .padding(10)
            .contentShape(Rectangle())
        }
    }

    private var taskSwipeActions: [GaryxSwipeAction] {
        var actions: [GaryxSwipeAction] = []
        if !task.threadId.isEmpty {
            actions.append(
                GaryxSwipeAction(title: "Open", systemImage: "message", tone: .accent) {
                    Task { await model.openThread(id: task.threadId) }
                }
            )
        }
        if task.status == .inProgress {
            actions.append(
                GaryxSwipeAction(title: "Stop", systemImage: "stop.fill", tone: .warning) {
                    Task { await model.stopTask(task) }
                }
            )
        }
        actions.append(
            GaryxSwipeAction(title: task.status.nextActionLabel, systemImage: task.status.nextActionIcon) {
                Task { await model.updateTask(task, to: task.status.next) }
            }
        )
        if task.assignee != nil || !task.assigneeLabel.isEmpty {
            actions.append(
                GaryxSwipeAction(title: "Unassign", systemImage: "person.crop.circle.badge.xmark") {
                    Task { await model.unassignTask(task) }
                }
            )
        }
        actions.append(
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                Task { await model.deleteTask(task) }
            }
        )
        return actions
    }
}

struct GaryxTaskMetaLine: View {
    let label: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
                .textCase(.lowercase)
                .frame(width: 76, alignment: .leading)
            Text(value.isEmpty ? "Unknown" : value)
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}

struct GaryxAutomationsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateAutomation = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Automation",
            subtitle: "\(model.enabledAutomationCount) enabled",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                if let run = model.lastAutomationRun {
                    GaryxNotice(
                        title: "Last run \(run.status)",
                        text: run.excerpt ?? run.threadId
                    )
                }
                if model.automations.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "clock.badge",
                        title: "No automations yet. Create your first scheduled prompt.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Automation") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.automations.enumerated()), id: \.element.id) { index, automation in
                                GaryxAutomationCard(automation: automation)
                                if index < model.automations.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Automation") {
                showsCreateAutomation = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateAutomation) {
            GaryxFormSheet(title: "New Automation") {
                GaryxCreateAutomationCard()
            }
        }
    }
}

struct GaryxAutomationCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let automation: GaryxAutomationSummary
    @State private var showsEditForm = false
    @State private var label = ""
    @State private var prompt = ""
    @State private var intervalHours = ""

    var body: some View {
        GaryxSwipeActionRow(actions: automationSwipeActions) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "clock.arrow.circlepath")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(automation.label)
                            .font(GaryxFont.body(weight: .semibold))
                        Text(automation.workspacePath.isEmpty ? automation.agentId : automation.workspacePath.lastPathComponent)
                            .font(GaryxFont.caption(weight: .medium))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    GaryxStatusPill(text: automation.enabled ? "Enabled" : "Paused", tone: automation.enabled ? .good : .muted)
                }
                if !automation.prompt.isEmpty {
                    Text(automation.prompt)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Automation") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Automation")
                    TextField("Name", text: $label)
                        .garyxInputStyle()
                    TextField("Prompt", text: $prompt, axis: .vertical)
                        .lineLimit(2...5)
                        .garyxInputStyle()
                    TextField("Every", text: $intervalHours)
                        .keyboardType(.numberPad)
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateAutomation(
                                automation,
                                label: label,
                                prompt: prompt,
                                intervalHours: intervalHours
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
    }

    private var automationSwipeActions: [GaryxSwipeAction] {
        var actions: [GaryxSwipeAction] = []
        actions.append(
            GaryxSwipeAction(title: "Run", systemImage: "play.fill", tone: .accent) {
                guard automation.enabled else { return }
                Task { await model.runAutomation(automation) }
            }
        )
        actions.append(
            GaryxSwipeAction(title: automation.enabled ? "Pause" : "Resume", systemImage: automation.enabled ? "pause.fill" : "play.fill") {
                Task { await model.toggleAutomation(automation) }
            }
        )
        if let threadId = automation.threadId, !threadId.isEmpty {
            actions.append(
                GaryxSwipeAction(title: "Open", systemImage: "arrow.up.right") {
                    Task { await model.openThread(id: threadId) }
                }
            )
        }
        actions.append(
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            }
        )
        actions.append(
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                Task { await model.deleteAutomation(automation) }
            }
        )
        return actions
    }

    private func fillDraft() {
        label = automation.label
        prompt = automation.prompt
        intervalHours = String(automation.schedule.hours ?? 24)
    }
}

struct GaryxCreateAutomationCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                GaryxFieldLabel("New Automation")
                Spacer()
                Text(model.selectedWorkspacePath.isEmpty ? "Choose workspace" : model.selectedWorkspacePath.lastPathComponent)
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            TextField("Name", text: $model.draftAutomationLabel)
                .garyxInputStyle()
            TextField("Prompt", text: $model.draftAutomationPrompt, axis: .vertical)
                .lineLimit(2...5)
                .garyxInputStyle()

            HStack(spacing: 10) {
                TextField("Every", text: $model.draftAutomationIntervalHours)
                    .keyboardType(.numberPad)
                    .garyxInputStyle()
                    .frame(maxWidth: 140)

                Spacer(minLength: 0)

                Button {
                    Task {
                        if await model.createAutomationFromDraft() {
                            dismiss()
                        }
                    }
                } label: {
                    Label("Create", systemImage: "plus")
                }
                .buttonStyle(GaryxPrimaryCompactButtonStyle())
                .disabled(model.selectedWorkspacePath.isEmpty)
            }
        }
        .garyxCardStyle()
    }
}

private enum GaryxAgentCreationSheet: String, Identifiable {
    case agent
    case team

    var id: String { rawValue }

    var title: String {
        switch self {
        case .agent:
            "New Agent"
        case .team:
            "New Team"
        }
    }
}

private enum GaryxAgentsTab: String, CaseIterable, Identifiable {
    case agents = "Agents"
    case teams = "Teams"

    var id: String { rawValue }
}

struct GaryxAgentsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var creationSheet: GaryxAgentCreationSheet?
    @State private var selectedTab: GaryxAgentsTab = .agents

    var body: some View {
        GaryxPanelScaffold(
            title: "Agents",
            subtitle: "\(model.agents.count) agents / \(model.teams.count) teams",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                Picker("Agent type", selection: $selectedTab) {
                    ForEach(GaryxAgentsTab.allCases) { tab in
                        Text(tab.rawValue).tag(tab)
                    }
                }
                .pickerStyle(.segmented)

                switch selectedTab {
                case .agents:
                    GaryxSectionBlock(title: "Agents") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.agents.enumerated()), id: \.element.id) { index, agent in
                                GaryxAgentCard(agent: agent)
                                if index < model.agents.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                case .teams:
                    GaryxSectionBlock(title: "Teams") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.teams.enumerated()), id: \.element.id) { index, team in
                                GaryxTeamCard(team: team)
                                if index < model.teams.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: selectedTab == .agents ? "New Agent" : "New Team") {
                creationSheet = selectedTab == .agents ? .agent : .team
            }
        }
        .fullScreenCover(item: $creationSheet) { sheet in
            GaryxFormSheet(title: sheet.title) {
                switch sheet {
                case .agent:
                    GaryxCreateAgentCard()
                case .team:
                    GaryxCreateTeamCard()
                }
            }
        }
    }
}

struct GaryxCreateAgentCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("New Agent")
            TextField("Agent ID", text: $model.draftAgentId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Display name", text: $model.draftAgentName)
                .garyxInputStyle()
            TextField("Provider", text: $model.draftAgentProvider)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Model", text: $model.draftAgentModel)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Default workspace directory", text: $model.draftAgentWorkspace)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("System Prompt", text: $model.draftAgentPrompt, axis: .vertical)
                .lineLimit(2...6)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createAgentFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Create Agent", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxCreateTeamCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("New Team")
            TextField("Team ID", text: $model.draftTeamId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Display name", text: $model.draftTeamName)
                .garyxInputStyle()
            TextField("Leader Agent", text: $model.draftTeamLeaderId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Members", text: $model.draftTeamMemberIds)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Workflow", text: $model.draftTeamWorkflow, axis: .vertical)
                .lineLimit(2...6)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createTeamFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Create Team", systemImage: "person.2.badge.plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxAgentCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let agent: GaryxAgentSummary
    @State private var showsEditForm = false
    @State private var agentId = ""
    @State private var displayName = ""
    @State private var providerType = ""
    @State private var modelName = ""
    @State private var workspace = ""
    @State private var systemPrompt = ""

    var body: some View {
        GaryxSwipeActionRow(actions: agentSwipeActions) {
            VStack(alignment: .leading, spacing: 10) {
                GaryxAgentIdentityRow(
                    id: agent.id,
                    title: agent.displayName,
                    subtitle: "",
                    kind: .agent,
                    avatarDataUrl: agent.avatarDataUrl,
                    providerType: agent.providerType,
                    builtIn: agent.builtIn,
                    selected: model.selectedAgentTargetId == agent.id
                )
            }
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Agent") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Agent")
                    TextField("Agent ID", text: $agentId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Display name", text: $displayName)
                        .garyxInputStyle()
                    TextField("Provider", text: $providerType)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Model", text: $modelName)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Default workspace directory", text: $workspace)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("System Prompt", text: $systemPrompt, axis: .vertical)
                        .lineLimit(2...6)
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateAgent(
                                agent,
                                agentId: agentId,
                                displayName: displayName,
                                providerType: providerType,
                                modelName: modelName,
                                workspace: workspace,
                                systemPrompt: systemPrompt
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save Agent", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
    }

    private var agentSwipeActions: [GaryxSwipeAction] {
        var actions = [
            GaryxSwipeAction(title: "Chat", systemImage: "message", tone: .accent) {
                model.setSelectedAgentTarget(agent.id)
                Task { await model.createThread() }
            },
            GaryxSwipeAction(title: "Use", systemImage: "checkmark.circle") {
                model.setSelectedAgentTarget(agent.id)
            }
        ]
        if !agent.builtIn {
            actions.append(
                GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                    fillDraft()
                    showsEditForm = true
                }
            )
            actions.append(
                GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                    Task { await model.deleteAgent(agent) }
                }
            )
        }
        return actions
    }

    private func fillDraft() {
        agentId = agent.id
        displayName = agent.displayName
        providerType = agent.providerType
        modelName = agent.model
        workspace = agent.defaultWorkspaceDir
        systemPrompt = agent.systemPrompt
    }
}

struct GaryxTeamCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let team: GaryxTeamSummary
    @State private var showsEditForm = false
    @State private var teamId = ""
    @State private var displayName = ""
    @State private var leaderAgentId = ""
    @State private var memberAgentIds = ""
    @State private var workflowText = ""

    var body: some View {
        GaryxSwipeActionRow(actions: teamSwipeActions) {
            VStack(alignment: .leading, spacing: 10) {
                GaryxAgentIdentityRow(
                    id: team.id,
                    title: team.displayName,
                    subtitle: "",
                    kind: .team,
                    avatarDataUrl: team.avatarDataUrl,
                    providerType: "",
                    selected: model.selectedAgentTargetId == team.id
                )
                if !team.workflowText.isEmpty {
                    Text(team.workflowText)
                        .font(GaryxFont.footnote())
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .padding(.horizontal, 10)
                }
            }
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Team") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Team")
                    TextField("Team ID", text: $teamId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Display name", text: $displayName)
                        .garyxInputStyle()
                    TextField("Leader Agent", text: $leaderAgentId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Members", text: $memberAgentIds)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Workflow", text: $workflowText, axis: .vertical)
                        .lineLimit(2...6)
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateTeam(
                                team,
                                teamId: teamId,
                                displayName: displayName,
                                leaderAgentId: leaderAgentId,
                                memberAgentIds: memberAgentIds,
                                workflowText: workflowText
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save Team", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
    }

    private var teamSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "Chat", systemImage: "message", tone: .accent) {
                model.setSelectedAgentTarget(team.id)
                Task { await model.createThread() }
            },
            GaryxSwipeAction(title: "Use", systemImage: "checkmark.circle") {
                model.setSelectedAgentTarget(team.id)
            },
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                Task { await model.deleteTeam(team) }
            }
        ]
    }

    private func fillDraft() {
        teamId = team.id
        displayName = team.displayName
        leaderAgentId = team.leaderAgentId
        memberAgentIds = team.memberAgentIds.joined(separator: ", ")
        workflowText = team.workflowText
    }
}

struct GaryxSkillsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateSkill = false

    private var skillEditorPresented: Binding<Bool> {
        Binding(
            get: { model.selectedSkillEditor != nil },
            set: { isPresented in
                if !isPresented {
                    model.selectedSkillEditor = nil
                    model.selectedSkillDocument = nil
                    model.selectedSkillFileContent = ""
                }
            }
        )
    }

    var body: some View {
        GaryxPanelScaffold(
            title: "Skills",
            subtitle: "\(model.skills.filter(\.enabled).count) enabled / \(model.skills.count) total",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                if model.skills.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "wand.and.stars",
                        title: "No skills installed. Create your first skill.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Skills") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.skills.enumerated()), id: \.element.id) { index, skill in
                                GaryxSkillCard(skill: skill)
                                if index < model.skills.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Skill") {
                showsCreateSkill = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateSkill) {
            GaryxFormSheet(title: "New Skill") {
                GaryxCreateSkillCard()
            }
        }
        .fullScreenCover(isPresented: skillEditorPresented) {
            GaryxFormSheet(title: "Skill Editor") {
                GaryxSkillEditorCard()
            }
        }
    }
}

struct GaryxCreateSkillCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("New Skill")
            TextField("ID", text: $model.draftSkillId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Name", text: $model.draftSkillName)
                .garyxInputStyle()
            TextField("Description", text: $model.draftSkillDescription, axis: .vertical)
                .lineLimit(2...4)
                .garyxInputStyle()
            TextField("Body", text: $model.draftSkillBody, axis: .vertical)
                .lineLimit(2...5)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createSkillFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Create Skill", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxSkillCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let skill: GaryxSkillSummary
    @State private var showsEditForm = false
    @State private var name = ""
    @State private var description = ""

    var body: some View {
        GaryxSwipeActionRow(actions: skillSwipeActions) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "wand.and.stars")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(skill.name)
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(skill.description.isEmpty ? skill.sourcePath.lastPathComponent : skill.description)
                            .font(GaryxFont.caption(weight: .medium))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                    Spacer()
                    GaryxStatusPill(text: skill.enabled ? "Enabled" : "Paused", tone: skill.enabled ? .good : .muted)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Skill") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Skill")
                    TextField("Name", text: $name)
                        .garyxInputStyle()
                    TextField("Description", text: $description, axis: .vertical)
                        .lineLimit(2...4)
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateSkill(skill, name: name, description: description)
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
    }

    private var skillSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "Open", systemImage: "doc.text", tone: .accent) {
                Task { await model.openSkillEditor(skill) }
            },
            GaryxSwipeAction(title: skill.enabled ? "Disable" : "Enable", systemImage: skill.enabled ? "pause.fill" : "play.fill") {
                Task { await model.toggleSkill(skill) }
            },
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                Task { await model.deleteSkill(skill) }
            }
        ]
    }

    private func fillDraft() {
        name = skill.name
        description = skill.description
    }
}

struct GaryxSkillEditorCard: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        if let editor = model.selectedSkillEditor {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    GaryxFieldLabel("Skill Editor")
                    Spacer()
                    Text(editor.skill.name)
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.secondary)
                }

                ForEach(editor.entries) { node in
                    GaryxSkillEntryRow(skillId: editor.skill.id, node: node, depth: 0)
                }

                HStack(spacing: 8) {
                    TextField("path/to/file.md", text: $model.draftSkillEntryPath)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    Picker("Type", selection: $model.draftSkillEntryType) {
                        Text("New File").tag("file")
                        Text("New Folder").tag("directory")
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 148)
                }
                Button {
                    Task { await model.createSkillEntry() }
                } label: {
                    Label("Create", systemImage: "plus")
                }
                .buttonStyle(GaryxSecondaryButtonStyle())

                if let document = model.selectedSkillDocument {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(document.path)
                            .font(GaryxFont.caption(weight: .semibold))
                            .foregroundStyle(.secondary)
                        TextField("Content", text: $model.selectedSkillFileContent, axis: .vertical)
                            .lineLimit(6...16)
                            .garyxInputStyle()
                            .disabled(!document.editable)
                        Button {
                            Task { await model.saveSelectedSkillFile() }
                        } label: {
                            Label("Save", systemImage: "square.and.arrow.down")
                        }
                        .buttonStyle(GaryxPrimaryCompactButtonStyle())
                        .disabled(!document.editable)
                    }
                }
            }
            .garyxCardStyle()
        }
    }
}

struct GaryxSkillEntryRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let skillId: String
    let node: GaryxSkillEntryNode
    let depth: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: node.entryType == "directory" ? "folder.fill" : "doc.text")
                    .frame(width: 18)
                Button {
                    if node.entryType == "file" {
                        Task { await model.openSkillFile(skillId: skillId, path: node.path) }
                    }
                } label: {
                    Text(node.name)
                        .font(GaryxFont.callout(weight: .medium))
                        .lineLimit(1)
                }
                .buttonStyle(.plain)
                Spacer(minLength: 0)
                Button(role: .destructive) {
                    Task { await model.deleteSkillEntry(skillId: skillId, path: node.path) }
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(GaryxMiniIconButtonStyle())
            }
            .padding(.leading, CGFloat(depth) * 14)

            ForEach(node.children) { child in
                GaryxSkillEntryRow(skillId: skillId, node: child, depth: depth + 1)
            }
        }
    }
}

struct GaryxCommandsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateCommand = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Commands",
            subtitle: "\(model.slashCommands.count) shortcuts",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            GaryxCommandsContent()
        } actions: {
            GaryxAddToolbarButton(label: "Add Command") {
                showsCreateCommand = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateCommand) {
            GaryxFormSheet(title: "Add Command") {
                GaryxCreateSlashCommandCard()
            }
        }
    }
}

struct GaryxCommandsContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if model.slashCommands.isEmpty {
                GaryxEmptyPanelView(
                    icon: "command",
                    title: "No shortcuts yet",
                    text: ""
                )
            } else {
                GaryxSectionBlock(title: "Slash Commands") {
                    GaryxCompactListGroup {
                        ForEach(Array(model.slashCommands.enumerated()), id: \.element.id) { index, command in
                            GaryxSlashCommandCard(command: command)
                            if index < model.slashCommands.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

struct GaryxCreateSlashCommandCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Add Command")
            TextField("Command name", text: $model.draftSlashName)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Description", text: $model.draftSlashDescription)
                .garyxInputStyle()
            TextField("Content", text: $model.draftSlashPrompt, axis: .vertical)
                .lineLimit(2...5)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createSlashCommandFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Save Command", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxSlashCommandCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let command: GaryxSlashCommand
    @State private var showsEditForm = false
    @State private var name = ""
    @State private var description = ""
    @State private var prompt = ""

    var body: some View {
        GaryxSwipeActionRow(actions: commandSwipeActions) {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "command")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)

                    VStack(alignment: .leading, spacing: 3) {
                        Text("/\(command.name)")
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(command.description.isEmpty ? command.prompt : command.description)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                    Spacer(minLength: 8)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear {
            name = command.name
            description = command.description
            prompt = command.prompt
        }
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Command") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Command")
                    TextField("name", text: $name)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Description", text: $description)
                        .garyxInputStyle()
                    TextField("Prompt", text: $prompt, axis: .vertical)
                        .lineLimit(2...6)
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateSlashCommand(
                                command,
                                name: name,
                                description: description,
                                prompt: prompt
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
    }

    private var commandSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "Edit", systemImage: "pencil", tone: .accent) {
                name = command.name
                description = command.description
                prompt = command.prompt
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                Task { await model.deleteSlashCommand(command) }
            }
        ]
    }
}

struct GaryxMcpServersView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateMcp = false

    var body: some View {
        GaryxPanelScaffold(
            title: "MCP",
            subtitle: "\(model.mcpServers.filter(\.enabled).count) enabled / \(model.mcpServers.count) servers",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            GaryxMcpServersContent()
        } actions: {
            GaryxAddToolbarButton(label: "Add Server") {
                showsCreateMcp = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateMcp) {
            GaryxFormSheet(title: "Add Server") {
                GaryxCreateMcpServerCard()
            }
        }
    }
}

struct GaryxMcpServersContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if model.mcpServers.isEmpty {
                GaryxEmptyPanelView(
                    icon: "point.3.connected.trianglepath.dotted",
                    title: "No MCP servers yet",
                    text: ""
                )
            } else {
                GaryxSectionBlock(title: "MCP Servers") {
                    GaryxCompactListGroup {
                        ForEach(Array(model.mcpServers.enumerated()), id: \.element.id) { index, server in
                            GaryxMcpServerCard(server: server)
                            if index < model.mcpServers.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

struct GaryxCreateMcpServerCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Add Server")
            TextField("Name", text: $model.draftMcpName)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Start command", text: $model.draftMcpCommand)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Arguments", text: $model.draftMcpArgs)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Environment variables", text: $model.draftMcpEnv, axis: .vertical)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(2...4)
                .garyxInputStyle()
            TextField("Working directory", text: $model.draftMcpWorkingDir)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("URL", text: $model.draftMcpUrl)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Headers", text: $model.draftMcpHeaders, axis: .vertical)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(2...4)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createMcpServerFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Save", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxMcpServerCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let server: GaryxMcpServer
    @State private var showsEditForm = false
    @State private var name = ""
    @State private var command = ""
    @State private var args = ""
    @State private var env = ""
    @State private var workingDir = ""
    @State private var url = ""
    @State private var headers = ""

    var body: some View {
        GaryxSwipeActionRow(actions: serverSwipeActions) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "point.3.connected.trianglepath.dotted")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(server.name)
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(server.transport == "streamable_http" ? server.url ?? "HTTP" : server.command)
                            .font(GaryxFont.caption(weight: .medium))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer()
                    GaryxStatusPill(text: server.enabled ? "Enabled" : "Paused", tone: server.enabled ? .good : .muted)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit MCP Server") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("MCP Server")
                    TextField("Name", text: $name)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Start command", text: $command)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Arguments", text: $args)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Environment variables", text: $env, axis: .vertical)
                        .lineLimit(2...4)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Working directory", text: $workingDir)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("URL", text: $url)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Headers", text: $headers, axis: .vertical)
                        .lineLimit(2...4)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateMcpServer(
                                server,
                                name: name,
                                command: command,
                                argsText: args,
                                envText: env,
                                workingDir: workingDir,
                                url: url,
                                headersText: headers
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
    }

    private var serverSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: server.enabled ? "Disable" : "Enable", systemImage: server.enabled ? "pause.fill" : "play.fill", tone: .accent) {
                Task { await model.toggleMcpServer(server) }
            },
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                Task { await model.deleteMcpServer(server) }
            }
        ]
    }

    private func fillDraft() {
        name = server.name
        command = server.command
        args = server.args.joined(separator: ", ")
        env = server.env.map { "\($0.key)=\($0.value)" }.sorted().joined(separator: "\n")
        workingDir = server.workingDir ?? ""
        url = server.url ?? ""
        headers = server.headers.map { "\($0.key)=\($0.value)" }.sorted().joined(separator: "\n")
    }
}

struct GaryxAutoResearchView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateRun = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Auto Research",
            subtitle: "\(model.runningResearchCount) active / \(model.autoResearchRuns.count) total",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                if model.autoResearchRuns.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "atom",
                        title: "No Auto Research runs",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Auto Research") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.autoResearchRuns.enumerated()), id: \.element.id) { index, run in
                                GaryxAutoResearchRunCard(run: run)
                                if index < model.autoResearchRuns.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Auto Research Run") {
                showsCreateRun = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateRun) {
            GaryxFormSheet(title: "Create Auto Research Run") {
                GaryxCreateAutoResearchCard()
            }
        }
    }
}

struct GaryxCreateAutoResearchCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                GaryxFieldLabel("Create Auto Research Run")
                Spacer()
                Text(model.selectedWorkspacePath.isEmpty ? "No workspace" : model.selectedWorkspacePath.lastPathComponent)
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            TextField("Goal", text: $model.draftAutoResearchGoal, axis: .vertical)
                .lineLimit(2...5)
                .garyxInputStyle()
            HStack {
                TextField("Iterations", text: $model.draftAutoResearchIterations)
                    .keyboardType(.numberPad)
                    .garyxInputStyle()
                    .frame(maxWidth: 120)
                Spacer(minLength: 0)
                Button {
                    Task {
                        if await model.createAutoResearchRunFromDraft() {
                            dismiss()
                        }
                    }
                } label: {
                    Label("Start", systemImage: "play.fill")
                }
                .buttonStyle(GaryxPrimaryCompactButtonStyle())
            }
        }
        .garyxCardStyle()
    }
}

struct GaryxAutoResearchRunCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let run: GaryxAutoResearchRun

    var body: some View {
        GaryxSwipeActionRow(actions: researchSwipeActions) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "atom")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(run.goal.isEmpty ? run.runId : run.goal)
                            .font(GaryxFont.body(weight: .semibold))
                            .lineLimit(2)
                        Text(run.workspaceDir?.lastPathComponent ?? run.runId)
                            .font(GaryxFont.caption(weight: .medium))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    GaryxStatusPill(text: run.state, tone: researchTone)
                }
                Text("\(run.iterationsUsed)/\(run.maxIterations) iterations")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                if let page = model.researchCandidatesByRunId[run.runId] {
                    ForEach(page.candidates) { candidate in
                        GaryxResearchCandidateRow(
                            run: run,
                            candidate: candidate,
                            isBest: page.bestCandidateId == candidate.candidateId
                        )
                    }
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
    }

    private var researchSwipeActions: [GaryxSwipeAction] {
        var actions = [
            GaryxSwipeAction(title: "Candidates", systemImage: "list.bullet", tone: .accent) {
                Task { await model.loadAutoResearchCandidates(run) }
            }
        ]
        if researchTone != .muted {
            actions.append(
                GaryxSwipeAction(title: "Stop", systemImage: "stop.fill", tone: .warning) {
                    Task { await model.stopAutoResearchRun(run) }
                }
            )
        }
        actions.append(
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                Task { await model.deleteAutoResearchRun(run) }
            }
        )
        return actions
    }

    private var researchTone: GaryxStatusPill.Tone {
        switch run.state.lowercased() {
        case "completed":
            .good
        case "failed":
            .danger
        case "stopped", "cancelled":
            .muted
        default:
            .warning
        }
    }
}

struct GaryxResearchCandidateRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let run: GaryxAutoResearchRun
    let candidate: GaryxResearchCandidate
    let isBest: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Candidate \(candidate.iteration)")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                if isBest {
                    GaryxStatusPill(text: "Current best", tone: .good)
                }
                Spacer()
                if run.selectedCandidate != candidate.candidateId {
                    Button {
                        Task { await model.selectAutoResearchCandidate(run: run, candidate: candidate) }
                    } label: {
                        Text("Select winner")
                    }
                    .buttonStyle(GaryxSecondaryButtonStyle())
                } else {
                    GaryxStatusPill(text: "Selected Winner", tone: .good)
                }
            }
            Text(candidate.output)
                .font(GaryxFont.footnote())
                .foregroundStyle(.secondary)
                .lineLimit(5)
            if let verdict = candidate.verdict {
                Text("Score \(Int(verdict.score)) / \(verdict.feedback)")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
        .padding(10)
        .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 8))
    }
}

struct GaryxBotsView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxPanelScaffold(
            title: "Bots",
            subtitle: "\(model.mobileBotGroups.count) bots",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            GaryxBotsContent()
        }
    }
}

struct GaryxBotsContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        let groups = model.mobileBotGroups
        VStack(alignment: .leading, spacing: 10) {
            if groups.isEmpty {
                if !model.isLoadingRemoteState {
                    GaryxEmptyPanelView(
                        icon: "bubble.left.and.bubble.right",
                        title: "No bots configured",
                        text: ""
                    )
                }
            } else {
                GaryxSectionBlock(title: "Bots") {
                    GaryxCompactListGroup {
                        ForEach(Array(groups.enumerated()), id: \.element.id) { index, group in
                            GaryxBotGroupRow(group: group)
                            if index < groups.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

struct GaryxBotGroupRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let group: GaryxMobileBotGroup

    var body: some View {
        GaryxSwipeActionRow(actions: swipeActions) {
                HStack(alignment: .center, spacing: 10) {
                    GaryxChannelLogoView(
                        channel: group.channel,
                        label: group.title,
                        iconDataUrl: group.iconDataUrl,
                        diameter: 28
                    )

                    VStack(alignment: .leading, spacing: 3) {
                        Text(group.title)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(group.compactDetailLine)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }

                    Spacer(minLength: 6)
                }
                .padding(.horizontal, 9)
                .padding(.vertical, 8)
        }
    }

    private var swipeActions: [GaryxSwipeAction] {
        var actions: [GaryxSwipeAction] = []
        let rootBehavior = group.rootBehavior.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let mainThreadId = group.mainThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !mainThreadId.isEmpty || rootBehavior != "expand_only" {
            actions.append(
                GaryxSwipeAction(title: "Open", systemImage: "arrow.up.right", tone: .accent) {
                    Task { await model.openBotGroup(group) }
                }
            )
        }
        if let configuredBot {
            if model.selectedThread != nil {
                actions.append(
                    GaryxSwipeAction(title: "Bind", systemImage: "link") {
                        Task { await model.bindBotToSelectedThread(configuredBot) }
                    }
                )
            }
            if group.boundEndpointCount > 0 || group.defaultOpenThreadId?.isEmpty == false {
                actions.append(
                    GaryxSwipeAction(title: "Unbind", systemImage: "link.badge.minus") {
                        Task { await model.unbindBot(configuredBot) }
                    }
                )
            }
        }
        return actions
    }

    private var configuredBot: GaryxConfiguredBot? {
        model.configuredBots.first {
            $0.channel.caseInsensitiveCompare(group.channel) == .orderedSame
                && $0.accountId == group.accountId
        }
    }
}

struct GaryxMobileSettingsPanel: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsGatewaySetup = false
    @State private var showsCreateCommand = false
    @State private var showsCreateMcp = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Settings",
            subtitle: model.activeSettingsTab.label,
            onRefresh: { await model.connectAndRefresh() }
        ) {
            VStack(alignment: .leading, spacing: 12) {
                GaryxSettingsTabStrip()
                GaryxSettingsTabContent()
            }
        } actions: {
            switch model.activeSettingsTab {
            case .gateway:
                GaryxAddToolbarButton(label: "Add Gateway") {
                    model.gatewaySettingsStatus = nil
                    model.lastError = nil
                    showsGatewaySetup = true
                }
            case .commands:
                GaryxAddToolbarButton(label: "Add Command") {
                    showsCreateCommand = true
                }
            case .mcp:
                GaryxAddToolbarButton(label: "Add Server") {
                    showsCreateMcp = true
                }
            case .provider, .channels:
                EmptyView()
            }
        }
        .fullScreenCover(isPresented: $showsGatewaySetup) {
            GaryxGatewaySetupView(isSheet: true, startsEmpty: true)
        }
        .fullScreenCover(isPresented: $showsCreateCommand) {
            GaryxFormSheet(title: "Add Command") {
                GaryxCreateSlashCommandCard()
            }
        }
        .fullScreenCover(isPresented: $showsCreateMcp) {
            GaryxFormSheet(title: "Add Server") {
                GaryxCreateMcpServerCard()
            }
        }
    }
}

struct GaryxSettingsTabStrip: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(GaryxMobileSettingsTab.allCases) { tab in
                    Button {
                        model.activeSettingsTab = tab
                    } label: {
                        HStack(spacing: 5) {
                            Image(systemName: tab.iconName)
                                .font(GaryxFont.system(size: 12, weight: .semibold))
                            Text(tab.label)
                                .font(GaryxFont.footnote(weight: .semibold))
                        }
                        .foregroundStyle(model.activeSettingsTab == tab ? Color(.systemBackground) : .primary)
                        .padding(.horizontal, 9)
                        .frame(height: 30)
                        .background(
                            model.activeSettingsTab == tab ? Color(.label) : GaryxTheme.surface,
                            in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                        )
                        .overlay {
                            if model.activeSettingsTab != tab {
                                RoundedRectangle(cornerRadius: 8, style: .continuous)
                                    .stroke(GaryxTheme.hairline, lineWidth: 1)
                            }
                        }
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 1)
        }
    }
}

struct GaryxSettingsTabContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        switch model.activeSettingsTab {
        case .gateway:
            GaryxSettingsGatewayContent()
        case .provider:
            GaryxSettingsProviderContent()
        case .channels:
            GaryxBotsContent()
        case .commands:
            GaryxCommandsContent()
        case .mcp:
            GaryxMcpServersContent()
        }
    }
}

struct GaryxSettingsGatewayContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxSectionBlock(title: "Current") {
                GaryxGatewayCurrentRow()
            }

            if !model.gatewayProfiles.isEmpty {
                GaryxSectionBlock(title: "Gateways") {
                    GaryxCompactListGroup {
                        ForEach(Array(model.gatewayProfiles.enumerated()), id: \.element.id) { index, profile in
                            GaryxSavedGatewayProfileRow(
                                profile: profile,
                                isCurrent: model.currentGatewayProfile?.id == profile.id
                            )
                            if index < model.gatewayProfiles.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            } else {
                GaryxSectionBlock(title: "Gateways") {
                    GaryxGatewayEmptyProfilesRow()
                }
            }

            if let status = model.gatewaySettingsStatus, !status.isEmpty {
                Text(status)
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(GaryxTheme.accent)
                    .padding(.horizontal, 2)
            }
        }
    }
}

struct GaryxGatewayCurrentRow: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: currentIcon)
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(currentColor)
                .frame(width: 22, height: 22)

            VStack(alignment: .leading, spacing: 2) {
                Text(currentTitle)
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                Text(model.gatewayURL.isEmpty ? "No gateway selected" : model.gatewayURL)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 0)

            Button {
                Task { await model.connectAndRefresh() }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(GaryxFont.system(size: 13, weight: .semibold))
                    .frame(width: 34, height: 34)
                    .background(Color(.secondarySystemFill), in: Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Reconnect gateway")
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 8)
    }

    private var currentTitle: String {
        switch model.connectionState {
        case .ready(let version):
            if let version, !version.isEmpty {
                return "Connected \(version)"
            }
            return "Connected"
        case .checking:
            return "Connecting"
        case .failed:
            return "Connection failed"
        case .disconnected:
            return "Not connected"
        }
    }

    private var currentIcon: String {
        switch model.connectionState {
        case .ready:
            return "checkmark.circle.fill"
        case .checking:
            return "arrow.triangle.2.circlepath"
        case .failed:
            return "exclamationmark.circle.fill"
        case .disconnected:
            return "network"
        }
    }

    private var currentColor: Color {
        switch model.connectionState {
        case .ready:
            return GaryxTheme.accent
        case .checking:
            return .secondary
        case .failed:
            return GaryxTheme.danger
        case .disconnected:
            return .secondary
        }
    }
}

struct GaryxGatewayEmptyProfilesRow: View {
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "network")
                .font(GaryxFont.system(size: 14, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 22, height: 22)
            Text("No saved gateways")
                .font(GaryxFont.subheadline(weight: .medium))
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 9)
    }
}

struct GaryxGatewayProfileMenuButton: View {
    @EnvironmentObject private var model: GaryxMobileModel
    var onSelect: ((GaryxGatewayProfile) -> Void)?

    var body: some View {
        if model.gatewayProfiles.isEmpty {
            EmptyView()
        } else {
            Menu {
                ForEach(model.gatewayProfiles) { profile in
                    Button {
                        if let onSelect {
                            onSelect(profile)
                        } else {
                            Task { await model.activateGatewayProfile(profile) }
                        }
                    } label: {
                        Label(profile.gatewayUrl, systemImage: profile.hasToken ? "key.fill" : "network")
                    }
                }
            } label: {
                GaryxToolbarIcon(systemName: "clock.arrow.circlepath")
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Choose gateway")
        }
    }
}

struct GaryxSavedGatewayProfileRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let profile: GaryxGatewayProfile
    let isCurrent: Bool

    var body: some View {
        GaryxSwipeActionRow(actions: profileSwipeActions) {
            HStack(spacing: 9) {
                Image(systemName: isCurrent ? "checkmark.circle.fill" : "network")
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(isCurrent ? GaryxTheme.accent : .secondary)
                    .frame(width: 20, height: 20)

                VStack(alignment: .leading, spacing: 2) {
                    Text(profile.label)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(profile.gatewayUrl)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                if profile.hasToken {
                    Image(systemName: "key.fill")
                        .font(GaryxFont.system(size: 11, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 7)
            .contentShape(Rectangle())
            .onTapGesture {
                Task { await model.activateGatewayProfile(profile) }
            }
        }
    }

    private var profileSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "Switch", systemImage: "arrow.triangle.2.circlepath", tone: .accent) {
                Task { await model.activateGatewayProfile(profile) }
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                model.removeGatewayProfile(profile)
            }
        ]
    }
}

struct GaryxSettingsProviderContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxSectionBlock(title: "Default Agent") {
                GaryxCompactListGroup {
                    ForEach(Array(model.agentTargets.enumerated()), id: \.element.id) { index, target in
                        Button {
                            model.setSelectedAgentTarget(target.id)
                        } label: {
                            GaryxAgentIdentityRow(
                                id: target.id,
                                title: target.title,
                                subtitle: target.subtitle,
                                kind: target.kind,
                                avatarDataUrl: target.avatarDataUrl,
                                providerType: target.providerType,
                                builtIn: target.builtIn,
                                selected: model.selectedAgentTargetId == target.id
                            )
                        }
                        .buttonStyle(.plain)
                        if index < model.agentTargets.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                }
            }
        }
    }
}

struct GaryxPanelScaffold<Content: View, Actions: View>: View {
    @EnvironmentObject private var model: GaryxMobileModel

    let title: String
    let subtitle: String
    let onRefresh: (() async -> Void)?
    let content: Content
    let actions: Actions

    init(
        title: String,
        subtitle: String,
        onRefresh: (() async -> Void)? = nil,
        @ViewBuilder content: () -> Content,
        @ViewBuilder actions: () -> Actions
    ) {
        self.title = title
        self.subtitle = subtitle
        self.onRefresh = onRefresh
        self.content = content()
        self.actions = actions()
    }

    var body: some View {
        ScrollView {
            content
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .frame(maxWidth: 560, alignment: .leading)
                .frame(maxWidth: .infinity)
        }
        .refreshable {
            if let onRefresh {
                await onRefresh()
            }
        }
        .background(GaryxTheme.background)
        .garyxAdaptiveTopBar {
            HStack(spacing: 10) {
                GaryxSidebarMenuButton {
                    model.setSidebarVisible(true)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(GaryxFont.body(weight: .medium))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer()

                if let onRefresh {
                    Button {
                        Task { await onRefresh() }
                    } label: {
                        GaryxToolbarIcon(systemName: "arrow.clockwise")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Refresh")
                }

                actions
            }
            .padding(.horizontal, 16)
            .padding(.top, 8)
            .padding(.bottom, 8)
            .modifier(GaryxSidebarHeaderBackdropModifier())
        }
        .garyxAdaptiveSoftScrollEdge(for: .top)
    }
}

extension GaryxPanelScaffold where Actions == EmptyView {
    init(
        title: String,
        subtitle: String,
        onRefresh: (() async -> Void)? = nil,
        @ViewBuilder content: () -> Content
    ) {
        self.init(
            title: title,
            subtitle: subtitle,
            onRefresh: onRefresh,
            content: content,
            actions: { EmptyView() }
        )
    }
}

struct GaryxAddToolbarButton: View {
    let label: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            GaryxToolbarIcon(systemName: "plus")
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

struct GaryxFormSheet<Content: View>: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                content
                    .padding(12)
                    .frame(maxWidth: 620, alignment: .leading)
                    .frame(maxWidth: .infinity)
            }
            .background(GaryxTheme.background)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Done") {
                        dismiss()
                    }
                }
            }
        }
    }
}

struct GaryxSectionBlock<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            GaryxFieldLabel(title)
            VStack(alignment: .leading, spacing: 10) {
                content
            }
        }
    }
}

struct GaryxCompactListGroup<Content: View>: View {
    let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

struct GaryxCompactRowDivider: View {
    var body: some View {
        Divider()
            .overlay(GaryxTheme.hairline)
            .padding(.leading, 10)
    }
}

struct GaryxSwipeAction {
    enum Tone {
        case accent
        case neutral
        case warning
        case destructive

        var background: Color {
            switch self {
            case .accent:
                GaryxTheme.accent
            case .neutral:
                Color(.systemGray3)
            case .warning:
                GaryxTheme.warning
            case .destructive:
                GaryxTheme.danger
            }
        }
    }

    let title: String
    let systemImage: String
    var tone: Tone = .neutral
    let action: () -> Void
}

struct GaryxSwipeActionRow<Content: View>: View {
    private enum DragAxis {
        case horizontal
        case vertical
    }

    let actions: [GaryxSwipeAction]
    let content: Content
    @State private var isOpen = false
    @State private var offset: CGFloat = 0
    @State private var dragAxis: DragAxis?

    init(actions: [GaryxSwipeAction], @ViewBuilder content: () -> Content) {
        self.actions = actions
        self.content = content()
    }

    var body: some View {
        if actions.isEmpty {
            content
        } else {
            ZStack(alignment: .trailing) {
                HStack(spacing: 0) {
                    ForEach(Array(visibleActions.enumerated()), id: \.offset) { _, action in
                        Button(role: action.tone == .destructive ? .destructive : nil) {
                            close()
                            action.action()
                        } label: {
                            VStack(spacing: 4) {
                                Image(systemName: action.systemImage)
                                    .font(GaryxFont.system(size: 14, weight: .semibold))
                                Text(action.title)
                                    .font(GaryxFont.system(size: 11, weight: .semibold))
                                    .lineLimit(1)
                                    .minimumScaleFactor(0.75)
                            }
                            .foregroundStyle(.white)
                            .frame(width: actionButtonWidth)
                            .frame(maxHeight: .infinity)
                            .padding(.vertical, 8)
                            .background(action.tone.background)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .frame(width: actionWidth, alignment: .trailing)
                .frame(maxHeight: .infinity)
                .offset(x: actionWidth + offset)

                content
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(GaryxTheme.surface)
                    .offset(x: offset)
                    .contentShape(Rectangle())
                    .overlay {
                        if isOpen {
                            Color.clear
                                .contentShape(Rectangle())
                                .onTapGesture {
                                    close()
                                }
                        }
                    }
                    .simultaneousGesture(swipeGesture)
                    .contextMenu {
                        ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                            Button(action.title, role: action.tone == .destructive ? .destructive : nil) {
                                close()
                                action.action()
                            }
                        }
                    }
                    .accessibilityHint("Swipe left for actions, or use the actions rotor.")
            }
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .clipped()
        }
    }

    private var actionButtonWidth: CGFloat { 72 }
    private var actionWidth: CGFloat { CGFloat(visibleActions.count) * actionButtonWidth }

    private var visibleActions: [GaryxSwipeAction] {
        actions
    }

    private var swipeGesture: some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .local)
            .onChanged { value in
                if dragAxis == nil {
                    let horizontal = abs(value.translation.width)
                    let vertical = abs(value.translation.height)
                    guard max(horizontal, vertical) > 8 else { return }
                    dragAxis = horizontal > vertical * 1.25 ? .horizontal : .vertical
                }
                guard dragAxis == .horizontal else { return }
                let base = isOpen ? -actionWidth : 0
                offset = min(0, max(-actionWidth, base + value.translation.width))
            }
            .onEnded { value in
                defer { dragAxis = nil }
                guard dragAxis == .horizontal else {
                    if !isOpen {
                        offset = 0
                    }
                    return
                }
                let shouldOpen = offset < -actionWidth * 0.35
                    || value.predictedEndTranslation.width < -actionWidth * 0.75
                isOpen = shouldOpen
                withAnimation(GaryxMobileMotion.rowSwipe) {
                    offset = shouldOpen ? -actionWidth : 0
                }
            }
    }

    private func close() {
        isOpen = false
        withAnimation(GaryxMobileMotion.rowSwipe) {
            offset = 0
        }
    }
}

struct GaryxCompactInfoRow: View {
    let title: String
    let subtitle: String
    let iconName: String

    var body: some View {
        HStack(spacing: 9) {
            Image(systemName: iconName)
                .font(GaryxFont.system(size: 14, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: 20, height: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
    }
}

struct GaryxChannelLogoView: View {
    let channel: String
    let label: String
    let iconDataUrl: String?
    var diameter: CGFloat = 30

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: diameter * 0.28, style: .continuous)
                .fill(Color(.secondarySystemFill))

            if let image = decodedImage {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .padding(diameter * 0.16)
            } else {
                Text(fallbackLabel)
                    .font(GaryxFont.system(size: diameter * 0.34, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.65)
            }
        }
        .frame(width: diameter, height: diameter)
        .overlay {
            RoundedRectangle(cornerRadius: diameter * 0.28, style: .continuous)
                .stroke(Color.primary.opacity(0.06), lineWidth: 1)
        }
        .accessibilityHidden(true)
    }

    private var decodedImage: UIImage? {
        let raw = (iconDataUrl ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return nil }
        let encoded = raw.split(separator: ",", maxSplits: 1).last.map(String.init) ?? raw
        guard let data = Data(base64Encoded: encoded) else { return nil }
        return UIImage(data: data)
    }

    private var fallbackLabel: String {
        let source = label.isEmpty ? channel : label
        let words = source
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")
            .split(separator: " ")
        let initials = words.prefix(2).compactMap { $0.first }.map { String($0).uppercased() }.joined()
        return initials.isEmpty ? "B" : initials
    }
}

private enum GaryxProviderAvatar {
    case codex
    case openAI
    case claude
    case gemini
    case generic

    var symbol: String? {
        switch self {
        case .codex:
            "chevron.left.forwardslash.chevron.right"
        case .openAI:
            "circle.hexagongrid.fill"
        case .claude:
            "sparkles"
        case .gemini:
            "diamond.fill"
        case .generic:
            nil
        }
    }

    var background: Color {
        switch self {
        case .codex:
            Color(red: 0.08, green: 0.10, blue: 0.12)
        case .openAI:
            Color(red: 0.10, green: 0.47, blue: 0.40)
        case .claude:
            Color(red: 0.50, green: 0.37, blue: 0.26)
        case .gemini:
            Color(red: 0.23, green: 0.38, blue: 0.86)
        case .generic:
            Color(.secondarySystemBackground)
        }
    }

    var foreground: Color {
        switch self {
        case .generic:
            Color(.secondaryLabel)
        default:
            Color.white
        }
    }

    func iconSize(for diameter: CGFloat) -> CGFloat {
        switch self {
        case .codex:
            diameter * 0.32
        case .openAI:
            diameter * 0.42
        case .claude:
            diameter * 0.40
        case .gemini:
            diameter * 0.34
        case .generic:
            diameter * 0.36
        }
    }
}

struct GaryxAgentAvatarView: View {
    let agentId: String
    let avatarDataUrl: String
    let kind: GaryxMobileAgentTarget.Kind
    let label: String
    let providerType: String
    var builtIn: Bool = false
    var diameter: CGFloat = 34

    var body: some View {
        ZStack {
            Circle()
                .fill(fallbackBackground)
            if let image = decodedImage {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
                    .frame(width: diameter, height: diameter)
                    .clipShape(Circle())
            } else if let remoteAvatarURL {
                AsyncImage(url: remoteAvatarURL) { phase in
                    if let image = phase.image {
                        image
                            .resizable()
                            .scaledToFill()
                    } else {
                        fallbackContent
                    }
                }
                .frame(width: diameter, height: diameter)
                .clipShape(Circle())
            } else if kind == .team {
                fallbackContent
            } else {
                fallbackContent
            }
        }
        .frame(width: diameter, height: diameter)
        .overlay {
            Circle()
                .stroke(Color.primary.opacity(0.06), lineWidth: 1)
        }
        .accessibilityHidden(true)
    }

    private var decodedImage: UIImage? {
        let raw = avatarDataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return nil }
        let encoded = raw.split(separator: ",", maxSplits: 1).last.map(String.init) ?? raw
        guard let data = Data(base64Encoded: encoded) else { return nil }
        return UIImage(data: data)
    }

    private var remoteAvatarURL: URL? {
        let raw = avatarDataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard raw.hasPrefix("http://") || raw.hasPrefix("https://") else { return nil }
        return URL(string: raw)
    }

    @ViewBuilder
    private var fallbackContent: some View {
        if kind == .team {
            Image(systemName: "person.2.fill")
                .font(GaryxFont.system(size: diameter * 0.36, weight: .semibold))
                .foregroundStyle(fallbackForeground)
        } else if let symbol = providerAvatar.symbol {
            Image(systemName: symbol)
                .font(GaryxFont.system(size: providerAvatar.iconSize(for: diameter), weight: .semibold))
                .foregroundStyle(fallbackForeground)
        } else {
            Text(agentInitials)
                .font(GaryxFont.system(size: diameter * 0.32, weight: .bold))
                .foregroundStyle(fallbackForeground)
        }
    }

    private var providerAvatar: GaryxProviderAvatar {
        let source = "\(agentId) \(providerType)".lowercased()
        if source.contains("codex") {
            return .codex
        }
        if source.contains("openai") || source.contains("gpt") {
            return .openAI
        }
        if source.contains("claude") || source.contains("anthropic") {
            return .claude
        }
        if source.contains("gemini") || source.contains("google") {
            return .gemini
        }
        return .generic
    }

    private var agentInitials: String {
        let source = (label.isEmpty ? agentId : label).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !source.isEmpty else { return "A" }
        let words = source
            .replacingOccurrences(of: "(", with: " ")
            .replacingOccurrences(of: ")", with: " ")
            .split { $0 == " " || $0 == "/" || $0 == "_" || $0 == "-" }
        if words.count >= 2, let first = words[0].first, let second = words[1].first {
            return "\(first)\(second)".uppercased()
        }
        return String(source.prefix(2)).uppercased()
    }

    private var fallbackBackground: Color {
        if builtIn, kind == .agent {
            return providerAvatar.background
        }

        let colors = [
            Color(red: 0.88, green: 0.95, blue: 0.90),
            Color(red: 0.90, green: 0.92, blue: 0.98),
            Color(red: 0.96, green: 0.91, blue: 0.84),
            Color(red: 0.91, green: 0.94, blue: 0.96)
        ]
        let seed = (label + agentId).unicodeScalars.reduce(0) { ($0 &+ Int($1.value)) % 997 }
        return colors[seed % colors.count]
    }

    private var fallbackForeground: Color {
        if kind == .team {
            return Color(.systemGray)
        }
        if builtIn {
            return providerAvatar.foreground
        }
        return GaryxTheme.accent
    }
}

struct GaryxAgentIdentityRow: View {
    let id: String
    let title: String
    let subtitle: String
    let kind: GaryxMobileAgentTarget.Kind
    let avatarDataUrl: String
    let providerType: String
    var builtIn: Bool = false
    let selected: Bool

    var body: some View {
        HStack(spacing: 12) {
            GaryxAgentAvatarView(
                agentId: id,
                avatarDataUrl: avatarDataUrl,
                kind: kind,
                label: title,
                providerType: providerType,
                builtIn: builtIn
            )
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(GaryxFont.body(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer()
            if selected {
                Image(systemName: "checkmark.circle.fill")
                    .font(GaryxFont.system(size: 19, weight: .semibold))
                    .foregroundStyle(GaryxTheme.accent)
            }
        }
        .padding(10)
        .contentShape(Rectangle())
    }
}

struct GaryxSelectableRow: View {
    let title: String
    let subtitle: String
    let iconName: String
    let selected: Bool

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: iconName)
                .foregroundStyle(selected ? GaryxTheme.accent : .secondary)
                .frame(width: 28, height: 28)
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(GaryxFont.body(weight: .medium))
                    .foregroundStyle(.primary)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer()
            if selected {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(GaryxTheme.accent)
            }
        }
        .padding(10)
        .contentShape(Rectangle())
    }
}

struct GaryxInfoRow: View {
    let title: String
    let subtitle: String
    let iconName: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: iconName)
                .foregroundStyle(GaryxTheme.accent)
                .frame(width: 28, height: 28)
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(GaryxFont.body(weight: .medium))
                    .foregroundStyle(.primary)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            Spacer()
        }
        .padding(9)
        .contentShape(Rectangle())
    }
}

struct GaryxStatusPill: View {
    enum Tone: Equatable {
        case good
        case warning
        case danger
        case muted
    }

    let text: String
    let tone: Tone

    var body: some View {
        Text(text)
            .font(GaryxFont.system(size: 11, weight: .semibold))
            .foregroundStyle(color)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(color.opacity(0.10), in: Capsule())
    }

    private var color: Color {
        switch tone {
        case .good:
            GaryxTheme.accent
        case .warning:
            GaryxTheme.warning
        case .danger:
            GaryxTheme.danger
        case .muted:
            .secondary
        }
    }
}

struct GaryxNotice: View {
    let title: String
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(title)
                .font(GaryxFont.footnote(weight: .semibold))
                .foregroundStyle(.primary)
            Text(text)
                .font(GaryxFont.callout())
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(10)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }
}

struct GaryxGlobalErrorToastHost: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let topOffset: CGFloat

    @State private var visibleError: String?
    @State private var toastToken = 0

    var body: some View {
        Group {
            if let visibleError {
                GaryxGlobalErrorToast(text: visibleError) {
                    hide(message: visibleError)
                }
                .padding(.horizontal, 18)
                .padding(.top, topOffset)
                .transition(toastTransition)
                .zIndex(100)
            }
        }
        .frame(maxWidth: .infinity, alignment: .top)
        .onAppear {
            present(model.lastError)
        }
        .onChange(of: model.lastError) { _, newValue in
            present(newValue)
        }
        .task(id: toastToken) {
            guard let message = visibleError else { return }
            try? await Task.sleep(nanoseconds: 3_200_000_000)
            guard !Task.isCancelled else { return }
            await MainActor.run {
                hide(message: message)
            }
        }
    }

    private var toastTransition: AnyTransition {
        if reduceMotion {
            return .opacity
        }
        return .move(edge: .top).combined(with: .opacity)
    }

    private var toastAnimation: Animation? {
        reduceMotion ? nil : .easeOut(duration: 0.18)
    }

    private func present(_ message: String?) {
        let trimmed = message?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !trimmed.isEmpty else {
            toastToken += 1
            withAnimation(toastAnimation) {
                visibleError = nil
            }
            return
        }

        toastToken += 1
        withAnimation(toastAnimation) {
            visibleError = trimmed
        }
    }

    private func hide(message: String) {
        guard visibleError == message else { return }
        toastToken += 1
        withAnimation(toastAnimation) {
            visibleError = nil
        }
        if model.lastError == message {
            model.lastError = nil
        }
    }
}

struct GaryxGlobalErrorToast: View {
    let text: String
    let onDismiss: () -> Void

    var body: some View {
        Button(action: onDismiss) {
            HStack(spacing: 9) {
                Image(systemName: "exclamationmark.circle.fill")
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(GaryxTheme.danger.opacity(0.86))

                Text(text)
                    .font(GaryxFont.footnote(weight: .medium))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)

                Spacer(minLength: 2)

                Image(systemName: "xmark")
                    .font(GaryxFont.system(size: 10, weight: .bold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .frame(maxWidth: 360, alignment: .leading)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
            .shadow(color: Color.black.opacity(0.10), radius: 18, y: 8)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(text)
        .accessibilityHint("Dismiss")
    }
}

struct GaryxEmptyPanelView: View {
    let icon: String
    let title: String
    let text: String

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: icon)
                .font(GaryxFont.title2(weight: .medium))
                .foregroundStyle(.secondary)
            Text(title)
                .font(GaryxFont.body(weight: .semibold))
                .foregroundStyle(.primary)
            if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(text)
                    .font(GaryxFont.callout())
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 24)
        .padding(.vertical, 36)
    }
}

struct GaryxFieldLabel: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(GaryxFont.caption(weight: .semibold))
            .foregroundStyle(.secondary)
            .textCase(.uppercase)
    }
}

struct GaryxAppLogo: View {
    var size: CGFloat
    var cornerRadius: CGFloat = 22
    var fontSize: CGFloat = 24

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                .fill(Color(.label))

            Text("GX")
                .font(.system(size: fontSize, weight: .semibold, design: .rounded))
                .foregroundStyle(Color(.systemBackground))
        }
        .frame(width: size, height: size)
    }
}

struct GaryxConnectionPill: View {
    let label: String
    let color: Color
    let isBusy: Bool

    @State private var dotPulse = false

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 6, height: 6)
                .scaleEffect(dotPulse ? 1.4 : 1.0)
                .opacity(dotPulse ? 0.6 : 1.0)
                .animation(
                    isBusy
                        ? .easeInOut(duration: 0.8).repeatForever(autoreverses: true)
                        : .default,
                    value: dotPulse
                )

            Text(label)
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
        .background(Capsule().fill(Color(.systemBackground)))
        .overlay(Capsule().stroke(GaryxTheme.hairline, lineWidth: 1))
        .onAppear {
            dotPulse = isBusy
        }
        .onChange(of: isBusy) { _, newValue in
            dotPulse = newValue
        }
    }
}

struct GaryxToolbarIcon: View {
    var systemName: String?
    var customContent: (() -> AnyView)?

    init(systemName: String) {
        self.systemName = systemName
        self.customContent = nil
    }

    init<Content: View>(@ViewBuilder content: @escaping () -> Content) {
        self.systemName = nil
        self.customContent = { AnyView(content()) }
    }

    var body: some View {
        Group {
            if let systemName {
                Image(systemName: systemName)
                    .font(GaryxFont.system(size: 16, weight: .semibold))
                    .foregroundStyle(.primary)
            } else if let customContent {
                customContent()
            }
        }
        .frame(width: 36, height: 36)
        .contentShape(Circle())
        .background {
            Circle()
                .fill(Color(.systemBackground).opacity(0.42))
                .background(.ultraThinMaterial, in: Circle())
        }
        .overlay {
            Circle()
                .stroke(Color.primary.opacity(0.03), lineWidth: 1)
        }
    }
}

struct GaryxSidebarMenuButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            GaryxHeaderMenuIcon()
                .frame(width: 48, height: 48)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .simultaneousGesture(TapGesture().onEnded { _ in action() })
        .accessibilityLabel("Open menu")
    }
}

struct GaryxHeaderMenuIcon: View {
    var body: some View {
        Image(systemName: "line.3.horizontal")
            .font(GaryxFont.system(size: 17, weight: .semibold))
            .foregroundStyle(.primary)
            .frame(width: 44, height: 44)
            .background {
                Circle()
                    .fill(Color(.systemBackground).opacity(0.42))
                    .background(.ultraThinMaterial, in: Circle())
            }
            .overlay {
                Circle()
                    .stroke(Color.primary.opacity(0.032), lineWidth: 1)
            }
            .contentShape(Circle())
    }
}

struct GaryxCircleBadge: View {
    let systemName: String
    let foreground: Color
    let background: Color
    var diameter: CGFloat = 32
    var iconSize: CGFloat = 12
    var iconWeight: Font.Weight = .bold

    var body: some View {
        Image(systemName: systemName)
            .font(GaryxFont.system(size: iconSize, weight: iconWeight))
            .foregroundStyle(foreground)
            .frame(width: diameter, height: diameter)
            .background(background, in: Circle())
    }
}

struct GaryxPrimaryCapsuleButton: View {
    let title: String
    var systemImage: String? = nil
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                if let systemImage, !systemImage.isEmpty {
                    Image(systemName: systemImage)
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                }

                Text(title)
                    .font(GaryxFont.body(weight: .semibold))
            }
            .foregroundStyle(Color(.systemBackground))
            .frame(maxWidth: .infinity)
            .frame(height: 56)
            .background(Color(.label), in: Capsule())
        }
        .buttonStyle(.plain)
    }
}

enum GaryxTheme {
    static let background = Color(.systemBackground)
    static let sidebar = Color(.systemBackground)
    static let header = Color(.systemBackground)
    static let surface = Color(.secondarySystemGroupedBackground)
    static let input = Color(.secondarySystemGroupedBackground)
    static let primaryText = Color.primary
    static let secondaryText = Color.secondary
    static let tertiaryText = Color(.tertiaryLabel)
    static let accent = Color(red: 0.000, green: 0.635, blue: 0.250)
    static let warning = Color.orange
    static let danger = Color.red
    static let hairline = Color.primary.opacity(0.08)
}

enum GaryxFont {
    static func largeTitle(weight: Font.Weight = .regular) -> Font {
        .system(size: 34, weight: weight)
    }

    static func title2(weight: Font.Weight = .regular) -> Font {
        .system(size: 22, weight: weight)
    }

    static func title3(weight: Font.Weight = .regular) -> Font {
        .system(size: 20, weight: weight)
    }

    static func body(weight: Font.Weight = .regular) -> Font {
        .system(size: 17, weight: weight)
    }

    static func callout(weight: Font.Weight = .regular) -> Font {
        .system(size: 16, weight: weight)
    }

    static func subheadline(weight: Font.Weight = .regular) -> Font {
        .system(size: 15, weight: weight)
    }

    static func footnote(weight: Font.Weight = .regular) -> Font {
        .system(size: 13, weight: weight)
    }

    static func caption(weight: Font.Weight = .regular) -> Font {
        .system(size: 12, weight: weight)
    }

    static func system(size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight)
    }
}

struct GaryxPrimaryCompactButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.footnote(weight: .semibold))
            .foregroundStyle(Color(.systemBackground))
            .padding(.vertical, 6)
            .padding(.horizontal, 9)
            .background(Color(.label).opacity(configuration.isPressed ? 0.72 : 1), in: Capsule())
    }
}

struct GaryxSecondaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.footnote(weight: .semibold))
            .foregroundStyle(.primary)
            .padding(.vertical, 6)
            .padding(.horizontal, 9)
            .background(.thinMaterial, in: Capsule())
            .opacity(configuration.isPressed ? 0.72 : 1)
    }
}

struct GaryxMiniIconButtonStyle: ButtonStyle {
    var isPrimary = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.system(size: 13, weight: .semibold))
            .foregroundStyle(isPrimary ? Color(.systemBackground) : Color.primary)
            .frame(width: 28, height: 28)
            .background(
                isPrimary
                    ? Color(.label).opacity(configuration.isPressed ? 0.72 : 1)
                    : Color.primary.opacity(configuration.isPressed ? 0.07 : 0),
                in: RoundedRectangle(cornerRadius: 7, style: .continuous)
            )
    }
}

struct GaryxIconButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.system(size: 15, weight: .semibold))
            .foregroundStyle(.primary)
            .frame(width: 32, height: 32)
            .background(.thinMaterial, in: Circle())
            .opacity(configuration.isPressed ? 0.72 : 1)
    }
}

struct GaryxStopButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.white)
            .frame(width: 32, height: 32)
            .background(GaryxTheme.danger.opacity(configuration.isPressed ? 0.72 : 1), in: Circle())
    }
}

private struct GaryxAdaptiveGlassModifier<S: Shape>: ViewModifier {
    let style: GaryxAdaptiveGlassStyle
    let isInteractive: Bool
    let tint: Color?
    let fallbackMaterial: Material
    let shape: S

    @ViewBuilder
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            switch style {
            case .automatic:
                content.glassEffect(in: shape)
            case .regular:
                content.glassEffect(resolvedGlass, in: shape)
            }
        } else if let tint {
            content.background(tint, in: shape)
        } else {
            content.background(fallbackMaterial, in: shape)
        }
    }

    @available(iOS 26, *)
    private var resolvedGlass: Glass {
        var glass = Glass.regular
        if let tint {
            glass = glass.tint(tint)
        }
        if isInteractive {
            glass = glass.interactive()
        }
        return glass
    }
}

enum GaryxAdaptiveGlassStyle {
    case automatic
    case regular
}

struct GaryxAdaptiveGlassContainer<Content: View>: View {
    let spacing: CGFloat
    private let content: () -> Content

    init(spacing: CGFloat, @ViewBuilder content: @escaping () -> Content) {
        self.spacing = spacing
        self.content = content
    }

    var body: some View {
        if #available(iOS 26, *) {
            GlassEffectContainer(spacing: spacing) {
                content()
            }
        } else {
            content()
        }
    }
}

private struct GaryxSoftScrollEdgeModifier: ViewModifier {
    let edges: Edge.Set

    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.scrollEdgeEffectStyle(.soft, for: edges)
        } else {
            content
        }
    }
}

private struct GaryxSidebarHeaderBackdropModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .background(Color(.systemBackground).opacity(0.92))
    }
}

extension View {
    func garyxInputStyle() -> some View {
        self
            .font(GaryxFont.body())
            .foregroundStyle(.primary)
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(GaryxTheme.input, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
    }

    func garyxCardStyle() -> some View {
        self
            .padding(8)
            .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
    }

    func garyxAdaptiveGlass(_ style: GaryxAdaptiveGlassStyle, in shape: some Shape) -> some View {
        garyxAdaptiveGlass(style, isInteractive: false, tint: nil, fallbackMaterial: .thinMaterial, in: shape)
    }

    func garyxAdaptiveGlass(
        _ style: GaryxAdaptiveGlassStyle,
        isInteractive: Bool,
        tint: Color? = nil,
        fallbackMaterial: Material = .thinMaterial,
        in shape: some Shape
    ) -> some View {
        modifier(
            GaryxAdaptiveGlassModifier(
                style: style,
                isInteractive: isInteractive,
                tint: tint,
                fallbackMaterial: fallbackMaterial,
                shape: shape
            )
        )
    }

    func garyxAdaptiveGlass(in shape: some Shape) -> some View {
        garyxAdaptiveGlass(.automatic, isInteractive: false, tint: nil, fallbackMaterial: .thinMaterial, in: shape)
    }

    func garyxAdaptiveSoftScrollEdge(for edges: Edge.Set) -> some View {
        modifier(GaryxSoftScrollEdgeModifier(edges: edges))
    }

    @ViewBuilder
    func garyxAdaptiveTopBar<Bar: View>(@ViewBuilder _ bar: () -> Bar) -> some View {
        self.safeAreaInset(edge: .top, spacing: 0, content: bar)
    }

    @ViewBuilder
    func `if`<Content: View>(_ condition: Bool, transform: (Self) -> Content) -> some View {
        if condition {
            transform(self)
        } else {
            self
        }
    }
}

private extension GaryxTaskStatus {
    var label: String {
        switch self {
        case .todo:
            "Todo"
        case .inProgress:
            "In Progress"
        case .inReview:
            "In Review"
        case .done:
            "Done"
        }
    }

    var nextActionLabel: String {
        switch self {
        case .todo:
            "Start"
        case .inProgress:
            "Send to Review"
        case .inReview:
            "Done"
        case .done:
            "Reopen"
        }
    }

    var nextActionIcon: String {
        switch self {
        case .todo:
            "play.fill"
        case .inProgress:
            "arrowshape.turn.up.right.fill"
        case .inReview:
            "checkmark"
        case .done:
            "arrow.counterclockwise"
        }
    }

    var next: GaryxTaskStatus {
        switch self {
        case .todo:
            .inProgress
        case .inProgress:
            .inReview
        case .inReview:
            .done
        case .done:
            .todo
        }
    }

    var tone: GaryxStatusPill.Tone {
        switch self {
        case .todo:
            .muted
        case .inProgress:
            .warning
        case .inReview:
            .danger
        case .done:
            .good
        }
    }
}

private extension GaryxTaskSummary {
    var displayId: String {
        if !id.isEmpty {
            id
        } else if number > 0 {
            "#TASK-\(number)"
        } else {
            "Task"
        }
    }

    var assigneeDisplayLabel: String {
        if let assignee {
            return assignee.garyxDisplayLabel
        }
        if !assigneeLabel.isEmpty {
            return assigneeLabel
        }
        return "Unassigned"
    }

    var formattedUpdatedAt: String {
        garyxFormattedTaskTimestamp(updatedAt)
    }
}

private extension GaryxTaskPrincipal {
    var garyxDisplayLabel: String {
        if kind == "human", let userId, !userId.isEmpty {
            return "@\(userId)"
        }
        if kind == "agent", let agentId, !agentId.isEmpty {
            return agentId
        }
        if let agentId, !agentId.isEmpty {
            return agentId
        }
        if let userId, !userId.isEmpty {
            return "@\(userId)"
        }
        return kind.isEmpty ? "Unknown" : kind
    }

}

private func garyxFormattedTaskTimestamp(_ value: String?) -> String {
    guard let value, let date = garyxTaskDate(from: value) else {
        return ""
    }
    let diff = max(0, Date().timeIntervalSince(date))
    let minutes = Int(diff / 60)
    let hours = Int(diff / 3_600)
    let days = Int(diff / 86_400)
    let months = days / 30
    if minutes < 1 { return "now" }
    if minutes < 60 { return "\(minutes)m" }
    if hours < 24 { return "\(hours)h" }
    if days < 30 { return "\(days)d" }
    if months < 12 { return "\(months)mo" }
    return "\(days / 365)y"
}

private func garyxTaskDate(from value: String) -> Date? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return nil }

    let fractional = ISO8601DateFormatter()
    fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    if let date = fractional.date(from: trimmed) {
        return date
    }

    let standard = ISO8601DateFormatter()
    standard.formatOptions = [.withInternetDateTime]
    return standard.date(from: trimmed)
}

private func garyxThreadDate(from value: String) -> Date? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return nil }

    let fractional = ISO8601DateFormatter()
    fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    if let date = fractional.date(from: trimmed) {
        return date
    }

    let standard = ISO8601DateFormatter()
    standard.formatOptions = [.withInternetDateTime]
    return standard.date(from: trimmed)
}

private extension String {
    var lastPathComponent: String {
        (self as NSString).lastPathComponent
    }

    var garyxDisambiguatedWorkspaceName: String {
        let current = (self as NSString).lastPathComponent
        let parent = ((self as NSString).deletingLastPathComponent as NSString).lastPathComponent
        guard !parent.isEmpty, parent != "/" else {
            return current.isEmpty ? self : current
        }
        return "\(parent)/\(current)"
    }
}
