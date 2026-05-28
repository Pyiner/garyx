import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func openBotThread(_ threadId: String?) async {
        guard let threadId, !threadId.isEmpty else { return }
        await openThread(id: threadId)
    }

    func loadBotStatus(_ bot: GaryxConfiguredBot) async {
        do {
            botStatusesById[bot.id] = try await client().botStatus(botId: bot.id)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func bindBotToSelectedThread(_ bot: GaryxConfiguredBot) async {
        guard let threadId = selectedThread?.id else { return }
        await bindBot(bot, toThreadId: threadId)
    }

    func bindBot(_ bot: GaryxConfiguredBot, toThreadId threadId: String) async {
        let threadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let status = try await client().bindBot(botId: bot.id, threadId: threadId)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            botStatusesById[bot.id] = status
            await refreshRemoteState()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func unbindBot(_ bot: GaryxConfiguredBot) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let status = try await client().unbindBot(botId: bot.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            botStatusesById[bot.id] = status
            await refreshRemoteState()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func saveConfiguredBotAccount(
        _ input: GaryxConfiguredBotAccountInput,
        original: GaryxConfiguredBotAccountSettings? = nil
    ) async -> Bool {
        let channel = input.channel.trimmingCharacters(in: .whitespacesAndNewlines)
        let accountId = input.accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !channel.isEmpty else {
            lastError = "Channel is required"
            return false
        }
        guard !accountId.isEmpty else {
            lastError = "Account ID is required"
            return false
        }
        if original == nil,
           configuredBotAccountSettings.contains(where: {
               $0.channel.caseInsensitiveCompare(channel) == .orderedSame && $0.accountId == accountId
           }) {
            lastError = "Bot account already exists"
            return false
        }

        isSavingBotSettings = true
        let runtimeGeneration = gatewayRuntimeGeneration
        defer {
            if runtimeGeneration == gatewayRuntimeGeneration {
                isSavingBotSettings = false
            }
        }
        do {
            var settings = try await client().gatewaySettings()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            let validation = try await client().validateChannelAccount(
                pluginId: channel,
                request: GaryxChannelAccountValidationRequest(
                    accountId: accountId,
                    enabled: input.enabled,
                    config: input.config
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            guard validation.validated else {
                lastError = validation.message
                return false
            }

            guard GaryxConfiguredBotAccountsDocument.setAccount(
                in: &settings,
                originalChannel: original?.channel,
                originalAccountId: original?.accountId,
                input: input
            ) else {
                lastError = "Bot account could not be saved"
                return false
            }
            _ = try await client().saveGatewaySettings(settings, merge: false)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            gatewaySettingsDocument = settings
            await refreshRemoteState()
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    func setConfiguredBotAccountEnabled(_ account: GaryxConfiguredBotAccountSettings, enabled: Bool) async {
        let input = GaryxConfiguredBotAccountInput(
            channel: account.channel,
            accountId: account.accountId,
            displayName: account.displayName,
            enabled: enabled,
            agentId: account.agentId,
            workspaceDir: account.workspaceDir,
            workspaceMode: account.workspaceMode,
            config: account.config
        )
        _ = await saveConfiguredBotAccount(input, original: account)
    }

    func deleteConfiguredBotAccount(_ account: GaryxConfiguredBotAccountSettings) async {
        isSavingBotSettings = true
        let runtimeGeneration = gatewayRuntimeGeneration
        defer {
            if runtimeGeneration == gatewayRuntimeGeneration {
                isSavingBotSettings = false
            }
        }
        do {
            var settings = try await client().gatewaySettings()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            guard GaryxConfiguredBotAccountsDocument.removeAccount(
                from: &settings,
                channel: account.channel,
                accountId: account.accountId
            ) else {
                lastError = "Bot account not found"
                return
            }
            _ = try await client().saveGatewaySettings(settings, merge: false)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            gatewaySettingsDocument = settings
            configuredBots.removeAll {
                $0.channel.caseInsensitiveCompare(account.channel) == .orderedSame
                    && $0.accountId == account.accountId
            }
            channelEndpoints.removeAll { endpoint in
                endpoint.channel.caseInsensitiveCompare(account.channel) == .orderedSame
                    && endpoint.accountId == account.accountId
            }
            botConsoles.removeAll {
                $0.channel.caseInsensitiveCompare(account.channel) == .orderedSame
                    && $0.accountId == account.accountId
            }
            botStatusesById.removeValue(forKey: account.id)
            persistCatalogCacheSnapshot()
            await refreshRemoteState()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func deleteConfiguredBotAccount(_ bot: GaryxConfiguredBot) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            var settings = try await client().gatewaySettings()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            guard GaryxConfiguredBotAccountsDocument.removeAccount(
                from: &settings,
                channel: bot.channel,
                accountId: bot.accountId
            ) else {
                lastError = "Bot account not found"
                return
            }
            _ = try await client().saveGatewaySettings(settings, merge: false)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            configuredBots.removeAll { $0.id == bot.id }
            channelEndpoints.removeAll { endpoint in
                endpoint.channel.caseInsensitiveCompare(bot.channel) == .orderedSame
                    && endpoint.accountId == bot.accountId
            }
            botConsoles.removeAll {
                $0.channel.caseInsensitiveCompare(bot.channel) == .orderedSame
                    && $0.accountId == bot.accountId
            }
            botStatusesById.removeValue(forKey: bot.id)
            persistCatalogCacheSnapshot()
            await refreshRemoteState()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func bindEndpointToSelectedThread(_ endpoint: GaryxChannelEndpoint) async {
        guard let threadId = selectedThread?.id else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().bindChannelEndpoint(endpointKey: endpoint.endpointKey, threadId: threadId)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            await refreshRemoteState()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func detachEndpoint(_ endpoint: GaryxChannelEndpoint) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().detachChannelEndpoint(endpointKey: endpoint.endpointKey)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            await refreshRemoteState()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func archiveBotConversationEndpoint(_ endpoint: GaryxChannelEndpoint) async {
        let threadId = endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !threadId.isEmpty else { return }
        guard canArchiveThreadId(threadId) else {
            lastError = "This thread is active or managed by an automation."
            return
        }
        await archiveThreadRecord(
            threadId: threadId,
            additionalEndpointKey: endpoint.endpointKey
        )
    }

    func archiveThreadRecord(threadId: String, additionalEndpointKey: String? = nil) async {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty else { return }

        var endpointKeys = Set(
            channelEndpoints
                .filter { $0.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) == normalizedThreadId }
                .map(\.endpointKey)
                .filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        )
        let currentEndpointKey = additionalEndpointKey?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !currentEndpointKey.isEmpty {
            endpointKeys.insert(currentEndpointKey)
        }

        let runtimeGeneration = gatewayRuntimeGeneration
        if selectedThread?.id == normalizedThreadId {
            openNewThreadDraft()
        }
        do {
            for endpointKey in endpointKeys {
                _ = try await client().detachChannelEndpoint(endpointKey: endpointKey)
            }
            _ = try await client().deleteThread(threadId: normalizedThreadId)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            removeArchivedThreadLocally(normalizedThreadId)
            channelEndpoints.removeAll { endpoint in
                let endpointThreadId = endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                return endpointKeys.contains(endpoint.endpointKey)
                    || endpointThreadId == normalizedThreadId
            }
            persistCatalogCacheSnapshot()
            if selectedThread?.id == normalizedThreadId {
                selectedThread = nil
                draftThreadTitle = ""
                resetComposerDraft()
                messages = []
                cancelSelectedThreadReconcileLoop()
                resetSelectedThreadHistoryPagination()
            }
            discardPendingAssistantDelta(for: normalizedThreadId)
            messagesByThread[normalizedThreadId] = nil
            messageSignaturesByThread[normalizedThreadId] = nil
            activeAssistantMessageIdsByThread[normalizedThreadId] = nil
            await refreshRemoteState()
            await refreshThreads()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }
}
