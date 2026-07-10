import Foundation
import SwiftUI
import UIKit

enum GaryxThreadRuntimeMorphID {
    static let surface = "thread-runtime-surface"
    static let avatar = "thread-runtime-avatar"
    static let title = "thread-runtime-title"
}

struct GaryxThreadRuntimeSettingsPanel: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    let width: CGFloat
    let morphNamespace: Namespace.ID
    let onClose: () -> Void

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
    @State private var showsPanelBody = false

    private var selectedThread: GaryxThreadSummary? { model.selectedThread }
    private var runtime: GaryxThreadRuntimeSummary? { selectedThread?.threadRuntime }

    private var threadTitle: String {
        normalized(selectedThread?.title)
            ?? normalized(model.draftThreadTitle)
            ?? "Thread"
    }

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
        let shape = RoundedRectangle(cornerRadius: 28, style: .continuous)

        VStack(spacing: 0) {
            header

            if page == .main {
                VStack(alignment: .leading, spacing: 12) {
                    currentAgentSection
                    runtimeSettingsSection
                }
                .padding(.horizontal, 14)
                .padding(.bottom, 14)
                .fixedSize(horizontal: false, vertical: true)
                .opacity(showsPanelBody ? 1 : 0)
            } else {
                ScrollView {
                    optionsPage
                        .padding(.horizontal, 14)
                        .padding(.bottom, 14)
                        .garyxVerticalScrollContentWidth()
                }
                .frame(height: optionsPageHeight)
                .scrollIndicators(.hidden)
                .opacity(showsPanelBody ? 1 : 0)
            }
        }
        .frame(width: width)
        .background {
            shape
                .fill(Color.clear)
                .garyxAdaptiveGlass(
                    .regular,
                    isInteractive: false,
                    tint: Color(.systemBackground).opacity(0.72),
                    fallbackMaterial: .ultraThinMaterial,
                    in: shape
                )
                .matchedGeometryEffect(
                    id: GaryxThreadRuntimeMorphID.surface,
                    in: morphNamespace
                )
        }
        .clipShape(shape)
        .overlay {
            shape
                .stroke(Color.white.opacity(0.30), lineWidth: 0.7)
        }
        .overlay {
            shape
                .stroke(Color.primary.opacity(0.06), lineWidth: 1)
        }
        .shadow(color: Color.black.opacity(0.10), radius: 24, x: 0, y: 10)
        .padding(1)
        .accessibilityAddTraits(.isModal)
        .accessibilityAction(.escape, onClose)
        .onAppear {
            if reduceMotion {
                showsPanelBody = true
            } else {
                withAnimation(.easeOut(duration: 0.18).delay(0.09)) {
                    showsPanelBody = true
                }
            }
        }
        .task(id: providerType) {
            guard !providerType.isEmpty,
                  model.providerModelsByType[providerType] == nil else {
                return
            }
            await model.loadProviderModels(providerType: providerType)
        }
        .onChange(of: selectedThread?.id) { _, _ in
            onClose()
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
                avatar(diameter: 26)
                    .matchedGeometryEffect(
                        id: GaryxThreadRuntimeMorphID.avatar,
                        in: morphNamespace
                    )

                VStack(alignment: .leading, spacing: 2) {
                    Text(threadTitle)
                        .font(GaryxFont.callout(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .matchedGeometryEffect(
                            id: GaryxThreadRuntimeMorphID.title,
                            in: morphNamespace
                        )
                    Text("Agent & model")
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
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
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }

            Spacer(minLength: 0)
        }
        .overlay(alignment: .trailing) {
            Button {
                onClose()
            } label: {
                Image(systemName: "xmark")
                    .font(GaryxFont.system(size: 13, weight: .semibold))
                    .foregroundStyle(.primary)
                    .frame(width: 44, height: 44)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Close")
        }
        .padding(.horizontal, 16)
        .padding(.top, 12)
        .padding(.bottom, 10)
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
