import SwiftUI
import UIKit

private struct GaryxPrefersCrossFadeTransitionsKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var garyxPrefersCrossFadeTransitions: Bool {
        get { self[GaryxPrefersCrossFadeTransitionsKey.self] }
        set { self[GaryxPrefersCrossFadeTransitionsKey.self] = newValue }
    }
}

private struct GaryxAccessibilityPreferencesModifier: ViewModifier {
    @State private var prefersCrossFadeTransitions = UIAccessibility.prefersCrossFadeTransitions

    func body(content: Content) -> some View {
        content
            .environment(\.garyxPrefersCrossFadeTransitions, prefersCrossFadeTransitions)
            .task {
                prefersCrossFadeTransitions = UIAccessibility.prefersCrossFadeTransitions
                for await _ in NotificationCenter.default.notifications(
                    named: UIAccessibility.prefersCrossFadeTransitionsStatusDidChange
                ) {
                    guard !Task.isCancelled else { return }
                    prefersCrossFadeTransitions = UIAccessibility.prefersCrossFadeTransitions
                }
            }
    }
}

extension View {
    func garyxAccessibilityPreferences() -> some View {
        modifier(GaryxAccessibilityPreferencesModifier())
    }
}
