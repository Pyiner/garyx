import XCTest
@testable import GaryxMobileCore

final class GaryxClaudeCodeAuthTests: XCTestCase {
    func testAuthSessionDecodesGatewaySnakeCaseAndAuthStatus() throws {
        let session = try JSONDecoder().decode(
            GaryxClaudeCodeAuthSession.self,
            from: Data(
                """
                {
                  "login_id": "login-test",
                  "status": "waiting_for_code",
                  "url": "https://claude.example.test/oauth",
                  "auth_status": {
                    "loggedIn": true,
                    "orgName": "Test Org",
                    "subscriptionType": "team",
                    "email": "bot@example.com"
                  },
                  "exit_code": 0
                }
                """.utf8
            )
        )

        XCTAssertEqual(session.loginId, "login-test")
        XCTAssertEqual(session.status, .waitingForCode)
        XCTAssertEqual(session.authorizationURL?.absoluteString, "https://claude.example.test/oauth")
        XCTAssertEqual(session.exitCode, 0)

        let account = GaryxClaudeCodeAuthAccount.make(authStatus: session.authStatus, usage: nil)
        XCTAssertTrue(account.loggedIn)
        XCTAssertEqual(account.orgName, "Test Org")
        XCTAssertEqual(account.plan, "team")
        XCTAssertEqual(account.email, "bot@example.com")
    }

    func testSubmittedAndSucceededResponsesDecode() throws {
        let submitted = try JSONDecoder().decode(
            GaryxClaudeCodeAuthSession.self,
            from: Data(
                """
                {
                  "login_id": "login-test",
                  "status": "submitted"
                }
                """.utf8
            )
        )
        let succeeded = try JSONDecoder().decode(
            GaryxClaudeCodeAuthSession.self,
            from: Data(
                """
                {
                  "login_id": "login-test",
                  "status": "succeeded",
                  "auth_status": {
                    "loggedIn": true,
                    "orgName": "Test Org",
                    "subscriptionType": "max"
                  }
                }
                """.utf8
            )
        )

        XCTAssertEqual(submitted.status, .submitted)
        XCTAssertEqual(succeeded.status, .succeeded)
        XCTAssertEqual(
            GaryxClaudeCodeAuthAccount.make(authStatus: succeeded.authStatus, usage: nil).plan,
            "max"
        )
    }

    func testAccountFallsBackToUsagePlanWhenAuthStatusIsAbsent() {
        let usage = GaryxProviderUsage(
            id: "claude_code",
            name: "Claude Code",
            available: true,
            plan: "pro"
        )

        let account = GaryxClaudeCodeAuthAccount.make(authStatus: nil, usage: usage)

        XCTAssertTrue(account.loggedIn)
        XCTAssertEqual(account.plan, "pro")
        XCTAssertEqual(account.displayName, "pro")
    }

    // MARK: Simplified start request (email removed)

    func testDefaultStartRequestEncodesModeOnlyAndNeverEmail() throws {
        let data = try JSONEncoder().encode(GaryxClaudeCodeLoginOptions().startRequest)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["mode"] as? String, "claudeai")
        XCTAssertNil(object["email"], "iOS must never send an email in the start request")
        XCTAssertNil(object["sso"], "sso is omitted unless explicitly enabled")
        XCTAssertEqual(object.count, 1)
        XCTAssertTrue(GaryxClaudeCodeLoginOptions().isDefault)
    }

    func testAdvancedConsoleWithSSOEncodesModeAndSSOButNeverEmail() throws {
        let options = GaryxClaudeCodeLoginOptions(mode: .console, useSSO: true)
        let data = try JSONEncoder().encode(options.startRequest)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])

        XCTAssertEqual(object["mode"] as? String, "console")
        XCTAssertEqual(object["sso"] as? Bool, true)
        XCTAssertNil(object["email"])
        XCTAssertFalse(options.isDefault)
    }

    // MARK: Guided login step machine

    func testLoginStepMapsEveryGatewayStatusToOneScreen() {
        typealias Present = GaryxClaudeCodeLoginPresentation
        XCTAssertEqual(Present.step(for: nil, hasOpenedAuthorizationURL: false), .intro)
        XCTAssertEqual(Present.step(for: .starting, hasOpenedAuthorizationURL: false), .authorize)
        // waiting_for_code splits on the client-only opened flag.
        XCTAssertEqual(Present.step(for: .waitingForCode, hasOpenedAuthorizationURL: false), .authorize)
        XCTAssertEqual(Present.step(for: .waitingForCode, hasOpenedAuthorizationURL: true), .enterCode)
        XCTAssertEqual(Present.step(for: .submitted, hasOpenedAuthorizationURL: false), .submitting)
        XCTAssertEqual(Present.step(for: .succeeded, hasOpenedAuthorizationURL: false), .success)
        XCTAssertEqual(Present.step(for: .failed, hasOpenedAuthorizationURL: false), .failure)
    }

    func testLoginPresentationIntroOffersSignInOnly() {
        let intro = GaryxClaudeCodeLoginPresentation.make(session: nil, usage: nil)
        XCTAssertEqual(intro.step, .intro)
        XCTAssertEqual(intro.symbolName, "sparkles")
        XCTAssertEqual(intro.primaryAction?.kind, .start)
        XCTAssertEqual(intro.primaryAction?.title, "Sign in with Claude")
        XCTAssertNil(intro.secondaryAction)
        XCTAssertFalse(intro.showsCodeField)
        XCTAssertFalse(intro.showsProgress)
    }

    func testLoginPresentationAuthorizePreparingVersusReady() {
        let preparing = GaryxClaudeCodeLoginPresentation.make(
            session: GaryxClaudeCodeAuthSession(loginId: "l", status: .starting),
            usage: nil
        )
        XCTAssertEqual(preparing.step, .authorize)
        XCTAssertTrue(preparing.showsProgress)
        XCTAssertEqual(preparing.primaryAction?.kind, .openAuthorizationURL)
        XCTAssertEqual(preparing.primaryAction?.isEnabled, false)
        XCTAssertNil(preparing.secondaryAction)

        let ready = GaryxClaudeCodeLoginPresentation.make(
            session: GaryxClaudeCodeAuthSession(
                loginId: "l",
                status: .waitingForCode,
                url: "https://claude.example.test/oauth"
            ),
            usage: nil,
            hasOpenedAuthorizationURL: false
        )
        XCTAssertEqual(ready.step, .authorize)
        XCTAssertFalse(ready.showsProgress)
        XCTAssertEqual(ready.primaryAction?.kind, .openAuthorizationURL)
        XCTAssertEqual(ready.primaryAction?.isEnabled, true)
        XCTAssertEqual(ready.secondaryAction?.kind, .enterCode)
    }

    func testLoginPresentationEnterCodeGatesSubmitOnCode() {
        let session = GaryxClaudeCodeAuthSession(
            loginId: "l",
            status: .waitingForCode,
            url: "https://claude.example.test/oauth"
        )
        let empty = GaryxClaudeCodeLoginPresentation.make(
            session: session,
            usage: nil,
            authorizationCode: "   ",
            hasOpenedAuthorizationURL: true
        )
        XCTAssertEqual(empty.step, .enterCode)
        XCTAssertTrue(empty.showsCodeField)
        XCTAssertEqual(empty.primaryAction?.kind, .submitCode)
        XCTAssertEqual(empty.primaryAction?.isEnabled, false)
        XCTAssertEqual(empty.secondaryAction?.kind, .openAuthorizationURL)

        let filled = GaryxClaudeCodeLoginPresentation.make(
            session: session,
            usage: nil,
            authorizationCode: "code-123",
            hasOpenedAuthorizationURL: true
        )
        XCTAssertEqual(filled.primaryAction?.isEnabled, true)
    }

    func testLoginPresentationSubmittingHasNoButtons() {
        let submitting = GaryxClaudeCodeLoginPresentation.make(
            session: GaryxClaudeCodeAuthSession(loginId: "l", status: .submitted),
            usage: nil
        )
        XCTAssertEqual(submitting.step, .submitting)
        XCTAssertTrue(submitting.showsProgress)
        XCTAssertNil(submitting.primaryAction)
        XCTAssertNil(submitting.secondaryAction)
    }

    func testLoginPresentationSuccessListsAccountDetails() {
        let success = GaryxClaudeCodeLoginPresentation.make(
            session: GaryxClaudeCodeAuthSession(
                loginId: "l",
                status: .succeeded,
                authStatus: .object([
                    "loggedIn": .bool(true),
                    "orgName": .string("Test Org"),
                    "subscriptionType": .string("max"),
                    "authMethod": .string("claudeai"),
                ])
            ),
            usage: nil
        )
        XCTAssertEqual(success.step, .success)
        XCTAssertEqual(success.symbolName, "checkmark.circle.fill")
        XCTAssertEqual(success.tone, .good)
        XCTAssertEqual(success.primaryAction?.kind, .done)
        XCTAssertNil(success.secondaryAction)
        XCTAssertEqual(success.detailRows.first(where: { $0.label == "Account" })?.value, "Test Org")
        XCTAssertEqual(success.detailRows.first(where: { $0.label == "Plan" })?.value, "max")
        XCTAssertEqual(success.detailRows.first(where: { $0.label == "Method" })?.value, "claudeai")
    }

    func testLoginPresentationFailureOffersRetryAndStartOver() {
        let failure = GaryxClaudeCodeLoginPresentation.make(
            session: GaryxClaudeCodeAuthSession(
                loginId: "",
                status: .failed,
                error: "Timed out waiting for Claude Code login URL."
            ),
            usage: nil
        )
        XCTAssertEqual(failure.step, .failure)
        XCTAssertEqual(failure.tone, .danger)
        XCTAssertEqual(failure.symbolName, "exclamationmark.triangle.fill")
        XCTAssertEqual(failure.message, "Timed out waiting for Claude Code login URL.")
        XCTAssertEqual(failure.primaryAction?.kind, .start)
        XCTAssertEqual(failure.primaryAction?.title, "Try Again")
        XCTAssertEqual(failure.secondaryAction?.kind, .startOver)
    }

    // MARK: Provider section entry

    func testEntryReflectsSignedOutState() {
        let entry = GaryxClaudeCodeAuthEntry.make(
            session: nil,
            usage: GaryxProviderUsage(
                id: "claude_code",
                name: "Claude Code",
                available: false,
                error: "Sign in required"
            )
        )
        XCTAssertFalse(entry.isSignedIn)
        XCTAssertEqual(entry.statusText, "Not signed in")
        XCTAssertEqual(entry.tone, .muted)
        XCTAssertEqual(entry.actionTitle, "Sign in with Claude")
        XCTAssertEqual(entry.actionSymbolName, "sparkles")
        XCTAssertNil(entry.accountText)
        XCTAssertEqual(entry.footnote, "Sign in required")
    }

    func testEntryReflectsSignedInState() {
        let entry = GaryxClaudeCodeAuthEntry.make(
            session: GaryxClaudeCodeAuthSession(
                loginId: "l",
                status: .succeeded,
                authStatus: .object([
                    "loggedIn": .bool(true),
                    "orgName": .string("Test Org"),
                    "subscriptionType": .string("max"),
                ])
            ),
            usage: nil
        )
        XCTAssertTrue(entry.isSignedIn)
        XCTAssertEqual(entry.statusText, "Signed in")
        XCTAssertEqual(entry.tone, .good)
        XCTAssertEqual(entry.actionTitle, "Re-authenticate")
        XCTAssertEqual(entry.actionSymbolName, "arrow.triangle.2.circlepath")
        XCTAssertEqual(entry.accountText, "Test Org")
    }

    func testClaudeCodeProviderDefaultsWriteOnlyDefaultFields() throws {
        let provider = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "claude_code"))
        var settings: [String: GaryxJSONValue] = [:]

        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: provider,
            model: "Claude Sonnet 4.6",
            reasoningEffort: "medium"
        )

        let config = GaryxModelProviderDefaults.providerConfig(in: settings, provider: provider)
        XCTAssertEqual(config["provider_type"], .string("claude_code"))
        XCTAssertEqual(config["default_model"], .string("Claude Sonnet 4.6"))
        XCTAssertEqual(config["model_reasoning_effort"], .string("medium"))
        XCTAssertNil(config["env"])
    }
}
