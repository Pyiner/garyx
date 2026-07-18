#if DEBUG
import SwiftUI
import UIKit

struct GaryxFluidFakeRouteDebugFixture {
    struct Configuration: Equatable {
        let initialDepth: Int
        let layoutDirection: GaryxRouteLayoutDirection
        let preferences: GaryxRouteVisualPreferences
        let payloadKilobytesPerHost: Int
        let automaticChurnIterations: Int

        static func load(environment: [String: String]) -> Configuration? {
            guard environment["GARYX_MOBILE_FLUID_FAKE_ROUTES"] == "1" else { return nil }
            let depth = min(max(Int(environment["GARYX_MOBILE_FLUID_FAKE_DEPTH"] ?? "2") ?? 2, 0), 100)
            let direction: GaryxRouteLayoutDirection = environment["GARYX_MOBILE_FLUID_FAKE_RTL"] == "1"
                ? .rightToLeft
                : .leftToRight
            let policy = environment["GARYX_MOBILE_FLUID_FAKE_VISUAL_POLICY"] ?? "spatial"
            let preferences: GaryxRouteVisualPreferences = switch policy {
            case "crossFade": .init(reduceMotion: false, prefersCrossFadeTransitions: true)
            case "immediate": .init(reduceMotion: true, prefersCrossFadeTransitions: false)
            default: .init(reduceMotion: false, prefersCrossFadeTransitions: false)
            }
            let payloadKilobytes = min(
                max(Int(environment["GARYX_MOBILE_FLUID_FAKE_PAYLOAD_KB"] ?? "64") ?? 64, 0),
                2_048
            )
            let churnIterations = min(
                max(Int(environment["GARYX_MOBILE_FLUID_FAKE_CHURN"] ?? "0") ?? 0, 0),
                2_000
            )
            return Configuration(
                initialDepth: depth,
                layoutDirection: direction,
                preferences: preferences,
                payloadKilobytesPerHost: payloadKilobytes,
                automaticChurnIterations: churnIterations
            )
        }
    }

    let configuration: Configuration

    static var current: GaryxFluidFakeRouteDebugFixture? {
        Configuration.load(environment: ProcessInfo.processInfo.environment).map(Self.init)
    }

    var view: AnyView {
        AnyView(GaryxFluidFakeRouteRoot(configuration: configuration))
    }
}

@MainActor
private final class GaryxFluidFakeRouteProbe: ObservableObject {
    @Published private(set) var depth: Int
    @Published private(set) var phase = GaryxPresentationTransactionPhase.active
    @Published private(set) var terminalOutcome = "none"
    @Published private(set) var transactionBeginCount = 0
    @Published private(set) var regrabCount = 0
    @Published private(set) var terminalCount = 0
    @Published private(set) var screenChangedCount = 0
    @Published private(set) var mountedHostCount = 0
    @Published private(set) var peakMountedHostCount = 0
    @Published private(set) var homeLeadingEdgeCount = 0
    @Published private(set) var curveCheck = "idle"
    @Published private(set) var curveProgress: CGFloat = 0
    @Published private(set) var gestureDiagnostic = "none"
    @Published private(set) var churnCheck = "idle"
    @Published private(set) var churnCompletedIterations = 0
    @Published private(set) var performanceCheck = "idle"
    @Published private(set) var settleFrameCount = 0
    @Published private(set) var maximumFrameGapMilliseconds: Double = 0
    @Published private(set) var backwardsFrameCount = 0
    @Published private(set) var transitionBodyDelta = 0

    private var bodyBaseline = 0
    private var settleTarget: CGFloat?
    private var settleFrameTimes: [CFTimeInterval] = []
    private var lastRenderedProgress: CGFloat?
    private var measuredBackwardsFrames = 0

    let layoutDirection: GaryxRouteLayoutDirection
    let visualPolicy: GaryxRouteVisualPolicy

    init(configuration: GaryxFluidFakeRouteDebugFixture.Configuration) {
        depth = configuration.initialDepth
        layoutDirection = configuration.layoutDirection
        visualPolicy = configuration.preferences.resolvedPolicy
    }

    func hostMounted() {
        mountedHostCount += 1
        peakMountedHostCount = max(peakMountedHostCount, mountedHostCount)
    }

    func hostUnmounted() {
        mountedHostCount = max(0, mountedHostCount - 1)
    }

    func transitionPhaseChanged(_ phase: GaryxPresentationTransactionPhase) {
        if phase == .preCommit, self.phase == .cancelSettle {
            regrabCount += 1
        }
        self.phase = phase
        if phase == .preCommit {
            transactionBeginCount += 1
            if settleTarget == nil {
                bodyBaseline = GaryxFluidFakeRouteBodyCounter.shared.count
                performanceCheck = "running"
            }
        } else if phase == .commitSettle || phase == .cancelSettle {
            settleTarget = phase == .commitSettle ? 1 : 0
            settleFrameTimes = []
            lastRenderedProgress = nil
            measuredBackwardsFrames = 0
        } else if phase == .terminal {
            settleFrameCount = settleFrameTimes.count
            maximumFrameGapMilliseconds = zip(
                settleFrameTimes,
                settleFrameTimes.dropFirst()
            ).map { ($1 - $0) * 1_000 }.max() ?? 0
            backwardsFrameCount = measuredBackwardsFrames
            transitionBodyDelta = GaryxFluidFakeRouteBodyCounter.shared.count - bodyBaseline
            // The simulator normally presents at 60 Hz even when the device
            // supports ProMotion. Twenty-five milliseconds catches a dropped
            // presentation interval while leaving a little scheduling noise
            // above the measured 18.35 ms acceptance run.
            let frameBudgetPassed = settleFrameCount >= 15
                && maximumFrameGapMilliseconds <= 25
            performanceCheck = frameBudgetPassed
                && backwardsFrameCount == 0
                && transitionBodyDelta == 0
                ? "pass"
                : "fail"
            settleTarget = nil
        }
    }

    func recordTransitionFrame(
        phase: GaryxPresentationTransactionPhase,
        progress: CGFloat,
        timestamp: CFTimeInterval
    ) {
        guard phase == .commitSettle || phase == .cancelSettle,
              let settleTarget
        else { return }
        settleFrameTimes.append(timestamp)
        if let lastRenderedProgress {
            if settleTarget == 1, progress + 0.000_1 < lastRenderedProgress {
                measuredBackwardsFrames += 1
            } else if settleTarget == 0, progress - 0.000_1 > lastRenderedProgress {
                measuredBackwardsFrames += 1
            }
        }
        self.lastRenderedProgress = progress
    }

    func canonicalPathChanged(_ path: [GaryxRouteEntry]) {
        depth = path.count
    }

    func terminalReached(_ terminal: GaryxPresentationTerminalState) {
        terminalCount += 1
        terminalOutcome = "\(terminal.outcome.rawValue)-\(terminal.visibility.rawValue)"
    }

    func screenChanged() {
        screenChangedCount += 1
    }

    func homeLeadingEdge() {
        homeLeadingEdgeCount += 1
    }

    func curveChecking(progress: CGFloat) {
        curveCheck = "running"
        curveProgress = progress
    }

    func curveFinished(passed: Bool) {
        curveCheck = passed ? "pass" : "fail"
        curveProgress = 0
    }

    func recordGestureDiagnostic(_ value: String) {
        let components = gestureDiagnostic == "none"
            ? []
            : gestureDiagnostic.split(separator: ",").map(String.init)
        gestureDiagnostic = (components.suffix(5) + [value]).joined(separator: ",")
    }

    func churnStarted() {
        churnCheck = "running"
        churnCompletedIterations = 0
    }

    func churnProgress(_ iterations: Int) {
        churnCompletedIterations = iterations
    }

    func churnFinished(
        passed: Bool,
        iterations: Int,
        mountedHostCount: Int,
        peakMountedHostCount: Int
    ) {
        churnCheck = passed ? "pass" : "fail"
        churnCompletedIterations = iterations
        self.mountedHostCount = mountedHostCount
        self.peakMountedHostCount = peakMountedHostCount
    }

    var status: String {
        [
            "depth=\(depth)",
            "phase=\(phase.rawValue)",
            "terminal=\(terminalOutcome)",
            "transactions=\(transactionBeginCount)",
            "regrabs=\(regrabCount)",
            "terminals=\(terminalCount)",
            "screenChanged=\(screenChangedCount)",
            "mounted=\(mountedHostCount)",
            "peakMounted=\(peakMountedHostCount)",
            "homeEdges=\(homeLeadingEdgeCount)",
            "direction=\(layoutDirection.rawValue)",
            "policy=\(visualPolicy.rawValue)",
            "curve=\(curveCheck)",
            "curveProgress=\(String(format: "%.2f", Double(curveProgress)))",
            "gesture=\(gestureDiagnostic)",
            "churn=\(churnCheck)",
            "churnIterations=\(churnCompletedIterations)",
            "performance=\(performanceCheck)",
            "settleFrames=\(settleFrameCount)",
            "maxGapMs=\(String(format: "%.2f", maximumFrameGapMilliseconds))",
            "backwards=\(backwardsFrameCount)",
            "bodyDelta=\(transitionBodyDelta)",
            "bodies=\(GaryxFluidFakeRouteBodyCounter.shared.count)",
        ].joined(separator: ";")
    }
}

@MainActor
private final class GaryxFluidFakeRouteActions: ObservableObject {
    weak var container: GaryxRouteStackContainer?
    private weak var probe: GaryxFluidFakeRouteProbe?
    private var nextRouteIndex: Int
    private var slowMotionTask: Task<Void, Never>?
    private var churnTask: Task<Void, Never>?

    init(initialDepth: Int, probe: GaryxFluidFakeRouteProbe) {
        nextRouteIndex = initialDepth
        self.probe = probe
    }

    func attach(_ container: GaryxRouteStackContainer) {
        self.container = container
    }

    func detach(_ container: GaryxRouteStackContainer) {
        if self.container === container { self.container = nil }
        slowMotionTask?.cancel()
        slowMotionTask = nil
        churnTask?.cancel()
        churnTask = nil
    }

    func push() {
        nextRouteIndex += 1
        _ = container?.push(Self.entry(nextRouteIndex))
    }

    func pop() {
        _ = container?.pop()
    }

    func runSlowMotionReference() {
        guard slowMotionTask == nil, let container, !container.path.isEmpty else { return }
        slowMotionTask = Task { @MainActor [weak self, weak container] in
            guard let self, let container, container.beginInteractivePop() else {
                self?.slowMotionTask = nil
                return
            }
            let sourceIdentity = GaryxRoutePresentationIdentity(container.path.last.map {
                GaryxRoutePresentationNode.entry($0)
            } ?? .home)
            let destinationIdentity = GaryxRoutePresentationIdentity(
                container.path.count > 1
                    ? .entry(container.path[container.path.count - 2])
                    : .home
            )
            var passed = true
            for step in 0...18 {
                guard !Task.isCancelled else { return }
                let progress = CGFloat(step) / 40
                container.updateInteractivePop(
                    logicalTranslation: container.view.bounds.width * progress
                )
                probe?.curveChecking(progress: progress)
                container.view.layoutIfNeeded()
                let wrappers = container.view.subviews.compactMap {
                    $0 as? GaryxRouteTransitionWrapperView
                }
                let source = wrappers.first { $0.representedIdentity == sourceIdentity }
                let destination = wrappers.first { $0.representedIdentity == destinationIdentity }
                let expected = GaryxRouteTransitionGeometry.visualState(
                    kind: .pop,
                    policy: container.visualPolicyForActiveTransaction ?? .spatial,
                    progress: progress,
                    viewportWidth: container.view.bounds.width,
                    layoutDirection: container.layoutDirectionOverride ?? .leftToRight
                )
                passed = passed
                    && abs((source?.transform.tx ?? .infinity) - expected.sourceTranslationX) < 0.01
                    && abs((destination?.transform.tx ?? .infinity) - expected.destinationTranslationX) < 0.01
                try? await Task.sleep(nanoseconds: 50_000_000)
            }
            container.cancelInteractivePop()
            container.completeSettleImmediately()
            probe?.curveFinished(passed: passed)
            slowMotionTask = nil
        }
    }

    func runAutomaticChurn(iterations: Int) {
        guard iterations > 0, churnTask == nil else { return }
        churnTask = Task { @MainActor [weak self, weak container] in
            guard let self, let container else { return }
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            guard !Task.isCancelled else { return }
            let baselinePath = container.path
            probe?.churnStarted()
            for index in 0..<iterations {
                guard !Task.isCancelled else { return }
                autoreleasepool {
                    self.nextRouteIndex += 1
                    _ = container.push(Self.entry(self.nextRouteIndex), animated: false)
                    _ = container.pop(animated: false)
                }
                if (index + 1).isMultiple(of: 25) {
                    probe?.churnProgress(index + 1)
                    await Task.yield()
                }
                if iterations > 500, index + 1 == 500 {
                    // A second batch lets the performance harness distinguish
                    // allocator warm-up from unbounded per-transition growth.
                    try? await Task.sleep(nanoseconds: 3_000_000_000)
                }
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
            let metrics = container.metrics
            let passed = container.path == baselinePath
                && metrics.mountedHostCount <= GaryxRouteStackContainer.maximumMountedHostCount
                && metrics.peakMountedHostCount <= GaryxRouteStackContainer.maximumMountedHostCount
                && metrics.stateStore.evictableEntryCount <= 32
                && metrics.stateStore.evictableCostBytes <= 2 * 1_024 * 1_024
                && !container.hasTerminalResidue
            probe?.churnFinished(
                passed: passed,
                iterations: iterations,
                mountedHostCount: metrics.mountedHostCount,
                peakMountedHostCount: metrics.peakMountedHostCount
            )
            churnTask = nil
        }
    }

    static func entry(_ index: Int) -> GaryxRouteEntry {
        GaryxRouteEntry(
            id: .init(rawValue: "fake-route-\(index)"),
            destination: .panel("fake-panel-\(index)")
        )
    }
}

@MainActor
private final class GaryxFluidFakeRouteBodyCounter {
    static let shared = GaryxFluidFakeRouteBodyCounter()
    private(set) var count = 0

    func record() { count += 1 }
}

private struct GaryxFluidFakeRouteRoot: View {
    let configuration: GaryxFluidFakeRouteDebugFixture.Configuration
    @StateObject private var probe: GaryxFluidFakeRouteProbe
    @StateObject private var actions: GaryxFluidFakeRouteActions

    init(configuration: GaryxFluidFakeRouteDebugFixture.Configuration) {
        self.configuration = configuration
        let probe = GaryxFluidFakeRouteProbe(configuration: configuration)
        _probe = StateObject(wrappedValue: probe)
        _actions = StateObject(
            wrappedValue: GaryxFluidFakeRouteActions(
                initialDepth: configuration.initialDepth,
                probe: probe
            )
        )
    }

    var body: some View {
        ZStack(alignment: .bottom) {
            GaryxFluidFakeRouteContainerRepresentable(
                configuration: configuration,
                probe: probe,
                actions: actions
            )
            Text(probe.status)
                .font(.caption2.monospaced())
                .lineLimit(3)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 10))
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
                .accessibilityIdentifier("fluid.fake.status")
                .accessibilityValue(probe.status)
        }
        .environment(
            \.layoutDirection,
            configuration.layoutDirection == .rightToLeft ? .rightToLeft : .leftToRight
        )
    }
}

private struct GaryxFluidFakeRouteContainerRepresentable: UIViewControllerRepresentable {
    let configuration: GaryxFluidFakeRouteDebugFixture.Configuration
    let probe: GaryxFluidFakeRouteProbe
    let actions: GaryxFluidFakeRouteActions

    final class Coordinator {
        let actions: GaryxFluidFakeRouteActions

        init(actions: GaryxFluidFakeRouteActions) {
            self.actions = actions
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(actions: actions)
    }

    func makeUIViewController(context: Context) -> GaryxRouteStackContainer {
        var callbacks = GaryxRouteStackContainerCallbacks()
        callbacks.hostMounted = { [weak probe] _ in
            Task { @MainActor in probe?.hostMounted() }
        }
        callbacks.hostUnmounted = { [weak probe] _ in
            Task { @MainActor in probe?.hostUnmounted() }
        }
        callbacks.phaseChanged = { [weak probe] in probe?.transitionPhaseChanged($0) }
        callbacks.canonicalPathChanged = { [weak probe] in probe?.canonicalPathChanged($0) }
        callbacks.terminalReached = { [weak probe] in probe?.terminalReached($0) }
        callbacks.screenChanged = { [weak probe] view in
            probe?.screenChanged()
            UIAccessibility.post(notification: .screenChanged, argument: view)
        }
        let payloadBytes = configuration.payloadKilobytesPerHost * 1_024
        let container = GaryxRouteStackContainer(
            initialPath: (0..<configuration.initialDepth).map {
                GaryxFluidFakeRouteActions.entry($0 + 1)
            },
            callbacks: callbacks,
            preferencesProvider: { configuration.preferences },
            hostBuilder: { node in
                AnyView(GaryxFluidFakeRoutePage(
                    node: node,
                    actions: actions,
                    retainedPayload: Data(repeating: 0xA4, count: payloadBytes)
                ))
            }
        )
        container.layoutDirectionOverride = configuration.layoutDirection
        container.homeLeadingEdgeAction = { [weak probe] in probe?.homeLeadingEdge() }
        container.trailingEdgeActionEligible = { true }
        container.trailingEdgeAction = {}
        container.gestureDiagnostic = { [weak probe] in probe?.recordGestureDiagnostic($0) }
        container.transitionFrameObserver = { [weak probe] phase, progress, timestamp in
            probe?.recordTransitionFrame(
                phase: phase,
                progress: progress,
                timestamp: timestamp
            )
        }
        actions.attach(container)
        actions.runAutomaticChurn(iterations: configuration.automaticChurnIterations)
        return container
    }

    func updateUIViewController(
        _ uiViewController: GaryxRouteStackContainer,
        context: Context
    ) {}

    static func dismantleUIViewController(
        _ uiViewController: GaryxRouteStackContainer,
        coordinator: Coordinator
    ) {
        coordinator.actions.detach(uiViewController)
    }
}

private struct GaryxFluidFakeRoutePage: View {
    let node: GaryxRoutePresentationNode
    let actions: GaryxFluidFakeRouteActions
    let retainedPayload: Data

    var body: some View {
        GaryxFluidFakeRouteBodyCounter.shared.record()
        return ZStack {
            LinearGradient(
                colors: isHome
                    ? [Color(uiColor: .systemBackground), Color(uiColor: .secondarySystemBackground)]
                    : [Color.indigo.opacity(0.14), Color(uiColor: .systemBackground)],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            .ignoresSafeArea()

            VStack(spacing: 18) {
                Text(title)
                    .font(.largeTitle.bold())
                    .accessibilityIdentifier("fluid.fake.route-title")

                Text("Instrumented fake route • \(retainedPayload.count / 1_024) KB")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)

                HStack(spacing: 12) {
                    Button("Push fake route") { actions.push() }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("fluid.fake.push")
                    Button("Pop fake route") { actions.pop() }
                        .buttonStyle(.bordered)
                        .disabled(isHome)
                        .accessibilityIdentifier("fluid.fake.pop")
                }

                Button("Run slow-motion reference") {
                    actions.runSlowMotionReference()
                }
                .buttonStyle(.bordered)
                .disabled(isHome)
                .accessibilityIdentifier("fluid.fake.slow-motion")

                ScrollView(.horizontal) {
                    HStack(spacing: 12) {
                        ForEach(0..<12, id: \.self) { index in
                            Text("Horizontal \(index)")
                                .frame(width: 120, height: 54)
                                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
                        }
                    }
                    .padding(.horizontal, 8)
                }
                .accessibilityIdentifier("fluid.fake.horizontal-scroll")

                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 10) {
                        ForEach(0..<36, id: \.self) { index in
                            HStack(alignment: .top, spacing: 10) {
                                Circle()
                                    .fill(index.isMultiple(of: 2) ? Color.indigo : Color.secondary)
                                    .frame(width: 24, height: 24)
                                Text("Synthetic conversation row \(index): wrapper transforms must not invalidate this subtree.")
                                    .font(.callout)
                            }
                        }
                    }
                    .padding(.horizontal, 20)
                }
                .accessibilityIdentifier("fluid.fake.vertical-scroll")
            }
            .padding(.top, 24)
        }
    }

    private var isHome: Bool {
        if case .home = node { return true }
        return false
    }

    private var title: String {
        switch node {
        case .home:
            "Fake home"
        case .entry(let entry):
            "Fake route depth \(entry.id.rawValue.replacingOccurrences(of: "fake-route-", with: ""))"
        }
    }
}
#endif
