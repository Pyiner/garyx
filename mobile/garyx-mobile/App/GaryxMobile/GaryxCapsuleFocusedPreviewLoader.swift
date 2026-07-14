import Foundation

enum GaryxCapsulePreviewScenePhase: Equatable {
    case active
    case inactive
    case background
}

struct GaryxCapsulePreviewSceneSignal: Equatable {
    var phase: GaryxCapsulePreviewScenePhase = .active
    var generation: UInt64 = 0

    mutating func publish(_ nextPhase: GaryxCapsulePreviewScenePhase) {
        phase = nextPhase
        generation &+= 1
    }
}

struct GaryxFocusedCapsuleHTMLRequestGate: Equatable {
    private(set) var key: GaryxCapsulePreviewLoadKey?
    private(set) var token: UUID?

    mutating func begin(_ nextKey: GaryxCapsulePreviewLoadKey) -> UUID {
        let nextToken = UUID()
        key = nextKey
        token = nextToken
        return nextToken
    }

    mutating func invalidate() {
        key = nil
        token = nil
    }

    func accepts(key candidateKey: GaryxCapsulePreviewLoadKey, token candidateToken: UUID) -> Bool {
        key == candidateKey && token == candidateToken
    }
}

enum GaryxFocusedCapsuleHTMLAttempt: Equatable {
    case html(String)
    case deleted
    case stale
}

/// App orchestration for one visible focused preview. Rendered HTML and request
/// state deliberately live in separate properties: a failed rev-2 request does
/// not displace already-rendered rev-1 HTML. The injected sleeper makes all
/// retry/cancellation interleavings deterministic in `GaryxMobileTests`.
@MainActor
final class GaryxCapsuleFocusedPreviewLoader: ObservableObject {
    typealias Sleeper = @Sendable (TimeInterval) async throws -> Void

    @Published private(set) var renderedContent: GaryxCapsulePreviewRenderedContent?
    @Published private(set) var loadStatus = GaryxCapsulePreviewLoadStatus()

    private let retryPolicy: GaryxCapsulePreviewRetryPolicy
    private let sleeper: Sleeper
    private var activeTask: Task<Void, Never>?
    private var activeTaskToken: UUID?
    private var retryState = GaryxCapsulePreviewRetryState()
    private var startsNextCycleFromSceneActive = false

    init(
        retryPolicy: GaryxCapsulePreviewRetryPolicy = .default,
        sleeper: @escaping Sleeper = { delay in
            let nanoseconds = UInt64(min(delay, 60 * 60) * 1_000_000_000)
            try await Task.sleep(nanoseconds: nanoseconds)
        }
    ) {
        self.retryPolicy = retryPolicy
        self.sleeper = sleeper
    }

    func reconcile(key: GaryxCapsulePreviewLoadKey, model: GaryxMobileModel) async {
        activeTask?.cancel()
        model.invalidateFocusedCapsuleHTMLRequests()

        let taskToken = UUID()
        activeTaskToken = taskToken
        let requestToken = model.beginFocusedCapsuleHTMLRequest(for: key)
        let worker = Task { @MainActor [weak self, weak model] in
            guard let self, let model else { return }
            do {
                try await self.runCycle(key: key, requestToken: requestToken, model: model)
            } catch is CancellationError {
                // Cancellation is a control signal. Scene cancellation already
                // published `.paused`; key changes/dismissal must write nothing.
            } catch {
                // Every non-cancellation attempt error is classified inside
                // `runCycle`; this is intentionally unreachable.
            }
        }
        activeTask = worker

        await withTaskCancellationHandler {
            await worker.value
        } onCancel: {
            worker.cancel()
        }

        if activeTaskToken == taskToken {
            activeTask = nil
            activeTaskToken = nil
        }
    }

    func cancelForScene(
        model: GaryxMobileModel,
        event: GaryxCapsulePreviewRetryEvent
    ) {
        precondition(event == .sceneInactive || event == .sceneBackground)
        GaryxCapsulePreviewRetryReducer.reduce(
            state: &retryState,
            event: event,
            policy: retryPolicy
        )
        let hadActiveWork = activeTask != nil
            || loadStatus.phase == .loading
            || loadStatus.failure?.isRetryable == true
        activeTask?.cancel()
        model.invalidateFocusedCapsuleHTMLRequests()
        if hadActiveWork, loadStatus.requestedKey != nil, loadStatus.phase != .loaded,
           loadStatus.phase != .deleted {
            loadStatus.phase = .paused
        }
    }

    func cancelForDismiss(model: GaryxMobileModel) {
        activeTask?.cancel()
        model.invalidateFocusedCapsuleHTMLRequests()
    }

    func needsForegroundResume(for key: GaryxCapsulePreviewLoadKey) -> Bool {
        loadStatus.needsForegroundResume(for: key)
    }

    func prepareForegroundResume() {
        startsNextCycleFromSceneActive = true
    }

    private func runCycle(
        key: GaryxCapsulePreviewLoadKey,
        requestToken: UUID,
        model: GaryxMobileModel
    ) async throws {
        retryState = GaryxCapsulePreviewRetryState()
        let cycleStart: GaryxCapsulePreviewRetryEvent = startsNextCycleFromSceneActive
            ? .sceneActive
            : .beginCycle
        startsNextCycleFromSceneActive = false
        GaryxCapsulePreviewRetryReducer.reduce(
            state: &retryState,
            event: cycleStart,
            policy: retryPolicy
        )
        guard model.acceptsFocusedCapsuleHTMLRequest(key: key, token: requestToken) else { return }
        loadStatus = GaryxCapsulePreviewLoadStatus(
            requestedKey: key,
            attempt: 0,
            phase: .loading
        )

        while true {
            try Task.checkCancellation()
            GaryxCapsulePreviewRetryReducer.reduce(
                state: &retryState,
                event: .attemptStarted,
                policy: retryPolicy
            )
            guard model.acceptsFocusedCapsuleHTMLRequest(key: key, token: requestToken) else { return }
            loadStatus.attempt = retryState.networkAttempt
            loadStatus.phase = .loading

            do {
                let result = try await model.loadFocusedCapsulePreviewHTMLAttempt(
                    key: key,
                    token: requestToken
                )
                try Task.checkCancellation()
                guard model.acceptsFocusedCapsuleHTMLRequest(key: key, token: requestToken) else { return }
                switch result {
                case let .html(html):
                    renderedContent = GaryxCapsulePreviewRenderedContent(
                        html: html,
                        revision: key.projectedRevision
                    )
                    loadStatus.phase = .loaded
                    loadStatus.failure = nil
                    loadStatus.retryExhausted = false
                    GaryxCapsulePreviewRetryReducer.reduce(
                        state: &retryState,
                        event: .succeeded,
                        policy: retryPolicy
                    )
                    return
                case .deleted:
                    renderedContent = nil
                    loadStatus.phase = .deleted
                    loadStatus.failure = GaryxCapsulePreviewFailure(
                        kind: .deleted,
                        message: "Capsule deleted."
                    )
                    GaryxCapsulePreviewRetryReducer.reduce(
                        state: &retryState,
                        event: .deleted,
                        policy: retryPolicy
                    )
                    return
                case .stale:
                    return
                }
            } catch {
                if GaryxGatewayRetryClassifier.isCancellation(error) {
                    throw CancellationError()
                }
                guard model.acceptsFocusedCapsuleHTMLRequest(key: key, token: requestToken) else { return }
                guard let failure = GaryxCapsulePreviewFailure.classify(error) else {
                    throw CancellationError()
                }
                let effect = GaryxCapsulePreviewRetryReducer.reduce(
                    state: &retryState,
                    event: .failed(failure),
                    policy: retryPolicy
                )
                loadStatus.phase = failure.kind == .deleted ? .deleted : .failed
                loadStatus.failure = failure
                loadStatus.retryExhausted = retryState.phase == .exhausted

                guard case let .retry(delay) = effect else { return }
                try await sleeper(delay)
                try Task.checkCancellation()
                guard model.acceptsFocusedCapsuleHTMLRequest(key: key, token: requestToken) else { return }
                GaryxCapsulePreviewRetryReducer.reduce(
                    state: &retryState,
                    event: .retryDelayElapsed,
                    policy: retryPolicy
                )
            }
        }
    }
}
