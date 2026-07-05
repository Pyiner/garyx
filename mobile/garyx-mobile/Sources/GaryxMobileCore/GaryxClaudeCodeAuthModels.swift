import Foundation

// Claude Code sign-in models. The gateway HTTP contract (start / submit / get)
// is unchanged; iOS drives it through a dedicated guided login sheet. The email
// field was removed from the start request entirely so iOS can never send it —
// gateway still accepts it, we simply omit it (default body `{"mode":"claudeai"}`).
// `console` and `sso` remain available as advanced-only options via the existing
// wire fields.

public enum GaryxClaudeCodeAuthMode: String, Codable, CaseIterable, Identifiable, Sendable {
    case claudeai
    case console

    public var id: String { rawValue }

    /// Short label used in the advanced login-method control.
    public var displayName: String {
        switch self {
        case .claudeai:
            return "Claude.ai"
        case .console:
            return "Console"
        }
    }

    /// One-line explanation shown under the advanced login-method control.
    public var advancedDescription: String {
        switch self {
        case .claudeai:
            return "Sign in with your Claude.ai subscription."
        case .console:
            return "Sign in with Anthropic Console (API billing)."
        }
    }
}

public enum GaryxClaudeCodeAuthStatus: String, Codable, Equatable, Sendable {
    case starting
    case waitingForCode = "waiting_for_code"
    case submitted
    case succeeded
    case failed

    public var isTerminal: Bool {
        self == .succeeded || self == .failed
    }
}

public struct GaryxClaudeCodeAuthStartRequest: Encodable, Equatable, Sendable {
    public var mode: GaryxClaudeCodeAuthMode
    public var sso: Bool

    public init(
        mode: GaryxClaudeCodeAuthMode = .claudeai,
        sso: Bool = false
    ) {
        self.mode = mode
        self.sso = sso
    }

    enum CodingKeys: String, CodingKey {
        case mode
        case sso
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(mode, forKey: .mode)
        // Only send `sso` when enabled; the default flow is a bare
        // `{"mode":"claudeai"}` with no email and no sso key.
        if sso {
            try container.encode(sso, forKey: .sso)
        }
    }
}

public struct GaryxClaudeCodeAuthSubmitRequest: Encodable, Equatable, Sendable {
    public var code: String

    public init(code: String) {
        self.code = code.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// The advanced options a user can adjust before starting sign-in. Defaults to a
/// one-tap Claude.ai login; `console` / `sso` are opt-in via Advanced Options.
public struct GaryxClaudeCodeLoginOptions: Equatable, Sendable {
    public var mode: GaryxClaudeCodeAuthMode
    public var useSSO: Bool

    public init(
        mode: GaryxClaudeCodeAuthMode = .claudeai,
        useSSO: Bool = false
    ) {
        self.mode = mode
        self.useSSO = useSSO
    }

    /// True when nothing is customized (default Claude.ai, no SSO). Used by the
    /// UI to keep the Advanced Options disclosure collapsed by default.
    public var isDefault: Bool {
        mode == .claudeai && !useSSO
    }

    /// The wire request for these options. Never carries an email.
    public var startRequest: GaryxClaudeCodeAuthStartRequest {
        GaryxClaudeCodeAuthStartRequest(mode: mode, sso: useSSO)
    }
}

public struct GaryxClaudeCodeAuthSession: Codable, Equatable, Sendable {
    public var loginId: String
    public var status: GaryxClaudeCodeAuthStatus
    public var url: String?
    public var authStatus: GaryxJSONValue?
    public var error: String?
    public var exitCode: Int?

    public init(
        loginId: String,
        status: GaryxClaudeCodeAuthStatus,
        url: String? = nil,
        authStatus: GaryxJSONValue? = nil,
        error: String? = nil,
        exitCode: Int? = nil
    ) {
        self.loginId = loginId
        self.status = status
        self.url = url?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.authStatus = authStatus
        self.error = error?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.exitCode = exitCode
    }

    enum CodingKeys: String, CodingKey {
        case loginId = "login_id"
        case status
        case url
        case authStatus = "auth_status"
        case error
        case exitCode = "exit_code"
    }

    public var authorizationURL: URL? {
        guard let url else { return nil }
        return URL(string: url)
    }
}

public struct GaryxClaudeCodeAuthAccount: Equatable, Sendable {
    public var loggedIn: Bool
    public var orgName: String?
    public var plan: String?
    public var email: String?
    public var authMethod: String?
    public var apiProvider: String?

    public init(
        loggedIn: Bool,
        orgName: String? = nil,
        plan: String? = nil,
        email: String? = nil,
        authMethod: String? = nil,
        apiProvider: String? = nil
    ) {
        self.loggedIn = loggedIn
        self.orgName = orgName?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.plan = plan?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.email = email?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.authMethod = authMethod?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.apiProvider = apiProvider?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
    }

    public static func make(
        authStatus: GaryxJSONValue?,
        usage: GaryxProviderUsage?
    ) -> GaryxClaudeCodeAuthAccount {
        var loggedIn = usage?.available == true
        var orgName: String?
        var plan = usage?.plan?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        var email: String?
        var authMethod: String?
        var apiProvider: String?

        if case .object(let object)? = authStatus?.garyxGatewayJSONStringDecodedIfNeeded {
            if let statusLoggedIn = object.garyxClaudeCodeAuthBoolValue(forKeys: ["loggedIn", "logged_in"]) {
                loggedIn = statusLoggedIn
            }
            orgName = object.garyxGatewayStringValue(forKeys: [
                "orgName",
                "org_name",
                "organizationName",
                "organization_name",
            ])
            email = object.garyxGatewayStringValue(forKeys: ["email"])
            plan = object.garyxGatewayStringValue(forKeys: [
                "subscriptionType",
                "subscription_type",
                "plan",
            ]) ?? plan
            authMethod = object.garyxGatewayStringValue(forKeys: ["authMethod", "auth_method"])
            apiProvider = object.garyxGatewayStringValue(forKeys: ["apiProvider", "api_provider"])
        }

        return GaryxClaudeCodeAuthAccount(
            loggedIn: loggedIn,
            orgName: orgName,
            plan: plan,
            email: email,
            authMethod: authMethod,
            apiProvider: apiProvider
        )
    }

    public var displayName: String? {
        orgName ?? email ?? plan
    }

    public var detailText: String? {
        let parts = [plan, authMethod, apiProvider]
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty }
        guard !parts.isEmpty else { return nil }
        return parts.joined(separator: " · ")
    }
}

public enum GaryxClaudeCodeAuthPresentationTone: Equatable, Sendable {
    case good
    case warning
    case danger
    case muted
}

// MARK: - Provider section entry

/// The slim entry shown inside the provider detail's Authentication section: a
/// status pill, an optional account summary, and one full-width button that
/// presents the guided login sheet. All step-by-step fields moved to the sheet.
public struct GaryxClaudeCodeAuthEntry: Equatable, Sendable {
    public var statusText: String
    public var tone: GaryxClaudeCodeAuthPresentationTone
    public var isSignedIn: Bool
    public var accountText: String?
    public var accountDetailText: String?
    public var actionTitle: String
    public var actionSymbolName: String
    /// A short helper line shown when signed out (e.g. a usage error).
    public var footnote: String?

    public init(
        statusText: String,
        tone: GaryxClaudeCodeAuthPresentationTone,
        isSignedIn: Bool,
        accountText: String? = nil,
        accountDetailText: String? = nil,
        actionTitle: String,
        actionSymbolName: String,
        footnote: String? = nil
    ) {
        self.statusText = statusText
        self.tone = tone
        self.isSignedIn = isSignedIn
        self.accountText = accountText?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.accountDetailText = accountDetailText?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.actionTitle = actionTitle
        self.actionSymbolName = actionSymbolName
        self.footnote = footnote?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
    }

    public static func make(
        session: GaryxClaudeCodeAuthSession?,
        usage: GaryxProviderUsage?
    ) -> GaryxClaudeCodeAuthEntry {
        let account = GaryxClaudeCodeAuthAccount.make(authStatus: session?.authStatus, usage: usage)
        if account.loggedIn {
            return GaryxClaudeCodeAuthEntry(
                statusText: "Signed in",
                tone: .good,
                isSignedIn: true,
                accountText: account.displayName,
                accountDetailText: account.detailText,
                actionTitle: "Re-authenticate",
                actionSymbolName: "arrow.triangle.2.circlepath",
                footnote: nil
            )
        }
        return GaryxClaudeCodeAuthEntry(
            statusText: "Not signed in",
            tone: .muted,
            isSignedIn: false,
            accountText: nil,
            accountDetailText: nil,
            actionTitle: "Sign in with Claude",
            actionSymbolName: "sparkles",
            footnote: usage?.error
        )
    }
}

// MARK: - Guided login sheet

/// One screen of the guided login sheet. `waiting_for_code` is split into two
/// client-only steps (`.authorize` / `.enterCode`) here in Core so the app never
/// re-derives the sub-step; the gateway state machine is unchanged.
public enum GaryxClaudeCodeLoginStep: Equatable, Sendable {
    case intro
    case authorize
    case enterCode
    case submitting
    case success
    case failure
}

public enum GaryxClaudeCodeLoginActionKind: Equatable, Sendable {
    /// Begin (or restart) a login: POST auth/start with the chosen options.
    case start
    /// Open the authorization URL in the browser and advance to code entry.
    case openAuthorizationURL
    /// Advance to code entry without opening the browser ("I already have a code").
    case enterCode
    /// Submit the pasted authorization code.
    case submitCode
    /// Dismiss the sheet after a successful sign-in.
    case done
    /// Discard the current login session and return to the intro screen.
    case startOver
}

public struct GaryxClaudeCodeLoginAction: Equatable, Sendable {
    public var kind: GaryxClaudeCodeLoginActionKind
    public var title: String
    public var isEnabled: Bool

    public init(
        _ kind: GaryxClaudeCodeLoginActionKind,
        title: String,
        isEnabled: Bool = true
    ) {
        self.kind = kind
        self.title = title
        self.isEnabled = isEnabled
    }
}

/// A labelled account attribute shown on the success screen.
public struct GaryxClaudeCodeLoginDetailRow: Equatable, Sendable, Identifiable {
    public var label: String
    public var value: String

    public init(label: String, value: String) {
        self.label = label
        self.value = value
    }

    public var id: String { label }
}

public struct GaryxClaudeCodeLoginPresentation: Equatable, Sendable {
    public var step: GaryxClaudeCodeLoginStep
    public var symbolName: String
    public var title: String
    public var message: String?
    public var tone: GaryxClaudeCodeAuthPresentationTone
    public var showsProgress: Bool
    public var showsCodeField: Bool
    public var detailRows: [GaryxClaudeCodeLoginDetailRow]
    public var primaryAction: GaryxClaudeCodeLoginAction?
    public var secondaryAction: GaryxClaudeCodeLoginAction?

    public init(
        step: GaryxClaudeCodeLoginStep,
        symbolName: String,
        title: String,
        message: String? = nil,
        tone: GaryxClaudeCodeAuthPresentationTone,
        showsProgress: Bool = false,
        showsCodeField: Bool = false,
        detailRows: [GaryxClaudeCodeLoginDetailRow] = [],
        primaryAction: GaryxClaudeCodeLoginAction? = nil,
        secondaryAction: GaryxClaudeCodeLoginAction? = nil
    ) {
        self.step = step
        self.symbolName = symbolName
        self.title = title
        self.message = message?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.tone = tone
        self.showsProgress = showsProgress
        self.showsCodeField = showsCodeField
        self.detailRows = detailRows
        self.primaryAction = primaryAction
        self.secondaryAction = secondaryAction
    }

    /// Derives the current login step. `waiting_for_code` resolves to `.authorize`
    /// until the client reports it has opened (or skipped to) code entry, then
    /// `.enterCode`. This is the single source of the sub-step split (design §
    /// state-machine mapping).
    public static func step(
        for status: GaryxClaudeCodeAuthStatus?,
        hasOpenedAuthorizationURL: Bool
    ) -> GaryxClaudeCodeLoginStep {
        switch status {
        case .none:
            return .intro
        case .starting:
            return .authorize
        case .waitingForCode:
            return hasOpenedAuthorizationURL ? .enterCode : .authorize
        case .submitted:
            return .submitting
        case .succeeded:
            return .success
        case .failed:
            return .failure
        }
    }

    public static func make(
        session: GaryxClaudeCodeAuthSession?,
        usage: GaryxProviderUsage?,
        authorizationCode: String = "",
        hasOpenedAuthorizationURL: Bool = false
    ) -> GaryxClaudeCodeLoginPresentation {
        let hasCode = !authorizationCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let urlReady = session?.authorizationURL != nil
        let step = step(for: session?.status, hasOpenedAuthorizationURL: hasOpenedAuthorizationURL)

        switch step {
        case .intro:
            return GaryxClaudeCodeLoginPresentation(
                step: .intro,
                symbolName: "sparkles",
                title: "Sign in to Claude Code",
                message: "Authorize Garyx in your browser, then paste the code back here to finish signing in.",
                tone: .muted,
                primaryAction: GaryxClaudeCodeLoginAction(.start, title: "Sign in with Claude")
            )

        case .authorize:
            let preparing = session?.status == .starting || !urlReady
            return GaryxClaudeCodeLoginPresentation(
                step: .authorize,
                symbolName: "globe",
                title: "Authorize in Browser",
                message: preparing
                    ? "Preparing your secure authorization link…"
                    : "Open the Claude authorization page, approve access, then come back with your code.",
                tone: .muted,
                showsProgress: preparing,
                primaryAction: GaryxClaudeCodeLoginAction(
                    .openAuthorizationURL,
                    title: preparing ? "Preparing…" : "Open Claude",
                    isEnabled: urlReady
                ),
                secondaryAction: urlReady
                    ? GaryxClaudeCodeLoginAction(.enterCode, title: "I already have a code")
                    : nil
            )

        case .enterCode:
            return GaryxClaudeCodeLoginPresentation(
                step: .enterCode,
                symbolName: "doc.on.clipboard",
                title: "Enter Authorization Code",
                message: "Paste the code Claude gave you after you approved access.",
                tone: .muted,
                showsCodeField: true,
                primaryAction: GaryxClaudeCodeLoginAction(.submitCode, title: "Submit Code", isEnabled: hasCode),
                secondaryAction: GaryxClaudeCodeLoginAction(
                    .openAuthorizationURL,
                    title: "Open Claude Again",
                    isEnabled: urlReady
                )
            )

        case .submitting:
            return GaryxClaudeCodeLoginPresentation(
                step: .submitting,
                symbolName: "hourglass",
                title: "Finishing Sign-In",
                message: "Completing sign-in on your gateway host…",
                tone: .warning,
                showsProgress: true
            )

        case .success:
            let account = GaryxClaudeCodeAuthAccount.make(authStatus: session?.authStatus, usage: usage)
            return GaryxClaudeCodeLoginPresentation(
                step: .success,
                symbolName: "checkmark.circle.fill",
                title: "Signed In",
                message: "You're signed in and ready to use Claude Code.",
                tone: .good,
                detailRows: successRows(account: account),
                primaryAction: GaryxClaudeCodeLoginAction(.done, title: "Done")
            )

        case .failure:
            return GaryxClaudeCodeLoginPresentation(
                step: .failure,
                symbolName: "exclamationmark.triangle.fill",
                title: "Sign-In Failed",
                message: session?.error ?? "Claude Code sign-in didn't complete. Please try again.",
                tone: .danger,
                primaryAction: GaryxClaudeCodeLoginAction(.start, title: "Try Again"),
                secondaryAction: GaryxClaudeCodeLoginAction(.startOver, title: "Start Over")
            )
        }
    }

    private static func successRows(account: GaryxClaudeCodeAuthAccount) -> [GaryxClaudeCodeLoginDetailRow] {
        var rows: [GaryxClaudeCodeLoginDetailRow] = []
        rows.append(
            GaryxClaudeCodeLoginDetailRow(
                label: "Account",
                value: account.orgName ?? account.email ?? "Claude account"
            )
        )
        if let plan = account.plan {
            rows.append(GaryxClaudeCodeLoginDetailRow(label: "Plan", value: plan))
        }
        if let authMethod = account.authMethod {
            rows.append(GaryxClaudeCodeLoginDetailRow(label: "Method", value: authMethod))
        }
        if let apiProvider = account.apiProvider {
            rows.append(GaryxClaudeCodeLoginDetailRow(label: "API", value: apiProvider))
        }
        return rows
    }
}

private extension Dictionary where Key == String, Value == GaryxJSONValue {
    func garyxClaudeCodeAuthBoolValue(forKeys keys: [String]) -> Bool? {
        for key in keys {
            switch self[key] {
            case .bool(let value):
                return value
            case .string(let value):
                switch value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
                case "true", "yes", "1":
                    return true
                case "false", "no", "0":
                    return false
                default:
                    continue
                }
            default:
                continue
            }
        }
        return nil
    }
}
