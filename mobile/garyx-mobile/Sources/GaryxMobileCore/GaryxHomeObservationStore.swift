import Foundation
import Observation

public struct GaryxRootSurfaceOccurrenceID: Hashable, Sendable {
    public let rawValue: UInt64

    public init(rawValue: UInt64) {
        self.rawValue = rawValue
    }
}

public enum GaryxRootSurface: Equatable, Sendable {
    case navigationShell(GaryxRootSurfaceOccurrenceID)
    case gatewaySetup
}

public enum GaryxRootSurfaceOccurrenceTransition: Equatable, Sendable {
    case navigationShellBegan(GaryxRootSurfaceOccurrenceID)
    case navigationShellEnded(GaryxRootSurfaceOccurrenceID)
}

@MainActor
@Observable
public final class GaryxHomeObservationStore {
    @ObservationIgnored public private(set) var publishCount = 0
    @ObservationIgnored private var activeNavigationShellOccurrenceID: GaryxRootSurfaceOccurrenceID?
    @ObservationIgnored private var nextNavigationShellOccurrenceRawValue: UInt64

    public private(set) var isGatewayConfigured: Bool
    public private(set) var connectionState: GaryxMobileConnectionState
    public private(set) var debugShowsGatewaySwitcher: Bool
    public private(set) var showsSettings: Bool
    public private(set) var lastError: String?
    public private(set) var isLoadingMoreThreads: Bool
    public private(set) var hasMoreThreadSummaries: Bool
    public private(set) var loadMoreFooterState: GaryxHomeLoadMoreFooterState

    public init(
        isGatewayConfigured: Bool = false,
        connectionState: GaryxMobileConnectionState = .disconnected,
        debugShowsGatewaySwitcher: Bool = false,
        showsSettings: Bool = false,
        lastError: String? = nil,
        isLoadingMoreThreads: Bool = false,
        hasMoreThreadSummaries: Bool = false,
        loadMoreFooterState: GaryxHomeLoadMoreFooterState = .hidden
    ) {
        let startsWithNavigationShell = Self.resolvesNavigationShell(
            isGatewayConfigured: isGatewayConfigured,
            connectionState: connectionState
        )
        activeNavigationShellOccurrenceID = startsWithNavigationShell
            ? GaryxRootSurfaceOccurrenceID(rawValue: 1)
            : nil
        nextNavigationShellOccurrenceRawValue = startsWithNavigationShell ? 2 : 1
        self.isGatewayConfigured = isGatewayConfigured
        self.connectionState = connectionState
        self.debugShowsGatewaySwitcher = debugShowsGatewaySwitcher
        self.showsSettings = showsSettings
        self.lastError = lastError
        self.isLoadingMoreThreads = isLoadingMoreThreads
        self.hasMoreThreadSummaries = hasMoreThreadSummaries
        self.loadMoreFooterState = loadMoreFooterState
    }

    /// The existing root-view branch expressed as a pure, testable decision.
    /// Changing this value replaces the complete navigation-shell occurrence,
    /// including its UIKit-owned public edge recognizers.
    public var rootSurface: GaryxRootSurface {
        guard Self.resolvesNavigationShell(
            isGatewayConfigured: isGatewayConfigured,
            connectionState: connectionState
        ) else {
            return .gatewaySetup
        }
        guard let activeNavigationShellOccurrenceID else {
            preconditionFailure("navigation shell is missing its occurrence identity")
        }
        return .navigationShell(activeNavigationShellOccurrenceID)
    }

    /// Applies the root branch transition as one ordered ownership boundary.
    /// The callback runs while the old branch is still publicly visible, so an
    /// ending Shell can synchronously release every interaction it owns before
    /// Observation lets SwiftUI replace that occurrence.
    @discardableResult
    public func applyConnection(
        isGatewayConfigured: Bool,
        connectionState: GaryxMobileConnectionState,
        willTransitionRootSurface: (GaryxRootSurfaceOccurrenceTransition) -> Void
    ) -> Bool {
        let currentlyShowsNavigationShell = Self.resolvesNavigationShell(
            isGatewayConfigured: self.isGatewayConfigured,
            connectionState: self.connectionState
        )
        let nextShowsNavigationShell = Self.resolvesNavigationShell(
            isGatewayConfigured: isGatewayConfigured,
            connectionState: connectionState
        )

        if currentlyShowsNavigationShell, !nextShowsNavigationShell {
            guard let activeNavigationShellOccurrenceID else {
                preconditionFailure("ending navigation shell is missing its occurrence identity")
            }
            willTransitionRootSurface(.navigationShellEnded(activeNavigationShellOccurrenceID))
            self.activeNavigationShellOccurrenceID = nil
        } else if !currentlyShowsNavigationShell, nextShowsNavigationShell {
            let occurrenceID = GaryxRootSurfaceOccurrenceID(
                rawValue: nextNavigationShellOccurrenceRawValue
            )
            nextNavigationShellOccurrenceRawValue &+= 1
            willTransitionRootSurface(.navigationShellBegan(occurrenceID))
            activeNavigationShellOccurrenceID = occurrenceID
        }

        var changed = false
        changed = set(\.isGatewayConfigured, to: isGatewayConfigured) || changed
        changed = set(\.connectionState, to: connectionState) || changed
        return changed
    }

    @discardableResult
    public func applyPagination(
        isLoadingMoreThreads: Bool,
        hasMoreThreadSummaries: Bool,
        loadMoreFooterState: GaryxHomeLoadMoreFooterState
    ) -> Bool {
        var changed = false
        changed = set(\.isLoadingMoreThreads, to: isLoadingMoreThreads) || changed
        changed = set(\.hasMoreThreadSummaries, to: hasMoreThreadSummaries) || changed
        changed = set(\.loadMoreFooterState, to: loadMoreFooterState) || changed
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

    private static func resolvesNavigationShell(
        isGatewayConfigured: Bool,
        connectionState: GaryxMobileConnectionState
    ) -> Bool {
        guard isGatewayConfigured, case .ready = connectionState else { return false }
        return true
    }
}
