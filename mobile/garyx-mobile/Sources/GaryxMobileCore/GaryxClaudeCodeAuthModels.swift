import Foundation

// Claude Code account + sign-in models. Account selection belongs to the
// provider; the client never receives a config path and never snapshots an
// account onto a thread. iOS drives login through its existing guided sheet,
// now with an explicit system/new-managed/existing-managed target.

public struct GaryxClaudeCodeAccounts: Codable, Equatable, Sendable {
    public var activeAccountId: String?
    public var accounts: [GaryxClaudeCodeAccount]
    public var refreshedAt: String

    public init(
        activeAccountId: String? = nil,
        accounts: [GaryxClaudeCodeAccount],
        refreshedAt: String
    ) {
        self.activeAccountId = activeAccountId?.trimmingCharacters(in: .whitespacesAndNewlines)
            .garyxGatewayTrimmedNilIfEmpty
        self.accounts = accounts
        self.refreshedAt = refreshedAt
    }

    enum CodingKeys: String, CodingKey {
        case activeAccountId = "active_account_id"
        case accounts
        case refreshedAt = "refreshed_at"
    }

    public var selectedAccount: GaryxClaudeCodeAccount? {
        accounts.first(where: \.selected)
    }
}

public struct GaryxClaudeCodeAccount: Codable, Equatable, Sendable {
    public var id: String?
    public var name: String
    public var systemDefault: Bool
    public var selected: Bool
    public var email: String?
    public var organization: String?
    public var plan: String?
    public var authMethod: String?
    public var usage: GaryxProviderUsage

    public init(
        id: String? = nil,
        name: String,
        systemDefault: Bool,
        selected: Bool,
        email: String? = nil,
        organization: String? = nil,
        plan: String? = nil,
        authMethod: String? = nil,
        usage: GaryxProviderUsage
    ) {
        self.id = id?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        self.systemDefault = systemDefault
        self.selected = selected
        self.email = email?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.organization = organization?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.plan = plan?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.authMethod = authMethod?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.usage = usage
    }

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case systemDefault = "system_default"
        case selected
        case email
        case organization
        case plan
        case authMethod = "auth_method"
        case usage
    }

    public var stableId: String { id ?? "system-default" }
}

/// Pure account-row projection shared by the Provider overview and account
/// switcher. SwiftUI only composes these values and dispatches actions.
public struct GaryxClaudeCodeAccountPresentation: Equatable, Identifiable, Sendable {
    public var id: String
    public var accountId: String?
    public var title: String
    public var detailText: String
    public var planText: String?
    public var systemDefault: Bool
    public var selected: Bool
    public var usage: GaryxProviderUsageDisplayModel?

    public static func make(
        account: GaryxClaudeCodeAccount,
        refreshedAt: String?,
        now: Date = Date()
    ) -> GaryxClaudeCodeAccountPresentation {
        let detail: String
        if let email = account.email {
            detail = email
        } else if let organization = account.organization {
            detail = organization
        } else if account.systemDefault {
            detail = "This Mac's default Claude Code login"
        } else {
            detail = "Managed Claude Code login"
        }
        return GaryxClaudeCodeAccountPresentation(
            id: account.stableId,
            accountId: account.id,
            title: account.name,
            detailText: detail,
            planText: account.plan ?? account.usage.plan,
            systemDefault: account.systemDefault,
            selected: account.selected,
            usage: GaryxProviderUsageDisplayModel.make(
                from: account.usage,
                refreshedAt: refreshedAt,
                now: now
            )
        )
    }
}

public struct GaryxClaudeCodeAccountSelectionRequest: Encodable, Equatable, Sendable {
    public var accountId: String?

    public init(accountId: String?) {
        self.accountId = accountId?.trimmingCharacters(in: .whitespacesAndNewlines)
            .garyxGatewayTrimmedNilIfEmpty
    }

    enum CodingKeys: String, CodingKey { case accountId = "account_id" }
}

public struct GaryxQuotaRecoverySummary: Codable, Equatable, Sendable {
    public var matchedThreads: Int
    public var expeditedThreads: Int
    public var alreadyClaimedThreads: Int

    public init(
        matchedThreads: Int = 0,
        expeditedThreads: Int = 0,
        alreadyClaimedThreads: Int = 0
    ) {
        self.matchedThreads = matchedThreads
        self.expeditedThreads = expeditedThreads
        self.alreadyClaimedThreads = alreadyClaimedThreads
    }

    enum CodingKeys: String, CodingKey {
        case matchedThreads = "matched_threads"
        case expeditedThreads = "expedited_threads"
        case alreadyClaimedThreads = "already_claimed_threads"
    }
}

public struct GaryxClaudeCodeAccountSelection: Codable, Equatable, Sendable {
    public var activeAccountId: String?
    public var selectionChanged: Bool
    public var recovery: GaryxQuotaRecoverySummary
    public var recoveryWarning: String?

    public init(
        activeAccountId: String? = nil,
        selectionChanged: Bool = true,
        recovery: GaryxQuotaRecoverySummary = GaryxQuotaRecoverySummary(),
        recoveryWarning: String? = nil
    ) {
        self.activeAccountId = activeAccountId
        self.selectionChanged = selectionChanged
        self.recovery = recovery
        self.recoveryWarning = recoveryWarning
    }

    enum CodingKeys: String, CodingKey {
        case activeAccountId = "active_account_id"
        case selectionChanged = "selection_changed"
        case recovery
        case recoveryWarning = "recovery_warning"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        activeAccountId = try container.decodeIfPresent(String.self, forKey: .activeAccountId)
        selectionChanged = try container.decodeIfPresent(Bool.self, forKey: .selectionChanged) ?? true
        recovery = try container.decodeIfPresent(
            GaryxQuotaRecoverySummary.self,
            forKey: .recovery
        ) ?? GaryxQuotaRecoverySummary()
        recoveryWarning = try container.decodeIfPresent(String.self, forKey: .recoveryWarning)
    }
}

public struct GaryxClaudeCodeAccountRenameRequest: Encodable, Equatable, Sendable {
    public var name: String

    public init(name: String) {
        self.name = name.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

public enum GaryxClaudeCodeAuthTarget: Equatable, Sendable {
    case systemDefault
    case newManagedAccount(name: String)
    case managedAccount(id: String, name: String)

    public var displayName: String {
        switch self {
        case .systemDefault:
            return "System default"
        case .newManagedAccount(let name), .managedAccount(_, let name):
            return name.trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }

    public var accountId: String? {
        guard case .managedAccount(let id, _) = self else { return nil }
        return id.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
    }

    public var managedAccountName: String? {
        guard case .newManagedAccount(let name) = self else { return nil }
        return name.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
    }
}

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
    public var managedAccountName: String?
    public var accountId: String?

    public init(
        mode: GaryxClaudeCodeAuthMode = .claudeai,
        sso: Bool = false,
        managedAccountName: String? = nil,
        accountId: String? = nil
    ) {
        self.mode = mode
        self.sso = sso
        self.managedAccountName = managedAccountName?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .garyxGatewayTrimmedNilIfEmpty
        self.accountId = accountId?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .garyxGatewayTrimmedNilIfEmpty
    }

    enum CodingKeys: String, CodingKey {
        case mode
        case sso
        case managedAccountName = "managed_account_name"
        case accountId = "account_id"
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(mode, forKey: .mode)
        // Only send `sso` when enabled; the default flow is a bare
        // `{"mode":"claudeai"}` with no email and no sso key.
        if sso {
            try container.encode(sso, forKey: .sso)
        }
        try container.encodeIfPresent(managedAccountName, forKey: .managedAccountName)
        try container.encodeIfPresent(accountId, forKey: .accountId)
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
        makeStartRequest(target: .systemDefault)
    }

    public func makeStartRequest(target: GaryxClaudeCodeAuthTarget) -> GaryxClaudeCodeAuthStartRequest {
        GaryxClaudeCodeAuthStartRequest(
            mode: mode,
            sso: useSSO,
            managedAccountName: target.managedAccountName,
            accountId: target.accountId
        )
    }
}

public struct GaryxClaudeCodeAuthSession: Codable, Equatable, Sendable {
    public var loginId: String
    public var accountId: String?
    public var status: GaryxClaudeCodeAuthStatus
    public var url: String?
    public var authStatus: GaryxJSONValue?
    public var error: String?
    public var exitCode: Int?

    public init(
        loginId: String,
        accountId: String? = nil,
        status: GaryxClaudeCodeAuthStatus,
        url: String? = nil,
        authStatus: GaryxJSONValue? = nil,
        error: String? = nil,
        exitCode: Int? = nil
    ) {
        self.loginId = loginId
        self.accountId = accountId?.trimmingCharacters(in: .whitespacesAndNewlines)
            .garyxGatewayTrimmedNilIfEmpty
        self.status = status
        self.url = url?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.authStatus = authStatus
        self.error = error?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.exitCode = exitCode
    }

    enum CodingKeys: String, CodingKey {
        case loginId = "login_id"
        case accountId = "account_id"
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
