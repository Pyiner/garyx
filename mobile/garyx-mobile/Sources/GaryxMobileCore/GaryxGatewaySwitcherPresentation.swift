import Foundation

enum GaryxGatewaySwitcherStatus: Equatable, Sendable {
    case connected
    case connecting
    case failed
    case notConnected
}

/// Root-sidebar gateway identity: what the header control shows for the
/// currently selected gateway. Non-interactive when no gateway is configured.
struct GaryxGatewaySwitcherIdentity: Equatable, Sendable {
    let title: String
    let subtitle: String?
    let status: GaryxGatewaySwitcherStatus
    let isInteractive: Bool
}

struct GaryxGatewaySwitcherRow: Equatable, Identifiable, Sendable {
    let id: String
    let title: String
    let subtitle: String
    let isCurrent: Bool
    /// Nil for the synthetic row representing a configured gateway that has no
    /// saved profile yet.
    let profileId: String?
}

enum GaryxGatewaySwitcherPresentation {
    static let unconfiguredTitle = "Garyx"

    static func status(for connectionState: GaryxMobileConnectionState) -> GaryxGatewaySwitcherStatus {
        switch connectionState {
        case .ready:
            .connected
        case .checking:
            .connecting
        case .failed:
            .failed
        case .disconnected:
            .notConnected
        }
    }

    static func statusLabel(for connectionState: GaryxMobileConnectionState) -> String {
        switch connectionState {
        case .ready:
            "Connected"
        case .checking:
            "Connecting"
        case .failed:
            "Connection failed"
        case .disconnected:
            "Not connected"
        }
    }

    static func identity(
        gatewayURL: String,
        profileLabel: String?,
        connectionState: GaryxMobileConnectionState
    ) -> GaryxGatewaySwitcherIdentity {
        let normalized = GaryxGatewayProfileStorage.normalizedURL(gatewayURL)
        guard !normalized.isEmpty else {
            return GaryxGatewaySwitcherIdentity(
                title: unconfiguredTitle,
                subtitle: nil,
                status: .notConnected,
                isInteractive: false
            )
        }
        let hostLabel = GaryxGatewayProfileStorage.label(for: normalized)
        let trimmedLabel = profileLabel?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let title = trimmedLabel.isEmpty ? hostLabel : trimmedLabel
        let statusText = statusLabel(for: connectionState)
        let subtitle = title.caseInsensitiveCompare(hostLabel) == .orderedSame
            ? statusText
            : "\(hostLabel) · \(statusText)"
        return GaryxGatewaySwitcherIdentity(
            title: title,
            subtitle: subtitle,
            status: status(for: connectionState),
            isInteractive: true
        )
    }

    /// Switcher rows keep the saved-profile order (most recently used first)
    /// except that the current gateway always leads, including a synthetic row
    /// when the current gateway was never saved as a profile.
    static func rows(
        profiles: [GaryxGatewayProfile],
        currentGatewayURL: String
    ) -> [GaryxGatewaySwitcherRow] {
        let normalizedCurrent = GaryxGatewayProfileStorage.normalizedURL(currentGatewayURL)
        let currentKey = normalizedCurrent.lowercased()
        var rows = profiles.map { profile in
            GaryxGatewaySwitcherRow(
                id: profile.id,
                title: profile.label,
                subtitle: profile.gatewayUrl,
                isCurrent: !currentKey.isEmpty && profile.gatewayUrl.lowercased() == currentKey,
                profileId: profile.id
            )
        }
        if let currentIndex = rows.firstIndex(where: \.isCurrent) {
            let current = rows.remove(at: currentIndex)
            rows.insert(current, at: 0)
        } else if !currentKey.isEmpty {
            rows.insert(
                GaryxGatewaySwitcherRow(
                    id: "gateway-switcher::current-unsaved",
                    title: GaryxGatewayProfileStorage.label(for: normalizedCurrent),
                    subtitle: normalizedCurrent,
                    isCurrent: true,
                    profileId: nil
                ),
                at: 0
            )
        }
        return rows
    }
}
