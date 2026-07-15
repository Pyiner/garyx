import Foundation

struct GaryxPinnedListMove: Equatable, Sendable {
    let order: [String]
    let destination: Int
}

/// Translates SwiftUI's flat `ForEach` move coordinates into the pinned-only
/// identity space. The destination is always clamped into the pinned segment,
/// so dragging across either section boundary cannot move a pinned thread into
/// a header, spacer, or the Recent segment.
enum GaryxPinnedListMoveTranslator {
    static func translate(
        items: [GaryxHomeThreadListItem],
        sourceOffsets: IndexSet,
        destination: Int
    ) -> GaryxPinnedListMove? {
        let pinnedEntries = items.enumerated().compactMap { index, item -> (Int, String)? in
            guard case let .thread(row, region) = item, region == .pinned else { return nil }
            return (index, row.id)
        }
        guard !pinnedEntries.isEmpty, !sourceOffsets.isEmpty else { return nil }

        let pinnedFlatOffsets = pinnedEntries.map(\.0)
        guard sourceOffsets.allSatisfy(pinnedFlatOffsets.contains) else { return nil }

        let firstPinnedOffset = pinnedFlatOffsets[0]
        let relativeSources = IndexSet(sourceOffsets.map { $0 - firstPinnedOffset })
        let relativeDestination = min(
            max(destination - firstPinnedOffset, 0),
            pinnedEntries.count
        )
        let originalOrder = pinnedEntries.map(\.1)
        let moved = relativeSources.map { originalOrder[$0] }
        var order = originalOrder.enumerated().compactMap { index, id in
            relativeSources.contains(index) ? nil : id
        }
        let removedBeforeDestination = relativeSources.filter { $0 < relativeDestination }.count
        let insertionIndex = min(
            max(relativeDestination - removedBeforeDestination, 0),
            order.count
        )
        order.insert(contentsOf: moved, at: insertionIndex)
        return GaryxPinnedListMove(order: order, destination: relativeDestination)
    }
}
