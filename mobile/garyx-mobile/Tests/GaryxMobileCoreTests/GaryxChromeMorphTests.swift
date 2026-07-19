import XCTest
@testable import GaryxMobileCore

final class GaryxChromeMorphTests: XCTestCase {
    private let metrics = GaryxChromeMorphSurfaceMetrics(
        horizontalMargin: 12,
        maximumExpandedWidth: 560,
        collapsedCornerRadius: 22,
        expandedCornerRadius: 28
    )
    private let anchor = CGRect(x: 64, y: 18, width: 180, height: 44)

    func testGeometryExpandsWithinMarginsAndCenters() {
        let compact = GaryxChromeMorphSurfaceGeometry.layout(
            isExpanded: false,
            anchorRect: anchor,
            containerSize: CGSize(width: 390, height: 844),
            metrics: metrics
        )
        XCTAssertEqual(compact.expandedWidth, 366)
        XCTAssertEqual(compact.expandedX, 12)
        XCTAssertEqual(compact.outerWidth, 180)
        XCTAssertEqual(compact.outerHeight, 44)
        XCTAssertEqual(compact.outerX, 64)
        XCTAssertEqual(compact.outerY, 18)
        XCTAssertEqual(compact.cornerRadius, 22)

        let expanded = GaryxChromeMorphSurfaceGeometry.layout(
            isExpanded: true,
            anchorRect: anchor,
            containerSize: CGSize(width: 390, height: 844),
            metrics: metrics
        )
        XCTAssertEqual(expanded.outerWidth, 366)
        XCTAssertNil(expanded.outerHeight)
        XCTAssertEqual(expanded.outerX, 12)
        XCTAssertEqual(expanded.cornerRadius, 28)
    }

    func testGeometryClampsWideContainerToMaximum() {
        let layout = GaryxChromeMorphSurfaceGeometry.layout(
            isExpanded: true,
            anchorRect: anchor,
            containerSize: CGSize(width: 1_024, height: 768),
            metrics: metrics
        )
        XCTAssertEqual(layout.expandedWidth, 560)
        XCTAssertEqual(layout.expandedX, 232)
    }

    func testPresentationNormalMotionFullTransitionTable() {
        var state = GaryxChromeMorphPresentationState.hidden

        var transition = reduce(state, .requestPresent)
        XCTAssertEqual(
            transition,
            .init(state: .presentedCollapsed, animation: .none, schedule: .expandOnNextTick)
        )
        state = transition.state

        transition = reduce(state, .expandTick)
        XCTAssertEqual(transition, .init(state: .expanded, animation: .open, schedule: .none))
        state = transition.state

        transition = reduce(state, .requestDismiss)
        XCTAssertEqual(
            transition,
            .init(state: .collapsing, animation: .close, schedule: .completeDismissAfterAnimation)
        )
        state = transition.state

        transition = reduce(state, .dismissAnimationCompleted)
        XCTAssertEqual(transition, .init(state: .hidden, animation: .none, schedule: .none))
    }

    func testPresentationImmediatePolicyJumpsDirectly() {
        XCTAssertEqual(
            GaryxChromeMorphPresentationReducer.reduce(
                state: .hidden,
                event: .requestPresent,
                transitionMode: .immediate
            ),
            .init(state: .expanded, animation: .none, schedule: .none)
        )
        XCTAssertEqual(
            GaryxChromeMorphPresentationReducer.reduce(
                state: .expanded,
                event: .requestDismiss,
                transitionMode: .immediate
            ),
            .init(state: .hidden, animation: .none, schedule: .none)
        )
    }

    func testPresentationCrossFadeRetainsStagedLifecycle() {
        XCTAssertEqual(
            GaryxChromeMorphPresentationReducer.reduce(
                state: .hidden,
                event: .requestPresent,
                transitionMode: .crossFade
            ),
            .init(
                state: .presentedCollapsed,
                animation: .none,
                schedule: .expandOnNextTick
            )
        )
    }

    func testPresentationCollapsedDismissAndInvalidEventsAreStable() {
        XCTAssertEqual(
            reduce(.presentedCollapsed, .requestDismiss),
            .init(state: .hidden, animation: .none, schedule: .none)
        )
        for state in [
            GaryxChromeMorphPresentationState.hidden,
            .presentedCollapsed,
            .expanded,
            .collapsing,
        ] {
            let transition = reduce(state, .dismissAnimationCompleted)
            if state == .collapsing {
                XCTAssertEqual(transition.state, .hidden)
            } else {
                XCTAssertEqual(transition.state, state)
            }
        }
    }

    private func reduce(
        _ state: GaryxChromeMorphPresentationState,
        _ event: GaryxChromeMorphPresentationEvent
    ) -> GaryxChromeMorphPresentationTransition {
        GaryxChromeMorphPresentationReducer.reduce(
            state: state,
            event: event,
            transitionMode: .spatial
        )
    }
}
