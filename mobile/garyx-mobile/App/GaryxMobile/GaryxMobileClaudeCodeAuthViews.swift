import Foundation
import SwiftUI
import UIKit

// Claude Code sign-in UI. The provider detail sheet shows only the slim
// `GaryxClaudeCodeAuthEntryRow` (status + account summary + one full-width
// button). The button presents `GaryxClaudeCodeLoginSheet`, a dedicated guided
// flow whose five screens are driven entirely by the Core
// `GaryxClaudeCodeLoginPresentation`. These views are a dumb renderer: all step
// derivation, copy, and enablement live in GaryxMobileCore; side effects
// (start / submit / poll / reset) live on GaryxMobileModel.

// MARK: - Provider section entry

/// The compact Authentication entry rendered inside the provider detail sheet.
struct GaryxClaudeCodeAuthEntryRow: View {
    let entry: GaryxClaudeCodeAuthEntry
    let onSignIn: () -> Void

    var body: some View {
        Group {
            Section {
                GaryxFormRow(title: "Status") {
                    GaryxStatusPill(text: entry.statusText, tone: entry.tone.garyxStatusPillTone)
                }
                if let account = entry.accountText {
                    GaryxFormReadOnlyRow(title: "Account", value: account)
                }
            } header: {
                Text("Authentication")
                    .textCase(nil)
            } footer: {
                if let caption {
                    Text(caption)
                }
            }

            Section {
                signInButton
            }
        }
    }

    private var caption: String? {
        entry.isSignedIn ? entry.accountDetailText : entry.footnote
    }

    @ViewBuilder
    private var signInButton: some View {
        Button(action: onSignIn) {
            Label(entry.actionTitle, systemImage: entry.actionSymbolName)
                .fontWeight(.semibold)
                .frame(maxWidth: .infinity)
        }
    }
}

// MARK: - Guided login sheet

struct GaryxClaudeCodeLoginSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @Environment(\.garyxMotion) private var motion
    @EnvironmentObject private var model: GaryxMobileModel

    @State private var authorizationCode = ""
    @State private var options = GaryxClaudeCodeLoginOptions()
    /// Client-only flag that splits `waiting_for_code` into Authorize / Enter
    /// Code. Set once the user opens the browser or elects to enter a code.
    @State private var hasOpenedAuthorizationURL = false
    @State private var showsAdvancedOptions = false
    @FocusState private var codeFieldFocused: Bool

    private var presentation: GaryxClaudeCodeLoginPresentation {
        GaryxClaudeCodeLoginPresentation.make(
            session: model.claudeCodeAuthSession,
            usage: claudeCodeUsage,
            authorizationCode: authorizationCode,
            hasOpenedAuthorizationURL: hasOpenedAuthorizationURL
        )
    }

    private var claudeCodeUsage: GaryxProviderUsage? {
        guard let provider = GaryxModelProviderDefaults.provider(for: "claude_code") else { return nil }
        return GaryxModelProviderDefaults.usage(in: model.codingUsage, provider: provider)
    }

    var body: some View {
        VStack(spacing: 0) {
            closeBar
            ScrollView {
                VStack(spacing: 20) {
                    hero
                    if let message = presentation.message {
                        Text(message)
                            .font(GaryxFont.callout())
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    stepContent
                }
                .frame(maxWidth: 440)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 24)
                .padding(.top, 16)
                .padding(.bottom, 20)
            }
            .scrollBounceBehavior(.basedOnSize)
            .scrollDismissesKeyboard(.interactively)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(background)
        .safeAreaInset(edge: .bottom, spacing: 0) { actionBar }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .presentationCornerRadius(28)
        .presentationBackground(Color(.systemBackground))
        .animation(motion.animation(.authenticationStep), value: presentation.step)
        .onChange(of: model.claudeCodeAuthSession?.loginId) { _, _ in
            authorizationCode = ""
        }
        .onDisappear {
            // Dismissing only stops local polling; the gateway login session is
            // untouched (design risk note). Reset so a re-open starts at intro.
            model.resetClaudeCodeAuthFlow()
        }
    }

    // MARK: Chrome

    private var closeBar: some View {
        HStack {
            Spacer(minLength: 0)
            Button { dismiss() } label: {
                GaryxCompactGlassIcon(systemName: "xmark")
            }
            .buttonStyle(GaryxPressableRowStyle())
            .accessibilityLabel("Close")
        }
        .padding(.horizontal, 18)
        .padding(.top, 14)
    }

    private var background: some View {
        ZStack {
            Color(.systemBackground)
            // A soft, tone-aware glow behind the hero adds depth without a heavy
            // card. Neutral steps get a subtle gray halo; success/failure tint it.
            RadialGradient(
                colors: [toneColor.opacity(0.12), .clear],
                center: .init(x: 0.5, y: 0.08),
                startRadius: 0,
                endRadius: 340
            )
        }
        .ignoresSafeArea()
    }

    // MARK: Hero

    private var hero: some View {
        VStack(spacing: 18) {
            heroBadge
            Text(presentation.title)
                .font(GaryxFont.system(size: 26, weight: .bold))
                .foregroundStyle(.primary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.top, 16)
    }

    private var heroBadge: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 26, style: .continuous)
                .fill(toneColor.opacity(presentation.tone == .muted ? 0.10 : 0.14))
                .frame(width: 92, height: 92)
                .overlay {
                    RoundedRectangle(cornerRadius: 26, style: .continuous)
                        .stroke(toneColor.opacity(0.16), lineWidth: 1)
                }
            if presentation.step == .submitting {
                ProgressView()
                    .controlSize(.large)
                    .tint(toneColor)
            } else {
                Image(systemName: presentation.symbolName)
                    .font(GaryxFont.system(size: 38, weight: .semibold))
                    .foregroundStyle(toneColor)
                    .symbolRenderingMode(.hierarchical)
            }
        }
    }

    // MARK: Per-step content

    @ViewBuilder
    private var stepContent: some View {
        switch presentation.step {
        case .intro:
            advancedOptions
        case .authorize:
            if presentation.showsProgress {
                ProgressView()
                    .controlSize(.regular)
                    .padding(.top, 2)
            }
        case .enterCode:
            codeEntry
        case .submitting:
            EmptyView()
        case .success:
            detailCard
        case .failure:
            EmptyView()
        }
    }

    private var advancedOptions: some View {
        VStack(spacing: 12) {
            Button {
                withAnimation(motion.animation(.disclosure)) {
                    showsAdvancedOptions.toggle()
                }
            } label: {
                HStack(spacing: 6) {
                    Text("Advanced Options")
                        .font(GaryxFont.subheadline(weight: .medium))
                    Image(systemName: "chevron.right")
                        .font(GaryxFont.system(size: 12, weight: .semibold))
                        .rotationEffect(.degrees(showsAdvancedOptions ? 90 : 0))
                }
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity)
                .contentShape(Rectangle())
            }
            .buttonStyle(GaryxPressableRowStyle())

            if showsAdvancedOptions {
                VStack(alignment: .leading, spacing: 12) {
                    Text("LOGIN METHOD")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.secondary)
                    Picker("Login method", selection: $options.mode) {
                        ForEach(GaryxClaudeCodeAuthMode.allCases) { mode in
                            Text(mode.displayName).tag(mode)
                        }
                    }
                    .pickerStyle(.segmented)
                    Text(options.mode.advancedDescription)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.tertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    Color(.secondarySystemBackground),
                    in: RoundedRectangle(cornerRadius: 16, style: .continuous)
                )
                .transition(motion.transition(.disclosure, moveFrom: .top))
            }
        }
        .padding(.top, 4)
    }

    private var codeEntry: some View {
        // Single row: the field plus a trailing Paste (empty) / Clear (filled)
        // affordance. Paste is the primary path — no keyboard is forced — so the
        // pinned Submit button below never collides with a standalone control.
        HStack(spacing: 10) {
            TextField("Authorization code", text: $authorizationCode)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled(true)
                .keyboardType(.asciiCapable)
                .submitLabel(.go)
                .focused($codeFieldFocused)
                .font(GaryxFont.body())
                .onSubmit(submitCodeIfPossible)
            if authorizationCode.isEmpty {
                Button(action: pasteCode) {
                    Label("Paste", systemImage: "doc.on.clipboard")
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(GaryxTheme.link)
                }
                .buttonStyle(GaryxPressableRowStyle())
                .accessibilityLabel("Paste code from clipboard")
            } else {
                Button {
                    authorizationCode = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(GaryxFont.system(size: 18, weight: .medium))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(GaryxPressableRowStyle())
                .accessibilityLabel("Clear code")
            }
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 56)
        .background(
            Color(.secondarySystemBackground),
            in: RoundedRectangle(cornerRadius: 14, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(codeFieldFocused ? GaryxTheme.accent.opacity(0.55) : GaryxTheme.hairline, lineWidth: 1)
        }
        .padding(.top, 4)
    }

    private var detailCard: some View {
        VStack(spacing: 0) {
            ForEach(Array(presentation.detailRows.enumerated()), id: \.element.id) { index, row in
                if index > 0 {
                    Divider().padding(.leading, 16)
                }
                HStack(spacing: 12) {
                    Text(row.label)
                        .font(GaryxFont.body())
                        .foregroundStyle(.primary)
                    Spacer(minLength: 12)
                    Text(row.value)
                        .font(GaryxFont.body())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .padding(.horizontal, 16)
                .frame(minHeight: 50)
            }
        }
        .background(
            Color(.secondarySystemBackground),
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
        .padding(.top, 4)
    }

    // MARK: Bottom actions

    @ViewBuilder
    private var actionBar: some View {
        if presentation.primaryAction != nil || presentation.secondaryAction != nil {
            VStack(spacing: 8) {
                if let primary = presentation.primaryAction {
                    GaryxPrimaryCapsuleButton(
                        title: primary.title,
                        systemImage: primaryIcon
                    ) {
                        runAction(primary)
                    }
                    .disabled(!primary.isEnabled)
                    .opacity(primary.isEnabled ? 1 : 0.45)
                }
                if let secondary = presentation.secondaryAction {
                    Button {
                        runAction(secondary)
                    } label: {
                        Text(secondary.title)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .frame(maxWidth: .infinity)
                            .frame(minHeight: 44)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(GaryxPressableRowStyle())
                    .foregroundStyle(secondary.kind == .startOver ? GaryxTheme.danger : .primary)
                    .disabled(!secondary.isEnabled)
                    .opacity(secondary.isEnabled ? 1 : 0.45)
                }
            }
            .padding(.horizontal, 22)
            .padding(.top, 10)
            .padding(.bottom, 14)
            .frame(maxWidth: 480)
            .frame(maxWidth: .infinity)
        }
    }

    // MARK: Actions

    private func runAction(_ action: GaryxClaudeCodeLoginAction) {
        guard action.isEnabled else { return }
        switch action.kind {
        case .start:
            hasOpenedAuthorizationURL = false
            authorizationCode = ""
            Task { await model.startClaudeCodeAuth(options: options) }
        case .openAuthorizationURL:
            if let url = model.claudeCodeAuthSession?.authorizationURL {
                openURL(url)
            }
            hasOpenedAuthorizationURL = true
        case .enterCode:
            hasOpenedAuthorizationURL = true
        case .submitCode:
            submitCodeIfPossible()
        case .done:
            dismiss()
        case .startOver:
            model.resetClaudeCodeAuthFlow()
            authorizationCode = ""
            hasOpenedAuthorizationURL = false
        }
    }

    private func submitCodeIfPossible() {
        guard !authorizationCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        codeFieldFocused = false
        Task { await model.submitClaudeCodeAuth(code: authorizationCode) }
    }

    private func pasteCode() {
        guard let string = UIPasteboard.general.string else { return }
        authorizationCode = string.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var primaryIcon: String? {
        guard let kind = presentation.primaryAction?.kind else { return nil }
        switch kind {
        case .start:
            return presentation.step == .failure ? "arrow.clockwise" : "sparkles"
        case .openAuthorizationURL:
            return "safari"
        case .submitCode:
            return "checkmark"
        case .done:
            return "checkmark"
        case .enterCode:
            return "arrow.right"
        case .startOver:
            return "arrow.counterclockwise"
        }
    }

    private var toneColor: Color {
        presentation.tone.garyxAuthToneColor
    }
}

// MARK: - Tone mapping

extension GaryxClaudeCodeAuthPresentationTone {
    var garyxStatusPillTone: GaryxStatusPill.Tone {
        switch self {
        case .good:
            return .good
        case .warning:
            return .warning
        case .danger:
            return .danger
        case .muted:
            return .muted
        }
    }

    var garyxAuthToneColor: Color {
        switch self {
        case .good:
            return GaryxTheme.accent
        case .warning:
            return GaryxTheme.warning
        case .danger:
            return GaryxTheme.danger
        case .muted:
            return .primary
        }
    }
}
