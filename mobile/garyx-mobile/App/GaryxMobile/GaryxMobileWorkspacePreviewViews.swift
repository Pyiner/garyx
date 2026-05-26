import SwiftUI
import UIKit
@preconcurrency import WebKit

struct GaryxWorkspacePreviewBody: View {
    let preview: GaryxWorkspaceFilePreview

    var body: some View {
        Group {
            switch preview.previewKind {
            case "markdown":
                GaryxWorkspaceMarkdownPreview(preview: preview)
            case "html":
                GaryxWorkspaceHTMLPreview(preview: preview)
            case "image":
                if let image {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .frame(maxWidth: .infinity, maxHeight: 260)
                        .background(
                            Color(.tertiarySystemBackground),
                            in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                        )
                } else {
                    unavailablePreview
                }
            case "text":
                if let text = preview.text, !text.isEmpty {
                    GaryxWorkspaceTextPreview(text: text)
                } else {
                    unavailablePreview
                }
            default:
                unavailablePreview
            }
        }
    }

    private var image: UIImage? {
        guard preview.previewKind == "image",
              let dataBase64 = preview.dataBase64 else {
            return nil
        }
        return GaryxDataURLImageCache.image(from: dataBase64)
    }

    private var unavailablePreview: some View {
        Text(preview.previewKind == "pdf" ? "PDF preview available on desktop." : "No inline preview available.")
            .font(GaryxFont.callout())
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

struct GaryxFullscreenWorkspaceFilePreview: View {
    @EnvironmentObject private var model: GaryxMobileModel

    let preview: GaryxWorkspaceFilePreview
    let onDismiss: () -> Void

    @State private var currentPreview: GaryxWorkspaceFilePreview
    @State private var isOpeningLinkedFile = false

    init(preview: GaryxWorkspaceFilePreview, onDismiss: @escaping () -> Void) {
        self.preview = preview
        self.onDismiss = onDismiss
        _currentPreview = State(initialValue: preview)
    }

    var body: some View {
        Group {
            if currentPreview.previewKind == "image" {
                GaryxFullscreenImagePreview(source: imagePreviewSource, onDismiss: onDismiss)
            } else {
                documentPreview
            }
        }
        .onChange(of: previewIdentity(for: preview)) {
            currentPreview = preview
        }
    }

    private var documentPreview: some View {
        ZStack {
            Color(.systemBackground).ignoresSafeArea()

            previewContent
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .ignoresSafeArea(edges: currentPreview.previewKind == "html" ? .all : [])
        }
        .overlay(alignment: .topTrailing) {
            closeButton
                .padding(.top, 12)
                .padding(.trailing, 16)
        }
        .overlay(alignment: .bottom) {
            if isOpeningLinkedFile {
                ProgressView()
                    .controlSize(.regular)
                    .padding(14)
                    .garyxAdaptiveGlass(
                        .regular,
                        isInteractive: false,
                        tint: Color(.systemBackground).opacity(0.72),
                        fallbackMaterial: .ultraThinMaterial,
                        in: Capsule()
                    )
                    .padding(.bottom, 22)
            }
        }
    }

    @ViewBuilder
    private var previewContent: some View {
        switch currentPreview.previewKind {
        case "html":
            GaryxHTMLWebView(
                html: currentPreview.text ?? "<body></body>",
                baseURL: currentPreview.garyxPreviewBaseURL,
                onFileLinkTap: openFileLink
            )
        case "markdown":
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    previewHeader
                    if let text = currentPreview.text, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        GaryxMarkdownText(
                            text: text,
                            foreground: .primary,
                            allowsRelativeFileLinks: true,
                            onFileLinkTap: openFileLink
                        )
                        .textSelection(.enabled)
                    } else {
                        unavailablePreview
                    }
                }
                .padding(.horizontal, 18)
                .padding(.bottom, 32)
            }
            .safeAreaPadding(.top, 72)
        case "text":
            ScrollView([.vertical, .horizontal], showsIndicators: true) {
                VStack(alignment: .leading, spacing: 14) {
                    previewHeader
                    if let text = currentPreview.text, !text.isEmpty {
                        Text(text)
                            .font(.system(size: 13, design: .monospaced))
                            .foregroundStyle(.primary)
                            .textSelection(.enabled)
                    } else {
                        unavailablePreview
                    }
                }
                .padding(.horizontal, 18)
                .padding(.bottom, 32)
            }
            .safeAreaPadding(.top, 72)
        default:
            VStack(spacing: 14) {
                Image(systemName: currentPreview.previewKind == "pdf" ? "doc.richtext" : "doc")
                    .font(GaryxFont.title2(weight: .medium))
                Text(currentPreview.previewKind == "pdf" ? "PDF preview available on desktop." : "No inline preview available.")
                    .font(GaryxFont.callout(weight: .medium))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .padding(.horizontal, 30)
        }
    }

    private var previewHeader: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(currentPreview.displayName)
                .font(GaryxFont.subheadline(weight: .semibold))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.middle)
            Text(currentPreview.path)
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.trailing, 54)
    }

    private var unavailablePreview: some View {
        Text("No inline preview available.")
            .font(GaryxFont.callout())
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    private var closeButton: some View {
        Button {
            onDismiss()
        } label: {
            Image(systemName: "xmark")
                .font(GaryxFont.system(size: 16, weight: .semibold))
                .foregroundStyle(.primary)
                .frame(width: 44, height: 44)
                .garyxAdaptiveGlass(
                    .regular,
                    isInteractive: true,
                    tint: Color(.systemBackground).opacity(0.32),
                    fallbackMaterial: .ultraThinMaterial,
                    in: Circle()
                )
        }
        .buttonStyle(.plain)
        .contentShape(Circle())
        .accessibilityLabel("Close file preview")
    }

    private var imagePreviewSource: GaryxImagePreviewSource {
        GaryxImagePreviewSource(
            title: currentPreview.displayName,
            dataUrl: currentPreview.dataBase64,
            remoteUrl: nil,
            filePath: nil
        )
    }

    private func openFileLink(_ target: String) {
        guard !isOpeningLinkedFile else { return }
        isOpeningLinkedFile = true
        let sourcePreview = currentPreview
        Task {
            let preview = await model.workspaceFilePreviewLink(target, from: sourcePreview)
            guard !Task.isCancelled else { return }
            if let preview {
                currentPreview = preview
            }
            isOpeningLinkedFile = false
        }
    }

    private func previewIdentity(for preview: GaryxWorkspaceFilePreview) -> String {
        [
            preview.workspaceDir,
            preview.path,
            preview.previewKind,
            String(preview.size),
            preview.modifiedAt ?? "",
        ].joined(separator: "|")
    }
}

private struct GaryxWorkspaceMarkdownPreview: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let preview: GaryxWorkspaceFilePreview

    var body: some View {
        if let text = preview.text, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            GaryxMarkdownText(
                text: text,
                foreground: .primary,
                allowsRelativeFileLinks: true,
                onFileLinkTap: openFileLink
            )
            .textSelection(.enabled)
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        } else {
            Text("No markdown content available.")
                .font(GaryxFont.callout())
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(10)
                .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
    }

    private func openFileLink(_ target: String) {
        Task { await model.openWorkspacePreviewLink(target, from: preview) }
    }
}

private extension GaryxWorkspaceFilePreview {
    var displayName: String {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmedName.isEmpty { return trimmedName }
        let fallback = path.garyxLastPathComponent.trimmingCharacters(in: .whitespacesAndNewlines)
        return fallback.isEmpty ? "Preview" : fallback
    }

    var garyxPreviewBaseURL: URL? {
        let workspace = workspaceDir.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty else { return URL(fileURLWithPath: "/", isDirectory: true) }
        let parent = (path as NSString).deletingLastPathComponent
        let directory = parent == "." ? workspace : "\(workspace)/\(parent)"
        return URL(fileURLWithPath: directory, isDirectory: true)
    }
}

private struct GaryxWorkspaceHTMLPreview: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let preview: GaryxWorkspaceFilePreview

    var body: some View {
        GaryxHTMLWebView(
            html: preview.text ?? "<body></body>",
            baseURL: baseURL,
            onFileLinkTap: openFileLink
        )
        .frame(height: 320)
        .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }

    private var baseURL: URL? {
        preview.garyxPreviewBaseURL
    }

    private func openFileLink(_ target: String) {
        Task { await model.openWorkspacePreviewLink(target, from: preview) }
    }
}

private struct GaryxWorkspaceTextPreview: View {
    let text: String

    var body: some View {
        ScrollView([.vertical, .horizontal], showsIndicators: true) {
            Text(text)
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .padding(10)
        }
        .frame(maxHeight: 240, alignment: .topLeading)
        .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

private struct GaryxHTMLWebView: UIViewRepresentable {
    let html: String
    let baseURL: URL?
    let onFileLinkTap: (String) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onFileLinkTap: onFileLinkTap)
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false

        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = context.coordinator
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.backgroundColor = .clear
        webView.scrollView.contentInsetAdjustmentBehavior = .never
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        context.coordinator.onFileLinkTap = onFileLinkTap
        let key = "\(baseURL?.absoluteString ?? ""):\(html.hashValue):\(html.count)"
        guard context.coordinator.loadedKey != key else { return }
        context.coordinator.loadedKey = key
        webView.loadHTMLString(html, baseURL: baseURL)
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var onFileLinkTap: (String) -> Void
        var loadedKey: String?

        init(onFileLinkTap: @escaping (String) -> Void) {
            self.onFileLinkTap = onFileLinkTap
        }

        func webView(
            _ webView: WKWebView,
            decidePolicyFor navigationAction: WKNavigationAction,
            decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
        ) {
            guard navigationAction.navigationType == .linkActivated,
                  let url = navigationAction.request.url else {
                decisionHandler(.allow)
                return
            }

            if let path = GaryxMobileFileLink.localFilePath(from: url) {
                onFileLinkTap(path)
                decisionHandler(.cancel)
                return
            }

            if let scheme = url.scheme?.lowercased(),
               ["http", "https", "mailto"].contains(scheme) {
                UIApplication.shared.open(url)
                decisionHandler(.cancel)
                return
            }

            decisionHandler(.allow)
        }
    }
}
