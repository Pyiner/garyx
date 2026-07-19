import SwiftUI

struct GaryxHomeNewThreadFab: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "plus.bubble")
                .font(GaryxFont.system(size: 20, weight: .semibold))
                .foregroundStyle(Color(.systemBackground))
                .frame(width: 56, height: 56)
                .background(Color(.label), in: Circle())
                .contentShape(Circle())
                .shadow(color: .black.opacity(0.18), radius: 16, x: 0, y: 8)
                .shadow(color: .black.opacity(0.08), radius: 3, x: 0, y: 1)
        }
        .buttonStyle(GaryxHomeFabPressStyle())
        .accessibilityLabel("New chat")
    }
}

private struct GaryxHomeFabPressStyle: ButtonStyle {
    @Environment(\.garyxMotion) private var motion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(motion.scale(.floatingPress, active: configuration.isPressed))
            .opacity(motion.opacity(.floatingPress, active: configuration.isPressed))
            .animation(motion.animation(.floatingPress), value: configuration.isPressed)
    }
}
