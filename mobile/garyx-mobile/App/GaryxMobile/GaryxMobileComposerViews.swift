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
    static let actionButtonSide: CGFloat = 32
    static let actionButtonFill = Color.primary.opacity(0.06)
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
    @State private var showsWorkspaceModeSheet = false

    private var hasLocalPayload: Bool {
        !draftText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !model.composerAttachments.isEmpty
    }

    private var canSendLocalPayload: Bool {
        model.canSendComposerPayload(text: draftText, attachments: model.composerAttachments)
    }

    private var showsSendButton: Bool {
        !model.isSelectedThreadVisiblyRunning || hasLocalPayload
    }

    private var canChangeWorkspaceMode: Bool {
        model.selectedThread == nil
            && !model.isSending
            && model.activeRunThreadId == nil
            && !model.newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var showsWorkspaceModeStrip: Bool {
        canChangeWorkspaceMode && model.newThreadWorkspaceCanUseWorktree
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
        .animation(.spring(response: 0.24, dampingFraction: 0.88), value: model.composerAttachments)
        .sheet(isPresented: $showsWorkspaceModeSheet) {
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
            #if DEBUG
            presentDebugWorkspaceModeSheetIfNeeded()
            #endif
        }
        .onChange(of: model.composerContextVersion) { _, newValue in
            draftContextVersion = newValue
            draftText = model.draft
        }
        .onChange(of: model.draft) { _, newValue in
            guard newValue != draftText else { return }
            draftText = newValue
        }
        .onChange(of: model.sidebarVisible) { _, visible in
            if visible {
                showsWorkspaceModeSheet = false
            }
        }
        .onChange(of: model.selectedThread?.id) { _, threadId in
            if threadId != nil {
                showsWorkspaceModeSheet = false
            }
        }
        .onChange(of: model.activePanel) { _, panel in
            if panel != .chat {
                showsWorkspaceModeSheet = false
            }
        }
        .onChange(of: showsWorkspaceModeStrip) { _, visible in
            if !visible {
                showsWorkspaceModeSheet = false
            }
        }
        #if DEBUG
        .onChange(of: model.debugShowsWorkspaceModeSheet) { _, _ in
            presentDebugWorkspaceModeSheetIfNeeded()
        }
        #endif
        .onDisappear {
            guard draftContextVersion == model.composerContextVersion else { return }
            if model.draft != draftText {
                model.draft = draftText
            }
        }
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
            .garyxAdaptiveGlass(.regular, isInteractive: false, fallbackMaterial: .ultraThinMaterial, in: composerCardShape)
            .overlay {
                composerCardShape
                    .stroke(GaryxComposerLayout.composerMaterialHighlight, lineWidth: 0.7)
                    .blendMode(.plusLighter)
                    .mask(composerTopEdgeMask)
            }
            .overlay {
                composerCardShape
                    .stroke(GaryxComposerLayout.composerMaterialStroke, lineWidth: 0.7)
                    .mask(composerTopEdgeMask)
            }
    }

    private var composerCardShape: RoundedRectangle {
        RoundedRectangle(cornerRadius: GaryxComposerLayout.composerCornerRadius, style: .continuous)
    }

    private var composerTopEdgeMask: some View {
        LinearGradient(
            stops: [
                .init(color: .white, location: 0),
                .init(color: .white, location: 0.45),
                .init(color: .clear, location: 0.82),
            ],
            startPoint: .top,
            endPoint: .bottom
        )
    }

    private var composerCardContent: some View {
        VStack(spacing: 0) {
            if !model.composerAttachments.isEmpty {
                composerAttachmentsPreview
            }

            composerInput
            composerBottomBar
        }
    }

    private var workspaceModeStrip: some View {
        Button {
            guard canChangeWorkspaceMode else { return }
            showsWorkspaceModeSheet = true
        } label: {
            HStack(spacing: 6) {
                Image(systemName: workspaceModeIcon)
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .frame(width: 19, height: 19)

                Text(workspaceModeTitle)
                    .font(GaryxFont.footnote(weight: .regular))
                    .lineLimit(1)

                Image(systemName: "chevron.down")
                    .font(GaryxFont.system(size: 10, weight: .semibold))

                Spacer(minLength: 0)
            }
            .foregroundStyle(GaryxComposerLayout.workspaceBaseForeground)
            .padding(.horizontal, 14)
            .frame(maxWidth: .infinity, minHeight: GaryxComposerLayout.workspaceStripHeight, alignment: .leading)
            .padding(.top, GaryxComposerLayout.workspaceBaseTopPadding)
            .padding(.bottom, GaryxComposerLayout.workspaceBaseBottomPadding)
            .background(GaryxComposerLayout.workspaceBaseFill, in: workspaceBaseShape)
            .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .ultraThinMaterial, in: workspaceBaseShape)
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
        .buttonStyle(.plain)
        .disabled(!canChangeWorkspaceMode)
        .accessibilityLabel("Workspace mode")
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
        .frame(maxWidth: .infinity, minHeight: GaryxComposerLayout.inputMinHeight, alignment: .topLeading)
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

            if model.isSelectedThreadVisiblyRunning {
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

        } label: {
            GaryxCircleBadge(
                systemName: "plus",
                foreground: .primary,
                background: GaryxComposerLayout.actionButtonFill,
                diameter: GaryxComposerLayout.actionButtonSide,
                iconSize: 20,
                iconWeight: .regular
            )
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
                    .font(GaryxFont.system(size: 16, weight: .semibold))
                    .foregroundStyle(.primary)
                    .frame(width: 34, height: 34)
                    .background(GaryxComposerLayout.actionButtonFill, in: Circle())

                Text(title)
                    .font(GaryxFont.callout(weight: .semibold))
                    .foregroundStyle(.primary)

                Spacer(minLength: 0)

                if selected {
                    Image(systemName: "checkmark.circle.fill")
                        .font(GaryxFont.system(size: 18, weight: .semibold))
                        .foregroundStyle(.primary)
                }
            }
            .padding(.horizontal, 12)
            .frame(height: GaryxComposerLayout.workspaceModeRowHeight)
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
        .buttonStyle(.plain)
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
