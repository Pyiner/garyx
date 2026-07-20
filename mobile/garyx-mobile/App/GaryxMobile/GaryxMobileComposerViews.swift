import AVFoundation
import Foundation
import ImageIO
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

private enum GaryxComposerLayout {
    static let composerCornerRadius: CGFloat = 22
    static let composerSpacing: CGFloat = 6
    static let bottomBarSpacing: CGFloat = 12
    static let bottomBarHorizontalPadding: CGFloat = 14
    static let bottomBarTopPadding: CGFloat = 2
    static let bottomBarBottomPadding: CGFloat = 7
    static let actionButtonSide: CGFloat = 36
    static let actionButtonFill = Color.primary.opacity(0.06)
    static let addPanelWidth: CGFloat = 286
    static let addPanelCornerRadius: CGFloat = 28
    static let addPanelRowCornerRadius: CGFloat = 16
    static let addPanelIconSide: CGFloat = 42
    static let inputHorizontalPadding: CGFloat = 16
    static let inputTopPadding: CGFloat = 15
    static let inputBottomPadding: CGFloat = 8
    static let inputMinHeight: CGFloat = 29
    static let composerOcclusionFill = Color(.systemBackground)
    static let composerMaterialTint = Color(.systemBackground).opacity(0.62)
    static let composerMaterialStroke = Color.primary.opacity(0.09)
    static let composerMaterialHighlight = Color.white.opacity(0.82)
    static let composerShadow = Color.black.opacity(0.1)
    static let composerLiftShadow = Color.black.opacity(0.13)
    static let workspaceBaseFill = Color(.systemGray5).opacity(0.58)
    static let workspaceBaseForeground = Color.primary.opacity(0.78)
    static let workspaceBaseStroke = Color.primary.opacity(0.035)
    static let workspaceBaseHighlight = Color.white.opacity(0.3)
    static let workspaceBaseTopShadow = Color.black.opacity(0.035)
    static let workspaceBaseOverlap: CGFloat = composerCornerRadius
    static let workspaceBaseTopPadding: CGFloat = 26
    static let workspaceBaseBottomPadding: CGFloat = 5
    static let workspaceBaseCornerRadius: CGFloat = 16
    static let workspaceBaseTopCornerRadius: CGFloat = 0
    static let workspaceStripHeight: CGFloat = 23
    static let workspaceSheetHeight: CGFloat = 264
    static let workspaceSheetCornerRadius: CGFloat = 34
    static let workspaceSheetTopPadding: CGFloat = 32
    static let workspaceModeRowHeight: CGFloat = 60
    static let workspaceModeRowCornerRadius: CGFloat = 18
    static let workspaceModeRowSpacing: CGFloat = 10
    static let workspaceModeSelectedFill = Color(.systemFill).opacity(0.74)
    static let workspaceModeRowFill = Color(.systemBackground).opacity(0.72)
    static let workspaceModeRowStroke = Color.primary.opacity(0.065)
    static let workspaceModeSelectedStroke = Color.primary.opacity(0.11)
}

private enum GaryxComposerCameraAlert: String, Identifiable {
    case permissionDenied
    case unavailable

    var id: String { rawValue }
}

/// The picker owns this image after capture and Garyx never mutates it. The
/// narrow wrapper makes that immutable handoff explicit so JPEG preparation
/// can happen away from the main actor without stalling the composer.
private struct GaryxCapturedCameraImage: @unchecked Sendable {
    let image: UIImage
}

/// Non-interactive first-frame counterpart of an empty thread composer. It
/// deliberately shares the production composer's geometry, glass recipe, and
/// controls so route preparation never substitutes a visually different card.
struct GaryxConversationOpeningComposerChrome: View {
    var body: some View {
        GaryxAdaptiveGlassContainer(spacing: GaryxComposerLayout.composerSpacing) {
            VStack(spacing: 0) {
                Text("Ask Garyx anything...")
                    .font(GaryxFont.subheadline())
                    .foregroundStyle(Color(.placeholderText))
                    .frame(
                        maxWidth: .infinity,
                        minHeight: GaryxComposerLayout.inputMinHeight,
                        alignment: .topLeading
                    )
                    .padding(.horizontal, GaryxComposerLayout.inputHorizontalPadding)
                    .padding(.top, GaryxComposerLayout.inputTopPadding + 2)
                    .padding(.bottom, GaryxComposerLayout.inputBottomPadding)

                HStack(spacing: GaryxComposerLayout.bottomBarSpacing) {
                    GaryxCircleBadge(
                        systemName: "plus",
                        foreground: .primary,
                        background: GaryxComposerLayout.actionButtonFill,
                        diameter: GaryxComposerLayout.actionButtonSide,
                        iconSize: 20,
                        iconWeight: .regular
                    )
                    .frame(width: 44, height: 44)

                    Spacer(minLength: 0)

                    GaryxCircleBadge(
                        systemName: "arrow.up",
                        foreground: Color(.systemGray2),
                        background: Color(.systemGray5)
                    )
                    .frame(width: 44, height: 44)
                }
                .padding(.horizontal, GaryxComposerLayout.bottomBarHorizontalPadding)
                .padding(.top, GaryxComposerLayout.bottomBarTopPadding)
                .padding(.bottom, GaryxComposerLayout.bottomBarBottomPadding)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                GaryxComposerLayout.composerMaterialTint,
                in: RoundedRectangle(
                    cornerRadius: GaryxComposerLayout.composerCornerRadius,
                    style: .continuous
                )
            )
            .background(
                GaryxComposerLayout.composerOcclusionFill,
                in: RoundedRectangle(
                    cornerRadius: GaryxComposerLayout.composerCornerRadius,
                    style: .continuous
                )
            )
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: false,
                in: RoundedRectangle(
                    cornerRadius: GaryxComposerLayout.composerCornerRadius,
                    style: .continuous
                )
            )
            .overlay {
                RoundedRectangle(
                    cornerRadius: GaryxComposerLayout.composerCornerRadius,
                    style: .continuous
                )
                .stroke(GaryxComposerLayout.composerMaterialHighlight, lineWidth: 0.7)
                .blendMode(.plusLighter)
            }
            .overlay {
                RoundedRectangle(
                    cornerRadius: GaryxComposerLayout.composerCornerRadius,
                    style: .continuous
                )
                .stroke(GaryxComposerLayout.composerMaterialStroke, lineWidth: 0.7)
            }
        }
        .padding(.horizontal, 12)
        .padding(.top, 10)
        .padding(.bottom, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.clear)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Message composer")
        .accessibilityValue("Ask Garyx anything...")
    }
}


struct GaryxComposer: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxRouteContext) private var routeContext
    @Environment(\.garyxMotion) private var motion
    @ObservedObject var payload: GaryxComposerPayloadCoordinator
    let isFocused: FocusState<Bool>.Binding
    @State private var isPickingAttachments = false
    @State private var isPickingPhotos = false
    @State private var selectedPhotoItems: [PhotosPickerItem] = []
    @State private var isPickingCamera = false
    @State private var cameraAlert: GaryxComposerCameraAlert?
    @State private var showsWorkspaceModeSheet = false
    @State private var showsAddPanel = false
    @State private var isAddingAttachments = false

    private var routeProjection: GaryxComposerPayloadProjection? {
        guard let key = routeContext.composerKey else { return payload.snapshot.projection }
        return payload.projection(forRouteKey: key)
    }

    private var routeText: String { routeProjection?.text ?? "" }

    private var routePayloadItems: [GaryxMobileComposerAttachment] {
        (routeProjection?.attachments ?? []).map { attachment in
            GaryxMobileComposerAttachment(
                id: attachment.id.rawValue,
                kind: attachment.kind ?? "file",
                name: attachment.name ?? "attachment",
                mediaType: attachment.mediaType ?? "application/octet-stream",
                path: attachment.uploadedPath ?? "",
                previewDataUrl: attachment.previewDataURL
            )
        }
    }

    private var hasLocalPayload: Bool {
        !routeText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !routePayloadItems.isEmpty
    }

    private var canSendLocalPayload: Bool {
        routeContext.isCanonicalTop
            && !isAddingAttachments
            && payload.canSend
            && model.canSendComposerPayload(
                text: routeText,
                attachments: routePayloadItems
            )
    }

    /// Composer affordances as a pure function of the thread's real run state +
    /// local draft (#TASK-1453 problem A).
    private var composerPresentation: GaryxComposerPresentation {
        GaryxComposerPresentationResolver.resolve(
            isThreadBusy: model.isSelectedThreadSending,
            hasLocalPayload: hasLocalPayload
        )
    }

    private var showsSendButton: Bool {
        composerPresentation.showsSendButton
    }

    private var canChangeWorkspaceMode: Bool {
        model.selectedThread == nil
            && !model.isSending
            && model.activeRunThreadId == nil
            && !model.newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var showsWorkspaceModeStrip: Bool {
        canChangeWorkspaceMode
    }

    var body: some View {
        GaryxAdaptiveGlassContainer(spacing: GaryxComposerLayout.composerSpacing) {
            composerStack
        }
        .padding(.horizontal, 12)
        .padding(.top, 10)
        .padding(.bottom, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.clear)
        .animation(
            motion.spatialAnimation(.composerPayload),
            value: routePayloadItems
        )
        .garyxSheet(isPresented: $showsWorkspaceModeSheet) {
            GaryxComposerWorkspaceModeSheet(
                selectedMode: model.newThreadUsesWorktree ? "worktree" : "local",
                canUseWorktree: model.newThreadWorkspaceCanUseWorktree
            ) { mode in
                model.setNewThreadWorkspaceMode(mode)
            }
            .presentationDetents([.height(GaryxComposerLayout.workspaceSheetHeight)])
            .presentationDragIndicator(.visible)
            .presentationBackground(.ultraThinMaterial)
            .presentationCornerRadius(GaryxComposerLayout.workspaceSheetCornerRadius)
        }
        .garyxResultFileImporter(
            isPresented: $isPickingAttachments,
            allowedContentTypes: [.item],
            allowsMultipleSelection: true
        ) { result, operationContext in
            switch result {
            case .success(let urls):
                Task { @MainActor in
                    isAddingAttachments = true
                    defer { isAddingAttachments = false }
                    await model.attachFiles(
                        from: urls,
                        operationContext: operationContext
                    )
                }
            case .failure(let error):
                model.lastError = error.localizedDescription
            }
        }
        .garyxPhotosPicker(
            isPresented: $isPickingPhotos,
            selection: $selectedPhotoItems,
            maxSelectionCount: 10,
            matching: .images,
            onSelection: { items, operationContext in
                Task { @MainActor in
                    isAddingAttachments = true
                    defer { isAddingAttachments = false }
                    await attachPhotos(items, operationContext: operationContext)
                    selectedPhotoItems = []
                }
            }
        )
        .garyxResultFullScreenCover(isPresented: $isPickingCamera) { resultActions in
            GaryxCameraPicker(isPresented: $isPickingCamera) { image in
                resultActions.recordResult()
                Task { @MainActor in
                    isAddingAttachments = true
                    defer { isAddingAttachments = false }
                    await attachCameraPhoto(
                        image,
                        operationContext: resultActions.operationContext
                    )
                }
            }
            .ignoresSafeArea()
        }
        .garyxAlert(item: $cameraAlert) { alert in
            switch alert {
            case .permissionDenied:
                Alert(
                    title: Text("Camera access is off"),
                    message: Text("Allow camera access in Settings to take a photo for this message."),
                    primaryButton: .default(Text("Open Settings"), action: openAppSettings),
                    secondaryButton: .cancel()
                )
            case .unavailable:
                Alert(
                    title: Text("Camera unavailable"),
                    message: Text("This device does not have a camera available right now."),
                    dismissButton: .default(Text("OK"))
                )
            }
        }
        .onAppear {
            #if DEBUG
            presentDebugWorkspaceModeSheetIfNeeded()
            #endif
        }
        .onChange(of: model.sidebarVisible) { _, visible in
            if visible {
                showsWorkspaceModeSheet = false
                showsAddPanel = false
            }
        }
        .onChange(of: model.selectedThread?.id) { _, threadId in
            if threadId != nil {
                showsWorkspaceModeSheet = false
            }
            showsAddPanel = false
        }
        .onChange(of: model.activePanel) { _, panel in
            if panel != .chat {
                showsWorkspaceModeSheet = false
                showsAddPanel = false
            }
        }
        .onChange(of: showsWorkspaceModeStrip) { _, visible in
            if !visible {
                showsWorkspaceModeSheet = false
            }
        }
        .onChange(of: isAddingAttachments) { _, isAdding in
            if isAdding {
                showsAddPanel = false
            }
        }
        .environment(
            \.garyxPresentationOperationContextProvider,
            GaryxPresentationOperationContextProvider {
                model.makeComposerPresentationOperationContext(
                    payload: payload
                )
            }
        )
        #if DEBUG
        .onChange(of: model.debugShowsWorkspaceModeSheet) { _, _ in
            presentDebugWorkspaceModeSheetIfNeeded()
        }
        #endif
    }

    private var composerStack: some View {
        Group {
            if model.selectedThread == nil {
                newThreadComposerDeck
            } else {
                composerCard
            }
        }
    }

    @ViewBuilder
    private var newThreadComposerDeck: some View {
        if showsWorkspaceModeStrip {
            VStack(spacing: -GaryxComposerLayout.workspaceBaseOverlap) {
                newThreadComposerCard

                workspaceModeStrip
                    .zIndex(0)
            }
        } else {
            newThreadComposerCard
        }
    }

    private var newThreadComposerCard: some View {
        composerCard
            .zIndex(1)
            .shadow(color: GaryxComposerLayout.composerShadow, radius: 22, x: 0, y: 12)
            .shadow(color: GaryxComposerLayout.composerLiftShadow, radius: 12, x: 0, y: 7)
            .shadow(color: Color.black.opacity(0.035), radius: 2, x: 0, y: 1)
    }

    private var composerCard: some View {
        composerCardContent
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(GaryxComposerLayout.composerMaterialTint, in: composerCardShape)
            .background(GaryxComposerLayout.composerOcclusionFill, in: composerCardShape)
            .garyxAdaptiveGlass(.regular, isInteractive: false, in: composerCardShape)
            .overlay {
                composerCardShape
                    .stroke(GaryxComposerLayout.composerMaterialHighlight, lineWidth: 0.7)
                    .blendMode(.plusLighter)
            }
            .overlay {
                composerCardShape
                    .stroke(GaryxComposerLayout.composerMaterialStroke, lineWidth: 0.7)
            }
    }

    private var composerCardShape: RoundedRectangle {
        RoundedRectangle(cornerRadius: GaryxComposerLayout.composerCornerRadius, style: .continuous)
    }

    private var composerCardContent: some View {
        VStack(spacing: 0) {
            if !payload.snapshot.notices.isEmpty {
                GaryxComposerDurableNoticeStack(notices: payload.snapshot.notices)
            }

            if !routePayloadItems.isEmpty {
                composerPayloadItemsPreview
            }

            composerInput
            composerBottomBar
        }
    }

    private var workspaceModeStrip: some View {
        // The gray apron is a non-interactive backdrop; only the leading Local
        // select control (icon + label + chevron) is tappable, not the whole strip.
        HStack(spacing: 0) {
            Button {
                guard canChangeWorkspaceMode else { return }
                showsWorkspaceModeSheet = true
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: workspaceModeIcon)
                        .font(GaryxFont.fixedSystem(size: 14, weight: .semibold))
                        .frame(width: 19, height: 19)

                    Text(workspaceModeTitle)
                        .font(GaryxFont.footnote(weight: .regular))
                        .garyxReadingLineLimit()

                    Image(systemName: "chevron.down")
                        .font(GaryxFont.fixedSystem(size: 10, weight: .semibold))
                }
                .foregroundStyle(GaryxComposerLayout.workspaceBaseForeground)
                .contentShape(Rectangle())
            }
            // The mode label is embedded in the composer's fixed accessory
            // apron; cap it at XXL while the editor remains fully scalable.
            .garyxTypographyBoundary(.composerAccessoryChrome)
            .buttonStyle(GaryxPressableRowStyle())
            .disabled(!canChangeWorkspaceMode)
            .accessibilityLabel("Workspace mode")

            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .frame(maxWidth: .infinity, minHeight: GaryxComposerLayout.workspaceStripHeight, alignment: .leading)
        .padding(.top, GaryxComposerLayout.workspaceBaseTopPadding)
        .padding(.bottom, GaryxComposerLayout.workspaceBaseBottomPadding)
        .background(GaryxComposerLayout.workspaceBaseFill, in: workspaceBaseShape)
        .garyxAdaptiveGlass(.regular, isInteractive: false, in: workspaceBaseShape)
        .overlay {
            workspaceBaseShape
                .stroke(GaryxComposerLayout.workspaceBaseHighlight, lineWidth: 0.6)
                .blendMode(.plusLighter)
        }
        .overlay {
            workspaceBaseShape
                .stroke(GaryxComposerLayout.workspaceBaseStroke, lineWidth: 0.6)
        }
        .overlay(alignment: .top) {
            LinearGradient(
                colors: [
                    GaryxComposerLayout.workspaceBaseTopShadow,
                    Color.clear,
                ],
                startPoint: .top,
                endPoint: .bottom
            )
            .frame(height: 18)
            .clipShape(workspaceBaseShape)
            .allowsHitTesting(false)
        }
        .shadow(color: Color.black.opacity(0.07), radius: 28, x: 0, y: 10)
    }

    private var workspaceBaseShape: UnevenRoundedRectangle {
        UnevenRoundedRectangle(
            topLeadingRadius: GaryxComposerLayout.workspaceBaseTopCornerRadius,
            bottomLeadingRadius: GaryxComposerLayout.workspaceBaseCornerRadius,
            bottomTrailingRadius: GaryxComposerLayout.workspaceBaseCornerRadius,
            topTrailingRadius: GaryxComposerLayout.workspaceBaseTopCornerRadius,
            style: .continuous
        )
    }

    private var workspaceModeTitle: String {
        model.newThreadUsesWorktree ? "WorkTree" : "Local"
    }

    private var workspaceModeIcon: String {
        model.newThreadUsesWorktree ? "arrow.triangle.branch" : "desktopcomputer"
    }

    #if DEBUG
    private func presentDebugWorkspaceModeSheetIfNeeded() {
        guard model.debugShowsWorkspaceModeSheet, showsWorkspaceModeStrip else { return }
        showsWorkspaceModeSheet = true
        model.debugShowsWorkspaceModeSheet = false
    }
    #endif

    private var composerPayloadItemsPreview: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(routePayloadItems) { attachment in
                    GaryxAttachmentChip(attachment: attachment)
                }
            }
            .padding(.horizontal, GaryxComposerLayout.inputHorizontalPadding)
            .padding(.top, 8)
            .padding(.bottom, 4)
        }
    }

    private var composerInput: some View {
        let layout = composerTextLayout

        return ZStack(alignment: .topLeading) {
            if let configuration = payload.inputConfiguration(),
               routeContext.composerKey.map(payload.routeKeyMatchesActiveSession) ?? true {
                GaryxComposerUIKitField(
                    occurrenceID: composerOccurrenceID,
                    configuration: configuration,
                    layout: layout,
                    isFocused: isFocused,
                    onRegister: { adapter in
                        payload.register(
                            adapter,
                            isCanonicalTop: routeContext.isCanonicalTop
                        )
                    },
                    onUnregister: payload.unregister,
                    onOrderedText: payload.acceptText,
                    onProducerTerminal: { producer in
                        payload.producerReachedTerminal(
                            producer,
                            occurrenceID: composerOccurrenceID
                        )
                    },
                    onSubmit: {
                        Task { await sendLocalDraft() }
                    }
                )
                .font(GaryxFont.subheadline())
            } else {
                Color.clear
                    .frame(
                        maxWidth: .infinity,
                        minHeight: layout.minimumControlHeight
                    )
                    .allowsHitTesting(false)
            }

            if routeText.isEmpty {
                Text(placeholderText)
                    .font(GaryxFont.subheadline())
                    .foregroundStyle(Color(.placeholderText))
                    .padding(
                        EdgeInsets(
                            top: layout.textContainerInsets.top + 2,
                            leading: layout.textContainerInsets.left,
                            bottom: layout.textContainerInsets.bottom,
                            trailing: layout.textContainerInsets.right
                        )
                    )
                    .allowsHitTesting(false)
            }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    private var composerTextLayout: GaryxComposerTextLayout {
        GaryxComposerTextLayout(
            textContainerInsets: UIEdgeInsets(
                top: routePayloadItems.isEmpty ? GaryxComposerLayout.inputTopPadding : 6,
                left: GaryxComposerLayout.inputHorizontalPadding,
                bottom: GaryxComposerLayout.inputBottomPadding,
                right: GaryxComposerLayout.inputHorizontalPadding
            ),
            minimumTextHeight: GaryxComposerLayout.inputMinHeight,
            maximumLineCount: 4
        )
    }

    private var composerOccurrenceID: GaryxRouteInstanceID {
        routeContext.occurrenceID
            ?? GaryxRouteInstanceID(rawValue: "transitional-composer-host")
    }

    private var placeholderText: String {
        // The follow-up placeholder is a busy/active-run affordance, not an
        // "is a thread open" one (#TASK-1453 problem A): an idle thread — even
        // one whose tail row is a capsule card — shows the normal prompt.
        switch composerPresentation.placeholder {
        case .prompt:
            return "Ask Garyx anything..."
        case .followUp:
            return "Ask for follow-up changes"
        }
    }

    private var composerBottomBar: some View {
        HStack(spacing: GaryxComposerLayout.bottomBarSpacing) {
            addMenuButton

            Spacer(minLength: 0)

            if composerPresentation.showsStopButton {
                Button {
                    Task { await model.interruptActiveRun() }
                } label: {
                    GaryxCircleBadge(
                        systemName: "stop.fill",
                        foreground: Color(.systemBackground),
                        background: Color(.label)
                    )
                    .frame(width: 44, height: 44)
                }
                .buttonStyle(GaryxPressableRowStyle())
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
                    .frame(width: 44, height: 44)
                }
                .buttonStyle(GaryxPressableRowStyle(prepares: .messageSendCommitted))
                .disabled(!canSendLocalPayload)
                .accessibilityLabel("Send")
            }
        }
        .padding(.horizontal, GaryxComposerLayout.bottomBarHorizontalPadding)
        .padding(.top, GaryxComposerLayout.bottomBarTopPadding)
        .padding(.bottom, GaryxComposerLayout.bottomBarBottomPadding)
    }

    private var addMenuButton: some View {
        Button {
            showsAddPanel.toggle()
        } label: {
            Group {
                if isAddingAttachments {
                    ProgressView()
                        .controlSize(.small)
                        .tint(.primary)
                        .frame(
                            width: GaryxComposerLayout.actionButtonSide,
                            height: GaryxComposerLayout.actionButtonSide
                        )
                        .background(GaryxComposerLayout.actionButtonFill, in: Circle())
                } else {
                    GaryxCircleBadge(
                        systemName: "plus",
                        foreground: .primary,
                        background: GaryxComposerLayout.actionButtonFill,
                        diameter: GaryxComposerLayout.actionButtonSide,
                        iconSize: 20,
                        iconWeight: .regular
                    )
                    .rotationEffect(.degrees(showsAddPanel ? 45 : 0))
                }
            }
            .frame(width: 44, height: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .disabled(isAddingAttachments)
        .accessibilityLabel(isAddingAttachments ? "Adding attachments" : "Add attachment")
        .accessibilityHint("Take a photo, choose photos or files, or insert a saved command")
        .animation(motion.spatialAnimation(.composerPanel), value: showsAddPanel)
        .garyxPopover(
            isPresented: $showsAddPanel,
            attachmentAnchor: .rect(.bounds),
            arrowEdge: .bottom
        ) {
            GaryxComposerAddPopover(
                commands: Array(model.slashCommands.prefix(6)),
                onChooseCamera: presentCameraPicker,
                onChoosePhotos: presentPhotoPicker,
                onChooseFiles: presentFilePicker,
                onChooseCommand: { command in
                    showsAddPanel = false
                    insertSlashCommand(command)
                }
            )
            .presentationCompactAdaptation(.popover)
            .presentationBackground(.ultraThinMaterial)
            .presentationCornerRadius(GaryxComposerLayout.addPanelCornerRadius)
        }
    }

    private func presentCameraPicker() {
        showsAddPanel = false
        guard UIImagePickerController.isSourceTypeAvailable(.camera) else {
            cameraAlert = .unavailable
            return
        }

        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            isPickingCamera = true
        case .notDetermined:
            Task {
                let granted = await AVCaptureDevice.requestAccess(for: .video)
                if granted {
                    isPickingCamera = true
                } else {
                    cameraAlert = .permissionDenied
                }
            }
        case .denied, .restricted:
            cameraAlert = .permissionDenied
        @unknown default:
            cameraAlert = .unavailable
        }
    }

    private func presentPhotoPicker() {
        showsAddPanel = false
        DispatchQueue.main.async {
            isPickingPhotos = true
        }
    }

    private func presentFilePicker() {
        showsAddPanel = false
        DispatchQueue.main.async {
            isPickingAttachments = true
        }
    }

    private func openAppSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }

    private func insertSlashCommand(_ command: GaryxSlashCommand) {
        let normalizedName = command.name.hasPrefix("/") ? command.name : "/\(command.name)"
        payload.replaceLiveText(normalizedName + " ")
        isFocused.wrappedValue = true
    }

    private func sendLocalDraft() async {
        guard canSendLocalPayload else { return }
        _ = await model.sendDraft()
    }

    private func attachPhotos(
        _ items: [PhotosPickerItem],
        operationContext: GaryxPresentationOperationContext?
    ) async {
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
        await model.attachImages(images, operationContext: operationContext)
    }

    private func attachCameraPhoto(
        _ image: UIImage,
        operationContext: GaryxPresentationOperationContext?
    ) async {
        let capture = GaryxCapturedCameraImage(image: image)
        let prepared = await Task.detached(priority: .userInitiated) {
            guard let data = capture.image.jpegData(compressionQuality: 0.92) else {
                return nil as GaryxMobileSelectedImage?
            }
            return Self.preparedPhotoUpload(
                data: data,
                index: 0,
                mediaType: "image/jpeg",
                fileExtension: "jpg"
            )
        }.value

        guard let prepared else {
            model.lastError = "That photo is too large to prepare for upload."
            return
        }
        await model.attachImages([prepared], operationContext: operationContext)
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

private struct GaryxCameraPicker: UIViewControllerRepresentable {
    @Binding var isPresented: Bool
    let onCapture: (UIImage) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.cameraCaptureMode = .photo
        picker.allowsEditing = false
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}

    final class Coordinator: NSObject, UINavigationControllerDelegate, UIImagePickerControllerDelegate {
        private let parent: GaryxCameraPicker

        init(parent: GaryxCameraPicker) {
            self.parent = parent
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            parent.isPresented = false
        }

        func imagePickerController(
            _ picker: UIImagePickerController,
            didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
        ) {
            parent.isPresented = false
            guard let image = info[.originalImage] as? UIImage else { return }
            parent.onCapture(image)
        }
    }
}

private struct GaryxComposerAddPopover: View {
    private enum Page {
        case root
        case commands
    }

    let commands: [GaryxSlashCommand]
    let onChooseCamera: () -> Void
    let onChoosePhotos: () -> Void
    let onChooseFiles: () -> Void
    let onChooseCommand: (GaryxSlashCommand) -> Void
    @Environment(\.garyxMotion) private var motion
    @State private var page: Page = .root

    var body: some View {
        Group {
            switch page {
            case .root:
                rootActions
            case .commands:
                commandActions
            }
        }
        .frame(width: GaryxComposerLayout.addPanelWidth)
        .padding(8)
        .animation(motion.spatialAnimation(.composerDrilldown), value: page)
    }

    private var rootActions: some View {
        VStack(spacing: 2) {
            actionRow(
                title: "Camera",
                subtitle: "Take a new photo",
                systemName: "camera",
                action: onChooseCamera
            )
            actionRow(
                title: "Photos",
                subtitle: "Choose up to 10 images",
                systemName: "photo.on.rectangle.angled",
                action: onChoosePhotos
            )
            actionRow(
                title: "Files",
                subtitle: "Documents, images, and more",
                systemName: "paperclip",
                action: onChooseFiles
            )
            if !commands.isEmpty {
                actionRow(
                    title: "Commands",
                    subtitle: "Insert a saved prompt",
                    systemName: "command",
                    trailingText: "\(commands.count)",
                    showsChevron: true
                ) {
                    page = .commands
                }
            }
        }
    }

    private var commandActions: some View {
        VStack(spacing: 2) {
            HStack(spacing: 8) {
                Button {
                    page = .root
                } label: {
                    Image(systemName: "chevron.left")
                        .font(GaryxFont.fixedSystem(size: 14, weight: .semibold))
                        .foregroundStyle(.primary)
                        .frame(width: 36, height: 36)
                        .background(Color.primary.opacity(0.055), in: Circle())
                }
                .buttonStyle(GaryxPressableRowStyle())
                .accessibilityLabel("Back to attachments")

                Text("Commands")
                    .font(GaryxFont.callout(weight: .semibold))
                    .foregroundStyle(.primary)

                Spacer(minLength: 0)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)

            ScrollView(showsIndicators: false) {
                VStack(spacing: 2) {
                    ForEach(commands) { command in
                        actionRow(
                            title: commandTitle(command),
                            subtitle: command.description.isEmpty ? "Insert command" : command.description,
                            systemName: "command"
                        ) {
                            onChooseCommand(command)
                        }
                    }
                }
            }
            .frame(maxHeight: 310)
        }
    }

    private func actionRow(
        title: String,
        subtitle: String,
        systemName: String,
        trailingText: String? = nil,
        showsChevron: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 13) {
                Image(systemName: systemName)
                    .font(GaryxFont.fixedSystem(size: 18, weight: .medium))
                    .symbolRenderingMode(.monochrome)
                    .foregroundStyle(.primary)
                    .frame(
                        width: GaryxComposerLayout.addPanelIconSide,
                        height: GaryxComposerLayout.addPanelIconSide
                    )
                    .background(Color.primary.opacity(0.055), in: Circle())

                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(GaryxFont.callout(weight: .semibold))
                        .foregroundStyle(.primary)
                        .garyxReadingLineLimit()
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .garyxReadingLineLimit()
                }

                Spacer(minLength: 8)

                if let trailingText {
                    Text(trailingText)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 7)
                        .frame(minHeight: 24)
                        .background(Color.primary.opacity(0.05), in: Capsule())
                }
                if showsChevron {
                    Image(systemName: "chevron.right")
                        .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .frame(minHeight: 58)
            .contentShape(
                RoundedRectangle(
                    cornerRadius: GaryxComposerLayout.addPanelRowCornerRadius,
                    style: .continuous
                )
            )
        }
        .buttonStyle(GaryxComposerAddPanelRowButtonStyle())
        .accessibilityElement(children: .combine)
    }

    private func commandTitle(_ command: GaryxSlashCommand) -> String {
        command.name.hasPrefix("/") ? command.name : "/\(command.name)"
    }
}

private struct GaryxComposerAddPanelRowButtonStyle: ButtonStyle {
    @Environment(\.garyxMotion) private var motion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(
                Color.primary.opacity(configuration.isPressed ? 0.075 : 0),
                in: RoundedRectangle(
                    cornerRadius: GaryxComposerLayout.addPanelRowCornerRadius,
                    style: .continuous
                )
            )
            .scaleEffect(motion.scale(.subtlePress, active: configuration.isPressed))
            .animation(motion.animation(.subtlePress), value: configuration.isPressed)
    }
}

private struct GaryxComposerWorkspaceModeSheet: View {
    @Environment(\.dismiss) private var dismiss
    let selectedMode: String
    let canUseWorktree: Bool
    let onSelect: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Run Location")
                .font(GaryxFont.title3(weight: .semibold))
                .foregroundStyle(.primary)
                .frame(maxWidth: .infinity, alignment: .leading)

            VStack(spacing: GaryxComposerLayout.workspaceModeRowSpacing) {
                modeRow(
                    mode: "local",
                    title: "Local",
                    systemImage: "desktopcomputer",
                    isEnabled: true
                )

                modeRow(
                    mode: "worktree",
                    title: "WorkTree",
                    systemImage: "arrow.triangle.branch",
                    isEnabled: canUseWorktree
                )
            }
        }
        .padding(.horizontal, 18)
        .padding(.top, GaryxComposerLayout.workspaceSheetTopPadding)
        .padding(.bottom, 18)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(.systemBackground).opacity(0.18))
    }

    private func modeRow(
        mode: String,
        title: String,
        systemImage: String,
        isEnabled: Bool
    ) -> some View {
        let selected = selectedMode == mode
        return Button {
            guard isEnabled else { return }
            onSelect(mode)
            dismiss()
        } label: {
            HStack(spacing: 12) {
                Image(systemName: systemImage)
                    .font(GaryxFont.fixedSystem(size: 16, weight: .semibold))
                    .foregroundStyle(.primary)
                    .frame(width: 34, height: 34)
                    .background(GaryxComposerLayout.actionButtonFill, in: Circle())

                Text(title)
                    .font(GaryxFont.callout(weight: .semibold))
                    .foregroundStyle(.primary)

                Spacer(minLength: 0)

                if selected {
                    Image(systemName: "checkmark.circle.fill")
                        .font(GaryxFont.fixedSystem(size: 18, weight: .semibold))
                        .foregroundStyle(.primary)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .frame(minHeight: GaryxComposerLayout.workspaceModeRowHeight)
            .background(
                selected ? GaryxComposerLayout.workspaceModeSelectedFill : GaryxComposerLayout.workspaceModeRowFill,
                in: workspaceModeRowShape
            )
            .background(.ultraThinMaterial, in: workspaceModeRowShape)
            .overlay {
                workspaceModeRowShape
                    .stroke(
                        selected ? GaryxComposerLayout.workspaceModeSelectedStroke : GaryxComposerLayout.workspaceModeRowStroke,
                        lineWidth: 1
                    )
            }
            .overlay {
                workspaceModeRowShape
                    .stroke(Color.white.opacity(0.72), lineWidth: 0.6)
                    .blendMode(.plusLighter)
            }
            .opacity(isEnabled ? 1 : 0.42)
            .contentShape(workspaceModeRowShape)
        }
        .buttonStyle(GaryxPressableRowStyle())
        .disabled(!isEnabled)
    }

    private var workspaceModeRowShape: RoundedRectangle {
        RoundedRectangle(cornerRadius: GaryxComposerLayout.workspaceModeRowCornerRadius, style: .continuous)
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
                model.removeComposerPayloadItem(attachment)
            } label: {
                Image(systemName: "xmark")
                    .font(GaryxFont.fixedSystem(size: 9, weight: .bold))
                    .foregroundStyle(Color.white)
                    .padding(4)
                    .background(Color.black.opacity(0.65), in: Circle())
            }
            .buttonStyle(GaryxPressableRowStyle())
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
                .garyxReadingLineLimit()
            Button {
                model.removeComposerPayloadItem(attachment)
            } label: {
                Image(systemName: "xmark")
                    .font(GaryxFont.caption(weight: .bold))
            }
            .buttonStyle(GaryxPressableRowStyle())
            .accessibilityLabel("Remove attachment")
        }
        .foregroundStyle(.primary)
        .padding(.horizontal, 10)
        .frame(height: 30)
        .background(Color(.tertiarySystemFill), in: Capsule())
        // File chips share the composer's fixed 30-point attachment tray;
        // XXL keeps the tray stable and the filename legible.
        .garyxTypographyBoundary(.composerAccessoryChrome)
    }

    private var decodedThumbnail: UIImage? {
        GaryxDataURLImageCache.image(from: attachment.previewDataUrl)
    }
}
