import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxRootView: View {
    let model: GaryxMobileModel
    @Environment(GaryxHomeObservationStore.self) private var homeObservationStore
    @Environment(\.layoutDirection) private var inheritedLayoutDirection

    var body: some View {
        ZStack {
            if homeObservationStore.rootSurface == .navigationShell {
                GaryxShellView(
                    model: model,
                    shellStore: model.shellChromeStore,
                    drawerStore: model.navigationDrawerStore,
                    drawerRevealInteraction: model.drawerRevealInteraction,
                    routeStore: model.productionRouteStore,
                    routeNotFoundStore: model.routeNotFoundStore,
                    homeListStore: model.homeThreadListStore,
                    onSetSidebarVisible: { visible, animated in
                        model.setSidebarVisible(visible, animated: animated)
                    },
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
                    onOpenThread: { thread, source in
                        Task { await model.openThread(thread, source: source) }
                    },
                    onPrepareThread: { thread in
                        model.prepareConversationRoute(for: thread)
                    },
                    onTogglePinnedThread: { threadId in
                        model.togglePinnedThread(threadId)
                    },
                    onToggleFavoriteThread: { threadId in
                        model.toggleThreadFavorite(threadId)
                    },
                    onUnpinThread: { threadId in
                        model.unpinThread(threadId)
                    },
                    onBeginPinnedOrderDrag: {
                        model.beginPinnedOrderDrag()
                    },
                    onPreviewPinnedOrderDrag: { order in
                        model.previewPinnedOrderDrag(order)
                    },
                    onAcceptPinnedOrderDrop: {
                        model.acceptPinnedOrderDrop()
                    },
                    onCancelPinnedOrderDrag: {
                        model.cancelPinnedOrderDrag()
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

                // Keep this last in the ZStack so its non-zero-opacity render
                // tree is composited instead of culled. It removes itself
                // after stable delivered frames and never accepts input.
                GaryxConversationRenderPrewarmer()
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
        .environment(\.layoutDirection, resolvedLayoutDirection)
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
        .garyxSheet(
            isPresented: Binding(
                get: { homeObservationStore.showsSettings },
                set: { model.showsSettings = $0 }
            )
        ) {
            GaryxGatewaySetupView(isSheet: true)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
        }
        .environment(
            \.garyxPresentationLeaseCoordinator,
            model.productionRouteStore.presentationCoordinator
        )
    }

    private var resolvedLayoutDirection: LayoutDirection {
        #if DEBUG
        if ProcessInfo.processInfo.environment["GARYX_MOBILE_DEBUG_RTL"] == "1" {
            return .rightToLeft
        }
        #endif
        return inheritedLayoutDirection
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
                                .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
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
                    .buttonStyle(GaryxPressableRowStyle())
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
        .garyxFullScreenCover(isPresented: $showsAddGateway) {
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
                        .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 26, height: 26)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(row.title)
                        .font(GaryxFont.callout(weight: .medium))
                        .foregroundStyle(.primary)
                        .garyxReadingLineLimit()
                    if !row.subtitle.isEmpty {
                        Text(row.subtitle)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .garyxReadingLineLimit()
                            .truncationMode(.middle)
                    }
                }

                Spacer(minLength: 0)
            }
            .padding(.horizontal, 14)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
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
    var leadingIsLeft: Bool

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
            topLeadingRadius: leadingIsLeft ? leadingCornerRadius : 0,
            bottomLeadingRadius: leadingIsLeft ? leadingCornerRadius : 0,
            bottomTrailingRadius: leadingIsLeft ? 0 : leadingCornerRadius,
            topTrailingRadius: leadingIsLeft ? 0 : leadingCornerRadius,
            style: .continuous
        )
        .path(in: expanded)
    }
}

struct GaryxShellView: View, Equatable {
    let model: GaryxMobileModel
    @ObservedObject var shellStore: GaryxShellChromeStore
    @ObservedObject var drawerStore: GaryxNavigationDrawerStore
    @ObservedObject var drawerRevealInteraction: GaryxHorizontalRevealInteractionStore
    @ObservedObject var routeStore: GaryxProductionRouteStore
    @ObservedObject var routeNotFoundStore: GaryxRouteNotFoundStore
    @ObservedObject var homeListStore: GaryxHomeThreadListStore
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.layoutDirection) private var layoutDirection
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.garyxMotion) private var motion

    let onSetSidebarVisible: (Bool, Bool) -> Void
    let onRefreshAll: () async -> Void
    let onRefreshSidebarThreads: () async -> Void
    let onLoadMoreThreads: (GaryxThreadListLoadMoreTrigger) async -> Void
    let onRetryLoadMoreThreads: () async -> Void
    let onSelectRecentFilter: (GaryxRecentThreadFilter) -> Void
    let onStartNewChat: () -> Void
    let onOpenThread: (GaryxThreadSummary, GaryxMobilePanelOpenSource) -> Void
    let onPrepareThread: (GaryxThreadSummary) -> Void
    let onTogglePinnedThread: (String) -> Void
    let onToggleFavoriteThread: (String) -> Void
    let onUnpinThread: (String) -> Void
    let onBeginPinnedOrderDrag: () -> Void
    let onPreviewPinnedOrderDrag: ([String]) -> Void
    let onAcceptPinnedOrderDrop: () -> Void
    let onCancelPinnedOrderDrag: () -> Void
    let onArchiveThread: (GaryxThreadSummary) async -> Void
    let onOpenPanel: (GaryxMobilePanel) -> Void
    let onOpenBotGroup: (GaryxMobileBotGroup) -> Void
    let onOpenBotDrilldown: (String) -> Void
    let onOpenWorkspaceDrilldown: (String) -> Void
    let onOpenSettings: () -> Void
    let onSwitchGateway: (GaryxGatewaySwitcherRow) -> Void
    let onManageGateways: () -> Void
    @Binding var debugShowsGatewaySwitcher: Bool

    @State private var openSwipeActionRowId: String?

    private let sidebarWidth: CGFloat = 330
    private let drawerMainPanelCornerRadius: CGFloat = 36

    static func == (lhs: GaryxShellView, rhs: GaryxShellView) -> Bool {
        lhs.model === rhs.model
            && lhs.shellStore === rhs.shellStore
            && lhs.drawerStore === rhs.drawerStore
            && lhs.drawerRevealInteraction === rhs.drawerRevealInteraction
            && lhs.routeStore === rhs.routeStore
            && lhs.routeNotFoundStore === rhs.routeNotFoundStore
            && lhs.homeListStore === rhs.homeListStore
            && lhs.debugShowsGatewaySwitcher == rhs.debugShowsGatewaySwitcher
    }

    var body: some View {
        GeometryReader { proxy in
            let width = drawerSidebarWidth(for: proxy.size)
            drawerBody(
                width: width,
                containerSize: proxy.size,
                safeAreaInsets: proxy.safeAreaInsets
            )
            .environment(
                \.garyxSidebarDragActive,
                drawerRevealInteraction.presentation.phase != .idle
            )
            .environment(\.garyxOpenSwipeActionRowId, $openSwipeActionRowId)
            .onAppear {
                drawerRevealInteraction.configure(
                    extent: width,
                    restingPosition: shellStore.snapshot.sidebarVisible ? .open : .closed
                )
            }
            .onChange(of: width) { oldWidth, newWidth in
                guard oldWidth != newWidth else { return }
                drawerRevealInteraction.configure(
                    extent: newWidth,
                    restingPosition: shellStore.snapshot.sidebarVisible ? .open : .closed
                )
            }
            .onChange(of: shellStore.snapshot.sidebarVisible) { _, visible in
                drawerRevealInteraction.setTarget(
                    visible ? .open : .closed,
                    animated: animatesTransitions
                )
            }
        }
        .onChange(of: horizontalSizeClass) { _, _ in
            drawerRevealInteraction.setTarget(
                shellStore.snapshot.sidebarVisible ? .open : .closed,
                animated: false
            )
        }
    }

    private func drawerSidebarWidth(for containerSize: CGSize) -> CGFloat {
        // The drawer is navigation-only now; it overlays the home list as a
        // partial sheet on every size class. Accessibility reading sizes need
        // nearly the full canvas so long module names wrap by words instead of
        // being forced into single-character fragments beside the icon column.
        if dynamicTypeSize.isAccessibilitySize {
            return containerSize.width * 0.96
        }
        return min(sidebarWidth, containerSize.width * 0.86)
    }

    private func drawerBody(width: CGFloat, containerSize: CGSize, safeAreaInsets: EdgeInsets) -> some View {
        let revealWidth = sidebarRevealWidth(for: width)
        let drawerProgress = drawerRevealProgress(revealWidth: revealWidth, width: width)
        let leadingIsLeft = layoutDirection == .leftToRight
        let drawerOffset = leadingIsLeft
            ? revealWidth - width
            : width - revealWidth
        let contentOffset = (leadingIsLeft ? 1 : -1) * revealWidth
        // Clip bounds extend through the surrounding safe areas so the panels'
        // full-bleed backgrounds and bottom chrome aprons reach the physical
        // screen edges instead of leaving a plain background band under the
        // notch and home indicator, while panel content keeps native safe-area
        // layout.
        let clipOutsets = EdgeInsets(
            top: safeAreaInsets.top,
            leading: safeAreaInsets.leading,
            bottom: safeAreaInsets.bottom,
            trailing: safeAreaInsets.trailing
        )

        // The public UIKit pan cancels descendant touches when it wins. Keep
        // both surfaces inert for the whole explicit drag/settle phase so a
        // regrab cannot accidentally activate a control under the finger. The
        // freeze is a hit-testing policy, never a disabled appearance.
        let drawerAllowsSurfaceHitTesting =
            drawerRevealInteraction.presentation.phase.allowsSurfaceHitTesting
        let drawerInteractionActive = !drawerAllowsSurfaceHitTesting

        return ZStack(alignment: .topLeading) {
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
            .frame(width: width, height: containerSize.height)
            .contentShape(Rectangle())
            .allowsHitTesting(
                drawerAllowsSurfaceHitTesting && revealWidth > width * 0.82
            )
            .offset(x: drawerOffset)

            GaryxRootNavigationView(
                routeStore: routeStore,
                routeNotFoundStore: routeNotFoundStore,
                homeListStore: homeListStore,
                model: model,
                isSidebarDragActive: drawerInteractionActive,
                onOpenDrawer: {
                    onSetSidebarVisible(true, true)
                },
                onRefreshAll: onRefreshAll,
                onRefreshSidebarThreads: onRefreshSidebarThreads,
                onLoadMoreThreads: onLoadMoreThreads,
                onRetryLoadMoreThreads: onRetryLoadMoreThreads,
                onSelectRecentFilter: onSelectRecentFilter,
                onStartNewChat: onStartNewChat,
                onOpenThread: onOpenThread,
                onPrepareThread: onPrepareThread,
                onTogglePinnedThread: onTogglePinnedThread,
                onToggleFavoriteThread: onToggleFavoriteThread,
                onUnpinThread: onUnpinThread,
                onBeginPinnedOrderDrag: onBeginPinnedOrderDrag,
                onPreviewPinnedOrderDrag: onPreviewPinnedOrderDrag,
                onAcceptPinnedOrderDrop: onAcceptPinnedOrderDrop,
                onCancelPinnedOrderDrag: onCancelPinnedOrderDrag,
                onArchiveThread: onArchiveThread
            )
            .equatable()
            .allowsHitTesting(abs(revealWidth) < 0.5)
            .overlay {
                // While any part of the drawer is revealed, the main panel
                // area becomes one big close target.
                if revealWidth > 1 {
                    Color.clear
                        .contentShape(Rectangle())
                        .onTapGesture { closeSidebar() }
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
                    safeAreaOutsets: clipOutsets,
                    leadingIsLeft: leadingIsLeft
                )
            )
            // A pre-baked gradient strip instead of `.shadow`: animated
            // shadow radii force a full-screen offscreen blur of the main
            // panel every drag frame, which drops drawer frames.
            .overlay(alignment: .leading) {
                LinearGradient(
                    gradient: Gradient(stops: [
                        .init(color: Color.black.opacity(0), location: 0),
                        .init(color: Color.black.opacity(0.04), location: 0.5),
                        .init(color: Color.black.opacity(0.16), location: 1),
                    ]),
                    startPoint: leadingIsLeft ? .leading : .trailing,
                    endPoint: leadingIsLeft ? .trailing : .leading
                )
                .frame(width: 40)
                .padding(.vertical, -safeAreaInsets.top - safeAreaInsets.bottom)
                .offset(x: leadingIsLeft ? -40 : 40)
                .opacity(Double(drawerProgress))
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
            .contentShape(Rectangle())
            .offset(x: contentOffset)
            .allowsHitTesting(drawerAllowsSurfaceHitTesting)
            .zIndex(0)
        }
        .frame(width: containerSize.width, height: containerSize.height, alignment: .topLeading)
        .clipShape(
            GaryxDrawerPanelClipShape(
                leadingCornerRadius: 0,
                safeAreaOutsets: clipOutsets,
                leadingIsLeft: leadingIsLeft
            )
        )
        .garyxPageBackground()
    }

    private func sidebarRevealWidth(for width: CGFloat) -> CGFloat {
        guard drawerRevealInteraction.isGestureEligible else {
            return shellStore.snapshot.sidebarVisible ? width : 0
        }
        return drawerRevealInteraction.reveal
    }

    private func drawerRevealProgress(revealWidth: CGFloat, width: CGFloat) -> CGFloat {
        guard width > 0 else { return 0 }
        return max(0, min(1, revealWidth / width))
    }

    private var animatesTransitions: Bool {
        motion.animatesSpatially(.settle)
    }

    private func closeSidebar() {
        onSetSidebarVisible(false, true)
    }
}
