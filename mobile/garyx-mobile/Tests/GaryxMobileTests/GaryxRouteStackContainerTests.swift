import SwiftUI
import UIKit
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxRouteStackContainerTests: XCTestCase {
    func testFakeRouteHostRequiresExplicitDebugEnvironmentOptIn() throws {
        XCTAssertNil(GaryxFluidFakeRouteDebugFixture.Configuration.load(environment: [:]))
        let configuration = try XCTUnwrap(
            GaryxFluidFakeRouteDebugFixture.Configuration.load(environment: [
                "GARYX_MOBILE_FLUID_FAKE_ROUTES": "1",
                "GARYX_MOBILE_FLUID_FAKE_DEPTH": "20",
                "GARYX_MOBILE_FLUID_FAKE_RTL": "1",
                "GARYX_MOBILE_FLUID_FAKE_VISUAL_POLICY": "crossFade",
            ])
        )
        XCTAssertEqual(configuration.initialDepth, 20)
        XCTAssertEqual(configuration.layoutDirection, .rightToLeft)
        XCTAssertEqual(configuration.preferences.resolvedPolicy, .crossFade)
    }

    func testCommitWritesCanonicalAtReleaseAndScreenChangedOnlyAtVisibleTerminal() {
        let harness = Harness(path: [entry(1), entry(2)])
        let bodyCountBeforeDrag = harness.bodyCounter.count

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.20)
        XCTAssertEqual(harness.container.path.count, 2)
        XCTAssertEqual(harness.probe.screenChangedCount, 0)
        XCTAssertEqual(harness.bodyCounter.count, bodyCountBeforeDrag)

        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 300), .committed)
        XCTAssertEqual(harness.container.path.count, 1, "canonical path changes at commit release")
        XCTAssertEqual(harness.probe.screenChangedCount, 0, "settle is not page terminal")
        XCTAssertEqual(harness.container.metrics.transitionPhase, .commitSettle)

        harness.completeDisplayLinkedSettle()

        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertEqual(
            harness.probe.terminals,
            [.init(outcome: .committed, visibility: .visible)]
        )
        XCTAssertFalse(harness.container.hasTerminalResidue)
        XCTAssertLessThanOrEqual(
            harness.container.metrics.mountedHostCount,
            GaryxRouteStackContainer.maximumMountedHostCount
        )
        XCTAssertTrue(harness.container.children.allSatisfy { $0.view.transform == .identity })
    }

    func testCancelSettleCanRegrabAndCarryPhysicalProgressIntoCommit() throws {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.3947)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .cancelled)
        XCTAssertEqual(harness.container.path.count, 1)
        XCTAssertEqual(harness.container.metrics.transitionPhase, .cancelSettle)

        harness.advance(by: 0.08)
        let regrab = try XCTUnwrap(harness.container.regrabCancelSettle())
        XCTAssertGreaterThan(regrab.value, 0)
        XCTAssertLessThan(regrab.value, 0.3947)
        XCTAssertEqual(harness.container.metrics.transitionPhase, .preCommit)
        XCTAssertFalse(harness.frames.isRunning)

        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.70)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        XCTAssertTrue(harness.container.path.isEmpty)
        harness.completeDisplayLinkedSettle()
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
        XCTAssertTrue(
            harness.probe.phases.containsSubsequence([
                .preCommit,
                .cancelSettle,
                .preCommit,
                .commitSettle,
                .terminal,
            ])
        )
    }

    func testPromotionDuringInteractivePopIsAppliedAfterCancellationWithoutInvalidatingGesture() {
        var draft = entry(2)
        draft.replacePayload(with: .conversationDraft(draftID: "draft-in-flight"))
        let harness = Harness(path: [entry(1), draft])

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.20)
        XCTAssertTrue(
            harness.container.promoteVisibleDraft(
                instanceID: draft.id,
                draftID: "draft-in-flight",
                threadID: "thread-after-cancel"
            )
        )
        XCTAssertEqual(harness.container.path.last?.destination, draft.destination)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .cancelled)

        harness.completeDisplayLinkedSettle()

        XCTAssertEqual(
            harness.container.path.last?.destination,
            .conversation(threadID: "thread-after-cancel")
        )
        XCTAssertEqual(harness.probe.terminals.last, .init(outcome: .cancelled, visibility: .visible))
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testBatchPushCommitsIntermediatePredecessorInOneTransaction() {
        let first = entry(1)
        let harness = Harness(path: [first])
        let overview = entry(2, destination: .settingsDetail("manage"))
        let detail = entry(3, destination: .settingsDetail("gateway"))

        XCTAssertTrue(harness.container.push([overview, detail], animated: false))

        XCTAssertEqual(harness.container.path, [first, overview, detail])
        XCTAssertEqual(harness.probe.terminals, [
            .init(outcome: .committed, visibility: .visible),
        ])
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testPresentedContentTouchesDoNotReachUnderlyingRouteGestures() {
        let harness = Harness(path: [entry(1)])
        let modal = UIViewController()

        XCTAssertTrue(harness.container.routeOwnsGestureTouch(in: harness.container.view))
        harness.container.present(modal, animated: false)
        pumpMainRunLoop(duration: 0.05)

        XCTAssertFalse(harness.container.routeOwnsGestureTouch(in: modal.view))

        harness.container.dismiss(animated: false)
    }

    func testHighVelocityRegrabSettleRemainsEndpointBoundedAndMonotonic() throws {
        let harness = Harness(path: [entry(1)])
        var commitProgress: [CGFloat] = []
        harness.container.transitionFrameObserver = { phase, progress, _ in
            if phase == .commitSettle {
                commitProgress.append(progress)
            }
        }
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.30)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .cancelled)
        harness.advance(by: 0.08)
        _ = try XCTUnwrap(harness.container.regrabCancelSettle())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.70)
        XCTAssertEqual(
            harness.container.endInteractivePop(logicalVelocity: harness.width * 5),
            .committed
        )

        let frameInterval = 1.0 / 120.0
        for _ in 0...Int(GaryxRouteTransitionCalibration.settleCurve.settlingDuration / frameInterval) {
            harness.advance(by: frameInterval)
        }
        harness.completeDisplayLinkedSettle()

        XCTAssertGreaterThan(commitProgress.count, 2)
        XCTAssertTrue(commitProgress.allSatisfy { (0...1).contains($0) })
        XCTAssertTrue(zip(commitProgress, commitProgress.dropFirst()).allSatisfy { pair in
            pair.1 + 0.000_1 >= pair.0
        })
        XCTAssertEqual(commitProgress.last, 1)
    }

    func testCancelledInactiveRestoresWithoutAnnouncementAndSupersededHasNoEffects() {
        let inactive = Harness(path: [entry(1)])
        XCTAssertTrue(inactive.container.beginInteractivePop())
        inactive.container.updateInteractivePop(logicalTranslation: 80)
        inactive.container.sceneDidBecomeInactive()

        XCTAssertEqual(inactive.container.path.count, 1)
        XCTAssertEqual(
            inactive.probe.terminals,
            [.init(outcome: .cancelled, visibility: .inactive)]
        )
        XCTAssertEqual(inactive.probe.screenChangedCount, 0)
        XCTAssertFalse(try! XCTUnwrap(inactive.visibleWrapper()).isUserInteractionEnabled)

        inactive.container.sceneDidBecomeActive()
        XCTAssertEqual(inactive.probe.screenChangedCount, 0)
        XCTAssertTrue(try! XCTUnwrap(inactive.visibleWrapper()).isUserInteractionEnabled)

        let superseded = Harness(path: [entry(1)])
        XCTAssertTrue(superseded.container.beginInteractivePop())
        superseded.container.updateInteractivePop(logicalTranslation: 90)
        superseded.container.supersedeActiveTransition()

        XCTAssertEqual(superseded.container.path.count, 1)
        XCTAssertEqual(
            superseded.probe.terminals,
            [.init(outcome: .cancelled, visibility: .superseded)]
        )
        XCTAssertEqual(superseded.probe.screenChangedCount, 0)
        XCTAssertFalse(try! XCTUnwrap(superseded.visibleWrapper()).isUserInteractionEnabled)
    }

    func testCommittedInactiveDefersOneAnnouncementAndCommittedSupersededEmitsNone() {
        let inactive = Harness(path: [entry(1)])
        XCTAssertTrue(inactive.container.beginInteractivePop())
        inactive.container.updateInteractivePop(logicalTranslation: inactive.width * 0.7)
        XCTAssertEqual(inactive.container.endInteractivePop(logicalVelocity: 0), .committed)
        XCTAssertTrue(inactive.container.path.isEmpty)
        inactive.container.sceneDidBecomeInactive()

        XCTAssertEqual(
            inactive.probe.terminals,
            [.init(outcome: .committed, visibility: .inactive)]
        )
        XCTAssertEqual(inactive.probe.screenChangedCount, 0)
        inactive.container.sceneDidBecomeActive()
        inactive.container.sceneDidBecomeActive()
        XCTAssertEqual(inactive.probe.screenChangedCount, 1)

        let superseded = Harness(path: [entry(1)])
        XCTAssertTrue(superseded.container.beginInteractivePop())
        superseded.container.updateInteractivePop(logicalTranslation: superseded.width * 0.7)
        XCTAssertEqual(superseded.container.endInteractivePop(logicalVelocity: 0), .committed)
        superseded.container.supersedeActiveTransition()

        XCTAssertTrue(superseded.container.path.isEmpty)
        XCTAssertEqual(
            superseded.probe.terminals,
            [.init(outcome: .committed, visibility: .superseded)]
        )
        XCTAssertEqual(superseded.probe.screenChangedCount, 0)
        XCTAssertFalse(try! XCTUnwrap(superseded.visibleWrapper()).isUserInteractionEnabled)
    }

    func testProgrammaticImmediateSettleWhileInactiveDefersVisibleEffects() {
        let harness = Harness(path: [entry(1)])
        harness.container.sceneDidBecomeInactive()

        XCTAssertTrue(harness.container.push(entry(2), animated: false))
        XCTAssertEqual(
            harness.probe.terminals,
            [.init(outcome: .committed, visibility: .inactive)]
        )
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        harness.container.sceneDidBecomeInactive()
        XCTAssertEqual(harness.probe.terminals.count, 1)
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        harness.container.sceneDidBecomeActive()
        harness.container.sceneDidBecomeActive()
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testInteractiveImmediatePolicyWhileInactiveDefersVisibleEffects() {
        let harness = Harness(
            path: [entry(1)],
            preferences: .init(reduceMotion: true, prefersCrossFadeTransitions: false)
        )
        harness.container.sceneDidBecomeInactive()

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.7)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        XCTAssertEqual(
            harness.probe.terminals,
            [.init(outcome: .committed, visibility: .inactive)]
        )
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        harness.container.sceneDidBecomeActive()
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testDeferredCommittedDestinationCanStartNextCommitWithoutLifecycleViolation() {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.7)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        harness.container.sceneDidBecomeInactive()

        XCTAssertEqual(
            harness.probe.terminals,
            [.init(outcome: .committed, visibility: .inactive)]
        )
        XCTAssertTrue(harness.container.push(entry(2), animated: false))
        XCTAssertEqual(harness.container.path, [entry(2)])
        XCTAssertEqual(
            harness.probe.terminals,
            [
                .init(outcome: .committed, visibility: .inactive),
                .init(outcome: .committed, visibility: .inactive),
            ]
        )
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        harness.container.sceneDidBecomeActive()
        harness.container.sceneDidBecomeActive()
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testNewTransitionPermanentlyCancelsDeferredVisibleEffects() {
        let harness = Harness(path: [entry(1), entry(2)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.7)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        harness.container.sceneDidBecomeInactive()
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.sceneDidBecomeActive()
        XCTAssertEqual(
            harness.probe.screenChangedCount,
            0,
            "a superseded inactive terminal must never replay during the next transaction"
        )

        harness.container.cancelInteractivePop()
        harness.container.completeSettleImmediately()
        XCTAssertEqual(
            harness.probe.terminals,
            [
                .init(outcome: .committed, visibility: .inactive),
                .init(outcome: .cancelled, visibility: .visible),
            ]
        )
        XCTAssertEqual(harness.probe.screenChangedCount, 0)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testStagedDestinationPerformsNoLifecycleWritesUntilCommittedVisible() {
        let harness = Harness(path: [entry(1)])
        let home = GaryxRoutePresentationIdentity.home
        let route = GaryxRoutePresentationIdentity.entry(entry(1).id)
        XCTAssertEqual(harness.probe.lifecycle[home, default: []], [])
        XCTAssertEqual(harness.probe.lifecycle[route, default: []], [.appeared, .active])

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.3)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .cancelled)
        harness.container.completeSettleImmediately()
        XCTAssertEqual(harness.probe.lifecycle[home, default: []], [])

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.7)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        XCTAssertEqual(harness.probe.lifecycle[home, default: []], [])
        harness.container.completeSettleImmediately()

        XCTAssertEqual(harness.probe.lifecycle[home, default: []], [.appeared, .active])
        XCTAssertEqual(
            harness.probe.lifecycle[route, default: []],
            [.appeared, .active, .inactive, .disappeared]
        )
    }

    func testAllVisualPoliciesWriteOnlyWrappers() throws {
        let policies: [(GaryxRouteVisualPreferences, GaryxRouteVisualPolicy)] = [
            (.init(reduceMotion: false, prefersCrossFadeTransitions: false), .spatial),
            (.init(reduceMotion: false, prefersCrossFadeTransitions: true), .crossFade),
            (.init(reduceMotion: true, prefersCrossFadeTransitions: false), .immediate),
        ]

        for (preferences, expectedPolicy) in policies {
            let harness = Harness(path: [entry(1)], preferences: preferences)
            XCTAssertTrue(harness.container.beginInteractivePop())
            harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.4)
            XCTAssertEqual(harness.container.visualPolicyForActiveTransaction, expectedPolicy)
            let wrappers = harness.wrappers()
            XCTAssertEqual(wrappers.count, 2)
            XCTAssertTrue(harness.container.children.allSatisfy { $0.view.transform == .identity })

            if expectedPolicy == .spatial {
                XCTAssertTrue(wrappers.contains { $0.transform.tx != 0 })
                XCTAssertTrue(wrappers.contains { $0.layer.shadowOpacity > 0 })
            } else {
                XCTAssertTrue(wrappers.allSatisfy { $0.transform == .identity })
                XCTAssertTrue(wrappers.allSatisfy { $0.layer.shadowOpacity == 0 })
                XCTAssertTrue(wrappers.allSatisfy { $0.scrimView.alpha == 0 })
            }
        }
    }

    func testPopZOrderAlwaysPlacesOutgoingWrapperAboveIncoming() throws {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        let source = try XCTUnwrap(
            harness.wrapper(identity: .entry(entry(1).id))
        )
        let destination = try XCTUnwrap(harness.wrapper(identity: .home))
        let sourceIndex = try XCTUnwrap(harness.container.view.subviews.firstIndex(of: source))
        let destinationIndex = try XCTUnwrap(
            harness.container.view.subviews.firstIndex(of: destination)
        )
        XCTAssertGreaterThan(sourceIndex, destinationIndex)
    }

    func testRotationRederivesWrapperGeometryAtCurrentProgress() throws {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.4)
        let sourceIdentity = GaryxRoutePresentationIdentity.entry(entry(1).id)
        let sourceBefore = try XCTUnwrap(harness.wrapper(identity: sourceIdentity))
        XCTAssertEqual(sourceBefore.transform.tx, harness.width * 0.4, accuracy: 0.01)

        harness.container.view.frame = CGRect(x: 0, y: 0, width: 844, height: 393)
        harness.container.view.setNeedsLayout()
        harness.container.view.layoutIfNeeded()

        let sourceAfter = try XCTUnwrap(harness.wrapper(identity: sourceIdentity))
        XCTAssertEqual(sourceAfter.bounds.width, 844, accuracy: 0.01)
        XCTAssertEqual(sourceAfter.transform.tx, 844 * 0.4, accuracy: 0.01)
        XCTAssertEqual(sourceAfter.center.x, 422, accuracy: 0.01)
    }

    func testOneHundredTwentyHertzDragFramesCauseZeroSwiftUIBodyRecomputations() {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.pumpUI()
        let baseline = harness.bodyCounter.count

        for frame in 1...120 {
            harness.container.updateInteractivePop(
                logicalTranslation: harness.width * CGFloat(frame) / 240
            )
            harness.pumpUI(duration: 0.0001)
        }

        XCTAssertEqual(
            harness.bodyCounter.count,
            baseline,
            "display progress must never publish SwiftUI state"
        )
    }

    func testTwentyLayerStackAndFiveHundredChurnNeverExceedHostBudget() {
        let deep = Harness(path: (1...20).map { entry($0) })
        XCTAssertLessThanOrEqual(deep.container.metrics.mountedHostCount, 4)
        XCTAssertLessThanOrEqual(deep.container.metrics.peakMountedHostCount, 4)

        let churn = Harness(path: [])
        for index in 0..<500 {
            XCTAssertTrue(churn.container.push(entry(index + 1), animated: false))
            XCTAssertTrue(churn.container.pop(animated: false))
            XCTAssertFalse(churn.container.hasTerminalResidue)
        }
        XCTAssertEqual(churn.container.path, [])
        XCTAssertLessThanOrEqual(churn.container.metrics.mountedHostCount, 4)
        XCTAssertLessThanOrEqual(churn.container.metrics.peakMountedHostCount, 4)
        XCTAssertLessThanOrEqual(churn.container.metrics.stateStore.evictableEntryCount, 32)
        XCTAssertLessThanOrEqual(
            churn.container.metrics.stateStore.evictableCostBytes,
            2 * 1_024 * 1_024
        )
    }

    func testPopMultipleUnmountsEveryPermanentlyRemovedHostAtTerminal() {
        let harness = Harness(path: [entry(1), entry(2), entry(3)])
        let middle = GaryxRoutePresentationIdentity.entry(entry(2).id)
        let source = GaryxRoutePresentationIdentity.entry(entry(3).id)
        XCTAssertTrue(harness.container.mountedHostIdentities.contains(middle))
        XCTAssertTrue(harness.container.mountedHostIdentities.contains(source))

        XCTAssertTrue(harness.container.pop(count: 2, animated: false))

        XCTAssertEqual(harness.container.path, [entry(1)])
        XCTAssertFalse(harness.container.mountedHostIdentities.contains(middle))
        XCTAssertFalse(harness.container.mountedHostIdentities.contains(source))
        XCTAssertTrue(harness.probe.unmounted.contains(middle))
        XCTAssertTrue(harness.probe.unmounted.contains(source))
        XCTAssertEqual(harness.probe.lifecycle[middle, default: []], [])
        XCTAssertEqual(
            harness.probe.lifecycle[source, default: []],
            [.appeared, .active, .inactive, .disappeared]
        )
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testPresentationLeaseJoinSameFrameRaceAndHardSnapBarrier() throws {
        let harness = Harness(path: [entry(1)])
        let parent = GaryxPresentationLeaseToken(rawValue: "synthetic-parent")
        let picker = GaryxPresentationLeaseToken(rawValue: "synthetic-picker")
        XCTAssertTrue(harness.container.acquirePresentationLease(parent))
        XCTAssertTrue(
            harness.container.acquirePresentationLease(
                picker,
                parent: parent,
                resultBearing: true
            )
        )
        XCTAssertFalse(harness.container.leadingEdgePanGestureRecognizer.isEnabled)

        let replacement = [entry(99)]
        XCTAssertFalse(harness.container.requestHardSnap(to: replacement))
        XCTAssertEqual(harness.container.path, [entry(1)])

        harness.container.presentationDismissalCompleted(picker)
        XCTAssertEqual(
            harness.container.presentationLeaseRecord(picker)?.joinState,
            .dismissedAwaitingResult
        )
        harness.container.recordPresentationResult(picker)
        XCTAssertEqual(harness.container.presentationLeaseRecord(picker)?.releaseCount, 1)
        XCTAssertTrue(harness.container.hasPresentationBarrier, "parent still blocks hard snap")

        harness.container.presentationDismissalCompleted(parent)
        harness.container.presentationDismissalCompleted(parent)
        XCTAssertEqual(harness.container.presentationLeaseRecord(parent)?.releaseCount, 1)
        XCTAssertFalse(harness.container.hasPresentationBarrier)
        XCTAssertEqual(harness.container.path, replacement)
        XCTAssertFalse(harness.container.hasTerminalResidue)

        let failed = GaryxPresentationLeaseToken(rawValue: "synthetic-failure")
        XCTAssertTrue(harness.container.acquirePresentationLease(failed, resultBearing: true))
        harness.container.presentationFailed(failed)
        XCTAssertEqual(harness.container.presentationLeaseRecord(failed)?.releaseCount, 1)
        XCTAssertFalse(harness.container.hasPresentationBarrier)
    }

    func testContainerDeinitReleasesAllHostingControllersAndRootViews() {
        let factory = LifetimeFactory()
        weak var weakContainer: GaryxRouteStackContainer?
        var window: UIWindow?
        autoreleasepool {
            var container: GaryxRouteStackContainer? = GaryxRouteStackContainer(
                initialPath: (1...20).map { entry($0) },
                preferencesProvider: {
                    .init(reduceMotion: false, prefersCrossFadeTransitions: false)
                },
                hostBuilder: { node in
                    AnyView(LifetimeRouteView(node: node, token: factory.make()))
                }
            )
            weakContainer = container
            window = makeTestWindow(frame: CGRect(x: 0, y: 0, width: 393, height: 852))
            window?.rootViewController = container
            window?.isHidden = false
            container?.loadViewIfNeeded()
            container?.view.layoutIfNeeded()
            XCTAssertLessThanOrEqual(container?.children.count ?? .max, 4)
            window?.rootViewController = nil
            container = nil
        }
        window = nil
        pumpMainRunLoop(duration: 0.05)

        XCTAssertNil(weakContainer)
        XCTAssertGreaterThan(factory.weakTokens.count, 0)
        XCTAssertTrue(factory.weakTokens.allSatisfy { $0.value == nil })
    }

    // MARK: Fixtures

    private func entry(
        _ index: Int,
        destination: GaryxRouteDestination? = nil
    ) -> GaryxRouteEntry {
        GaryxRouteEntry(
            id: .init(rawValue: "synthetic-route-\(index)"),
            destination: destination ?? .panel("synthetic-panel-\(index)")
        )
    }

    private final class Probe {
        var mounted: [GaryxRoutePresentationIdentity] = []
        var unmounted: [GaryxRoutePresentationIdentity] = []
        var lifecycle: [GaryxRoutePresentationIdentity: [GaryxRouteHostLifecyclePhase]] = [:]
        var phases: [GaryxPresentationTransactionPhase] = []
        var paths: [[GaryxRouteEntry]] = []
        var terminals: [GaryxPresentationTerminalState] = []
        var screenChangedCount = 0
    }

    private final class BodyCounter {
        private(set) var count = 0
        func record() { count += 1 }
    }

    private struct CountingRouteView: View {
        let node: GaryxRoutePresentationNode
        let counter: BodyCounter

        var body: some View {
            counter.record()
            return VStack {
                Text(label)
                    .accessibilityIdentifier("synthetic route label")
                ScrollView(.horizontal) {
                    HStack {
                        ForEach(0..<8, id: \.self) { index in
                            Text("Item \(index)")
                        }
                    }
                }
            }
        }

        private var label: String {
            switch node {
            case .home:
                "Synthetic home"
            case .entry(let entry):
                "Synthetic \(entry.id.rawValue)"
            }
        }
    }

    private final class ManualTimeSource: GaryxGestureSettleTimeSource {
        var now: TimeInterval = 10
    }

    private final class ManualFrameSource: GaryxGestureSettleFrameSource {
        var onFrame: (() -> Void)?
        private(set) var isRunning = false

        func start() { isRunning = true }
        func invalidate() { isRunning = false }
        func fire() {
            guard isRunning else { return }
            onFrame?()
        }
    }

    @MainActor
    private final class Harness {
        let width: CGFloat = 393
        let probe = Probe()
        let bodyCounter = BodyCounter()
        let clock = ManualTimeSource()
        let frames = ManualFrameSource()
        let container: GaryxRouteStackContainer
        let window: UIWindow

        init(
            path: [GaryxRouteEntry],
            preferences: GaryxRouteVisualPreferences = .init(
                reduceMotion: false,
                prefersCrossFadeTransitions: false
            )
        ) {
            var callbacks = GaryxRouteStackContainerCallbacks()
            callbacks.hostMounted = { [probe] in probe.mounted.append($0) }
            callbacks.hostUnmounted = { [probe] in probe.unmounted.append($0) }
            callbacks.hostLifecycleChanged = { [probe] identity, phase in
                probe.lifecycle[identity, default: []].append(phase)
            }
            callbacks.phaseChanged = { [probe] in probe.phases.append($0) }
            callbacks.canonicalPathChanged = { [probe] in probe.paths.append($0) }
            callbacks.terminalReached = { [probe] in probe.terminals.append($0) }
            callbacks.screenChanged = { [probe] _ in probe.screenChangedCount += 1 }

            container = GaryxRouteStackContainer(
                initialPath: path,
                settleDriver: GaryxGestureSettleDriver(
                    timeSource: clock,
                    frameSource: frames
                ),
                callbacks: callbacks,
                preferencesProvider: { preferences },
                hostBuilder: { [bodyCounter] node in
                    AnyView(CountingRouteView(node: node, counter: bodyCounter))
                }
            )
            window = makeTestWindow(frame: CGRect(x: 0, y: 0, width: width, height: 852))
            window.rootViewController = container
            window.isHidden = false
            container.loadViewIfNeeded()
            container.view.frame = window.bounds
            container.view.setNeedsLayout()
            container.view.layoutIfNeeded()
            pumpUI()
        }

        func advance(by delta: TimeInterval) {
            clock.now += delta
            frames.fire()
            pumpUI(duration: 0.001)
        }

        func completeDisplayLinkedSettle() {
            advance(by: GaryxRouteTransitionCalibration.settleCurve.settlingDuration + 0.001)
        }

        func wrappers() -> [GaryxRouteTransitionWrapperView] {
            container.view.subviews.compactMap { $0 as? GaryxRouteTransitionWrapperView }
        }

        func wrapper(
            identity: GaryxRoutePresentationIdentity
        ) -> GaryxRouteTransitionWrapperView? {
            wrappers().first { $0.representedIdentity == identity }
        }

        func visibleWrapper() -> GaryxRouteTransitionWrapperView? {
            wrappers().first { !$0.isHidden }
        }

        func pumpUI(duration: TimeInterval = 0.01) {
            pumpMainRunLoop(duration: duration)
        }
    }

    private final class LifetimeToken {}

    private final class WeakLifetimeToken {
        weak var value: LifetimeToken?
        init(_ value: LifetimeToken) { self.value = value }
    }

    private final class LifetimeFactory {
        private(set) var weakTokens: [WeakLifetimeToken] = []

        func make() -> LifetimeToken {
            let token = LifetimeToken()
            weakTokens.append(WeakLifetimeToken(token))
            return token
        }
    }

    private struct LifetimeRouteView: View {
        let node: GaryxRoutePresentationNode
        let token: LifetimeToken

        var body: some View {
            Text(String(describing: node))
        }
    }
}

@MainActor
private func pumpMainRunLoop(duration: TimeInterval) {
    RunLoop.main.run(until: Date().addingTimeInterval(duration))
}

@MainActor
private func makeTestWindow(frame: CGRect) -> UIWindow {
    guard let scene = UIApplication.shared.connectedScenes
        .compactMap({ $0 as? UIWindowScene })
        .first
    else { preconditionFailure("hosted iOS tests require an active UIWindowScene") }
    let window = UIWindow(windowScene: scene)
    window.frame = frame
    return window
}

private extension Array where Element: Equatable {
    func containsSubsequence(_ subsequence: [Element]) -> Bool {
        guard !subsequence.isEmpty else { return true }
        var index = subsequence.startIndex
        for element in self where element == subsequence[index] {
            index = subsequence.index(after: index)
            if index == subsequence.endIndex { return true }
        }
        return false
    }
}
