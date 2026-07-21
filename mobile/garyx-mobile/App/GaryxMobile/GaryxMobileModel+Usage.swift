import Foundation
import WidgetKit

extension GaryxMobileModel {
    /// Warm the coding-usage widget from the app: fetch the latest usage over the
    /// authenticated gateway client and persist only the numeric snapshot into the
    /// shared App Group, then reload the widget. The widget never fetches or holds
    /// the gateway token itself — the app owns the network call. Failures are
    /// non-fatal: the widget keeps its last app-warmed snapshot.
    func refreshCodingUsageWidget(
        runtimeGeneration: GaryxGatewayRequestToken? = nil
    ) async {
        let observedGeneration = runtimeGeneration ?? gatewayRequestToken
        guard let gateway = try? client() else { return }
        guard let usage = try? await gateway.codingUsage() else { return }
        guard observedGeneration == gatewayRequestToken else { return }
        codingUsage = usage
        GaryxUsageWidgetStore.saveSnapshot(
            GaryxUsageWidgetSnapshot(usage: usage, fetchedAt: Date())
        )
        WidgetCenter.shared.reloadTimelines(ofKind: GaryxCodingUsageWidgetConstants.kind)
    }
}
