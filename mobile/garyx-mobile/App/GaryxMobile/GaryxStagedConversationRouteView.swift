import Combine
import QuartzCore
import SwiftUI
import UIKit

/// Non-observable lifecycle fan-out owned by one route container. Staged
/// destinations subscribe by occurrence identity, so terminal activation can
/// start their frame policy without invalidating the SwiftUI destination tree
/// through an environment-value change.
@MainActor
final class GaryxRouteLifecycleRegistry {
    typealias Observer = @MainActor (GaryxRouteHostLifecyclePhase) -> Void
    typealias ContentReadyObserver = @MainActor () -> Void
    typealias PresentedFrameDemand = @MainActor () -> Bool
    typealias PresentedFrameObserver = @MainActor (TimeInterval?) -> Void

    private struct PresentedFrameSubscription {
        let demand: PresentedFrameDemand
        let observer: PresentedFrameObserver
    }

    private var phaseByIdentity: [GaryxRoutePresentationIdentity: GaryxRouteHostLifecyclePhase] = [:]
    private var observers: [GaryxRoutePresentationIdentity: [UUID: Observer]] = [:]
    private var contentReadyIdentities = Set<GaryxRoutePresentationIdentity>()
    private var contentReadyObservers: [
        GaryxRoutePresentationIdentity: [UUID: ContentReadyObserver]
    ] = [:]
    private var presentedFrameSubscriptions: [
        GaryxRoutePresentationIdentity: [UUID: PresentedFrameSubscription]
    ] = [:]
    private var previousPresentedFrameTimestamp: CFTimeInterval?

    var presentedFrameDemandDidBecomeActive: (@MainActor () -> Void)? {
        didSet {
            if hasPresentedFrameDemand {
                presentedFrameDemandDidBecomeActive?()
            }
        }
    }
    var contentPreparationDidBegin: (@MainActor (GaryxRoutePresentationIdentity) -> Void)?

    func hostMounted(_ identity: GaryxRoutePresentationIdentity) {
        update(identity, lifecycle: .mounted)
    }

    func update(
        _ identity: GaryxRoutePresentationIdentity,
        lifecycle: GaryxRouteHostLifecyclePhase
    ) {
        guard phaseByIdentity[identity] != lifecycle else { return }
        phaseByIdentity[identity] = lifecycle
        let currentObservers = observers[identity].map { Array($0.values) } ?? []
        for observer in currentObservers {
            observer(lifecycle)
        }
        if hasPresentedFrameDemand {
            presentedFrameDemandDidBecomeActive?()
        }
    }

    func hostUnmounted(_ identity: GaryxRoutePresentationIdentity) {
        phaseByIdentity.removeValue(forKey: identity)
        let removedObservers = observers.removeValue(forKey: identity)
            .map { Array($0.values) } ?? []
        contentReadyIdentities.remove(identity)
        contentReadyObservers.removeValue(forKey: identity)
        presentedFrameSubscriptions.removeValue(forKey: identity)
        for observer in removedObservers {
            observer(.disappeared)
        }
    }

    func observe(
        _ identity: GaryxRoutePresentationIdentity,
        observer: @escaping Observer
    ) -> UUID {
        let token = UUID()
        observers[identity, default: [:]][token] = observer
        observer(phaseByIdentity[identity] ?? .mounted)
        return token
    }

    func removeObserver(_ token: UUID, for identity: GaryxRoutePresentationIdentity) {
        observers[identity]?[token] = nil
        if observers[identity]?.isEmpty == true {
            observers[identity] = nil
        }
    }

    func observeContentReady(
        _ identity: GaryxRoutePresentationIdentity,
        observer: @escaping ContentReadyObserver
    ) -> UUID {
        let token = UUID()
        contentReadyObservers[identity, default: [:]][token] = observer
        if contentReadyIdentities.contains(identity) {
            observer()
        }
        return token
    }

    func removeContentReadyObserver(
        _ token: UUID,
        for identity: GaryxRoutePresentationIdentity
    ) {
        contentReadyObservers[identity]?[token] = nil
        if contentReadyObservers[identity]?.isEmpty == true {
            contentReadyObservers[identity] = nil
        }
    }

    func contentDidBecomeReady(_ identity: GaryxRoutePresentationIdentity) {
        guard contentReadyIdentities.insert(identity).inserted else { return }
        let currentObservers = contentReadyObservers[identity]
            .map { Array($0.values) } ?? []
        for observer in currentObservers {
            observer()
        }
        if hasPresentedFrameDemand {
            previousPresentedFrameTimestamp = nil
            presentedFrameDemandDidBecomeActive?()
        }
    }

    func observePresentedFrames(
        _ identity: GaryxRoutePresentationIdentity,
        demand: @escaping PresentedFrameDemand,
        observer: @escaping PresentedFrameObserver
    ) -> UUID {
        let token = UUID()
        presentedFrameSubscriptions[identity, default: [:]][token] =
            PresentedFrameSubscription(demand: demand, observer: observer)
        if phaseByIdentity[identity] == .active, demand() {
            presentedFrameDemandDidBecomeActive?()
        }
        return token
    }

    func removePresentedFrameObserver(
        _ token: UUID,
        for identity: GaryxRoutePresentationIdentity
    ) {
        presentedFrameSubscriptions[identity]?[token] = nil
        if presentedFrameSubscriptions[identity]?.isEmpty == true {
            presentedFrameSubscriptions[identity] = nil
        }
    }

    var hasPresentedFrameDemand: Bool {
        for (identity, subscriptions) in presentedFrameSubscriptions
        where phaseByIdentity[identity] == .active {
            if subscriptions.values.contains(where: { $0.demand() }) {
                return true
            }
        }
        return false
    }

    func presentedFrame() {
        let timestamp = CACurrentMediaTime()
        let interval = previousPresentedFrameTimestamp.map { timestamp - $0 }
        previousPresentedFrameTimestamp = timestamp
        let activeSubscriptions = presentedFrameSubscriptions.compactMap {
            identity, subscriptions -> [PresentedFrameSubscription]? in
            guard phaseByIdentity[identity] == .active else { return nil }
            return Array(subscriptions.values)
        }.flatMap { $0 }
        for subscription in activeSubscriptions where subscription.demand() {
            subscription.observer(interval)
        }
        if !hasPresentedFrameDemand {
            previousPresentedFrameTimestamp = nil
        }
    }
}

private struct GaryxRouteLifecycleRegistryKey: EnvironmentKey {
    static let defaultValue: GaryxRouteLifecycleRegistry? = nil
}

extension EnvironmentValues {
    var garyxRouteLifecycleRegistry: GaryxRouteLifecycleRegistry? {
        get { self[GaryxRouteLifecycleRegistryKey.self] }
        set { self[GaryxRouteLifecycleRegistryKey.self] = newValue }
    }
}

/// Conversation route owner that keeps first-time SwiftUI/RenderBox work out
/// of the list-to-thread push. The static UIKit placeholder is the complete
/// destination surface until the container reaches terminal. Live content is
/// then prepared behind that opaque surface and revealed only after it has
/// delivered stable frames.
struct GaryxStagedConversationRouteView: View {
    @Environment(\.garyxRouteLifecycleRegistry) private var lifecycleRegistry
    @StateObject private var presentationDriver = GaryxConversationRoutePresentationDriver()

    let destination: GaryxRouteDestination
    let occurrenceID: GaryxRouteInstanceID

    var body: some View {
        ZStack {
            if presentationDriver.renderPhase != .transitionPlaceholder {
                GaryxConversationView(destination: destination)
            }

            if presentationDriver.renderPhase == .transitionPlaceholder {
                // The moving push surface stays isomorphic to the measured
                // zero-hitch plain destination: no representable, material,
                // shape, text, or first-use render pipeline.
                Color(uiColor: .systemBackground)
            } else {
                GaryxConversationTransitionPlaceholder { placeholder in
                    presentationDriver.attachPlaceholder(placeholder)
                }
            }
        }
        .onAppear {
            presentationDriver.connect(
                to: lifecycleRegistry,
                identity: .entry(occurrenceID)
            )
        }
        .onDisappear {
            presentationDriver.disconnect()
        }
    }
}

/// Non-rendering adapter from UIKit's delivered-frame clock to the pure Core
/// presentation policy. Keeping the clock off the SwiftUI destination tree is
/// essential: even an otherwise empty `UIViewRepresentable` incurred a
/// first-use RenderBox commit in the moving push window on iOS 26.
@MainActor
private final class GaryxConversationRoutePresentationDriver: ObservableObject {
    @Published private(set) var renderPhase: GaryxConversationRouteRenderPhase = .transitionPlaceholder
    private var presentation = GaryxConversationRoutePresentationState()
    private weak var lifecycleRegistry: GaryxRouteLifecycleRegistry?
    private var observedIdentity: GaryxRoutePresentationIdentity?
    private var observationToken: UUID?
    private var contentReadyObservationToken: UUID?
    private var frameObservationToken: UUID?
    private var fallbackFrameTask: Task<Void, Never>?
    private weak var placeholderView: GaryxConversationTransitionPlaceholderView?

    func attachPlaceholder(_ view: GaryxConversationTransitionPlaceholderView) {
        placeholderView = view
        if presentation.hasPresentedLiveContent {
            view.revealPreparedContent()
        }
    }

    func connect(
        to registry: GaryxRouteLifecycleRegistry?,
        identity: GaryxRoutePresentationIdentity
    ) {
        if lifecycleRegistry === registry,
           observedIdentity == identity,
           observationToken != nil,
           contentReadyObservationToken != nil,
           frameObservationToken != nil {
            return
        }
        disconnect()
        guard let registry else {
            apply(lifecycle: .active)
            scheduleFallbackFrames()
            return
        }
        lifecycleRegistry = registry
        observedIdentity = identity
        observationToken = registry.observe(identity) { [weak self] lifecycle in
            self?.apply(lifecycle: lifecycle)
        }
        contentReadyObservationToken = registry.observeContentReady(identity) { [weak self] in
            self?.contentDidBecomeReady()
        }
        frameObservationToken = registry.observePresentedFrames(
            identity,
            demand: { [weak self] in
                self?.presentation.needsPresentedFrameClock == true
            },
            observer: { [weak self] interval in
                self?.presentedFrame(interval: interval)
            }
        )
    }

    func disconnect() {
        if let lifecycleRegistry, let observedIdentity, let observationToken {
            lifecycleRegistry.removeObserver(observationToken, for: observedIdentity)
        }
        if let lifecycleRegistry, let observedIdentity, let frameObservationToken {
            lifecycleRegistry.removePresentedFrameObserver(
                frameObservationToken,
                for: observedIdentity
            )
        }
        if let lifecycleRegistry, let observedIdentity, let contentReadyObservationToken {
            lifecycleRegistry.removeContentReadyObserver(
                contentReadyObservationToken,
                for: observedIdentity
            )
        }
        lifecycleRegistry = nil
        observedIdentity = nil
        observationToken = nil
        contentReadyObservationToken = nil
        frameObservationToken = nil
        fallbackFrameTask?.cancel()
        fallbackFrameTask = nil
        if !presentation.hasPresentedLiveContent {
            var next = presentation
            next.apply(lifecycle: .disappeared)
            commit(next)
        }
    }

    func apply(lifecycle: GaryxRouteHostLifecyclePhase) {
        var next = presentation
        next.apply(lifecycle: lifecycle)
        commit(next)
        scheduleFallbackFrames()
    }

    private func contentDidBecomeReady() {
        var next = presentation
        next.contentDidBecomeReady()
        commit(next)
    }

    private func presentedFrame(interval: TimeInterval?) {
        let previous = presentation.renderPhase
        var nextPresentation = presentation
        let next = nextPresentation.presentedFrame(interval: interval)
        commit(nextPresentation)
        guard previous != next else { return }
        switch nextPresentation.renderPhase {
        case .transitionPlaceholder:
            break
        case .preparingLiveContent:
            GaryxRoutePushPerformanceProbe.shared?.liveContentPreparationBegan()
            if let lifecycleRegistry, let observedIdentity {
                lifecycleRegistry.contentPreparationDidBegin?(observedIdentity)
            }
        case .live:
            GaryxRoutePushPerformanceProbe.shared?.placeholderRevealBegan()
            placeholderView?.revealPreparedContent()
        }
    }

    private func scheduleFallbackFrames() {
        // Production route hosts always receive the container registry and its
        // shared settle display link. This task keeps isolated previews and
        // tests functional without installing a competing CADisplayLink.
        guard lifecycleRegistry == nil,
              presentation.needsPresentedFrameClock,
              fallbackFrameTask == nil
        else { return }
        fallbackFrameTask = Task { @MainActor [weak self] in
            while let self,
                  !Task.isCancelled,
                  self.lifecycleRegistry == nil,
                  self.presentation.needsPresentedFrameClock {
                await Task.yield()
                guard !Task.isCancelled else { return }
                self.presentedFrame(interval: 1.0 / 60.0)
            }
            self?.fallbackFrameTask = nil
        }
    }

    /// Lifecycle and per-phase frame counters are policy internals. Publishing
    /// them would reevaluate the destination host at terminal even though its
    /// visible surface remains the same plain placeholder. SwiftUI observes
    /// only the placeholder-to-preparation mount; UIKit owns the live reveal.
    private func commit(_ next: GaryxConversationRoutePresentationState) {
        guard next != presentation else { return }
        presentation = next
        // Publishing `.live` would reevaluate the complete conversation tree
        // at the reveal boundary. The opaque UIKit cover instead hides its one
        // precomposited layer directly; SwiftUI only observes the earlier
        // placeholder -> preparation mount transition.
        if renderPhase != next.renderPhase, next.renderPhase != .live {
            renderPhase = next.renderPhase
        }
    }
}

/// One opaque UIKit draw pass instead of a SwiftUI header/transcript/composer
/// subtree. It intentionally has no shimmer or continuous animation: if the
/// first live display-list commit is expensive, the user sees a stable staged
/// snapshot rather than a frozen moving affordance.
private struct GaryxConversationTransitionPlaceholder: UIViewRepresentable {
    let didMount: @MainActor (GaryxConversationTransitionPlaceholderView) -> Void

    func makeUIView(context: Context) -> GaryxConversationTransitionPlaceholderView {
        let view = GaryxConversationTransitionPlaceholderView()
        didMount(view)
        return view
    }

    func updateUIView(
        _ uiView: GaryxConversationTransitionPlaceholderView,
        context: Context
    ) {
        didMount(uiView)
        uiView.setNeedsDisplay()
    }
}

@MainActor
private final class GaryxConversationTransitionPlaceholderView: UIView {
    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = true
        isUserInteractionEnabled = true
        isAccessibilityElement = true
        accessibilityViewIsModal = true
        accessibilityLabel = "Opening conversation"
        backgroundColor = .systemBackground
        contentMode = .redraw
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) {
            (view: GaryxConversationTransitionPlaceholderView, _) in
            view.setNeedsDisplay()
        }
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func revealPreparedContent() {
        guard !isHidden else { return }
        accessibilityViewIsModal = false
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        isHidden = true
    }

    override func draw(_ rect: CGRect) {
        guard let context = UIGraphicsGetCurrentContext() else { return }
        UIColor.systemBackground.setFill()
        context.fill(bounds)

        let faint = UIColor.label.withAlphaComponent(0.065)
        let stronger = UIColor.label.withAlphaComponent(0.09)
        let horizontalInset: CGFloat = 18
        let contentWidth = max(0, bounds.width - horizontalInset * 2)
        let safeTop = safeAreaInsets.top
        let safeBottom = safeAreaInsets.bottom

        fillRoundedRect(
            CGRect(x: horizontalInset, y: safeTop + 15, width: 34, height: 34),
            radius: 17,
            color: stronger
        )
        fillRoundedRect(
            CGRect(x: horizontalInset + 48, y: safeTop + 24, width: 126, height: 15),
            radius: 7.5,
            color: stronger
        )
        fillRoundedRect(
            CGRect(x: bounds.width - horizontalInset - 34, y: safeTop + 15, width: 34, height: 34),
            radius: 17,
            color: faint
        )

        let transcriptTop = safeTop + 104
        fillRoundedRect(
            CGRect(x: bounds.width - horizontalInset - 156, y: transcriptTop, width: 156, height: 38),
            radius: 19,
            color: faint
        )
        fillRoundedRect(
            CGRect(x: horizontalInset, y: transcriptTop + 66, width: contentWidth - 28, height: 13),
            radius: 6.5,
            color: faint
        )
        fillRoundedRect(
            CGRect(x: horizontalInset, y: transcriptTop + 88, width: contentWidth - 82, height: 13),
            radius: 6.5,
            color: faint
        )
        fillRoundedRect(
            CGRect(x: horizontalInset, y: transcriptTop + 110, width: contentWidth - 142, height: 13),
            radius: 6.5,
            color: faint
        )

        let composerHeight: CGFloat = 82
        let composerY = max(transcriptTop + 150, bounds.height - safeBottom - composerHeight - 14)
        fillRoundedRect(
            CGRect(
                x: horizontalInset,
                y: composerY,
                width: contentWidth,
                height: composerHeight
            ),
            radius: 26,
            color: stronger
        )
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        setNeedsDisplay()
    }

    private func fillRoundedRect(
        _ rect: CGRect,
        radius: CGFloat,
        color: UIColor
    ) {
        guard rect.width > 0, rect.height > 0 else { return }
        color.setFill()
        UIBezierPath(roundedRect: rect, cornerRadius: radius).fill()
    }
}
