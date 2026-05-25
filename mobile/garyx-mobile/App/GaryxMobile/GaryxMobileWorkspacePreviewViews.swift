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
        let workspace = preview.workspaceDir.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty else { return URL(fileURLWithPath: "/", isDirectory: true) }
        let parent = (preview.path as NSString).deletingLastPathComponent
        let directory = parent == "." ? workspace : "\(workspace)/\(parent)"
        return URL(fileURLWithPath: directory, isDirectory: true)
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
