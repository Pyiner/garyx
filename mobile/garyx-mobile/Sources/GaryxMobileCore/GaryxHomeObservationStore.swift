import Foundation
import Observation

@MainActor
@Observable
public final class GaryxHomeObservationStore {
    @ObservationIgnored public private(set) var publishCount = 0

    public private(set) var isGatewayConfigured: Bool
    public private(set) var connectionState: GaryxMobileConnectionState
    public private(set) var debugShowsGatewaySwitcher: Bool
    public private(set) var showsSettings: Bool
    public private(set) var lastError: String?
    public private(set) var isLoadingMoreThreads: Bool
    public private(set) var hasMoreThreadSummaries: Bool

    public init(
        isGatewayConfigured: Bool = false,
        connectionState: GaryxMobileConnectionState = .disconnected,
        debugShowsGatewaySwitcher: Bool = false,
        showsSettings: Bool = false,
        lastError: String? = nil,
        isLoadingMoreThreads: Bool = false,
        hasMoreThreadSummaries: Bool = false
    ) {
        self.isGatewayConfigured = isGatewayConfigured
        self.connectionState = connectionState
        self.debugShowsGatewaySwitcher = debugShowsGatewaySwitcher
        self.showsSettings = showsSettings
        self.lastError = lastError
        self.isLoadingMoreThreads = isLoadingMoreThreads
        self.hasMoreThreadSummaries = hasMoreThreadSummaries
    }

    @discardableResult
    public func applyConnection(
        isGatewayConfigured: Bool,
        connectionState: GaryxMobileConnectionState
    ) -> Bool {
        var changed = false
        changed = set(\.isGatewayConfigured, to: isGatewayConfigured) || changed
        changed = set(\.connectionState, to: connectionState) || changed
        return changed
    }

    @discardableResult
    public func applyPagination(
        isLoadingMoreThreads: Bool,
        hasMoreThreadSummaries: Bool
    ) -> Bool {
        var changed = false
        changed = set(\.isLoadingMoreThreads, to: isLoadingMoreThreads) || changed
        changed = set(\.hasMoreThreadSummaries, to: hasMoreThreadSummaries) || changed
        return changed
    }

    @discardableResult
    public func setDebugShowsGatewaySwitcher(_ value: Bool) -> Bool {
        set(\.debugShowsGatewaySwitcher, to: value)
    }

    @discardableResult
    public func setShowsSettings(_ value: Bool) -> Bool {
        set(\.showsSettings, to: value)
    }

    @discardableResult
    public func setLastError(_ value: String?) -> Bool {
        set(\.lastError, to: value)
    }

    @discardableResult
    private func set<Value: Equatable>(
        _ keyPath: ReferenceWritableKeyPath<GaryxHomeObservationStore, Value>,
        to value: Value
    ) -> Bool {
        guard self[keyPath: keyPath] != value else { return false }
        self[keyPath: keyPath] = value
        publishCount += 1
        return true
    }
}
