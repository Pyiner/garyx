import Foundation
import WidgetKit

extension GaryxMobileModel {
    func loadClaudeCodeAccounts(
        runtimeGeneration: GaryxGatewayRequestToken? = nil
    ) async {
        let observedGeneration = runtimeGeneration ?? gatewayRequestToken
        let loadGeneration = UUID()
        claudeCodeAccountsLoadGeneration = loadGeneration
        isLoadingClaudeCodeAccounts = true
        claudeCodeAccountsError = nil
        do {
            let accounts = try await client().claudeCodeAccounts()
            guard observedGeneration == gatewayRequestToken,
                  claudeCodeAccountsLoadGeneration == loadGeneration else { return }
            claudeCodeAccounts = accounts
            isLoadingClaudeCodeAccounts = false
        } catch {
            guard !GaryxGatewayRetryClassifier.isCancellation(error),
                  observedGeneration == gatewayRequestToken,
                  claudeCodeAccountsLoadGeneration == loadGeneration else { return }
            let message = displayMessage(for: error)
            claudeCodeAccountsError = message
            lastError = message
            isLoadingClaudeCodeAccounts = false
        }
    }

    @discardableResult
    func selectClaudeCodeAccount(
        accountId: String?
    ) async -> GaryxClaudeCodeAccountSelection? {
        await mutateClaudeCodeAccount { gateway in
            try await gateway.selectClaudeCodeAccount(accountId: accountId)
        }
    }

    @discardableResult
    func renameClaudeCodeAccount(accountId: String, name: String) async -> Bool {
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedName.isEmpty else {
            claudeCodeAccountsError = "Enter an account name."
            return false
        }
        return await mutateClaudeCodeAccount(refreshesUsage: false) { gateway in
            try await gateway.renameClaudeCodeAccount(accountId: accountId, name: trimmedName)
        } != nil
    }

    @discardableResult
    func deleteClaudeCodeAccount(accountId: String) async -> Bool {
        await mutateClaudeCodeAccount { gateway in
            try await gateway.deleteClaudeCodeAccount(accountId: accountId)
        } != nil
    }

    private func mutateClaudeCodeAccount<Result>(
        refreshesUsage: Bool = true,
        operation: (GaryxGatewayClient) async throws -> Result
    ) async -> Result? {
        let runtimeGeneration = gatewayRequestToken
        let mutationGeneration = UUID()
        claudeCodeAccountMutationGeneration = mutationGeneration
        isMutatingClaudeCodeAccount = true
        claudeCodeAccountsError = nil
        do {
            let gateway = try client()
            let result = try await operation(gateway)
            guard runtimeGeneration == gatewayRequestToken,
                  claudeCodeAccountMutationGeneration == mutationGeneration else { return nil }
            async let accountsRefresh: Void = loadClaudeCodeAccounts(runtimeGeneration: runtimeGeneration)
            if refreshesUsage {
                async let usageRefresh: Void = refreshCodingUsageWidget(runtimeGeneration: runtimeGeneration)
                _ = await (accountsRefresh, usageRefresh)
            } else {
                _ = await accountsRefresh
            }
            guard runtimeGeneration == gatewayRequestToken,
                  claudeCodeAccountMutationGeneration == mutationGeneration else { return nil }
            isMutatingClaudeCodeAccount = false
            return result
        } catch {
            guard !GaryxGatewayRetryClassifier.isCancellation(error),
                  runtimeGeneration == gatewayRequestToken,
                  claudeCodeAccountMutationGeneration == mutationGeneration else { return nil }
            let message = displayMessage(for: error)
            claudeCodeAccountsError = message
            lastError = message
            isMutatingClaudeCodeAccount = false
            return nil
        }
    }

    func retryThreadQuotaRecovery(threadId: String) async {
        do {
            try await client().retryThreadQuotaRecovery(threadId: threadId)
        } catch {
            guard !GaryxGatewayRetryClassifier.isCancellation(error) else { return }
            lastError = displayMessage(for: error)
        }
    }

    /// Begins a Claude Code sign-in with the chosen advanced options. Never
    /// sends an email. The target selects System default, reserves a new
    /// managed profile, or reauthenticates an existing managed profile.
    func startClaudeCodeAuth(
        options: GaryxClaudeCodeLoginOptions,
        target: GaryxClaudeCodeAuthTarget = .systemDefault
    ) async {
        resetClaudeCodeAuthFlow()
        let runtimeGeneration = gatewayRequestToken
        let flowGeneration = UUID()
        claudeCodeAuthFlowGeneration = flowGeneration
        claudeCodeAuthSession = GaryxClaudeCodeAuthSession(loginId: "", status: .starting)
        do {
            let gateway = try client()
            let session = try await gateway.startClaudeCodeAuth(
                options.makeStartRequest(target: target)
            )
            guard runtimeGeneration == gatewayRequestToken,
                  flowGeneration == claudeCodeAuthFlowGeneration else {
                if !session.loginId.isEmpty {
                    _ = try? await gateway.cancelClaudeCodeAuth(loginId: session.loginId)
                }
                return
            }
            claudeCodeAuthSession = session
            if session.status == .succeeded {
                await refreshClaudeCodeAuthSuccessState(
                    runtimeGeneration: runtimeGeneration,
                    flowGeneration: flowGeneration
                )
            }
        } catch {
            guard runtimeGeneration == gatewayRequestToken,
                  flowGeneration == claudeCodeAuthFlowGeneration else { return }
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
        let runtimeGeneration = gatewayRequestToken
        let flowGeneration = claudeCodeAuthFlowGeneration
        cancelClaudeCodeAuthPolling()
        claudeCodeAuthSession = GaryxClaudeCodeAuthSession(
            loginId: current.loginId,
            accountId: current.accountId,
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
            guard runtimeGeneration == gatewayRequestToken,
                  flowGeneration == claudeCodeAuthFlowGeneration else { return }
            claudeCodeAuthSession = session
            if session.status == .succeeded {
                await refreshClaudeCodeAuthSuccessState(
                    runtimeGeneration: runtimeGeneration,
                    flowGeneration: flowGeneration
                )
            } else if session.status == .failed {
                lastError = session.error
            } else {
                startClaudeCodeAuthPolling(
                    loginId: session.loginId,
                    runtimeGeneration: runtimeGeneration,
                    flowGeneration: flowGeneration
                )
            }
        } catch {
            guard runtimeGeneration == gatewayRequestToken,
                  flowGeneration == claudeCodeAuthFlowGeneration else { return }
            if isClaudeCodeAuthSessionMissing(error) {
                markClaudeCodeAuthSessionExpired()
                return
            }
            let message = displayMessage(for: error)
            claudeCodeAuthSession = GaryxClaudeCodeAuthSession(
                loginId: current.loginId,
                accountId: current.accountId,
                status: .failed,
                url: current.url,
                error: message
            )
            lastError = message
        }
    }

    func resetClaudeCodeAuthFlow() {
        let previousSession = claudeCodeAuthSession
        claudeCodeAuthFlowGeneration = UUID()
        cancelClaudeCodeAuthPolling()
        claudeCodeAuthSession = nil
        guard let previousSession,
              !previousSession.loginId.isEmpty,
              let gateway = try? client() else { return }
        Task {
            _ = try? await gateway.cancelClaudeCodeAuth(loginId: previousSession.loginId)
        }
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
            accountId: claudeCodeAuthSession?.accountId,
            status: .failed,
            error: "Your Claude sign-in session expired. Start over to sign in again."
        )
    }

    private func startClaudeCodeAuthPolling(
        loginId: String,
        runtimeGeneration: GaryxGatewayRequestToken,
        flowGeneration: UUID
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
                flowGeneration: flowGeneration,
                pollGeneration: pollGeneration
            )
        }
    }

    private func pollClaudeCodeAuth(
        loginId: String,
        runtimeGeneration: GaryxGatewayRequestToken,
        flowGeneration: UUID,
        pollGeneration: UUID
    ) async {
        while !Task.isCancelled {
            do {
                try await Task.sleep(nanoseconds: 1_500_000_000)
                try Task.checkCancellation()
                let session = try await client().claudeCodeAuth(loginId: loginId)
                guard runtimeGeneration == gatewayRequestToken,
                      flowGeneration == claudeCodeAuthFlowGeneration,
                      pollGeneration == claudeCodeAuthPollGeneration,
                      claudeCodeAuthSession?.loginId == loginId else {
                    return
                }
                claudeCodeAuthSession = session
                switch session.status {
                case .succeeded:
                    cancelClaudeCodeAuthPolling()
                    await refreshClaudeCodeAuthSuccessState(
                        runtimeGeneration: runtimeGeneration,
                        flowGeneration: flowGeneration
                    )
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
                guard runtimeGeneration == gatewayRequestToken,
                      flowGeneration == claudeCodeAuthFlowGeneration,
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
                    accountId: claudeCodeAuthSession?.accountId,
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

    private func refreshClaudeCodeAuthSuccessState(
        runtimeGeneration: GaryxGatewayRequestToken,
        flowGeneration: UUID
    ) async {
        do {
            let usage = try await client().codingUsage()
            guard runtimeGeneration == gatewayRequestToken,
                  flowGeneration == claudeCodeAuthFlowGeneration else { return }
            codingUsage = usage
            GaryxUsageWidgetStore.saveSnapshot(
                GaryxUsageWidgetSnapshot(usage: usage, fetchedAt: Date())
            )
            WidgetCenter.shared.reloadTimelines(ofKind: GaryxCodingUsageWidgetConstants.kind)
        } catch {
            guard runtimeGeneration == gatewayRequestToken,
                  flowGeneration == claudeCodeAuthFlowGeneration else { return }
            lastError = displayMessage(for: error)
        }
        await loadClaudeCodeAccounts(runtimeGeneration: runtimeGeneration)
        guard flowGeneration == claudeCodeAuthFlowGeneration else { return }
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
        guard case GaryxGatewayError.httpStatus(let status, _, _) = error else {
            return false
        }
        return status == 404
    }
}
