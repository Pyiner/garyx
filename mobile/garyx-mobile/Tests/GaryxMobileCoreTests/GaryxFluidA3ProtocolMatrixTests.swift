import XCTest
@testable import GaryxMobileCore

final class GaryxFluidA3ProtocolMatrixTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "gateway", epoch: 1)
    private let otherScope = GaryxGatewayScope(identity: "other", epoch: 1)

    func testPromotionAcrossPopPhasesFinishCancelAndGatewaySwitch() {
        struct Scenario {
            let label: String
            let pathPoppedBeforePromotion: Bool
            let pathPopsAfterPromotion: Bool
        }
        let scenarios = [
            Scenario(
                label: "preCommit-finish",
                pathPoppedBeforePromotion: false,
                pathPopsAfterPromotion: true
            ),
            Scenario(
                label: "preCommit-cancel",
                pathPoppedBeforePromotion: false,
                pathPopsAfterPromotion: false
            ),
            Scenario(
                label: "cancelSettle-finishCancel",
                pathPoppedBeforePromotion: false,
                pathPopsAfterPromotion: false
            ),
            Scenario(
                label: "cancelSettle-regrabThenFinish",
                pathPoppedBeforePromotion: false,
                pathPopsAfterPromotion: true
            ),
            Scenario(
                label: "commitSettle-finish",
                pathPoppedBeforePromotion: true,
                pathPopsAfterPromotion: false
            ),
            Scenario(
                label: "terminal-cancelled",
                pathPoppedBeforePromotion: false,
                pathPopsAfterPromotion: false
            ),
            Scenario(
                label: "terminal-committed",
                pathPoppedBeforePromotion: true,
                pathPopsAfterPromotion: false
            ),
        ]

        for scenario in scenarios {
            for scopeMatches in [true, false] {
                var routes = GaryxCanonicalRouteState()
                let base = route("base", .panel("agents"))
                let draft = route("draft-occurrence", .conversationDraft(draftID: "draft"))
                _ = routes.open(base)
                _ = routes.open(draft)
                if scenario.pathPoppedBeforePromotion { _ = routes.pop() }
                let stackRevisionAtPromotion = routes.stackRevision

                var promotionScopes = GaryxGatewayScopeRegistry(initialActiveScope: scope)
                if !scopeMatches {
                    XCTAssertTrue(promotionScopes.switchActive(to: otherScope))
                }
                let result = routes.promoteDraft(
                    promotionRequest(),
                    scopes: promotionScopes,
                    outboxAdmission: .succeeded
                )

                let expectedNavigation: GaryxDraftPromotionNavigationDisposition
                if !scopeMatches {
                    expectedNavigation = .originScopePartitionOnly
                } else if scenario.pathPoppedBeforePromotion {
                    expectedNavigation = .domainOnlyLate
                } else {
                    expectedNavigation = .updatedInPlace
                }
                XCTAssertEqual(
                    result.navigation,
                    expectedNavigation,
                    "scenario=\(scenario.label), scopeMatches=\(scopeMatches)"
                )
                XCTAssertEqual(routes.stackRevision, stackRevisionAtPromotion)
                XCTAssertTrue(result.preservedPresentationLease)
                XCTAssertTrue(result.keptOptimisticThread)
                XCTAssertTrue(result.migratedDomainInOriginScope)
                XCTAssertEqual(result.outboxInsertCount, 0)
                XCTAssertEqual(result.dispatchCountDelta, 0)

                if scenario.pathPopsAfterPromotion { _ = routes.pop() }
                if scenario.pathPoppedBeforePromotion || scenario.pathPopsAfterPromotion {
                    XCTAssertEqual(routes.path, [base], scenario.label)
                } else {
                    XCTAssertEqual(
                        routes.path.last?.destination,
                        scopeMatches
                            ? .conversation(threadID: "thread")
                            : .conversationDraft(draftID: "draft"),
                        "scenario=\(scenario.label), scopeMatches=\(scopeMatches)"
                    )
                }
            }
        }
    }

    func testDestructiveIdentityByBarrierPhaseOrderAndEveryMutationKind() throws {
        enum DestructiveEvent: CaseIterable {
            case destination
            case entry
        }
        enum RaceOrder: CaseIterable {
            case discardWins
            case ordinaryBarrierMutationWins
        }

        for destructiveEvent in DestructiveEvent.allCases {
            for initialPhase in GaryxSendCommitBarrierPhase.allCases {
                for order in RaceOrder.allCases {
                    var store = GaryxComposerPayloadStore()
                    let entry = makeEntry()
                    XCTAssertTrue(store.insert(entry))
                    let capture = GaryxPayloadLifecycleCapture(
                        token: entry.lifecycle.token,
                        revision: entry.lifecycle.revision
                    )
                    var setup = try makeBarrier(phase: initialPhase, entry: entry)

                    if order == .ordinaryBarrierMutationWins {
                        switch initialPhase {
                        case .idle:
                            XCTAssertEqual(
                                setup.barrier.seal(
                                    reservationID: reservation,
                                    envelope: envelope(),
                                    followupGeneration: 2,
                                    readiness: .ready,
                                    quota: .init(),
                                    producerPhase: .live,
                                    lifecycle: entry.lifecycle.snapshot
                                ),
                                .sealed
                            )
                        case .sealed:
                            let settlement = try XCTUnwrap(
                                setup.barrier.durableCommit(
                                    deliveryID: deliveryID("race"),
                                    correlationID: "race",
                                    clientIntentID: "intent",
                                    lifecycle: entry.lifecycle.snapshot
                                )
                            )
                            let delivery = try XCTUnwrap(settlement.deliveryRecord)
                            setup.deliveries[delivery.id] = delivery
                        case .durableCommitted, .revoked:
                            setup.barrier.returnToIdle()
                        }
                    }

                    let identityDisposition = GaryxPayloadIdentityReducer.apply(
                        destructiveEvent == .destination
                            ? .destinationDiscarded(.draft("draft"), revision: 9)
                            : .payloadEntryDiscarded(entryID, revision: 9),
                        scope: scope,
                        store: &store
                    )
                    XCTAssertEqual(identityDisposition, .beganDiscard([entryID]))
                    let discarding = try XCTUnwrap(
                        store.entry(entryID, scope: scope)?.lifecycle.snapshot
                    )

                    for kind in GaryxPayloadMutationKind.allCases {
                        XCTAssertEqual(
                            GaryxPayloadMutationGate.admit(
                                kind,
                                capture: capture,
                                lifecycle: discarding
                            ),
                            .rejectedLifecycle,
                            "event=\(destructiveEvent), phase=\(initialPhase), order=\(order), kind=\(kind)"
                        )
                    }

                    if order == .discardWins {
                        XCTAssertFalse(
                            setup.barrier.replaceProvisionalText(
                                "late",
                                lifecycle: discarding
                            )
                        )
                        XCTAssertFalse(
                            setup.barrier.addProvisionalAttachment(
                                GaryxAttachmentID(rawValue: "late"),
                                lifecycle: discarding
                            )
                        )
                        XCTAssertNil(
                            setup.barrier.durableCommit(
                                deliveryID: deliveryID("late"),
                                correlationID: "late",
                                clientIntentID: "late",
                                lifecycle: discarding
                            )
                        )
                        XCTAssertNil(
                            setup.barrier.revoke(
                                mergeGeneration: 3,
                                lifecycle: discarding
                            )
                        )
                        XCTAssertEqual(
                            setup.barrier.seal(
                                reservationID: GaryxSendReservationID(rawValue: 2),
                                envelope: envelope(),
                                followupGeneration: 2,
                                readiness: .ready,
                                quota: .init(),
                                producerPhase: .live,
                                lifecycle: discarding
                            ),
                            .rejectedLifecycle
                        )
                    }

                    let lifecycle = try XCTUnwrap(store.entry(entryID, scope: scope)?.lifecycle)
                    var convergence = GaryxPayloadDiscardConvergence(
                        lifecycle: lifecycle,
                        barrier: setup.barrier,
                        deliveries: setup.deliveries
                    )
                    convergence.settleDeliveries()
                    convergence.settleReservation()
                    convergence.settleSessions()
                    convergence.settleResources()
                    XCTAssertTrue(
                        convergence.finishToken(),
                        "event=\(destructiveEvent), phase=\(initialPhase), order=\(order)"
                    )
                    XCTAssertEqual(convergence.lifecycle.phase, .discarded)
                    XCTAssertTrue(convergence.deliveriesSettled)
                    XCTAssertTrue(convergence.descendantsEmpty)
                }
            }
        }
    }

    func testIdentityDiscardIsBoundToOneGatewayPartition() {
        var store = GaryxComposerPayloadStore()
        let first = makeEntry(scope: scope, id: "first")
        let second = makeEntry(scope: otherScope, id: "second")
        XCTAssertTrue(store.insert(first))
        XCTAssertTrue(store.insert(second))

        XCTAssertEqual(
            GaryxPayloadIdentityReducer.apply(
                .destinationDiscarded(.draft("draft"), revision: 4),
                scope: scope,
                store: &store
            ),
            .beganDiscard([first.id])
        )
        XCTAssertEqual(store.entry(first.id, scope: scope)?.lifecycle.phase, .discarding)
        XCTAssertEqual(store.entry(second.id, scope: otherScope)?.lifecycle.phase, .active)
    }

    func testDiscardedLifecycleRejectsEveryConcreteInputPath() throws {
        let entry = makeEntry()
        let active = entry.lifecycle.snapshot
        let discarding = GaryxPayloadLifecycleSnapshot(
            token: active.token,
            revision: active.revision + 1,
            phase: .discarding
        )
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope)

        var live = makeInputState(entry: entry, session: "live", epoch: 1)
        XCTAssertEqual(
            live.applyText(
                "late",
                identity: inputEvent(for: live, sequence: 1),
                lifecycle: discarding,
                scopes: scopes
            ),
            .rejectedToken
        )
        XCTAssertEqual(
            live.beginSend(
                reservationID: reservation,
                followupGeneration: 2,
                lifecycle: discarding,
                scopes: scopes
            ),
            .rejectedToken
        )
        XCTAssertEqual(
            live.releaseForCommittedNavigation(
                pendingProducers: [.dictation],
                lifecycle: discarding,
                scopes: scopes
            ),
            .rejectedToken
        )

        var finalizing = makeInputState(entry: entry, session: "finalizing", epoch: 2)
        XCTAssertEqual(
            finalizing.releaseForCommittedNavigation(
                pendingProducers: [.dictation],
                lifecycle: active,
                scopes: scopes
            ),
            .released
        )
        XCTAssertEqual(
            finalizing.applyText(
                "late dictation",
                identity: inputEvent(for: finalizing, sequence: 1),
                lifecycle: discarding,
                scopes: scopes
            ),
            .rejectedToken
        )
        XCTAssertEqual(
            finalizing.producerReachedTerminal(
                .dictation,
                lifecycle: discarding,
                scopes: scopes
            ),
            .rejectedToken
        )
        XCTAssertEqual(
            finalizing.cancelPendingProducers(
                .scopeRevoke,
                lifecycle: discarding,
                scopes: scopes
            ),
            .rejectedToken
        )
        XCTAssertNil(finalizing.producerDrained)
        XCTAssertNil(finalizing.nextEpochSnapshot)

        var sealed = makeInputState(entry: entry, session: "sealed", epoch: 3)
        XCTAssertEqual(
            sealed.beginSend(
                reservationID: reservation,
                followupGeneration: 2,
                lifecycle: active,
                scopes: scopes
            ),
            .sealed(envelope: "T", followupGeneration: 2)
        )
        XCTAssertEqual(
            sealed.releaseForCommittedNavigation(
                pendingProducers: [],
                lifecycle: active,
                scopes: scopes
            ),
            .released
        )
        XCTAssertNotNil(sealed.producerDrained)
        XCTAssertFalse(sealed.commitReservation(lifecycle: discarding, scopes: scopes))
        XCTAssertFalse(
            sealed.revokeReservation(
                mergeGeneration: 3,
                lifecycle: discarding,
                scopes: scopes
            )
        )
        XCTAssertNil(sealed.nextEpochSnapshot)
        XCTAssertEqual(sealed.closePublicationCount, 0)

        var entryForReset = entry
        XCTAssertFalse(
            entryForReset.resetGeneration(
                entryForReset.currentGeneration,
                to: entryForReset.currentGeneration + 1,
                barrierIdle: false,
                producerLive: true
            )
        )
        XCTAssertFalse(
            entryForReset.resetGeneration(
                entryForReset.currentGeneration,
                to: entryForReset.currentGeneration + 1,
                barrierIdle: true,
                producerLive: false
            )
        )
        XCTAssertTrue(entryForReset.beginDiscard(revision: 10))
        XCTAssertFalse(
            entryForReset.resetGeneration(
                entryForReset.currentGeneration,
                to: entryForReset.currentGeneration + 1,
                barrierIdle: true,
                producerLive: true
            )
        )
    }

    func testEpochHandoffAcceptsNewInputBeforeOrAfterOldCloseAck() throws {
        for acknowledgeOldFirst in [false, true] {
            let entry = makeEntry(text: "日")
            let lifecycle = entry.lifecycle.snapshot
            let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope)
            var old = makeInputState(entry: entry, session: "old", epoch: 7)
            XCTAssertEqual(
                old.releaseForCommittedNavigation(
                    pendingProducers: [.dictation],
                    lifecycle: lifecycle,
                    scopes: scopes
                ),
                .released
            )
            XCTAssertEqual(
                old.applyText(
                    "日本語完成",
                    identity: inputEvent(for: old, sequence: 1),
                    lifecycle: lifecycle,
                    scopes: scopes
                ),
                .applied(target: .currentGeneration, generation: 1)
            )
            XCTAssertEqual(
                old.producerReachedTerminal(
                    .dictation,
                    lifecycle: lifecycle,
                    scopes: scopes
                ),
                .dualTerminalCommitted
            )
            let next = try XCTUnwrap(old.nextEpochSnapshot)
            XCTAssertEqual(next.text, "日本語完成")
            XCTAssertEqual(next.sessionEpoch, 8)

            var current = makeInputState(
                entry: entry,
                session: "new",
                epoch: next.sessionEpoch,
                text: next.text
            )
            if acknowledgeOldFirst {
                old.acknowledgeClose(lifecycle: lifecycle, scopes: scopes)
            }
            XCTAssertEqual(
                current.applyText(
                    "日本語完成！",
                    identity: inputEvent(for: current, sequence: 1),
                    lifecycle: lifecycle,
                    scopes: scopes
                ),
                .applied(target: .currentGeneration, generation: 1)
            )
            if !acknowledgeOldFirst {
                old.acknowledgeClose(lifecycle: lifecycle, scopes: scopes)
            }
            XCTAssertTrue(old.isRetired)
            XCTAssertEqual(current.currentText, "日本語完成！")

            let staleOldIdentity = GaryxComposerInputEventIdentity(
                composerKey: old.session.composerKey,
                sessionID: old.session.sessionID,
                inputSessionEpoch: old.session.epoch,
                payloadGeneration: 1,
                reservationID: nil,
                inputSequence: 2
            )
            XCTAssertEqual(
                current.applyText(
                    "must not overwrite",
                    identity: staleOldIdentity,
                    lifecycle: lifecycle,
                    scopes: scopes
                ),
                .rejectedUnknownSession
            )
            XCTAssertEqual(current.currentText, "日本語完成！")
        }
    }

    private var entryID: GaryxComposerPayloadEntryID {
        GaryxComposerPayloadEntryID(rawValue: "entry")
    }

    private var reservation: GaryxSendReservationID {
        GaryxSendReservationID(rawValue: 1)
    }

    private func route(
        _ id: String,
        _ destination: GaryxRouteDestination
    ) -> GaryxRouteEntry {
        GaryxRouteEntry(id: GaryxRouteInstanceID(rawValue: id), destination: destination)
    }

    private func promotionRequest() -> GaryxDraftPromotionRequest {
        GaryxDraftPromotionRequest(
            instanceID: GaryxRouteInstanceID(rawValue: "draft-occurrence"),
            draftID: "draft",
            threadID: "thread",
            originScope: scope,
            clientIntentID: "intent",
            sendStage: .serverAcknowledged
        )
    }

    private func makeEntry(
        scope: GaryxGatewayScope? = nil,
        id: String = "entry",
        text: String = "T"
    ) -> GaryxComposerPayloadEntry {
        let resolvedScope = scope ?? self.scope
        let id = GaryxComposerPayloadEntryID(rawValue: id)
        return GaryxComposerPayloadEntry(
            id: id,
            scope: resolvedScope,
            destination: .draft("draft"),
            lifecycleToken: GaryxPayloadLifecycleToken(entryID: id, nonce: "token-\(id.rawValue)"),
            currentGeneration: 1,
            text: text
        )
    }

    private func envelope() -> GaryxDeliveryEnvelope {
        GaryxDeliveryEnvelope(
            text: "T",
            attachmentIDs: [],
            generation: 1,
            clientIntentID: "intent"
        )
    }

    private func deliveryID(_ value: String) -> GaryxDeliveryRecordID {
        GaryxDeliveryRecordID(rawValue: value)
    }

    private func makeBarrier(
        phase: GaryxSendCommitBarrierPhase,
        entry: GaryxComposerPayloadEntry
    ) throws -> (
        barrier: GaryxSendCommitBarrier,
        deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord]
    ) {
        var barrier = GaryxSendCommitBarrier(
            entryID: entry.id,
            scope: entry.scope,
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        var deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord] = [:]
        guard phase != .idle else { return (barrier, deliveries) }
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservation,
                envelope: envelope(),
                followupGeneration: 2,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        switch phase {
        case .idle, .sealed:
            break
        case .durableCommitted:
            let settlement = try XCTUnwrap(
                barrier.durableCommit(
                    deliveryID: deliveryID("existing"),
                    correlationID: "existing",
                    clientIntentID: "intent",
                    lifecycle: entry.lifecycle.snapshot
                )
            )
            let delivery = try XCTUnwrap(settlement.deliveryRecord)
            deliveries[delivery.id] = delivery
        case .revoked:
            _ = try XCTUnwrap(
                barrier.revoke(
                    mergeGeneration: 3,
                    lifecycle: entry.lifecycle.snapshot
                )
            )
        }
        return (barrier, deliveries)
    }

    private func makeInputState(
        entry: GaryxComposerPayloadEntry,
        session: String,
        epoch: UInt64,
        text: String? = nil
    ) -> GaryxComposerInputReducerState {
        let session = GaryxComposerInputSession(
            composerKey: entry.destination,
            sessionID: GaryxComposerInputSessionID(rawValue: session),
            epoch: epoch,
            scope: entry.scope,
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        return GaryxComposerInputReducerState(
            session: session,
            payloadGeneration: entry.currentGeneration,
            initialText: text ?? entry.currentText
        )
    }

    private func inputEvent(
        for state: GaryxComposerInputReducerState,
        sequence: UInt64
    ) -> GaryxComposerInputEventIdentity {
        GaryxComposerInputEventIdentity(
            composerKey: state.session.composerKey,
            sessionID: state.session.sessionID,
            inputSessionEpoch: state.session.epoch,
            payloadGeneration: state.currentGeneration,
            reservationID: state.activeReservationID,
            inputSequence: sequence
        )
    }
}
