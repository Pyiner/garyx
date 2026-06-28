import Foundation
import SwiftUI
import UIKit
import WebKit

struct GaryxCapsulesView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxOpenSidebar) private var openSidebar
    @State private var detailCapsule: GaryxCapsuleSummary?
    @State private var deletionCandidate: GaryxCapsuleSummary?

    private var detailPresented: Binding<Bool> {
        Binding(
            get: { detailCapsule != nil },
            set: { isPresented in
                if !isPresented {
                    detailCapsule = nil
                    model.clearCapsuleDetailState()
                }
            }
        )
    }

    var body: some View {
        content
            .garyxPageBackground()
            .garyxAdaptiveTopBar {
                GaryxAdaptiveGlassContainer(spacing: 10) {
                    HStack(spacing: 12) {
                        leadingButton

                        GaryxPanelHeaderTitle(title: "Capsules")
                            .layoutPriority(1)

                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 10)
                    .padding(.bottom, 8)
                }
            }
            .task {
                if model.capsules.isEmpty {
                    await model.refreshCapsules()
                }
            }
            .refreshable {
                await model.refreshCapsules()
            }
            .fullScreenCover(isPresented: detailPresented) {
                if let detailCapsule {
                    GaryxCapsuleDetailView(capsule: detailCapsule)
                }
            }
            .onChange(of: model.capsuleHTMLState.selectedCapsuleId) { _, selectedId in
                detailCapsule = selectedId.flatMap { id in model.capsules.first { $0.id == id } }
            }
            .confirmationDialog("Delete capsule?", isPresented: deleteConfirmationPresented, titleVisibility: .visible) {
                Button("Delete", role: .destructive) {
                    if let deletionCandidate {
                        Task { await model.deleteCapsule(deletionCandidate) }
                    }
                    deletionCandidate = nil
                }
                Button("Cancel", role: .cancel) {
                    deletionCandidate = nil
                }
            } message: {
                Text("This removes the Capsule metadata and HTML file.")
            }
    }

    @ViewBuilder
    private var content: some View {
        if model.capsules.isEmpty, model.isRemoteStatePending {
            GaryxLoadingPanelView(title: "Loading capsules...")
        } else if model.capsules.isEmpty {
            List {
                GaryxEmptyPanelView(
                    icon: GaryxMobilePanel.capsules.iconName,
                    title: "No capsules yet.",
                    text: "Capsules created by agents will appear here."
                )
                .listRowSeparator(.hidden)
                .listRowBackground(Color.clear)
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
        } else {
            List {
                Section("Capsules") {
                    ForEach(model.capsules) { capsule in
                        GaryxCapsuleRow(capsule: capsule) {
                            detailCapsule = capsule
                            Task { await model.openCapsule(capsule) }
                        } onDelete: {
                            deletionCandidate = capsule
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
        }
    }

    @ViewBuilder
    private var leadingButton: some View {
        if model.mainPanelLeadingEdgeAction != .openSidebar {
            Button {
                model.performMainPanelLeadingEdgeAction()
            } label: {
                GaryxToolbarIcon(systemName: "chevron.left")
            }
            .buttonStyle(.plain)
            .accessibilityLabel(model.mainPanelLeadingEdgeActionLabel)
        } else {
            GaryxSidebarMenuButton {
                openSidebar()
            }
        }
    }

    private var deleteConfirmationPresented: Binding<Bool> {
        Binding(
            get: { deletionCandidate != nil },
            set: { isPresented in
                if !isPresented { deletionCandidate = nil }
            }
        )
    }
}

private struct GaryxCapsuleRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let capsule: GaryxCapsuleSummary
    let onOpen: () -> Void
    let onDelete: () -> Void

    var body: some View {
        GaryxRowActionMenu(actions: actions) {
            Button(action: onOpen) {
                HStack(alignment: .center, spacing: 12) {
                    Image(systemName: GaryxMobilePanel.capsules.iconName)
                        .font(GaryxFont.system(size: 16, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 30, height: 30)
                        .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8, style: .continuous))

                    VStack(alignment: .leading, spacing: 5) {
                        Text(capsule.displayTitle)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)

                        if !capsule.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            Text(capsule.description)
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                        }

                        HStack(spacing: 6) {
                            if let timestamp = capsule.formattedUpdatedAt, !timestamp.isEmpty {
                                GaryxCapsuleMetadataChip(text: timestamp, systemImage: "clock")
                            }
                            GaryxCapsuleMetadataChip(text: capsule.byteSizeLabel, systemImage: "doc")
                            GaryxCapsuleMetadataChip(text: "r\(capsule.revision)", systemImage: "number")
                            GaryxCapsuleOwnerBadge(capsule: capsule, agents: model.agents, teams: model.teams)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)

                    Image(systemName: "chevron.right")
                        .font(GaryxFont.system(size: 11, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .padding(.vertical, 10)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private var actions: [GaryxRowAction] {
        [
            GaryxRowAction(title: "Delete", systemImage: "trash", tone: .destructive, action: onDelete)
        ]
    }
}

private struct GaryxCapsuleMetadataChip: View {
    let text: String
    var systemImage: String? = nil

    var body: some View {
        HStack(spacing: 4) {
            if let systemImage {
                Image(systemName: systemImage)
                    .font(GaryxFont.system(size: 9, weight: .semibold))
            }
            Text(text)
                .lineLimit(1)
        }
        .font(GaryxFont.caption(weight: .medium))
        .foregroundStyle(.secondary)
        .padding(.horizontal, 7)
        .padding(.vertical, 3)
        .background(Color.primary.opacity(0.05), in: Capsule())
    }
}

private struct GaryxCapsuleOwnerBadge: View {
    let capsule: GaryxCapsuleSummary
    let agents: [GaryxAgentSummary]
    let teams: [GaryxTeamSummary]

    var body: some View {
        let presentation = ownerPresentation
        GaryxCapsuleMetadataChip(text: presentation.displayName, systemImage: presentation.symbolName)
    }

    private var ownerPresentation: GaryxProviderPresentation {
        let agentId = capsule.agentId?.trimmingCharacters(in: .whitespacesAndNewlines)
        let fallbackName = agentId.flatMap { id in
            agents.first { $0.id == id }?.displayName
                ?? teams.first { $0.id == id }?.displayName
        }
        return GaryxProviderPresentation.make(
            agentId: agentId,
            providerType: capsule.providerType,
            fallbackName: fallbackName
        )
    }
}

struct GaryxCapsuleDetailView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let capsule: GaryxCapsuleSummary
    @State private var showsDeleteConfirmation = false

    var body: some View {
        VStack(spacing: 0) {
            detailContent
        }
        .garyxPageBackground()
        .garyxAdaptiveTopBar {
            GaryxAdaptiveGlassContainer(spacing: 10) {
                HStack(spacing: 12) {
                    Button {
                        dismiss()
                    } label: {
                        GaryxToolbarIcon(systemName: "chevron.down")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Close Capsule")

                    GaryxPanelHeaderTitle(title: capsule.displayTitle)
                        .layoutPriority(1)

                    Spacer(minLength: 0)

                    Button {
                        Task { await model.loadSelectedCapsuleHTML(forceRefresh: true) }
                    } label: {
                        GaryxToolbarIcon(systemName: "arrow.clockwise")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Reload Capsule")

                    Menu {
                        Button(role: .destructive) {
                            showsDeleteConfirmation = true
                        } label: {
                            Label("Delete", systemImage: "trash")
                        }
                    } label: {
                        GaryxToolbarIcon(systemName: "ellipsis")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Capsule actions")
                }
                .padding(.horizontal, 16)
                .padding(.top, 10)
                .padding(.bottom, 8)
            }
        }
        .task(id: capsule.id) {
            if model.capsuleHTMLState.selectedCapsuleId != capsule.id {
                await model.openCapsule(capsule)
            } else if !model.isCapsuleHTMLLoaded, !model.capsuleHTMLState.isLoading {
                await model.loadSelectedCapsuleHTML()
            }
        }
        .confirmationDialog("Delete capsule?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task {
                    await model.deleteCapsule(capsule)
                    dismiss()
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the Capsule metadata and HTML file.")
        }
    }

    @ViewBuilder
    private var detailContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 8) {
                Text(capsule.displayTitle)
                    .font(GaryxFont.title3(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                if !capsule.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(capsule.description)
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                HStack(spacing: 6) {
                    if let timestamp = capsule.formattedUpdatedAt, !timestamp.isEmpty {
                        GaryxCapsuleMetadataChip(text: timestamp, systemImage: "clock")
                    }
                    GaryxCapsuleMetadataChip(text: capsule.byteSizeLabel, systemImage: "doc")
                    GaryxCapsuleMetadataChip(text: "revision \(capsule.revision)", systemImage: "number")
                    GaryxCapsuleOwnerBadge(capsule: capsule, agents: model.agents, teams: model.teams)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 10)

            switch contentState {
            case .loading:
                GaryxLoadingPanelView(title: "Loading capsule...")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .failure(let message):
                GaryxEmptyPanelView(
                    icon: "exclamationmark.triangle",
                    title: "Unable to load capsule.",
                    text: message
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .html(let html):
                GaryxCapsuleWebView(html: html, cacheKey: GaryxCapsuleHTMLCacheKey(capsule: capsule))
                    .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .stroke(GaryxTheme.hairline, lineWidth: 1)
                    }
                    .padding(.horizontal, 16)
                    .padding(.bottom, 16)
            }
        }
    }

    private enum ContentState {
        case loading
        case failure(String)
        case html(String)
    }

    private var contentState: ContentState {
        if let html = model.capsuleHTMLState.html,
           model.capsuleHTMLState.loadedKey == GaryxCapsuleHTMLCacheKey(capsule: capsule) {
            return .html(html)
        }
        if let message = model.capsuleHTMLState.errorMessage,
           !model.capsuleHTMLState.isLoading {
            return .failure(message)
        }
        return .loading
    }
}

struct GaryxCapsuleWebView: UIViewRepresentable {
    let html: String
    let cacheKey: GaryxCapsuleHTMLCacheKey

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
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
        let loadedKey = "\(cacheKey.id):\(cacheKey.revision):\(cacheKey.htmlSha256):\(html.count):\(html.hashValue)"
        guard context.coordinator.loadedKey != loadedKey else { return }
        context.coordinator.loadedKey = loadedKey
        webView.loadHTMLString(html, baseURL: nil)
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var loadedKey: String?

        func webView(
            _ webView: WKWebView,
            decidePolicyFor navigationAction: WKNavigationAction,
            decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
        ) {
            guard navigationAction.targetFrame?.isMainFrame != false else {
                decisionHandler(.allow)
                return
            }

            guard let url = navigationAction.request.url else {
                decisionHandler(.allow)
                return
            }

            let scheme = url.scheme?.lowercased() ?? ""
            if scheme == "about" {
                decisionHandler(.allow)
                return
            }
            if ["http", "https", "mailto"].contains(scheme) {
                UIApplication.shared.open(url)
                decisionHandler(.cancel)
                return
            }
            decisionHandler(.cancel)
        }
    }
}

private extension GaryxCapsuleSummary {
    var displayTitle: String {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Untitled Capsule" : trimmed
    }

    var formattedUpdatedAt: String? {
        garyxFormattedTaskTimestamp(updatedAt ?? createdAt)
    }

    var byteSizeLabel: String {
        ByteCountFormatter.string(fromByteCount: Int64(byteSize), countStyle: .file)
    }
}
