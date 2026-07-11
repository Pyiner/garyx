import SwiftUI

struct GaryxRecentThreadFilterPicker: View {
    let selection: GaryxRecentThreadFilter
    let onSelect: (GaryxRecentThreadFilter) -> Void

    var body: some View {
        Picker(
            "Recent filter",
            selection: Binding(
                get: { selection },
                set: onSelect
            )
        ) {
            ForEach(GaryxRecentThreadFilter.allCases, id: \.self) { filter in
                Text(filter.displayName).tag(filter)
            }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .accessibilityLabel("Recent filter")
        .frame(maxWidth: .infinity)
        .frame(minHeight: 44)
        .padding(.horizontal, 18)
    }
}
