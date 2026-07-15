import Foundation

final class GaryxPinnedOrderUserDefaultsStore: GaryxPinnedOrderOutboxPersisting {
    private let defaults: UserDefaults
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    func loadPinnedOrderOutbox(gatewayIdentity: String) -> GaryxPinnedOrderOutbox? {
        guard let data = defaults.data(forKey: key(gatewayIdentity)) else { return nil }
        return try? decoder.decode(GaryxPinnedOrderOutbox.self, from: data)
    }

    func savePinnedOrderOutbox(
        _ outbox: GaryxPinnedOrderOutbox?,
        gatewayIdentity: String
    ) {
        let key = key(gatewayIdentity)
        guard let outbox else {
            defaults.removeObject(forKey: key)
            return
        }
        guard let data = try? encoder.encode(outbox) else { return }
        defaults.set(data, forKey: key)
    }

    private func key(_ gatewayIdentity: String) -> String {
        let identity = gatewayIdentity.trimmingCharacters(in: .whitespacesAndNewlines)
        let scope = identity.isEmpty ? "unconfigured" : identity
        return "\(GaryxMobileSettingsKeys.pinnedOrderOutbox).\(scope)"
    }
}
