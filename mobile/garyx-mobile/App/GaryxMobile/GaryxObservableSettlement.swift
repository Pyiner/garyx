import Foundation

/// Observable projections normally publish in the same call that accepts a
/// semantic state transition. UIKit representable lifecycle callbacks are the
/// exception: SwiftUI can invoke them while it already owns the graph's update
/// access, so only the projection commit must move to the next main-queue turn.
enum GaryxObservableSettlementTiming {
    case immediate
    case afterViewGraphUpdate
}

@MainActor
protocol GaryxObservableSettlementScheduling: AnyObject {
    func schedule(_ action: @escaping @MainActor () -> Void)
}

@MainActor
final class GaryxNextMainQueueObservableSettlementScheduler:
    GaryxObservableSettlementScheduling
{
    static let shared = GaryxNextMainQueueObservableSettlementScheduler()

    private init() {}

    func schedule(_ action: @escaping @MainActor () -> Void) {
        DispatchQueue.main.async {
            action()
        }
    }
}

/// Single owner for a semantic value and its observable projection.
///
/// Deferred commits are coalesced and always read `semanticValue` when they
/// execute. A remount or newer transition can therefore publish immediately
/// without a queued teardown commit later restoring stale state.
@MainActor
final class GaryxObservableStateSettler<Value: Equatable> {
    private(set) var semanticValue: Value

    private var publishedValue: Value
    private var deferredFlushIsScheduled = false
    private var isPublishing = false
    private var immediateFlushRequested = false
    private let scheduler: any GaryxObservableSettlementScheduling
    private let publish: @MainActor (Value) -> Void

    init(
        initialValue: Value,
        scheduler: (any GaryxObservableSettlementScheduling)? = nil,
        publish: @escaping @MainActor (Value) -> Void
    ) {
        semanticValue = initialValue
        publishedValue = initialValue
        self.scheduler = scheduler
            ?? GaryxNextMainQueueObservableSettlementScheduler.shared
        self.publish = publish
    }

    @discardableResult
    func settle(
        _ next: Value,
        timing: GaryxObservableSettlementTiming = .immediate
    ) -> Bool {
        let changed = semanticValue != next
        semanticValue = next

        switch timing {
        case .immediate:
            flushImmediately()
        case .afterViewGraphUpdate:
            scheduleDeferredFlushIfNeeded()
        }
        return changed
    }

    private func scheduleDeferredFlushIfNeeded() {
        guard publishedValue != semanticValue,
              !deferredFlushIsScheduled else { return }
        deferredFlushIsScheduled = true
        scheduler.schedule { [weak self] in
            guard let self else { return }
            deferredFlushIsScheduled = false
            flushImmediately()
        }
    }

    private func flushImmediately() {
        guard !isPublishing else {
            immediateFlushRequested = true
            return
        }

        repeat {
            immediateFlushRequested = false
            guard publishedValue != semanticValue else { return }
            let next = semanticValue
            isPublishing = true
            publish(next)
            isPublishing = false
            publishedValue = next
        } while immediateFlushRequested
    }
}
