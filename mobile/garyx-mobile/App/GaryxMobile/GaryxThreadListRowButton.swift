import SwiftUI

struct GaryxThreadListRowInput: Equatable {
    var thread: GaryxThreadSummary
    var presentation: GaryxSidebarThreadRowPresentation
    var avatar: GaryxSidebarThreadRowAvatar?
    var timestampValue: String?
    var capabilities: GaryxThreadRowCapabilities
    var motion: GaryxThreadRowMotion = .stable
    var showsDivider = false
    var isFullBleed = false
    var density: GaryxSidebarThreadRowDensity = .regular
    var selectionDisplay: GaryxSidebarThreadSelectionDisplay = .sidebar
    var indent: CGFloat = 0
    var menuDismissToken = 0
    var menuMovementSuppression = false
    var openSource: GaryxMobilePanelOpenSource
}

/// The single interactive thread row used by Home and every drilldown list.
/// Its value input is equatable; closures are deliberately outside equality.
struct GaryxThreadListRowButton: View, Equatable {
    let input: GaryxThreadListRowInput
    let onOpenThread: (GaryxThreadSummary, GaryxMobilePanelOpenSource) -> Void
    var onPrepareOpen: ((GaryxThreadSummary) -> Void)? = nil
    let onSetPinned: (String, Bool) -> Void
    let onSetFavorite: (String, Bool) -> Void
    let onArchive: (GaryxThreadSummary, GaryxThreadArchiveStrategy) -> Void

    @Environment(\.garyxMotion) private var motion
    @State private var suppressNextPrimaryTap = false

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.input == rhs.input
    }

    var body: some View {
        #if DEBUG
        let _ = GaryxHomeScrollPerformanceProbe.shared.markRowBody()
        #endif
        rowContent
            .padding(.leading, input.indent)
            .frame(height: isExiting ? 0 : nil, alignment: .top)
            .opacity(motion.opacity(.rowRemoval, active: isExiting))
            .scaleEffect(motion.scale(.rowRemoval, active: isExiting), anchor: .trailing)
            .offset(x: motion.offset(.rowRemoval, active: isExiting).width)
            .clipped()
            .allowsHitTesting(!isExiting)
            .accessibilityHidden(isExiting)
            .animation(motion.animation(.rowRemoval), value: isExiting)
    }

    private var rowContent: some View {
        VStack(spacing: 0) {
            if input.showsDivider {
                GaryxSidebarRowDivider()
            }
            GaryxSidebarThreadRowView(
                presentation: input.presentation,
                avatar: input.avatar,
                isFullBleed: input.isFullBleed,
                density: input.density,
                selectionDisplay: input.selectionDisplay,
                liveTimestampValue: input.timestampValue,
                usesExternalSelectionGesture: true,
                onSelect: open,
                onUnpin: input.presentation.isPinned && input.capabilities.canPin
                    ? consumeNestedTapAndUnpin
                    : nil
            )
            .garyxThreadActionMenu(
                dismissToken: input.menuDismissToken,
                movementSuppressesMenu: input.menuMovementSuppression,
                preparePrimaryAction: prepareOpen,
                primaryAction: {
                    DispatchQueue.main.async {
                        guard !suppressNextPrimaryTap else {
                            suppressNextPrimaryTap = false
                            return
                        }
                        open()
                    }
                },
                items: menuItems
            )
        }
    }

    private var actionPlan: [GaryxThreadRowActionKind] {
        GaryxThreadRowActionPlanner.actions(
            capabilities: input.capabilities,
            isPinned: input.presentation.isPinned,
            isFavorite: input.presentation.isFavorite
        )
    }

    private func menuItems() -> [GaryxThreadActionMenuItem] {
        actionPlan.map { action in
            let descriptor = descriptor(for: action)
            return GaryxThreadActionMenuItem(
                title: descriptor.title,
                systemImage: descriptor.systemImage,
                role: descriptor.destructive ? .destructive : .standard,
                isEnabled: actionsEnabled
            ) {
                guard actionsEnabled else { return }
                perform(action)
            }
        }
    }

    private func descriptor(
        for action: GaryxThreadRowActionKind
    ) -> (title: String, systemImage: String, destructive: Bool) {
        switch action {
        case .pin: return ("Pin thread", "pin", false)
        case .unpin: return ("Unpin thread", "pin.slash", false)
        case .favorite: return ("Favorite thread", "star", false)
        case .unfavorite: return ("Unfavorite thread", "star.slash", false)
        case .archive: return ("Archive thread", "archivebox", true)
        }
    }

    private func perform(_ action: GaryxThreadRowActionKind) {
        switch action {
        case .pin:
            onSetPinned(input.thread.id, true)
        case .unpin:
            onSetPinned(input.thread.id, false)
        case .favorite:
            onSetFavorite(input.thread.id, true)
        case .unfavorite:
            onSetFavorite(input.thread.id, false)
        case .archive(let strategy):
            onArchive(input.thread, strategy)
        }
    }

    private func open() {
        guard input.capabilities.canOpen, actionsEnabled else { return }
        prepareOpen()
        onOpenThread(input.thread, input.openSource)
    }

    private func prepareOpen() {
        guard input.capabilities.canOpen, actionsEnabled else { return }
        onPrepareOpen?(input.thread)
    }

    private func consumeNestedTapAndUnpin() {
        guard actionsEnabled else { return }
        suppressNextPrimaryTap = true
        onSetPinned(input.thread.id, false)
        DispatchQueue.main.async {
            DispatchQueue.main.async {
                suppressNextPrimaryTap = false
            }
        }
    }

    private var actionsEnabled: Bool {
        input.motion != .pinning
    }

    private var isExiting: Bool {
        input.motion == .archiving || input.motion == .leavingFilteredList
    }

}
