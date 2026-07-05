import Foundation
import WidgetKit

extension GaryxMobileModel {
    /// Begins a Claude Code sign-in with the chosen advanced options. Never
    /// sends an email (the field was removed from the request). The guided login
    /// sheet reacts to `claudeCodeAuthSession` and opens the authorization URL on
    /// the Authorize screen, so this no longer auto-opens the browser.
    func startClaudeCodeAuth(options: GaryxClaudeCodeLoginOptions) async {
        cancelClaudeCodeAuthPolling()
        let runtimeGeneration = gatewayRuntimeGeneration
        claudeCodeAuthSession = GaryxClaudeCodeAuthSession(loginId: "", status: .starting)
        do {
            let session = try await client().startClaudeCodeAuth(options.startRequest)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            claudeCodeAuthSession = session
            if session.status == .succeeded {
                await refreshClaudeCodeAuthSuccessState(runtimeGeneration: runtimeGeneration)
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let message = displayMessage(for: error)
            claudeCodeAuthSession = GaryxClaudeCodeAuthSession(
                loginId: "",
                status: .failed,
                error: message
            )
            lastError = message
        }
    }

    func submitClaudeCodeAuth(code: String) async {
        let trimmedCode = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedCode.isEmpty,
              let current = claudeCodeAuthSession,
              !current.loginId.isEmpty else {
            return
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        cancelClaudeCodeAuthPolling()
        claudeCodeAuthSession = GaryxClaudeCodeAuthSession(
            loginId: current.loginId,
            status: .submitted,
            url: current.url,
            authStatus: current.authStatus,
            error: nil,
            exitCode: current.exitCode
        )
        do {
            let session = try await client().submitClaudeCodeAuth(
                loginId: current.loginId,
                code: trimmedCode
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            claudeCodeAuthSession = session
            if session.status == .succeeded {
                await refreshClaudeCodeAuthSuccessState(runtimeGeneration: runtimeGeneration)
            } else if session.status == .failed {
                lastError = session.error
            } else {
                startClaudeCodeAuthPolling(
                    loginId: session.loginId,
                    runtimeGeneration: runtimeGeneration
                )
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            if isClaudeCodeAuthSessionMissing(error) {
                markClaudeCodeAuthSessionExpired()
                return
            }
            let message = displayMessage(for: error)
            claudeCodeAuthSession = GaryxClaudeCodeAuthSession(
                loginId: current.loginId,
                status: .failed,
                url: current.url,
                error: message
            )
            lastError = message
        }
    }

    func resetClaudeCodeAuthFlow() {
        cancelClaudeCodeAuthPolling()
        claudeCodeAuthSession = nil
    }

    /// A 404 means the gateway no longer knows this login session — it lives in
    /// an in-memory map with no TTL and is dropped on gateway restart. Surface it
    /// as a terminal failure so the sheet shows an explicit "session expired"
    /// screen (Try Again / Start Over) instead of silently snapping back to the
    /// intro. The UI must not imply the remote login was cancelled.
    private func markClaudeCodeAuthSessionExpired() {
        cancelClaudeCodeAuthPolling()
        claudeCodeAuthSession = GaryxClaudeCodeAuthSession(
            loginId: "",
            status: .failed,
            error: "Your Claude sign-in session expired. Start over to sign in again."
        )
    }

    private func startClaudeCodeAuthPolling(
        loginId: String,
        runtimeGeneration: UUID
    ) {
        guard !loginId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        cancelClaudeCodeAuthPolling()
        let pollGeneration = UUID()
        claudeCodeAuthPollGeneration = pollGeneration
        claudeCodeAuthPollTask = Task { [weak self] in
            guard let self else { return }
            await self.pollClaudeCodeAuth(
                loginId: loginId,
                runtimeGeneration: runtimeGeneration,
                pollGeneration: pollGeneration
            )
        }
    }

    private func pollClaudeCodeAuth(
        loginId: String,
        runtimeGeneration: UUID,
        pollGeneration: UUID
    ) async {
        while !Task.isCancelled {
            do {
                try await Task.sleep(nanoseconds: 1_500_000_000)
                try Task.checkCancellation()
                let session = try await client().claudeCodeAuth(loginId: loginId)
                guard runtimeGeneration == gatewayRuntimeGeneration,
                      pollGeneration == claudeCodeAuthPollGeneration,
                      claudeCodeAuthSession?.loginId == loginId else {
                    return
                }
                claudeCodeAuthSession = session
                switch session.status {
                case .succeeded:
                    cancelClaudeCodeAuthPolling()
                    await refreshClaudeCodeAuthSuccessState(runtimeGeneration: runtimeGeneration)
                    return
                case .failed:
                    cancelClaudeCodeAuthPolling()
                    if let error = session.error {
                        lastError = error
                    }
                    return
                case .starting, .waitingForCode, .submitted:
                    continue
                }
            } catch {
                guard !GaryxGatewayRetryClassifier.isCancellation(error) else { return }
                guard runtimeGeneration == gatewayRuntimeGeneration,
                      pollGeneration == claudeCodeAuthPollGeneration,
                      claudeCodeAuthSession?.loginId == loginId else {
                    return
                }
                if isClaudeCodeAuthSessionMissing(error) {
                    markClaudeCodeAuthSessionExpired()
                    return
                }
                let message = displayMessage(for: error)
                claudeCodeAuthSession = GaryxClaudeCodeAuthSession(
                    loginId: loginId,
                    status: .failed,
                    url: claudeCodeAuthSession?.url,
                    error: message
                )
                lastError = message
                cancelClaudeCodeAuthPolling()
                return
            }
        }
    }

    private func refreshClaudeCodeAuthSuccessState(runtimeGeneration: UUID) async {
        do {
            let usage = try await client().codingUsage()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            codingUsage = usage
            GaryxUsageWidgetStore.saveSnapshot(
                GaryxUsageWidgetSnapshot(usage: usage, fetchedAt: Date())
            )
            WidgetCenter.shared.reloadTimelines(ofKind: GaryxCodingUsageWidgetConstants.kind)
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
        await loadProviderModels(
            providerType: "claude_code",
            runtimeGeneration: runtimeGeneration
        )
    }

    private func cancelClaudeCodeAuthPolling() {
        claudeCodeAuthPollTask?.cancel()
        claudeCodeAuthPollTask = nil
        claudeCodeAuthPollGeneration = nil
    }

    private func isClaudeCodeAuthSessionMissing(_ error: Error) -> Bool {
        guard case GaryxGatewayError.httpStatus(let status, _) = error else {
            return false
        }
        return status == 404
    }
}
