import XCTest
import Combine
@testable import GaryxMobileCore

/// TASK-1037 diagnosis — invalid `objectWillChange` publishes.
///
/// The home view holds the whole model via `@EnvironmentObject`, so any
/// `@Published` mutation fires `objectWillChange` and recomputes the home body
/// (and the O(n) section derivation). These tests reproduce the two assignment
/// patterns the app uses and count the resulting `objectWillChange` emissions
/// with a Combine sink — no UI.
@MainActor
final class GaryxHomePublishStormTests: XCTestCase {

    /// Mirrors the two real assignment patterns:
    ///  - `applyAgentTargets` (GaryxMobileModel+StateSync.swift:13-20):
    ///    `agents = next` with NO Equatable diff — publishes on identical content.
    ///  - `refreshThreads` (GaryxMobileModel+Threads.swift:215):
    ///    `if threads != next { threads = next }` — gated, no publish when equal.
    final class PublishProbeModel: ObservableObject {
        @Published var threads: [GaryxThreadSummary] = []
        @Published var agents: [GaryxAgentSummary] = []

        func applyThreadsGated(_ next: [GaryxThreadSummary]) {
            if threads != next { threads = next } // refreshThreads:215
        }

        func applyAgentsUngated(_ next: [GaryxAgentSummary]) {
            agents = next // applyAgentTargets:14
        }
    }

    func testUngatedAgentAssignmentPublishesOnIdenticalContent() {
        let model = PublishProbeModel()
        var publishes = 0
        let cancellable = model.objectWillChange.sink { publishes += 1 }
        defer { cancellable.cancel() }

        let agents = GaryxHomeListFixture.makeInputs().agents
        model.applyAgentsUngated(agents) // 1 (genuine first set)
        let afterFirst = publishes

        // Catalog stale-while-refresh re-applies identical agents repeatedly.
        for _ in 0..<20 {
            model.applyAgentsUngated(agents)
        }
        let wasted = publishes - afterFirst
        print("[TASK-1037] agents (ungated, applyAgentTargets): 20 identical re-applies -> \(wasted) wasted objectWillChange (each recomputes the O(n) home sections)")
        XCTAssertEqual(wasted, 20, "Ungated assignment publishes on every identical re-apply")
    }

    func testGatedThreadAssignmentDoesNotPublishOnIdenticalContent() {
        let model = PublishProbeModel()
        let threads = GaryxHomeListFixture.makeInputs().threads
        model.applyThreadsGated(threads) // genuine first set

        var publishes = 0
        let cancellable = model.objectWillChange.sink { publishes += 1 }
        defer { cancellable.cancel() }

        // 40 background reconciles (1.5s loop over ~60s) returning identical data.
        for _ in 0..<40 {
            model.applyThreadsGated(threads)
        }
        print("[TASK-1037] threads (gated, refreshThreads:215): 40 identical reconciles -> \(publishes) objectWillChange (gate holds)")
        XCTAssertEqual(publishes, 0, "Gated assignment must not publish when content is identical")
    }

    /// End-to-end recompute accounting that ties publishes to the O(n) section
    /// derivation, comparing CURRENT (recompute on every publish) vs PROPOSED
    /// (Equatable section cache).
    ///
    /// Scenario: 60s in the foreground with ONE thread actively streaming at
    /// ~5 events/sec. Each event: runStateByThread mutates (publish) AND the
    /// whole `threads` array is rebuilt via applyThreadRunStateSummary (publish).
    func testActiveRunStormRecomputeCountCurrentVsProposed() {
        var input = GaryxHomeListFixture.makeInputs(runningCount: 1)
        let cache = HomeThreadSectionsCache()

        // Prime: initial render computes once for both.
        var currentRecomputes = 1
        _ = HomeThreadSectionsReference.build(input)
        _ = cache.sections(for: input)

        let eventCount = 300 // ~5/sec * 60s
        for tick in 0..<eventCount {
            // applyTranscriptRunState changed -> runStateByThread publish +
            // applyThreadRunStateSummary rebuilds `threads` -> threads publish.
            // Two publishes per event in the current model; the home body
            // recomputes the sections on each.
            input.threads[0].runState = (tick % 2 == 0) ? "running" : "idle"
            input.threads[0].activeRunId = (tick % 2 == 0) ? "run-\(tick)" : nil

            // CURRENT: each publish recomputes the O(n) sections. Two publishes.
            currentRecomputes += 2
            _ = HomeThreadSectionsReference.build(input)
            _ = HomeThreadSectionsReference.build(input)

            // PROPOSED: section identity excludes run state -> 0 section recomputes;
            // only a row-scoped running signal updates.
            _ = cache.sections(for: input)
        }

        print("[TASK-1037] ACTIVE-RUN storm (300 events / 60s, 1 running thread): CURRENT section recomputes = \(currentRecomputes) | PROPOSED = \(cache.computeCount)")
        XCTAssertEqual(currentRecomputes, 1 + eventCount * 2)
        XCTAssertEqual(cache.computeCount, 1, "Run-state-decoupled section cache survives the storm")
    }

    /// Idle scenario: with the threads gate holding and an Equatable section
    /// cache, a quiet foreground minute should cost ZERO section recomputes,
    /// versus the current path which recomputes whenever ANY of the model's ~80
    /// @Published fires (e.g. an ungated catalog re-apply, a bot-status poll).
    func testIdleMinuteRecomputeCountCurrentVsProposed() {
        let input = GaryxHomeListFixture.makeInputs()
        let cache = HomeThreadSectionsCache()
        _ = cache.sections(for: input) // initial render

        // Model a quiet minute: 40x 1.5s background reconcile (threads gated ->
        // no publish), but 6 unrelated ungated publishes land from other
        // subsystems (catalog stale-while-refresh, bot status, workspace git).
        let unrelatedPublishes = 6
        var currentRecomputes = 0
        for _ in 0..<unrelatedPublishes {
            // CURRENT: an unrelated publish still recomputes home sections,
            // because the home view observes the entire model.
            currentRecomputes += 1
            _ = HomeThreadSectionsReference.build(input)
            // PROPOSED: section identity unchanged -> served from cache.
            _ = cache.sections(for: input)
        }

        print("[TASK-1037] IDLE minute (6 unrelated publishes): CURRENT section recomputes = \(currentRecomputes) | PROPOSED = \(cache.computeCount - 1)")
        XCTAssertEqual(currentRecomputes, unrelatedPublishes)
        XCTAssertEqual(cache.computeCount, 1, "No genuine section change -> no recompute beyond initial render")
    }
}
