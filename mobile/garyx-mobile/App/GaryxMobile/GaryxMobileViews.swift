import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

enum GaryxMobileMotion {
    static let sidebar = Animation.interactiveSpring(response: 0.28, dampingFraction: 0.92, blendDuration: 0.08)
    static let sidebarDrilldown = Animation.easeOut(duration: 0.16)
    static let rowSwipe = Animation.interactiveSpring(response: 0.22, dampingFraction: 0.92, blendDuration: 0.04)
}

struct GaryxRootView: View {
    let model: GaryxMobileModel
    @Environment(GaryxHomeObservationStore.self) private var homeObservationStore

    var body: some View {
        ZStack {
            if homeObservationStore.isGatewayConfigured, case .ready = homeObservationStore.connectionState {
                GaryxShellView(
                    shellStore: model.shellChromeStore,
                    drawerStore: model.navigationDrawerStore,
                    navigationStore: model.rootNavigationPathStore,
                    routeNotFoundStore: model.routeNotFoundStore,
                    homeListStore: model.homeThreadListStore,
                    onSetSidebarVisible: { visible, animated in
                        model.setSidebarVisible(visible, animated: animated)
                    },
                    onPerformMainPanelLeadingEdgeAction: {
                        model.performMainPanelLeadingEdgeAction()
                    },
                    applyRootNavigationPath: { model.applyRootNavigationPath($0) },
                    onRefreshAll: {
                        // Pull-to-refresh awaits only the thread list; the
                        // catalog sweep refreshes in the background so the
                        // spinner ends when the list is fresh (TASK-1802 R1).
                        Task { await model.refreshRemoteState() }
                        await model.refreshThreads(source: .userPullToRefresh)
                    },
                    onRefreshSidebarThreads: {
                        await model.refreshThreads(source: .backgroundLoop)
                    },
                    onLoadMoreThreads: { trigger in
                        await model.loadMoreThreads(trigger: trigger)
                    },
                    onRetryLoadMoreThreads: {
                        await model.retryLoadMoreThreads()
                    },
                    onSelectRecentFilter: { filter in
                        model.selectRecentThreadFilter(filter)
                    },
                    onStartNewChat: {
                        model.openNewThreadDraft()
                    },
                    onOpenThread: { thread in
                        Task { await model.openThread(thread, source: .replace) }
                    },
                    onTogglePinnedThread: { threadId in
                        model.togglePinnedThread(threadId)
                    },
                    onArchiveThread: { thread in
                        await model.archiveThread(thread)
                    },
                    onOpenPanel: { panel in
                        model.openPanel(panel, source: .sidebar)
                    },
                    onOpenBotGroup: { group in
                        Task { await model.openBotGroup(group) }
                    },
                    onOpenBotDrilldown: { groupId in
                        model.openWorkspaceBotsDrilldown(.bot(groupId), source: .sidebar)
                    },
                    onOpenWorkspaceDrilldown: { path in
                        model.openWorkspaceBotsDrilldown(.workspace(path), source: .sidebar)
                    },
                    onOpenSettings: {
                        model.openSettings()
                    },
                    onSwitchGateway: { row in
                        model.switchGateway(from: row)
                    },
                    onManageGateways: {
                        model.openSettings(tab: .gateway)
                    },
                    debugShowsGatewaySwitcher: Binding(
                        get: { homeObservationStore.debugShowsGatewaySwitcher },
                        set: { model.debugShowsGatewaySwitcher = $0 }
                    )
                )
                .equatable()
            } else {
                GaryxGatewaySetupView()
            }
        }
        .garyxPageBackground()
        .overlay(alignment: .top) {
            GaryxGlobalErrorToastHost(
                topOffset: 72,
                onClearError: model.clearLastErrorIfCurrent
            )
        }
        .environment(\.garyxOpenSidebar) {
            model.setSidebarVisible(true)
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
            Task { await model.handleOpenURL(url) }
        }
        .sheet(
            isPresented: Binding(
                get: { homeObservationStore.showsSettings },
                set: { model.showsSettings = $0 }
            )
        ) {
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
    @State private var draftGatewayLabel = ""
    @State private var draftGatewayURL = ""
    @State private var draftGatewayAuthToken = ""
    @State private var draftGatewayHeaders = ""
    @State private var didInitializeDraft = false
    @State private var showsAddGateway = false

    var body: some View {
        if isSheet, showsSetupDetails {
            gatewaySettingsSheet
        } else {
            gatewaySetupNavigation
        }
    }

    private var gatewaySetupNavigation: some View {
        NavigationStack {
            Group {
                if showsSetupDetails {
                    setupForm
                } else {
                    connectingBody
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(GaryxTheme.background)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if showsSetupDetails {
                    ToolbarItem(placement: .principal) {
                        Text("Garyx")
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                    }
                }
                if isSheet {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Done") {
                            model.showsSettings = false
                            dismiss()
                        }
                    }
                }
            }
            .onAppear(perform: initializeDraft)
            .overlay(alignment: .top) {
                if isSheet {
                    GaryxGlobalErrorToastHost(
                        topOffset: 8,
                        onClearError: model.clearLastErrorIfCurrent
                    )
                }
            }
        }
    }

    private var gatewaySettingsSheet: some View {
        GaryxFormSheet(
            title: "Gateway",
            canSave: canSaveGateway && !setupIsBusy,
            onCancel: closeSettingsSheet,
            onSave: { Task { await saveGatewaySettings() } }
        ) {
            if let failureMessage {
                Section {
                    GaryxFormErrorText(text: failureMessage)
                }
            }

            GaryxFormGroupedSection(title: "Gateway") {
                GaryxFormTextFieldRow(title: "Name", text: $draftGatewayLabel)
                GaryxFormTextFieldRow(
                    title: "Gateway URL",
                    text: $draftGatewayURL,
                    valuePlacement: .below,
                    keyboardType: .URL,
                    textContentType: .URL,
                    autocapitalization: .never,
                    autocorrectionDisabled: true,
                    wrapsValue: true
                )
                GaryxFormSecureFieldRow(
                    title: "Gateway Token",
                    text: $draftGatewayAuthToken,
                    valuePlacement: .below,
                    autocapitalization: .never,
                    autocorrectionDisabled: true
                )
                GaryxGatewayHeadersEditor(text: $draftGatewayHeaders)
            }
        }
        .onAppear(perform: initializeDraft)
        .overlay(alignment: .top) {
            GaryxGlobalErrorToastHost(
                topOffset: 8,
                onClearError: model.clearLastErrorIfCurrent
            )
        }
    }

    private var connectingBody: some View {
        GaryxStartupLoadingView()
    }

    private var setupForm: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 24)

            VStack(spacing: 22) {
                Image("GaryxAppMark")
                    .resizable()
                    .scaledToFit()
                    .frame(width: 104, height: 104)
                    .shadow(color: Color(red: 0.10, green: 0.11, blue: 0.12).opacity(0.16), radius: 13, x: 0, y: 9)

                if let failureMessage {
                    Text(failureMessage)
                        .font(GaryxFont.footnote(weight: .medium))
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: 300)
                } else {
                    Text("Choose a gateway to connect.")
                        .font(GaryxFont.footnote())
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 300)
                }

                VStack(spacing: 0) {
                    ForEach(Array(model.gatewaySwitcherRows.enumerated()), id: \.element.id) { index, row in
                        if index > 0 {
                            Divider().padding(.leading, 48)
                        }
                        GaryxSetupGatewayRow(row: row) {
                            guard !setupIsBusy,
                                  let profile = model.gatewayProfiles.first(where: { $0.id == row.profileId }) else {
                                return
                            }
                            Task { await model.activateGatewayProfile(profile) }
                        }
                    }

                    if !model.gatewaySwitcherRows.isEmpty {
                        Divider().padding(.leading, 48)
                    }

                    Button {
                        showsAddGateway = true
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: "plus")
                                .font(GaryxFont.system(size: 15, weight: .semibold))
                                .foregroundStyle(.secondary)
                                .frame(width: 26, height: 26)
                            Text("Add Gateway")
                                .font(GaryxFont.callout(weight: .medium))
                                .foregroundStyle(.primary)
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 52)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Add Gateway")
                }
                .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .stroke(Color.primary.opacity(0.06), lineWidth: 1)
                }
                .frame(maxWidth: 360)
            }
            .padding(.horizontal, 24)

            Spacer(minLength: 24)
        }
        .fullScreenCover(isPresented: $showsAddGateway) {
            GaryxGatewaySetupView(isSheet: true, startsEmpty: true)
        }
    }

    private var showsSetupDetails: Bool {
        GaryxGatewaySetupPresentation.showsDetails(
            isSheet: isSheet,
            startsEmpty: startsEmpty,
            hasGatewaySettings: model.hasGatewaySettings,
            phase: setupConnectionPhase
        )
    }

    private var setupConnectionPhase: GaryxGatewaySetupConnectionPhase {
        switch model.connectionState {
        case .disconnected:
            return .disconnected
        case .checking:
            return .checking
        case .failed:
            return .failed
        case .ready:
            return .ready
        }
    }

    private var failureMessage: String? {
        if case .failed(let message) = model.connectionState, !message.isEmpty {
            return message
        }
        return nil
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
        draftGatewayLabel = startsEmpty ? "" : (model.currentGatewayProfile?.label ?? "")
        draftGatewayURL = startsEmpty ? "" : model.gatewayURL
        draftGatewayAuthToken = startsEmpty ? "" : model.gatewayAuthToken
        draftGatewayHeaders = startsEmpty ? "" : model.gatewayHeaders
        didInitializeDraft = true
    }

    private func closeSettingsSheet() {
        model.showsSettings = false
        dismiss()
    }

    private func saveGatewaySettings() async {
        guard canSaveGateway, !setupIsBusy else { return }
        model.gatewayURL = draftGatewayURL
        model.gatewayAuthToken = draftGatewayAuthToken
        model.gatewayHeaders = draftGatewayHeaders
        // Persist the gateway locally before probing the connection. A saved
        // gateway must stick around even when it cannot be reached, so the user
        // can retry it later instead of it silently disappearing. Saving with
        // the user-entered name up front also keeps the reconnect-time remember
        // inside `connectAndRefresh` from overwriting it with the URL-derived
        // default name.
        model.rememberCurrentGatewayProfile(label: draftGatewayLabel)
        await model.connectAndRefresh()
        if isSheet, case .ready = model.connectionState {
            closeSettingsSheet()
        }
    }

    private var setupIsBusy: Bool {
        if case .checking = model.connectionState {
            return true
        }
        return false
    }
}

private struct GaryxSetupGatewayRow: View {
    let row: GaryxGatewaySwitcherRow
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                if row.isCurrent {
                    GaryxSelectionCheckmark(style: .circle, size: 15)
                        .frame(width: 26, height: 26)
                } else {
                    Image(systemName: "network")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 26, height: 26)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(row.title)
                        .font(GaryxFont.callout(weight: .medium))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    if !row.subtitle.isEmpty {
                        Text(row.subtitle)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }

                Spacer(minLength: 0)
            }
            .padding(.horizontal, 14)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(row.title)
    }
}

// Drawer panel clip whose bounds are outset through the surrounding safe areas:
// the clipped panel keeps its safe-area layout while its full-bleed background
// still reaches the physical screen edges. The leading corner radius is driven
// by the drawer drag progress.
private struct GaryxDrawerPanelClipShape: Shape {
    var leadingCornerRadius: CGFloat
    var safeAreaOutsets: EdgeInsets

    var animatableData: CGFloat {
        get { leadingCornerRadius }
        set { leadingCornerRadius = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let expanded = CGRect(
            x: rect.minX - safeAreaOutsets.leading,
            y: rect.minY - safeAreaOutsets.top,
            width: rect.width + safeAreaOutsets.leading + safeAreaOutsets.trailing,
            height: rect.height + safeAreaOutsets.top + safeAreaOutsets.bottom
        )
        return UnevenRoundedRectangle(
            topLeadingRadius: leadingCornerRadius,
            bottomLeadingRadius: leadingCornerRadius,
            bottomTrailingRadius: 0,
            topTrailingRadius: 0,
            style: .continuous
        )
        .path(in: expanded)
    }
}

struct GaryxShellView: View, Equatable {
    @ObservedObject var shellStore: GaryxShellChromeStore
    @ObservedObject var drawerStore: GaryxNavigationDrawerStore
    @ObservedObject var navigationStore: GaryxRootNavigationPathStore
    @ObservedObject var routeNotFoundStore: GaryxRouteNotFoundStore
    @ObservedObject var homeListStore: GaryxHomeThreadListStore
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    let onSetSidebarVisible: (Bool, Bool) -> Void
    let onPerformMainPanelLeadingEdgeAction: () -> Void
    let applyRootNavigationPath: ([GaryxMobileRootRoute]) -> Void
    let onRefreshAll: () async -> Void
    let onRefreshSidebarThreads: () async -> Void
    let onLoadMoreThreads: (GaryxThreadListLoadMoreTrigger) async -> Void
    let onRetryLoadMoreThreads: () async -> Void
    let onSelectRecentFilter: (GaryxRecentThreadFilter) -> Void
    let onStartNewChat: () -> Void
    let onOpenThread: (GaryxThreadSummary) -> Void
    let onTogglePinnedThread: (String) -> Void
    let onArchiveThread: (GaryxThreadSummary) async -> Void
    let onOpenPanel: (GaryxMobilePanel) -> Void
    let onOpenBotGroup: (GaryxMobileBotGroup) -> Void
    let onOpenBotDrilldown: (String) -> Void
    let onOpenWorkspaceDrilldown: (String) -> Void
    let onOpenSettings: () -> Void
    let onSwitchGateway: (GaryxGatewaySwitcherRow) -> Void
    let onManageGateways: () -> Void
    @Binding var debugShowsGatewaySwitcher: Bool

    @State private var sidebarDragOffset: CGFloat = 0
    @State private var sidebarDragAxis: GaryxSidebarDragAxis?
    @State private var openSwipeActionRowId: String?
    /// Auto-resetting liveness for the drawer drag. `DragGesture.onEnded` is
    /// skipped when the system cancels a gesture, which used to leave
    /// `sidebarDragAxis` stuck on `.horizontal` and the conversation scroll
    /// permanently disabled; `@GestureState` always resets, so the
    /// `onChange(of: sidebarDragLive)` below can clean up after cancellation.
    @GestureState private var sidebarDragLive = false

    private let sidebarWidth: CGFloat = 330
    private let drawerMainPanelCornerRadius: CGFloat = 36
    private let sidebarEdgeGestureWidth: CGFloat = 24
    private let sidebarAxisDecisionDistance: CGFloat = 14
    private let sidebarAxisDecisionRatio: CGFloat = 1.5

    static func == (lhs: GaryxShellView, rhs: GaryxShellView) -> Bool {
        lhs.shellStore === rhs.shellStore
            && lhs.drawerStore === rhs.drawerStore
            && lhs.navigationStore === rhs.navigationStore
            && lhs.routeNotFoundStore === rhs.routeNotFoundStore
            && lhs.homeListStore === rhs.homeListStore
            && lhs.debugShowsGatewaySwitcher == rhs.debugShowsGatewaySwitcher
    }

    var body: some View {
        GeometryReader { proxy in
            drawerBody(
                width: drawerSidebarWidth(for: proxy.size),
                containerSize: proxy.size,
                safeAreaInsets: proxy.safeAreaInsets
            )
            .environment(\.garyxSidebarDragActive, sidebarDragAxis == .horizontal)
            .environment(\.garyxOpenSwipeActionRowId, $openSwipeActionRowId)
            .onChange(of: sidebarDragLive) { _, live in
                guard !live, sidebarDragAxis != nil else { return }
                sidebarDragAxis = nil
                resetSidebarDrag()
            }
        }
        .onChange(of: horizontalSizeClass) { _, _ in
            sidebarDragOffset = 0
        }
    }

    private func drawerSidebarWidth(for containerSize: CGSize) -> CGFloat {
        // The drawer is navigation-only now; it overlays the home list as a
        // partial sheet on every size class.
        min(sidebarWidth, containerSize.width * 0.86)
    }

    private func drawerBody(width: CGFloat, containerSize: CGSize, safeAreaInsets: EdgeInsets) -> some View {
        let revealWidth = sidebarRevealWidth(for: width)
        let drawerOffset = revealWidth - width
        let drawerProgress = drawerRevealProgress(revealWidth: revealWidth, width: width)
        // Clip bounds extend through the surrounding safe areas so the panels'
        // full-bleed backgrounds and bottom chrome aprons reach the physical
        // screen edges instead of leaving a plain background band under the
        // notch and home indicator, while panel content keeps native safe-area
        // layout.
        let clipOutsets = EdgeInsets(
            top: safeAreaInsets.top,
            leading: 0,
            bottom: safeAreaInsets.bottom,
            trailing: safeAreaInsets.trailing
        )

        // The drawer pan runs as a simultaneous gesture, so it does not cancel
        // child taps by itself: a button under the finger would still fire on
        // touch-up mid-drag and could present covers above the opened
        // sidebar. While a horizontal drag is in flight both panels' controls
        // are disabled so the in-flight tap lands dead; while any part of the
        // sidebar stays revealed, the main panel additionally rejects new
        // touches without the disabled dimming.
        let drawerDragActive = sidebarDragAxis == .horizontal

        return ZStack(alignment: .topLeading) {
            HStack(spacing: 0) {
                GaryxNavigationDrawerView(
                    drawerStore: drawerStore,
                    onOpenPanel: onOpenPanel,
                    onOpenBotGroup: onOpenBotGroup,
                    onOpenBotDrilldown: onOpenBotDrilldown,
                    onOpenWorkspaceDrilldown: onOpenWorkspaceDrilldown,
                    onOpenSettings: onOpenSettings,
                    onSwitchGateway: onSwitchGateway,
                    onManageGateways: onManageGateways,
                    debugShowsGatewaySwitcher: $debugShowsGatewaySwitcher
                )
                    .disabled(drawerDragActive)
                    .frame(width: width)
                    .frame(maxHeight: .infinity)
                    .contentShape(Rectangle())
                    .allowsHitTesting(revealWidth > width * 0.82)
                    .simultaneousGesture(closingSidebarGesture(sidebarWidth: width))

                GaryxRootNavigationView(
                    navigationStore: navigationStore,
                    routeNotFoundStore: routeNotFoundStore,
                    homeListStore: homeListStore,
                    isSidebarDragActive: drawerDragActive,
                    onOpenDrawer: {
                        onSetSidebarVisible(true, true)
                    },
                    applyRootNavigationPath: applyRootNavigationPath,
                    onRefreshAll: onRefreshAll,
                    onRefreshSidebarThreads: onRefreshSidebarThreads,
                    onLoadMoreThreads: onLoadMoreThreads,
                    onRetryLoadMoreThreads: onRetryLoadMoreThreads,
                    onSelectRecentFilter: onSelectRecentFilter,
                    onStartNewChat: onStartNewChat,
                    onOpenThread: onOpenThread,
                    onTogglePinnedThread: onTogglePinnedThread,
                    onArchiveThread: onArchiveThread
                )
                .equatable()
                    .disabled(drawerDragActive)
                    .allowsHitTesting(revealWidth == 0)
                    .overlay {
                        // While any part of the drawer is revealed, the main
                        // panel area becomes one big close target.
                        if revealWidth > 1 {
                            Color.clear
                                .contentShape(Rectangle())
                                .onTapGesture { closeSidebar() }
                                .simultaneousGesture(closingSidebarGesture(sidebarWidth: width))
                        }
                    }
                    .frame(width: containerSize.width, height: containerSize.height)
                    .garyxPageBackground()
                    .overlay(alignment: .leading) {
                        Rectangle()
                            .fill(Color.primary.opacity(0.10))
                            .frame(width: 1 / UIScreen.main.scale)
                            .opacity(drawerProgress)
                            .allowsHitTesting(false)
                    }
                    .clipShape(
                        GaryxDrawerPanelClipShape(
                            leadingCornerRadius: drawerMainPanelCornerRadius * drawerProgress,
                            safeAreaOutsets: clipOutsets
                        )
                    )
                    // A pre-baked gradient strip instead of `.shadow`: animated
                    // shadow radii force a full-screen offscreen blur of the
                    // main panel every drag frame, which drops drawer frames.
                    .overlay(alignment: .leading) {
                        LinearGradient(
                            gradient: Gradient(stops: [
                                .init(color: Color.black.opacity(0), location: 0),
                                .init(color: Color.black.opacity(0.04), location: 0.5),
                                .init(color: Color.black.opacity(0.16), location: 1),
                            ]),
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                        .frame(width: 40)
                        .padding(.vertical, -safeAreaInsets.top - safeAreaInsets.bottom)
                        .offset(x: -40)
                        .opacity(Double(drawerProgress))
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                    }
                    .contentShape(Rectangle())
                    .simultaneousGesture(openingSidebarGesture(sidebarWidth: width))
            }
            .frame(
                width: width + containerSize.width,
                height: containerSize.height,
                alignment: .topLeading
            )
            .offset(x: drawerOffset)
            .zIndex(0)

        }
        .frame(width: containerSize.width, height: containerSize.height, alignment: .topLeading)
        .clipShape(GaryxDrawerPanelClipShape(leadingCornerRadius: 0, safeAreaOutsets: clipOutsets))
        .garyxPageBackground()
    }

    private func sidebarRevealWidth(for width: CGFloat) -> CGFloat {
        if shellStore.snapshot.sidebarVisible {
            return max(0, min(width, width + sidebarDragOffset))
        }
        return max(0, min(width, sidebarDragOffset))
    }

    private func drawerRevealProgress(revealWidth: CGFloat, width: CGFloat) -> CGFloat {
        guard width > 0 else { return 0 }
        return max(0, min(1, revealWidth / width))
    }

    private func openingSidebarGesture(sidebarWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .global)
            .updating($sidebarDragLive) { _, state, _ in
                state = true
            }
            .onChanged { value in
                guard !shellStore.snapshot.sidebarVisible else { return }
                if sidebarDragAxis == nil {
                    sidebarDragAxis = decideSidebarAxis(
                        translation: value.translation,
                        startLocation: value.startLocation,
                        opening: true
                    )
                }
                guard sidebarDragAxis == .horizontal else { return }
                switch shellStore.snapshot.leadingEdgeAction {
                case .openSidebar:
                    sidebarDragOffset = max(0, min(sidebarWidth, value.translation.width))
                case .popToHome, .mainPanelBack, .settingsOverview, .workspaceBotsOverview:
                    sidebarDragOffset = 0
                }
            }
            .onEnded { value in
                // The closing gesture owns drags while the drawer is open;
                // touching the shared axis/offset here would clobber its
                // decision before it runs.
                guard !shellStore.snapshot.sidebarVisible else { return }
                defer {
                    sidebarDragAxis = nil
                }
                guard sidebarDragAxis == .horizontal else {
                    resetSidebarDrag()
                    return
                }
                let shouldOpen = value.translation.width > sidebarWidth * 0.22
                    || value.predictedEndTranslation.width > sidebarWidth * 0.35
                switch shellStore.snapshot.leadingEdgeAction {
                case .openSidebar:
                    finishGesture(open: shouldOpen)
                case .popToHome, .mainPanelBack, .settingsOverview, .workspaceBotsOverview:
                    resetSidebarDrag()
                    if shouldOpen {
                        hideKeyboard()
                        withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                            onPerformMainPanelLeadingEdgeAction()
                        }
                    }
                }
            }
    }

    private func closingSidebarGesture(sidebarWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .global)
            .updating($sidebarDragLive) { _, state, _ in
                state = true
            }
            .onChanged { value in
                guard shellStore.snapshot.sidebarVisible else { return }
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
                // Mirror of the opening gesture: stay inert while the drawer
                // is closed so the opening gesture's state is untouched.
                guard shellStore.snapshot.sidebarVisible else { return }
                defer {
                    sidebarDragAxis = nil
                }
                guard sidebarDragAxis == .horizontal else {
                    resetSidebarDrag()
                    return
                }
                let shouldClose = -value.translation.width > sidebarWidth * 0.12
                    || -value.predictedEndTranslation.width > sidebarWidth * 0.28
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
        // Opening competes with vertical list scrolling and stays strict;
        // closing an open drawer is an unambiguous intent, so any
        // horizontally-dominant swipe qualifies.
        let ratio = opening ? sidebarAxisDecisionRatio : 1.0
        guard horizontalMag > verticalMag * ratio else {
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
            onSetSidebarVisible(open, false)
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
