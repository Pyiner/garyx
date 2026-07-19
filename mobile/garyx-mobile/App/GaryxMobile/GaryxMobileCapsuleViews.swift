import Foundation
import SwiftUI
import UIKit
import WebKit

// MARK: - Gallery

struct GaryxCapsulesView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxOpenSidebar) private var openSidebar
    @Environment(\.garyxRouteNavigationActions) private var routeNavigation
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @State private var deletionCandidate: GaryxCapsuleSummary?
    @State private var galleryTab = GaryxCapsuleGalleryTab.all

    private var visibleCapsules: [GaryxCapsuleSummary] {
        model.filteredCapsules(for: galleryTab)
    }

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
                    VStack(spacing: 8) {
                        HStack(spacing: 12) {
                            leadingButton
                            GaryxPanelHeaderTitle(title: "Capsules")
                                .layoutPriority(1)
                            Spacer(minLength: 0)
                        }
                        Picker("Capsule gallery", selection: $galleryTab) {
                            ForEach(GaryxCapsuleGalleryTab.allCases) { tab in
                                Text(tab.rawValue).tag(tab)
                            }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .tint(GaryxTheme.controlTint)
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
            .garyxFullScreenCover(item: $model.galleryFocusedCapsule) { selection in
                GaryxCapsuleFocusedPreviewView(selection: selection)
            }
            .garyxConfirmationDialog(
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
        } else if visibleCapsules.isEmpty {
            ScrollView {
                GaryxEmptyPanelView(
                    icon: "star",
                    title: "No favorite Capsules yet.",
                    text: "Favorite Capsules will appear here."
                )
                .frame(maxWidth: .infinity, minHeight: 360)
            }
        } else {
            ScrollView {
                LazyVGrid(columns: columns, alignment: .leading, spacing: 14) {
                    ForEach(visibleCapsules) { capsule in
                        GaryxCapsuleGalleryCard(
                            capsule: capsule,
                            onOpen: {
                                model.galleryFocusedCapsule = GaryxCapsulePreviewSelection(capsule: capsule)
                            },
                            onFavorite: {
                                Task { await model.toggleCapsuleFavorite(capsule) }
                            },
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
        if let dismiss = routeNavigation.dismiss {
            Button(action: dismiss) {
                GaryxToolbarIcon(systemName: "chevron.left")
            }
            .buttonStyle(.plain)
            .accessibilityLabel(routeNavigation.backLabel)
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
    let onOpen: () -> Void
    let onFavorite: () -> Void
    let onDelete: () -> Void

    var body: some View {
        Button(action: onOpen) {
            VStack(alignment: .leading, spacing: 0) {
                GaryxCapsulePreviewThumbnail(
                    capsuleId: capsule.id,
                    revision: capsule.revision,
                    rendition: .gallery,
                    cacheEpoch: model.capsuleHTMLCacheEpoch,
                    cornerRadius: 0,
                    showsBorder: false
                )
                .aspectRatio(16.0 / 10.0, contentMode: .fit)

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
            .overlay(alignment: .topTrailing) {
                if model.isCapsuleFavorited(capsule) {
                    GaryxFavoriteStar(isFavorited: true, size: 13)
                        .padding(7)
                        .background(.regularMaterial, in: Circle())
                        .padding(8)
                        .accessibilityHidden(true)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button(action: onFavorite) {
                Label(
                    model.isCapsuleFavorited(capsule) ? "Unfavorite" : "Favorite",
                    systemImage: model.isCapsuleFavorited(capsule) ? "star.slash" : "star"
                )
            }
            Divider()
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
            agents: model.agents
        )
        return GaryxCapsuleGalleryCardPresentation.subline(
            timeDisplay: capsule.formattedUpdatedAt,
            creator: creator
        )
    }
}

// MARK: - Shared preview thumbnail

/// Capsule preview thumbnail. Displays a **cached rendered image** — zero live
/// `WKWebView`. A cache miss renders once through the model's thumbnail stack
/// (`capsuleThumbnail`), which writes through to memory + disk. `cacheEpoch` is
/// part of the `.task` identity so an already-mounted thumbnail re-reconciles
/// when the cache is invalidated (delete/prune), re-resolving to the new image
/// or `.deleted`.
struct GaryxCapsulePreviewThumbnail: View {
    let capsuleId: String
    let revision: Int
    /// The surface shape: gallery cards are 16:10, chat cards 16:9. Part of the
    /// cache key so a snapshot is never served cropped-wrong to the other.
    let rendition: GaryxCapsuleThumbnailRendition
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
        case image(UIImage)
        case deleted
        case failed
    }

    private struct LoadKey: Equatable {
        let capsuleId: String
        let revision: Int
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
        .task(id: LoadKey(capsuleId: capsuleId, revision: revision, epoch: cacheEpoch)) {
            await reconcile()
        }
    }

    @ViewBuilder
    private var content: some View {
        switch phase {
        case let .image(image):
            Image(uiImage: image)
                .resizable()
                .aspectRatio(contentMode: .fill)
        case .deleted:
            placeholder(systemName: "trash", text: "Capsule deleted")
        case .failed:
            placeholder(systemName: "exclamationmark.triangle", text: "Preview unavailable")
        case .idle:
            // Pre-render placeholder: the capsule gem glyph, faint.
            GaryxCapsuleGlyph()
                .frame(width: 30, height: 30)
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
        let result = await model.capsuleThumbnail(
            capsuleId: capsuleId,
            revision: revision,
            rendition: rendition
        )
        switch result {
        case let .image(image):
            phase = .image(image)
        case .deleted:
            phase = .deleted
        case .failed:
            phase = .failed
        }
    }
}

// MARK: - Focused preview (de-nested)

struct GaryxCapsuleFocusedPreviewView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.garyxMotion) private var motion
    @EnvironmentObject private var model: GaryxMobileModel
    let selection: GaryxCapsulePreviewSelection
    @StateObject private var loader = GaryxCapsuleFocusedPreviewLoader()
    @StateObject private var gestureBridge = GaryxCapsuleDismissGestureBridge()
    @State private var settleDriver = GaryxGestureSettleDriver.displayLinked()
    @State private var retryGeneration = 0
    @State private var showsDeleteConfirmation = false
    @State private var dragState = GaryxCapsuleDragDismissState()
    @State private var dragGestureOrigin = CGSize.zero
    @State private var dragGestureActive = false
    @State private var webAtTop = true
    @State private var morphState = GaryxChromeMorphPresentationState.hidden
    @State private var pendingChromeAction: GaryxCapsuleChromeAction?
    @State private var fetchedSourceThread: GaryxThreadSummary?

    private var projectedCapsule: GaryxCapsuleSummary? {
        GaryxCapsulePreviewProjection.currentSummary(selection: selection, catalog: model.capsules)
    }

    private var displayCapsule: GaryxCapsuleSummary {
        GaryxCapsulePreviewProjection.displaySummary(selection: selection, catalog: model.capsules)
    }

    private var loadKey: GaryxCapsulePreviewLoadKey {
        GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: model.capsules,
            retryGeneration: retryGeneration
        )
    }

    private var isFavorited: Bool {
        projectedCapsule.map(model.isCapsuleFavorited) ?? false
    }

    private var sourceThreadId: String? {
        guard let id = displayCapsule.threadId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !id.isEmpty else { return nil }
        return id
    }

    private var sourceThread: GaryxThreadSummary? {
        guard let sourceThreadId else { return nil }
        return model.sidebarThreadSummary(for: sourceThreadId) ?? fetchedSourceThread
    }

    var body: some View {
        GeometryReader { geometry in
            let progress = GaryxCapsuleDragDismiss.dragProgress(
                phase: dragState.phase,
                translation: dragState.translation
            )
            ZStack {
                Color.black
                    .opacity(0.35 * (1 - progress))
                    .ignoresSafeArea()

                previewSurface
                    .offset(
                        x: dragState.translation.width,
                        y: dragState.translation.height
                    )
                    .scaleEffect(1 - progress * 0.06)

                GaryxCapsuleDismissGestureInstaller(
                    bridge: gestureBridge,
                    webAtTop: webAtTop,
                    panelPresented: morphState.isPresented || showsDeleteConfirmation,
                    containerWidth: geometry.size.width,
                    onChanged: handleGestureChanged,
                    onReleased: { velocity in
                        handleGestureReleased(velocity: velocity, containerWidth: geometry.size.width)
                    },
                    onCancelled: handleGestureCancelled
                )
                .frame(width: 0, height: 0)
                .accessibilityHidden(true)
            }
        }
        .overlayPreferenceValue(GaryxCapsuleChromeAnchorKey.self) { anchor in
            capsuleChromeOverlay(anchor: anchor)
        }
        .task(id: loadKey) {
            await loader.reconcile(key: loadKey, model: model)
        }
        .task {
            // Opening a focused preview always schedules a lightweight catalog
            // refresh through the single-flight/trailing coordinator.
            await model.refreshCapsules(reportFailure: false)
        }
        .task(id: sourceThreadId) {
            fetchedSourceThread = nil
            guard let sourceThreadId,
                  model.sidebarThreadSummary(for: sourceThreadId) == nil else { return }
            let fetched = await model.capsuleSourceThreadSummary(threadId: sourceThreadId)
            guard !Task.isCancelled, self.sourceThreadId == sourceThreadId else { return }
            fetchedSourceThread = fetched
        }
        .onChange(of: model.capsulePreviewSceneSignal) { _, signal in
            handleSceneSignal(signal)
        }
        .onDisappear {
            loader.cancelForDismiss(model: model)
            invalidateGestureMotion()
        }
        .garyxConfirmationDialog(
            "Delete capsule?",
            isPresented: $showsDeleteConfirmation,
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                Task {
                    // The model clears the owning cover binding only on success;
                    // failure keeps this detail visible.
                    await model.deleteCapsule(displayCapsule)
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the Capsule metadata and HTML file.")
        }
    }

    private var previewSurface: some View {
        content
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .garyxPageBackground()
            .garyxAdaptiveTopBar {
                GaryxAdaptiveGlassContainer(spacing: 10) {
                    HStack(spacing: 12) {
                        Button {
                            loader.cancelForDismiss(model: model)
                            dismiss()
                        } label: {
                            GaryxToolbarIcon(systemName: "chevron.down")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Close Capsule")

                        GaryxCapsuleChromeHeaderControl(
                            title: displayCapsule.displayTitle,
                            isHidden: morphState.isPresented,
                            onToggle: toggleCapsuleChromePanel
                        )

                        Spacer(minLength: 0)

                        Button {
                            if let projectedCapsule {
                                Task { await model.toggleCapsuleFavorite(projectedCapsule) }
                            }
                        } label: {
                            GaryxToolbarIcon {
                                GaryxFavoriteStar(
                                    isFavorited: isFavorited,
                                    size: 18
                                )
                            }
                        }
                        .buttonStyle(.plain)
                        .disabled(projectedCapsule == nil)
                        .accessibilityLabel(isFavorited ? "Unfavorite Capsule" : "Favorite Capsule")
                        .accessibilityHint(
                            projectedCapsule == nil ? "This Capsule is no longer in the catalog." : ""
                        )
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 10)
                    .padding(.bottom, 8)
                }
            }
    }

    @ViewBuilder
    private var content: some View {
        if let renderedContent = loader.renderedContent {
            GaryxCapsuleWebView(
                html: renderedContent.html,
                gestureBridge: gestureBridge,
                onScrollAtTopChange: { webAtTop = $0 }
            )
        } else if loader.loadStatus.phase == .deleted {
            GaryxEmptyPanelView(
                icon: "trash",
                title: "Capsule deleted.",
                text: "This capsule is no longer available."
            )
        } else if loader.loadStatus.phase == .failed {
            GaryxEmptyPanelView(
                icon: "exclamationmark.triangle",
                title: "Unable to load capsule.",
                text: loader.loadStatus.retryExhausted
                    ? "Return to the foreground to retry."
                    : "The Capsule will retry automatically."
            )
        } else {
            GaryxLoadingPanelView(title: "Loading capsule...")
        }
    }

    @ViewBuilder
    private func capsuleChromeOverlay(anchor: Anchor<CGRect>?) -> some View {
        if morphState.isPresented, let anchor {
            GeometryReader { geometry in
                ZStack(alignment: .topLeading) {
                    Color.black.opacity(morphState.isExpanded ? 0.10 : 0)
                        .ignoresSafeArea()
                        .contentShape(Rectangle())
                        .onTapGesture { requestCapsuleChromeDismiss() }
                        .accessibilityLabel("Close Capsule actions")
                        .accessibilityAddTraits(.isButton)

                    let renderedExpanded = motion.allowsSpatialMotion(.morphOpen)
                        ? morphState.isExpanded
                        : true
                    GaryxChromeMorphSurface(
                        isExpanded: renderedExpanded,
                        anchorRect: geometry[anchor],
                        containerSize: geometry.size,
                        metrics: GaryxCapsuleChromeMetrics.metrics,
                        onClose: requestCapsuleChromeDismiss
                    ) {
                        GaryxCapsuleChromePanel(
                            capsule: displayCapsule,
                            sourceThread: sourceThread,
                            compactRowWidth: geometry[anchor].width,
                            isExpanded: renderedExpanded,
                            onToggle: requestCapsuleChromeDismiss,
                            onAction: requestChromeAction
                        )
                    }
                    .opacity(
                        motion.allowsSpatialMotion(.morphOpen) || morphState.isExpanded ? 1 : 0
                    )
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
        }
    }

    private func toggleCapsuleChromePanel() {
        if morphState.isPresented {
            requestCapsuleChromeDismiss()
        } else {
            applyMorphEvent(.requestPresent)
        }
    }

    private func requestCapsuleChromeDismiss() {
        applyMorphEvent(.requestDismiss)
    }

    private func requestChromeAction(_ action: GaryxCapsuleChromeAction) {
        pendingChromeAction = action
        applyMorphEvent(.requestDismiss)
    }

    private func applyMorphEvent(_ event: GaryxChromeMorphPresentationEvent) {
        let transition = GaryxChromeMorphPresentationReducer.reduce(
            state: morphState,
            event: event,
            transitionMode: motion.resolution(.morphOpen).mode
        )
        switch transition.animation {
        case .none:
            morphState = transition.state
        case .open:
            withAnimation(motion.animation(.morphOpen)) {
                morphState = transition.state
            }
        case .close:
            withAnimation(
                motion.animation(.morphClose),
                completionCriteria: .logicallyComplete
            ) {
                morphState = transition.state
            } completion: {
                applyMorphEvent(.dismissAnimationCompleted)
            }
        }

        switch transition.schedule {
        case .none:
            if transition.state == .hidden { performPendingChromeAction() }
        case .expandOnNextTick:
            Task { @MainActor in applyMorphEvent(.expandTick) }
        case .completeDismissAfterAnimation:
            break
        }
    }

    private func performPendingChromeAction() {
        guard let action = pendingChromeAction else { return }
        pendingChromeAction = nil
        switch action {
        case .openSourceConversation:
            guard let sourceThreadId = displayCapsule.threadId?
                .trimmingCharacters(in: .whitespacesAndNewlines),
                !sourceThreadId.isEmpty else { return }
            Task { await model.openMobileRoute(.thread(sourceThreadId)) }
        case .copyLink:
            copyLink()
        case .copyID:
            copyID()
        case .delete:
            showsDeleteConfirmation = true
        }
    }

    private func handleSceneSignal(_ signal: GaryxCapsulePreviewSceneSignal) {
        switch signal.phase {
        case .inactive:
            invalidateGestureMotion()
            loader.cancelForScene(model: model, event: .sceneInactive)
        case .background:
            invalidateGestureMotion()
            loader.cancelForScene(model: model, event: .sceneBackground)
        case .active:
            Task { await handleSceneActive() }
        }
    }

    private func handleSceneActive() async {
        let keyBeforeRefresh = loadKey
        let refreshResult = await model.refreshCapsules(reportFailure: false)
        let keyAfterRefresh = loadKey
        // A revision change (including present -> missing) is owned solely by
        // `.task(id:)`; never revive the stale key's retry cycle.
        guard keyAfterRefresh.projectedRevision == keyBeforeRefresh.projectedRevision else { return }
        switch refreshResult {
        case .success, .failure:
            if loader.needsForegroundResume(for: keyBeforeRefresh) {
                loader.prepareForegroundResume()
                retryGeneration &+= 1
            }
        }
    }

    private func handleGestureChanged(startX: CGFloat, translation: CGSize) {
        if !dragGestureActive {
            dragGestureActive = true
            GaryxMobileHaptics.shared.prepare(.capsuleDismissCommitted)
            if let interrupted = settleDriver.interrupt() {
                dragState.translation = capsuleTranslation(
                    phase: dragState.phase,
                    distance: interrupted.value
                )
            }
            dragGestureOrigin = dragState.translation
        }
        let accumulatedTranslation = CGSize(
            width: dragGestureOrigin.width + translation.width,
            height: dragGestureOrigin.height + translation.height
        )
        var next = dragState
        GaryxCapsuleDragDismiss.reduce(
            state: &next,
            event: .changed(
                startX: startX,
                translation: accumulatedTranslation,
                webAtTop: webAtTop,
                panelPresented: morphState.isPresented || showsDeleteConfirmation
            )
        )
        dragState = next
    }

    private func handleGestureReleased(velocity: CGSize, containerWidth: CGFloat) {
        let releasedState = dragState
        dragGestureActive = false
        dragGestureOrigin = .zero
        var next = dragState
        let effect = GaryxCapsuleDragDismiss.reduce(
            state: &next,
            event: .released(velocity: velocity, containerWidth: containerWidth)
        )
        switch effect {
        case .dismiss:
            settleDriver.invalidate()
            dragState = next
            GaryxMobileHaptics.shared.play(.capsuleDismissCommitted)
            loader.cancelForDismiss(model: model)
            dismiss()
        case .snapBack:
            settleCapsuleBack(
                from: releasedState,
                releaseVelocity: velocity,
                targetState: next,
                curve: GaryxMotion.springCurve(for: .momentumSnapBack)
            )
        case .none:
            settleDriver.invalidate()
            dragState = next
        }
    }

    private func handleGestureCancelled() {
        let cancelledState = dragState
        dragGestureActive = false
        dragGestureOrigin = .zero
        var next = dragState
        GaryxCapsuleDragDismiss.reduce(state: &next, event: .cancelled)
        guard cancelledState.phase.ownsGesture else {
            settleDriver.invalidate()
            dragState = next
            return
        }
        settleCapsuleBack(
            from: cancelledState,
            releaseVelocity: .zero,
            targetState: next,
            curve: GaryxMotion.springCurve(for: .cancelSnapBack)
        )
    }

    private func settleCapsuleBack(
        from state: GaryxCapsuleDragDismissState,
        releaseVelocity: CGSize,
        targetState: GaryxCapsuleDragDismissState,
        curve: GaryxMotionPhysics.SpringCurve
    ) {
        let phase = state.phase
        let initialDistance = phase == .horizontalDismiss
            ? state.translation.width
            : state.translation.height
        let initialVelocity = phase == .horizontalDismiss
            ? releaseVelocity.width
            : releaseVelocity.height
        settleDriver.settle(
            from: initialDistance,
            to: 0,
            initialVelocity: initialVelocity,
            curve: curve,
            onUpdate: { sample in
                dragState = GaryxCapsuleDragDismissState(
                    phase: phase,
                    translation: capsuleTranslation(phase: phase, distance: sample.value)
                )
            },
            onCompletion: {
                dragState = targetState
            }
        )
    }

    private func capsuleTranslation(
        phase: GaryxCapsuleDragPhase,
        distance: CGFloat
    ) -> CGSize {
        let distance = max(0, distance)
        return phase == .horizontalDismiss
            ? CGSize(width: distance, height: 0)
            : CGSize(width: 0, height: distance)
    }

    private func invalidateGestureMotion() {
        settleDriver.invalidate()
        dragGestureOrigin = .zero
        dragGestureActive = false
        dragState = GaryxCapsuleDragDismissState()
    }

    private func copyLink() {
        if let url = GaryxMobileRouteLink.make(.capsule(selection.id)) {
            GaryxClipboard.copyString(url.absoluteString)
        }
    }

    private func copyID() {
        GaryxClipboard.copyString(selection.id)
    }
}

/// Interactive focused-preview web view. External links open in the system
/// browser; unknown schemes are cancelled. Non-persistent, no script-message
/// bridge, `baseURL: nil` so the injected meta CSP governs.
struct GaryxCapsuleWebView: UIViewRepresentable {
    let html: String
    var gestureBridge: GaryxCapsuleDismissGestureBridge? = nil
    /// Reports whether the web content is scrolled to the very top. Drives the
    /// full-screen pull-to-dismiss so a downward drag only dismisses from the top
    /// and never fights content scrolling (#TASK-1470).
    var onScrollAtTopChange: ((Bool) -> Void)? = nil

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
        // Disable vertical rubber-banding so a downward pull at the top isn't
        // swallowed by an overscroll bounce — it's left for the pull-to-dismiss
        // gesture (#TASK-1470). Scrolling within content is unaffected.
        webView.scrollView.bounces = false
        webView.scrollView.delegate = context.coordinator
        context.coordinator.onScrollAtTopChange = onScrollAtTopChange
        context.coordinator.gestureBridge = gestureBridge
        gestureBridge?.webViewPanGestureRecognizer = webView.scrollView.panGestureRecognizer
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        context.coordinator.onScrollAtTopChange = onScrollAtTopChange
        if context.coordinator.gestureBridge !== gestureBridge {
            if context.coordinator.gestureBridge?.webViewPanGestureRecognizer
                === webView.scrollView.panGestureRecognizer {
                context.coordinator.gestureBridge?.webViewPanGestureRecognizer = nil
            }
            context.coordinator.gestureBridge = gestureBridge
        }
        gestureBridge?.webViewPanGestureRecognizer = webView.scrollView.panGestureRecognizer
        let token = "\(html.count):\(html.hashValue)"
        guard context.coordinator.loadedToken != token else { return }
        context.coordinator.loadedToken = token
        // Force a device-width, non-zoomable viewport so the self-contained card
        // (served with only a CSP meta, no viewport) fills the screen instead of
        // laying out at WKWebView's desktop default and shrinking with gutters.
        webView.loadHTMLString(GaryxCapsuleViewport.ensuringMobileViewport(in: html), baseURL: nil)
    }

    static func dismantleUIView(_ webView: WKWebView, coordinator: Coordinator) {
        if coordinator.gestureBridge?.webViewPanGestureRecognizer
            === webView.scrollView.panGestureRecognizer {
            coordinator.gestureBridge?.webViewPanGestureRecognizer = nil
        }
        webView.scrollView.delegate = nil
        webView.navigationDelegate = nil
    }

    final class Coordinator: NSObject, WKNavigationDelegate, UIScrollViewDelegate {
        var loadedToken: String?
        var onScrollAtTopChange: ((Bool) -> Void)?
        weak var gestureBridge: GaryxCapsuleDismissGestureBridge?
        private var lastAtTop = true

        func scrollViewDidScroll(_ scrollView: UIScrollView) {
            let atTop = scrollView.contentOffset.y <= 0.5
            guard atTop != lastAtTop else { return }
            lastAtTop = atTop
            onScrollAtTopChange?(atTop)
        }

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

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(cards) { card in
                GaryxMobileCapsuleChatCard(card: card) {
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
    let onOpen: () -> Void

    var body: some View {
        Button(action: onOpen) {
            VStack(alignment: .leading, spacing: 0) {
                GaryxCapsulePreviewThumbnail(
                    capsuleId: card.capsuleId,
                    revision: card.revision,
                    rendition: .chatCard,
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

extension GaryxCapsuleSummary {
    var displayTitle: String {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Untitled Capsule" : trimmed
    }

    var formattedUpdatedAt: String? {
        garyxFormattedTaskTimestamp(updatedAt ?? createdAt)
    }
}
