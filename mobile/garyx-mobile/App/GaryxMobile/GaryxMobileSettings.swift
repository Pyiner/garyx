import Foundation
import Security

enum GaryxMobileSettingsKeys {
    static let gatewayUrl = "garyx.gatewayUrl"
    static let legacyGatewayURL = "garyx.mobile.gatewayURL"
    static let legacyGatewayToken = "garyx.mobile.gatewayToken"
    static let selectedAgentTargetId = "garyx.mobile.selectedAgentTargetId"
    static let newThreadWorkspace = "garyx.mobile.newThreadWorkspace"
    static let newThreadWorkspaceMode = "garyx.mobile.newThreadWorkspaceMode"
    static let userWorkspacePaths = "garyx.mobile.userWorkspacePaths"
    static let catalogCacheSnapshot = "garyx.mobile.catalogCacheSnapshot"
    static let pinnedThreadIds = "garyx.mobile.pinnedThreadIds"
    static let lastOpenedThreadId = "garyx.mobile.lastOpenedThreadId"
    static let lastSessionOnThread = "garyx.mobile.lastSessionOnThread"
    static let gatewayProfiles = "garyx.mobile.gatewayProfiles"
    static let keychainService = "com.garyx.mobile"
    static let gatewayAuthToken = "gatewayAuthToken"
    static let gatewayProfileTokenPrefix = "gatewayProfileToken."
}

final class GaryxMobileKeychain {
    static let shared = GaryxMobileKeychain()

    func readGatewayAuthToken() -> String {
        read(
            service: GaryxMobileSettingsKeys.keychainService,
            account: GaryxMobileSettingsKeys.gatewayAuthToken
        ) ?? ""
    }

    func saveGatewayAuthToken(_ token: String) {
        let trimmed = token.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            delete(
                service: GaryxMobileSettingsKeys.keychainService,
                account: GaryxMobileSettingsKeys.gatewayAuthToken
            )
            return
        }
        save(
            trimmed,
            service: GaryxMobileSettingsKeys.keychainService,
            account: GaryxMobileSettingsKeys.gatewayAuthToken
        )
    }

    func readGatewayProfileToken(profileId: String) -> String {
        read(
            service: GaryxMobileSettingsKeys.keychainService,
            account: GaryxMobileSettingsKeys.gatewayProfileTokenPrefix + profileId
        ) ?? ""
    }

    func saveGatewayProfileToken(_ token: String, profileId: String) {
        let account = GaryxMobileSettingsKeys.gatewayProfileTokenPrefix + profileId
        let trimmed = token.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            delete(service: GaryxMobileSettingsKeys.keychainService, account: account)
            return
        }
        save(trimmed, service: GaryxMobileSettingsKeys.keychainService, account: account)
    }

    func deleteGatewayProfileToken(profileId: String) {
        delete(
            service: GaryxMobileSettingsKeys.keychainService,
            account: GaryxMobileSettingsKeys.gatewayProfileTokenPrefix + profileId
        )
    }

    private func read(service: String, account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess,
              let data = item as? Data,
              let value = String(data: data, encoding: .utf8) else {
            return nil
        }
        return value
    }

    private func save(_ value: String, service: String, account: String) {
        let data = Data(value.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            var item = query
            item[kSecValueData as String] = data
            item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            SecItemAdd(item as CFDictionary, nil)
        }
    }

    private func delete(service: String, account: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }
}
