import Foundation
import SwiftUI
import UIKit
import WebKit

// MARK: - Gallery

struct GaryxCapsulesView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxOpenSidebar) private var openSidebar
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @StateObject private var previewLoad = GaryxCapsulePreviewLoadCoordinator(maxActive: 2)
    @State private var deletionCandidate: GaryxCapsuleSummary?

    private var maxActivePreviews: Int { horizontalSizeClass == .regular ? 4 : 2 }

    private var columns: [GridItem] {
        horizontalSizeClass == .regular
            ? [GridItem(.adaptive(minimum: 170, maximum: 260), spacing: 14)]
            : [GridItem(.flexible(), spacing: 14), GridItem(.flexible(), spacing: 14)]
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
            .onAppear { previewLoad.setMaxActive(maxActivePreviews) }
            .onChange(of: horizontalSizeClass) { _, _ in previewLoad.setMaxActive(maxActivePreviews) }
            .onChange(of: model.capsules.map(\.id)) { _, ids in
                previewLoad.prune(validIds: Set(ids))
            }
            .fullScreenCover(item: $model.galleryFocusedCapsule) { capsule in
                GaryxCapsuleFocusedPreviewView(capsule: capsule)
            }
            .confirmationDialog(
                "Delete capsule?",
                isPresented: deleteConfirmationPresented,
                titleVisibility: .visible
            ) {
                Button("Delete", role: .destructive) {
                    if let deletionCandidate {
                        Task { await model.deleteCapsule(deletionCandidate) }
                    }
                    deletionCandidate = nil
                }
                Button("Cancel", role: .cancel) { deletionCandidate = nil }
            } message: {
                Text("This removes the Capsule metadata and HTML file.")
            }
    }

    @ViewBuilder
    private var content: some View {
        if model.capsules.isEmpty, model.isRemoteStatePending {
            GaryxLoadingPanelView(title: "Loading capsules...")
        } else if model.capsules.isEmpty {
            ScrollView {
                GaryxEmptyPanelView(
                    icon: GaryxMobilePanel.capsules.iconName,
                    title: "No capsules yet.",
                    text: "Capsules created by agents will appear here."
                )
                .frame(maxWidth: .infinity, minHeight: 360)
            }
        } else {
            ScrollView {
                LazyVGrid(columns: columns, alignment: .leading, spacing: 14) {
                    ForEach(model.capsules) { capsule in
                        GaryxCapsuleGalleryCard(
                            capsule: capsule,
                            previewLoad: previewLoad,
                            onOpen: { model.galleryFocusedCapsule = capsule },
                            onDelete: { deletionCandidate = capsule }
                        )
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 12)
                .padding(.bottom, 28)
            }
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
            GaryxSidebarMenuButton { openSidebar() }
        }
    }

    private var deleteConfirmationPresented: Binding<Bool> {
        Binding(
            get: { deletionCandidate != nil },
            set: { if !$0 { deletionCandidate = nil } }
        )
    }
}

// MARK: - Gallery card

private struct GaryxCapsuleGalleryCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let capsule: GaryxCapsuleSummary
    @ObservedObject var previewLoad: GaryxCapsulePreviewLoadCoordinator
    let onOpen: () -> Void
    let onDelete: () -> Void

    var body: some View {
        Button(action: onOpen) {
            VStack(alignment: .leading, spacing: 0) {
                GaryxCapsulePreviewThumbnail(
                    capsuleId: capsule.id,
                    revision: capsule.revision,
                    isActive: previewLoad.isActive(capsule.id),
                    cacheEpoch: model.capsuleHTMLCacheEpoch,
                    cornerRadius: 0,
                    showsBorder: false
                )
                .aspectRatio(16.0 / 10.0, contentMode: .fit)
                .onAppear { previewLoad.markVisible(capsule.id) }
                .onDisappear { previewLoad.markHidden(capsule.id) }

                // Hairline divider between the full-bleed preview and the meta,
                // mirroring Mac `.capsule-card-preview-shell` border-bottom.
                Rectangle()
                    .fill(GaryxTheme.hairline)
                    .frame(height: 0.5)

                VStack(alignment: .leading, spacing: 3) {
                    Text(capsule.displayTitle)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(subline)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.top, 10)
                .padding(.bottom, 12)
            }
            .background(Color.primary.opacity(0.03))
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button(role: .destructive, action: onDelete) {
                Label("Delete", systemImage: "trash")
            }
        }
    }

    /// Mac-style single-line subinfo ("time · creator"), derived in Core so the
    /// card stays a dumb renderer (no pill chips, no local switch tables).
    private var subline: String {
        let creator = GaryxCapsuleGalleryCardPresentation.creatorName(
            agentId: capsule.agentId,
            providerType: capsule.providerType,
            agents: model.agents,
            teams: model.teams
        )
        return GaryxCapsuleGalleryCardPresentation.subline(
            timeDisplay: capsule.formattedUpdatedAt,
            creator: creator
        )
    }
}

// MARK: - Shared preview thumbnail

/// Live capsule preview thumbnail. Mounts a non-interactive `WKWebView` only
/// when `isActive` (planner-approved). `cacheEpoch` is part of the `.task`
/// identity so an already-mounted thumbnail re-reconciles when the preview-HTML
/// cache is invalidated (e.g. a capsule was deleted and its cache pruned),
/// re-fetching `/serve` and resolving to `.deleted`.
struct GaryxCapsulePreviewThumbnail: View {
    let capsuleId: String
    let revision: Int
    let isActive: Bool
    let cacheEpoch: Int
    let cornerRadius: CGFloat
    /// Full-bleed card previews suppress the thumbnail's own rounded border so
    /// the containing card owns clipping and outlining. Focused thumbnails keep
    /// it by default.
    var showsBorder: Bool = true

    @EnvironmentObject private var model: GaryxMobileModel
    @State private var phase: Phase = .idle

    enum Phase: Equatable {
        case idle
        case loading
        case loaded(String)
        case deleted
        case failed
    }

    private struct LoadKey: Equatable {
        let capsuleId: String
        let revision: Int
        let isActive: Bool
        let epoch: Int
    }

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                .fill(Color.primary.opacity(0.045))
            content
        }
        .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
        .overlay {
            if showsBorder {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
        }
        .task(id: LoadKey(capsuleId: capsuleId, revision: revision, isActive: isActive, epoch: cacheEpoch)) {
            await reconcile()
        }
    }

    @ViewBuilder
    private var content: some View {
        switch phase {
        case let .loaded(html) where isActive:
            GaryxCapsuleThumbnailWebView(html: html)
        case .deleted:
            placeholder(systemName: "trash", text: "Capsule deleted")
        case .failed:
            placeholder(systemName: "exclamationmark.triangle", text: "Preview unavailable")
        default:
            Image(systemName: GaryxMobilePanel.capsules.iconName)
                .font(GaryxFont.system(size: 22, weight: .semibold))
                .foregroundStyle(.tertiary)
        }
    }

    private func placeholder(systemName: String, text: String) -> some View {
        VStack(spacing: 6) {
            Image(systemName: systemName)
                .font(GaryxFont.system(size: 18, weight: .semibold))
            Text(text)
                .font(GaryxFont.caption(weight: .medium))
        }
        .foregroundStyle(.secondary)
        .padding(8)
        .multilineTextAlignment(.center)
    }

    private func reconcile() async {
        guard isActive else { return }
        if case .idle = phase { phase = .loading }
        let result = await model.loadCapsulePreviewHTML(capsuleId: capsuleId, revision: revision)
        switch result {
        case let .html(html):
            phase = .loaded(html)
        case .deleted:
            phase = .deleted
        case .failed:
            phase = .failed
        }
    }
}

/// Renders capsule HTML into a fixed virtual canvas scaled to the card width, so
/// the page lays out at a stable desktop-ish viewport and is shrunk to a
/// thumbnail. Non-interactive and opaque-origin sandboxed (no bridge, no
/// persistence, `baseURL: nil` so the meta CSP still governs).
private struct GaryxCapsuleThumbnailWebView: View {
    let html: String
    private let virtualWidth: CGFloat = 760

    var body: some View {
        GeometryReader { geometry in
            let scale = geometry.size.width / virtualWidth
            let virtualHeight = scale > 0 ? geometry.size.height / scale : geometry.size.height
            GaryxCapsuleThumbnailWebRepresentable(html: html)
                .frame(width: virtualWidth, height: max(1, virtualHeight))
                .scaleEffect(scale, anchor: .topLeading)
                .frame(width: geometry.size.width, height: geometry.size.height, alignment: .topLeading)
                .clipped()
                .allowsHitTesting(false)
        }
    }
}

private struct GaryxCapsuleThumbnailWebRepresentable: UIViewRepresentable {
    let html: String

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false

        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = context.coordinator
        webView.isUserInteractionEnabled = false
        webView.scrollView.isScrollEnabled = false
        webView.scrollView.bounces = false
        webView.scrollView.contentInsetAdjustmentBehavior = .never
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.backgroundColor = .clear
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        let token = "\(html.count):\(html.hashValue)"
        guard context.coordinator.loadedToken != token else { return }
        context.coordinator.loadedToken = token
        webView.loadHTMLString(html, baseURL: nil)
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var loadedToken: String?

        func webView(
            _ webView: WKWebView,
            decidePolicyFor navigationAction: WKNavigationAction,
            decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
        ) {
            // Non-interactive thumbnail: allow sub-frames and the initial
            // about:blank load, cancel any main-frame navigation away.
            guard navigationAction.targetFrame?.isMainFrame != false else {
                decisionHandler(.allow)
                return
            }
            let scheme = navigationAction.request.url?.scheme?.lowercased() ?? ""
            decisionHandler(scheme == "about" ? .allow : .cancel)
        }
    }
}

// MARK: - Focused preview (de-nested)

struct GaryxCapsuleFocusedPreviewView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let capsule: GaryxCapsuleSummary
    @State private var phase: Phase = .loading
    @State private var showsDeleteConfirmation = false

    enum Phase: Equatable {
        case loading
        case html(String)
        case deleted
        case failed
    }

    var body: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .garyxPageBackground()
            .garyxAdaptiveTopBar {
                GaryxAdaptiveGlassContainer(spacing: 10) {
                    HStack(spacing: 12) {
                        Button { dismiss() } label: {
                            GaryxToolbarIcon(systemName: "chevron.down")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Close Capsule")

                        GaryxPanelHeaderTitle(title: capsule.displayTitle)
                            .layoutPriority(1)

                        Spacer(minLength: 0)

                        Button { Task { await load(forceRefresh: true) } } label: {
                            GaryxToolbarIcon(systemName: "arrow.clockwise")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Reload Capsule")

                        Menu {
                            if let sourceThreadId = capsule.threadId?
                                .trimmingCharacters(in: .whitespacesAndNewlines),
                               !sourceThreadId.isEmpty {
                                Button {
                                    Task { await model.openMobileRoute(.thread(sourceThreadId)) }
                                } label: {
                                    Label(
                                        "Open source conversation",
                                        systemImage: "bubble.left.and.bubble.right"
                                    )
                                }
                            }
                            Button { copyLink() } label: { Label("Copy Link", systemImage: "link") }
                            Button { copyID() } label: { Label("Copy ID", systemImage: "number") }
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
            // Focused open is the authoritative surface: always force-refresh so
            // a since-deleted capsule resolves to 404 -> deleted immediately.
            .task(id: "\(capsule.id):\(capsule.revision)") { await load(forceRefresh: true) }
            .confirmationDialog(
                "Delete capsule?",
                isPresented: $showsDeleteConfirmation,
                titleVisibility: .visible
            ) {
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
    private var content: some View {
        switch phase {
        case .loading:
            GaryxLoadingPanelView(title: "Loading capsule...")
        case let .html(html):
            GaryxCapsuleWebView(html: html)
        case .deleted:
            GaryxEmptyPanelView(
                icon: "trash",
                title: "Capsule deleted.",
                text: "This capsule is no longer available."
            )
        case .failed:
            GaryxEmptyPanelView(
                icon: "exclamationmark.triangle",
                title: "Unable to load capsule.",
                text: "Check your connection and try again."
            )
        }
    }

    private func load(forceRefresh: Bool) async {
        let result = await model.loadCapsulePreviewHTML(
            capsuleId: capsule.id,
            revision: capsule.revision,
            forceRefresh: forceRefresh
        )
        switch result {
        case let .html(html):
            phase = .html(html)
        case .deleted:
            phase = .deleted
        case .failed:
            // Keep any already-rendered page on a transient refresh failure;
            // only show the failure state when nothing is rendered yet.
            if case .html = phase { return }
            phase = .failed
        }
    }

    private func copyLink() {
        if let url = GaryxMobileRouteLink.make(.capsule(capsule.id)) {
            UIPasteboard.general.string = url.absoluteString
        }
    }

    private func copyID() {
        UIPasteboard.general.string = capsule.id
    }
}

/// Interactive focused-preview web view. External links open in the system
/// browser; unknown schemes are cancelled. Non-persistent, no script-message
/// bridge, `baseURL: nil` so the injected meta CSP governs.
struct GaryxCapsuleWebView: UIViewRepresentable {
    let html: String

    func makeCoordinator() -> Coordinator { Coordinator() }

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
        // Full-screen detail renders like a browser: fill the width and never
        // zoom. The injected viewport meta drives this; disabling pinch is a
        // belt-and-suspenders guarantee (#TASK-1453 problem B). Vertical
        // scrolling stays enabled — the card can be taller than the screen.
        webView.scrollView.pinchGestureRecognizer?.isEnabled = false
        webView.scrollView.bouncesZoom = false
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        let token = "\(html.count):\(html.hashValue)"
        guard context.coordinator.loadedToken != token else { return }
        context.coordinator.loadedToken = token
        // Force a device-width, non-zoomable viewport so the self-contained card
        // (served with only a CSP meta, no viewport) fills the screen instead of
        // laying out at WKWebView's desktop default and shrinking with gutters.
        webView.loadHTMLString(GaryxCapsuleViewport.ensuringMobileViewport(in: html), baseURL: nil)
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var loadedToken: String?

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

// MARK: - Chat capsule cards (dumb render of render_state.capsule_cards)

struct GaryxMobileCapsuleChatCardsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let turnId: String
    let cards: [GaryxRenderCapsuleCard]
    let activeKeys: Set<String>

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(cards) { card in
                GaryxMobileCapsuleChatCard(
                    card: card,
                    isActive: activeKeys.contains("\(turnId):\(card.capsuleId)")
                ) {
                    Task { await model.openMobileRoute(.capsule(card.capsuleId), source: .conversation) }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct GaryxMobileCapsuleChatCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let card: GaryxRenderCapsuleCard
    let isActive: Bool
    let onOpen: () -> Void

    var body: some View {
        Button(action: onOpen) {
            VStack(alignment: .leading, spacing: 0) {
                GaryxCapsulePreviewThumbnail(
                    capsuleId: card.capsuleId,
                    revision: card.revision,
                    isActive: isActive,
                    cacheEpoch: model.capsuleHTMLCacheEpoch,
                    cornerRadius: 0,
                    showsBorder: false
                )
                .aspectRatio(16.0 / 9.0, contentMode: .fit)

                VStack(alignment: .leading, spacing: 2) {
                    Text(displayTitle)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(GaryxCapsuleChatCardPresentation.subtitle(action: card.action))
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
            .background(Color.primary.opacity(0.03))
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(maxWidth: 320, alignment: .leading)
    }

    private var displayTitle: String {
        let trimmed = card.title.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Untitled Capsule" : trimmed
    }
}

// MARK: - Preview load coordinator

/// App-target observable wrapper around the pure `GaryxCapsulePreviewLoadPlanner`
/// for the gallery. A narrow store so cards observe only admission changes, not
/// the whole model. Publishes only when the active set actually changes.
@MainActor
final class GaryxCapsulePreviewLoadCoordinator: ObservableObject {
    @Published private(set) var activeIds: Set<String> = []
    private var planner: GaryxCapsulePreviewLoadPlanner

    init(maxActive: Int) {
        planner = GaryxCapsulePreviewLoadPlanner(maxActive: maxActive)
    }

    func isActive(_ id: String) -> Bool { activeIds.contains(id) }

    func setMaxActive(_ n: Int) {
        guard planner.maxActive != max(0, n) else { return }
        planner.setMaxActive(n)
        recompute()
    }

    func markVisible(_ id: String) {
        if planner.markVisible(id) { recompute() }
    }

    func markHidden(_ id: String) {
        if planner.markHidden(id) { recompute() }
    }

    func prune(validIds: Set<String>) {
        let before = planner.visibleIds
        planner.prune(keeping: validIds)
        if planner.visibleIds != before { recompute() }
    }

    private func recompute() {
        let next = Set(planner.activeIds)
        if next != activeIds { activeIds = next }
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
}
