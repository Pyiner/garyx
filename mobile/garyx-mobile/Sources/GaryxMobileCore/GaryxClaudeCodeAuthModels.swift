import Foundation

public enum GaryxClaudeCodeAuthMode: String, Codable, CaseIterable, Identifiable, Sendable {
    case claudeai
    case console

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .claudeai:
            return "Claude.ai"
        case .console:
            return "Console"
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
    public var email: String?

    public init(
        mode: GaryxClaudeCodeAuthMode = .claudeai,
        sso: Bool = false,
        email: String? = nil
    ) {
        self.mode = mode
        self.sso = sso
        self.email = email?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
    }

    enum CodingKeys: String, CodingKey {
        case mode
        case sso
        case email
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(mode, forKey: .mode)
        if sso {
            try container.encode(sso, forKey: .sso)
        }
        try container.encodeIfPresent(email, forKey: .email)
    }
}

public struct GaryxClaudeCodeAuthSubmitRequest: Encodable, Equatable, Sendable {
    public var code: String

    public init(code: String) {
        self.code = code.trimmingCharacters(in: .whitespacesAndNewlines)
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

public enum GaryxClaudeCodeAuthPrimaryAction: Equatable, Sendable {
    case start
    case openAuthorizationURL
    case none
}

public struct GaryxClaudeCodeAuthPresentation: Equatable, Sendable {
    public var statusText: String
    public var detailText: String?
    public var tone: GaryxClaudeCodeAuthPresentationTone
    public var accountText: String?
    public var accountDetailText: String?
    public var primaryActionTitle: String
    public var primaryAction: GaryxClaudeCodeAuthPrimaryAction
    public var primaryActionEnabled: Bool
    public var showsLoginOptions: Bool
    public var showsAuthorizationControls: Bool
    public var showsCodeField: Bool
    public var submitEnabled: Bool

    public init(
        statusText: String,
        detailText: String? = nil,
        tone: GaryxClaudeCodeAuthPresentationTone,
        accountText: String? = nil,
        accountDetailText: String? = nil,
        primaryActionTitle: String,
        primaryAction: GaryxClaudeCodeAuthPrimaryAction,
        primaryActionEnabled: Bool,
        showsLoginOptions: Bool,
        showsAuthorizationControls: Bool,
        showsCodeField: Bool,
        submitEnabled: Bool
    ) {
        self.statusText = statusText
        self.detailText = detailText?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.tone = tone
        self.accountText = accountText?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.accountDetailText = accountDetailText?.trimmingCharacters(in: .whitespacesAndNewlines).garyxGatewayTrimmedNilIfEmpty
        self.primaryActionTitle = primaryActionTitle
        self.primaryAction = primaryAction
        self.primaryActionEnabled = primaryActionEnabled
        self.showsLoginOptions = showsLoginOptions
        self.showsAuthorizationControls = showsAuthorizationControls
        self.showsCodeField = showsCodeField
        self.submitEnabled = submitEnabled
    }

    public static func make(
        session: GaryxClaudeCodeAuthSession?,
        usage: GaryxProviderUsage?,
        authorizationCode: String = ""
    ) -> GaryxClaudeCodeAuthPresentation {
        let account = GaryxClaudeCodeAuthAccount.make(
            authStatus: session?.authStatus,
            usage: usage
        )
        let hasCode = !authorizationCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty

        switch session?.status {
        case .starting:
            return GaryxClaudeCodeAuthPresentation(
                statusText: "Starting",
                detailText: "Waiting for the gateway to return an authorization URL.",
                tone: .warning,
                accountText: account.displayName,
                accountDetailText: account.detailText,
                primaryActionTitle: "Starting",
                primaryAction: .none,
                primaryActionEnabled: false,
                showsLoginOptions: false,
                showsAuthorizationControls: false,
                showsCodeField: false,
                submitEnabled: false
            )
        case .waitingForCode:
            return GaryxClaudeCodeAuthPresentation(
                statusText: "Waiting for code",
                detailText: session?.error,
                tone: .warning,
                accountText: account.displayName,
                accountDetailText: account.detailText,
                primaryActionTitle: "Open authorization page",
                primaryAction: .openAuthorizationURL,
                primaryActionEnabled: session?.authorizationURL != nil,
                showsLoginOptions: false,
                showsAuthorizationControls: session?.authorizationURL != nil,
                showsCodeField: true,
                submitEnabled: hasCode
            )
        case .submitted:
            return GaryxClaudeCodeAuthPresentation(
                statusText: "Submitted",
                detailText: "Waiting for Claude Code to finish sign-in on the gateway.",
                tone: .warning,
                accountText: account.displayName,
                accountDetailText: account.detailText,
                primaryActionTitle: "Submitted",
                primaryAction: .none,
                primaryActionEnabled: false,
                showsLoginOptions: false,
                showsAuthorizationControls: session?.authorizationURL != nil,
                showsCodeField: true,
                submitEnabled: false
            )
        case .succeeded:
            return signedInPresentation(
                account: account,
                primaryActionTitle: "Re-authenticate"
            )
        case .failed:
            return GaryxClaudeCodeAuthPresentation(
                statusText: "Login failed",
                detailText: session?.error ?? "Claude Code login failed.",
                tone: .danger,
                accountText: account.displayName,
                accountDetailText: account.detailText,
                primaryActionTitle: "Retry sign in",
                primaryAction: .start,
                primaryActionEnabled: true,
                showsLoginOptions: true,
                showsAuthorizationControls: false,
                showsCodeField: false,
                submitEnabled: false
            )
        case .none:
            if account.loggedIn {
                return signedInPresentation(
                    account: account,
                    primaryActionTitle: "Re-authenticate"
                )
            }
            return GaryxClaudeCodeAuthPresentation(
                statusText: "Needs login",
                detailText: usage?.error,
                tone: .muted,
                accountText: account.displayName,
                accountDetailText: account.detailText,
                primaryActionTitle: "Sign in with Claude",
                primaryAction: .start,
                primaryActionEnabled: true,
                showsLoginOptions: true,
                showsAuthorizationControls: false,
                showsCodeField: false,
                submitEnabled: false
            )
        }
    }

    private static func signedInPresentation(
        account: GaryxClaudeCodeAuthAccount,
        primaryActionTitle: String
    ) -> GaryxClaudeCodeAuthPresentation {
        GaryxClaudeCodeAuthPresentation(
            statusText: "Signed in",
            detailText: nil,
            tone: .good,
            accountText: account.displayName,
            accountDetailText: account.detailText,
            primaryActionTitle: primaryActionTitle,
            primaryAction: .start,
            primaryActionEnabled: true,
            showsLoginOptions: true,
            showsAuthorizationControls: false,
            showsCodeField: false,
            submitEnabled: false
        )
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
