import XCTest
@testable import GaryxMobileCore

@MainActor
final class GaryxRootInteractionDeadlockReproTests: XCTestCase {
    func testSameGatewayReconnectTerminatesDrawerOwnerBeforeRemount() throws {
        let root = makeCheckingRoot()
        var drawer = HostedRevealHarness()

        root.applyConnection(
            isGatewayConfigured: true,
            connectionState: .ready(version: "before-reconnect"),
            willTransitionRootSurface: { drawer.apply($0) }
        )
        let firstRootOccurrence = try XCTUnwrap(root.navigationShellOccurrenceID)
        let firstHost = makeHost(root: firstRootOccurrence, label: "first-host")
        XCTAssertEqual(drawer.attach(firstHost), .attached)

        XCTAssertTrue(drawer.beginDrag(in: firstHost))
        drawer.updateDrag(logicalTranslation: 120, in: firstHost)
        XCTAssertEqual(drawer.state.phase, .dragging)
        XCTAssertFalse(drawer.state.phase.allowsSurfaceHitTesting)

        // This is the phase-1 sequence: same-gateway `connectAndRefresh`
        // publishes `.checking`, replacing the complete Shell occurrence. The
        // old branch is still visible while its ownership callback force-ends
        // the recognizer-owned drag.
        root.applyConnection(
            isGatewayConfigured: true,
            connectionState: .checking,
            willTransitionRootSurface: { transition in
                XCTAssertEqual(root.rootSurface, .navigationShell(firstRootOccurrence))
                drawer.apply(transition)
                assertIdleAndInteractive(drawer.state)
            }
        )
        XCTAssertEqual(root.rootSurface, .gatewaySetup)
        assertIdleAndInteractive(drawer.state)
        XCTAssertNil(drawer.ownership.rootSurfaceOccurrenceID)
        XCTAssertNil(drawer.ownership.activeHostOccurrenceID)

        root.applyConnection(
            isGatewayConfigured: true,
            connectionState: .ready(version: "after-reconnect"),
            willTransitionRootSurface: { drawer.apply($0) }
        )
        let secondRootOccurrence = try XCTUnwrap(root.navigationShellOccurrenceID)
        XCTAssertNotEqual(secondRootOccurrence, firstRootOccurrence)
        let secondHost = makeHost(root: secondRootOccurrence, label: "second-host")
        XCTAssertEqual(drawer.attach(secondHost), .attached)

        // A coalesced root update may attach the replacement before SwiftUI
        // dismantles the old container. Its late teardown and callbacks are
        // occurrence-gated and cannot revoke the current owner.
        XCTAssertFalse(drawer.detach(firstHost))
        XCTAssertEqual(drawer.ownership.activeHostOccurrenceID, secondHost)
        XCTAssertFalse(drawer.beginDrag(in: firstHost))
        assertIdleAndInteractive(drawer.state)

        // The replacement host immediately admits a fresh leading-edge drag;
        // the prior global input freeze did not survive the root transition.
        XCTAssertTrue(drawer.beginDrag(in: secondHost))
        XCTAssertEqual(drawer.state.phase, .dragging)
        XCTAssertTrue(drawer.detach(secondHost))
        assertIdleAndInteractive(drawer.state)
    }

    func testEveryRootExitTimingLeavesNoNonIdleRevealResidue() throws {
        let exits: [(configured: Bool, state: GaryxMobileConnectionState, label: String)] = [
            (true, .checking, "checking"),
            (true, .disconnected, "disconnected"),
            (true, .failed("offline"), "failed"),
            (false, .ready(version: "configuration-removed"), "configuration removed"),
        ]

        for phase in RevealPhaseFixture.allCases {
            for exit in exits {
                let root = makeCheckingRoot()
                var drawer = HostedRevealHarness()
                root.applyConnection(
                    isGatewayConfigured: true,
                    connectionState: .ready(version: "ready"),
                    willTransitionRootSurface: { drawer.apply($0) }
                )
                let rootOccurrence = try XCTUnwrap(root.navigationShellOccurrenceID)
                let host = makeHost(
                    root: rootOccurrence,
                    label: "\(phase.rawValue)-\(exit.label)"
                )
                XCTAssertEqual(drawer.attach(host), .attached)
                try drawer.enter(phase, in: host)

                root.applyConnection(
                    isGatewayConfigured: exit.configured,
                    connectionState: exit.state,
                    willTransitionRootSurface: { drawer.apply($0) }
                )

                XCTAssertEqual(root.rootSurface, .gatewaySetup, "\(phase) / \(exit.label)")
                assertIdleAndInteractive(drawer.state, "\(phase) / \(exit.label)")
                XCTAssertNil(
                    drawer.ownership.rootSurfaceOccurrenceID,
                    "\(phase) / \(exit.label)"
                )
                XCTAssertNil(
                    drawer.ownership.activeHostOccurrenceID,
                    "\(phase) / \(exit.label)"
                )
            }
        }
    }

    func testHostSupersedeAndDismantleTerminateOnlyTheirOwnOccurrence() throws {
        let root = makeCheckingRoot()
        var drawer = HostedRevealHarness()
        root.applyConnection(
            isGatewayConfigured: true,
            connectionState: .ready(version: "ready"),
            willTransitionRootSurface: { drawer.apply($0) }
        )
        let rootOccurrence = try XCTUnwrap(root.navigationShellOccurrenceID)
        let firstHost = makeHost(root: rootOccurrence, label: "first-host")
        let replacementHost = makeHost(root: rootOccurrence, label: "replacement-host")

        XCTAssertEqual(drawer.attach(firstHost), .attached)
        XCTAssertTrue(drawer.beginDrag(in: firstHost))
        drawer.updateDrag(logicalTranslation: 90, in: firstHost)

        XCTAssertEqual(drawer.attach(replacementHost), .superseded(firstHost))
        assertIdleAndInteractive(drawer.state)
        XCTAssertEqual(drawer.ownership.activeHostOccurrenceID, replacementHost)

        // A late dismantle from the superseded UIKit container cannot revoke
        // the replacement's ownership.
        XCTAssertFalse(drawer.detach(firstHost))
        XCTAssertFalse(drawer.beginDrag(in: firstHost))
        XCTAssertTrue(drawer.beginDrag(in: replacementHost))
        XCTAssertEqual(drawer.state.phase, .dragging)
        XCTAssertEqual(drawer.attach(replacementHost), .alreadyAttached)
        XCTAssertEqual(drawer.state.phase, .dragging)

        XCTAssertTrue(drawer.detach(replacementHost))
        assertIdleAndInteractive(drawer.state)
        XCTAssertNil(drawer.ownership.activeHostOccurrenceID)
    }

    private func makeCheckingRoot() -> GaryxHomeObservationStore {
        GaryxHomeObservationStore(
            isGatewayConfigured: true,
            connectionState: .checking
        )
    }

    private func makeHost(
        root: GaryxRootSurfaceOccurrenceID,
        label: String
    ) -> GaryxHorizontalRevealHostOccurrenceID {
        GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: root,
            rawValue: label
        )
    }

    private func assertIdleAndInteractive(
        _ state: GaryxHorizontalRevealState,
        _ context: String = "",
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(state.phase, .idle, context, file: file, line: line)
        XCTAssertEqual(state.settledPosition, .closed, context, file: file, line: line)
        XCTAssertEqual(state.reveal, 0, context, file: file, line: line)
        XCTAssertTrue(
            state.phase.allowsSurfaceHitTesting,
            context,
            file: file,
            line: line
        )
    }
}

private enum RevealPhaseFixture: String, CaseIterable {
    case idle
    case dragging
    case settling
}

private struct HostedRevealHarness {
    let extent: CGFloat = 330
    var state = GaryxHorizontalRevealState(position: .closed, extent: 330)
    var ownership = GaryxHorizontalRevealHostOwnership()

    mutating func apply(_ transition: GaryxRootSurfaceOccurrenceTransition) {
        if ownership.applyRootSurfaceTransition(transition) {
            _ = state.forceTerminal(
                .hostOccurrenceEnded,
                to: .closed,
                extent: extent
            )
        }
    }

    mutating func attach(
        _ occurrenceID: GaryxHorizontalRevealHostOccurrenceID
    ) -> GaryxHorizontalRevealHostAttachmentResult {
        let result = ownership.attachHost(occurrenceID)
        if case .superseded = result {
            _ = state.forceTerminal(
                .hostOccurrenceEnded,
                to: .closed,
                extent: extent
            )
        }
        return result
    }

    mutating func detach(_ occurrenceID: GaryxHorizontalRevealHostOccurrenceID) -> Bool {
        let detached = ownership.detachHost(occurrenceID)
        if detached {
            _ = state.forceTerminal(
                .hostOccurrenceEnded,
                to: .closed,
                extent: extent
            )
        }
        return detached
    }

    mutating func beginDrag(in occurrenceID: GaryxHorizontalRevealHostOccurrenceID) -> Bool {
        guard ownership.accepts(occurrenceID) else { return false }
        state.beginDrag(extent: extent)
        return true
    }

    mutating func updateDrag(
        logicalTranslation: CGFloat,
        in occurrenceID: GaryxHorizontalRevealHostOccurrenceID
    ) {
        guard ownership.accepts(occurrenceID) else { return }
        state.updateDrag(logicalTranslation: logicalTranslation, extent: extent)
    }

    mutating func enter(
        _ phase: RevealPhaseFixture,
        in occurrenceID: GaryxHorizontalRevealHostOccurrenceID
    ) throws {
        switch phase {
        case .idle:
            break
        case .dragging:
            guard beginDrag(in: occurrenceID) else {
                throw HarnessError.hostRejected
            }
            updateDrag(logicalTranslation: 120, in: occurrenceID)
        case .settling:
            guard ownership.accepts(occurrenceID) else {
                throw HarnessError.hostRejected
            }
            _ = try XCTUnwrap(state.beginProgrammaticSettle(
                to: .open,
                initialVelocity: 180,
                extent: extent
            ))
            state.updateSettle(sampledReveal: 150, extent: extent)
        }
    }

    private enum HarnessError: Error {
        case hostRejected
    }
}

private extension GaryxHomeObservationStore {
    var navigationShellOccurrenceID: GaryxRootSurfaceOccurrenceID? {
        guard case .navigationShell(let occurrenceID) = rootSurface else { return nil }
        return occurrenceID
    }
}
