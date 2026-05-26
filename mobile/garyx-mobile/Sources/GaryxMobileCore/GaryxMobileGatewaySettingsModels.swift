import Foundation

struct GaryxGatewayProfile: Identifiable, Codable, Equatable {
    var id: String
    var label: String
    var gatewayUrl: String
    var updatedAt: Date
    var hasToken: Bool
}

enum GaryxGatewaySetupConnectionPhase: Equatable {
    case disconnected
    case checking
    case failed
    case ready
}

enum GaryxGatewaySetupPresentation {
    static func showsDetails(
        isSheet: Bool,
        startsEmpty: Bool,
        hasGatewaySettings: Bool,
        phase: GaryxGatewaySetupConnectionPhase
    ) -> Bool {
        if isSheet || startsEmpty { return true }
        if !hasGatewaySettings { return true }
        switch phase {
        case .disconnected, .checking, .failed:
            return true
        case .ready:
            return false
        }
    }
}

enum GaryxGatewayProfileStorage {
    static func load(defaults: UserDefaults, key: String) -> [GaryxGatewayProfile] {
        guard let data = defaults.data(forKey: key) else {
            return []
        }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        guard let profiles = try? decoder.decode([GaryxGatewayProfile].self, from: data) else {
            return []
        }
        return normalizedProfiles(profiles)
    }

    static func normalizedProfiles(_ profiles: [GaryxGatewayProfile]) -> [GaryxGatewayProfile] {
        var byKey: [String: GaryxGatewayProfile] = [:]
        for profile in profiles {
            let url = normalizedURL(profile.gatewayUrl)
            guard !url.isEmpty else { continue }
            let key = url.lowercased()
            var normalized = profile
            normalized.gatewayUrl = url
            normalized.id = stableId(for: url)
            normalized.label = profile.label.trimmingCharacters(in: .whitespacesAndNewlines)
            if normalized.label.isEmpty {
                normalized.label = label(for: url)
            }
            if let current = byKey[key], current.updatedAt >= normalized.updatedAt {
                continue
            }
            byKey[key] = normalized
        }
        return byKey.values
            .sorted { $0.updatedAt > $1.updatedAt }
            .prefix(8)
            .map { $0 }
    }

    static func normalizedURL(_ value: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        return trimmed.replacingOccurrences(
            of: "/+$",
            with: "",
            options: .regularExpression
        )
    }

    static func stableId(for gatewayUrl: String) -> String {
        var hash: UInt64 = 14695981039346656037
        for byte in gatewayUrl.lowercased().utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 1099511628211
        }
        return String(format: "gateway::%016llx", hash)
    }

    static func label(for gatewayUrl: String) -> String {
        guard let url = URL(string: gatewayUrl) else {
            return gatewayUrl
        }
        if let host = url.host, let port = url.port {
            return "\(host):\(port)"
        }
        return url.host ?? gatewayUrl
    }
}

struct GaryxConfiguredBotAccountSettings: Identifiable, Equatable {
    var id: String { "\(channel):\(accountId)" }
    var channel: String
    var accountId: String
    var displayName: String
    var enabled: Bool
    var agentId: String?
    var workspaceDir: String?
    var workspaceMode: String?
    var config: [String: GaryxJSONValue]
}

struct GaryxConfiguredBotAccountInput: Equatable {
    var channel: String
    var accountId: String
    var displayName: String
    var enabled: Bool
    var agentId: String?
    var workspaceDir: String?
    var workspaceMode: String?
    var config: [String: GaryxJSONValue]
    var configEditedKeys: Set<String>? = nil

    func mergingFetchedConfigForCachedProjection(_ fetchedConfig: [String: GaryxJSONValue]) -> Self {
        var next = self
        var mergedConfig = fetchedConfig
        let configPatch: [String: GaryxJSONValue]
        if let configEditedKeys {
            for key in configEditedKeys {
                mergedConfig.removeValue(forKey: key)
            }
            configPatch = config.filter { configEditedKeys.contains($0.key) }
        } else {
            configPatch = config
        }
        for (key, value) in configPatch {
            mergedConfig[key] = value
        }
        next.config = mergedConfig
        return next
    }
}

enum GaryxConfiguredBotAccountsDocument {
    static func accounts(from settings: [String: GaryxJSONValue]) -> [GaryxConfiguredBotAccountSettings] {
        guard let channels = settings["channels"]?.garyxSettingsObjectValue else { return [] }
        var accounts: [GaryxConfiguredBotAccountSettings] = []
        for (channel, channelValue) in channels where channel != "api" {
            guard let channelConfig = channelValue.garyxSettingsObjectValue,
                  let accountValues = channelConfig.garyxSettingsObject(forKeys: ["accounts"]) else {
                continue
            }
            for (accountId, rawAccount) in accountValues {
                guard let account = rawAccount.garyxSettingsObjectValue else { continue }
                let name = account.garyxSettingsString(forKeys: ["name"])
                let config = account.garyxSettingsObject(forKeys: ["config"]) ?? [:]
                accounts.append(
                    GaryxConfiguredBotAccountSettings(
                        channel: channel,
                        accountId: accountId,
                        displayName: name ?? accountId,
                        enabled: account.garyxSettingsBool(forKeys: ["enabled"]) ?? true,
                        agentId: account.garyxSettingsString(forKeys: ["agent_id", "agentId"]),
                        workspaceDir: account.garyxSettingsString(forKeys: ["workspace_dir", "workspaceDir"]),
                        workspaceMode: account.garyxSettingsString(forKeys: ["workspace_mode", "workspaceMode"]),
                        config: config
                    )
                )
            }
        }
        return accounts.sorted { lhs, rhs in
            let channelOrder = lhs.channel.localizedCaseInsensitiveCompare(rhs.channel)
            if channelOrder != .orderedSame {
                return channelOrder == .orderedAscending
            }
            let nameOrder = lhs.displayName.localizedCaseInsensitiveCompare(rhs.displayName)
            if nameOrder != .orderedSame {
                return nameOrder == .orderedAscending
            }
            return lhs.accountId.localizedCaseInsensitiveCompare(rhs.accountId) == .orderedAscending
        }
    }

    static func removeAccount(
        from settings: inout [String: GaryxJSONValue],
        channel: String,
        accountId: String
    ) -> Bool {
        guard var channels = settings["channels"]?.garyxSettingsObjectValue else { return false }
        let channelKey = channels.keys.first {
            $0.caseInsensitiveCompare(channel.trimmingCharacters(in: .whitespacesAndNewlines)) == .orderedSame
        }
        guard let channelKey,
              var channelConfig = channels[channelKey]?.garyxSettingsObjectValue,
              var accounts = channelConfig["accounts"]?.garyxSettingsObjectValue,
              accounts.keys.contains(accountId) else {
            return false
        }

        accounts.removeValue(forKey: accountId)
        channelConfig["accounts"] = .object(accounts)
        channels[channelKey] = .object(channelConfig)
        settings["channels"] = .object(channels)
        return true
    }

    static func setAccount(
        in settings: inout [String: GaryxJSONValue],
        originalChannel: String?,
        originalAccountId: String?,
        input: GaryxConfiguredBotAccountInput
    ) -> Bool {
        let channel = input.channel.trimmingCharacters(in: .whitespacesAndNewlines)
        let accountId = input.accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !channel.isEmpty, !accountId.isEmpty else { return false }

        var channels = settings["channels"]?.garyxSettingsObjectValue ?? [:]
        if let originalChannel,
           let originalAccountId,
           (originalChannel.caseInsensitiveCompare(channel) != .orderedSame || originalAccountId != accountId) {
            _ = removeAccount(from: &settings, channel: originalChannel, accountId: originalAccountId)
            channels = settings["channels"]?.garyxSettingsObjectValue ?? channels
        }

        var channelConfig = channels[channel]?.garyxSettingsObjectValue ?? [:]
        var accounts = channelConfig["accounts"]?.garyxSettingsObjectValue ?? [:]
        var account: [String: GaryxJSONValue] = [
            "enabled": .bool(input.enabled),
            "config": .object(input.config),
        ]
        if let name = input.displayName.garyxSettingsTrimmedNilIfEmpty {
            account["name"] = .string(name)
        }
        if let agentId = input.agentId?.garyxSettingsTrimmedNilIfEmpty {
            account["agent_id"] = .string(agentId)
        }
        if let workspaceDir = input.workspaceDir?.garyxSettingsTrimmedNilIfEmpty {
            account["workspace_dir"] = .string(workspaceDir)
        }
        if let workspaceMode = input.workspaceMode?.garyxSettingsTrimmedNilIfEmpty {
            account["workspace_mode"] = .string(workspaceMode)
        }
        accounts[accountId] = .object(account)
        channelConfig["accounts"] = .object(accounts)
        channels[channel] = .object(channelConfig)
        settings["channels"] = .object(channels)
        return true
    }
}

private extension GaryxJSONValue {
    var garyxSettingsObjectValue: [String: GaryxJSONValue]? {
        if case .object(let value) = self {
            return value
        }
        return nil
    }

    var garyxSettingsStringValue: String? {
        switch self {
        case .string(let value):
            return value.garyxSettingsTrimmedNilIfEmpty
        case .number(let value):
            if value.rounded() == value,
               let exactInteger = Int(exactly: value) {
                return String(exactInteger)
            }
            return String(value).garyxSettingsTrimmedNilIfEmpty
        case .bool(let value):
            return value ? "true" : "false"
        case .null, .array, .object:
            return nil
        }
    }

    var garyxSettingsBoolValue: Bool? {
        switch self {
        case .bool(let value):
            return value
        case .string(let value):
            let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if ["true", "yes", "1"].contains(normalized) {
                return true
            }
            if ["false", "no", "0"].contains(normalized) {
                return false
            }
            return nil
        default:
            return nil
        }
    }
}

private extension Dictionary where Key == String, Value == GaryxJSONValue {
    func garyxSettingsString(forKeys keys: [String]) -> String? {
        for key in keys {
            if let value = self[key]?.garyxSettingsStringValue?.garyxSettingsTrimmedNilIfEmpty {
                return value
            }
        }
        return nil
    }

    func garyxSettingsBool(forKeys keys: [String]) -> Bool? {
        for key in keys {
            if let value = self[key]?.garyxSettingsBoolValue {
                return value
            }
        }
        return nil
    }

    func garyxSettingsObject(forKeys keys: [String]) -> [String: GaryxJSONValue]? {
        for key in keys {
            if let value = self[key]?.garyxSettingsObjectValue {
                return value
            }
        }
        return nil
    }
}

private extension String {
    var garyxSettingsTrimmedNilIfEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
