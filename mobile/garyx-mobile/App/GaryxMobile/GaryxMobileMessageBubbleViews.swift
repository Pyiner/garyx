import Foundation
import SwiftUI
import UIKit

/// The single owner of user-row alignment and Dynamic Type width policy.
/// Task cards and ordinary user bubbles must both pass through this container.
struct GaryxUserRoleMessageContainer<Content: View>: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    private let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        GaryxUserRoleRowLayout(
            maximumContentWidth: UIScreen.main.bounds.width
                * (dynamicTypeSize.garyxUsesExpandedReadingLayout ? 0.94 : 0.77),
            minimumLeadingSpacing: dynamicTypeSize.garyxUsesExpandedReadingLayout ? 12 : 60
        ) {
            content
        }
    }
}

/// Proposes the shared width cap to the row content before it is measured.
/// A flexible card therefore fills the cap without drawing past it, while an
/// intrinsically narrower ordinary bubble keeps its natural width.
private struct GaryxUserRoleRowLayout: Layout {
    let maximumContentWidth: CGFloat
    let minimumLeadingSpacing: CGFloat

    func sizeThatFits(
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) -> CGSize {
        guard let content = subviews.first else { return .zero }
        let rowWidth = proposal.width ?? (maximumContentWidth + minimumLeadingSpacing)
        let contentWidth = min(maximumContentWidth, max(0, rowWidth - minimumLeadingSpacing))
        let contentSize = content.sizeThatFits(
            ProposedViewSize(width: contentWidth, height: proposal.height)
        )
        return CGSize(width: rowWidth, height: contentSize.height)
    }

    func placeSubviews(
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) {
        guard let content = subviews.first else { return }
        let contentLimit = min(
            maximumContentWidth,
            max(0, bounds.width - minimumLeadingSpacing)
        )
        let contentSize = content.sizeThatFits(
            ProposedViewSize(width: contentLimit, height: proposal.height)
        )
        content.place(
            at: CGPoint(x: bounds.maxX - contentSize.width, y: bounds.minY),
            anchor: .topLeading,
            proposal: ProposedViewSize(width: contentLimit, height: proposal.height)
        )
    }
}

struct GaryxMessageBubble: View {
    let message: GaryxMobileMessage
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.garyxMessageBubbleActions) private var actions
    @Environment(\.garyxMotion) private var motion
    @ScaledMetric(relativeTo: .body) private var userBubbleVerticalPadding: CGFloat = 8
    @ScaledMetric(relativeTo: .body) private var messageSpacing: CGFloat = 8
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
        .garyxFullScreenCover(item: $filePreviewSheet) { sheet in
            GaryxFullscreenWorkspaceFilePreview(preview: sheet.preview) {
                filePreviewSheet = nil
            }
            .garyxOptionalEnvironmentObject(actions.model)
        }
    }

    @ViewBuilder
    private var messageRow: some View {
        switch messagePresentation {
        case let .taskNotification(_, notification):
            taskNotificationRow(notification)
        default:
            roleMessageRow
        }
    }

    @ViewBuilder
    private func taskNotificationRow(_ notification: GaryxTaskNotification) -> some View {
        GaryxUserRoleMessageContainer {
            VStack(alignment: .trailing, spacing: messageSpacing / 2) {
                if !message.attachments.isEmpty {
                    GaryxMessageAttachmentStack(attachments: message.attachments, isUser: true)
                        .garyxMessageCopyContext(text: messageCopyText, edge: .trailing)
                }
                GaryxTaskNotificationCard(
                    notification: notification,
                    onExpand: {
                        actions.selectTaskNotification(
                            GaryxTaskNotificationSelection(
                                messageId: message.id,
                                messageSeq: message.historyIndex.map { $0 + 1 },
                                notification: notification
                            )
                        )
                    },
                    onFileLinkTap: openMessageFileLink,
                    onImageFilePreview: messageImageFilePreview
                )
                    .garyxMessageInteraction(
                        text: taskNotificationCopyText(notification),
                        edge: .trailing
                    )
            }
        }
    }

    @ViewBuilder
    private var roleMessageRow: some View {
        switch message.role {
        case .user:
            GaryxUserRoleMessageContainer {
                VStack(alignment: .trailing, spacing: messageSpacing / 2) {
                    if !message.attachments.isEmpty {
                        GaryxMessageAttachmentStack(attachments: message.attachments, isUser: true)
                            .garyxMessageCopyContext(text: messageCopyText, edge: .trailing)
                    }

                    if let restart = restartNotice {
                        GaryxRestartNoticeCard(notice: restart)
                            .garyxMessageInteraction(text: restart.message, edge: .trailing)
                    } else if messagePresentation == .historySkeleton {
                        GaryxUserMessageLoadingBubble()
                    } else if !displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        GaryxMarkdownText(
                            text: displayText,
                            foreground: .primary,
                            codeBackground: userCodeBackground,
                            codeBorder: GaryxTheme.hairline,
                            fillsAvailableWidth: false,
                            allowsRelativeFileLinks: true,
                            allowsTextSelection: false,
                            onFileLinkTap: openMessageFileLink,
                            onImageFilePreview: messageImageFilePreview
                        )
                        .padding(.horizontal, 12)
                        .padding(.vertical, userBubbleVerticalPadding)
                        .background(userBubbleBackground, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                        .garyxMessageInteraction(text: displayText, edge: .trailing)
                    }

                    if let statusText = message.statusText, !statusText.isEmpty {
                        failureStatusRow(statusText: statusText)
                    }
                }
            }
        case .assistant:
            VStack(alignment: .leading, spacing: messageSpacing) {
                if !message.attachments.isEmpty {
                    GaryxMessageAttachmentStack(attachments: message.attachments, isUser: false)
                        .garyxMessageCopyContext(text: messageCopyText)
                }
                if message.isStreaming && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    if case .thinkingLabel(let text) = messagePresentation {
                        GaryxThinkingLabel(text: text)
                    }
                } else if let restart = restartNotice {
                    GaryxRestartNoticeCard(notice: restart)
                        .garyxMessageInteraction(text: restart.message)
                } else if !displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    GaryxMarkdownText(
                        text: displayText,
                        foreground: .primary,
                        allowsRelativeFileLinks: true,
                        allowsTextSelection: false,
                        onFileLinkTap: openMessageFileLink,
                        onImageFilePreview: messageImageFilePreview
                    )
                    .garyxMessageInteraction(text: displayText)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // Smooth the height growth while this bubble streams. Settled
            // bubbles compare their (storage-shared) text in O(1) and never
            // animate, so long transcripts pay nothing.
            .animation(
                message.isStreaming ? motion.spatialAnimation(.streamingResize) : nil,
                value: message.text
            )
        case .system:
            GaryxMarkdownText(
                text: displayText,
                foreground: .secondary,
                fillsAvailableWidth: false,
                allowsRelativeFileLinks: true,
                allowsTextSelection: false,
                onFileLinkTap: openMessageFileLink,
                onImageFilePreview: messageImageFilePreview
            )
                .font(GaryxFont.footnote())
                .padding(.horizontal, 10)
                .padding(.vertical, userBubbleVerticalPadding)
                .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .stroke(GaryxTheme.hairline, style: StrokeStyle(lineWidth: 1, dash: [4, 4]))
                }
                .frame(maxWidth: 720, alignment: .center)
                .frame(maxWidth: .infinity, alignment: .center)
                .garyxMessageInteraction(text: displayText)
        case .tool:
            EmptyView()
        }
    }

    private var messagePresentation: GaryxMobileMessagePresentation {
        GaryxMobileMessagePresentation.make(for: message)
    }

    private var displayText: String {
        messagePresentation.text
    }

    private var restartNotice: GaryxRestartNotice? {
        guard !message.isStreaming else { return nil }
        return GaryxRestartNoticePresentation.parse(displayText)
    }

    private func taskNotificationCopyText(_ notification: GaryxTaskNotification) -> String {
        [
            notification.taskId,
            notification.title,
            notification.finalMessage,
        ]
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: "\n\n")
    }

    private var messageCopyText: String {
        var parts: [String] = []
        if !displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            parts.append(displayText)
        }
        let attachmentText = message.attachments
            .compactMap(Self.copyTextLine(for:))
            .joined(separator: "\n")
        if !attachmentText.isEmpty {
            parts.append(attachmentText)
        }
        return parts.joined(separator: "\n\n")
    }

    private static func copyTextLine(for attachment: GaryxMobileMessageAttachment) -> String? {
        let title = attachment.name.trimmingCharacters(in: .whitespacesAndNewlines)
        let fallback = attachment.isImage ? "Image" : "Attachment"
        let label = title.isEmpty ? fallback : title
        if let path = attachment.path?.trimmingCharacters(in: .whitespacesAndNewlines),
           !path.isEmpty {
            return "\(label): \(path)"
        }
        if let remoteUrl = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
           !remoteUrl.isEmpty {
            return "\(label): \(remoteUrl)"
        }
        if attachment.dataUrl?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false {
            return "\(label): inline \(attachment.isImage ? "image" : "attachment")"
        }
        return title.isEmpty ? nil : label
    }

    private var userBubbleBackground: Color {
        (colorScheme == .dark ? Color.white.opacity(0.12) : Color.black.opacity(0.05))
    }

    private var userCodeBackground: Color {
        colorScheme == .dark ? Color.white.opacity(0.08) : Color.black.opacity(0.055)
    }

    private func openMessageFileLink(_ target: String) {
        Task {
            guard let preview = await actions.localFilePreview(target, true) else { return }
            filePreviewSheet = GaryxMessageFilePreviewSheet(preview: preview)
        }
    }

    @MainActor
    private func messageImageFilePreview(_ target: String) async -> GaryxWorkspaceFilePreview? {
        await actions.localFilePreview(target, false)
    }

    @ViewBuilder
    private func failureStatusRow(statusText: String) -> some View {
        let canRetry = message.localState != nil
            && message.localState != .remoteFinal
        if canRetry {
            Button {
                guard !retrying else { return }
                retrying = true
                Task {
                    _ = await actions.retryFailedUserMessage(message.id)
                    retrying = false
                }
            } label: {
                HStack(spacing: 6) {
                    if retrying {
                        ProgressView()
                            .controlSize(.mini)
                    } else {
                        Image(systemName: "arrow.clockwise")
                            .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                    }
                    Text(retrying ? "Retrying…" : statusText)
                        .font(GaryxFont.caption())
                        .garyxReadingLineLimit(2)
                        .multilineTextAlignment(.trailing)
                }
                .foregroundStyle(Color(.systemRed))
            }
            .buttonStyle(GaryxPressableRowStyle(prepares: .messageSendCommitted))
            .disabled(retrying)
            .accessibilityLabel(Text("Retry message"))
            .accessibilityHint(Text(statusText))
        } else {
            Text(statusText)
                .font(GaryxFont.caption())
                .foregroundStyle(Color(.systemRed))
                .garyxReadingLineLimit(2)
                .multilineTextAlignment(.trailing)
        }
    }
}

private extension View {
    @ViewBuilder
    func garyxOptionalEnvironmentObject<Object: ObservableObject>(_ object: Object?) -> some View {
        if let object {
            environmentObject(object)
        } else {
            self
        }
    }
}

struct GaryxTaskNotificationCardMeasurement: Equatable {
    let naturalHeight: CGFloat
    let clampHeight: CGFloat
    let overflows: Bool
}

struct GaryxTaskNotificationCard: View {
    let notification: GaryxTaskNotification
    let onExpand: () -> Void
    let onFileLinkTap: (String) -> Void
    let onImageFilePreview: GaryxMarkdownImagePreviewResolver
    let onMeasurement: (GaryxTaskNotificationCardMeasurement) -> Void
    @ScaledMetric(relativeTo: .body) private var bodyFontLineHeight: CGFloat = UIFont
        .preferredFont(forTextStyle: .body).lineHeight
    @ScaledMetric(relativeTo: .body) private var bodyLineSpacing: CGFloat = 5
    @State private var naturalBodyHeight: CGFloat?

    init(
        notification: GaryxTaskNotification,
        onExpand: @escaping () -> Void,
        onFileLinkTap: @escaping (String) -> Void,
        onImageFilePreview: @escaping GaryxMarkdownImagePreviewResolver,
        onMeasurement: @escaping (GaryxTaskNotificationCardMeasurement) -> Void = { _ in }
    ) {
        self.notification = notification
        self.onExpand = onExpand
        self.onFileLinkTap = onFileLinkTap
        self.onImageFilePreview = onImageFilePreview
        self.onMeasurement = onMeasurement
    }

    private var clampHeight: CGFloat {
        (bodyFontLineHeight + bodyLineSpacing) * 10
    }

    private var bodyLayout: GaryxTaskNotificationBodyLayout? {
        naturalBodyHeight.map {
            GaryxTaskNotificationOverflow.collapsedBodyLayout(
                naturalHeight: Double($0),
                clampHeight: Double(clampHeight),
                epsilon: 0.5
            )
        }
    }

    private var bodyOverflows: Bool {
        bodyLayout?.showsExpand ?? false
    }

    var body: some View {
        GaryxFillProposedWidthLayout {
            cardSurface
        }
        .clipped()
        .accessibilityActions {
            if bodyOverflows {
                Button("Expand task notification", action: onExpand)
            }
        }
    }

    private var cardSurface: some View {
        VStack(alignment: .leading, spacing: 10) {
            GaryxTaskNotificationHeader(notification: notification)

            Rectangle()
                .fill(GaryxTheme.hairline)
                .frame(height: 1)

            GaryxMarkdownText(
                text: notification.finalMessage,
                foreground: GaryxTheme.primaryText,
                allowsRelativeFileLinks: true,
                allowsTextSelection: false,
                onFileLinkTap: onFileLinkTap,
                onImageFilePreview: onImageFilePreview
            )
            .fixedSize(horizontal: false, vertical: true)
            .onGeometryChange(for: CGSize.self) { geometry in
                geometry.size
            } action: { size in
                let measuredLayout = GaryxTaskNotificationOverflow.collapsedBodyLayout(
                    naturalHeight: Double(size.height),
                    clampHeight: Double(clampHeight),
                    epsilon: 0.5
                )
                onMeasurement(
                    GaryxTaskNotificationCardMeasurement(
                        naturalHeight: size.height,
                        clampHeight: clampHeight,
                        overflows: measuredLayout.isTruncated
                    )
                )
                if naturalBodyHeight.map({ abs($0 - size.height) > 0.25 }) ?? true {
                    naturalBodyHeight = size.height
                }
            }
            .frame(height: bodyLayout.map { CGFloat($0.displayedHeight) }, alignment: .top)
            .clipped()

            if bodyOverflows {
                Button(action: onExpand) {
                    Label("Expand", systemImage: "arrow.up.left.and.arrow.down.right")
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(GaryxTheme.secondaryText)
                }
                .buttonStyle(GaryxPressableRowStyle())
                .frame(maxWidth: .infinity, alignment: .trailing)
                .accessibilityLabel("Expand task notification")
                .accessibilityIdentifier("garyx-task-notification-expand")
            }
        }
        .padding(12)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
        .contentShape(Rectangle())
        .onTapGesture {
            guard bodyOverflows else { return }
            onExpand()
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Task ready for review")
        .accessibilityIdentifier("garyx-task-notification-card")
    }
}

/// Makes a flexible card consume exactly the width proposed by the shared
/// user-row owner. The card owns no width policy or numeric cap of its own.
private struct GaryxFillProposedWidthLayout: Layout {
    func sizeThatFits(
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) -> CGSize {
        guard let content = subviews.first else { return .zero }
        let contentSize = content.sizeThatFits(proposal)
        return CGSize(
            width: proposal.width ?? contentSize.width,
            height: contentSize.height
        )
    }

    func placeSubviews(
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) {
        guard let content = subviews.first else { return }
        content.place(
            at: bounds.origin,
            anchor: .topLeading,
            proposal: ProposedViewSize(width: bounds.width, height: proposal.height)
        )
    }
}

private struct GaryxTaskNotificationHeader: View {
    let notification: GaryxTaskNotification

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 7) {
                if !notification.taskId.isEmpty {
                    Text(notification.taskId)
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(GaryxTheme.secondaryText)
                }

                Text(GaryxTaskNotificationPresentation.statusLabel(for: notification.status))
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(GaryxTheme.secondaryText)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(Color.primary.opacity(0.035), in: Capsule())
                    .overlay {
                        Capsule()
                            .stroke(GaryxTheme.hairline, lineWidth: 1)
                    }
            }

            Text(notification.title)
                .font(GaryxFont.subheadline(weight: .semibold))
                .foregroundStyle(GaryxTheme.primaryText)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

struct GaryxTaskNotificationFullScreenView: View {
    let notification: GaryxTaskNotification
    let onDismiss: () -> Void

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    GaryxTaskNotificationHeader(notification: notification)

                    Rectangle()
                        .fill(GaryxTheme.hairline)
                        .frame(height: 1)

                    GaryxMarkdownText(
                        text: notification.finalMessage,
                        foreground: GaryxTheme.primaryText,
                        allowsRelativeFileLinks: false,
                        allowsTextSelection: true,
                        onFileLinkTap: nil,
                        onImageFilePreview: nil
                    )
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 18)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .navigationTitle("Task notification")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: onDismiss)
                        .fontWeight(.semibold)
                }
            }
        }
        .tint(GaryxTheme.controlTint)
        .garyxPageBackground()
        .accessibilityIdentifier("garyx-task-notification-full-screen")
        .accessibilityAction(.escape, onDismiss)
    }
}

private struct GaryxRestartNoticeCard: View {
    let notice: GaryxRestartNotice

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 6) {
                Circle()
                    .fill(GaryxTheme.accent)
                    .frame(width: 7, height: 7)
                Text("Garyx restarted")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(GaryxTheme.secondaryText)
            }

            Rectangle()
                .fill(GaryxTheme.hairline)
                .frame(height: 1)

            GaryxMarkdownText(
                text: notice.message,
                foreground: GaryxTheme.primaryText,
                fillsAvailableWidth: false,
                allowsRelativeFileLinks: true,
                allowsTextSelection: false,
                onFileLinkTap: nil,
                onImageFilePreview: nil
            )
        }
        .padding(12)
        // Restart notice is short — hug the content instead of stretching to the
        // full message width (which leaves the right side empty).
        .fixedSize(horizontal: true, vertical: false)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Garyx restarted")
    }
}

private struct GaryxMessageFilePreviewSheet: Identifiable {
    let id = UUID()
    let preview: GaryxWorkspaceFilePreview
}

private struct GaryxMessageCopyContextModifier: ViewModifier {
    let text: String
    var title = "Copy Message"
    var edge: GaryxMessageMenuEdge = .leading

    private var copyableText: String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func body(content: Content) -> some View {
        content
            .garyxInPlaceMessageMenu(edge: edge) {
                guard !copyableText.isEmpty else { return [] }
                return [
                    GaryxMessageMenuItem(title: title, systemImage: "doc.on.doc") {
                        GaryxClipboard.copyString(text)
                    }
                ]
            }
            .accessibilityAction(named: Text(title)) {
                guard !copyableText.isEmpty else { return }
                GaryxClipboard.copyString(text)
            }
    }
}

private extension View {
    func garyxMessageCopyContext(
        text: String,
        title: String = "Copy Message",
        edge: GaryxMessageMenuEdge = .leading
    ) -> some View {
        modifier(GaryxMessageCopyContextModifier(text: text, title: title, edge: edge))
    }

    func garyxMessageInteraction(text: String, edge: GaryxMessageMenuEdge = .leading) -> some View {
        modifier(GaryxMessageInteractionModifier(text: text, edge: edge))
    }
}

/// Long-press surface for message bubbles: copy the whole message, open the
/// drag-handle selection sheet, or share. Presented through the in-place
/// floating menu — the pressed message must keep its exact position, size,
/// and style (no system context-menu lift).
private struct GaryxMessageInteractionModifier: ViewModifier {
    let text: String
    var edge: GaryxMessageMenuEdge = .leading

    @State private var showsTextSelection = false
    @State private var showsShareSheet = false

    private var copyableText: String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func body(content: Content) -> some View {
        content
            .garyxInPlaceMessageMenu(edge: edge) {
                guard !copyableText.isEmpty else { return [] }
                return [
                    GaryxMessageMenuItem(title: "Copy", systemImage: "doc.on.doc") {
                        GaryxClipboard.copyString(text)
                    },
                    GaryxMessageMenuItem(title: "Select Text", systemImage: "character.cursor.ibeam") {
                        showsTextSelection = true
                    },
                    GaryxMessageMenuItem(title: "Share", systemImage: "square.and.arrow.up") {
                        showsShareSheet = true
                    },
                ]
            }
            .garyxSheet(isPresented: $showsTextSelection) {
                GaryxMessageTextSelectionSheet(text: text)
            }
            .garyxSheet(isPresented: $showsShareSheet) {
                GaryxActivityShareSheet(items: [text])
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
    @EnvironmentObject private var model: GaryxMobileModel

    let attachment: GaryxMobileMessageAttachment
    let isUser: Bool

    @State private var decodedImage: UIImage?
    @State private var decodedImageKey: String?
    @State private var gatewayPreviewDataUrl: String?
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
        .buttonStyle(GaryxPressableRowStyle())
        .garyxFullScreenCover(isPresented: $showsPreview) {
            GaryxFullscreenImagePreview(
                source: GaryxImagePreviewSource(
                    title: attachment.name.isEmpty ? "Image" : attachment.name,
                    dataUrl: attachment.dataUrl ?? gatewayPreviewDataUrl,
                    remoteUrl: attachment.remoteUrl,
                    filePath: gatewayPreviewDataUrl == nil ? Self.localFilePath(from: attachment.path) : nil,
                    initialImage: decodedImage
                )
            ) {
                showsPreview = false
            }
        }
        .garyxInPlaceMessageMenu(edge: isUser ? .trailing : .leading) {
            var items: [GaryxMessageMenuItem] = []
            if let decodedImage {
                items.append(GaryxMessageMenuItem(title: "Copy Image", systemImage: "photo.on.rectangle") {
                    GaryxClipboard.copyImage(decodedImage)
                })
            }
            if let sourceText = imageSourceText {
                items.append(GaryxMessageMenuItem(title: "Copy Image Source", systemImage: "link") {
                    GaryxClipboard.copyString(sourceText)
                })
            }
            if !attachment.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                items.append(GaryxMessageMenuItem(title: "Copy Name", systemImage: "text.cursor") {
                    GaryxClipboard.copyString(attachment.name)
                })
            }
            return items
        }
        .accessibilityAction(named: Text("Copy Image Source")) {
            guard let imageSourceText else { return }
            GaryxClipboard.copyString(imageSourceText)
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
                .garyxReadingLineLimit()
                .truncationMode(.middle)
                .padding(.horizontal, 10)
        }
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var dataUrlDecodeKey: String {
        let raw = attachment.dataUrl ?? ""
        let path = attachment.path ?? ""
        return "\(attachment.id):\(raw.count):\(raw.hashValue):\(path.hashValue)"
    }

    @MainActor
    private func updateDecodedImage() async {
        let key = dataUrlDecodeKey
        guard decodedImageKey != key else { return }
        decodedImage = nil
        gatewayPreviewDataUrl = nil
        decodedImageKey = key
        if let raw = attachment.dataUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
           !raw.isEmpty {
            let image = await Task.detached(priority: .utility) {
                GaryxImageDecoder.image(fromDataUrl: raw, maxPixelSize: 520)
            }.value
            guard !Task.isCancelled, decodedImageKey == key else { return }
            decodedImage = image
            return
        }
        guard let path = attachment.path?.trimmingCharacters(in: .whitespacesAndNewlines),
              !path.isEmpty,
              let preview = await model.localFilePreview(path, reportsError: false),
              preview.previewKind == "image",
              let dataUrl = preview.dataBase64?.trimmingCharacters(in: .whitespacesAndNewlines),
              !dataUrl.isEmpty else {
            return
        }
        let image = await Task.detached(priority: .utility) {
            GaryxImageDecoder.image(fromDataUrl: dataUrl, maxPixelSize: 520)
        }.value
        guard !Task.isCancelled, decodedImageKey == key else { return }
        gatewayPreviewDataUrl = dataUrl
        decodedImage = image
    }

    private var remoteURL: URL? {
        guard let raw = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
              raw.hasPrefix("http://") || raw.hasPrefix("https://") else {
            return nil
        }
        return URL(string: raw)
    }

    private var imageSourceText: String? {
        if let remoteUrl = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
           !remoteUrl.isEmpty {
            return remoteUrl
        }
        if let path = attachment.path?.trimmingCharacters(in: .whitespacesAndNewlines),
           !path.isEmpty {
            return path
        }
        return nil
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
    @ScaledMetric(relativeTo: .footnote) private var verticalPadding: CGFloat = 8
    let attachment: GaryxMobileMessageAttachment
    let isUser: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "doc")
                .font(GaryxFont.footnote(weight: .semibold))
                .frame(width: 18, height: 18)
            Text(attachment.name.isEmpty ? "Attachment" : attachment.name)
                .font(GaryxFont.footnote(weight: .medium))
                .garyxReadingLineLimit()
                .truncationMode(.middle)
        }
        .foregroundStyle(.primary)
        .padding(.horizontal, 11)
        .padding(.vertical, verticalPadding)
        .frame(minHeight: 34)
        .background(
            isUser ? Color.black.opacity(0.06) : Color(.secondarySystemFill),
            in: Capsule()
        )
        .garyxInPlaceMessageMenu(edge: isUser ? .trailing : .leading) {
            var items: [GaryxMessageMenuItem] = []
            if let sourceText {
                items.append(GaryxMessageMenuItem(title: "Copy File Path", systemImage: "doc.on.doc") {
                    GaryxClipboard.copyString(sourceText)
                })
            }
            if !attachment.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                items.append(GaryxMessageMenuItem(title: "Copy Name", systemImage: "text.cursor") {
                    GaryxClipboard.copyString(attachment.name)
                })
            }
            return items
        }
        .accessibilityAction(named: Text("Copy File Path")) {
            guard let sourceText else { return }
            GaryxClipboard.copyString(sourceText)
        }
        .accessibilityLabel(attachment.name.isEmpty ? "File attachment" : attachment.name)
    }

    private var sourceText: String? {
        if let path = attachment.path?.trimmingCharacters(in: .whitespacesAndNewlines),
           !path.isEmpty {
            return path
        }
        if let remoteUrl = attachment.remoteUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
           !remoteUrl.isEmpty {
            return remoteUrl
        }
        return nil
    }
}

#if DEBUG
@MainActor
struct GaryxTaskNotificationDebugFixture {
    static var current: Self? {
        ProcessInfo.processInfo.environment["GARYX_MOBILE_TASK_NOTIFICATION_FIXTURE"] == "1"
            ? Self()
            : nil
    }

    var view: some View {
        GaryxTaskNotificationDebugFixtureView()
    }
}

@MainActor
private struct GaryxTaskNotificationDebugFixtureView: View {
    @State private var selection: GaryxTaskNotificationSelection?
    @State private var interactionStatus = "ready"

    private static let body = """
    [Open validation file](validation-report.md)

    This synthetic notification verifies the production card on an iOS 26 simulator.

    1. The collapsed body uses ten measured line boxes.
    2. The shared user-role owner controls the trailing width.
    3. Relative links retain their own interaction.
    4. Long press keeps Copy, Select Text, and Share available.
    5. The full-screen owner retains an immutable snapshot.
    6. The final lines remain outside the collapsed viewport.
    7. Dynamic Type changes remeasure the body.
    8. A repeated task notification is identified by message sequence.
    9. Row eviction cannot truncate the selected snapshot.
    10. Gateway and occurrence changes dismiss stale selection.
    11. This line proves the notification exceeds the clamp.

    Complete body end marker: TASK-NOTIFICATION-E2E-END
    """

    private static let notification = GaryxTaskNotification(
        event: "ready_for_review",
        status: "in_review",
        taskId: "#TASK-42",
        title: "Structured notification review",
        finalMessage: body
    )

    var body: some View {
        NavigationStack {
            ZStack(alignment: .bottom) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        Text("Task notification fixture")
                            .font(GaryxFont.title2(weight: .bold))

                        GaryxUserRoleMessageContainer {
                            GaryxTaskNotificationCard(
                                notification: Self.notification,
                                onExpand: present,
                                onFileLinkTap: { target in
                                    interactionStatus = "link:\(target)"
                                },
                                onImageFilePreview: { _ in nil }
                            )
                            .garyxMessageInteraction(text: Self.body, edge: .trailing)
                        }

                        Text(interactionStatus)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .accessibilityIdentifier("task-notification.fixture.status")
                    }
                    .padding(16)
                }
            }
            .garyxMessageMenuHost()
            .navigationTitle("Notification fixture")
            .navigationBarTitleDisplayMode(.inline)
        }
        .garyxPageBackground()
        .garyxFullScreenCover(item: $selection) { selected in
            GaryxTaskNotificationFullScreenView(notification: selected.notification) {
                selection = nil
            }
        }
    }

    private func present() {
        selection = GaryxTaskNotificationSelection(
            messageId: "fixture-task-notification",
            messageSeq: 42,
            notification: Self.notification
        )
    }
}
#endif
