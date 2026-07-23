import Foundation
import SwiftUI

// Model-provider settings surfaces: the provider list with inline quota and
// account state, plus the provider detail sheet with sectioned editing.
// Business rules (patch shape and usage display models) live in
// GaryxMobileCore; these views dumb-render Core models.

struct GaryxSettingsProviderContent: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var selectedProvider: GaryxModelProviderDefault?
    @State private var showsClaudeAccounts = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxSectionBlock(title: "Model Providers") {
                GaryxCompactListGroup {
                    let providers = GaryxModelProviderDefaults.providers
                    ForEach(Array(providers.enumerated()), id: \.element.id) { index, provider in
                        GaryxModelProviderOverview(
                            provider: provider,
                            catalog: model.providerModelsByType[provider.providerType],
                            settings: model.gatewaySettingsDocument,
                            usage: GaryxModelProviderDefaults.usage(
                                in: model.codingUsage,
                                provider: provider
                            ),
                            usageRefreshedAt: model.codingUsage?.refreshedAt,
                            claudeAccounts: model.claudeCodeAccounts,
                            claudeAccountsLoading: model.isLoadingClaudeCodeAccounts,
                            claudeAccountsError: model.claudeCodeAccountsError,
                            onEdit: {
                                selectedProvider = provider
                                Task { await model.loadProviderModels(providerType: provider.providerType) }
                            },
                            onManageClaudeAccounts: {
                                showsClaudeAccounts = true
                            }
                        )

                        if index < providers.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                }
            }
        }
        .task {
            async let usageRefresh: Void = model.refreshCodingUsageWidget()
            async let accountsRefresh: Void = model.loadClaudeCodeAccounts()
            _ = await (usageRefresh, accountsRefresh)
            for provider in GaryxModelProviderDefaults.providers
            where model.providerModelsByType[provider.providerType] == nil {
                await model.loadProviderModels(providerType: provider.providerType)
            }
        }
        .garyxFullScreenCover(item: $selectedProvider) { provider in
            GaryxModelProviderDefaultsSheet(provider: provider)
        }
        .garyxSheet(isPresented: $showsClaudeAccounts) {
            GaryxClaudeCodeAccountsSheet()
        }
    }
}

/// The mobile Provider card shared by every built-in model provider. Identity,
/// quota and defaults keep one visual hierarchy; Claude Code alone inserts the
/// managed-account row because the other providers do not support account
/// selection.
private struct GaryxModelProviderOverview: View {
    let provider: GaryxModelProviderDefault
    let catalog: GaryxProviderModels?
    let settings: [String: GaryxJSONValue]
    let usage: GaryxProviderUsage?
    var usageRefreshedAt: String?
    let claudeAccounts: GaryxClaudeCodeAccounts?
    let claudeAccountsLoading: Bool
    let claudeAccountsError: String?
    let onEdit: () -> Void
    let onManageClaudeAccounts: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            providerHeader
            if supportsAccountSelection {
                GaryxCompactRowDivider()
                currentAccountRow
            }
            GaryxCompactRowDivider()
            quotaRows
            GaryxCompactRowDivider()
            defaultsRow
        }
        .accessibilityIdentifier(accessibilityIdentifier)
    }

    private var providerHeader: some View {
        HStack(spacing: 10) {
            GaryxProviderAgentAvatarView(
                providerType: provider.providerType,
                diameter: 32
            )

            VStack(alignment: .leading, spacing: 2) {
                Text(providerPresentation.displayName)
                    .font(GaryxFont.headline())
                    .foregroundStyle(.primary)
                Text(providerHeaderDescription)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
            }

            Spacer(minLength: 8)

            Button(action: onEdit) {
                Label("Edit", systemImage: "pencil")
                    .font(GaryxFont.subheadline(weight: .medium))
                    .foregroundStyle(.primary)
                    .frame(minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(GaryxPressableRowStyle())
            .accessibilityHint("Edits \(providerPresentation.displayName) defaults")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
    }

    private var currentAccountRow: some View {
        Button(action: onManageClaudeAccounts) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Current account")
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                    Text(accountTitle)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(accountDetail)
                        .font(GaryxFont.caption())
                        .foregroundStyle(claudeAccountsError == nil ? Color.secondary : GaryxTheme.danger)
                        .fixedSize(horizontal: false, vertical: true)
                }

                Spacer(minLength: 8)

                HStack(spacing: 5) {
                    Text("Switch")
                        .font(GaryxFont.subheadline(weight: .medium))
                    Image(systemName: "chevron.right")
                        .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                }
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .frame(minHeight: 58)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .accessibilityIdentifier("provider.claude.current-account")
    }

    @ViewBuilder
    private var quotaRows: some View {
        if let usageDisplay {
            if usageDisplay.available, !usageDisplay.windows.isEmpty || !usageDisplay.models.isEmpty {
                VStack(spacing: 0) {
                    ForEach(Array(usageDisplay.windows.enumerated()), id: \.element.id) { index, window in
                        if index > 0 {
                            GaryxCompactRowDivider()
                        }
                        GaryxProviderQuotaConsoleRow(
                            label: window.label,
                            remainingPercent: window.remainingPercent,
                            remainingText: window.remainingText,
                            detailText: window.detailText
                        )
                    }
                    ForEach(Array(usageDisplay.models.enumerated()), id: \.element.id) { index, model in
                        if !usageDisplay.windows.isEmpty || index > 0 {
                            GaryxCompactRowDivider()
                        }
                        GaryxProviderQuotaConsoleRow(
                            label: model.title,
                            remainingPercent: model.remainingPercent,
                            remainingText: model.remainingText,
                            detailText: model.detailText
                        )
                    }
                }
                .opacity(usageDisplay.stale ? 0.55 : 1)
            } else {
                quotaUnavailable(usageDisplay.summaryText)
            }
        } else {
            quotaUnavailable(supportsAccountSelection && claudeAccountsLoading ? "Loading quota…" : "No quota data")
        }
    }

    private var defaultsRow: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text("Defaults")
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
            Spacer(minLength: 8)
            Text(rowModel.detailText)
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.trailing)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    private func quotaUnavailable(_ text: String) -> some View {
        HStack(spacing: 8) {
            Text("Remaining quota")
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
            Spacer(minLength: 8)
            if supportsAccountSelection && claudeAccountsLoading {
                ProgressView().controlSize(.small)
            }
            Text(text)
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 12)
    }

    private var selectedAccount: GaryxClaudeCodeAccountPresentation? {
        guard supportsAccountSelection,
              let claudeAccounts,
              let account = claudeAccounts.selectedAccount else { return nil }
        return GaryxClaudeCodeAccountPresentation.make(
            account: account,
            refreshedAt: claudeAccounts.refreshedAt
        )
    }

    private var accountTitle: String {
        guard let selectedAccount else {
            if claudeAccountsLoading { return "Loading…" }
            if claudeAccountsError != nil { return "Account unavailable" }
            return "System default"
        }
        return selectedAccount.title
    }

    private var accountDetail: String {
        if let selectedAccount {
            return selectedAccount.detailText
        }
        return claudeAccountsError ?? "Uses this Mac's default Claude Code login"
    }

    private var usageDisplay: GaryxProviderUsageDisplayModel? {
        selectedAccount?.usage ?? GaryxProviderUsageDisplayModel.make(
            from: usage,
            refreshedAt: usageRefreshedAt
        )
    }

    private var providerPresentation: GaryxProviderPresentation {
        GaryxProviderPresentation.make(providerType: provider.providerType)
    }

    private var rowModel: GaryxProviderSettingsPresentation.RowModel {
        .make(provider: provider, catalog: catalog, settings: settings)
    }

    private var providerHeaderDescription: String {
        var parts = [rowModel.providerDescription]
        if let plan = usageDisplay?.plan {
            parts.append(plan)
        }
        if usageDisplay?.stale == true {
            parts.append("stale")
        }
        return parts.joined(separator: " · ")
    }

    private var supportsAccountSelection: Bool {
        GaryxProviderSettingsPresentation.authSection(for: provider) == .claudeCode
    }

    private var accessibilityIdentifier: String {
        supportsAccountSelection
            ? "provider.claude.overview"
            : "provider.\(provider.providerType).overview"
    }
}

private struct GaryxProviderQuotaConsoleRow: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let label: String
    let remainingPercent: Double
    let remainingText: String
    let detailText: String
    var horizontalPadding: CGFloat = 12

    init(
        label: String,
        remainingPercent: Double,
        remainingText: String,
        detailText: String,
        horizontalPadding: CGFloat = 12
    ) {
        self.label = label
        self.remainingPercent = remainingPercent
        self.remainingText = remainingText
        self.detailText = detailText
        self.horizontalPadding = horizontalPadding
    }

    init(window: GaryxProviderUsageWindowDisplayModel, horizontalPadding: CGFloat = 12) {
        self.init(
            label: window.label,
            remainingPercent: window.remainingPercent,
            remainingText: window.remainingText,
            detailText: window.detailText,
            horizontalPadding: horizontalPadding
        )
    }

    var body: some View {
        Group {
            if dynamicTypeSize.garyxUsesExpandedReadingLayout {
                stackedLayout
            } else {
                ViewThatFits(in: .horizontal) {
                    compactLayout
                    stackedLayout
                }
            }
        }
        .padding(.horizontal, horizontalPadding)
        .padding(.vertical, 9)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(label), \(remainingText), \(detailText)")
    }

    private var compactLayout: some View {
        HStack(spacing: 12) {
            labelView
                .fixedSize(horizontal: true, vertical: true)
                .frame(minWidth: 108, alignment: .leading)
            track
                .frame(minWidth: 80)
            percent.frame(minWidth: 42, alignment: .trailing)
        }
    }

    private var stackedLayout: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(label)
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 8)
                percent
            }
            if !detailText.isEmpty {
                Text(detailText)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            track
        }
    }

    private var labelView: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label)
                .font(GaryxFont.subheadline(weight: .semibold))
                .foregroundStyle(.primary)
            if !detailText.isEmpty {
                Text(detailText)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var track: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.primary.opacity(0.10))
                Capsule()
                    .fill(Color.primary.opacity(0.82))
                    .frame(width: proxy.size.width * max(0, min(remainingPercent, 100)) / 100)
            }
        }
        .frame(maxWidth: .infinity)
        .frame(height: 5)
    }

    private var percent: some View {
        Text(remainingText)
            .font(GaryxFont.subheadline(weight: .semibold))
            .foregroundStyle(.primary)
            .monospacedDigit()
    }
}

private struct GaryxClaudeCodeAccountFlow: Identifiable {
    enum Kind {
        case add
        case login(GaryxClaudeCodeAuthTarget)
    }

    let id = UUID()
    let kind: Kind
}

struct GaryxClaudeCodeAccountsSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var accountFlow: GaryxClaudeCodeAccountFlow?
    var selectionOnly = false
    var onSelection: ((GaryxClaudeCodeAccountSelection) -> Void)?

    var body: some View {
        NavigationStack {
            List {
                if let error = model.claudeCodeAccountsError {
                    Section {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(error)
                                .font(GaryxFont.callout())
                                .foregroundStyle(GaryxTheme.danger)
                                .fixedSize(horizontal: false, vertical: true)
                            Button("Try Again") {
                                Task { await model.loadClaudeCodeAccounts() }
                            }
                            .fontWeight(.semibold)
                            .foregroundStyle(.primary)
                        }
                        .padding(.vertical, 3)
                    }
                }

                Section {
                    if accountRows.isEmpty, model.isLoadingClaudeCodeAccounts {
                        HStack(spacing: 10) {
                            ProgressView().controlSize(.small)
                            Text("Loading accounts…")
                                .foregroundStyle(.secondary)
                        }
                    } else {
                        ForEach(accountRows) { account in
                            if selectionOnly {
                                Button {
                                    selectImmediately(account)
                                } label: {
                                    GaryxClaudeCodeAccountRow(account: account)
                                }
                                .buttonStyle(.plain)
                                // A selected account is a harmless no-op, not
                                // an unavailable account. Keep its quota at
                                // full contrast instead of inheriting SwiftUI's
                                // disabled-row opacity.
                                .disabled(model.isMutatingClaudeCodeAccount)
                            } else {
                                NavigationLink {
                                    GaryxClaudeCodeAccountDetailView(accountStableId: account.id)
                                } label: {
                                    GaryxClaudeCodeAccountRow(account: account)
                                }
                                .disabled(model.isMutatingClaudeCodeAccount)
                            }
                        }
                    }
                } header: {
                    Text("Claude Code accounts")
                        .textCase(nil)
                } footer: {
                    Text(selectionOnly
                         ? "Choosing an account resumes every Claude thread paused by quota. Active runs continue unchanged."
                         : "The selected account applies to new and restarted Claude runs. Active runs continue unchanged.")
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle(selectionOnly ? "Switch account" : "Claude Code")
            .navigationBarTitleDisplayMode(.inline)
            .refreshable {
                await model.loadClaudeCodeAccounts()
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .foregroundStyle(.primary)
                }
                ToolbarItem(placement: .primaryAction) {
                    if !selectionOnly {
                        Button {
                            accountFlow = GaryxClaudeCodeAccountFlow(kind: .add)
                        } label: {
                            Image(systemName: "plus")
                                .foregroundStyle(.primary)
                        }
                        .disabled(model.isMutatingClaudeCodeAccount)
                        .accessibilityLabel("Add Claude Code account")
                    }
                }
            }
        }
        .tint(GaryxTheme.controlTint)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .garyxFullScreenCover(item: $accountFlow, onDismiss: refreshAfterAccountFlow) { flow in
            switch flow.kind {
            case .add:
                GaryxClaudeCodeAddAccountFlow()
            case .login(let target):
                GaryxClaudeCodeLoginSheet(target: target)
            }
        }
        .task {
            if model.claudeCodeAccounts == nil {
                await model.loadClaudeCodeAccounts()
            }
        }
    }

    private var accountRows: [GaryxClaudeCodeAccountPresentation] {
        guard let accounts = model.claudeCodeAccounts else { return [] }
        return accounts.accounts.map {
            GaryxClaudeCodeAccountPresentation.make(
                account: $0,
                refreshedAt: accounts.refreshedAt
            )
        }
    }

    private func refreshAfterAccountFlow() {
        Task {
            await model.loadClaudeCodeAccounts()
            await model.refreshCodingUsageWidget()
        }
    }

    private func selectImmediately(_ account: GaryxClaudeCodeAccountPresentation) {
        guard !account.selected, !model.isMutatingClaudeCodeAccount else { return }
        Task {
            if let result = await model.selectClaudeCodeAccount(accountId: account.accountId) {
                onSelection?(result)
                dismiss()
            }
        }
    }
}

private struct GaryxClaudeCodeAccountRow: View {
    let account: GaryxClaudeCodeAccountPresentation

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(alignment: .top, spacing: 10) {
                Group {
                    if account.selected {
                        GaryxSelectionCheckmark(style: .circle, size: 17)
                    } else {
                        Color.clear.frame(width: 17, height: 17)
                    }
                }
                .frame(width: 20, height: 20)

                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(account.title)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .fixedSize(horizontal: false, vertical: true)
                        if let plan = account.planText {
                            Text(plan)
                                .font(GaryxFont.caption(weight: .medium))
                                .foregroundStyle(.secondary)
                        }
                    }
                    Text(account.detailText)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            accountQuota
                .padding(.leading, 30)
        }
        .padding(.vertical, 5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .accessibilityLabel(accessibilityLabel)
    }

    @ViewBuilder
    private var accountQuota: some View {
        if let usage = account.usage, usage.available, !usage.windows.isEmpty {
            VStack(alignment: .leading, spacing: 7) {
                ForEach(usage.windows) { window in
                    GaryxProviderQuotaConsoleRow(window: window, horizontalPadding: 0)
                }
            }
            .opacity(usage.stale ? 0.55 : 1)
        } else {
            Text(account.usage?.summaryText ?? "No quota data")
                .font(GaryxFont.caption())
                .foregroundStyle(.tertiary)
        }
    }

    private var accessibilityLabel: String {
        let state = account.selected ? "selected" : "not selected"
        return "\(account.title), \(account.detailText), \(state)"
    }
}

private struct GaryxClaudeCodeAccountDetailView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let accountStableId: String

    @State private var accountFlow: GaryxClaudeCodeAccountFlow?
    @State private var renameAccount: GaryxClaudeCodeAccountPresentation?
    @State private var deleteAccount: GaryxClaudeCodeAccountPresentation?

    var body: some View {
        Group {
            if let account {
                List {
                    identitySection(account)
                    quotaSection(account)
                    if let error = model.claudeCodeAccountsError {
                        accountErrorSection(error)
                    }
                    actionsSection(account)
                    if !account.systemDefault {
                        deleteSection(account)
                    }
                }
                .listStyle(.insetGrouped)
                .refreshable {
                    await model.loadClaudeCodeAccounts()
                }
            } else if model.isLoadingClaudeCodeAccounts {
                ProgressView("Loading account…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ContentUnavailableView(
                    "Account Unavailable",
                    systemImage: "person.crop.circle.badge.questionmark",
                    description: Text("This Claude Code account may have been removed.")
                )
            }
        }
        .navigationTitle("Account")
        .navigationBarTitleDisplayMode(.inline)
        .tint(GaryxTheme.controlTint)
        .garyxFullScreenCover(item: $accountFlow, onDismiss: refreshAfterAuthentication) { flow in
            switch flow.kind {
            case .add:
                GaryxClaudeCodeAddAccountFlow()
            case .login(let target):
                GaryxClaudeCodeLoginSheet(target: target)
            }
        }
        .garyxSheet(item: $renameAccount) { account in
            GaryxClaudeCodeRenameAccountSheet(account: account)
        }
        .garyxAlert(item: $deleteAccount) { account in
            Alert(
                title: Text("Delete \(account.title)?"),
                message: Text("Garyx will remove this managed Claude Code login. Active runs continue, and future runs use System default if this account is selected."),
                primaryButton: .destructive(Text("Delete")) {
                    delete(account)
                },
                secondaryButton: .cancel()
            )
        }
    }

    private func accountErrorSection(_ error: String) -> some View {
        Section {
            Label {
                Text(error)
                    .fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "exclamationmark.circle")
            }
            .font(GaryxFont.callout())
            .foregroundStyle(GaryxTheme.danger)

            Button("Refresh Accounts") {
                Task { await model.loadClaudeCodeAccounts() }
            }
            .fontWeight(.semibold)
            .foregroundStyle(.primary)
        }
    }

    private func identitySection(_ account: GaryxClaudeCodeAccountPresentation) -> some View {
        Section {
            VStack(alignment: .leading, spacing: 5) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(account.title)
                        .font(GaryxFont.headline())
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let plan = account.planText {
                        Text(plan)
                            .font(GaryxFont.subheadline(weight: .medium))
                            .foregroundStyle(.secondary)
                    }
                }
                Text(account.detailText)
                    .font(GaryxFont.subheadline())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                if account.selected {
                    Label("Current account", systemImage: "checkmark.circle.fill")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.primary)
                        .padding(.top, 3)
                }
            }
            .padding(.vertical, 5)
        }
    }

    @ViewBuilder
    private func quotaSection(_ account: GaryxClaudeCodeAccountPresentation) -> some View {
        Section {
            if let usage = account.usage, usage.available, !usage.windows.isEmpty {
                ForEach(usage.windows) { window in
                    GaryxProviderQuotaConsoleRow(window: window, horizontalPadding: 0)
                        .listRowInsets(
                            EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16)
                        )
                }
                .opacity(usage.stale ? 0.55 : 1)
            } else {
                Text(account.usage?.summaryText ?? "No quota data")
                    .foregroundStyle(.secondary)
            }
        } header: {
            Text("Quota")
                .textCase(nil)
        }
    }

    private func actionsSection(_ account: GaryxClaudeCodeAccountPresentation) -> some View {
        Section {
            if !account.selected {
                Button {
                    select(account)
                } label: {
                    Label("Use This Account", systemImage: "checkmark.circle")
                }
                .disabled(model.isMutatingClaudeCodeAccount)
            }

            Button {
                reauthenticate(account)
            } label: {
                Label("Re-authenticate", systemImage: "arrow.triangle.2.circlepath")
            }

            if !account.systemDefault {
                Button {
                    renameAccount = account
                } label: {
                    Label("Rename", systemImage: "pencil")
                }
            }
        }
        .foregroundStyle(.primary)
    }

    private func deleteSection(_ account: GaryxClaudeCodeAccountPresentation) -> some View {
        Section {
            Button(role: .destructive) {
                deleteAccount = account
            } label: {
                Label("Delete Account", systemImage: "trash")
            }
            .disabled(model.isMutatingClaudeCodeAccount)
        }
    }

    private var account: GaryxClaudeCodeAccountPresentation? {
        guard let accounts = model.claudeCodeAccounts,
              let account = accounts.accounts.first(where: { $0.stableId == accountStableId })
        else { return nil }
        return GaryxClaudeCodeAccountPresentation.make(
            account: account,
            refreshedAt: accounts.refreshedAt
        )
    }

    private func select(_ account: GaryxClaudeCodeAccountPresentation) {
        guard !account.selected, !model.isMutatingClaudeCodeAccount else { return }
        Task {
            if await model.selectClaudeCodeAccount(accountId: account.accountId) != nil {
                dismiss()
            }
        }
    }

    private func reauthenticate(_ account: GaryxClaudeCodeAccountPresentation) {
        let target: GaryxClaudeCodeAuthTarget
        if let accountId = account.accountId {
            target = .managedAccount(id: accountId, name: account.title)
        } else {
            target = .systemDefault
        }
        accountFlow = GaryxClaudeCodeAccountFlow(kind: .login(target))
    }

    private func delete(_ account: GaryxClaudeCodeAccountPresentation) {
        guard let accountId = account.accountId else { return }
        Task {
            if await model.deleteClaudeCodeAccount(accountId: accountId) {
                dismiss()
            }
        }
    }

    private func refreshAfterAuthentication() {
        Task {
            await model.loadClaudeCodeAccounts()
            await model.refreshCodingUsageWidget()
        }
    }
}

private struct GaryxClaudeCodeAddAccountFlow: View {
    @Environment(\.dismiss) private var dismiss
    @State private var accountName = ""
    @State private var loginTarget: GaryxClaudeCodeAuthTarget?
    @FocusState private var nameFocused: Bool

    var body: some View {
        Group {
            if let loginTarget {
                GaryxClaudeCodeLoginSheet(target: loginTarget)
            } else {
                GaryxFormSheet(
                    title: "Add Claude Account",
                    canSave: canContinue,
                    saveTitle: "Continue",
                    onCancel: { dismiss() },
                    onSave: continueToLogin
                ) {
                    Section {
                        TextField("Account name", text: $accountName)
                            .textInputAutocapitalization(.words)
                            .autocorrectionDisabled()
                            .focused($nameFocused)
                            .submitLabel(.continue)
                            .onSubmit {
                                if canContinue { continueToLogin() }
                            }
                    } header: {
                        Text("Name")
                            .textCase(nil)
                    } footer: {
                        Text("Use a short name such as Work or Personal. Garyx keeps this login isolated from System default.")
                    }
                }
                .onAppear { nameFocused = true }
            }
        }
    }

    private var trimmedName: String {
        accountName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canContinue: Bool { !trimmedName.isEmpty }

    private func continueToLogin() {
        guard canContinue else { return }
        nameFocused = false
        loginTarget = .newManagedAccount(name: trimmedName)
    }
}

private struct GaryxClaudeCodeRenameAccountSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let account: GaryxClaudeCodeAccountPresentation
    @State private var name: String
    @State private var isSaving = false
    @FocusState private var nameFocused: Bool

    init(account: GaryxClaudeCodeAccountPresentation) {
        self.account = account
        _name = State(initialValue: account.title)
    }

    var body: some View {
        GaryxFormSheet(
            title: "Rename Account",
            canSave: canSave,
            isSaving: isSaving,
            onSave: save
        ) {
            if let error = model.claudeCodeAccountsError {
                Section {
                    Label {
                        Text(error)
                            .fixedSize(horizontal: false, vertical: true)
                    } icon: {
                        Image(systemName: "exclamationmark.circle")
                    }
                    .font(GaryxFont.callout())
                    .foregroundStyle(GaryxTheme.danger)
                }
            }

            Section {
                TextField("Account name", text: $name)
                    .textInputAutocapitalization(.words)
                    .autocorrectionDisabled()
                    .focused($nameFocused)
                    .submitLabel(.done)
                    .onSubmit {
                        if canSave { save() }
                    }
            } header: {
                Text("Name")
                    .textCase(nil)
            }
        }
        .onAppear { nameFocused = true }
    }

    private var trimmedName: String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canSave: Bool {
        !isSaving && !trimmedName.isEmpty && trimmedName != account.title
    }

    private func save() {
        guard canSave, let accountId = account.accountId else { return }
        isSaving = true
        Task {
            let didSave = await model.renameClaudeCodeAccount(accountId: accountId, name: trimmedName)
            isSaving = false
            if didSave { dismiss() }
        }
    }
}

// MARK: - Shared §4 usage visualization

extension GaryxUsageLevel {
    /// Provider settings keep every available quota meter monochrome. Severity
    /// still lives in Core for accessibility/copy, but it does not introduce a
    /// second visual color language on this page.
    var garyxTint: Color {
        switch self {
        case .healthy, .warning, .critical:
            return Color.primary.opacity(0.82)
        case .unavailable:
            return Color.secondary
        }
    }
}

/// One labelled remaining-quota meter: `label ····· 73%` over a fill track
/// with a `resets in 2d 4h` caption. Used in the detail sheet's Usage section.
private struct GaryxUsageMeterRow: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .caption) private var readingSpacing: CGFloat = 3
    @ScaledMetric(relativeTo: .caption) private var trackScale: CGFloat = 1
    let label: String
    let remainingPercent: Double
    let remainingText: String
    let caption: String
    let level: GaryxUsageLevel
    var compact = false

    var body: some View {
        VStack(alignment: .leading, spacing: readingSpacing) {
            if dynamicTypeSize.garyxUsesExpandedReadingLayout {
                VStack(alignment: .leading, spacing: readingSpacing) {
                    meterLabel
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        remainingLabel
                        captionLabel
                    }
                }
            } else {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    meterLabel
                    Spacer(minLength: 6)
                    remainingLabel
                    captionLabel
                }
            }
            GeometryReader { proxy in
                ZStack(alignment: .leading) {
                    Capsule()
                        .fill(Color.primary.opacity(0.08))
                    Capsule()
                        .fill(level.garyxTint)
                        .frame(width: max(0, proxy.size.width * remainingPercent / 100))
                }
            }
            .frame(height: (compact ? 4 : 5) * trackScale)
        }
    }

    private var meterLabel: some View {
        Text(label)
            .font(GaryxFont.caption(weight: compact ? .regular : .medium))
            .foregroundStyle(.secondary)
            .garyxReadingLineLimit()
    }

    private var remainingLabel: some View {
        Text(remainingText)
            .font(GaryxFont.caption(weight: .semibold))
            .foregroundStyle(.primary)
            .garyxReadingLineLimit()
    }

    @ViewBuilder
    private var captionLabel: some View {
        if !caption.isEmpty {
            Text(caption)
                .font(GaryxFont.caption())
                .foregroundStyle(.tertiary)
                .garyxReadingLineLimit()
        }
    }
}

private struct GaryxUsagePillsRow: View {
    let display: GaryxProviderUsageDisplayModel
    var showsUpdated = false

    var body: some View {
        if display.plan != nil || display.stale || (showsUpdated && display.updatedText != nil) {
            HStack(spacing: 6) {
                if let plan = display.plan {
                    GaryxStatusPill(text: plan, tone: .good)
                }
                if display.stale {
                    GaryxStatusPill(text: "stale", tone: .warning)
                }
                if showsUpdated, let updated = display.updatedText {
                    Text(updated)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.tertiary)
                        .garyxReadingLineLimit()
                }
            }
        }
    }
}

// MARK: - Provider detail sheet

struct GaryxModelProviderDefaultsSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let provider: GaryxModelProviderDefault
    @State private var modelName = ""
    @State private var reasoningEffort = ""
    @State private var serviceTier = ""
    @State private var showsClaudeAccountsSheet = false
    @State private var isHydrated = false
    @State private var hydrationFailed = false
    @State private var isSaving = false

    var body: some View {
        GaryxFormSheet(
            title: "\(providerPresentation.displayName) Defaults",
            canSave: isHydrated && !isSaving,
            onCancel: closeSheet,
            onSave: saveDefaults
        ) {
            Group {
                if hydrationFailed {
                    Section {
                        GaryxFormErrorText(text: "Couldn't load the current provider settings from the gateway, so editing is disabled to avoid overwriting newer values.")
                        Button {
                            Task { await hydrate() }
                        } label: {
                            Text("Retry")
                                .fontWeight(.semibold)
                                .frame(maxWidth: .infinity)
                        }
                    }
                }

                GaryxFormGroupedSection(title: "Provider") {
                    GaryxFormReadOnlyRow(title: "Name", value: providerPresentation.displayName)
                    GaryxFormReadOnlyRow(title: "Type", value: provider.providerType)
                }

                if provider.usageProviderId != nil {
                    GaryxProviderUsageFormSection(
                        usageDisplay: GaryxProviderUsageDisplayModel.make(
                            from: GaryxModelProviderDefaults.usage(in: model.codingUsage, provider: provider),
                            refreshedAt: model.codingUsage?.refreshedAt
                        )
                    )
                }

                GaryxFormGroupedSection(title: "Defaults") {
                    GaryxProviderDefaultPickerRow(
                        title: "Model",
                        value: $modelName,
                        placeholder: defaultModelLabel,
                        options: modelOptions,
                        iconName: "cpu"
                    )
                    if !reasoningOptions.isEmpty {
                        GaryxProviderDefaultPickerRow(
                            title: "Thinking level",
                            value: $reasoningEffort,
                            placeholder: "Provider default",
                            options: reasoningOptions,
                            iconName: "brain"
                        )
                    }
                    if supportsServiceTier {
                        GaryxProviderDefaultPickerRow(
                            title: "Speed",
                            value: $serviceTier,
                            placeholder: "Standard",
                            options: serviceTierOptions,
                            iconName: "gauge.with.needle",
                            emptyOptionLabel: "Standard"
                        )
                    }
                }

                authenticationSection

                hostRuntimeSection
            }
        }
        .task { await hydrate() }
        .garyxSheet(isPresented: $showsClaudeAccountsSheet) {
            GaryxClaudeCodeAccountsSheet()
        }
        .onDisappear {
            if authSection == .claudeCode {
                model.resetClaudeCodeAuthFlow()
            }
        }
        .onChange(of: modelName) { _, _ in
            reasoningEffort = GaryxThreadModelOverridePresentation.sanitizedReasoningEffort(
                providerModels: catalog,
                model: modelName,
                reasoningEffort: reasoningEffort
            ) ?? ""
        }
    }

    // MARK: Sections

    @ViewBuilder
    private var authenticationSection: some View {
        switch authSection {
        case .claudeCode:
            Section {
                Button {
                    showsClaudeAccountsSheet = true
                } label: {
                    HStack(spacing: 12) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(claudeSelectedAccountTitle)
                                .font(GaryxFont.body(weight: .medium))
                                .foregroundStyle(.primary)
                                .fixedSize(horizontal: false, vertical: true)
                            Text(claudeSelectedAccountDetail)
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer(minLength: 8)
                        Image(systemName: "chevron.right")
                            .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                            .foregroundStyle(.tertiary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(GaryxPressableRowStyle())
            } header: {
                Text("Account")
                    .textCase(nil)
            } footer: {
                Text("Manage Claude Code logins and choose the account used by future runs.")
            }
        case .managedOAuth:
            GaryxFormGroupedSection(title: "Authentication") {
                GaryxFormReadOnlyRow(title: "OAuth", value: "Managed on the Mac app")
            }
        case .managedCLI:
            GaryxFormGroupedSection(title: "Authentication") {
                GaryxFormReadOnlyRow(title: "Grok CLI", value: "Managed by grok on the gateway host")
            }
        }
    }

    @ViewBuilder
    private var hostRuntimeSection: some View {
        let fields = GaryxModelProviderDefaults.hostRuntimeFields(
            in: model.gatewaySettingsDocument,
            provider: provider
        )
        if !fields.isEmpty {
            Section {
                ForEach(fields) { field in
                    if field.value.contains("\n") {
                        GaryxFormReadOnlyMultilineRow(
                            title: field.label,
                            value: field.value,
                            valuePlacement: .below
                        )
                    } else {
                        GaryxFormReadOnlyRow(title: field.label, value: field.value)
                    }
                }
            } header: {
                Text("CLI Runtime")
                    .textCase(nil)
            } footer: {
                Text("Gateway-host runtime settings. Managed on the Mac app.")
            }
        }
    }

    // MARK: State

    private var providerPresentation: GaryxProviderPresentation {
        GaryxProviderPresentation.make(providerType: provider.providerType)
    }

    private var catalog: GaryxProviderModels? {
        model.providerModelsByType[provider.providerType]
    }

    private var modelOptions: [GaryxProviderModelOption] {
        catalog?.models ?? []
    }

    private var reasoningOptions: [GaryxProviderModelOption] {
        GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: catalog,
            model: modelName
        )
    }

    private var authSection: GaryxProviderSettingsPresentation.AuthSection {
        GaryxProviderSettingsPresentation.authSection(for: provider)
    }

    private var supportsServiceTier: Bool {
        GaryxProviderSettingsPresentation.supportsServiceTier(provider: provider, catalog: catalog)
    }

    private var serviceTierOptions: [GaryxProviderModelOption] {
        catalog?.serviceTiers ?? []
    }

    private var claudeSelectedAccount: GaryxClaudeCodeAccountPresentation? {
        guard let accounts = model.claudeCodeAccounts,
              let account = accounts.selectedAccount else { return nil }
        return GaryxClaudeCodeAccountPresentation.make(
            account: account,
            refreshedAt: accounts.refreshedAt
        )
    }

    private var claudeSelectedAccountTitle: String {
        guard let account = claudeSelectedAccount else { return "System default" }
        guard let plan = account.planText else { return account.title }
        return "\(account.title) · \(plan)"
    }

    private var claudeSelectedAccountDetail: String {
        claudeSelectedAccount?.detailText ?? "Uses this Mac's default Claude Code login"
    }

    private var defaultModelLabel: String {
        GaryxProviderSettingsPresentation.defaultModelLabel(provider: provider, catalog: catalog)
    }

    /// Loads the authoritative settings document before echoing values (D1 /
    /// §6.2). Hydration gates editing: on failure nothing is echoed and Save
    /// stays disabled so a stale restored projection can never be written back.
    private func hydrate() async {
        async let catalogLoad: Void = model.loadProviderModels(providerType: provider.providerType)
        if provider.providerType == "claude_code" {
            await model.loadClaudeCodeAccounts()
        }
        let fetched = await model.refreshAuthoritativeGatewaySettings()
        _ = await catalogLoad
        if fetched {
            fillDraft()
            isHydrated = true
            hydrationFailed = false
        } else if !isHydrated {
            hydrationFailed = true
        }
    }

    private func fillDraft() {
        let draft = GaryxProviderSettingsPresentation.Draft.make(
            settings: model.gatewaySettingsDocument,
            provider: provider
        )
        modelName = draft.modelName
        reasoningEffort = draft.reasoningEffort
        serviceTier = draft.serviceTier
    }

    private func saveDefaults() {
        guard !isSaving, isHydrated else { return }
        isSaving = true
        Task {
            let didSave = await model.updateModelProviderDefaults(
                provider: provider,
                request: GaryxProviderSettingsPresentation.SaveRequest.make(
                    provider: provider,
                    catalog: catalog,
                    modelName: modelName,
                    reasoningEffort: reasoningEffort,
                    serviceTier: serviceTier
                )
            )
            await MainActor.run {
                isSaving = false
                if didSave {
                    dismiss()
                }
            }
        }
    }

    private func closeSheet() {
        if authSection == .claudeCode {
            model.resetClaudeCodeAuthFlow()
        }
        dismiss()
    }
}

/// The detail sheet's Usage section: full §4 treatment — plan pill, stale tag,
/// freshness line, session/weekly/scoped meters, or all Antigravity buckets.
private struct GaryxProviderUsageFormSection: View {
    let usageDisplay: GaryxProviderUsageDisplayModel?

    var body: some View {
        GaryxFormGroupedSection(title: "Usage") {
            VStack(alignment: .leading, spacing: 10) {
                if let usageDisplay {
                    GaryxUsagePillsRow(display: usageDisplay, showsUpdated: true)
                    if !usageDisplay.available {
                        Text(usageDisplay.summaryText)
                            .font(GaryxFont.body())
                            .foregroundStyle(.secondary)
                        Text(usageDisplay.detailText)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.tertiary)
                    } else if usageDisplay.windows.isEmpty && usageDisplay.models.isEmpty {
                        Text("No quota data")
                            .font(GaryxFont.body())
                            .foregroundStyle(.secondary)
                    } else {
                        VStack(alignment: .leading, spacing: 9) {
                            ForEach(usageDisplay.windows) { window in
                                GaryxUsageMeterRow(
                                    label: window.label,
                                    remainingPercent: window.remainingPercent,
                                    remainingText: window.remainingText,
                                    caption: window.detailText,
                                    level: window.level
                                )
                            }
                            ForEach(usageDisplay.models) { modelRow in
                                GaryxUsageMeterRow(
                                    label: modelRow.title,
                                    remainingPercent: modelRow.remainingPercent,
                                    remainingText: modelRow.remainingText,
                                    caption: modelRow.detailText,
                                    level: modelRow.level
                                )
                            }
                        }
                        .opacity(usageDisplay.stale ? 0.55 : 1)
                    }
                } else {
                    Text("No quota data")
                        .font(GaryxFont.body())
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.vertical, 4)
        }
    }
}

private struct GaryxProviderDefaultPickerRow: View {
    let title: String
    @Binding var value: String
    let placeholder: String
    let options: [GaryxProviderModelOption]
    let iconName: String
    var emptyOptionLabel = "Provider default"

    var body: some View {
        GaryxFormMenuRow(title: title) {
            Button(emptyOptionLabel) {
                value = ""
            }
            if !options.isEmpty {
                Divider()
            }
            ForEach(options, id: \.id) { option in
                Button(optionTitle(option)) {
                    value = option.id
                }
            }
        } valueLabel: {
            HStack(spacing: 6) {
                Text(selectedLabel)
                    .garyxReadingLineLimit()
                    .truncationMode(.middle)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.fixedSystem(size: 10, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .foregroundStyle(.primary)
        }
        .disabled(options.isEmpty && normalizedValue.isEmpty)
    }

    private var normalizedValue: String {
        value.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var selectedLabel: String {
        guard !normalizedValue.isEmpty else { return placeholder }
        return options.first(where: { $0.id == normalizedValue })?.label ?? normalizedValue
    }

    private func optionTitle(_ option: GaryxProviderModelOption) -> String {
        option.recommended ? "\(option.label) · Recommended" : option.label
    }
}
