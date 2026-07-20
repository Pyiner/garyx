import Foundation
import SwiftUI
import UIKit

/// Anchor of the compact title capsule in the conversation top bar. The
/// runtime-panel morph surface resolves this anchor to start and end its
/// expansion exactly at the capsule's rect.
struct GaryxThreadRuntimeChromeAnchorKey: PreferenceKey {
    static var defaultValue: Anchor<CGRect>?

    static func reduce(value: inout Anchor<CGRect>?, nextValue: () -> Anchor<CGRect>?) {
        value = nextValue() ?? value
    }
}

enum GaryxThreadRuntimeMorphMetrics {
    static let collapsedCornerRadius: CGFloat = 22
    static let expandedCornerRadius: CGFloat = 28
    /// The expanded panel intentionally overlaps the back and ellipsis
    /// buttons, leaving only a slim margin — like the Dynamic Island
    /// growing over surrounding status content.
    static let horizontalMargin: CGFloat = 12
    static let maxExpandedWidth: CGFloat = 560
}

/// The avatar+title row shared by the top-bar capsule and the expanded
/// panel header. Both render the exact same view, so the morph never
/// re-lays-out the title — the text cannot jump or jitter.
struct GaryxThreadRuntimeCompactRow: View {
    @EnvironmentObject private var model: GaryxMobileModel

    /// The in-bar capsule caps itself so it shares the bar with the side
    /// buttons; the expanded panel header passes `nil` to let the title
    /// run the full panel width.
    var maxWidth: CGFloat? = 282

    private var title: String {
        model.selectedThread?.title ?? model.draftThreadTitle
    }

    var body: some View {
        GaryxThreadRuntimeCompactContentRow(
            title: title,
            target: model.selectedThreadAgentTarget,
            maxWidth: maxWidth
        )
    }
}

/// Model-free content shared by the live title control and the first route
/// frame. Sharing the row keeps avatar, typography, and capsule geometry
/// identical while the destination is still isolated from model updates.
struct GaryxThreadRuntimeCompactContentRow: View {
    let title: String
    let target: GaryxMobileAgentTarget?
    var maxWidth: CGFloat? = 282

    var body: some View {
        HStack(spacing: 8) {
            avatar(diameter: 22)

            Text(title)
                .font(GaryxFont.callout(weight: .medium))
                .foregroundStyle(.primary)
                .garyxReadingLineLimit()
                .truncationMode(.tail)
                .layoutPriority(1)
        }
        .padding(.horizontal, 12)
        .frame(height: 44, alignment: .leading)
        .frame(maxWidth: maxWidth ?? .infinity, alignment: .leading)
        // Compact and expanded morph headers must share this 44-point anchor;
        // XXL is the largest size that preserves the no-jump geometry.
        .garyxTypographyBoundary(.navigationChrome)
    }

    @ViewBuilder
    private func avatar(diameter: CGFloat) -> some View {
        if let target {
            GaryxAgentAvatarView(
                agentId: target.id,
                avatarDataUrl: target.avatarDataUrl,
                label: target.title,
                providerType: target.providerType,
                builtIn: target.builtIn,
                diameter: diameter
            )
        } else {
            Image(systemName: "person.crop.circle")
                .font(GaryxFont.fixedSystem(size: diameter * 0.72, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: diameter, height: diameter)
        }
    }
}

/// A single glass surface that morphs between the compact title capsule
/// rect and the expanded settings panel — one shape, one glass, no
/// matched-geometry pairs. The header row stays put; only the surface
/// frame, corner radius, and body opacity animate.
struct GaryxThreadRuntimeMorphSurface: View {
    @Environment(\.garyxMotion) private var motion
    let isExpanded: Bool
    let anchorRect: CGRect
    let containerSize: CGSize
    let onClose: () -> Void

    var body: some View {
        let renderedExpanded = motion.allowsSpatialMotion(.morphOpen) ? isExpanded : true
        GaryxChromeMorphSurface(
            isExpanded: renderedExpanded,
            anchorRect: anchorRect,
            containerSize: containerSize,
            metrics: GaryxChromeMorphSurfaceMetrics(
                horizontalMargin: GaryxThreadRuntimeMorphMetrics.horizontalMargin,
                maximumExpandedWidth: GaryxThreadRuntimeMorphMetrics.maxExpandedWidth,
                collapsedCornerRadius: GaryxThreadRuntimeMorphMetrics.collapsedCornerRadius,
                expandedCornerRadius: GaryxThreadRuntimeMorphMetrics.expandedCornerRadius
            ),
            onClose: onClose
        ) {
            GaryxThreadRuntimeSettingsPanel(
                compactRowWidth: anchorRect.width,
                isExpanded: renderedExpanded
            )
        }
        .opacity(motion.allowsSpatialMotion(.morphOpen) || isExpanded ? 1 : 0)
    }
}

struct GaryxThreadRuntimeSettingsPanel: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxMotion) private var motion
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .callout) private var settingsRowVerticalPadding: CGFloat = 8
    @ScaledMetric(relativeTo: .callout) private var optionsRowVerticalPadding: CGFloat = 8

    let compactRowWidth: CGFloat
    let isExpanded: Bool

    private enum Page: Hashable {
        case main
        case model
        case thinkingLevel
        case speed

        var title: String {
            switch self {
            case .main: "Thread settings"
            case .model: "Model"
            case .thinkingLevel: "Thinking level"
            case .speed: "Speed"
            }
        }
    }

    /// Two-phase drill-in coordinator (GaryxMobileCore): the current page
    /// fades out fully, then the page swaps and fades in. Only one page is
    /// ever mounted; the view owns clocks and curves, the pager owns the
    /// ordering rules.
    @State private var pager = GaryxRuntimePanelPager<Page>(page: .main)
    @State private var measuredOptionsHeight: CGFloat?

    private var page: Page { pager.page }
    private var pageContentVisible: Bool { pager.isContentVisible }

    private var selectedThread: GaryxThreadSummary? { model.selectedThread }
    private var runtime: GaryxThreadRuntimeSummary? { selectedThread?.threadRuntime }

    private var providerType: String {
        normalized(runtime?.providerType)
            ?? normalized(selectedThread?.providerType)
            ?? normalized(model.selectedThreadAgentTarget?.providerType)
            ?? ""
    }

    private var providerModels: GaryxProviderModels? {
        guard !providerType.isEmpty else { return nil }
        return model.providerModelsByType[providerType]
    }

    private var providerDefaultModel: String? {
        normalized(providerModels?.defaultModel)
    }

    private var modelOverride: String? {
        normalized(runtime?.modelOverride)
    }

    private var reasoningEffortOverride: String? {
        normalized(runtime?.modelReasoningEffortOverride)
    }

    private var effectiveModel: String? {
        normalized(runtime?.model) ?? providerDefaultModel
    }

    private var effectiveReasoningEffort: String? {
        normalized(runtime?.modelReasoningEffort) ?? defaultReasoningEffort(for: effectiveModel)
    }

    private var effortFilterModel: String? {
        GaryxThreadModelOverridePresentation.effortFilterModel(
            override: modelOverride,
            agentConfiguredModel: effectiveModel,
            providerModels: providerModels
        )
    }

    private var reasoningEfforts: [GaryxProviderModelOption] {
        GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: providerModels,
            model: effortFilterModel
        )
    }

    private var canSelectModel: Bool {
        providerModels?.supportsModelSelection == true && !modelOptions.isEmpty
    }

    private var canSelectReasoningEffort: Bool {
        !reasoningEffortOptions.isEmpty
    }

    private var serviceTierOverride: String? {
        normalized(runtime?.modelServiceTierOverride)
    }

    private var effectiveServiceTier: String? {
        normalized(runtime?.modelServiceTier)
    }

    private var serviceTiers: [GaryxProviderModelOption] {
        GaryxThreadModelOverridePresentation.serviceTierOptions(
            providerModels: providerModels,
            model: effortFilterModel
        )
    }

    private var canSelectServiceTier: Bool {
        !serviceTierOptions.isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header

            Group {
                if page == .main {
                    VStack(alignment: .leading, spacing: 12) {
                        currentAgentSection
                        runtimeSettingsSection
                    }
                    .padding(.horizontal, 14)
                    .padding(.bottom, 14)
                    .fixedSize(horizontal: false, vertical: true)
                    // The swap happens while the content is fully faded out;
                    // .identity suppresses the DEFAULT branch crossfade that
                    // would otherwise keep both pages visible together.
                    .transition(.identity)
                } else {
                    ScrollView {
                        // NOTE: no garyxVerticalScrollContentWidth here — its
                        // containerRelativeFrame resolves to the SCREEN inside
                        // this panel-embedded scroll view and pinned rows 12pt
                        // past the panel's right edge (asymmetric insets). A
                        // vertical ScrollView already proposes its own width.
                        optionsPage
                            .padding(.horizontal, 12)
                            .padding(.top, 4)
                            .padding(.bottom, 12)
                            .frame(maxWidth: .infinity)
                            .onGeometryChange(for: CGFloat.self) { geometry in
                                geometry.size.height
                            } action: { height in
                                guard height > 0 else { return }
                                measuredOptionsHeight = height
                            }
                    }
                    .frame(height: optionsViewportHeight)
                    // Measured-height corrections (Dynamic Type growth)
                    // settle with the same curve as the page entrance.
                    .animation(motion.spatialAnimation(.panelResize), value: optionsViewportHeight)
                    .scrollIndicators(.hidden)
                    .garyxAdaptiveSoftScrollEdge(for: [.top, .bottom])
                    .transition(.identity)
                }
            }
            // Exactly one page is ever mounted; drill-in is a two-phase
            // fade-out → swap → fade-in, so two pages can never overlap on
            // the translucent backing.
            .opacity(isExpanded && pageContentVisible ? 1 : 0)
            .offset(x: pageContentVisible ? 0 : pageHiddenOffset)
        }
        // Collapsing from a sub-page resets to the compact-row header
        // instantly (no page transition), keeping the capsule hand-off
        // free of "< Model" residue. The reset also invalidates any
        // in-flight phase-2 continuation.
        .onChange(of: isExpanded) { _, expanded in
            if !expanded {
                pager.reset(to: .main)
            }
        }
        .task(id: providerType) {
            guard !providerType.isEmpty,
                  model.providerModelsByType[providerType] == nil else {
                return
            }
            await model.loadProviderModels(providerType: providerType)
        }
    }

    /// The leaving page slips toward its side of the stack; the entering
    /// page arrives from its own side. Reduce Motion keeps only the fade.
    private var pageHiddenOffset: CGFloat {
        let distance = motion.offset(.runtimeDrilldownEnter, active: true).width
        return page == .main ? -distance : distance
    }

    private var panelMaxHeight: CGFloat {
        min(UIScreen.main.bounds.height * 0.62, 520)
    }

    /// Estimate from the default-size row metrics, corrected by the
    /// measured content height once laid out — Dynamic Type sizes grow the
    /// rows, and the viewport follows instead of forcing an inner scroll.
    private var optionsViewportHeight: CGFloat {
        GaryxRuntimeOptionsViewportMetrics.height(
            rowCount: optionsPageCount,
            hairlineHeight: 1 / UIScreen.main.scale,
            measuredContentHeight: measuredOptionsHeight,
            maxHeight: panelMaxHeight
        )
    }

    private var optionsPageCount: Int {
        switch page {
        case .main:
            0
        case .model:
            modelOptions.count
        case .thinkingLevel:
            reasoningEffortOptions.count
        case .speed:
            serviceTierOptions.count
        }
    }

    @ViewBuilder
    private var optionsPage: some View {
        switch page {
        case .main:
            EmptyView()
        case .model:
            optionsMenu(
                options: modelOptions,
                selectedId: selectedModelOptionId
            ) { selected in
                setPage(.main)
                Task {
                    await selectModel(selected)
                }
            }
        case .thinkingLevel:
            optionsMenu(
                options: reasoningEffortOptions,
                selectedId: selectedReasoningEffortOptionId
            ) { selected in
                setPage(.main)
                Task {
                    await model.updateSelectedThreadRuntimeSettings(reasoningEffort: selected)
                }
            }
        case .speed:
            optionsMenu(
                options: serviceTierOptions,
                selectedId: selectedServiceTierOptionId
            ) { selected in
                setPage(.main)
                Task {
                    await model.updateSelectedThreadRuntimeSettings(serviceTier: selected)
                }
            }
        }
    }

    /// Options render as a quiet menu directly on the glass surface —
    /// no filled card, hairline separators, trailing checkmark. The panel
    /// itself is the menu surface.
    private func optionsMenu(
        options: [(id: String, label: String)],
        selectedId: String,
        onSelect: @escaping (String) -> Void
    ) -> some View {
        VStack(spacing: 0) {
            ForEach(Array(options.enumerated()), id: \.element.id) { index, option in
                Button {
                    onSelect(option.id)
                } label: {
                    HStack(spacing: 12) {
                        Text(option.label)
                            .font(GaryxFont.callout(weight: selectedId == option.id ? .semibold : .regular))
                            .foregroundStyle(.primary)
                            .garyxReadingLineLimit()
                            .truncationMode(.tail)

                        Spacer(minLength: 0)

                        if selectedId == option.id {
                            GaryxSelectionCheckmark(size: 15)
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, optionsRowVerticalPadding)
                    .frame(minHeight: 44)
                    .contentShape(RoundedRectangle(cornerRadius: 11, style: .continuous))
                }
                .buttonStyle(GaryxRuntimeMenuRowButtonStyle())

                if index < options.count - 1 {
                    Rectangle()
                        .fill(Color.primary.opacity(0.075))
                        .frame(height: 1 / UIScreen.main.scale)
                        .padding(.horizontal, 16)
                }
            }
        }
    }

    private var header: some View {
        HStack(alignment: .center, spacing: 12) {
            Group {
                if page == .main {
                    // Collapsed, the row is pinned to the capsule's width so
                    // the morph hand-off is pixel-exact. Expanded, the title
                    // runs the full panel width. The width flips INSTANTLY at
                    // the expansion boundary (transaction strips the spring):
                    // the long title is simply revealed by the growing
                    // surface, and the collapse starts from the short title —
                    // truncation never re-flows mid-animation.
                    GaryxThreadRuntimeCompactRow(maxWidth: nil)
                        .frame(
                            width: isExpanded ? nil : compactRowWidth,
                            alignment: .leading
                        )
                        .transaction { $0.animation = nil }
                        .transition(.identity)
                } else {
                    // No back control: the whole header row — title and its
                    // trailing empty space — pops back to the main page, and
                    // picking an option pops too. The chevron is only a hint.
                    Button {
                        setPage(.main)
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: "chevron.left")
                                .font(GaryxFont.fixedSystem(size: 12, weight: .semibold))
                                .foregroundStyle(.secondary)

                            Text(page.title)
                                .font(GaryxFont.callout(weight: .medium))
                                .foregroundStyle(.primary)
                                .garyxReadingLineLimit()

                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 16)
                        .frame(minHeight: 44)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(GaryxPressableRowStyle())
                    // VoiceOver keeps the visible page title (with header
                    // semantics); going back is the action, not the label.
                    .accessibilityLabel(page.title)
                    .accessibilityHint("Back to thread settings")
                    .accessibilityAddTraits(.isHeader)
                    .transition(.identity)
                }
            }
            // The header crossfades in place — no directional slide. The
            // thread title must never move (the ±12pt micro-push on it read
            // as a jitter); direction lives in the body content only.
            .opacity(pageContentVisible ? 1 : 0)

            Spacer(minLength: 0)
        }
    }

    private var currentAgentSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Agent")
                .font(GaryxFont.footnote(weight: .semibold))
                .foregroundStyle(.secondary)
                .padding(.leading, 4)

            contentCard {
                HStack(spacing: 12) {
                    avatar(diameter: 32)

                    VStack(alignment: .leading, spacing: 2) {
                        Text(agentTitle)
                            .font(GaryxFont.callout(weight: .semibold))
                            .foregroundStyle(.primary)
                            .garyxReadingLineLimit()

                        if let subtitle = agentSubtitle {
                            Text(subtitle)
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                                .garyxReadingLineLimit()
                        }
                    }

                    Spacer(minLength: 0)
                    GaryxSelectionCheckmark(size: 16)
                }
                .padding(.horizontal, 12)
                .frame(minHeight: 56)
                .contentShape(Rectangle())
            }
        }
    }

    private var runtimeSettingsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("This thread")
                .font(GaryxFont.footnote(weight: .semibold))
                .foregroundStyle(.secondary)
                .padding(.leading, 4)
                .padding(.top, 10)

            contentCard {
                VStack(spacing: 0) {
                    settingsRow(
                        title: "Model",
                        value: actualModelLabel,
                        enabled: canSelectModel
                    ) {
                        setPage(.model)
                    }

                    if canSelectReasoningEffort {
                        Divider().padding(.leading, 16)

                        settingsRow(
                            title: "Thinking level",
                            value: actualReasoningEffortLabel,
                            enabled: true
                        ) {
                            setPage(.thinkingLevel)
                        }
                    }

                    if canSelectServiceTier {
                        Divider().padding(.leading, 16)

                        settingsRow(
                            title: "Speed",
                            value: actualServiceTierLabel,
                            enabled: true
                        ) {
                            setPage(.speed)
                        }
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
            }
        }
    }

    private func contentCard<Content: View>(
        @ViewBuilder content: () -> Content
    ) -> some View {
        let shape = RoundedRectangle(cornerRadius: 18, style: .continuous)
        return content()
            .background(Color(.secondarySystemBackground).opacity(0.64), in: shape)
            .overlay {
                shape
                    .stroke(Color.primary.opacity(0.06), lineWidth: 1)
            }
    }

    private func settingsRow(
        title: String,
        value: String,
        enabled: Bool,
        onTap: @escaping () -> Void
    ) -> some View {
        Button(action: onTap) {
            Group {
                if dynamicTypeSize.garyxUsesExpandedReadingLayout {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(title)
                            .font(GaryxFont.callout(weight: .medium))
                            .foregroundStyle(.primary)

                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text(value)
                                .font(GaryxFont.callout())
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                            Spacer(minLength: 0)
                            if enabled {
                                Image(systemName: "chevron.right")
                                    .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                                    .foregroundStyle(.tertiary)
                            }
                        }
                    }
                } else {
                    HStack(spacing: 10) {
                        Text(title)
                            .font(GaryxFont.callout(weight: .medium))
                            .foregroundStyle(.primary)

                        Spacer(minLength: 0)

                        Text(value)
                            .font(GaryxFont.callout())
                            .foregroundStyle(.secondary)
                            .garyxReadingLineLimit()

                        if enabled {
                            Image(systemName: "chevron.right")
                                .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                                .foregroundStyle(.tertiary)
                        }
                    }
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, settingsRowVerticalPadding)
            .frame(minHeight: 48)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .disabled(!enabled)
    }

    /// The model the empty "use default" row represents: the provider default,
    /// or the effective model when the provider advertises no default (e.g.
    /// `claude_code` reports `default_model: null`). `selectedModelOptionId` is
    /// resolved against the same basis, so a missing provider default still maps
    /// the running model to the default row instead of a phantom id with no row.
    private var modelDefaultBasis: String? {
        providerDefaultModel ?? effectiveModel
    }

    private var modelOptions: [(id: String, label: String)] {
        var seen = Set<String>()
        var options: [(id: String, label: String)] = []
        if let defaultModel = modelDefaultBasis,
           seen.insert("").inserted {
            options.append((id: "", label: modelLabel(defaultModel) ?? defaultModel))
            seen.insert(defaultModel)
        }
        for option in providerModels?.models ?? [] where seen.insert(option.id).inserted {
            options.append((id: option.id, label: option.label))
        }
        if let effective = effectiveModel,
           seen.insert(effective).inserted {
            options.append((id: effective, label: modelLabel(effective) ?? effective))
        }
        return options
    }

    private var selectedModelOptionId: String {
        // Reflect the model the thread actually runs (the summary row's value),
        // not just the per-thread override, so the picker checkmark agrees. The
        // default basis matches the empty row in `modelOptions`.
        GaryxThreadModelOverridePresentation.selectedOptionId(
            effective: effectiveModel,
            default: modelDefaultBasis
        )
    }

    private var reasoningEffortOptions: [(id: String, label: String)] {
        var seen = Set<String>()
        var options: [(id: String, label: String)] = []
        if let defaultEffort = defaultReasoningEffort(for: effortFilterModel),
           seen.insert("").inserted {
            options.append((id: "", label: reasoningEffortLabel(defaultEffort) ?? defaultEffort))
            seen.insert(defaultEffort)
        }
        for option in reasoningEfforts where seen.insert(option.id).inserted {
            options.append((id: option.id, label: option.label))
        }
        if let effective = effectiveReasoningEffort,
           seen.insert(effective).inserted {
            options.append((id: effective, label: reasoningEffortLabel(effective) ?? effective))
        }
        return options
    }

    private var selectedReasoningEffortOptionId: String {
        // Check the level the thread actually runs (the summary row's value), not
        // just the per-thread override, so "Max" outside no longer shows "High"
        // checked in the picker.
        GaryxThreadModelOverridePresentation.selectedOptionId(
            effective: effectiveReasoningEffort,
            default: defaultReasoningEffort(for: effortFilterModel)
        )
    }

    private var serviceTierOptions: [(id: String, label: String)] {
        let tiers = serviceTiers
        guard !tiers.isEmpty else { return [] }
        var seen = Set<String>()
        var options: [(id: String, label: String)] = [(id: "", label: "Standard")]
        seen.insert("")
        for option in tiers where seen.insert(option.id).inserted {
            options.append((id: option.id, label: option.label))
        }
        if let effective = effectiveServiceTier, seen.insert(effective).inserted {
            options.append((id: effective, label: serviceTierLabel(effective) ?? effective))
        }
        return options
    }

    private var selectedServiceTierOptionId: String {
        // No provider-default tier ("Standard" = no explicit tier), so the
        // default basis is nil: an effective tier marks its own row, otherwise
        // the empty "Standard" row is selected.
        GaryxThreadModelOverridePresentation.selectedOptionId(
            effective: effectiveServiceTier,
            default: nil
        )
    }

    private var actualServiceTierLabel: String {
        effectiveServiceTier.flatMap { serviceTierLabel($0) } ?? "Standard"
    }

    private func serviceTierLabel(_ tier: String) -> String? {
        GaryxThreadModelOverridePresentation.serviceTierLabel(
            providerModels: providerModels,
            model: effortFilterModel,
            serviceTier: tier
        )
    }

    private func selectModel(_ selected: String) async {
        let selectedModel = selected.isEmpty ? providerDefaultModel : selected
        var nextReasoningEffort: String?
        if let currentReasoning = reasoningEffortOverride,
           GaryxThreadModelOverridePresentation.sanitizedReasoningEffort(
            providerModels: providerModels,
            model: selectedModel,
            reasoningEffort: currentReasoning
           ) == nil {
            nextReasoningEffort = ""
        }
        var nextServiceTier: String?
        if let currentTier = serviceTierOverride,
           GaryxThreadModelOverridePresentation.sanitizedServiceTier(
            providerModels: providerModels,
            model: selectedModel,
            serviceTier: currentTier
           ) == nil {
            nextServiceTier = ""
        }
        await model.updateSelectedThreadRuntimeSettings(
            model: selected,
            reasoningEffort: nextReasoningEffort,
            serviceTier: nextServiceTier
        )
    }

    private func setPage(_ nextPage: Page) {
        var exiting = pager
        guard let token = exiting.begin(to: nextPage) else { return }
        withAnimation(motion.animation(.runtimeDrilldownExit)) {
            pager = exiting
        }
        Task { @MainActor in
            // 100ms exit + 50ms rest before the new page enters.
            try? await Task.sleep(for: .seconds(GaryxMotion.runtimeDrilldownSwapDelay))
            var entering = pager
            guard entering.complete(token: token, to: nextPage) else { return }
            measuredOptionsHeight = nil
            withAnimation(motion.animation(.runtimeDrilldownEnter)) {
                pager = entering
            }
        }
    }

    private var actualModelLabel: String {
        effectiveModel.flatMap { modelLabel($0) } ?? "Model"
    }

    private var actualReasoningEffortLabel: String {
        effectiveReasoningEffort.flatMap { reasoningEffortLabel($0) } ?? "Thinking level"
    }

    private var agentTitle: String {
        normalized(model.selectedThreadAgentTarget?.title)
            ?? normalized(runtime?.agentId)
            ?? normalized(runtime?.providerLabel)
            ?? "Current agent"
    }

    private var agentSubtitle: String? {
        normalized(model.selectedThreadAgentTarget?.subtitle)
            ?? normalized(runtime?.providerLabel)
            ?? normalized(providerType)
    }

    @ViewBuilder
    private func avatar(diameter: CGFloat) -> some View {
        if let target = model.selectedThreadAgentTarget {
            GaryxAgentAvatarView(
                agentId: target.id,
                avatarDataUrl: target.avatarDataUrl,
                label: target.title,
                providerType: target.providerType,
                builtIn: target.builtIn,
                diameter: diameter
            )
        } else {
            Image(systemName: "person.crop.circle")
                .font(GaryxFont.fixedSystem(size: 22, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: diameter, height: diameter)
        }
    }

    private func modelLabel(_ modelId: String) -> String? {
        GaryxThreadModelOverridePresentation.modelLabel(
            providerModels: providerModels,
            model: modelId
        )
    }

    private func reasoningEffortLabel(_ effort: String) -> String? {
        GaryxThreadModelOverridePresentation.reasoningEffortLabel(
            providerModels: providerModels,
            model: effortFilterModel,
            reasoningEffort: effort
        )
    }

    private func defaultReasoningEffort(for modelId: String?) -> String? {
        GaryxThreadModelOverridePresentation.defaultReasoningEffort(
            providerModels: providerModels,
            model: modelId
        )
    }

    private func normalized(_ value: String?) -> String? {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty else {
            return nil
        }
        return value
    }
}

/// Pressed-only highlight for menu rows on the glass surface: a faint wash
/// while touched, nothing persistent — selection is carried by the
/// checkmark alone.
private struct GaryxRuntimeMenuRowButtonStyle: ButtonStyle {
    @Environment(\.garyxMotion) private var motion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(
                Color.primary.opacity(configuration.isPressed ? 0.045 : 0),
                in: RoundedRectangle(cornerRadius: 11, style: .continuous)
            )
            .animation(motion.animation(.pressHighlight), value: configuration.isPressed)
    }
}

struct GaryxThreadBotBindingSheet: View {
    let threadId: String

    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var isApplying = false

    private var boundGroup: GaryxMobileBotGroup? {
        GaryxMobileBotGroupBuilder.selectedGroup(
            threadId: threadId,
            groups: model.mobileBotGroups
        )
    }

    private var boundBot: GaryxConfiguredBot? {
        guard let boundGroup else { return nil }
        return garyxConfiguredBot(for: boundGroup, in: model.configuredBots)
    }

    private var selectableGroups: [GaryxMobileBotGroup] {
        model.mobileBotGroups.filter {
            garyxConfiguredBot(for: $0, in: model.configuredBots) != nil
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            botBindingSheetHeader

            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    GaryxGlassPanel(cornerRadius: 28, shadowOpacity: 0.045) {
                        VStack(spacing: 0) {
                            if !selectableGroups.isEmpty || boundBot != nil {
                                botOptionRow(
                                    title: "No bot",
                                    subtitle: "Do not bind this thread to any bot",
                                    channel: boundBot?.channel ?? "",
                                    iconDataUrl: nil,
                                    systemName: "link.slash",
                                    isSelected: boundGroup == nil,
                                    usesBotLogo: false
                                ) {
                                    if let boundBot {
                                        apply {
                                            await model.unbindBot(boundBot)
                                        }
                                    } else {
                                        dismiss()
                                    }
                                }

                                if !selectableGroups.isEmpty {
                                    Divider().padding(.leading, 56)
                                }
                            }

                            if selectableGroups.isEmpty {
                                emptyState
                            } else {
                                ForEach(Array(selectableGroups.enumerated()), id: \.element.id) { index, group in
                                    if let bot = garyxConfiguredBot(for: group, in: model.configuredBots) {
                                        botOptionRow(
                                            title: group.title,
                                            subtitle: group.subtitle,
                                            channel: group.channel,
                                            iconDataUrl: group.iconDataUrl,
                                            systemName: "bubble.left.and.bubble.right",
                                            isSelected: group.id == boundGroup?.id
                                        ) {
                                            guard group.id != boundGroup?.id else {
                                                dismiss()
                                                return
                                            }
                                            apply {
                                                await model.bindBot(bot, toThreadId: threadId)
                                            }
                                        }
                                        if index < selectableGroups.count - 1 {
                                            Divider().padding(.leading, 56)
                                        }
                                    }
                                }
                            }
                        }
                        .padding(.horizontal, 10)
                        .padding(.vertical, 8)
                    }
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
                .garyxVerticalScrollContentWidth()
            }
            .scrollIndicators(.hidden)
        }
        .garyxBotBindingSheetStyle()
        .onChange(of: model.selectedThread?.id) { _, nextThreadId in
            if nextThreadId != threadId {
                dismiss()
            }
        }
        .onChange(of: model.sidebarVisible) { _, visible in
            if visible {
                dismiss()
            }
        }
        .onChange(of: model.activePanel) { _, panel in
            if panel != .chat {
                dismiss()
            }
        }
    }

    private var botBindingSheetHeader: some View {
        HStack(alignment: .center, spacing: 12) {
            Text("Thread Bot")
                .font(GaryxFont.callout(weight: .medium))
                .foregroundStyle(.primary)
                .garyxReadingLineLimit()
            Spacer(minLength: 0)
            Button {
                dismiss()
            } label: {
                GaryxCompactGlassIcon(systemName: "xmark")
            }
            .buttonStyle(GaryxPressableRowStyle())
            .accessibilityLabel("Close")
        }
        .padding(.horizontal, 22)
        .padding(.top, 22)
        .padding(.bottom, 14)
    }

    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("No bots configured")
                .font(GaryxFont.subheadline(weight: .semibold))
                .foregroundStyle(.primary)
            Text("Add a bot in Settings before binding one to this thread.")
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 14)
    }

    private func botOptionRow(
        title: String,
        subtitle: String,
        channel: String,
        iconDataUrl: String?,
        systemName: String,
        isSelected: Bool,
        usesBotLogo: Bool = true,
        role: ButtonRole? = nil,
        isDestructive: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(role: role, action: action) {
            HStack(spacing: 12) {
                if usesBotLogo {
                    GaryxChannelLogoView(
                        channel: channel,
                        label: title,
                        iconDataUrl: iconDataUrl,
                        diameter: 34
                    )
                } else {
                    Image(systemName: systemName)
                        .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                        .foregroundStyle(isDestructive ? .red : .secondary)
                        .frame(width: 34, height: 34)
                        .background(Color(.secondarySystemFill).opacity(0.72), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(isDestructive ? .red : .primary)
                        .garyxReadingLineLimit()
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .garyxReadingLineLimit()
                        .truncationMode(.tail)
                }
                Spacer(minLength: 0)
                if isSelected {
                    GaryxSelectionCheckmark(size: 12)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, minHeight: 54, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .disabled(isApplying)
        .opacity(isApplying ? 0.62 : 1)
    }

    private func apply(_ operation: @escaping () async -> Void) {
        guard !isApplying else { return }
        isApplying = true
        dismiss()
        Task {
            await operation()
            await MainActor.run {
                isApplying = false
            }
        }
    }
}

private func garyxConfiguredBot(
    for group: GaryxMobileBotGroup,
    in configuredBots: [GaryxConfiguredBot]
) -> GaryxConfiguredBot? {
    configuredBots.first {
        $0.channel.caseInsensitiveCompare(group.channel) == .orderedSame
            && $0.accountId == group.accountId
    }
}

private extension View {
    func garyxBotBindingSheetStyle() -> some View {
        self
            .background {
                Rectangle()
                    .fill(Color(.systemBackground).opacity(0.98))
                    .overlay {
                        LinearGradient(
                            colors: [
                                Color.white.opacity(0.28),
                                Color.white.opacity(0.10)
                            ],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    }
                    .ignoresSafeArea()
            }
            .presentationBackground(.clear)
            .presentationBackgroundInteraction(.enabled)
            .presentationDetents([.fraction(0.93), .large])
            .presentationDragIndicator(.hidden)
            .presentationCornerRadius(38)
    }
}
