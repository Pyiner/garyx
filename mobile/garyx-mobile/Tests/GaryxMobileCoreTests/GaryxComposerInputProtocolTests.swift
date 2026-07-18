import XCTest
@testable import GaryxMobileCore

final class GaryxComposerInputProtocolTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "gateway", epoch: 1)

    func testReservationByProducerProductReducerAllTwelveCells() {
        let expected: [GaryxInputReservationPhase: [GaryxInputProductTarget]] = [
            .none: [.currentGeneration, .currentGeneration, .terminalAudit],
            .sealed: [.provisionalNextGeneration, .provisionalNextGeneration, .terminalAudit],
            .committed: [.committedNextGeneration, .committedNextGeneration, .terminalAudit],
            .revoked: [.revokedMergeGeneration, .revokedMergeGeneration, .terminalAudit],
        ]
        var visited = 0
        for reservation in GaryxInputReservationPhase.allCases {
            for (index, producer) in GaryxProducerFinalizationPhase.allCases.enumerated() {
                XCTAssertEqual(
                    GaryxComposerInputProductReducer.target(
                        reservation: reservation,
                        producer: producer
                    ),
                    expected[reservation]?[index],
                    "reservation=\(reservation), producer=\(producer)"
                )
                visited += 1
            }
        }
        XCTAssertEqual(visited, 12)
    }

    func testSixTupleRejectsSessionMismatchDuplicateOldFutureAndReservationMismatch() {
        var state = makeState(text: "A")
        let context = activeContext(for: state)
        XCTAssertEqual(
            state.applyText(
                "AB",
                identity: event(state, sequence: 1),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .applied(target: .currentGeneration, generation: 10)
        )
        XCTAssertEqual(
            state.applyText(
                "duplicate",
                identity: event(state, sequence: 1),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .duplicateOrOutOfOrder
        )

        var wrongSession = event(state, sequence: 2)
        wrongSession = GaryxComposerInputEventIdentity(
            composerKey: wrongSession.composerKey,
            sessionID: GaryxComposerInputSessionID(rawValue: "wrong"),
            inputSessionEpoch: wrongSession.inputSessionEpoch,
            payloadGeneration: wrongSession.payloadGeneration,
            reservationID: nil,
            inputSequence: wrongSession.inputSequence
        )
        XCTAssertEqual(
            state.applyText(
                "wrong",
                identity: wrongSession,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedUnknownSession
        )

        XCTAssertEqual(
            state.applyText(
                "old",
                identity: event(state, sequence: 2, generation: 9),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedOldGeneration
        )
        XCTAssertEqual(
            state.applyText(
                "future",
                identity: event(state, sequence: 2, generation: 11),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedFutureGeneration
        )

        let reservation = GaryxSendReservationID(rawValue: 1)
        XCTAssertEqual(
            state.beginSend(
                reservationID: reservation,
                followupGeneration: 11,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .sealed(envelope: "AB", followupGeneration: 11)
        )
        XCTAssertEqual(
            state.applyText(
                "U",
                identity: event(state, sequence: 2, generation: 11, reservation: nil),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedReservation
        )
    }

    func testProductReducerAppliesLiveAndFinalizingTextToEveryReservationTarget() {
        for phase in GaryxInputReservationPhase.allCases {
            for finalizing in [false, true] {
                var state = makeState(text: "T")
                let context = activeContext(for: state)
                let reservation = GaryxSendReservationID(rawValue: 10)
                if phase != .none {
                    _ = state.beginSend(
                        reservationID: reservation,
                        followupGeneration: 11,
                        lifecycle: context.lifecycle,
                        scopes: context.scopes
                    )
                    if phase == .committed {
                        XCTAssertTrue(state.commitReservation(
                            lifecycle: context.lifecycle,
                            scopes: context.scopes
                        ))
                    }
                    if phase == .revoked {
                        XCTAssertTrue(state.revokeReservation(
                            mergeGeneration: 12,
                            lifecycle: context.lifecycle,
                            scopes: context.scopes
                        ))
                    }
                }
                if finalizing {
                    XCTAssertEqual(
                        state.releaseForCommittedNavigation(
                            pendingProducers: [.dictation],
                            lifecycle: context.lifecycle,
                            scopes: context.scopes
                        ),
                        .released
                    )
                }

                let expectedTarget = GaryxComposerInputProductReducer.target(
                    reservation: phase,
                    producer: finalizing ? .finalizing : .live
                )
                let expectedGeneration: UInt64 = switch phase {
                case .none: 10
                case .sealed, .committed: 11
                case .revoked: 12
                }
                XCTAssertEqual(
                    state.applyText(
                        "result-\(phase)-\(finalizing)",
                        identity: event(
                            state,
                            sequence: 1,
                            generation: phase == .none ? 10 : 11,
                            reservation: phase == .none ? nil : reservation
                        ),
                        lifecycle: context.lifecycle,
                        scopes: context.scopes
                    ),
                    .applied(target: expectedTarget, generation: expectedGeneration)
                )
                let expectedText = phase == .revoked
                    ? "Tresult-\(phase)-\(finalizing)"
                    : "result-\(phase)-\(finalizing)"
                XCTAssertEqual(state.textByGeneration[expectedGeneration], expectedText)
            }
        }
    }

    func testProducerDrainWhileSealedDoesNotMaterializeOrPublishClose() {
        var state = makeState(text: "T")
        let context = activeContext(for: state)
        let reservation = GaryxSendReservationID(rawValue: 1)
        _ = state.beginSend(
            reservationID: reservation,
            followupGeneration: 11,
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        _ = state.applyText(
            "U",
            identity: event(state, sequence: 1, generation: 11, reservation: reservation),
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        XCTAssertEqual(
            state.releaseForCommittedNavigation(
                pendingProducers: [.dictation],
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .released
        )

        XCTAssertEqual(
            state.producerReachedTerminal(
                .dictation,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .producerDrainedAwaitingReservation
        )
        XCTAssertEqual(state.producerPhase, .terminal)
        XCTAssertEqual(state.producerDrained?.bufferedText, "U")
        XCTAssertNil(state.finalText)
        XCTAssertNil(state.nextEpochSnapshot)
        XCTAssertEqual(state.closePublicationCount, 0)
    }

    func testBeginSendThenReleaseCommitMaterializesGPlusOneExactlyOnce() {
        var state = makeState(text: "T")
        let context = activeContext(for: state)
        let reservation = GaryxSendReservationID(rawValue: 1)
        _ = state.beginSend(
            reservationID: reservation,
            followupGeneration: 11,
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        _ = state.applyText(
            "U",
            identity: event(state, sequence: 1, generation: 11, reservation: reservation),
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        XCTAssertEqual(
            state.releaseForCommittedNavigation(
                pendingProducers: [.dictation],
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .released
        )
        XCTAssertEqual(
            state.producerReachedTerminal(
                .dictation,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .producerDrainedAwaitingReservation
        )

        XCTAssertTrue(state.commitReservation(
            lifecycle: context.lifecycle,
            scopes: context.scopes
        ))
        XCTAssertEqual(state.finalText, "U")
        XCTAssertEqual(
            state.nextEpochSnapshot,
            GaryxComposerEpochSnapshot(sessionEpoch: 2, payloadGeneration: 11, text: "U")
        )
        XCTAssertEqual(state.closePublicationCount, 1)
        XCTAssertFalse(state.commitReservation(
            lifecycle: context.lifecycle,
            scopes: context.scopes
        ))
        XCTAssertEqual(state.closePublicationCount, 1)
    }

    func testBeginSendThenReleaseRevokeMaterializesEnvelopePlusBufferAtGPlusTwo() {
        var state = makeState(text: "T")
        let context = activeContext(for: state)
        let reservation = GaryxSendReservationID(rawValue: 1)
        _ = state.beginSend(
            reservationID: reservation,
            followupGeneration: 11,
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        _ = state.applyText(
            "U",
            identity: event(state, sequence: 1, generation: 11, reservation: reservation),
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        _ = state.releaseForCommittedNavigation(
            pendingProducers: [.dictation],
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        _ = state.producerReachedTerminal(
            .dictation,
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )

        XCTAssertTrue(state.revokeReservation(
            mergeGeneration: 12,
            lifecycle: context.lifecycle,
            scopes: context.scopes
        ))
        XCTAssertEqual(state.finalText, "TU")
        XCTAssertEqual(
            state.nextEpochSnapshot,
            GaryxComposerEpochSnapshot(sessionEpoch: 2, payloadGeneration: 12, text: "TU")
        )
        XCTAssertEqual(state.closePublicationCount, 1)
    }

    func testReservationSettlesBeforeProducerInBothCommitAndRevokeOrders() {
        for committed in [false, true] {
            var state = makeState(text: "T")
            let context = activeContext(for: state)
            let reservation = GaryxSendReservationID(rawValue: 1)
            _ = state.beginSend(
                reservationID: reservation,
                followupGeneration: 11,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            )
            _ = state.applyText(
                "U",
                identity: event(state, sequence: 1, generation: 11, reservation: reservation),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            )
            if committed {
                XCTAssertTrue(state.commitReservation(
                    lifecycle: context.lifecycle,
                    scopes: context.scopes
                ))
            } else {
                XCTAssertTrue(state.revokeReservation(
                    mergeGeneration: 12,
                    lifecycle: context.lifecycle,
                    scopes: context.scopes
                ))
            }
            XCTAssertEqual(
                state.releaseForCommittedNavigation(
                    pendingProducers: [.dictation],
                    lifecycle: context.lifecycle,
                    scopes: context.scopes
                ),
                .released
            )
            XCTAssertEqual(
                state.producerReachedTerminal(
                    .dictation,
                    lifecycle: context.lifecycle,
                    scopes: context.scopes
                ),
                .dualTerminalCommitted
            )
            XCTAssertEqual(state.finalText, committed ? "U" : "TU")
            XCTAssertEqual(state.closePublicationCount, 1)
        }
    }

    func testTwoConsecutiveSealsRejectFirstReservationLateCallback() {
        var state = makeState(text: "S1")
        let context = activeContext(for: state)
        let firstReservation = GaryxSendReservationID(rawValue: 1)
        XCTAssertEqual(
            state.beginSend(
                reservationID: firstReservation,
                followupGeneration: 11,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .sealed(envelope: "S1", followupGeneration: 11)
        )
        XCTAssertEqual(
            state.applyText(
                "U1",
                identity: event(
                    state,
                    sequence: 1,
                    generation: 11,
                    reservation: firstReservation
                ),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .applied(target: .provisionalNextGeneration, generation: 11)
        )
        XCTAssertTrue(
            state.commitReservation(lifecycle: context.lifecycle, scopes: context.scopes)
        )
        XCTAssertTrue(
            state.returnReservationToIdle(
                lifecycle: context.lifecycle,
                scopes: context.scopes
            )
        )
        XCTAssertEqual(state.currentGeneration, 11)
        XCTAssertEqual(state.terminalReservations[firstReservation]?.outcome, .committed)

        let secondReservation = GaryxSendReservationID(rawValue: 2)
        XCTAssertEqual(
            state.beginSend(
                reservationID: secondReservation,
                followupGeneration: 12,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .sealed(envelope: "U1", followupGeneration: 12)
        )
        let before = state.textByGeneration
        XCTAssertEqual(
            state.applyText(
                "late S1",
                identity: event(
                    state,
                    sequence: 2,
                    generation: 11,
                    reservation: firstReservation
                ),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .auditedTerminalReservation
        )
        XCTAssertEqual(state.textByGeneration, before)
        XCTAssertEqual(state.activeReservationID, secondReservation)
    }

    func testReleaseBeforeBeginSendRejectsSendAndFinalizesCurrentGeneration() {
        var state = makeState(text: "final")
        let context = activeContext(for: state)
        XCTAssertEqual(
            state.releaseForCommittedNavigation(
                pendingProducers: [.markedText],
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .released
        )
        XCTAssertEqual(
            state.beginSend(
                reservationID: GaryxSendReservationID(rawValue: 1),
                followupGeneration: 11,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedFinalizing
        )
        XCTAssertEqual(
            state.producerReachedTerminal(
                .markedText,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .dualTerminalCommitted
        )
        XCTAssertEqual(state.finalText, "final")
        XCTAssertEqual(state.nextEpochSnapshot?.payloadGeneration, 10)
    }

    func testReleaseBoundaryImmediatelyClearsFocusAndCommitsPath() {
        var state = makeState(text: "text")
        let context = activeContext(for: state)
        XCTAssertEqual(
            state.releaseForCommittedNavigation(
                pendingProducers: [.dictation],
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .released
        )
        XCTAssertTrue(state.focusClearedAtRelease)
        XCTAssertTrue(state.canonicalPathCommittedAtRelease)
        XCTAssertEqual(state.producerPhase, .finalizing)
        XCTAssertFalse(state.inputReady)
    }

    func testSixDeterministicCancellationEventsTerminateFinalizer() {
        for cancellation in GaryxInputProducerCancellation.allCases {
            var state = makeState(text: "text")
            let context = activeContext(for: state)
            XCTAssertEqual(
                state.releaseForCommittedNavigation(
                    pendingProducers: [.markedText, .dictation, .scribble],
                    lifecycle: context.lifecycle,
                    scopes: context.scopes
                ),
                .released
            )
            XCTAssertEqual(
                state.cancelPendingProducers(
                    cancellation,
                    lifecycle: context.lifecycle,
                    scopes: context.scopes
                ),
                .dualTerminalCommitted,
                "cancellation=\(cancellation)"
            )
            XCTAssertEqual(state.finalizationLease?.terminalCancellation, cancellation)
            XCTAssertEqual(state.closePublicationCount, 1)
        }
    }

    func testCancelledVisibleSourceNeverEntersOrCancelsFinalization() {
        var state = makeState(text: "text")
        let context = activeContext(for: state)
        XCTAssertEqual(
            state.cancelPendingProducers(
                .transactionSettleTerminal,
                committedPath: false,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .alreadyTerminal
        )
        XCTAssertEqual(state.producerPhase, .live)
        XCTAssertNil(state.finalizationLease)
    }

    func testTerminalBoundaryRejectsLateResultButAuditsDuplicateSequence() {
        var state = makeState(text: "A")
        let context = activeContext(for: state)
        _ = state.applyText(
            "AB",
            identity: event(state, sequence: 1),
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        _ = state.releaseForCommittedNavigation(
            pendingProducers: [],
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        XCTAssertEqual(state.finalSequence, 1)

        XCTAssertEqual(
            state.applyText(
                "duplicate",
                identity: event(state, sequence: 1),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .auditedTerminalDuplicate
        )
        XCTAssertEqual(
            state.applyText(
                "late",
                identity: event(state, sequence: 2),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .producerContractFault
        )
        XCTAssertEqual(state.finalText, "AB", "final text is immutable")
    }

    func testTerminalBoundaryAuditsDifferentGenerationBeforeSequenceFaultRule() {
        var state = makeState(text: "A")
        let context = activeContext(for: state)
        _ = state.applyText(
            "AB",
            identity: event(state, sequence: 1),
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        _ = state.releaseForCommittedNavigation(
            pendingProducers: [],
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        let faultsBefore = state.faultCount

        XCTAssertEqual(
            state.applyText(
                "late old generation",
                identity: event(state, sequence: 2, generation: 9),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .auditedTerminalDuplicate
        )
        XCTAssertEqual(state.faultCount, faultsBefore)
        XCTAssertEqual(state.finalText, "AB")
    }

    func testTokenAndScopeCASRejectEveryDurableInputMutation() {
        var state = makeState(text: "A")
        var context = activeContext(for: state)
        let discarding = GaryxPayloadLifecycleSnapshot(
            token: context.lifecycle.token,
            revision: context.lifecycle.revision + 1,
            phase: .discarding
        )
        XCTAssertEqual(
            state.applyText(
                "B",
                identity: event(state, sequence: 1),
                lifecycle: discarding,
                scopes: context.scopes
            ),
            .rejectedToken
        )
        XCTAssertEqual(
            state.beginSend(
                reservationID: GaryxSendReservationID(rawValue: 1),
                followupGeneration: 11,
                lifecycle: discarding,
                scopes: context.scopes
            ),
            .rejectedToken
        )

        _ = context.scopes.revoke(scope)
        XCTAssertEqual(
            state.applyText(
                "B",
                identity: event(state, sequence: 1),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedScope
        )
    }

    func testCloseAckRetiresAndIsIdempotent() {
        var state = makeState(text: "A")
        let context = activeContext(for: state)
        _ = state.releaseForCommittedNavigation(
            pendingProducers: [],
            lifecycle: context.lifecycle,
            scopes: context.scopes
        )
        XCTAssertFalse(state.isRetired)
        state.acknowledgeClose(lifecycle: context.lifecycle, scopes: context.scopes)
        state.acknowledgeClose(lifecycle: context.lifecycle, scopes: context.scopes)
        XCTAssertTrue(state.isRetired)
        XCTAssertEqual(state.closePublicationCount, 1)
    }

    func testRetiredSessionRejectsEveryNewDurableInputAdmission() {
        var state = makeState(text: "old")
        let context = activeContext(for: state)
        XCTAssertEqual(
            state.releaseForCommittedNavigation(
                pendingProducers: [],
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .released
        )
        state.acknowledgeClose(lifecycle: context.lifecycle, scopes: context.scopes)
        XCTAssertTrue(state.isRetired)
        XCTAssertEqual(
            state.applyText(
                "late",
                identity: event(state, sequence: 1),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedRetiredSession
        )
        XCTAssertEqual(
            state.beginSend(
                reservationID: GaryxSendReservationID(rawValue: 99),
                followupGeneration: 11,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedRetiredSession
        )
        XCTAssertEqual(
            state.releaseForCommittedNavigation(
                pendingProducers: [],
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedRetiredSession
        )
    }

    func testTwoConsecutiveEpochHandoffsKeepOldSessionsRetiredAndNewestLive() throws {
        var first = makeState(text: "N", session: "session-1", epoch: 7)
        let context = activeContext(for: first)
        XCTAssertEqual(
            first.releaseForCommittedNavigation(
                pendingProducers: [],
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .released
        )
        let secondSnapshot = try XCTUnwrap(first.nextEpochSnapshot)
        var second = makeState(
            text: secondSnapshot.text,
            session: "session-2",
            epoch: secondSnapshot.sessionEpoch,
            generation: secondSnapshot.payloadGeneration
        )
        XCTAssertEqual(
            second.applyText(
                "N+1",
                identity: event(
                    second,
                    sequence: 1,
                    generation: secondSnapshot.payloadGeneration
                ),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .applied(target: .currentGeneration, generation: secondSnapshot.payloadGeneration)
        )
        first.acknowledgeClose(lifecycle: context.lifecycle, scopes: context.scopes)

        XCTAssertEqual(
            second.releaseForCommittedNavigation(
                pendingProducers: [],
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .released
        )
        let thirdSnapshot = try XCTUnwrap(second.nextEpochSnapshot)
        var third = makeState(
            text: thirdSnapshot.text,
            session: "session-3",
            epoch: thirdSnapshot.sessionEpoch,
            generation: thirdSnapshot.payloadGeneration
        )
        second.acknowledgeClose(lifecycle: context.lifecycle, scopes: context.scopes)
        XCTAssertEqual(third.session.epoch, 9)
        XCTAssertEqual(
            third.applyText(
                "N+2",
                identity: event(
                    third,
                    sequence: 1,
                    generation: thirdSnapshot.payloadGeneration
                ),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .applied(target: .currentGeneration, generation: thirdSnapshot.payloadGeneration)
        )
        XCTAssertEqual(third.currentText, "N+2")
        XCTAssertEqual(
            first.applyText(
                "stale-1",
                identity: event(first, sequence: 1),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedRetiredSession
        )
        XCTAssertEqual(
            second.applyText(
                "stale-2",
                identity: event(
                    second,
                    sequence: 2,
                    generation: thirdSnapshot.payloadGeneration
                ),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedRetiredSession
        )
        XCTAssertEqual(third.currentText, "N+2")
    }

    func testComposerHostActivationUsesLiveFinalizingClosingTransferredPhases() {
        var activation = GaryxComposerHostActivation(
            sourceKey: .thread("A"),
            destinationKey: .thread("A")
        )
        XCTAssertEqual(activation.phase, .live)
        XCTAssertTrue(activation.commitReleased())
        XCTAssertEqual(activation.phase, .finalizingInput)
        activation.producerAndReservationReachedTerminal()
        XCTAssertEqual(activation.phase, .closing)
        activation.closeAcknowledged()
        XCTAssertEqual(activation.phase, .transferred)

        var cancelled = GaryxComposerHostActivation(
            sourceKey: .thread("A"),
            destinationKey: .thread("B")
        )
        cancelled.cancelled()
        XCTAssertEqual(cancelled.phase, .retained)
    }

    func testComposerAdapterTerminalPolicyCoversOutcomeVisibilityAndKeyPreconditions() {
        let key = GaryxComposerKey.thread("same")
        let sameKeyCases: [(
            GaryxPresentationTerminalOutcome,
            GaryxPresentationVisibility,
            GaryxComposerAdapterTerminalDisposition
        )] = [
            (.committed, .visible, .destinationContinuesSameKeyAtNextEpoch),
            (.committed, .inactive, .deferSameKeyDestinationUntilActive),
            (.committed, .superseded, .nextTransaction),
            (.cancelled, .visible, .sourceRemainsLive),
            (.cancelled, .inactive, .deferSourceUntilActive),
            (.cancelled, .superseded, .nextTransaction),
        ]
        for (outcome, visibility, expected) in sameKeyCases {
            XCTAssertEqual(
                GaryxComposerAdapterTerminalPolicy.resolve(
                    sourceKey: key,
                    destinationKey: key,
                    terminal: .init(outcome: outcome, visibility: visibility)
                ),
                expected,
                "outcome=\(outcome), visibility=\(visibility)"
            )
        }

        XCTAssertEqual(
            GaryxComposerAdapterTerminalPolicy.resolve(
                sourceKey: .thread("source"),
                destinationKey: .thread("destination"),
                terminal: .init(outcome: .committed, visibility: .visible)
            ),
            .destinationStartsOwnKeySession
        )
        XCTAssertEqual(
            GaryxComposerAdapterTerminalPolicy.resolve(
                sourceKey: nil,
                destinationKey: .thread("destination"),
                terminal: .init(outcome: .committed, visibility: .inactive)
            ),
            .deferOwnKeyDestinationUntilActive
        )
        XCTAssertEqual(
            GaryxComposerAdapterTerminalPolicy.resolve(
                sourceKey: .thread("source"),
                destinationKey: nil,
                terminal: .init(outcome: .committed, visibility: .visible)
            ),
            .none
        )
        XCTAssertEqual(
            GaryxComposerAdapterTerminalPolicy.resolve(
                sourceKey: nil,
                destinationKey: nil,
                terminal: .init(outcome: .cancelled, visibility: .visible)
            ),
            .none
        )
    }

    func testAliasResolutionIsScopePartitionedAndIndependentFromEmptyText() {
        var registry = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        let other = GaryxGatewayScope(identity: "other", epoch: 1)
        var aliases = GaryxComposerAliasTable()
        aliases.establishPromotion(
            scope: scope,
            source: .draft("D"),
            target: .thread("T"),
            activeOrClosingSessions: 1,
            pendingCloseAcknowledgements: 1
        )
        aliases.establishPromotion(
            scope: other,
            source: .draft("D"),
            target: .thread("OTHER"),
            activeOrClosingSessions: 1
        )

        XCTAssertEqual(
            aliases.resolve(.draft("D"), scope: scope, scopes: registry),
            .resolved(.thread("T"))
        )
        XCTAssertEqual(
            aliases.resolve(.draft("D"), scope: other, scopes: registry),
            .rejectedRevokedScope,
            "unknown scopes default to revoked"
        )
        XCTAssertFalse(aliases.retireIfDrained(source: .draft("D"), scope: scope))
        XCTAssertTrue(aliases.markDrained(source: .draft("D"), scope: scope))
        XCTAssertEqual(aliases.aliasCount, aliases.activeRetiringSourceCount)

        XCTAssertTrue(registry.switchActive(to: other))
        XCTAssertEqual(
            aliases.resolve(.draft("D"), scope: other, scopes: registry),
            .resolved(.thread("OTHER"))
        )
    }

    func testAliasPromotionChurnDrainsToZeroWithinByteBudget() {
        var aliases = GaryxComposerAliasTable()
        for index in 0..<500 {
            let source = GaryxComposerKey.draft("draft-\(index)")
            aliases.establishPromotion(
                scope: scope,
                source: source,
                target: .thread("thread-\(index)"),
                activeOrClosingSessions: 1
            )
            XCTAssertTrue(aliases.markDrained(source: source, scope: scope))
        }
        XCTAssertEqual(aliases.aliasCount, 0)
        XCTAssertEqual(aliases.activeRetiringSourceCount, 0)
        XCTAssertLessThanOrEqual(aliases.estimatedBytes, 64 * 1024)
    }

    func testAliasLineageReleaseDecrementsEverySharedSuffixOccupancyCounter() {
        let discardedSource = GaryxComposerKey.draft("discarded")
        let liveSource = GaryxComposerKey.draft("live")
        let sharedIntermediate = GaryxComposerKey.thread("shared")
        let destination = GaryxComposerKey.thread("destination")
        var aliases = GaryxComposerAliasTable()
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: discardedSource,
                target: sharedIntermediate,
                activeOrClosingSessions: 1,
                pendingCloseAcknowledgements: 1,
                promotionsInFlight: 1
            ),
            .established
        )
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: liveSource,
                target: sharedIntermediate,
                activeOrClosingSessions: 2
            ),
            .established
        )
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: sharedIntermediate,
                target: destination,
                activeOrClosingSessions: 3,
                pendingCloseAcknowledgements: 1,
                promotionsInFlight: 1
            ),
            .established
        )

        XCTAssertEqual(
            aliases.retireLineage(
                startingAt: [discardedSource],
                endingAt: destination,
                scope: scope
            ),
            1
        )
        XCTAssertNil(aliases.partitions[scope]?[discardedSource])
        XCTAssertEqual(
            aliases.partitions[scope]?[sharedIntermediate]?.activeOrClosingSessions,
            2
        )
        XCTAssertEqual(
            aliases.partitions[scope]?[sharedIntermediate]?.pendingCloseAcknowledgements,
            0
        )
        XCTAssertEqual(aliases.partitions[scope]?[sharedIntermediate]?.promotionsInFlight, 0)
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        XCTAssertEqual(
            aliases.resolve(liveSource, scope: scope, scopes: scopes),
            .resolved(destination)
        )
        XCTAssertTrue(aliases.invariantHolds)
        XCTAssertEqual(
            aliases.retireLineage(
                startingAt: [liveSource],
                endingAt: destination,
                scope: scope
            ),
            2,
            "the final predecessor must own full cleanup of the shared suffix"
        )
        XCTAssertEqual(aliases.aliasCount, 0)
    }

    func testAliasLineageReleaseCountsNestedCapturedOriginsOnlyOnce() {
        let earliestSource = GaryxComposerKey.draft("earliest")
        let liveSource = GaryxComposerKey.draft("live-nested")
        let laterCapturedSource = GaryxComposerKey.thread("later-captured")
        let destination = GaryxComposerKey.thread("nested-destination")
        var aliases = GaryxComposerAliasTable()
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: earliestSource,
                target: laterCapturedSource,
                activeOrClosingSessions: 2
            ),
            .established
        )
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: liveSource,
                target: laterCapturedSource,
                activeOrClosingSessions: 1
            ),
            .established
        )
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: laterCapturedSource,
                target: destination,
                activeOrClosingSessions: 2
            ),
            .established
        )

        XCTAssertEqual(
            aliases.retireLineage(
                startingAt: [earliestSource, laterCapturedSource],
                endingAt: destination,
                scope: scope
            ),
            1
        )
        XCTAssertNil(aliases.partitions[scope]?[earliestSource])
        XCTAssertEqual(
            aliases.partitions[scope]?[laterCapturedSource]?.activeOrClosingSessions,
            1
        )
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        XCTAssertEqual(
            aliases.resolve(liveSource, scope: scope, scopes: scopes),
            .resolved(destination)
        )
    }

    private func makeState(
        text: String,
        session: String = "session",
        epoch: UInt64 = 1,
        generation: UInt64 = 10
    ) -> GaryxComposerInputReducerState {
        let entryID = GaryxComposerPayloadEntryID(rawValue: "entry")
        let token = GaryxPayloadLifecycleToken(entryID: entryID, nonce: "token")
        return GaryxComposerInputReducerState(
            session: GaryxComposerInputSession(
                composerKey: .draft("draft"),
                sessionID: GaryxComposerInputSessionID(rawValue: session),
                epoch: epoch,
                scope: scope,
                payloadLifecycle: GaryxPayloadLifecycleCapture(token: token, revision: 1)
            ),
            payloadGeneration: generation,
            initialText: text
        )
    }

    private func activeContext(
        for state: GaryxComposerInputReducerState
    ) -> (lifecycle: GaryxPayloadLifecycleSnapshot, scopes: GaryxGatewayScopeRegistry) {
        (
            GaryxPayloadLifecycleSnapshot(
                token: state.session.payloadLifecycle.token,
                revision: state.session.payloadLifecycle.revision,
                phase: .active
            ),
            GaryxGatewayScopeRegistry(initialActiveScope: scope)
        )
    }

    private func event(
        _ state: GaryxComposerInputReducerState,
        sequence: UInt64,
        generation: UInt64 = 10,
        reservation: GaryxSendReservationID? = nil
    ) -> GaryxComposerInputEventIdentity {
        GaryxComposerInputEventIdentity(
            composerKey: state.session.composerKey,
            sessionID: state.session.sessionID,
            inputSessionEpoch: state.session.epoch,
            payloadGeneration: generation,
            reservationID: reservation,
            inputSequence: sequence
        )
    }
}
