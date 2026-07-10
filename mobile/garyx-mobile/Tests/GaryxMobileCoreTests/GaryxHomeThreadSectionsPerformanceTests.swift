import XCTest
@testable import GaryxMobileCore

/// TASK-1037 diagnosis — cost of the home-list section derivation.
///
/// `homeThreadSections` is recomputed on EVERY home `body` invalidation (it is a
/// computed property read inside the view body). Any of the model's ~80
/// `@Published` fields firing `objectWillChange` re-runs it synchronously on the
/// main thread. These tests quantify the per-recompute cost at realistic scale.
final class GaryxHomeThreadSectionsPerformanceTests: XCTestCase {

    func testDerivationCostAtRealisticScale() {
        // ~30-50 threads, ~20-100 agents, ~10-30 automations (task spec).
        let input = GaryxHomeListFixture.makeInputs(
            threadCount: 50, agentCount: 80, automationCount: 25,
            pinnedCount: 6, runningCount: 4
        )

        // Warm the timestamp parse cache (matches steady-state app behavior).
        _ = HomeThreadSectionsReference.build(input)

        let ms = GaryxBench.medianMillis(iterations: 200) {
            _ = HomeThreadSectionsReference.build(input)
        }
        print("[TASK-1037] homeThreadSections derivation @ 50 threads / 80 agents / 25 automations: \(String(format: "%.3f", ms)) ms / recompute")

        let sections = HomeThreadSectionsReference.build(input)
        XCTAssertEqual(sections.pinned.count, 6)
        XCTAssertEqual(sections.recent.count, 44) // 50 recent minus 6 pinned
        // Stable identities: every row id is unique (List/ForEach correctness).
        let ids = (sections.pinned + sections.recent).map(\.id)
        XCTAssertEqual(Set(ids).count, ids.count)
        // Sanity floor: a real recompute is not free.
        XCTAssertGreaterThan(ms, 0)
    }

    func testDerivationCostScalesLinearlyWithThreadCount() {
        func cost(_ threadCount: Int) -> Double {
            let input = GaryxHomeListFixture.makeInputs(
                threadCount: threadCount, agentCount: 80, automationCount: 25,
                pinnedCount: 6, runningCount: 4
            )
            _ = HomeThreadSectionsReference.build(input)
            return GaryxBench.medianMillis(iterations: 100) {
                _ = HomeThreadSectionsReference.build(input)
            }
        }

        let small = cost(25)
        let medium = cost(50)
        let large = cost(200)
        print("[TASK-1037] derivation scaling — 25 threads: \(String(format: "%.3f", small)) ms | 50: \(String(format: "%.3f", medium)) ms | 200: \(String(format: "%.3f", large)) ms")
        // O(n): the full list is rebuilt every recompute, including off-screen
        // rows. LazyVStack virtualizes the VIEWS, but the row DATA model is
        // built eagerly for all N here.
        XCTAssertGreaterThan(large, medium)
    }

    /// XCTest's own metric harness for the headline number (shows up as a
    /// baseline-comparable measurement in test output).
    func testDerivationMeasureMetric() {
        let input = GaryxHomeListFixture.makeInputs()
        _ = HomeThreadSectionsReference.build(input)
        measure {
            for _ in 0..<50 {
                _ = HomeThreadSectionsReference.build(input)
            }
        }
    }
}
