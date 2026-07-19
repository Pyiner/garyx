import Foundation

public enum GaryxHapticImpact: String, Equatable, Sendable {
    case light
    case medium
}

public enum GaryxHapticNotification: String, Equatable, Sendable {
    case success
    case error
}

public enum GaryxHapticPattern: Equatable, Sendable {
    case impact(GaryxHapticImpact)
    case notification(GaryxHapticNotification)
    case selection
}

/// The earliest causal point where UIKit should warm the generator used by an
/// eventual feedback event. Preparation never emits feedback; the matching
/// commit still owns the actual haptic.
public enum GaryxHapticPreparationPoint: String, Equatable, Sendable {
    case touchDown
    case gestureBegan
    case operationBegan
}

public struct GaryxHapticSpecification: Equatable, Sendable {
    public let pattern: GaryxHapticPattern
    public let preparationPoint: GaryxHapticPreparationPoint

    public init(
        pattern: GaryxHapticPattern,
        preparationPoint: GaryxHapticPreparationPoint
    ) {
        self.pattern = pattern
        self.preparationPoint = preparationPoint
    }
}

/// Semantic haptic events shared by the app's UIKit adapter and audit tests.
///
/// These are meaningful commits, successful landings, and terminal outcomes;
/// ordinary navigation taps and passive state updates deliberately have no
/// event. Visual state and `play` must be written in the same main-run-loop
/// transaction at each call site.
public enum GaryxHapticEvent: String, CaseIterable, Hashable, Sendable {
    case messageSendCommitted
    case threadPinChanged
    case threadFavoriteChanged
    case capsuleFavoriteChanged
    case capsuleDismissCommitted
    case interactiveBackCommitted
    case messageActionMenuPresented
    case rowSwipeFullyRevealed
    case clipboardCopySucceeded
    case avatarGenerationSucceeded
    case avatarGenerationFailed
    case drawerVisibilityCommitted
    case taskTreeVisibilityCommitted
    case pinnedOrderDropCommitted

    public var specification: GaryxHapticSpecification {
        switch self {
        case .messageSendCommitted:
            GaryxHapticSpecification(
                pattern: .impact(.light),
                preparationPoint: .touchDown
            )
        case .threadPinChanged, .threadFavoriteChanged, .capsuleFavoriteChanged:
            GaryxHapticSpecification(
                pattern: .selection,
                preparationPoint: .touchDown
            )
        case .capsuleDismissCommitted:
            GaryxHapticSpecification(
                pattern: .impact(.medium),
                preparationPoint: .gestureBegan
            )
        case .interactiveBackCommitted:
            GaryxHapticSpecification(
                pattern: .impact(.light),
                preparationPoint: .gestureBegan
            )
        case .messageActionMenuPresented:
            GaryxHapticSpecification(
                pattern: .impact(.light),
                preparationPoint: .gestureBegan
            )
        case .rowSwipeFullyRevealed:
            GaryxHapticSpecification(
                pattern: .impact(.medium),
                preparationPoint: .gestureBegan
            )
        case .clipboardCopySucceeded:
            GaryxHapticSpecification(
                pattern: .notification(.success),
                preparationPoint: .touchDown
            )
        case .avatarGenerationSucceeded:
            GaryxHapticSpecification(
                pattern: .notification(.success),
                preparationPoint: .operationBegan
            )
        case .avatarGenerationFailed:
            GaryxHapticSpecification(
                pattern: .notification(.error),
                preparationPoint: .operationBegan
            )
        case .drawerVisibilityCommitted, .taskTreeVisibilityCommitted:
            GaryxHapticSpecification(
                pattern: .impact(.light),
                preparationPoint: .gestureBegan
            )
        case .pinnedOrderDropCommitted:
            GaryxHapticSpecification(
                pattern: .selection,
                preparationPoint: .gestureBegan
            )
        }
    }
}
