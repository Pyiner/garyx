import SwiftUI

struct GaryxRecentThreadFilterMenu: View {
    let selection: GaryxRecentThreadFilter
    let onSelect: (GaryxRecentThreadFilter) -> Void

    var body: some View {
        Menu {
            Picker(
                "Recent filter",
                selection: Binding(
                    get: { selection },
                    set: onSelect
                )
            ) {
                ForEach(GaryxRecentThreadFilter.homeMenuOptions, id: \.self) { filter in
                    Text(filter.displayName).tag(filter)
                }
            }
            .pickerStyle(.inline)
            .labelsHidden()
        } label: {
            ZStack(alignment: .topTrailing) {
                Image(systemName: "line.3.horizontal.decrease")
                    .font(GaryxFont.system(size: 16, weight: .semibold))
                    .foregroundStyle(.primary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                if selection.activeStatusLabel != nil {
                    Circle()
                        .fill(Color.primary)
                        .frame(width: 7, height: 7)
                        .overlay {
                            Circle().stroke(GaryxTheme.header, lineWidth: 1.5)
                        }
                        .offset(x: -6, y: 6)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .frame(width: 44, height: 44)
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: true,
                fallbackMaterial: .ultraThinMaterial,
                in: Circle()
            )
            .contentShape(Circle())
        }
        .menuOrder(.fixed)
        .menuIndicator(.hidden)
        .buttonStyle(.plain)
        .accessibilityLabel("Recent filter")
        .accessibilityValue(selection.displayName)
    }
}
