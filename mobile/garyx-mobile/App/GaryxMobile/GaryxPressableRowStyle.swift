import SwiftUI

/// Shared pointer-down feedback for plain interactive rows and controls.
///
/// The P1-C `press` token owns scale, opacity, timing, and accessibility
/// resolution. Reduce Motion therefore removes spatial scaling centrally while
/// retaining the short opacity response that communicates touch-down.
struct GaryxPressableRowStyle: ButtonStyle {
    @Environment(\.garyxMotion) private var motion

    private let preparedHaptic: GaryxHapticEvent?

    init(prepares haptic: GaryxHapticEvent? = nil) {
        preparedHaptic = haptic
    }

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(motion.scale(.press, active: configuration.isPressed))
            .opacity(motion.opacity(.press, active: configuration.isPressed))
            .animation(motion.animation(.press), value: configuration.isPressed)
            .onChange(of: configuration.isPressed) { _, isPressed in
                guard isPressed, let preparedHaptic else { return }
                GaryxMobileHaptics.shared.prepare(preparedHaptic)
            }
    }
}
