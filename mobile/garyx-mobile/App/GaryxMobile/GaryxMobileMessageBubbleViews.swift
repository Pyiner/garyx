import Foundation
import SwiftUI
import UIKit

struct GaryxMessageBubble: View {
    let message: GaryxMobileMessage
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.garyxMessageBubbleActions) private var actions
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
            .garyxOptionalEnvironmentObject(actions.model)
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
                            .garyxMessageCopyContext(text: messageCopyText, edge: .trailing)
                    }

                    if let notification = taskNotification {
                        GaryxTaskNotificationCard(notification: notification)
                            .garyxMessageInteraction(text: taskNotificationCopyText(notification), edge: .trailing)
                    } else if let restart = restartNotice {
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
                        .padding(.vertical, 8)
                        .background(userBubbleBackground, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                        .garyxMessageInteraction(text: displayText, edge: .trailing)
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
                        .garyxMessageCopyContext(text: messageCopyText)
                }
                if message.isStreaming && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    if case .thinkingLabel(let text) = messagePresentation {
                        GaryxThinkingLabel(text: text)
                    }
                } else if let notification = taskNotification {
                    GaryxTaskNotificationCard(notification: notification)
                        .garyxMessageInteraction(text: taskNotificationCopyText(notification))
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
            .animation(message.isStreaming ? .easeOut(duration: 0.16) : nil, value: message.text)
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
                .padding(.vertical, 8)
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

    private var taskNotification: GaryxTaskNotification? {
        guard !message.isStreaming else { return nil }
        return GaryxTaskNotificationPresentation.parse(displayText)
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

private struct GaryxTaskNotificationCard: View {
    let notification: GaryxTaskNotification

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
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

            Rectangle()
                .fill(GaryxTheme.hairline)
                .frame(height: 1)

            GaryxMarkdownText(
                text: notification.finalMessage,
                foreground: GaryxTheme.primaryText,
                allowsRelativeFileLinks: true,
                allowsTextSelection: false,
                onFileLinkTap: nil,
                onImageFilePreview: nil
            )
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Task ready for review")
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
            .sheet(isPresented: $showsTextSelection) {
                GaryxMessageTextSelectionSheet(text: text)
            }
            .sheet(isPresented: $showsShareSheet) {
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
        .buttonStyle(.plain)
        .fullScreenCover(isPresented: $showsPreview) {
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
                .lineLimit(1)
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
