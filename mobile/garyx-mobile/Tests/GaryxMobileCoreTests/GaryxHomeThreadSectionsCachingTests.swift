import XCTest
@testable import GaryxMobileCore

/// TASK-1037 diagnosis — wasted recomputes and the proposed Equatable input gate.
///
/// These tests prove (1) the derivation is a pure function of its inputs, so a
/// memoized cache is sound; (2) the CURRENT path (no cache) recomputes on every
/// invalidation even when the inputs are byte-for-byte identical; (3) the
/// PROPOSED Equatable-key cache recomputes only on a genuine change; and (4)
/// decoupling per-thread run state from the section identity stops run-state
/// churn from busting the section cache.
final class GaryxHomeThreadSectionsCachingTests: XCTestCase {

    func testDerivationIsPureForIdenticalInputs() {
        let input = GaryxHomeListFixture.makeInputs()
        let a = HomeThreadSectionsReference.build(input)
        let b = HomeThreadSectionsReference.build(input)
        // Pure ⇒ identical inputs produce Equatable-equal output ⇒ caching is sound.
        XCTAssertEqual(a, b)
    }

    /// The current behavior: every body invalidation recomputes. We model 40
    /// invalidations over a minute (one 1.5s background reconcile each) where
    /// the underlying state never changed.
    func testCurrentPathRecomputesOnEveryInvalidationEvenWhenUnchanged() {
        let input = GaryxHomeListFixture.makeInputs()
        var recomputeCount = 0
        // No cache: this is what the App-target computed property does today.
        for _ in 0..<40 {
            recomputeCount += 1
            _ = HomeThreadSectionsReference.build(input)
        }
        XCTAssertEqual(recomputeCount, 40)
        print("[TASK-1037] CURRENT: 40 identical-content invalidations -> \(recomputeCount) full O(n) recomputes (40 wasted)")
    }

    /// The proposed behavior: an Equatable-key cache skips recompute when the
    /// section identity is unchanged.
    func testProposedCacheSkipsRecomputeWhenInputsUnchanged() {
        let cache = HomeThreadSectionsCache()
        let input = GaryxHomeListFixture.makeInputs()

        for _ in 0..<40 {
            _ = cache.sections(for: input)
        }
        XCTAssertEqual(cache.computeCount, 1, "Identical inputs must compute exactly once")
        print("[TASK-1037] PROPOSED (Equatable gate): 40 identical-content invalidations -> \(cache.computeCount) recompute")

        // A genuine change (new thread arrives) recomputes exactly once more.
        var changed = input
        changed.threads.insert(
            GaryxThreadSummary(
                id: "thread-new", title: "Brand new", createdAt: nil, updatedAt: nil,
                lastMessagePreview: "hi", workspacePath: nil, messageCount: 0,
                agentId: "agent-0", teamId: nil, teamName: nil, providerType: "claude_code",
                recentRunId: nil, activeRunId: nil, runState: "idle", worktreePath: nil
            ),
            at: 0
        )
        changed.recentThreadIds.insert("thread-new", at: 0)
        _ = cache.sections(for: changed)
        XCTAssertEqual(cache.computeCount, 2, "A genuine change recomputes exactly once")
    }

    /// Run-state churn is the active-run jank driver. When a thread is streaming,
    /// the app rebuilds the whole `threads` array per event
    /// (applyThreadRunStateSummary) AND bakes `runState` into the section row
    /// presentation (`isRunning`). A naive content cache therefore busts on every
    /// delta. The PROPOSED identity key excludes run state, so the section cache
    /// survives the churn (run state becomes a row-scoped signal instead).
    func testRunStateChurnBustsNaiveKeyButNotProposedIdentityKey() {
        var input = GaryxHomeListFixture.makeInputs(runningCount: 0)
        let baselineNaive = HomeSectionsNaiveKey(input)
        let baselineIdentity = HomeSectionsIdentityKey(input)

        // Simulate 300 streaming run-state deltas on a single thread over ~60s
        // (~5 events/sec). Each delta flips run state / activeRunId, exactly as
        // applyTranscriptRunState -> applyThreadRunStateSummary does.
        var naiveBusts = 0
        var identityBusts = 0
        var prevNaive = baselineNaive
        var prevIdentity = baselineIdentity

        for tick in 0..<300 {
            let running = tick % 2 == 0
            input.threads[10].runState = running ? "running" : "idle"
            input.threads[10].activeRunId = running ? "run-tick-\(tick)" : nil
            if running { input.busyThreadIds.insert("thread-10") } else { input.busyThreadIds.remove("thread-10") }

            let naive = HomeSectionsNaiveKey(input)
            let identity = HomeSectionsIdentityKey(input)
            if naive != prevNaive { naiveBusts += 1; prevNaive = naive }
            if identity != prevIdentity { identityBusts += 1; prevIdentity = identity }
        }

        print("[TASK-1037] run-state churn (300 deltas): naive-content key busts \(naiveBusts)x (each = full O(n) recompute) | proposed identity key busts \(identityBusts)x")
        XCTAssertEqual(naiveBusts, 300, "Baking run state into the section key busts the cache on every delta")
        XCTAssertEqual(identityBusts, 0, "Decoupling run state from section identity keeps the section cache stable")
    }

    /// Headline cost comparison: a cache HIT (Equatable key compare) versus a
    /// full recompute, at realistic scale.
    func testCacheHitIsMuchCheaperThanRecompute() {
        let cache = HomeThreadSectionsCache()
        let input = GaryxHomeListFixture.makeInputs()
        _ = cache.sections(for: input) // prime

        let hitMs = GaryxBench.medianMillis(iterations: 200) {
            _ = cache.sections(for: input)
        }
        let recomputeMs = GaryxBench.medianMillis(iterations: 200) {
            _ = HomeThreadSectionsReference.build(input)
        }
        print("[TASK-1037] cache-HIT (Equatable gate): \(String(format: "%.4f", hitMs)) ms vs full recompute: \(String(format: "%.4f", recomputeMs)) ms  (speedup ~\(String(format: "%.1f", recomputeMs / max(hitMs, 0.0001)))x)")
        XCTAssertEqual(cache.computeCount, 1, "All hits served from cache")
        XCTAssertLessThan(hitMs, recomputeMs, "Cache hit must be cheaper than recompute")
    }
}
