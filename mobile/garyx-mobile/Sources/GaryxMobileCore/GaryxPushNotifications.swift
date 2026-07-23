import Foundation

public enum GaryxPushEnvironment: String, Codable, Equatable, Sendable {
    case development
    case production

    public static func forBuild(isDebugBuild: Bool) -> Self {
        isDebugBuild ? .development : .production
    }
}

public enum GaryxPushKind: String, Equatable, Sendable {
    case manual
}

public struct GaryxPushPayload: Equatable, Sendable {
    public var version: Int
    public var kind: GaryxPushKind
    public var threadID: String?

    public init(version: Int, kind: GaryxPushKind, threadID: String?) {
        self.version = version
        self.kind = kind
        self.threadID = threadID
    }
}

public enum GaryxPushRoute: Equatable, Sendable {
    case appHome
    case thread(String)
}

public enum GaryxPushForegroundPresentation: Equatable, Sendable {
    case present
    case suppress
}

public enum GaryxPushPayloadParser {
    public static func parse(userInfo: [AnyHashable: Any]) -> GaryxPushPayload? {
        guard let garyx = stringKeyedDictionary(userInfo["garyx"]),
              let version = integer(garyx["v"]),
              version == 1,
              let rawKind = garyx["kind"] as? String,
              let kind = GaryxPushKind(rawValue: rawKind) else {
            return nil
        }
        let threadID = (garyx["thread_id"] as? String)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return GaryxPushPayload(
            version: version,
            kind: kind,
            threadID: threadID.flatMap { $0.isEmpty ? nil : $0 }
        )
    }

    public static func route(for payload: GaryxPushPayload) -> GaryxPushRoute {
        payload.threadID.map(GaryxPushRoute.thread) ?? .appHome
    }

    public static func foregroundPresentation(
        for payload: GaryxPushPayload?,
        openThreadID: String?
    ) -> GaryxPushForegroundPresentation {
        guard let notificationThreadID = payload?.threadID,
              let openThreadID = openThreadID?
                .trimmingCharacters(in: .whitespacesAndNewlines),
              !openThreadID.isEmpty,
              notificationThreadID == openThreadID else {
            return .present
        }
        return .suppress
    }

    private static func stringKeyedDictionary(_ value: Any?) -> [String: Any]? {
        if let dictionary = value as? [String: Any] {
            return dictionary
        }
        guard let dictionary = value as? [AnyHashable: Any] else {
            return nil
        }
        var result: [String: Any] = [:]
        for (key, value) in dictionary {
            guard let key = key as? String else { return nil }
            result[key] = value
        }
        return result
    }

    private static func integer(_ value: Any?) -> Int? {
        if let value = value as? Int {
            return value
        }
        return (value as? NSNumber)?.intValue
    }
}

public enum GaryxPushDeviceToken {
    public static func hexadecimal(_ data: Data) -> String {
        data.map { String(format: "%02x", $0) }.joined()
    }
}

public struct GaryxPushDeviceRegistration: Equatable, Sendable {
    public var token: String
    public var environment: GaryxPushEnvironment
    public var bundleID: String
    public var deviceName: String?

    public init(
        token: String,
        environment: GaryxPushEnvironment,
        bundleID: String,
        deviceName: String? = nil
    ) {
        self.token = token.trimmingCharacters(in: .whitespacesAndNewlines)
        self.environment = environment
        self.bundleID = bundleID.trimmingCharacters(in: .whitespacesAndNewlines)
        self.deviceName = deviceName?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .nilIfEmpty
    }

    fileprivate var isValid: Bool {
        !token.isEmpty && !bundleID.isEmpty
    }
}

public struct GaryxPushRegistrationKey: Equatable, Sendable {
    public var targetID: String
    public var device: GaryxPushDeviceRegistration

    public init(targetID: String, device: GaryxPushDeviceRegistration) {
        self.targetID = targetID
        self.device = device
    }
}

public enum GaryxPushRegistrationEvent: Equatable, Sendable {
    case gatewayChanged(String?)
    case deviceTokenReceived(GaryxPushDeviceRegistration)
    case foregrounded
    case registrationFinished(GaryxPushRegistrationKey, succeeded: Bool)
}

public enum GaryxPushRegistrationAction: Equatable, Sendable {
    case register(GaryxPushRegistrationKey)
    case unregister(targetID: String, token: String)
}

public struct GaryxPushRegistrationState: Equatable, Sendable {
    public private(set) var currentTargetID: String?
    public private(set) var device: GaryxPushDeviceRegistration?
    public private(set) var registrationInFlight: GaryxPushRegistrationKey?
    public private(set) var lastSuccessfulRegistration: GaryxPushRegistrationKey?

    public init() {}

    public mutating func handle(
        _ event: GaryxPushRegistrationEvent
    ) -> [GaryxPushRegistrationAction] {
        switch event {
        case .gatewayChanged(let rawTargetID):
            let targetID = rawTargetID?
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .nilIfEmpty
            guard targetID != currentTargetID else { return [] }
            let oldTargetID = currentTargetID
            currentTargetID = targetID
            registrationInFlight = nil
            if lastSuccessfulRegistration?.targetID != targetID {
                lastSuccessfulRegistration = nil
            }
            var actions: [GaryxPushRegistrationAction] = []
            if let oldTargetID, let device {
                actions.append(.unregister(targetID: oldTargetID, token: device.token))
            }
            if let registration = registrationAction(force: false) {
                actions.append(registration)
            }
            return actions

        case .deviceTokenReceived(let nextDevice):
            guard nextDevice.isValid else { return [] }
            let oldDevice = device
            device = nextDevice
            if lastSuccessfulRegistration?.device != nextDevice {
                lastSuccessfulRegistration = nil
            }
            if registrationInFlight?.device != nextDevice {
                registrationInFlight = nil
            }
            var actions: [GaryxPushRegistrationAction] = []
            if let oldDevice,
               oldDevice.token != nextDevice.token,
               let currentTargetID {
                actions.append(
                    .unregister(targetID: currentTargetID, token: oldDevice.token)
                )
            }
            if let registration = registrationAction(force: false) {
                actions.append(registration)
            }
            return actions

        case .foregrounded:
            return registrationAction(force: true).map { [$0] } ?? []

        case .registrationFinished(let key, let succeeded):
            guard registrationInFlight == key else { return [] }
            registrationInFlight = nil
            if succeeded,
               currentTargetID == key.targetID,
               device == key.device {
                lastSuccessfulRegistration = key
            }
            return []
        }
    }

    private mutating func registrationAction(
        force: Bool
    ) -> GaryxPushRegistrationAction? {
        guard let currentTargetID, let device, device.isValid else {
            return nil
        }
        let key = GaryxPushRegistrationKey(
            targetID: currentTargetID,
            device: device
        )
        guard registrationInFlight != key else { return nil }
        if !force, lastSuccessfulRegistration == key {
            return nil
        }
        registrationInFlight = key
        return .register(key)
    }
}

public struct GaryxPushDeviceRegistrationRequest: Encodable, Equatable, Sendable {
    public var token: String
    public var platform: String
    public var environment: GaryxPushEnvironment
    public var bundleID: String
    public var deviceName: String?

    public init(device: GaryxPushDeviceRegistration) {
        token = device.token
        platform = "ios"
        environment = device.environment
        bundleID = device.bundleID
        deviceName = device.deviceName
    }

    enum CodingKeys: String, CodingKey {
        case token
        case platform
        case environment
        case bundleID = "bundle_id"
        case deviceName = "device_name"
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
