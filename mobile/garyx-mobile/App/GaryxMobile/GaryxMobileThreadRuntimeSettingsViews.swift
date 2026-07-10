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

enum GaryxThreadRuntimeMorph {
    /// Dynamic-island style: a quick spring with a hint of bounce on open,
    /// a tighter settle on close.
    static let openAnimation = Animation.spring(response: 0.42, dampingFraction: 0.76)
    static let closeAnimation = Animation.spring(response: 0.32, dampingFraction: 0.92)
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

    private var title: String {
        model.selectedThread?.title ?? model.draftThreadTitle
    }

    var body: some View {
        HStack(spacing: 8) {
            avatar(diameter: 22)

            Text(title)
                .font(GaryxFont.callout(weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.tail)
                .layoutPriority(1)
        }
        .padding(.horizontal, 12)
        .frame(height: 44, alignment: .leading)
        .frame(maxWidth: 282, alignment: .leading)
    }

    @ViewBuilder
    private func avatar(diameter: CGFloat) -> some View {
        if let target = model.selectedThreadAgentTarget {
            GaryxAgentAvatarView(
                agentId: target.id,
                avatarDataUrl: target.avatarDataUrl,
                kind: target.kind,
                label: target.title,
                providerType: target.providerType,
                builtIn: target.builtIn,
                diameter: diameter
            )
        } else {
            Image(systemName: "person.crop.circle")
                .font(GaryxFont.system(size: diameter * 0.72, weight: .semibold))
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
    let isExpanded: Bool
    let anchorRect: CGRect
    let containerSize: CGSize
    let onClose: () -> Void

    var body: some View {
        let expandedWidth = min(
            containerSize.width - GaryxThreadRuntimeMorph.horizontalMargin * 2,
            GaryxThreadRuntimeMorph.maxExpandedWidth
        )
        let expandedX = (containerSize.width - expandedWidth) / 2
        let cornerRadius = isExpanded
            ? GaryxThreadRuntimeMorph.expandedCornerRadius
            : GaryxThreadRuntimeMorph.collapsedCornerRadius
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)

        GaryxThreadRuntimeSettingsPanel(
            compactRowWidth: anchorRect.width,
            isExpanded: isExpanded
        )
        // Inner frame keeps the panel laid out at its final width the whole
        // time, so text never reflows while the surface window grows.
        .frame(width: expandedWidth, alignment: .topLeading)
        .frame(
            width: isExpanded ? expandedWidth : anchorRect.width,
            height: isExpanded ? nil : anchorRect.height,
            alignment: .topLeading
        )
        // Readability backing fades in with the expansion; the collapsed
        // capsule stays pure glass like the top-bar original.
        .background {
            shape.fill(Color(.systemBackground).opacity(isExpanded ? 0.72 : 0))
        }
        .garyxAdaptiveGlass(
            .regular,
            isInteractive: false,
            fallbackMaterial: .ultraThinMaterial,
            in: shape
        )
        .clipShape(shape)
        .overlay {
            shape
                .stroke(Color.white.opacity(0.30), lineWidth: 0.7)
                .opacity(isExpanded ? 1 : 0)
        }
        .overlay {
            shape
                .stroke(Color.primary.opacity(0.06), lineWidth: 1)
                .opacity(isExpanded ? 1 : 0)
        }
        .shadow(
            color: Color.black.opacity(isExpanded ? 0.10 : 0),
            radius: 24, x: 0, y: 10
        )
        .offset(
            x: isExpanded ? expandedX : anchorRect.minX,
            y: anchorRect.minY
        )
        .accessibilityAddTraits(.isModal)
        .accessibilityAction(.escape, onClose)
    }
}

struct GaryxThreadRuntimeSettingsPanel: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

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

    @State private var page = Page.main

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
                } else {
                    ScrollView {
                        optionsPage
                            .padding(.horizontal, 14)
                            .padding(.bottom, 14)
                            .garyxVerticalScrollContentWidth()
                    }
                    .frame(height: optionsPageHeight)
                    .scrollIndicators(.hidden)
                }
            }
            .opacity(isExpanded ? 1 : 0)
        }
        .task(id: providerType) {
            guard !providerType.isEmpty,
                  model.providerModelsByType[providerType] == nil else {
                return
            }
            await model.loadProviderModels(providerType: providerType)
        }
    }

    private var panelMaxHeight: CGFloat {
        min(UIScreen.main.bounds.height * 0.62, 520)
    }

    private var optionsPageHeight: CGFloat {
        min(max(CGFloat(optionsPageCount) * 52 + 28, 96), panelMaxHeight)
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
            optionsCard {
                GaryxAgentSheetOptionsPanel(
                    options: modelOptions,
                    selectedId: selectedModelOptionId
                ) { selected in
                    setPage(.main)
                    Task {
                        await selectModel(selected)
                    }
                }
            }
        case .thinkingLevel:
            optionsCard {
                GaryxAgentSheetOptionsPanel(
                    options: reasoningEffortOptions,
                    selectedId: selectedReasoningEffortOptionId
                ) { selected in
                    setPage(.main)
                    Task {
                        await model.updateSelectedThreadRuntimeSettings(reasoningEffort: selected)
                    }
                }
            }
        case .speed:
            optionsCard {
                GaryxAgentSheetOptionsPanel(
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
    }

    private var header: some View {
        HStack(alignment: .center, spacing: 12) {
            if page == .main {
                // The exact view that renders inside the top-bar capsule,
                // pinned to the capsule's width: the morph never moves or
                // re-truncates the title.
                GaryxThreadRuntimeCompactRow()
                    .frame(width: compactRowWidth, alignment: .leading)
            } else {
                Button {
                    setPage(.main)
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "chevron.left")
                            .font(GaryxFont.system(size: 13, weight: .semibold))
                            .foregroundStyle(.secondary)
                        Text(page.title)
                            .font(GaryxFont.callout(weight: .medium))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                    }
                    .padding(.horizontal, 12)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .frame(height: 44)
            }

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
                            .lineLimit(1)

                        if let subtitle = agentSubtitle {
                            Text(subtitle)
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
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

    private func optionsCard<Content: View>(
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        contentCard {
            content()
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
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
            HStack(spacing: 10) {
                Text(title)
                    .font(GaryxFont.callout(weight: .medium))
                    .foregroundStyle(.primary)

                Spacer(minLength: 0)

                Text(value)
                    .font(GaryxFont.callout())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)

                if enabled {
                    Image(systemName: "chevron.right")
                        .font(GaryxFont.system(size: 11, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(.horizontal, 8)
            .frame(minHeight: 48)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
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
        guard page != nextPage else { return }
        if reduceMotion {
            page = nextPage
        } else {
            withAnimation(.easeOut(duration: 0.18)) {
                page = nextPage
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
                kind: target.kind,
                label: target.title,
                providerType: target.providerType,
                builtIn: target.builtIn,
                diameter: diameter
            )
        } else {
            Image(systemName: "person.crop.circle")
                .font(GaryxFont.system(size: 22, weight: .semibold))
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
                    GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
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
                .lineLimit(1)
            Spacer(minLength: 0)
            Button {
                dismiss()
            } label: {
                GaryxCompactGlassIcon(systemName: "xmark")
            }
            .buttonStyle(.plain)
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
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(isDestructive ? .red : .secondary)
                        .frame(width: 34, height: 34)
                        .background(Color(.secondarySystemFill).opacity(0.72), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(isDestructive ? .red : .primary)
                        .lineLimit(1)
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
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
        .buttonStyle(.plain)
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
