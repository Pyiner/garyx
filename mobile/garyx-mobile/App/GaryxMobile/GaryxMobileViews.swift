import Foundation
import ImageIO
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers
import WebKit

enum GaryxMobileMotion {
    static let sidebar = Animation.interactiveSpring(response: 0.28, dampingFraction: 0.92, blendDuration: 0.08)
    static let sidebarDrilldown = Animation.easeOut(duration: 0.16)
    static let rowSwipe = Animation.interactiveSpring(response: 0.22, dampingFraction: 0.92, blendDuration: 0.04)
}

private func garyxDismissKeyboard() {
    UIApplication.shared.sendAction(
        #selector(UIResponder.resignFirstResponder),
        to: nil,
        from: nil,
        for: nil
    )
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
    @State private var sidebarDragAxis: GaryxSidebarDragAxis?

    private let sidebarWidth: CGFloat = 330
    private let sidebarEdgeGestureWidth: CGFloat = 24
    private let sidebarAxisDecisionDistance: CGFloat = 14
    private let sidebarAxisDecisionRatio: CGFloat = 1.5

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
        let closeStripX = max(0, min(revealWidth, max(0, containerSize.width - 28)))

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
                    .offset(x: closeStripX)
                    .contentShape(Rectangle())
                    .simultaneousGesture(closingSidebarGesture(sidebarWidth: width))
                    .zIndex(3)
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
        DragGesture(minimumDistance: 18, coordinateSpace: .global)
            .onChanged { value in
                guard !model.sidebarVisible else { return }
                if sidebarDragAxis == nil {
                    sidebarDragAxis = decideSidebarAxis(
                        translation: value.translation,
                        startLocation: value.startLocation,
                        opening: true
                    )
                }
                guard sidebarDragAxis == .horizontal else { return }
                switch model.mainPanelLeadingEdgeAction {
                case .openSidebar:
                    sidebarDragOffset = max(0, min(sidebarWidth, value.translation.width))
                case .settingsOverview:
                    sidebarDragOffset = 0
                }
            }
            .onEnded { value in
                defer {
                    sidebarDragAxis = nil
                }
                guard !model.sidebarVisible, sidebarDragAxis == .horizontal else {
                    resetSidebarDrag()
                    return
                }
                let shouldOpen = value.translation.width > sidebarWidth * 0.22
                    || value.predictedEndTranslation.width > sidebarWidth * 0.35
                switch model.mainPanelLeadingEdgeAction {
                case .openSidebar:
                    finishGesture(open: shouldOpen)
                case .settingsOverview:
                    resetSidebarDrag()
                    if shouldOpen {
                        hideKeyboard()
                        withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                            model.performMainPanelLeadingEdgeAction()
                        }
                    }
                }
            }
    }

    private func closingSidebarGesture(sidebarWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .global)
            .onChanged { value in
                guard model.sidebarVisible else { return }
                if sidebarDragAxis == nil {
                    sidebarDragAxis = decideSidebarAxis(
                        translation: value.translation,
                        startLocation: value.startLocation,
                        opening: false
                    )
                }
                guard sidebarDragAxis == .horizontal else { return }
                sidebarDragOffset = min(0, max(-sidebarWidth, value.translation.width))
            }
            .onEnded { value in
                defer {
                    sidebarDragAxis = nil
                }
                guard model.sidebarVisible, sidebarDragAxis == .horizontal else {
                    resetSidebarDrag()
                    return
                }
                let shouldClose = -value.translation.width > sidebarWidth * 0.22
                    || -value.predictedEndTranslation.width > sidebarWidth * 0.35
                finishGesture(open: !shouldClose)
            }
    }

    private func decideSidebarAxis(
        translation: CGSize,
        startLocation: CGPoint,
        opening: Bool
    ) -> GaryxSidebarDragAxis? {
        let horizontal = translation.width
        let vertical = translation.height
        let horizontalMag = abs(horizontal)
        let verticalMag = abs(vertical)
        let dominant = max(horizontalMag, verticalMag)
        guard dominant >= sidebarAxisDecisionDistance else { return nil }
        guard horizontalMag > verticalMag * sidebarAxisDecisionRatio else {
            return .vertical
        }
        if opening {
            guard horizontal > 0,
                  startLocation.x <= sidebarEdgeGestureWidth else {
                return .vertical
            }
        } else {
            guard horizontal < 0 else { return .vertical }
        }
        return .horizontal
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

private enum GaryxSidebarDragAxis {
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
            .task {
                await refreshSidebarThreads(silent: true)
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
        guard !model.isLoadingThreads else { return }
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
        let channelName = garyxBotChannelDisplayName(channel)
        let account = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        let botId = account.isEmpty ? channelName : "\(channelName) · \(account)"
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
                    subtitle: endpoint.conversationLabel ?? endpoint.threadLabel ?? endpoint.workspaceDir?.lastPathComponent,
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
            thread: thread,
            isSelected: model.selectedThread?.id == thread.id,
            isPinned: showsPinnedMarker || model.isThreadPinned(thread.id),
            showsWorkspaceMeta: showsWorkspaceMeta,
            trailingTimestamp: trailingTimestamp,
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

struct GaryxSidebarThreadRowView: View {
    let thread: GaryxThreadSummary
    let isSelected: Bool
    var isPinned = false
    var showsWorkspaceMeta = true
    var trailingTimestamp: String?
    var isFullBleed = false
    var onSelect: (() -> Void)?
    var onUnpin: (() -> Void)?

    var body: some View {
        HStack(alignment: .center, spacing: 8) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Text(thread.title.isEmpty ? "Untitled" : thread.title)
                        .font(GaryxFont.subheadline(weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(.primary)
                        .layoutPriority(1)

                    if isPinned {
                        Button {
                            onUnpin?()
                        } label: {
                            Image(systemName: "pin.fill")
                                .font(GaryxFont.system(size: 10.5, weight: .semibold))
                                .foregroundStyle(.tertiary)
                                .rotationEffect(.degrees(-28))
                                .frame(width: 18, height: 18)
                                .contentShape(Circle())
                        }
                        .buttonStyle(.plain)
                        .disabled(onUnpin == nil)
                        .accessibilityLabel("Unpin thread")
                    }
                }

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
        .contentShape(Rectangle())
        .onTapGesture {
            onSelect?()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(minHeight: GaryxSidebarMetrics.threadRowMinHeight, alignment: .leading)
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
    var subtitle: String? {
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

    var trailingMeta: some View {
        HStack(spacing: 6) {
            if showsWorkspaceMeta, let workspacePath = thread.workspacePath, !workspacePath.isEmpty {
                Text(workspacePath.lastPathComponent)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .frame(maxWidth: 72, alignment: .trailing)
            }

            if isRunning {
                GaryxSidebarRunningIndicator()
            } else if isSelected {
                Circle()
                    .fill(.secondary)
                    .frame(width: 7, height: 7)
            } else if let trailingTimestamp, !trailingTimestamp.isEmpty {
                Text(trailingTimestamp)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
        }
    }

    var isRunning: Bool {
        let state = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let activeRunId = thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if let state, ["running", "active", "queued", "pending", "working", "in_progress"].contains(state) {
            return true
        }
        return !activeRunId.isEmpty
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

struct GaryxConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @FocusState private var isComposerFocused: Bool
    @State private var scrollPreservationThreadId: String?

    var body: some View {
        ScrollViewReader { proxy in
            messageScroll
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    GaryxComposer(isFocused: $isComposerFocused)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onChange(of: model.messages) { oldValue, newValue in
                    if shouldPreserveScrollForPrependedHistory(oldValue: oldValue, newValue: newValue) {
                        return
                    }
                    guard !newValue.isEmpty || model.showsTailThinkingIndicator else { return }
                    withAnimation(.easeOut(duration: 0.2)) {
                        scrollToConversationTail(proxy)
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
            }
            .padding(.horizontal, 16)
            .padding(.top, 18)
            .padding(.bottom, 24)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
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
                    Button("Refresh", systemImage: "arrow.clockwise") {
                        Task { await model.loadSelectedThreadHistory() }
                    }
                    Button("New Thread", systemImage: "square.and.pencil") {
                        model.openNewThreadDraft()
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

private enum GaryxDataURLImageCache {
    private static let cache: NSCache<NSString, UIImage> = {
        let cache = NSCache<NSString, UIImage>()
        cache.countLimit = 128
        cache.totalCostLimit = 32 * 1024 * 1024
        return cache
    }()

    static func image(from rawValue: String?) -> UIImage? {
        let raw = (rawValue ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return nil }
        let cacheKey = NSString(string: raw)
        if let cached = cache.object(forKey: cacheKey) {
            return cached
        }
        let encoded = raw.split(separator: ",", maxSplits: 1).last.map(String.init) ?? raw
        guard let data = Data(base64Encoded: encoded),
              let image = UIImage(data: data) else {
            return nil
        }
        cache.setObject(image, forKey: cacheKey, cost: data.count)
        return image
    }
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

struct GaryxWorkspacesView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var isPickingFiles = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Workspaces",
            subtitle: subtitle,
            onRefresh: { await model.refreshSelectedWorkspace() }
        ) {
            GaryxWorkspacesContent()
        } actions: {
            Button {
                isPickingFiles = true
            } label: {
                GaryxToolbarIcon(systemName: model.isUploadingWorkspaceFiles ? "hourglass" : "square.and.arrow.up")
            }
            .buttonStyle(.plain)
            .disabled(model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.isUploadingWorkspaceFiles)
            .accessibilityLabel("Upload Files")
        }
        .task {
            await model.prepareWorkspaceBrowser()
        }
        .onChange(of: model.knownWorkspacePaths) { _, _ in
            Task { await model.prepareWorkspaceBrowser() }
        }
        .fileImporter(
            isPresented: $isPickingFiles,
            allowedContentTypes: [.item],
            allowsMultipleSelection: true
        ) { result in
            switch result {
            case .success(let urls):
                Task { await model.uploadFilesToSelectedWorkspace(from: urls) }
            case .failure(let error):
                model.lastError = error.localizedDescription
            }
        }
    }

    private var subtitle: String {
        let workspace = model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty else { return "\(model.knownWorkspacePaths.count) workspaces" }
        let name = workspace.lastPathComponent.isEmpty ? workspace : workspace.lastPathComponent
        let directory = model.selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        return directory.isEmpty ? name : "\(name) / \(directory)"
    }
}

struct GaryxWorkspacesContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        let paths = model.knownWorkspacePaths
        VStack(alignment: .leading, spacing: 12) {
            if paths.isEmpty {
                GaryxEmptyPanelView(
                    icon: "folder",
                    title: "No workspaces",
                    text: ""
                )
            } else {
                GaryxSectionBlock(title: "Workspace") {
                    GaryxCompactListGroup {
                        ForEach(Array(paths.enumerated()), id: \.element) { index, path in
                            GaryxWorkspacePathRow(
                                path: path,
                                isSelected: model.selectedWorkspacePath == path
                            )
                            if index < paths.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }

                GaryxWorkspaceFilesSection()

                if let status = model.workspaceUploadStatus, !status.isEmpty {
                    Text(status)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 2)
                }

                if let preview = model.workspacePreview {
                    GaryxWorkspacePreviewSection(preview: preview)
                }
            }
        }
    }
}

struct GaryxWorkspacePathRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let path: String
    let isSelected: Bool

    var body: some View {
        Button {
            Task { await model.selectWorkspace(path) }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: isSelected ? "folder.fill" : "folder")
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(isSelected ? .primary : .secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(path.lastPathComponent.isEmpty ? path : path.lastPathComponent)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(garyxCompactPathLabel(path))
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 0)

                if isSelected {
                    Image(systemName: "checkmark")
                        .font(GaryxFont.system(size: 12, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(path.lastPathComponent.isEmpty ? path : path.lastPathComponent)
        .accessibilityValue(garyxCompactPathLabel(path))
    }
}

struct GaryxWorkspaceFilesSection: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxSectionBlock(title: "Files") {
            if let listing = model.workspaceListing {
                GaryxCompactListGroup {
                    if !model.selectedWorkspaceDirectory.isEmpty {
                        GaryxWorkspaceUpRow()
                        if !listing.entries.isEmpty {
                            GaryxCompactRowDivider()
                        }
                    }
                    ForEach(Array(listing.entries.enumerated()), id: \.element.id) { index, entry in
                        GaryxWorkspaceFileRow(entry: entry)
                        if index < listing.entries.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                    if listing.entries.isEmpty, model.selectedWorkspaceDirectory.isEmpty {
                        GaryxWorkspaceEmptyDirectoryRow()
                    }
                }
            } else {
                GaryxEmptyPanelView(
                    icon: "folder.badge.questionmark",
                    title: "Select a workspace",
                    text: ""
                )
            }
        }
    }
}

struct GaryxWorkspaceUpRow: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        Button {
            Task { await model.goUpWorkspaceDirectory() }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "arrow.turn.up.left")
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)
                Text("Parent Folder")
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct GaryxWorkspaceEmptyDirectoryRow: View {
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "tray")
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 28, height: 28)
            Text("Empty folder")
                .font(GaryxFont.subheadline(weight: .medium))
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 9)
        .frame(minHeight: 52)
    }
}

struct GaryxWorkspaceFileRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let entry: GaryxWorkspaceFileEntry

    var body: some View {
        Button {
            Task { await model.openWorkspaceEntry(entry) }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: iconName)
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(entry.entryType == "directory" ? .primary : .secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(entry.name.isEmpty ? entry.path.lastPathComponent : entry.name)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(detail)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                Image(systemName: entry.entryType == "directory" ? "chevron.right" : "doc.text.magnifyingglass")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(entry.name.isEmpty ? entry.path.lastPathComponent : entry.name)
    }

    private var iconName: String {
        if entry.entryType == "directory" { return "folder" }
        let mediaType = entry.mediaType?.lowercased() ?? ""
        if mediaType.starts(with: "image/") { return "photo" }
        if mediaType == "application/pdf" { return "doc.richtext" }
        if mediaType.starts(with: "text/") { return "doc.text" }
        return "doc"
    }

    private var detail: String {
        if entry.entryType == "directory" {
            return entry.hasChildren ? "Folder" : "Empty folder"
        }
        var parts: [String] = []
        if let size = entry.size {
            parts.append(garyxFormattedFileSize(size))
        }
        if let modified = entry.modifiedAt, !modified.isEmpty {
            parts.append(garyxFormattedTaskTimestamp(modified))
        }
        return parts.isEmpty ? "File" : parts.joined(separator: " · ")
    }
}

struct GaryxWorkspacePreviewSection: View {
    let preview: GaryxWorkspaceFilePreview

    var body: some View {
        GaryxSectionBlock(title: "Preview") {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: previewIconName)
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 28)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(preview.name)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(preview.path)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    Spacer(minLength: 0)
                    GaryxStatusPill(text: preview.previewKind.capitalized, tone: .muted)
                }

                if let text = preview.text, !text.isEmpty {
                    ScrollView([.vertical, .horizontal], showsIndicators: true) {
                        Text(text)
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(.primary)
                            .textSelection(.enabled)
                            .padding(10)
                    }
                    .frame(maxHeight: 240, alignment: .topLeading)
                    .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                } else if let image {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .frame(maxWidth: .infinity, maxHeight: 260)
                        .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                } else {
                    Text(preview.previewKind == "pdf" ? "PDF preview available on desktop." : "No inline preview available.")
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                        .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }

                HStack(spacing: 8) {
                    Text(garyxFormattedFileSize(preview.size))
                    if preview.truncated {
                        Text("Truncated")
                    }
                }
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
            }
            .padding(12)
            .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
        }
    }

    private var image: UIImage? {
        guard preview.previewKind == "image",
              let dataBase64 = preview.dataBase64,
              let data = Data(base64Encoded: dataBase64) else {
            return nil
        }
        return UIImage(data: data)
    }

    private var previewIconName: String {
        switch preview.previewKind {
        case "image":
            "photo"
        case "pdf":
            "doc.richtext"
        case "markdown", "html", "text":
            "doc.text"
        default:
            "doc"
        }
    }
}

private func garyxFormattedFileSize(_ size: Int) -> String {
    ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file)
}

private func garyxCompactPathLabel(_ path: String) -> String {
    let normalized = path
        .trimmingCharacters(in: .whitespacesAndNewlines)
        .replacingOccurrences(of: "\\", with: "/")
    guard !normalized.isEmpty else { return "" }
    let parts = normalized
        .split(separator: "/", omittingEmptySubsequences: true)
        .map(String.init)
    if parts.count >= 2,
       parts[0] == "Users" {
        let remainder = Array(parts.dropFirst(2))
        guard !remainder.isEmpty else { return "Home folder" }
        return "~/" + remainder.prefix(2).joined(separator: "/")
    }
    if parts.count > 2 {
        return parts.suffix(2).joined(separator: "/")
    }
    return normalized
}

struct GaryxTasksView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateTask = false

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
        } actions: {
            GaryxAddToolbarButton(label: "New Task") {
                showsCreateTask = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateTask) {
            GaryxFormSheet(title: "New Task") {
                GaryxCreateTaskCard()
            }
        }
    }
}

struct GaryxDreamsView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxPanelScaffold(
            title: "Dreams",
            subtitle: subtitle,
            onRefresh: { await model.refreshDreams() }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                GaryxSectionBlock(title: "Settings") {
                    GaryxCompactListGroup {
                        GaryxDreamsAutoScanRow()
                    }
                }

                if model.dreams.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "moon.stars",
                        title: "No dreams yet.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Last 24 Hours") {
                        GaryxCompactListGroup {
                            GaryxDreamTopicList(dreams: model.dreams)
                        }
                    }
                }
            }
        } actions: {
            Button {
                Task { await model.scanDreams() }
            } label: {
                GaryxToolbarIcon(systemName: model.isScanningDreams ? "hourglass" : "sparkles")
            }
            .buttonStyle(.plain)
            .disabled(model.isScanningDreams)
            .accessibilityLabel("Scan dreams")
        }
    }

    private var subtitle: String {
        if let scan = model.latestDreamScan {
            let status = scan.status.trimmingCharacters(in: .whitespacesAndNewlines)
            let updated = garyxFormattedTaskTimestamp(scan.createdAt)
            let statusText = status.isEmpty ? "scan" : status
            return updated.isEmpty
                ? "\(model.dreams.count) topics / \(statusText)"
                : "\(model.dreams.count) topics / \(statusText) \(updated)"
        }
        return "\(model.dreams.count) topics"
    }
}

struct GaryxDreamsAutoScanRow: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "clock.arrow.2.circlepath")
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 24, height: 24)

            VStack(alignment: .leading, spacing: 3) {
                Text("Dreams")
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                Text("Shows Dreams in the app and runs periodic scans when recent user messages exist.")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }

            Spacer(minLength: 0)

            Toggle(
                "Dreams",
                isOn: Binding(
                    get: { model.dreamsAutoScanEnabled },
                    set: { nextValue in
                        Task { await model.setDreamsAutoScanEnabled(nextValue) }
                    }
                )
            )
            .labelsHidden()
            .toggleStyle(.switch)
            .disabled(model.isSavingDreamsSettings)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }
}

struct GaryxDreamTopicList: View {
    let dreams: [GaryxDreamTopic]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(dreams.enumerated()), id: \.element.id) { index, dream in
                GaryxDreamTopicRow(dream: dream)
                if index < dreams.count - 1 {
                    GaryxCompactRowDivider()
                }
            }
        }
    }
}

struct GaryxDreamTopicRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let dream: GaryxDreamTopic

    var body: some View {
        Button {
            if let firstSpan = dream.spans.first {
                Task { await model.openDreamSpan(firstSpan) }
            }
        } label: {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(dream.title)
                        .font(GaryxFont.body(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)

                    Spacer(minLength: 8)

                    Text("\(dream.messageCount)")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .frame(height: 24)
                        .background(Color(.tertiarySystemFill), in: Capsule())
                }

                if !dream.summary.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(dream.summary)
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                }

                VStack(alignment: .leading, spacing: 6) {
                    ForEach(dream.spans.prefix(3)) { span in
                        GaryxDreamSpanRow(span: span)
                    }
                }

                HStack(spacing: 8) {
                    Text(dream.sourceDisplayLabel)
                    Spacer(minLength: 8)
                    Text(dream.formattedLastMessageAt)
                }
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(dream.spans.isEmpty)
    }
}

struct GaryxDreamSpanRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let span: GaryxDreamSpan

    var body: some View {
        Button {
            Task { await model.openDreamSpan(span) }
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "arrow.turn.down.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .frame(width: 14)

                VStack(alignment: .leading, spacing: 2) {
                    Text(span.excerpt.isEmpty ? span.threadId : span.excerpt)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                    Text(span.threadDisplayLabel)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 6)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
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

struct GaryxCreateTaskCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var workspacePath = ""
    @State private var startImmediately = true
    @State private var notificationTargetId = "none"

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Task")
            TextField("Title", text: $model.draftTaskTitle)
                .garyxInputStyle()
            TextField("Details", text: $model.draftTaskBody, axis: .vertical)
                .lineLimit(3...8)
                .garyxInputStyle()

            GaryxFieldLabel("Assignee")
                .padding(.top, 4)
            Menu {
                ForEach(model.agentTargets) { target in
                    Button {
                        model.setSelectedAgentTarget(target.id)
                    } label: {
                        Label(target.title, systemImage: target.kind == .team ? "person.3" : "person")
                    }
                }
            } label: {
                GaryxAgentPickerLabel(
                    target: model.selectedAgentTarget,
                    title: model.selectedAgentLabel,
                    showsChevron: true,
                    style: .compact
                )
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(GaryxTheme.input, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
            .buttonStyle(.plain)
            .disabled(model.agentTargets.isEmpty)

            GaryxFieldLabel("Workspace")
                .padding(.top, 4)
            TextField("Workspace directory", text: $workspacePath)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()

            GaryxFieldLabel("Notification")
                .padding(.top, 4)
            Menu {
                Button {
                    notificationTargetId = "none"
                } label: {
                    Label("Do not notify", systemImage: notificationTargetId == "none" ? "checkmark" : "bell.slash")
                }
                if !model.mobileBotGroups.isEmpty {
                    Divider()
                    ForEach(model.mobileBotGroups) { group in
                        Button {
                            notificationTargetId = group.id
                        } label: {
                            Label(group.title, systemImage: notificationTargetId == group.id ? "checkmark" : "bell")
                        }
                    }
                }
            } label: {
                HStack(spacing: 8) {
                    Text(notificationTargetLabel)
                        .font(GaryxFont.callout(weight: .medium))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.down")
                        .font(GaryxFont.system(size: 10, weight: .bold))
                        .foregroundStyle(.tertiary)
                }
                .padding(.horizontal, 12)
                .frame(height: 42)
                .background(GaryxTheme.input, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
            .buttonStyle(.plain)

            Toggle("Start immediately", isOn: $startImmediately)
                .font(GaryxFont.callout(weight: .medium))

            Button {
                Task {
                    model.setNewThreadWorkspace(workspacePath)
                    await model.createTaskFromDraft(
                        start: startImmediately,
                        notificationTarget: notificationTargetRequest
                    )
                    if model.draftTaskTitle.isEmpty, model.draftTaskBody.isEmpty {
                        dismiss()
                    }
                }
            } label: {
                Label("Create Task", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
            .disabled(!canCreate)
        }
        .garyxCardStyle()
        .onAppear {
            workspacePath = model.newThreadWorkspace
        }
    }

    private var canCreate: Bool {
        !model.draftTaskTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !model.draftTaskBody.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var selectedNotificationGroup: GaryxMobileBotGroup? {
        model.mobileBotGroups.first { $0.id == notificationTargetId }
    }

    private var notificationTargetLabel: String {
        selectedNotificationGroup?.title ?? "Do not notify"
    }

    private var notificationTargetRequest: GaryxTaskNotificationTargetRequest {
        guard let group = selectedNotificationGroup else { return .none }
        return .bot(channel: group.channel, accountId: group.accountId)
    }
}

struct GaryxTaskListRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let task: GaryxTaskSummary
    @State private var showsAssignSheet = false
    @State private var showsDeleteConfirmation = false
    @State private var showsMoreActions = false
    @State private var showsRenamePrompt = false
    @State private var showsStatusActions = false
    @State private var showsTaskDetails = false
    @State private var renameDraftTitle = ""

    var body: some View {
        GaryxSwipeActionRow(actions: taskSwipeActions) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top, spacing: 8) {
                    Button {
                        if task.threadId.isEmpty {
                            showsTaskDetails = true
                        } else {
                            Task { await model.openThread(id: task.threadId) }
                        }
                    } label: {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(task.title)
                                .font(GaryxFont.subheadline(weight: .semibold))
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

                    GaryxStatusPill(text: task.status.label, tone: task.status.tone)
                        .fixedSize(horizontal: true, vertical: false)
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
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .fullScreenCover(isPresented: $showsAssignSheet) {
            GaryxFormSheet(title: "Assign Task") {
                GaryxTaskAssignCard(task: task)
            }
        }
        .fullScreenCover(isPresented: $showsTaskDetails) {
            GaryxFormSheet(title: "Task Details") {
                GaryxTaskDetailCard(task: task)
            }
        }
        .alert("Rename Task", isPresented: $showsRenamePrompt) {
            TextField("Task title", text: $renameDraftTitle)
            Button("Cancel", role: .cancel) {}
            Button("Save") {
                Task { await model.updateTaskTitle(task, title: renameDraftTitle) }
            }
        }
        .confirmationDialog("Task Actions", isPresented: $showsMoreActions, titleVisibility: .visible) {
            Button("Rename") {
                openRenamePrompt()
            }
            if !model.agentTargets.isEmpty {
                Button("Assign") {
                    showsAssignSheet = true
                }
            }
            Button("Details") {
                showsTaskDetails = true
            }
            if task.assignee != nil || !task.assigneeLabel.isEmpty {
                Button("Unassign") {
                    Task { await model.unassignTask(task) }
                }
            }
            Button("Delete", role: .destructive) {
                showsDeleteConfirmation = true
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog("Set Status", isPresented: $showsStatusActions, titleVisibility: .visible) {
            ForEach(task.status.allowedTransitions, id: \.rawValue) { status in
                Button {
                    Task { await model.updateTask(task, to: status) }
                } label: {
                    Label(status.label, systemImage: status.systemImage)
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog("Delete task?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteTask(task) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the task from the task list.")
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
        if task.threadId.isEmpty {
            actions.append(
                GaryxSwipeAction(title: "Details", systemImage: "info.circle", tone: .accent) {
                    showsTaskDetails = true
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
            GaryxSwipeAction(title: "Status", systemImage: "arrow.left.arrow.right.circle") {
                showsStatusActions = true
            }
        )
        actions.append(
            GaryxSwipeAction(title: "More", systemImage: "ellipsis.circle") {
                showsMoreActions = true
            }
        )
        return actions
    }

    private func openRenamePrompt() {
        renameDraftTitle = task.title
        showsRenamePrompt = true
    }
}

struct GaryxTaskDetailCard: View {
    let task: GaryxTaskSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(task.title)
                        .font(GaryxFont.title3(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(3)
                    Spacer(minLength: 0)
                    GaryxStatusPill(text: task.status.label, tone: task.status.tone)
                }
                Text(task.displayId)
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
            }

            GaryxCompactListGroup {
                GaryxTaskMetaLine(label: "Assignee", value: task.assigneeDisplayLabel)
                GaryxCompactRowDivider()
                GaryxTaskMetaLine(label: "Runtime", value: task.runtimeAgentId.isEmpty ? "Not assigned" : task.runtimeAgentId)
                GaryxCompactRowDivider()
                GaryxTaskMetaLine(label: "Thread", value: task.threadId.isEmpty ? "No thread" : task.threadId)
                GaryxCompactRowDivider()
                GaryxTaskMetaLine(label: "Replies", value: "\(task.replyCount)")
                GaryxCompactRowDivider()
                GaryxTaskMetaLine(label: "Updated", value: task.formattedUpdatedAt)
                if let creator = task.creator {
                    GaryxCompactRowDivider()
                    GaryxTaskMetaLine(label: "Creator", value: creator.label)
                }
                if let updatedBy = task.updatedBy {
                    GaryxCompactRowDivider()
                    GaryxTaskMetaLine(label: "Updated by", value: updatedBy.label)
                }
                if let source = task.source {
                    GaryxCompactRowDivider()
                    GaryxTaskMetaLine(label: "Source", value: source.detailLabel)
                }
            }

            if task.threadId.isEmpty {
                GaryxNotice(
                    title: "No chat thread yet",
                    text: "Assign or start this task to create a runnable thread."
                )
            }
        }
        .garyxCardStyle()
    }
}

struct GaryxTaskAssignCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let task: GaryxTaskSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Assign To")
            if model.agentTargets.isEmpty {
                GaryxEmptyPanelView(
                    icon: "person.crop.circle.badge.exclamationmark",
                    title: "No agents available.",
                    text: ""
                )
            } else {
                GaryxCompactListGroup {
                    ForEach(Array(model.agentTargets.enumerated()), id: \.element.id) { index, target in
                        Button {
                            Task {
                                await model.assignTask(task, agentId: target.id)
                                dismiss()
                            }
                        } label: {
                            GaryxAgentIdentityRow(
                                id: target.id,
                                title: target.title,
                                subtitle: target.subtitle,
                                kind: target.kind,
                                avatarDataUrl: target.avatarDataUrl,
                                providerType: target.providerType,
                                builtIn: target.builtIn,
                                selected: task.assignee?.agentId == target.id
                                    || task.assigneeLabel == target.id
                                    || task.runtimeAgentId == target.id
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
        .garyxCardStyle()
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
                GaryxSectionBlock(title: "Browse") {
                    GaryxCompactListGroup {
                        GaryxSettingsPanelLinkRow(panel: .workspaceBots)
                    }
                }

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
    @State private var showsDeleteConfirmation = false
    @State private var label = ""
    @State private var prompt = ""
    @State private var intervalHours = ""
    @State private var targetsExistingThread = false
    @State private var targetThreadId = ""
    @State private var workspacePath = ""

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
                        Text(automationTargetLabel)
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

                    GaryxFieldLabel("Run In")
                    Picker("Run In", selection: $targetsExistingThread) {
                        Text("New Thread").tag(false)
                        Text("Existing Thread").tag(true)
                    }
                    .pickerStyle(.segmented)

                    if targetsExistingThread {
                        if model.threads.isEmpty && effectiveEditThreadId.isEmpty {
                            Text("No existing threads loaded")
                                .font(GaryxFont.caption(weight: .semibold))
                                .foregroundStyle(.secondary)
                        } else {
                            Picker("Thread", selection: editThreadSelection) {
                                if !targetThreadId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                                   !model.threads.contains(where: { $0.id == targetThreadId }) {
                                    Text(targetThreadId).tag(targetThreadId)
                                }
                                ForEach(model.threads, id: \.id) { thread in
                                    Text(thread.title).tag(thread.id)
                                }
                            }
                            .pickerStyle(.menu)
                            .garyxInputStyle()
                        }
                        Text("Each run posts the prompt into the selected thread.")
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                    } else if editWorkspaceOptions.isEmpty {
                        Text("No workspaces available")
                            .font(GaryxFont.caption(weight: .semibold))
                            .foregroundStyle(.secondary)
                    } else {
                        Picker("Workspace", selection: editWorkspaceSelection) {
                            ForEach(editWorkspaceOptions, id: \.self) { path in
                                Text(path.lastPathComponent).tag(path)
                            }
                        }
                        .pickerStyle(.menu)
                        .garyxInputStyle()
                        Text("Each run creates a fresh automation thread in the selected workspace.")
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                    }

                    if automation.schedule.kind == .interval {
                        TextField("Every", text: $intervalHours)
                            .keyboardType(.numberPad)
                            .garyxInputStyle()
                    } else {
                        GaryxFieldLabel("Schedule")
                        Text(garyxAutomationScheduleSummary(automation.schedule))
                            .font(GaryxFont.callout(weight: .medium))
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 12)
                            .frame(minHeight: 42, alignment: .leading)
                            .background(GaryxTheme.input, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                    Button {
                        Task {
                            await model.updateAutomation(
                                automation,
                                label: label,
                                prompt: prompt,
                                intervalHours: intervalHours,
                                targetsExistingThread: targetsExistingThread,
                                targetThreadId: effectiveEditThreadId,
                                workspacePath: effectiveEditWorkspacePath
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                    .disabled(!canSave)
                }
                .garyxCardStyle()
                .onChange(of: targetsExistingThread) { _, _ in
                    ensureEditTargetSelection()
                }
            }
        }
        .confirmationDialog("Delete automation?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteAutomation(automation) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the scheduled automation and its saved configuration.")
        }
    }

    private var automationSwipeActions: [GaryxSwipeAction] {
        var actions: [GaryxSwipeAction] = []
        if automation.enabled {
            actions.append(
                GaryxSwipeAction(title: "Run", systemImage: "play.fill", tone: .accent) {
                    Task { await model.runAutomation(automation) }
                }
            )
        }
        actions.append(
            GaryxSwipeAction(title: automation.enabled ? "Pause" : "Resume", systemImage: automation.enabled ? "pause.fill" : "play.fill") {
                Task { await model.toggleAutomation(automation) }
            }
        )
        if let threadId = automationOpenThreadId {
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
                showsDeleteConfirmation = true
            }
        )
        return actions
    }

    private var automationOpenThreadId: String? {
        let target = automation.targetThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !target.isEmpty {
            return target
        }
        let latest = automation.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return latest.isEmpty ? nil : latest
    }

    private var automationTargetLabel: String {
        if let targetThreadId = automationOpenThreadId,
           let thread = model.threads.first(where: { $0.id == targetThreadId }) {
            return "Thread · \(thread.title)"
        }
        if let targetThreadId = automationOpenThreadId,
           automation.targetThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) == targetThreadId {
            return "Thread · \(targetThreadId)"
        }
        return automation.workspacePath.isEmpty ? automation.agentId : automation.workspacePath.lastPathComponent
    }

    private func fillDraft() {
        label = automation.label
        prompt = automation.prompt
        intervalHours = String(automation.schedule.hours ?? 24)
        let target = automation.targetThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        targetsExistingThread = !target.isEmpty
        targetThreadId = target
        let targetWorkspace = target.isEmpty
            ? ""
            : model.threads.first(where: { $0.id == target })?.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let automationWorkspace = automation.workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        workspacePath = automationWorkspace.isEmpty ? targetWorkspace : automationWorkspace
        ensureEditTargetSelection()
    }

    private var editWorkspaceOptions: [String] {
        var seen = Set<String>()
        return ([workspacePath] + model.knownWorkspacePaths)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .filter { seen.insert($0).inserted }
    }

    private var editWorkspaceSelection: Binding<String> {
        Binding {
            effectiveEditWorkspacePath
        } set: { value in
            workspacePath = value
        }
    }

    private var editThreadSelection: Binding<String> {
        Binding {
            effectiveEditThreadId
        } set: { value in
            targetThreadId = value
            if let thread = model.threads.first(where: { $0.id == value }),
               let nextWorkspace = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !nextWorkspace.isEmpty {
                workspacePath = nextWorkspace
            }
        }
    }

    private var effectiveEditWorkspacePath: String {
        let selected = workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, editWorkspaceOptions.contains(selected) {
            return selected
        }
        return editWorkspaceOptions.first ?? ""
    }

    private var effectiveEditThreadId: String {
        let selected = targetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty {
            return selected
        }
        return model.threads.first?.id ?? ""
    }

    private var canSave: Bool {
        !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (targetsExistingThread ? !effectiveEditThreadId.isEmpty : !effectiveEditWorkspacePath.isEmpty)
            && (automation.schedule.kind != .interval || positiveInteger(intervalHours) != nil)
    }

    private func ensureEditTargetSelection() {
        if targetsExistingThread {
            let nextThreadId = effectiveEditThreadId
            if targetThreadId != nextThreadId {
                targetThreadId = nextThreadId
            }
            if let thread = model.threads.first(where: { $0.id == nextThreadId }),
               let nextWorkspace = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !nextWorkspace.isEmpty {
                workspacePath = nextWorkspace
            }
        } else {
            let nextWorkspace = effectiveEditWorkspacePath
            if workspacePath != nextWorkspace {
                workspacePath = nextWorkspace
            }
        }
    }

    private func positiveInteger(_ value: String) -> Int? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = Int(trimmed), parsed > 0 else { return nil }
        return parsed
    }
}

private func garyxAutomationScheduleSummary(_ schedule: GaryxAutomationSchedule) -> String {
    func nonEmpty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }

    switch schedule.kind {
    case .interval:
        return "Every \(max(1, schedule.hours ?? 24)) hours"
    case .daily:
        let time = nonEmpty(schedule.time) ?? "09:00"
        let timezone = nonEmpty(schedule.timezone) ?? "UTC"
        if schedule.weekdays.isEmpty {
            return "Daily at \(time) \(timezone)"
        }
        return "\(schedule.weekdays.map { $0.uppercased() }.joined(separator: ", ")) at \(time) \(timezone)"
    case .once:
        return "Once at \(nonEmpty(schedule.at) ?? "scheduled time")"
    }
}

struct GaryxWorkspaceBotsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var activeDrilldown: GaryxSidebarDrilldown?

    var body: some View {
        GaryxPanelScaffold(
            title: title,
            subtitle: subtitle,
            onRefresh: { await refresh() },
            leadingActionLabel: activeDrilldown == nil ? "Automation" : "Workspace & Bots",
            leadingAction: { goBack() }
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

    private var subtitle: String {
        switch activeDrilldown {
        case let .bot(id):
            return model.mobileBotGroups.first { $0.id == id }?.compactDetailLine ?? "Bot threads"
        case let .workspace(path):
            return model.sidebarWorkspaceThreadGroups.first { $0.path == path }?.path ?? "Workspace threads"
        case nil:
            return "\(model.mobileBotGroups.count) bots · \(visibleWorkspaceCount) workspaces"
        }
    }

    private func refresh() async {
        await model.refreshRemoteState()
        await model.refreshWorkspaceAndBotThreads()
    }

    private var visibleWorkspaceCount: Int {
        model.knownWorkspacePaths
            .filter(GaryxMobileModel.isVisibleMobileWorkspacePath)
            .count
    }

    private func goBack() {
        if activeDrilldown != nil {
            withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                activeDrilldown = nil
            }
            return
        }
        model.openPanel(.automations)
    }
}

struct GaryxCreateAutomationCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("New Automation")
            Picker("Run In", selection: $model.draftAutomationTargetsExistingThread) {
                Text("New Thread").tag(false)
                Text("Existing Thread").tag(true)
            }
            .pickerStyle(.segmented)

            if model.draftAutomationTargetsExistingThread {
                if threadOptions.isEmpty {
                    Text("No existing threads loaded")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.secondary)
                } else {
                    Picker("Thread", selection: threadSelection) {
                        ForEach(threadOptions, id: \.id) { thread in
                            Text(thread.title).tag(thread.id)
                        }
                    }
                    .pickerStyle(.menu)
                    .garyxInputStyle()
                }
            } else if workspacePaths.isEmpty {
                Text("No workspaces available")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
            } else {
                Picker("Workspace", selection: workspaceSelection) {
                    ForEach(workspacePaths, id: \.self) { path in
                        Text(path.lastPathComponent).tag(path)
                    }
                }
                .pickerStyle(.menu)
                .garyxInputStyle()
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
                .disabled(!canCreate)
            }
        }
        .garyxCardStyle()
        .onAppear(perform: ensureTargetSelection)
        .onChange(of: model.draftAutomationTargetsExistingThread) { _, _ in
            ensureTargetSelection()
        }
    }

    private var workspacePaths: [String] {
        model.knownWorkspacePaths
    }

    private var threadOptions: [GaryxThreadSummary] {
        model.threads
    }

    private var workspaceSelection: Binding<String> {
        Binding {
            effectiveWorkspacePath
        } set: { value in
            model.selectedWorkspacePath = value
        }
    }

    private var threadSelection: Binding<String> {
        Binding {
            effectiveThreadId
        } set: { value in
            model.draftAutomationTargetThreadId = value
            if let thread = model.threads.first(where: { $0.id == value }),
               let workspacePath = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !workspacePath.isEmpty {
                model.selectedWorkspacePath = workspacePath
            }
        }
    }

    private var effectiveWorkspacePath: String {
        let selected = model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, workspacePaths.contains(selected) {
            return selected
        }
        return workspacePaths.first ?? ""
    }

    private var effectiveThreadId: String {
        let selected = model.draftAutomationTargetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, threadOptions.contains(where: { $0.id == selected }) {
            return selected
        }
        return threadOptions.first?.id ?? ""
    }

    private var canCreate: Bool {
        !model.draftAutomationLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !model.draftAutomationPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (model.draftAutomationTargetsExistingThread ? !effectiveThreadId.isEmpty : !effectiveWorkspacePath.isEmpty)
            && positiveInteger(model.draftAutomationIntervalHours) != nil
    }

    private func ensureTargetSelection() {
        if model.draftAutomationTargetsExistingThread {
            let nextThreadId = effectiveThreadId
            if model.draftAutomationTargetThreadId != nextThreadId {
                model.draftAutomationTargetThreadId = nextThreadId
            }
            if let thread = model.threads.first(where: { $0.id == nextThreadId }),
               let workspacePath = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !workspacePath.isEmpty {
                model.selectedWorkspacePath = workspacePath
            }
        } else {
            let nextSelection = effectiveWorkspacePath
            if model.selectedWorkspacePath != nextSelection {
                model.selectedWorkspacePath = nextSelection
            }
        }
    }

    private func positiveInteger(_ value: String) -> Int? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = Int(trimmed), parsed > 0 else { return nil }
        return parsed
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
    @State private var showsDeleteConfirmation = false
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
        .confirmationDialog("Delete agent?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteAgent(agent) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the custom agent configuration.")
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
                    showsDeleteConfirmation = true
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
    @State private var showsDeleteConfirmation = false
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
        .confirmationDialog("Delete team?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteTeam(team) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the team configuration.")
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
                showsDeleteConfirmation = true
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
    @State private var showsDiscardSkillEditorConfirmation = false

    private var skillEditorPresented: Binding<Bool> {
        Binding(
            get: { model.selectedSkillEditor != nil },
            set: { isPresented in
                if !isPresented {
                    requestCloseSkillEditor()
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
            GaryxFormSheet(title: "Skill Editor", onDone: requestCloseSkillEditor) {
                GaryxSkillEditorCard()
            }
            .interactiveDismissDisabled(skillEditorHasUnsavedChanges)
            .confirmationDialog(
                "Discard unsaved skill changes?",
                isPresented: $showsDiscardSkillEditorConfirmation,
                titleVisibility: .visible
            ) {
                Button("Discard", role: .destructive) {
                    closeSkillEditor()
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Your current file edits have not been saved.")
            }
        }
    }

    private var skillEditorHasUnsavedChanges: Bool {
        guard let document = model.selectedSkillDocument, document.editable else { return false }
        return model.selectedSkillFileContent != document.content
    }

    private func requestCloseSkillEditor() {
        if skillEditorHasUnsavedChanges {
            showsDiscardSkillEditorConfirmation = true
        } else {
            closeSkillEditor()
        }
    }

    private func closeSkillEditor() {
        model.selectedSkillEditor = nil
        model.selectedSkillDocument = nil
        model.selectedSkillFileContent = ""
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
    @State private var showsDeleteConfirmation = false
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
        .confirmationDialog("Delete skill?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSkill(skill) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the skill directory.")
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
                showsDeleteConfirmation = true
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
    @State private var showsDiscardFileSwitchConfirmation = false
    @State private var pendingFileSkillId = ""
    @State private var pendingFilePath = ""

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
                    GaryxSkillEntryRow(skillId: editor.skill.id, node: node, depth: 0) { path in
                        requestOpenSkillFile(skillId: editor.skill.id, path: path)
                    }
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
            .confirmationDialog(
                "Discard unsaved skill changes?",
                isPresented: $showsDiscardFileSwitchConfirmation,
                titleVisibility: .visible
            ) {
                Button("Discard", role: .destructive) {
                    openPendingSkillFile()
                }
                Button("Cancel", role: .cancel) {
                    clearPendingSkillFile()
                }
            } message: {
                Text("Your current file edits have not been saved.")
            }
        }
    }

    private var skillEditorHasUnsavedChanges: Bool {
        guard let document = model.selectedSkillDocument, document.editable else { return false }
        return model.selectedSkillFileContent != document.content
    }

    private func requestOpenSkillFile(skillId: String, path: String) {
        if model.selectedSkillDocument?.path == path {
            return
        }
        if skillEditorHasUnsavedChanges {
            pendingFileSkillId = skillId
            pendingFilePath = path
            showsDiscardFileSwitchConfirmation = true
        } else {
            Task { await model.openSkillFile(skillId: skillId, path: path) }
        }
    }

    private func openPendingSkillFile() {
        let skillId = pendingFileSkillId
        let path = pendingFilePath
        clearPendingSkillFile()
        guard !skillId.isEmpty, !path.isEmpty else { return }
        Task { await model.openSkillFile(skillId: skillId, path: path) }
    }

    private func clearPendingSkillFile() {
        pendingFileSkillId = ""
        pendingFilePath = ""
    }
}

struct GaryxSkillEntryRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let skillId: String
    let node: GaryxSkillEntryNode
    let depth: Int
    let onOpenFile: (String) -> Void
    @State private var showsDeleteConfirmation = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: node.entryType == "directory" ? "folder.fill" : "doc.text")
                    .frame(width: 18)
                Button {
                    if node.entryType == "file" {
                        onOpenFile(node.path)
                    }
                } label: {
                    Text(node.name)
                        .font(GaryxFont.callout(weight: .medium))
                        .lineLimit(1)
                }
                .buttonStyle(.plain)
                Spacer(minLength: 0)
                Button(role: .destructive) {
                    showsDeleteConfirmation = true
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(GaryxMiniIconButtonStyle())
            }
            .padding(.leading, CGFloat(depth) * 14)

            ForEach(node.children) { child in
                GaryxSkillEntryRow(skillId: skillId, node: child, depth: depth + 1, onOpenFile: onOpenFile)
            }
        }
        .confirmationDialog("Delete skill entry?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSkillEntry(skillId: skillId, path: node.path) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(node.path)
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
    @State private var showsDeleteConfirmation = false
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
        .confirmationDialog("Delete command?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSlashCommand(command) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the slash command.")
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
                showsDeleteConfirmation = true
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
    @State private var showsDeleteConfirmation = false
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
        .confirmationDialog("Delete MCP server?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteMcpServer(server) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(server.name)
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
                showsDeleteConfirmation = true
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
    @State private var detailRun: GaryxAutoResearchRun?

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
                                GaryxAutoResearchRunCard(run: run) {
                                    detailRun = run
                                    Task { await model.loadAutoResearchDetail(run) }
                                }
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
        .sheet(item: $detailRun) { run in
            GaryxAutoResearchDetailSheet(run: run)
        }
    }
}

struct GaryxCreateAutoResearchCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Create Auto Research Run")
            TextField("Goal", text: $model.draftAutoResearchGoal, axis: .vertical)
                .lineLimit(2...5)
                .garyxInputStyle()
            if workspacePaths.isEmpty {
                Text("No workspaces available")
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
            } else {
                Picker("Workspace", selection: workspaceSelection) {
                    ForEach(workspacePaths, id: \.self) { path in
                        Text(path.lastPathComponent).tag(path)
                    }
                }
                .pickerStyle(.menu)
                .garyxInputStyle()
            }
            HStack {
                TextField("Iterations", text: $model.draftAutoResearchIterations)
                    .keyboardType(.numberPad)
                    .garyxInputStyle()
                TextField("Budget min", text: $model.draftAutoResearchTimeBudgetMinutes)
                    .keyboardType(.numberPad)
                    .garyxInputStyle()
            }
            HStack {
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
                .disabled(!canStart)
            }
        }
        .garyxCardStyle()
        .onAppear(perform: ensureWorkspaceSelection)
    }

    private var workspacePaths: [String] {
        model.knownWorkspacePaths
    }

    private var workspaceSelection: Binding<String> {
        Binding {
            effectiveWorkspacePath
        } set: { value in
            model.selectedWorkspacePath = value
        }
    }

    private var effectiveWorkspacePath: String {
        let selected = model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, workspacePaths.contains(selected) {
            return selected
        }
        return workspacePaths.first ?? ""
    }

    private var canStart: Bool {
        !model.draftAutoResearchGoal.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !effectiveWorkspacePath.isEmpty
            && positiveInteger(model.draftAutoResearchIterations) != nil
            && positiveAutoResearchBudgetMinutes(model.draftAutoResearchTimeBudgetMinutes) != nil
    }

    private func ensureWorkspaceSelection() {
        let nextSelection = effectiveWorkspacePath
        if model.selectedWorkspacePath != nextSelection {
            model.selectedWorkspacePath = nextSelection
        }
    }

    private func positiveInteger(_ value: String) -> Int? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = Int(trimmed), parsed > 0 else { return nil }
        return parsed
    }

    private func positiveAutoResearchBudgetMinutes(_ value: String) -> Int? {
        guard let parsed = positiveInteger(value), parsed <= Int.max / 60 else { return nil }
        return parsed
    }
}

struct GaryxAutoResearchRunCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let run: GaryxAutoResearchRun
    let onOpenDetail: () -> Void
    @State private var showsDeleteConfirmation = false

    var body: some View {
        GaryxSwipeActionRow(actions: researchSwipeActions) {
            Button(action: onOpenDetail) {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(alignment: .center, spacing: 10) {
                        Image(systemName: "atom")
                            .font(GaryxFont.system(size: 15, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: 24, height: 24)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(run.goal.isEmpty ? run.runId : run.goal)
                                .font(GaryxFont.body(weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(2)
                            Text(run.workspaceDir?.lastPathComponent ?? run.runId)
                                .font(GaryxFont.caption(weight: .medium))
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        GaryxStatusPill(text: garyxAutoResearchStateLabel(run.state), tone: researchTone)
                    }
                    Text("\(run.iterationsUsed) of \(run.maxIterations) iterations")
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 11)
            }
            .buttonStyle(.plain)
            .accessibilityHint("Open Auto Research details")
        }
        .confirmationDialog("Delete Auto Research run?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteAutoResearchRun(run) }
            }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("This removes the run, iterations, and candidates.")
        }
    }

    private var researchSwipeActions: [GaryxSwipeAction] {
        var actions: [GaryxSwipeAction] = []
        if !garyxAutoResearchIsTerminal(run.state) {
            actions.append(
                GaryxSwipeAction(title: "Stop", systemImage: "stop.fill", tone: .warning) {
                    Task { await model.stopAutoResearchRun(run) }
                }
            )
        }
        actions.append(
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        )
        return actions
    }

    private var researchTone: GaryxStatusPill.Tone {
        garyxAutoResearchTone(run)
    }
}

struct GaryxAutoResearchDetailSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let run: GaryxAutoResearchRun
    @State private var feedbackCandidate: GaryxResearchCandidate?
    @State private var feedbackDraft = ""

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    summaryBlock
                    iterationBlock
                    if orphanCandidates.count > 0 {
                        candidateBlock
                    }
                }
                .padding(12)
                .frame(maxWidth: 620, alignment: .leading)
                .frame(maxWidth: .infinity)
            }
            .background(GaryxTheme.background)
            .refreshable {
                await model.loadAutoResearchDetail(runId: run.runId)
            }
            .navigationTitle("Auto Research")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Done") {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    if let activeThreadId {
                        Button {
                            openThread(activeThreadId)
                        } label: {
                            Label("Open", systemImage: "arrow.up.right")
                        }
                    }
                }
            }
        }
        .task {
            await model.loadAutoResearchDetail(runId: run.runId)
        }
        .sheet(item: $feedbackCandidate, onDismiss: {
            feedbackDraft = ""
        }) { candidate in
            GaryxAutoResearchFeedbackSheet(candidate: candidate, feedback: $feedbackDraft) { feedback in
                let current = currentRun
                feedbackCandidate = nil
                feedbackDraft = ""
                Task {
                    await model.sendAutoResearchFeedback(
                        run: current,
                        candidate: candidate,
                        feedback: feedback
                    )
                }
            }
        }
    }

    private var currentRun: GaryxAutoResearchRun {
        model.autoResearchDetailsByRunId[run.runId]?.run
            ?? model.autoResearchRuns.first { $0.runId == run.runId }
            ?? run
    }

    private var detail: GaryxAutoResearchDetail? {
        model.autoResearchDetailsByRunId[run.runId]
    }

    private var candidatesPage: GaryxAutoResearchCandidatesPage? {
        model.researchCandidatesByRunId[run.runId]
    }

    private var candidates: [GaryxResearchCandidate] {
        candidatesPage?.candidates ?? []
    }

    private var candidatesByIteration: [Int: GaryxResearchCandidate] {
        var result: [Int: GaryxResearchCandidate] = [:]
        for candidate in candidates {
            result[candidate.iteration] = candidate
        }
        return result
    }

    private var displayIterations: [GaryxAutoResearchIteration] {
        var items = model.autoResearchIterationsByRunId[run.runId] ?? []
        if let latest = detail?.latestIteration,
           !items.contains(where: { $0.iterationIndex == latest.iterationIndex }) {
            items.append(latest)
        }
        return items.sorted { $0.iterationIndex < $1.iterationIndex }
    }

    private var orphanCandidates: [GaryxResearchCandidate] {
        let iterationIds = Set(displayIterations.map(\.iterationIndex))
        return candidates
            .filter { !iterationIds.contains($0.iteration) }
            .sorted { $0.iteration > $1.iteration }
    }

    private var summaryBlock: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "atom")
                    .font(GaryxFont.system(size: 16, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)
                    .background(Color(.secondarySystemGroupedBackground), in: Circle())
                VStack(alignment: .leading, spacing: 5) {
                    Text(currentRun.goal.isEmpty ? currentRun.runId : currentRun.goal)
                        .font(GaryxFont.body(weight: .semibold))
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(summarySubtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Spacer(minLength: 0)
                GaryxStatusPill(text: garyxAutoResearchStateLabel(currentRun.state), tone: garyxAutoResearchTone(currentRun))
            }
            if let terminalReason {
                Text(terminalReason)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            HStack(spacing: 8) {
                GaryxAutoResearchMetricPill(
                    title: "Iterations",
                    value: "\(currentRun.iterationsUsed) of \(currentRun.maxIterations)"
                )
                if let selectedCandidate {
                    GaryxAutoResearchMetricPill(
                        title: "Winner",
                        value: candidateMetricValue(selectedCandidate)
                    )
                } else if let bestCandidate {
                    GaryxAutoResearchMetricPill(
                        title: "Best",
                        value: candidateMetricValue(bestCandidate)
                    )
                }
                Spacer(minLength: 0)
            }
            if let activeThreadId {
                Button {
                    openThread(activeThreadId)
                } label: {
                    Label("Open Active Thread", systemImage: "arrow.up.right")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(GaryxSecondaryButtonStyle())
            }
        }
        .padding(12)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }

    private var iterationBlock: some View {
        GaryxSectionBlock(title: "Iterations") {
            if displayIterations.isEmpty {
                Text("No iteration records yet.")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 4)
            } else {
                GaryxCompactListGroup {
                    ForEach(Array(displayIterations.enumerated()), id: \.element.id) { index, iteration in
                        let candidate = candidatesByIteration[iteration.iterationIndex]
                        GaryxResearchIterationRow(
                            iteration: iteration,
                            candidate: candidate,
                            isBest: candidate?.candidateId == candidatesPage?.bestCandidateId,
                            isSelected: candidate?.candidateId == currentRun.selectedCandidate,
                            isRunTerminal: garyxAutoResearchIsTerminal(currentRun.state),
                            onSelect: { candidate in
                                Task { await model.selectAutoResearchCandidate(run: currentRun, candidate: candidate) }
                            },
                            onReverify: { candidate in
                                Task { await model.reverifyAutoResearchCandidate(run: currentRun, candidate: candidate) }
                            },
                            onFeedback: openFeedback,
                            onOpenThread: openThread
                        )
                        if index < displayIterations.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                }
            }
        }
    }

    private var candidateBlock: some View {
        GaryxSectionBlock(title: "Candidates") {
            GaryxCompactListGroup {
                ForEach(Array(orphanCandidates.enumerated()), id: \.element.id) { index, candidate in
                    GaryxResearchCandidateRow(
                        candidate: candidate,
                        isBest: candidate.candidateId == candidatesPage?.bestCandidateId,
                        isSelected: candidate.candidateId == currentRun.selectedCandidate,
                        isRunTerminal: garyxAutoResearchIsTerminal(currentRun.state),
                        onSelect: {
                            Task { await model.selectAutoResearchCandidate(run: currentRun, candidate: candidate) }
                        },
                        onReverify: {
                            Task { await model.reverifyAutoResearchCandidate(run: currentRun, candidate: candidate) }
                        },
                        onFeedback: {
                            openFeedback(candidate)
                        }
                    )
                    if index < orphanCandidates.count - 1 {
                        GaryxCompactRowDivider()
                    }
                }
            }
        }
    }

    private var summarySubtitle: String {
        let workspace = currentRun.workspaceDir?.lastPathComponent ?? "No workspace"
        let updated = garyxFormattedTaskTimestamp(currentRun.updatedAt)
        return updated.isEmpty ? workspace : "\(workspace) · updated \(updated)"
    }

    private var activeThreadId: String? {
        let value = detail?.activeThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    private var terminalReason: String? {
        let value = currentRun.terminalReason?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : garyxAutoResearchReasonLabel(value)
    }

    private var selectedCandidate: GaryxResearchCandidate? {
        let selectedId = currentRun.selectedCandidate?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !selectedId.isEmpty else { return nil }
        return candidates.first { $0.candidateId == selectedId }
    }

    private var bestCandidate: GaryxResearchCandidate? {
        let bestId = candidatesPage?.bestCandidateId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !bestId.isEmpty else { return nil }
        return candidates.first { $0.candidateId == bestId }
    }

    private func candidateMetricValue(_ candidate: GaryxResearchCandidate) -> String {
        if let score = candidate.verdict?.score {
            return String(format: "%.1f/10", score)
        }
        return "Candidate \(candidate.iteration)"
    }

    private func openFeedback(_ candidate: GaryxResearchCandidate) {
        feedbackCandidate = candidate
        feedbackDraft = ""
    }

    private func openThread(_ threadId: String?) {
        let threadId = threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !threadId.isEmpty else { return }
        dismiss()
        Task { await model.openThread(id: threadId) }
    }
}

struct GaryxAutoResearchMetricPill: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
            Text(value)
                .font(GaryxFont.footnote(weight: .semibold))
                .foregroundStyle(.primary)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

struct GaryxAutoResearchFeedbackSheet: View {
    @Environment(\.dismiss) private var dismiss
    let candidate: GaryxResearchCandidate
    @Binding var feedback: String
    let onSend: (String) -> Void

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 12) {
                TextEditor(text: $feedback)
                    .font(GaryxFont.body())
                    .scrollContentBackground(.hidden)
                    .padding(8)
                    .frame(minHeight: 160)
                    .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .stroke(GaryxTheme.hairline, lineWidth: 1)
                    }
                Text("\(feedback.trimmingCharacters(in: .whitespacesAndNewlines).count) characters")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
            .padding(16)
            .background(GaryxTheme.background)
            .navigationTitle("Feedback on Candidate \(candidate.iteration)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Cancel") {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Send") {
                        let value = feedback.trimmingCharacters(in: .whitespacesAndNewlines)
                        onSend(value)
                        dismiss()
                    }
                    .disabled(feedback.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}

struct GaryxResearchIterationRow: View {
    let iteration: GaryxAutoResearchIteration
    let candidate: GaryxResearchCandidate?
    let isBest: Bool
    let isSelected: Bool
    let isRunTerminal: Bool
    let onSelect: (GaryxResearchCandidate) -> Void
    let onReverify: (GaryxResearchCandidate) -> Void
    let onFeedback: (GaryxResearchCandidate) -> Void
    let onOpenThread: (String?) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 8) {
                Text("Iteration \(iteration.iterationIndex)")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                GaryxStatusPill(
                    text: garyxAutoResearchStateLabel(iteration.state.isEmpty ? "pending" : iteration.state),
                    tone: garyxAutoResearchTone(iteration.state)
                )
                if isSelected {
                    GaryxStatusPill(text: "Winner", tone: .good)
                } else if isBest {
                    GaryxStatusPill(text: "Current best", tone: .good)
                }
                Spacer(minLength: 0)
            }
            if let candidate {
                GaryxResearchCandidateContent(candidate: candidate)
                GaryxResearchCandidateActions(
                    candidate: candidate,
                    isSelected: isSelected,
                    isRunTerminal: isRunTerminal,
                    onSelect: { onSelect(candidate) },
                    onReverify: { onReverify(candidate) },
                    onFeedback: { onFeedback(candidate) }
                )
            } else {
                Text(iteration.state.lowercased() == "completed" ? "No candidate recorded for this iteration." : "This iteration is still running.")
                    .font(GaryxFont.footnote())
                    .foregroundStyle(.secondary)
            }
            if hasThreadLinks {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) {
                        threadLinkControls
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        threadLinkControls
                    }
                }
            }
        }
        .padding(10)
    }

    private var workThreadId: String? {
        let value = iteration.workThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    private var verifyThreadId: String? {
        let value = iteration.verifyThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    private var hasThreadLinks: Bool {
        workThreadId != nil || verifyThreadId != nil
    }

    @ViewBuilder
    private var threadLinkControls: some View {
        if let workThreadId {
            Button {
                onOpenThread(workThreadId)
            } label: {
                Label("Work", systemImage: "doc.text")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
        if let verifyThreadId {
            Button {
                onOpenThread(verifyThreadId)
            } label: {
                Label("Verify", systemImage: "checkmark.seal")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
    }
}

struct GaryxResearchCandidateRow: View {
    let candidate: GaryxResearchCandidate
    let isBest: Bool
    let isSelected: Bool
    let isRunTerminal: Bool
    let onSelect: () -> Void
    let onReverify: () -> Void
    let onFeedback: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack {
                Text("Candidate \(candidate.iteration)")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                if isSelected {
                    GaryxStatusPill(text: "Winner", tone: .good)
                } else if isBest {
                    GaryxStatusPill(text: "Current best", tone: .good)
                }
                Spacer(minLength: 0)
            }
            GaryxResearchCandidateContent(candidate: candidate)
            GaryxResearchCandidateActions(
                candidate: candidate,
                isSelected: isSelected,
                isRunTerminal: isRunTerminal,
                onSelect: onSelect,
                onReverify: onReverify,
                onFeedback: onFeedback
            )
        }
        .padding(10)
    }
}

struct GaryxResearchCandidateContent: View {
    let candidate: GaryxResearchCandidate

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(candidate.output.isEmpty ? "No candidate output yet." : candidate.output)
                .font(GaryxFont.footnote())
                .foregroundStyle(.secondary)
                .lineLimit(8)
            if let verdict = candidate.verdict {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Score \(String(format: "%.1f", verdict.score))/10")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.primary)
                    if !verdict.feedback.isEmpty {
                        Text(verdict.feedback)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(3)
                    }
                }
            }
        }
    }
}

struct GaryxResearchCandidateActions: View {
    let candidate: GaryxResearchCandidate
    let isSelected: Bool
    let isRunTerminal: Bool
    let onSelect: () -> Void
    let onReverify: () -> Void
    let onFeedback: () -> Void

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                controls
            }
            VStack(alignment: .leading, spacing: 8) {
                controls
            }
        }
    }

    @ViewBuilder
    private var controls: some View {
        if isSelected {
            GaryxStatusPill(text: "Selected Winner", tone: .good)
                .fixedSize(horizontal: true, vertical: false)
        } else {
            Button {
                onSelect()
            } label: {
                Label("Select", systemImage: "checkmark")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
        if !isRunTerminal {
            Button {
                onReverify()
            } label: {
                Label("Reverify", systemImage: "arrow.clockwise")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
            Button {
                onFeedback()
            } label: {
                Label("Feedback", systemImage: "text.bubble")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
    }
}

func garyxAutoResearchIsTerminal(_ state: String) -> Bool {
    switch state.lowercased() {
    case "user_stopped", "budget_exhausted", "blocked":
        true
    default:
        false
    }
}

func garyxAutoResearchStateLabel(_ state: String) -> String {
    switch state.lowercased() {
    case "queued":
        "Queued"
    case "researching":
        "Researching"
    case "judging":
        "Judging"
    case "budget_exhausted":
        "Budget exhausted"
    case "blocked":
        "Blocked"
    case "user_stopped":
        "Stopped"
    case "completed":
        "Completed"
    case "pending":
        "Pending"
    default:
        state
            .split(separator: "_")
            .map { word in
                word.prefix(1).uppercased() + String(word.dropFirst())
            }
            .joined(separator: " ")
    }
}

func garyxAutoResearchReasonLabel(_ reason: String) -> String {
    let normalized = reason.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !normalized.isEmpty else { return "" }
    switch normalized.lowercased() {
    case "user_requested", "user_stopped":
        return "Stopped by user"
    case "time_budget_exhausted":
        return "Time budget exhausted"
    case "budget_exhausted":
        return "Budget exhausted"
    case "blocked":
        return "Blocked"
    default:
        return normalized
            .split(separator: "_")
            .map { word in
                word.prefix(1).uppercased() + String(word.dropFirst())
            }
            .joined(separator: " ")
    }
}

func garyxAutoResearchTone(_ run: GaryxAutoResearchRun) -> GaryxStatusPill.Tone {
    let selected = run.selectedCandidate?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    if !selected.isEmpty {
        return .good
    }
    return garyxAutoResearchTone(run.state)
}

func garyxAutoResearchTone(_ state: String) -> GaryxStatusPill.Tone {
    switch state.lowercased() {
    case "completed":
        .good
    case "blocked":
        .danger
    case "user_stopped", "budget_exhausted":
        .muted
    default:
        .warning
    }
}

struct GaryxBotsView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxPanelScaffold(
            title: "Bots",
            subtitle: subtitle,
            onRefresh: { await model.refreshRemoteState() }
        ) {
            GaryxBotsContent()
        }
    }

    private var subtitle: String {
        let groups = model.mobileBotGroups
        let endpointCount = groups.reduce(0) { $0 + $1.endpointCount }
        guard endpointCount > 0 else {
            return "\(groups.count) bots"
        }
        return "\(groups.count) bots · \(endpointCount) endpoints"
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
                            ForEach(sortedEndpoints(for: group)) { endpoint in
                                GaryxCompactRowDivider()
                                GaryxBotEndpointRow(endpoint: endpoint)
                            }
                            if index < groups.count - 1 {
                                GaryxCompactGroupDivider()
                            }
                        }
                    }
                }
            }
        }
    }

    private func sortedEndpoints(for group: GaryxMobileBotGroup) -> [GaryxChannelEndpoint] {
        group.endpoints.sorted { lhs, rhs in
            let labelOrder = lhs.displayLabel.localizedCaseInsensitiveCompare(rhs.displayLabel)
            if labelOrder != .orderedSame {
                return labelOrder == .orderedAscending
            }
            return lhs.endpointKey.localizedCaseInsensitiveCompare(rhs.endpointKey) == .orderedAscending
        }
    }
}

struct GaryxBotGroupRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let group: GaryxMobileBotGroup
    @State private var showsAccountActions = false
    @State private var showsDeleteConfirmation = false

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

                if group.endpointCount > 0 {
                    Text("\(group.boundEndpointCount)/\(group.endpointCount)")
                        .font(GaryxFont.system(size: 11, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 3)
                        .background(Color(.tertiarySystemFill), in: Capsule())
                        .accessibilityLabel("\(group.boundEndpointCount) of \(group.endpointCount) endpoints linked")
                }
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 8)
        }
        .confirmationDialog("Bot Actions", isPresented: $showsAccountActions, titleVisibility: .visible) {
            if configuredBot != nil {
                Button("Delete", systemImage: "trash", role: .destructive) {
                    showsDeleteConfirmation = true
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog("Delete bot account?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                if let configuredBot {
                    Task { await model.deleteConfiguredBotAccount(configuredBot) }
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the channel account from the gateway configuration.")
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
            if !mainThreadId.isEmpty {
                actions.append(
                    GaryxSwipeAction(title: "Unbind", systemImage: "link.badge.minus") {
                        Task { await model.unbindBot(configuredBot) }
                    }
                )
            }
            actions.append(
                GaryxSwipeAction(title: "More", systemImage: "ellipsis.circle") {
                    showsAccountActions = true
                }
            )
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

struct GaryxBotEndpointRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let endpoint: GaryxChannelEndpoint
    @State private var showsBindConfirmation = false

    var body: some View {
        GaryxSwipeActionRow(actions: endpointActions) {
            HStack(alignment: .center, spacing: 10) {
                Image(systemName: endpointIconName)
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(.primary)
                    .frame(width: 26, height: 26)
                    .background(Color(.tertiarySystemFill), in: Circle())

                VStack(alignment: .leading, spacing: 3) {
                    Text(endpointTitle)
                        .font(GaryxFont.subheadline(weight: .medium))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(endpointDetail)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 6)

                GaryxStatusPill(text: statusText, tone: statusTone)
                    .padding(.leading, 12)
            }
            .padding(.leading, 34)
            .padding(.trailing, 9)
            .padding(.vertical, 8)
        }
        .confirmationDialog("Bind endpoint?", isPresented: $showsBindConfirmation, titleVisibility: .visible) {
            Button("Bind to \(selectedThreadTitle)") {
                Task { await model.bindEndpointToSelectedThread(endpoint) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("\(endpointTitle) will be linked to \(selectedThreadTitle).")
        }
    }

    private var endpointActions: [GaryxSwipeAction] {
        var actions: [GaryxSwipeAction] = []
        let threadId = boundThreadId
        if !threadId.isEmpty {
            actions.append(
                GaryxSwipeAction(title: "Open", systemImage: "arrow.up.right", tone: .accent) {
                    Task { await model.openBotThread(threadId) }
                }
            )
        }
        if let selectedThreadId, selectedThreadId != threadId {
            actions.append(
                GaryxSwipeAction(title: "Bind", systemImage: "link") {
                    showsBindConfirmation = true
                }
            )
        }
        if !threadId.isEmpty {
            actions.append(
                GaryxSwipeAction(title: "Detach", systemImage: "link.badge.minus", tone: .warning) {
                    Task { await model.detachEndpoint(endpoint) }
                }
            )
        }
        return actions
    }

    private var endpointTitle: String {
        firstHumanLabel(endpoint.displayLabel, endpoint.conversationLabel, endpoint.threadLabel)
            ?? friendlyEndpointFallback
    }

    private var endpointDetail: String {
        if !boundThreadId.isEmpty {
            return "Linked · \(firstHumanLabel(endpoint.threadLabel, endpoint.conversationLabel) ?? friendlyEndpointFallback)"
        }
        return "Unlinked · \(firstHumanLabel(endpoint.conversationLabel) ?? friendlyEndpointFallback)"
    }

    private var boundThreadId: String {
        endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }

    private var selectedThreadId: String? {
        let threadId = model.selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return threadId.isEmpty ? nil : threadId
    }

    private var selectedThreadTitle: String {
        firstNonEmpty(model.selectedThread?.title, selectedThreadId, "selected thread")
    }

    private var statusText: String {
        boundThreadId.isEmpty ? "Unlinked" : "Linked"
    }

    private var statusTone: GaryxStatusPill.Tone {
        boundThreadId.isEmpty ? .muted : .good
    }

    private var conversationKindLabel: String {
        let kind = endpoint.conversationKind?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        if kind.contains("group") || kind.contains("room") {
            return "Group chat"
        }
        if kind.contains("direct") || kind.contains("private") || kind.contains("dm") {
            return "Direct chat"
        }
        if kind.contains("channel") {
            return "Channel"
        }
        return kind.isEmpty ? "" : kind.capitalized
    }

    private var channelLabel: String {
        let channel = endpoint.channel.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !channel.isEmpty else { return "" }
        switch channel.lowercased() {
        case "api":
            return "API"
        case "telegram":
            return "Telegram"
        case "discord":
            return "Discord"
        default:
            return channel.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    private var friendlyEndpointFallback: String {
        let kind = conversationKindLabel
        if !kind.isEmpty {
            return "\(channelLabel.isEmpty ? "Channel" : channelLabel) \(kind.lowercased())"
        }
        return channelLabel.isEmpty ? "Endpoint" : channelLabel
    }

    private var endpointIconName: String {
        let kind = endpoint.conversationKind?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        if kind.contains("group") || kind.contains("room") || kind.contains("channel") {
            return "person.2.fill"
        }
        if kind.contains("direct") || kind.contains("private") || kind.contains("dm") {
            return "person.fill"
        }
        return "bubble.left.fill"
    }

    private func firstNonEmpty(_ values: String?...) -> String {
        for value in values {
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if !trimmed.isEmpty {
                return trimmed
            }
        }
        return "Endpoint"
    }

    private func firstHumanLabel(_ values: String?...) -> String? {
        for value in values {
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if !trimmed.isEmpty, !isTechnicalEndpointLabel(trimmed) {
                return trimmed
            }
        }
        return nil
    }

    private func isTechnicalEndpointLabel(_ value: String) -> Bool {
        let lowercased = value.lowercased()
        if lowercased.hasPrefix("thread-") || lowercased.contains("::") {
            return true
        }
        if lowercased.contains("/") {
            return value.rangeOfCharacter(from: .decimalDigits) != nil
        }
        if lowercased.contains("...") {
            return value.rangeOfCharacter(from: .decimalDigits) != nil
        }
        return false
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
            onRefresh: { await model.connectAndRefresh() },
            leadingActionLabel: settingsLeadingActionLabel,
            leadingActionSystemName: "chevron.left",
            leadingAction: settingsLeadingAction,
            background: GaryxTheme.background
        ) {
            VStack(alignment: .leading, spacing: 12) {
                GaryxSettingsTabContent()
            }
        } actions: {
            HStack(spacing: 8) {
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
                case .manage, .provider, .channels:
                    EmptyView()
                }
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

    private var settingsLeadingActionLabel: String? {
        model.activeSettingsTab == .manage ? nil : "All Settings"
    }

    private var settingsLeadingAction: (() -> Void)? {
        guard model.activeSettingsTab != .manage else { return nil }
        return {
            model.showSettingsOverview()
        }
    }
}

struct GaryxSettingsTabContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        switch model.activeSettingsTab {
        case .manage:
            GaryxSettingsOverviewContent()
        case .gateway:
            GaryxSettingsDetailContent {
                GaryxSettingsGatewayContent()
            }
        case .provider:
            GaryxSettingsDetailContent {
                GaryxSettingsProviderContent()
            }
        case .channels:
            GaryxSettingsDetailContent {
                GaryxBotsContent()
            }
        case .commands:
            GaryxSettingsDetailContent {
                GaryxCommandsContent()
            }
        case .mcp:
            GaryxSettingsDetailContent {
                GaryxMcpServersContent()
            }
        }
    }
}

struct GaryxSettingsOverviewContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    private var managementPanels: [GaryxMobilePanel] {
        [
            model.dreamsAutoScanEnabled ? .dreams : nil,
            .tasks,
            .autoResearch,
            .agents,
            .skills,
        ].compactMap { $0 }
    }
    private let settingsTabs: [GaryxMobileSettingsTab] = [
        .gateway,
        .provider,
        .channels,
        .commands,
        .mcp,
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            GaryxSettingsOverviewSection(title: "Manage") {
                ForEach(Array(managementPanels.enumerated()), id: \.element.id) { index, panel in
                    GaryxSettingsPanelLinkRow(panel: panel)
                    if index < managementPanels.count - 1 {
                        Divider()
                            .padding(.leading, 54)
                    }
                }
            }

            GaryxSettingsOverviewSection(title: "Settings") {
                GaryxDreamsAutoScanRow()
                Divider()
                    .padding(.leading, 54)

                ForEach(Array(settingsTabs.enumerated()), id: \.element.id) { index, tab in
                    GaryxSettingsTabLinkRow(tab: tab)
                    if index < settingsTabs.count - 1 {
                        Divider()
                            .padding(.leading, 54)
                    }
                }
            }
        }
    }
}

struct GaryxSettingsOverviewSection<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 16)

            VStack(spacing: 0) {
                content
            }
            .background(GaryxTheme.surface)
        }
    }
}

struct GaryxSettingsDetailContent<Content: View>: View {
    let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            content
        }
    }
}

struct GaryxSettingsPanelLinkRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let panel: GaryxMobilePanel

    var body: some View {
        Button {
            model.openPanel(panel)
        } label: {
            HStack(spacing: 10) {
                Image(systemName: panel.iconName)
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(panel.label)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(panel.label)
    }

    private var subtitle: String {
        switch panel {
        case .workspaces:
            "\(model.knownWorkspacePaths.count) workspaces"
        case .dreams:
            "\(model.dreams.count) topics"
        case .tasks:
            "\(model.activeTaskCount) active / \(model.tasks.count) total"
        case .autoResearch:
            "\(model.runningResearchCount) active / \(model.autoResearchRuns.count) total"
        case .workspaceBots:
            "\(model.mobileBotGroups.count) bots / \(visibleWorkspaceCount) workspaces"
        case .agents:
            "\(model.agents.count) agents / \(model.teams.count) teams"
        case .skills:
            "\(model.skills.filter(\.enabled).count) enabled / \(model.skills.count) total"
        default:
            ""
        }
    }

    private var visibleWorkspaceCount: Int {
        model.knownWorkspacePaths
            .filter(GaryxMobileModel.isVisibleMobileWorkspacePath)
            .count
    }
}

struct GaryxSettingsTabLinkRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let tab: GaryxMobileSettingsTab

    var body: some View {
        Button {
            model.activeSettingsTab = tab
        } label: {
            HStack(spacing: 10) {
                Image(systemName: tab.iconName)
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(tab.label)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(tab.label)
    }

    private var subtitle: String {
        switch tab {
        case .manage:
            "All mobile settings"
        case .gateway:
            model.gatewayURL.isEmpty ? "Connection and saved gateways" : model.gatewayURL
        case .provider:
            model.providerModelsByType.isEmpty ? "Model providers" : "\(model.providerModelsByType.count) provider types"
        case .channels:
            "\(model.configuredBots.count) bots / \(model.channelEndpoints.count) endpoints"
        case .commands:
            "\(model.slashCommands.count) slash commands"
        case .mcp:
            "\(model.mcpServers.count) servers"
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
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var label = ""
    @State private var gatewayUrl = ""
    @State private var token = ""

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
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Gateway") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Gateway")
                    TextField("Name", text: $label)
                        .garyxInputStyle()
                    TextField("Gateway URL", text: $gatewayUrl)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    SecureField("Gateway Token", text: $token)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    Button {
                        if model.updateGatewayProfile(
                            profile,
                            label: label,
                            gatewayUrl: gatewayUrl,
                            token: token
                        ) {
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save Gateway", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                    .disabled(gatewayUrl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                .garyxCardStyle()
            }
        }
        .confirmationDialog("Delete gateway?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                model.removeGatewayProfile(profile)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the saved gateway profile from this device.")
        }
    }

    private var profileSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "Switch", systemImage: "arrow.triangle.2.circlepath", tone: .accent) {
                Task { await model.activateGatewayProfile(profile) }
            },
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }

    private func fillDraft() {
        label = profile.label
        gatewayUrl = profile.gatewayUrl
        token = model.gatewayProfileToken(profile)
    }
}

struct GaryxSettingsProviderContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if !model.providerModelsByType.isEmpty {
                GaryxSectionBlock(title: "Model Providers") {
                    GaryxCompactListGroup {
                        let providers = model.providerModelsByType
                            .values
                            .sorted { lhs, rhs in
                                let lhsName = garyxProviderDisplayName(lhs.providerType)
                                let rhsName = garyxProviderDisplayName(rhs.providerType)
                                if lhsName != rhsName {
                                    return lhsName < rhsName
                                }
                                return lhs.providerType < rhs.providerType
                            }
                        ForEach(Array(providers.enumerated()), id: \.element.providerType) { index, provider in
                            GaryxProviderModelsRow(provider: provider)
                            if index < providers.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }

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

struct GaryxProviderModelsRow: View {
    let provider: GaryxProviderModels

    var body: some View {
        HStack(spacing: 9) {
            Image(systemName: iconName)
                .font(GaryxFont.system(size: 14, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 20, height: 20)

            VStack(alignment: .leading, spacing: 2) {
                Text(garyxProviderDisplayName(provider.providerType))
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                Text(detail)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            GaryxStatusPill(text: hasError ? "Error" : "Ready", tone: hasError ? .danger : .good)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
    }

    private var iconName: String {
        let source = provider.providerType.lowercased()
        if source.contains("codex") {
            return "chevron.left.forwardslash.chevron.right"
        }
        if source.contains("claude") || source.contains("anthropic") {
            return "sparkles"
        }
        if source.contains("gemini") || source.contains("google") {
            return "diamond.fill"
        }
        if source.contains("gpt") || source.contains("openai") {
            return "circle.hexagongrid.fill"
        }
        return "cpu"
    }

    private var hasError: Bool {
        let error = provider.error?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return !error.isEmpty
    }

    private var detail: String {
        var parts: [String] = []
        if let defaultModel = provider.defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines), !defaultModel.isEmpty {
            parts.append("Default \(defaultModel)")
        }
        if provider.supportsModelSelection {
            parts.append("\(provider.models.count) models")
        }
        if provider.supportsReasoningEffortSelection {
            parts.append("\(provider.reasoningEfforts.count) reasoning")
        }
        if provider.supportsServiceTierSelection {
            parts.append("\(provider.serviceTiers.count) tiers")
        }
        if parts.isEmpty {
            if hasError {
                return "Model metadata unavailable"
            }
            return provider.source.isEmpty ? "Provider metadata" : provider.source.capitalized
        }
        return parts.joined(separator: " · ")
    }
}

private func garyxProviderDisplayName(_ providerType: String) -> String {
    switch providerType {
    case "codex_app_server":
        return "Codex"
    case "claude_code":
        return "Claude Code"
    case "gemini_cli":
        return "Gemini CLI"
    case "gpt":
        return "OpenAI"
    case "anthropic", "claude_llm":
        return "Anthropic"
    case "google", "gemini_llm":
        return "Google"
    default:
        let words = providerType
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")
            .split(separator: " ")
            .map { word in
                word.prefix(1).uppercased() + word.dropFirst()
            }
        return words.isEmpty ? "Provider" : words.joined(separator: " ")
    }
}

struct GaryxPanelScaffold<Content: View, Actions: View>: View {
    @EnvironmentObject private var model: GaryxMobileModel

    let title: String
    let subtitle: String
    let onRefresh: (() async -> Void)?
    let leadingActionLabel: String?
    let leadingActionSystemName: String
    let leadingAction: (() -> Void)?
    let background: Color
    let content: Content
    let actions: Actions

    init(
        title: String,
        subtitle: String,
        onRefresh: (() async -> Void)? = nil,
        leadingActionLabel: String? = nil,
        leadingActionSystemName: String = "chevron.left",
        leadingAction: (() -> Void)? = nil,
        background: Color = GaryxTheme.background,
        @ViewBuilder content: () -> Content,
        @ViewBuilder actions: () -> Actions
    ) {
        self.title = title
        self.subtitle = subtitle
        self.onRefresh = onRefresh
        self.leadingActionLabel = leadingActionLabel
        self.leadingActionSystemName = leadingActionSystemName
        self.leadingAction = leadingAction
        self.background = background
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
        .background(background)
        .garyxAdaptiveTopBar {
            GaryxAdaptiveGlassContainer(spacing: 10) {
                HStack(spacing: 12) {
                    if let leadingAction {
                        Button {
                            leadingAction()
                        } label: {
                            GaryxToolbarIcon(systemName: leadingActionSystemName)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(leadingActionLabel ?? "Back")
                    } else {
                        GaryxSidebarMenuButton {
                            model.setSidebarVisible(true)
                        }
                    }

                    GaryxPanelHeaderTitle(title: title, subtitle: subtitle)
                        .layoutPriority(1)

                    Spacer(minLength: 0)

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
            }
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)
        }
    }
}

struct GaryxPanelHeaderTitle: View {
    let title: String
    let subtitle: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(GaryxFont.callout(weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)

            if !subtitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(subtitle)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
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
    }
}

extension GaryxPanelScaffold where Actions == EmptyView {
    init(
        title: String,
        subtitle: String,
        onRefresh: (() async -> Void)? = nil,
        leadingActionLabel: String? = nil,
        leadingActionSystemName: String = "chevron.left",
        leadingAction: (() -> Void)? = nil,
        background: Color = GaryxTheme.background,
        @ViewBuilder content: () -> Content
    ) {
        self.init(
            title: title,
            subtitle: subtitle,
            onRefresh: onRefresh,
            leadingActionLabel: leadingActionLabel,
            leadingActionSystemName: leadingActionSystemName,
            leadingAction: leadingAction,
            background: background,
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
    let onDone: (() -> Void)?
    let content: Content

    init(title: String, onDone: (() -> Void)? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.onDone = onDone
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
                        if let onDone {
                            onDone()
                        } else {
                            dismiss()
                        }
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
        .background(GaryxTheme.surface)
    }
}

struct GaryxCompactRowDivider: View {
    var body: some View {
        Divider()
            .overlay(GaryxTheme.hairline)
            .padding(.leading, 10)
    }
}

struct GaryxCompactGroupDivider: View {
    var body: some View {
        VStack(spacing: 0) {
            Divider()
                .overlay(GaryxTheme.hairline)
            GaryxTheme.background
                .frame(height: 7)
            Divider()
                .overlay(GaryxTheme.hairline)
        }
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

final class GaryxSwipeRowCoordinator: ObservableObject {
    static let shared = GaryxSwipeRowCoordinator()

    @Published private(set) var activeRowID: UUID?

    func open(_ id: UUID) {
        if activeRowID != id {
            activeRowID = id
        }
    }

    func close(_ id: UUID) {
        if activeRowID == id {
            activeRowID = nil
        }
    }
}

struct GaryxSwipeActionRow<Content: View>: View {
    let actions: [GaryxSwipeAction]
    let content: Content

    @State private var identityID = UUID()
    @ObservedObject private var coordinator = GaryxSwipeRowCoordinator.shared
    @State private var offset: CGFloat = 0
    @State private var isOpen = false
    @State private var dragBaseOffset: CGFloat = 0
    @State private var dragDirectionLocked = false
    @State private var dragIsHorizontal = false

    init(actions: [GaryxSwipeAction], @ViewBuilder content: () -> Content) {
        self.actions = actions
        self.content = content()
    }

    var body: some View {
        if actions.isEmpty {
            content
        } else {
            ZStack(alignment: .trailing) {
                if abs(offset) > 0.5 {
                    actionsHStack
                }

                content
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(GaryxTheme.surface)
                    .offset(x: offset)
                    .contentShape(Rectangle())
                    .accessibilityHint("Swipe left for actions, or use the actions rotor.")
                    .modifier(GaryxSwipeRowAccessibilityActions(actions: actions, onAction: handle))
                    .contextMenu {
                        ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                            Button(action.title, role: action.tone == .destructive ? .destructive : nil) {
                                handle(action)
                            }
                        }
                    }
                    .overlay {
                        if isOpen {
                            Color.clear
                                .contentShape(Rectangle())
                                .onTapGesture { close() }
                        }
                    }
            }
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .clipped()
            .simultaneousGesture(swipeDragGesture)
            .onChange(of: coordinator.activeRowID) { _, newID in
                if newID != identityID, isOpen {
                    close(notifyCoordinator: false)
                }
            }
            .onDisappear {
                coordinator.close(identityID)
            }
        }
    }

    private var actionsHStack: some View {
        HStack(spacing: 0) {
            ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                Button(role: action.tone == .destructive ? .destructive : nil) {
                    handle(action)
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
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .padding(.vertical, 8)
                    .contentShape(Rectangle())
                }
                .frame(width: actionButtonWidth)
                .frame(maxHeight: .infinity)
                .background(action.tone.background)
                .buttonStyle(.plain)
            }
        }
        .frame(width: actionWidth, alignment: .trailing)
        .frame(maxHeight: .infinity)
        .offset(x: actionWidth + offset)
    }

    private var actionButtonWidth: CGFloat { 72 }
    private var actionWidth: CGFloat { CGFloat(actions.count) * actionButtonWidth }

    private var swipeDragGesture: some Gesture {
        DragGesture(minimumDistance: 10, coordinateSpace: .local)
            .onChanged { value in
                if !dragDirectionLocked {
                    let dx = value.translation.width
                    let dy = value.translation.height
                    guard abs(dx) > 6 || abs(dy) > 6 else { return }
                    dragDirectionLocked = true
                    dragIsHorizontal = abs(dx) > abs(dy) * 1.6
                    dragBaseOffset = offset
                }
                guard dragIsHorizontal else { return }
                offset = clampedSwipeOffset(dragBaseOffset + value.translation.width)
            }
            .onEnded { value in
                defer { resetDragState() }
                guard dragIsHorizontal else {
                    if isOpen {
                        close()
                    }
                    return
                }
                finishSwipe(projectedOffset: dragBaseOffset + value.predictedEndTranslation.width)
            }
    }

    private func handle(_ action: GaryxSwipeAction) {
        close()
        action.action()
    }

    private func clampedSwipeOffset(_ raw: CGFloat) -> CGFloat {
        if raw > 0 {
            return 0
        }
        if raw < -actionWidth {
            let overshoot = raw + actionWidth
            return -actionWidth + overshoot * 0.35
        }
        return raw
    }

    private func finishSwipe(projectedOffset: CGFloat) {
        let opening = projectedOffset < -actionWidth * 0.5
        let target: CGFloat = opening ? -actionWidth : 0
        isOpen = opening
        if opening {
            coordinator.open(identityID)
        } else {
            coordinator.close(identityID)
        }
        withAnimation(GaryxMobileMotion.rowSwipe) {
            offset = target
        }
    }

    private func resetDragState() {
        dragBaseOffset = 0
        dragDirectionLocked = false
        dragIsHorizontal = false
    }

    private func close(notifyCoordinator: Bool = true) {
        if notifyCoordinator {
            coordinator.close(identityID)
        }
        isOpen = false
        withAnimation(GaryxMobileMotion.rowSwipe) {
            offset = 0
        }
    }
}

private struct GaryxSwipeRowAccessibilityActions: ViewModifier {
    let actions: [GaryxSwipeAction]
    let onAction: (GaryxSwipeAction) -> Void

    func body(content: Content) -> some View {
        content.accessibilityActions {
            ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                Button(action.title) {
                    onAction(action)
                }
            }
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
            } else if let image = builtInFallbackImage {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .padding(diameter * 0.16)
            } else if let svgDataUrl {
                GaryxSVGIconDataURLView(dataURL: svgDataUrl)
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
        GaryxDataURLImageCache.image(from: iconDataUrl)
    }

    private var builtInFallbackImage: UIImage? {
        switch channel.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "telegram":
            UIImage(named: "ChannelTelegram")
        case "discord":
            UIImage(named: "ChannelDiscord")
        case "feishu":
            UIImage(named: "ChannelFeishu")
        case "weixin":
            UIImage(named: "ChannelWeixin")
        default:
            nil
        }
    }

    private var svgDataUrl: String? {
        let raw = (iconDataUrl ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard raw.lowercased().hasPrefix("data:image/svg+xml") else { return nil }
        return raw
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

private struct GaryxSVGIconDataURLView: UIViewRepresentable {
    let dataURL: String

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.backgroundColor = .clear
        webView.scrollView.isScrollEnabled = false
        webView.isUserInteractionEnabled = false
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        guard context.coordinator.loadedDataURL != dataURL else { return }
        context.coordinator.loadedDataURL = dataURL
        webView.loadHTMLString(Self.html(for: dataURL), baseURL: nil)
    }

    private static func html(for dataURL: String) -> String {
        """
        <!doctype html>
        <html>
          <head>
            <meta name="viewport" content="width=device-width,initial-scale=1">
            <style>
              html, body {
                background: transparent;
                height: 100%;
                margin: 0;
                overflow: hidden;
                width: 100%;
              }
              body {
                align-items: center;
                display: flex;
                justify-content: center;
              }
              img {
                display: block;
                height: 100%;
                object-fit: contain;
                width: 100%;
              }
            </style>
          </head>
          <body><img alt="" src="\(dataURL.garyxHTMLEscapedAttribute)"></body>
        </html>
        """
    }

    final class Coordinator {
        var loadedDataURL: String?
    }
}

private extension String {
    var garyxHTMLEscapedAttribute: String {
        replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
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
        GaryxDataURLImageCache.image(from: avatarDataUrl)
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
                    .font(GaryxFont.system(size: 18, weight: .semibold))
                    .foregroundStyle(.primary)
            } else if let customContent {
                customContent()
            }
        }
        .frame(width: 44, height: 44)
        .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .ultraThinMaterial, in: Circle())
        .contentShape(Rectangle())
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
        .accessibilityLabel("Open menu")
    }
}

struct GaryxHeaderMenuIcon: View {
    var body: some View {
        Image(systemName: "line.3.horizontal")
            .font(GaryxFont.system(size: 17, weight: .semibold))
            .foregroundStyle(.primary)
            .frame(width: 44, height: 44)
            .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .ultraThinMaterial, in: Circle())
            .contentShape(Rectangle())
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
    private static let sampledLightBackground = UIColor(
        red: 253.0 / 255.0,
        green: 253.0 / 255.0,
        blue: 253.0 / 255.0,
        alpha: 1
    )
    private static let adaptivePageBackground = UIColor { traits in
        traits.userInterfaceStyle == .dark ? .systemBackground : sampledLightBackground
    }

    static let background = Color(adaptivePageBackground)
    static let sidebar = Color(adaptivePageBackground)
    static let header = Color(adaptivePageBackground)
    static let surface = Color(adaptivePageBackground)
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
            .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .thinMaterial, in: Capsule())
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
            .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .thinMaterial, in: Circle())
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
#if compiler(>=6.2)
        if #available(iOS 26, *) {
            switch style {
            case .automatic:
                content.glassEffect(in: shape)
            case .regular:
                content.glassEffect(resolvedGlass, in: shape)
            }
        } else {
            fallback(content: content)
        }
#else
        fallback(content: content)
#endif
    }

    @ViewBuilder
    private func fallback(content: Content) -> some View {
        if let tint {
            content.background(tint, in: shape)
        } else {
            content.background(fallbackMaterial, in: shape)
        }
    }

#if compiler(>=6.2)
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
#endif
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
#if compiler(>=6.2)
        if #available(iOS 26, *) {
            GlassEffectContainer(spacing: spacing) {
                content()
            }
        } else {
            content()
        }
#else
        content()
#endif
    }
}

private struct GaryxSoftScrollEdgeModifier: ViewModifier {
    let edges: Edge.Set

    func body(content: Content) -> some View {
#if compiler(>=6.2)
        if #available(iOS 26, *) {
            content.scrollEdgeEffectStyle(.soft, for: edges)
        } else {
            content
        }
#else
        content
#endif
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

    var systemImage: String {
        switch self {
        case .todo:
            "circle"
        case .inProgress:
            "play.circle.fill"
        case .inReview:
            "arrowshape.turn.up.right.circle.fill"
        case .done:
            "checkmark.circle.fill"
        }
    }

    var allowedTransitions: [GaryxTaskStatus] {
        switch self {
        case .todo:
            [.inProgress]
        case .inProgress:
            [.inReview, .todo]
        case .inReview:
            [.done, .inProgress]
        case .done:
            [.todo]
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

private extension GaryxTaskSource {
    var detailLabel: String {
        if let taskId, !taskId.isEmpty {
            return taskId
        }
        if let taskThreadId, !taskThreadId.isEmpty {
            return taskThreadId
        }
        if let threadId, !threadId.isEmpty {
            return threadId
        }
        if let botId, !botId.isEmpty {
            return botId
        }
        let channel = channel ?? ""
        let account = accountId ?? ""
        if !channel.isEmpty, !account.isEmpty {
            return "\(channel) / \(account)"
        }
        if !channel.isEmpty {
            return channel
        }
        return "Unknown"
    }
}

private extension GaryxDreamTopic {
    var sourceDisplayLabel: String {
        let normalized = source.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return "unknown" }
        return normalized.replacingOccurrences(of: "_", with: " ")
    }

    var formattedLastMessageAt: String {
        garyxFormattedTaskTimestamp(lastMessageAt)
    }
}

private extension GaryxDreamSpan {
    var threadDisplayLabel: String {
        let seqLabel = startSeq == endSeq ? "#\(startSeq)" : "#\(startSeq)-#\(endSeq)"
        let workspace = workspacePath?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lastPathComponent ?? ""
        if workspace.isEmpty {
            return "\(threadId) \(seqLabel)"
        }
        return "\(workspace) / \(seqLabel)"
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
