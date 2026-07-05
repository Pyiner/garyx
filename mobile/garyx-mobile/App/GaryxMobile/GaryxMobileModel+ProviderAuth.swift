import Foundation
import WidgetKit

extension GaryxMobileModel {
    func startClaudeCodeAuth(
        mode: GaryxClaudeCodeAuthMode,
        sso: Bool,
        email: String
    ) async -> URL? {
        cancelClaudeCodeAuthPolling()
        let runtimeGeneration = gatewayRuntimeGeneration
        claudeCodeAuthSession = GaryxClaudeCodeAuthSession(loginId: "", status: .starting)
        do {
            let session = try await client().startClaudeCodeAuth(
                GaryxClaudeCodeAuthStartRequest(
                    mode: mode,
                    sso: sso,
                    email: email
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return nil }
            claudeCodeAuthSession = session
            if session.status == .succeeded {
                await refreshClaudeCodeAuthSuccessState(runtimeGeneration: runtimeGeneration)
            }
            return session.authorizationURL
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return nil }
            let message = displayMessage(for: error)
            claudeCodeAuthSession = GaryxClaudeCodeAuthSession(
                loginId: "",
                status: .failed,
                error: message
            )
            lastError = message
            return nil
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
                resetClaudeCodeAuthFlow()
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
                    resetClaudeCodeAuthFlow()
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
