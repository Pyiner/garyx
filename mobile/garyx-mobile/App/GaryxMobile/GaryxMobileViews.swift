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
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        ZStack {
            if model.hasGatewaySettings, case .ready = model.connectionState {
                GaryxShellView()
            } else {
                GaryxGatewaySetupView()
            }
        }
        .garyxPageBackground()
        .garyxRootChromeBackground()
        .overlay(alignment: .top) {
            GaryxGlobalErrorToastHost(topOffset: 72)
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
                    GaryxGlobalErrorToastHost(topOffset: 8)
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
            VStack(alignment: .leading, spacing: 22) {
                if let failureMessage {
                    GaryxFormErrorText(text: failureMessage)
                }

                GaryxFormGroupedSection(title: "Connection") {
                    GaryxFormTextFieldRow(
                        title: "Gateway URL",
                        text: $draftGatewayURL,
                        keyboardType: .URL,
                        textContentType: .URL,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    Divider().padding(.leading, 16)
                    GaryxFormSecureFieldRow(
                        title: "Gateway Token",
                        text: $draftGatewayAuthToken,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                }
            }
        }
        .onAppear(perform: initializeDraft)
        .overlay(alignment: .top) {
            GaryxGlobalErrorToastHost(topOffset: 8)
        }
    }

    private var connectingBody: some View {
        VStack {
            Spacer()
            GaryxIonicLoader(fontSize: 88)
                .padding(.horizontal, 24)
            Spacer()
        }
    }

    private var setupForm: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 24)

            VStack(spacing: 24) {
                GaryxIonicLoader(fontSize: 72, isAnimating: setupIsBusy)

                if let failureMessage {
                    Text(failureMessage)
                        .font(GaryxFont.callout(weight: .medium))
                        .foregroundStyle(Color.orange)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: 300)
                } else {
                    Text("Set the gateway address and token, then save. Saving verifies the gateway before continuing.")
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: 300)
                }

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
                        await saveGatewaySettings()
                    }
                }
                .disabled(!canSaveGateway || setupIsBusy)
                .opacity(canSaveGateway && !setupIsBusy ? 1 : 0.45)
            }
            .frame(maxWidth: 320)
            .padding(.horizontal, 24)

            Spacer(minLength: 24)
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
        draftGatewayURL = startsEmpty ? "" : model.gatewayURL
        draftGatewayAuthToken = startsEmpty ? "" : model.gatewayAuthToken
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

struct GaryxShellView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var sidebarDragOffset: CGFloat = 0
    @State private var sidebarDragAxis: GaryxSidebarDragAxis?

    private let sidebarWidth: CGFloat = 330
    private let drawerMainPanelCornerRadius: CGFloat = 36
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
            // Final open state is a full-width sidebar (full-screen page swap).
            // The rounded card / shadow / divider effects are intentionally only
            // visible mid-drag as a transition, driven by drawerProgress.
            return containerSize.width
        }
        return min(sidebarWidth, containerSize.width * 0.92)
    }

    private func drawerBody(width: CGFloat, containerSize: CGSize) -> some View {
        let revealWidth = sidebarRevealWidth(for: width)
        let drawerProgress = drawerRevealProgress(revealWidth: revealWidth, width: width)
        let drawerOffset = revealWidth - width
        let closeCaptureWidth = max(0, containerSize.width - revealWidth)

        return ZStack(alignment: .topLeading) {
            HStack(spacing: 0) {
                GaryxThreadSidebar(showsInlineCloseButton: true)
                    .frame(width: width)
                    .frame(maxHeight: .infinity)
                    .contentShape(Rectangle())
                    .allowsHitTesting(revealWidth > width * 0.82)
                    .simultaneousGesture(closingSidebarGesture(sidebarWidth: width))

                GaryxMainPanelView()
                    .frame(width: containerSize.width, height: containerSize.height)
                    .modifier(
                        GaryxDrawerMainPanelStyle(
                            progress: drawerProgress,
                            cornerRadius: drawerMainPanelCornerRadius
                        )
                    )
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

            if revealWidth > 1, closeCaptureWidth > 0 {
                Color.clear
                    .frame(width: closeCaptureWidth, height: containerSize.height)
                    .offset(x: revealWidth)
                    .contentShape(Rectangle())
                    .onTapGesture { closeSidebar() }
                    .simultaneousGesture(closingSidebarGesture(sidebarWidth: width))
                    .zIndex(1)
                    .accessibilityHidden(true)
            }
        }
        .frame(width: containerSize.width, height: containerSize.height, alignment: .topLeading)
        .clipped()
        .background(GaryxTheme.background)
    }

    private func drawerRevealProgress(revealWidth: CGFloat, width: CGFloat) -> CGFloat {
        guard width > 0 else { return 0 }
        return max(0, min(1, revealWidth / width))
    }

    private func sidebarRevealWidth(for width: CGFloat) -> CGFloat {
        if model.sidebarVisible {
            return max(0, min(width, width + sidebarDragOffset))
        }
        return max(0, min(width, sidebarDragOffset))
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
                case .mainPanelBack, .settingsOverview, .workspaceBotsOverview:
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
                case .mainPanelBack, .settingsOverview, .workspaceBotsOverview:
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

private struct GaryxDrawerMainPanelStyle: ViewModifier {
    let progress: CGFloat
    let cornerRadius: CGFloat

    func body(content: Content) -> some View {
        let clampedProgress = max(0, min(1, progress))
        let resolvedCornerRadius = cornerRadius * clampedProgress
        let shape = UnevenRoundedRectangle(
            topLeadingRadius: resolvedCornerRadius,
            bottomLeadingRadius: resolvedCornerRadius,
            bottomTrailingRadius: 0,
            topTrailingRadius: 0,
            style: .continuous
        )

        content
            .background(GaryxTheme.background)
            .overlay(alignment: .leading) {
                Rectangle()
                    .fill(Color.primary.opacity(0.10))
                    .frame(width: 1 / UIScreen.main.scale)
                    .opacity(clampedProgress)
                    .allowsHitTesting(false)
            }
            .clipShape(shape)
            .shadow(
                color: Color.black.opacity(0.18 * Double(clampedProgress)),
                radius: 30 * clampedProgress,
                x: -10 * clampedProgress,
                y: 0
            )
            .shadow(
                color: Color.black.opacity(0.06 * Double(clampedProgress)),
                radius: 10 * clampedProgress,
                x: -3 * clampedProgress,
                y: 0
            )
    }
}
