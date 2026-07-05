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

    func testPresentationStatesFollowAuthFlow() {
        let idle = GaryxClaudeCodeAuthPresentation.make(session: nil, usage: nil)
        XCTAssertEqual(idle.statusText, "Needs login")
        XCTAssertEqual(idle.primaryAction, .start)
        XCTAssertTrue(idle.showsLoginOptions)

        let waiting = GaryxClaudeCodeAuthPresentation.make(
            session: GaryxClaudeCodeAuthSession(
                loginId: "login-test",
                status: .waitingForCode,
                url: "https://claude.example.test/oauth"
            ),
            usage: nil,
            authorizationCode: "code-test"
        )
        XCTAssertEqual(waiting.statusText, "Waiting for code")
        XCTAssertEqual(waiting.primaryAction, .openAuthorizationURL)
        XCTAssertTrue(waiting.showsCodeField)
        XCTAssertTrue(waiting.submitEnabled)

        let submitted = GaryxClaudeCodeAuthPresentation.make(
            session: GaryxClaudeCodeAuthSession(loginId: "login-test", status: .submitted),
            usage: nil,
            authorizationCode: "code-test"
        )
        XCTAssertEqual(submitted.statusText, "Submitted")
        XCTAssertFalse(submitted.submitEnabled)

        let succeeded = GaryxClaudeCodeAuthPresentation.make(
            session: GaryxClaudeCodeAuthSession(
                loginId: "login-test",
                status: .succeeded,
                authStatus: .object([
                    "loggedIn": .bool(true),
                    "orgName": .string("Test Org"),
                ])
            ),
            usage: nil
        )
        XCTAssertEqual(succeeded.statusText, "Signed in")
        XCTAssertEqual(succeeded.accountText, "Test Org")
        XCTAssertEqual(succeeded.primaryActionTitle, "Re-authenticate")

        let failed = GaryxClaudeCodeAuthPresentation.make(
            session: GaryxClaudeCodeAuthSession(
                loginId: "login-test",
                status: .failed,
                error: "Timed out waiting for Claude Code login URL."
            ),
            usage: nil
        )
        XCTAssertEqual(failed.statusText, "Login failed")
        XCTAssertEqual(failed.tone, .danger)
        XCTAssertEqual(failed.primaryActionTitle, "Retry sign in")
    }

    func testClaudeCodeProviderDefaultsNeverWriteAuthSourceOrTokenSettings() throws {
        let provider = try XCTUnwrap(GaryxModelProviderDefaults.provider(for: "claude_code"))
        var settings: [String: GaryxJSONValue] = [:]

        GaryxModelProviderDefaults.update(
            settings: &settings,
            provider: provider,
            model: "Claude Sonnet 4.6",
            reasoningEffort: "medium",
            authSource: "api_key",
            baseUrl: "https://example.invalid",
            apiKey: .set("${TOKEN}")
        )

        let config = GaryxModelProviderDefaults.providerConfig(in: settings, provider: provider)
        XCTAssertEqual(config["provider_type"], .string("claude_code"))
        XCTAssertNil(config["auth_source"])
        XCTAssertNil(config["base_url"])
        XCTAssertNil(config["env"])
    }
}
