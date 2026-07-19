#if os(macOS)
import Darwin
import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxComposerDurabilityCrashHarnessTests: XCTestCase {
    private static let deliveryID = "crash-delivery"
    private static let reservationID = "9"

    func testCommitSendKillRelaunchAtEverySQLiteBoundaryNeverHangs() throws {
        for boundary in commitSendBoundaries {
            let fixture = try makeFixture(label: "send-kill-\(boundary.name)")
            try seedSealed(fixture)
            _ = try run(
                "commit-send",
                fixture: fixture,
                extra: ["--kill", boundary.name],
                expecting: .killed
            )
            let summary = try recover(fixture)
            assertSendRecovery(summary, committed: boundary.committed, context: boundary.name)
        }
    }

    func testCommitSendENOSPCAndFsyncFailureAtEverySQLiteBoundaryAreAtomic() throws {
        for failure in [FailureKind.enospc, .fsync] {
            for boundary in commitSendBoundaries {
                let fixture = try makeFixture(label: "send-\(failure.rawValue)-\(boundary.name)")
                try seedSealed(fixture)
                _ = try run(
                    "commit-send",
                    fixture: fixture,
                    extra: [
                        "--fail", boundary.name,
                        "--failure", failure.rawValue,
                    ],
                    expecting: .failed
                )
                let summary = try recover(fixture)
                assertSendRecovery(
                    summary,
                    committed: boundary.committed,
                    context: "\(failure.rawValue):\(boundary.name)"
                )
            }
        }
    }

    func testUndispatchedRecoveryExitIsAtomicAtEverySQLiteBoundaryAndReclaimsQuota() throws {
        for boundary in undispatchedRecoveryBoundaries {
            let fixture = try makeFixture(label: "undispatched-recovery-\(boundary.name)")
            try seedCommitted(fixture)
            _ = try run(
                "recover",
                fixture: fixture,
                extra: [
                    "--kill", boundary.name,
                    "--occurrence", String(boundary.occurrence),
                ],
                expecting: .killed
            )
            var summary = try recover(fixture)
            assertUndispatchedRecovered(summary, context: boundary.name)
            summary = try recover(fixture)
            assertUndispatchedRecovered(summary, context: "second:\(boundary.name)")
        }
    }

    func testRealSQLiteFullRollsBackAndRelaunchesWithoutPartialState() throws {
        let fixture = try makeFixture(label: "true-sqlite-full")
        let result = try run("true-sqlite-full", fixture: fixture)
        let full = try JSONDecoder().decode(
            TrueSQLiteFullSummary.self,
            from: result.standardOutput
        )
        XCTAssertEqual(full.sqliteCode & 0xff, 13, "must be SQLite's real SQLITE_FULL")
        XCTAssertGreaterThan(full.maximumPageCount, 0)
        XCTAssertEqual(full.revision, 0)
        XCTAssertFalse(full.entryPresent)

        let relaunched = try inspect(fixture)
        XCTAssertEqual(relaunched.revision, 0)
        XCTAssertNil(relaunched.currentText)
        XCTAssertTrue(relaunched.deliveryPhases.isEmpty)
    }

    func testAttemptAmbiguousAndAcknowledgementCrashBoundariesHaveOnlySafeExits() throws {
        let cases: [TransportCrashCase] = [
            .init(
                setupActions: [],
                action: "attempt",
                extra: ["--kill", "beforeCommit"],
                expectedPhase: "abandoned",
                expectedDisposition: "terminal"
            ),
            .init(
                setupActions: [],
                action: "attempt",
                extra: ["--kill", "afterCommit"],
                expectedPhase: "ambiguous",
                expectedDisposition: "userTerminable"
            ),
            .init(
                setupActions: [],
                action: "attempt-then-kill",
                extra: [],
                expectedPhase: "ambiguous",
                expectedDisposition: "userTerminable"
            ),
            .init(
                setupActions: ["attempt"],
                action: "ambiguous",
                extra: ["--kill", "beforeCommit"],
                expectedPhase: "ambiguous",
                expectedDisposition: "userTerminable"
            ),
            .init(
                setupActions: ["attempt"],
                action: "ambiguous",
                extra: ["--kill", "afterCommit"],
                expectedPhase: "ambiguous",
                expectedDisposition: "userTerminable"
            ),
            .init(
                setupActions: ["attempt"],
                action: "ack",
                extra: ["--kill", "beforeCommit"],
                expectedPhase: "ambiguous",
                expectedDisposition: "userTerminable"
            ),
            .init(
                setupActions: ["attempt"],
                action: "ack",
                extra: ["--kill", "afterCommit"],
                expectedPhase: "acknowledged",
                expectedDisposition: "acknowledged"
            ),
        ]

        for (index, crashCase) in cases.enumerated() {
            let fixture = try makeFixture(label: "transport-\(index)")
            try seedCommitted(fixture)
            for action in crashCase.setupActions {
                _ = try run(action, fixture: fixture)
            }
            _ = try run(
                crashCase.action,
                fixture: fixture,
                extra: crashCase.extra,
                expecting: .killed
            )
            let summary = try recover(fixture)
            XCTAssertEqual(
                summary.deliveryPhases[Self.deliveryID],
                crashCase.expectedPhase,
                "case \(index)"
            )
            XCTAssertEqual(
                summary.deliveryDispositions[Self.deliveryID],
                crashCase.expectedDisposition,
                "case \(index)"
            )
            if crashCase.expectedPhase == "abandoned" {
                assertUndispatchedRecovered(summary, context: "case \(index)")
            }
        }
    }

    func testAmbiguousExitTransactionsAndLateEvidenceRacesConverge() throws {
        for action in ["restore-delivery", "resend-delivery"] {
            for boundary in ["beforeCommit", "afterCommit"] {
                let fixture = try makeFixture(label: "\(action)-\(boundary)")
                try seedAmbiguousDelivery(fixture)
                _ = try run(
                    action,
                    fixture: fixture,
                    extra: ["--kill", boundary],
                    expecting: .killed
                )
                let summary = try recover(fixture)
                if boundary == "beforeCommit" {
                    XCTAssertEqual(summary.deliveryPhases[Self.deliveryID], "ambiguous")
                    XCTAssertEqual(summary.deliveryUserDispositions[Self.deliveryID], "none")
                    XCTAssertEqual(summary.conflictCount, 0)
                    XCTAssertEqual(summary.entryCount, 1)
                } else if action == "restore-delivery" {
                    XCTAssertEqual(summary.deliveryPhases[Self.deliveryID], "abandoned")
                    XCTAssertEqual(
                        summary.deliveryUserDispositions[Self.deliveryID],
                        "restoredToDraft"
                    )
                    XCTAssertEqual(summary.conflictCount, 1)
                    XCTAssertEqual(summary.entryCount, 2)
                } else {
                    XCTAssertEqual(
                        summary.deliveryPhases[Self.deliveryID],
                        "supersededByDuplicate"
                    )
                    XCTAssertEqual(
                        summary.deliveryUserDispositions[Self.deliveryID],
                        "resentAsDuplicate"
                    )
                    XCTAssertEqual(summary.deliveryPhases["crash-delivery-copy"], "abandoned")
                    XCTAssertEqual(
                        summary.deliveryUserDispositions["crash-delivery-copy"],
                        "restoredToDraft"
                    )
                    XCTAssertEqual(summary.conflictCount, 1)
                    XCTAssertEqual(summary.nonTerminalDeliveryGlobal, 0)
                }
            }

            let exitFirst = try makeFixture(label: "\(action)-then-evidence")
            try seedAmbiguousDelivery(exitFirst)
            _ = try run(
                action,
                fixture: exitFirst,
                extra: ["--kill", "afterCommit"],
                expecting: .killed
            )
            _ = try run("evidence", fixture: exitFirst)
            let exitFirstSummary = try inspect(exitFirst)
            XCTAssertEqual(
                exitFirstSummary.deliveryEvidence[Self.deliveryID],
                "serverAcknowledged"
            )
            XCTAssertEqual(
                exitFirstSummary.deliveryPhases[Self.deliveryID],
                action == "restore-delivery" ? "abandoned" : "supersededByDuplicate"
            )

            let evidenceFirst = try makeFixture(label: "evidence-then-\(action)")
            try seedAmbiguousDelivery(evidenceFirst)
            _ = try run("evidence", fixture: evidenceFirst)
            _ = try run(action, fixture: evidenceFirst, expecting: .failed)
            let evidenceFirstSummary = try inspect(evidenceFirst)
            XCTAssertEqual(evidenceFirstSummary.deliveryPhases[Self.deliveryID], "acknowledged")
            XCTAssertEqual(
                evidenceFirstSummary.deliveryEvidence[Self.deliveryID],
                "serverAcknowledged"
            )
            XCTAssertEqual(evidenceFirstSummary.deliveryUserDispositions[Self.deliveryID], "none")
        }
    }

    func testEveryMultiStageCreateServerCommitDeathHasAtomicUserExits() throws {
        let stages = [
            (name: "create", ambiguousAfter: "createPending", deliveryPhase: "notDispatched"),
            (name: "binding", ambiguousAfter: "threadCreated", deliveryPhase: "notDispatched"),
            (name: "chat", ambiguousAfter: "chatStartAttempted", deliveryPhase: "ambiguous"),
        ]

        for stage in stages {
            let death = try makeFixture(label: "create-death-\(stage.name)")
            try seedCreateDeath(death, stage: stage.name)
            let deathSummary = try recover(death)
            XCTAssertEqual(deathSummary.createDeliveryPhases["crash-correlation"], "ambiguous")
            XCTAssertEqual(
                deathSummary.createDeliveryAmbiguousAfter["crash-correlation"],
                stage.ambiguousAfter
            )
            XCTAssertEqual(deathSummary.createDeliveryDispositions["crash-correlation"], "none")
            XCTAssertEqual(deathSummary.deliveryPhases[Self.deliveryID], stage.deliveryPhase)

            let restore = try makeFixture(label: "create-restore-\(stage.name)")
            try seedCreateDeath(restore, stage: stage.name)
            _ = try recover(restore)
            _ = try run(
                "restore-create",
                fixture: restore,
                extra: ["--kill", "afterCommit"],
                expecting: .killed
            )
            let restored = try recover(restore)
            XCTAssertEqual(restored.deliveryPhases[Self.deliveryID], "abandoned")
            XCTAssertEqual(
                restored.createDeliveryDispositions["crash-correlation"],
                "restoredToDraft"
            )
            XCTAssertEqual(restored.conflictCount, 1)
            XCTAssertEqual(restored.entryCount, 2)

            let rebuild = try makeFixture(label: "create-rebuild-\(stage.name)")
            try seedCreateDeath(rebuild, stage: stage.name)
            _ = try recover(rebuild)
            _ = try run(
                "rebuild-create",
                fixture: rebuild,
                extra: ["--kill", "afterCommit"],
                expecting: .killed
            )
            let rebuilt = try recover(rebuild)
            XCTAssertEqual(
                rebuilt.deliveryPhases[Self.deliveryID],
                "supersededByDuplicate"
            )
            XCTAssertEqual(
                rebuilt.deliveryPhases["crash-create-delivery-copy"],
                stage.name == "create" ? "notDispatched" : "abandoned"
            )
            XCTAssertEqual(
                rebuilt.createDeliveryDispositions["crash-correlation"],
                "rebuildMayCreateDuplicateThread"
            )
            XCTAssertEqual(rebuilt.createDeliveryCount, stage.name == "create" ? 2 : 1)
            if stage.name == "create" {
                XCTAssertEqual(
                    rebuilt.createDeliveryPhases["crash-create-copy-intent"],
                    "ambiguous",
                    "a crash before duplicate-risk create dispatch remains user-terminable"
                )
            } else {
                XCTAssertEqual(rebuilt.conflictCount, 1)
                XCTAssertEqual(rebuilt.nonTerminalDeliveryGlobal, 0)
            }
        }
    }

    func testScopeRevokeCASCrashAndEvidenceIngressRemainOrthogonal() throws {
        let phases: [(name: String, setup: [String], before: String, after: String, evidence: Bool)] = [
            ("not", [], "notDispatched", "cancelledByDiscard", false),
            ("attempted", ["attempt"], "transportAttempted", "evidence", true),
            ("ambiguous", ["attempt", "ambiguous"], "ambiguous", "evidence", true),
            ("acknowledged", ["attempt", "ack"], "acknowledged", "terminalEvidence", true),
        ]

        for phase in phases {
            for boundary in ["beforeCommit", "afterCommit"] {
                let fixture = try makeFixture(label: "revoke-\(phase.name)-\(boundary)")
                try seedCommitted(fixture)
                for action in phase.setup {
                    _ = try run(action, fixture: fixture)
                }
                _ = try run(
                    "revoke-scope",
                    fixture: fixture,
                    extra: ["--kill", boundary],
                    expecting: .killed
                )
                var summary = try inspect(fixture)
                if boundary == "beforeCommit" {
                    XCTAssertEqual(summary.deliveryPhases[Self.deliveryID], phase.before)
                    XCTAssertEqual(summary.revokedThroughEpoch, 0)
                    continue
                }

                XCTAssertEqual(summary.deliveryPhases[Self.deliveryID], phase.after)
                XCTAssertEqual(summary.deliveryUserDispositions[Self.deliveryID], "scopeRevoked")
                XCTAssertEqual(summary.revokedThroughEpoch, 1)
                _ = try run("assert-revoked-gate", fixture: fixture)
                _ = try run(
                    "evidence",
                    fixture: fixture,
                    expecting: phase.evidence ? .success : .failed
                )
                summary = try recover(fixture, scope: "revoked")
                XCTAssertEqual(summary.deliveryPhases[Self.deliveryID], phase.after)
                if phase.evidence {
                    XCTAssertEqual(
                        summary.deliveryEvidence[Self.deliveryID],
                        "serverAcknowledged"
                    )
                }
                XCTAssertEqual(summary.revokedThroughEpoch, 1)
            }
        }
    }

    func testSyntheticRevocationFiveStepTransactionResumesAtEveryKillBoundary() throws {
        try assertSyntheticRecoveryBoundaries(mode: .kill)
    }

    func testSyntheticRevocationFiveStepTransactionResumesAtEveryENOSPCBoundary() throws {
        try assertSyntheticRecoveryBoundaries(mode: .enospc)
    }

    func testSyntheticRevocationFiveStepTransactionResumesAtEveryFsyncBoundary() throws {
        try assertSyntheticRecoveryBoundaries(mode: .fsync)
    }

    func testSealedWindowOperationManifestReservationAndScopeRelaunchMatrix() throws {
        let operationCases: [(GaryxOperationCapabilityState, Bool)] = [
            (.requested, false),
            (.preparing, false),
            (.uploading, false),
            (.uploading, true),
            (.completed, true),
            (.failedRetryable, true),
            (.failedTerminal, true),
            (.cancelled, false),
            (.superseded, false),
        ]
        let reservationOutcomes = ["nil", "none", "committed", "revoked"]
        let scopes = ["active", "suspended", "revoked"]

        for scope in scopes {
            for reservation in reservationOutcomes {
                for (state, attempted) in operationCases {
                    let context = "\(scope):\(reservation):\(state.rawValue):\(attempted)"
                    let fixture = try makeFixture(label: "manifest-\(context)")
                    var seedArguments = [
                        "--state", state.rawValue,
                        "--reservation", reservation,
                        "--kill", "afterCommit",
                    ]
                    if attempted { seedArguments.append("--attempted") }
                    _ = try run(
                        "seed-operation",
                        fixture: fixture,
                        extra: seedArguments,
                        expecting: .killed
                    )
                    let summary = try recover(fixture, scope: scope)
                    assertOperationRecovery(
                        summary,
                        state: state,
                        attempted: attempted,
                        scope: scope,
                        context: context
                    )
                    switch reservation {
                    case "nil":
                        XCTAssertTrue(summary.ledgerOutcomes.isEmpty, context)
                    case "none":
                        XCTAssertEqual(summary.ledgerOutcomes[Self.reservationID], "revoked", context)
                        XCTAssertGreaterThan(summary.targetGenerations[Self.reservationID] ?? 0, 11, context)
                    default:
                        if summary.ledgerOutcomes[Self.reservationID] == nil {
                            let operationID = "manifest-\(state.rawValue)"
                            XCTAssertNil(summary.operationStates[operationID], context)
                            XCTAssertEqual(summary.manifestCount, 0, context)
                            XCTAssertEqual(summary.replacementCount, 0, context)
                            XCTAssertEqual(summary.producerDrainedCount, 0, context)
                            XCTAssertEqual(summary.recoveredCloseCount, 0, context)
                            XCTAssertEqual(summary.barrierCount, 0, context)
                            XCTAssertTrue(summary.deliveryPhases.isEmpty, context)
                        } else {
                            XCTAssertEqual(
                                summary.ledgerOutcomes[Self.reservationID],
                                reservation,
                                context
                            )
                        }
                    }
                }
            }
        }
    }

    func testSharedEntryMultipleOperationsResumeAcrossScopeAndKillBoundaries() throws {
        for scope in ["active", "suspended", "revoked"] {
            for boundary in ["beforeCommit", "afterCommit"] {
                let context = "\(scope):\(boundary)"
                let fixture = try makeFixture(label: "multi-operation-\(context)")
                _ = try run(
                    "seed-multi-operation",
                    fixture: fixture,
                    extra: ["--kill", "afterCommit"],
                    expecting: .killed
                )
                _ = try run(
                    "recover",
                    fixture: fixture,
                    extra: [
                        "--scope", scope,
                        "--kill", boundary,
                    ],
                    expecting: .killed
                )
                var summary = try recover(fixture, scope: scope)
                XCTAssertTrue(summary.operationStates.isEmpty, context)
                XCTAssertEqual(summary.entryOperationMembershipCount, 0, context)
                XCTAssertEqual(summary.manifestCount, 0, context)
                XCTAssertEqual(summary.feedbackCount, 0, context)
                XCTAssertEqual(summary.reservedBytes, 0, context)
                XCTAssertEqual(summary.stagedOwnerCount, 0, context)
                XCTAssertEqual(summary.pendingCleanupCount, 0, context)
                XCTAssertEqual(summary.currentText, scope == "revoked" ? nil : "multi-operation", context)

                summary = try recover(fixture, scope: scope)
                XCTAssertTrue(summary.operationStates.isEmpty, "second relaunch \(context)")
                XCTAssertEqual(summary.entryOperationMembershipCount, 0, "second relaunch \(context)")
            }
        }
    }

    func testOwnerlessManifestRecoveryResumesAcrossKillBoundaries() throws {
        for boundary in ["beforeCommit", "afterCommit"] {
            let fixture = try makeFixture(label: "ownerless-manifest-\(boundary)")
            _ = try run(
                "seed-ownerless-manifest",
                fixture: fixture,
                extra: ["--kill", "afterCommit"],
                expecting: .killed
            )
            _ = try run(
                "recover",
                fixture: fixture,
                extra: ["--kill", boundary],
                expecting: .killed
            )
            var summary = try recover(fixture)
            XCTAssertEqual(summary.manifestCount, 0, boundary)
            XCTAssertEqual(summary.entryOperationMembershipCount, 0, boundary)
            summary = try recover(fixture)
            XCTAssertEqual(summary.manifestCount, 0, "second relaunch \(boundary)")
            XCTAssertEqual(summary.entryOperationMembershipCount, 0, "second relaunch \(boundary)")
        }
    }

    func testReplacementRecoveryPhaseScopeAndKillBoundaryMatrixConvergesTwice() throws {
        let phases = ["pendingReplacement", "aborted", "committed"]
        for phase in phases {
            for scope in ["active", "suspended", "revoked"] {
                for boundary in ["beforeCommit", "afterCommit"] {
                    let context = "\(phase):\(scope):\(boundary)"
                    let fixture = try makeFixture(label: "replacement-\(context)")
                    var seedArguments = [
                        "--phase", phase,
                        "--kill", "afterCommit",
                    ]
                    if scope == "revoked" { seedArguments.append("--families") }
                    _ = try run(
                        "seed-replacement",
                        fixture: fixture,
                        extra: seedArguments,
                        expecting: .killed
                    )
                    _ = try run(
                        "recover",
                        fixture: fixture,
                        extra: [
                            "--scope", scope,
                            "--kill", boundary,
                        ],
                        expecting: .killed
                    )

                    var summary = try recover(fixture, scope: scope)
                    try assertReplacementRecovery(
                        summary,
                        fixture: fixture,
                        phase: phase,
                        scope: scope,
                        context: context
                    )
                    summary = try recover(fixture, scope: scope)
                    try assertReplacementRecovery(
                        summary,
                        fixture: fixture,
                        phase: phase,
                        scope: scope,
                        context: "second relaunch \(context)"
                    )
                }
            }
        }
    }

    func testRevokedEntryReplacementFeedbackAndLineageFamiliesConvergeAcrossKillBoundaries() throws {
        for boundary in ["beforeCommit", "afterCommit"] {
            let fixture = try makeFixture(label: "replacement-families-\(boundary)")
            _ = try run(
                "seed-replacement",
                fixture: fixture,
                extra: [
                    "--phase", "pendingReplacement",
                    "--families",
                    "--erase-entry",
                    "--kill", "afterCommit",
                ],
                expecting: .killed
            )
            _ = try run(
                "recover",
                fixture: fixture,
                extra: [
                    "--scope", "revoked",
                    "--kill", boundary,
                ],
                expecting: .killed
            )
            var summary = try recover(fixture, scope: "revoked")
            assertRevokedReplacementFamiliesCleared(summary, context: boundary)
            summary = try recover(fixture, scope: "revoked")
            assertRevokedReplacementFamiliesCleared(
                summary,
                context: "second relaunch \(boundary)"
            )
        }
    }

    func testEveryOperationStateDestinationDiscardCrashRelaunchReleasesAllResources() throws {
        for state in GaryxOperationCapabilityState.allCases {
            let fixture = try makeFixture(label: "discard-operation-\(state.rawValue)")
            _ = try run(
                "seed-discard-operation",
                fixture: fixture,
                extra: [
                    "--state", state.rawValue,
                    "--kill", "afterCommit",
                ],
                expecting: .killed
            )
            _ = try run(
                "recover",
                fixture: fixture,
                extra: ["--kill", "afterCommit"],
                expecting: .killed
            )
            let summary = try recover(fixture)
            assertDiscardedAndResourceFree(summary, context: state.rawValue)
            XCTAssertTrue(summary.deliveryPhases.isEmpty, state.rawValue)
        }
    }

    func testCrossPromotionSessionDiscardResumesAfterEveryCommittedStep() throws {
        for scope in ["active", "revoked"] {
            for commitOccurrence in 1...5 {
                let context = "\(scope):\(commitOccurrence)"
                let fixture = try makeFixture(label: "discard-sessions-\(context)")
                _ = try run(
                    "seed-discard-sessions",
                    fixture: fixture,
                    extra: ["--kill", "afterCommit"],
                    expecting: .killed
                )
                _ = try run(
                    "recover",
                    fixture: fixture,
                    extra: [
                        "--scope", scope,
                        "--kill", "afterCommit",
                        "--kill-occurrence", String(commitOccurrence),
                    ],
                    expecting: .killed
                )
                if commitOccurrence == 1 {
                    let persisted = try inspect(fixture)
                    XCTAssertEqual(persisted.discardTombstoneCount, 2, context)
                    XCTAssertEqual(persisted.discardCount, 1, context)
                }
                let summary = try recover(fixture, scope: scope)
                assertDiscardedAndResourceFree(summary, context: context)
            }
        }
    }

    func testMixedDeliverySealedReservationAndSessionsResumeAfterEveryDiscardStep() throws {
        for scope in ["active", "revoked"] {
            for commitOccurrence in 1...8 {
                let context = "\(scope):\(commitOccurrence)"
                let fixture = try makeFixture(label: "discard-mixed-\(context)")
                _ = try run(
                    "seed-discard-mixed",
                    fixture: fixture,
                    extra: ["--kill", "afterCommit"],
                    expecting: .killed
                )
                _ = try run(
                    "ack-delivery",
                    fixture: fixture,
                    extra: ["--delivery", "mixed-attempted"]
                )
                let acknowledged = try inspect(fixture)
                XCTAssertEqual(acknowledged.deliveryPhases["mixed-attempted"], "acknowledged", context)
                XCTAssertEqual(
                    acknowledged.deliveryEvidence["mixed-attempted"],
                    "serverAcknowledged",
                    context
                )
                _ = try run(
                    "recover",
                    fixture: fixture,
                    extra: [
                        "--scope", scope,
                        "--kill", "afterCommit",
                        "--kill-occurrence", String(commitOccurrence),
                    ],
                    expecting: .killed
                )
                if commitOccurrence == 4 {
                    XCTAssertEqual(try inspect(fixture).discardTombstoneCount, 2, context)
                }
                let summary = try recover(fixture, scope: scope)
                assertDiscardedAndResourceFree(summary, context: context)
                XCTAssertEqual(
                    summary.deliveryPhases["mixed-not-dispatched"],
                    "cancelledByDiscard",
                    context
                )
                XCTAssertEqual(
                    summary.deliveryPhases["mixed-attempted"],
                    "terminalEvidence",
                    context
                )
                XCTAssertEqual(
                    summary.deliveryEvidence["mixed-attempted"],
                    "serverAcknowledged",
                    context
                )
                XCTAssertEqual(
                    summary.deliveryUserDispositions["mixed-attempted"],
                    "none",
                    context
                )
                XCTAssertEqual(
                    summary.deliveryPhases["mixed-acknowledged"],
                    "terminalEvidence",
                    context
                )
                XCTAssertNil(
                    summary.ledgerOutcomes[Self.reservationID],
                    "descendant-free terminal reservation should be compacted: \(context)"
                )
            }
        }
    }

    func testProtectedStagingKillENOSPCAndFsyncBoundariesLeaveNoOrphans() throws {
        let boundaries = [
            "quotaReserved",
            "beforeCopy",
            "copiedToTemporaryFile",
            "beforeFileSync",
            "fileSynced",
            "atomicallyRenamed",
            "directorySynced",
        ]
        for mode in BoundaryMode.allCases {
            for boundary in boundaries {
                let context = "\(mode.rawValue):\(boundary)"
                let fixture = try makeFixture(label: "staging-\(context)")
                _ = try run(
                    "stage",
                    fixture: fixture,
                    extra: ["--source", fixture.sourceURL.path]
                        + mode.arguments(boundary: "staging:\(boundary)", occurrence: 1),
                    expecting: mode.termination
                )
                let summary = try recover(fixture)
                XCTAssertTrue(summary.operationStates.isEmpty, context)
                XCTAssertEqual(summary.manifestCount, 0, context)
                XCTAssertEqual(summary.stagedOwnerCount, 0, context)
                XCTAssertEqual(summary.reservedBytes, 0, context)
                XCTAssertEqual(summary.pendingCleanupCount, 0, context)
                XCTAssertTrue(try stagedFiles(fixture).isEmpty, context)
            }
        }
    }

    func testFiveHundredCrossPromotionSessionDiscardsRemainBoundedAcrossRelaunch() throws {
        let fixture = try makeFixture(label: "discard-churn")
        var summary = try decodeSummary(
            run(
                "churn-discard",
                fixture: fixture,
                extra: ["--count", "500"]
            ).standardOutput
        )
        assertDiscardedAndResourceFree(summary, context: "churn")
        summary = try inspect(fixture)
        assertDiscardedAndResourceFree(summary, context: "churn relaunch")
    }

    func testFullCorrelationPoolCannotBrickProcessRelaunchDiscardRecovery() throws {
        let fixture = try makeFixture(label: "correlation-capacity-discard")
        _ = try run(
            "seed-correlation-capacity-discard",
            fixture: fixture,
            extra: ["--kill", "afterCommit"],
            expecting: .killed
        )

        for relaunch in 1...2 {
            let result = try run("recover-correlation-capacity", fixture: fixture)
            let summary = try decodeCorrelationCapacitySummary(result.standardOutput)
            XCTAssertEqual(summary.correlationTombstoneCount, 4_096, "relaunch \(relaunch)")
            XCTAssertFalse(summary.oldestHistoricalPresent, "relaunch \(relaunch)")
            XCTAssertEqual(summary.targetPhase, "evidence", "relaunch \(relaunch)")
            XCTAssertFalse(summary.entryPresent, "relaunch \(relaunch)")
            XCTAssertEqual(summary.discardCount, 0, "relaunch \(relaunch)")
        }
    }

    // MARK: - Assertions

    private func assertSendRecovery(
        _ summary: HarnessSummary,
        committed: Bool,
        context: String
    ) {
        if committed {
            XCTAssertEqual(summary.currentText, "U", context)
            XCTAssertEqual(summary.currentGeneration, 11, context)
            XCTAssertEqual(summary.ledgerOutcomes[Self.reservationID], "committed", context)
            assertUndispatchedRecovered(summary, context: context)
        } else {
            XCTAssertEqual(summary.currentText, "TU", context)
            XCTAssertGreaterThan(summary.currentGeneration ?? 0, 11, context)
            XCTAssertEqual(summary.ledgerOutcomes[Self.reservationID], "revoked", context)
            XCTAssertTrue(summary.deliveryPhases.isEmpty, context)
        }
    }

    private func assertUndispatchedRecovered(
        _ summary: HarnessSummary,
        context: String
    ) {
        XCTAssertEqual(summary.deliveryPhases[Self.deliveryID], "abandoned", context)
        XCTAssertEqual(
            summary.deliveryUserDispositions[Self.deliveryID],
            "restoredToDraft",
            context
        )
        XCTAssertEqual(summary.deliveryDispositions[Self.deliveryID], "terminal", context)
        XCTAssertEqual(summary.recoveredEntryTexts, ["T"], context)
        XCTAssertEqual(summary.entryCount, 2, context)
        XCTAssertEqual(summary.conflictCount, 1, context)
        XCTAssertEqual(summary.hostDeliveryReferenceCount, 0, context)
        XCTAssertEqual(summary.nonTerminalDeliveryGlobal, 0, context)
        XCTAssertEqual(summary.nonTerminalDeliveryForScope, 0, context)
        XCTAssertEqual(summary.durableNoticeKinds, ["payloadConflict"], context)
    }

    private func assertSyntheticRevocation(_ summary: HarnessSummary, context: String) {
        XCTAssertEqual(summary.currentText, "TU", context)
        XCTAssertGreaterThan(summary.currentGeneration ?? 0, 11, context)
        XCTAssertEqual(summary.ledgerOutcomes[Self.reservationID], "revoked", context)
        XCTAssertEqual(
            summary.targetGenerations[Self.reservationID],
            summary.currentGeneration,
            context
        )
        XCTAssertEqual(summary.recoveredCloseCount, 1, context)
        XCTAssertEqual(summary.closePublicationTotal, 1, context)
    }

    private func assertSyntheticRecoveryBoundaries(mode: BoundaryMode) throws {
        for boundary in syntheticRecoveryBoundaries {
            let fixture = try makeFixture(
                label: "synthetic-\(mode.rawValue)-\(boundary.name)"
            )
            _ = try run("seed-unsettled", fixture: fixture)
            _ = try run(
                "recover",
                fixture: fixture,
                extra: mode.arguments(
                    boundary: boundary.name,
                    occurrence: boundary.occurrence
                ),
                expecting: mode.termination
            )
            var summary = try recover(fixture)
            assertSyntheticRevocation(summary, context: "\(mode.rawValue):\(boundary.name)")
            summary = try recover(fixture)
            assertSyntheticRevocation(summary, context: "second:\(mode.rawValue):\(boundary.name)")
        }
    }

    private func assertOperationRecovery(
        _ summary: HarnessSummary,
        state: GaryxOperationCapabilityState,
        attempted: Bool,
        scope: String,
        context: String
    ) {
        let operationID = "manifest-\(state.rawValue)"
        if scope == "revoked" {
            XCTAssertNil(summary.operationStates[operationID], context)
            if state == .cancelled || state == .failedRetryable {
                XCTAssertNotNil(summary.currentText, context)
            }
            XCTAssertEqual(summary.entryOperationMembershipCount, 0, context)
            XCTAssertEqual(summary.manifestCount, 0, context)
            XCTAssertEqual(summary.feedbackCount, 0, context)
            XCTAssertEqual(summary.reservedBytes, 0, context)
            XCTAssertEqual(summary.stagedOwnerCount, 0, context)
            return
        }
        switch (state, attempted) {
        case (.uploading, false):
            XCTAssertEqual(summary.operationStates[operationID], "uploading", context)
            XCTAssertEqual(summary.manifestCount, 1, context)
            XCTAssertEqual(summary.feedbackCount, 0, context)
            XCTAssertEqual(summary.reservedBytes, 31, context)
            XCTAssertEqual(summary.stagedOwnerCount, 1, context)
        case (.uploading, true):
            XCTAssertEqual(summary.operationStates[operationID], "failedRetryable", context)
            XCTAssertEqual(summary.manifestCount, 1, context)
            XCTAssertEqual(summary.feedbackCount, 1, context)
            XCTAssertEqual(summary.reservedBytes, 31, context)
            XCTAssertEqual(summary.stagedOwnerCount, 1, context)
        case (.failedRetryable, _):
            XCTAssertEqual(summary.operationStates[operationID], "failedRetryable", context)
            XCTAssertEqual(summary.manifestCount, 1, context)
            XCTAssertEqual(summary.reservedBytes, 31, context)
            XCTAssertEqual(summary.stagedOwnerCount, 1, context)
        case (.failedTerminal, _):
            XCTAssertEqual(summary.operationStates[operationID], "failedTerminal", context)
            XCTAssertEqual(summary.manifestCount, 0, context)
            XCTAssertEqual(summary.feedbackCount, 1, context)
            XCTAssertEqual(summary.reservedBytes, 0, context)
            XCTAssertEqual(summary.stagedOwnerCount, 0, context)
        default:
            XCTAssertNil(summary.operationStates[operationID], context)
            XCTAssertEqual(summary.manifestCount, 0, context)
            XCTAssertEqual(summary.reservedBytes, 0, context)
            XCTAssertEqual(summary.stagedOwnerCount, 0, context)
        }
    }

    private func assertDiscardedAndResourceFree(
        _ summary: HarnessSummary,
        context: String
    ) {
        XCTAssertNil(summary.currentText, context)
        XCTAssertEqual(summary.aliasCount, 0, context)
        XCTAssertTrue(summary.operationStates.isEmpty, context)
        XCTAssertEqual(summary.manifestCount, 0, context)
        XCTAssertEqual(summary.replacementCount, 0, context)
        XCTAssertEqual(summary.feedbackCount, 0, context)
        XCTAssertEqual(summary.producerDrainedCount, 0, context)
        XCTAssertLessThanOrEqual(summary.ledgerCount, summary.deliveryPhases.count, context)
        XCTAssertLessThanOrEqual(
            summary.claimedGenerationCount,
            Int(GaryxDurableHiLoAllocator.maximumBlockSize),
            context
        )
        XCTAssertEqual(summary.createDeliveryCount, 0, context)
        XCTAssertEqual(summary.recoveredCloseCount, 0, context)
        XCTAssertEqual(summary.closePublicationTotal, 0, context)
        XCTAssertEqual(summary.barrierCount, 0, context)
        XCTAssertEqual(summary.nonIdleBarrierCount, 0, context)
        XCTAssertEqual(summary.barrierPayloadFieldCount, 0, context)
        XCTAssertEqual(summary.discardCount, 0, context)
        XCTAssertEqual(summary.discardTombstoneCount, 0, context)
        XCTAssertEqual(summary.reservedBytes, 0, context)
        XCTAssertEqual(summary.stagedOwnerCount, 0, context)
        XCTAssertEqual(summary.pendingCleanupCount, 0, context)
    }

    private func assertReplacementRecovery(
        _ summary: HarnessSummary,
        fixture: Fixture,
        phase: String,
        scope: String,
        context: String
    ) throws {
        XCTAssertEqual(summary.currentText, "replacement-sibling-text", context)
        XCTAssertEqual(summary.entryAttachmentCount, 1, context)
        XCTAssertEqual(summary.feedbackCount, 0, context)
        XCTAssertEqual(summary.attachmentLineageCount, 0, context)
        if phase == "committed", scope != "revoked" {
            XCTAssertEqual(
                summary.operationStates,
                ["replacement-successor": "failedRetryable"],
                context
            )
            XCTAssertEqual(summary.entryOperationMembershipCount, 1, context)
            XCTAssertEqual(summary.manifestCount, 1, context)
            XCTAssertEqual(summary.replacementCount, 1, context)
            XCTAssertEqual(summary.reservedBytes, 43, context)
            XCTAssertEqual(summary.stagedOwnerCount, 1, context)
            XCTAssertEqual(summary.pendingCleanupCount, 0, context)
            XCTAssertEqual(try stagedFiles(fixture).count, 1, context)
        } else {
            XCTAssertTrue(summary.operationStates.isEmpty, context)
            XCTAssertEqual(summary.entryOperationMembershipCount, 0, context)
            XCTAssertEqual(summary.manifestCount, 0, context)
            XCTAssertEqual(summary.replacementCount, 0, context)
            XCTAssertEqual(summary.reservedBytes, 0, context)
            XCTAssertEqual(summary.stagedOwnerCount, 0, context)
            XCTAssertEqual(summary.pendingCleanupCount, 0, context)
            XCTAssertTrue(try stagedFiles(fixture).isEmpty, context)
        }
    }

    private func assertRevokedReplacementFamiliesCleared(
        _ summary: HarnessSummary,
        context: String
    ) {
        XCTAssertNil(summary.currentText, context)
        XCTAssertEqual(summary.entryAttachmentCount, 0, context)
        XCTAssertTrue(summary.operationStates.isEmpty, context)
        XCTAssertEqual(summary.entryOperationMembershipCount, 0, context)
        XCTAssertEqual(summary.manifestCount, 0, context)
        XCTAssertEqual(summary.replacementCount, 0, context)
        XCTAssertEqual(summary.feedbackCount, 0, context)
        XCTAssertEqual(summary.attachmentLineageCount, 0, context)
        XCTAssertEqual(summary.reservedBytes, 0, context)
        XCTAssertEqual(summary.stagedOwnerCount, 0, context)
        XCTAssertEqual(summary.pendingCleanupCount, 0, context)
    }

    // MARK: - Process harness

    private var commitSendBoundaries: [(name: String, committed: Bool)] {
        [
            ("transactionBegan", false),
            ("mutation:0", false),
            ("mutation:1", false),
            ("mutation:2", false),
            ("mutation:3", false),
        ] + GaryxComposerDurabilityRecordFamily.allCases.map {
            ("family:\($0.rawValue)", false)
        } + [
            ("metadata", false),
            ("beforeCommit", false),
            ("afterCommit", true),
        ]
    }

    private var undispatchedRecoveryBoundaries: [(name: String, occurrence: Int)] {
        [
            ("transactionBegan", 2),
            ("mutation:0", 2),
            ("mutation:1", 1),
            ("mutation:2", 1),
            ("mutation:3", 1),
            ("mutation:4", 1),
        ] + GaryxComposerDurabilityRecordFamily.allCases.map {
            ("family:\($0.rawValue)", 2)
        } + [
            ("metadata", 2),
            ("beforeCommit", 2),
            ("afterCommit", 2),
        ]
    }

    private var syntheticRecoveryBoundaries: [(name: String, occurrence: Int)] {
        [
            ("transactionBegan", 2),
            ("mutation:0", 2),
            ("mutation:1", 1),
            ("mutation:2", 1),
            ("mutation:3", 1),
            ("mutation:4", 1),
            ("mutation:5", 1),
            ("mutation:6", 1),
        ] + GaryxComposerDurabilityRecordFamily.allCases.map {
            ("family:\($0.rawValue)", 2)
        } + [
            ("metadata", 2),
            ("beforeCommit", 2),
            ("afterCommit", 2),
        ]
    }

    private func seedSealed(_ fixture: Fixture) throws {
        _ = try run("seed-sealed", fixture: fixture)
    }

    private func seedCommitted(_ fixture: Fixture) throws {
        try seedSealed(fixture)
        _ = try run("commit-send", fixture: fixture)
    }

    private func seedAmbiguousDelivery(_ fixture: Fixture) throws {
        try seedCommitted(fixture)
        _ = try run("attempt", fixture: fixture)
        _ = try run("ambiguous", fixture: fixture)
    }

    private func seedCreateDeath(_ fixture: Fixture, stage: String) throws {
        try seedCommitted(fixture)
        _ = try run(
            "create-server-commit-then-kill",
            fixture: fixture,
            extra: ["--stage", stage],
            expecting: .killed
        )
    }

    private func recover(_ fixture: Fixture, scope: String = "active") throws -> HarnessSummary {
        try decodeSummary(
            run(
                "recover",
                fixture: fixture,
                extra: ["--scope", scope]
            ).standardOutput
        )
    }

    private func inspect(_ fixture: Fixture) throws -> HarnessSummary {
        try decodeSummary(run("inspect", fixture: fixture).standardOutput)
    }

    private func run(
        _ action: String,
        fixture: Fixture,
        extra: [String] = [],
        expecting termination: ExpectedTermination = .success
    ) throws -> ProcessResult {
        let process = Process()
        process.executableURL = try harnessExecutableURL()
        process.arguments = [
            "--db", fixture.databaseURL.path,
            "--app-support", fixture.applicationSupportURL.path,
            "--action", action,
        ] + extra
        let standardOutput = Pipe()
        let standardError = Pipe()
        process.standardOutput = standardOutput
        process.standardError = standardError
        try process.run()
        process.waitUntilExit()
        let result = ProcessResult(
            terminationReason: process.terminationReason,
            terminationStatus: process.terminationStatus,
            standardOutput: standardOutput.fileHandleForReading.readDataToEndOfFile(),
            standardError: standardError.fileHandleForReading.readDataToEndOfFile()
        )
        switch termination {
        case .success:
            XCTAssertEqual(result.terminationReason, .exit, result.diagnostic)
            XCTAssertEqual(result.terminationStatus, 0, result.diagnostic)
        case .failed:
            XCTAssertEqual(result.terminationReason, .exit, result.diagnostic)
            XCTAssertEqual(result.terminationStatus, 1, result.diagnostic)
        case .killed:
            XCTAssertEqual(result.terminationReason, .uncaughtSignal, result.diagnostic)
            XCTAssertEqual(result.terminationStatus, SIGKILL, result.diagnostic)
        }
        return result
    }

    private func harnessExecutableURL() throws -> URL {
        let testBundle = Bundle(for: Self.self).bundleURL
        let candidates = [
            testBundle.deletingLastPathComponent()
                .appendingPathComponent("GaryxComposerDurabilityCrashHarness"),
            testBundle.appendingPathComponent("Contents/MacOS/GaryxComposerDurabilityCrashHarness"),
        ]
        guard let executable = candidates.first(where: {
            FileManager.default.isExecutableFile(atPath: $0.path)
        }) else {
            throw HarnessTestError.executableMissing(candidates.map(\.path))
        }
        return executable
    }

    private func makeFixture(label: String) throws -> Fixture {
        let safeLabel = label.replacingOccurrences(of: "/", with: "-")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(
            "GaryxComposerDurability-\(safeLabel)-\(UUID().uuidString)",
            isDirectory: true
        )
        let applicationSupport = root.appendingPathComponent("ApplicationSupport", isDirectory: true)
        try FileManager.default.createDirectory(
            at: applicationSupport,
            withIntermediateDirectories: true
        )
        let sourceURL = root.appendingPathComponent("source.bin")
        try Data(repeating: 0xA4, count: 4_096).write(to: sourceURL, options: .atomic)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return Fixture(
            rootURL: root,
            databaseURL: root.appendingPathComponent("composer.sqlite"),
            applicationSupportURL: applicationSupport,
            sourceURL: sourceURL
        )
    }

    private func stagedFiles(_ fixture: Fixture) throws -> [URL] {
        let root = fixture.applicationSupportURL
            .appendingPathComponent("Garyx", isDirectory: true)
            .appendingPathComponent("ComposerPayload", isDirectory: true)
        guard FileManager.default.fileExists(atPath: root.path) else { return [] }
        return try FileManager.default.contentsOfDirectory(
            at: root,
            includingPropertiesForKeys: nil
        )
    }

    private func decodeSummary(_ data: Data) throws -> HarnessSummary {
        do {
            return try JSONDecoder().decode(HarnessSummary.self, from: data)
        } catch {
            throw HarnessTestError.invalidSummary(
                String(data: data, encoding: .utf8) ?? "<non-UTF8>",
                String(describing: error)
            )
        }
    }

    private func decodeCorrelationCapacitySummary(
        _ data: Data
    ) throws -> CorrelationCapacitySummary {
        do {
            return try JSONDecoder().decode(CorrelationCapacitySummary.self, from: data)
        } catch {
            throw HarnessTestError.invalidSummary(
                String(data: data, encoding: .utf8) ?? "<non-UTF8>",
                String(describing: error)
            )
        }
    }
}

private struct Fixture {
    let rootURL: URL
    let databaseURL: URL
    let applicationSupportURL: URL
    let sourceURL: URL
}

private struct ProcessResult {
    let terminationReason: Process.TerminationReason
    let terminationStatus: Int32
    let standardOutput: Data
    let standardError: Data

    var diagnostic: String {
        "status=\(terminationStatus) stdout=\(String(data: standardOutput, encoding: .utf8) ?? "") stderr=\(String(data: standardError, encoding: .utf8) ?? "")"
    }
}

private struct HarnessSummary: Decodable {
    let revision: UInt64
    let currentText: String?
    let currentGeneration: UInt64?
    let entryCount: Int
    let recoveredEntryTexts: [String]
    let hostDeliveryReferenceCount: Int
    let aliasCount: Int
    let deliveryPhases: [String: String]
    let deliveryEvidence: [String: String]
    let deliveryUserDispositions: [String: String]
    let deliveryDispositions: [String: String]
    let nonTerminalDeliveryGlobal: Int
    let nonTerminalDeliveryForScope: Int
    let durableNoticeKinds: [String]
    let ledgerOutcomes: [String: String]
    let targetGenerations: [String: UInt64]
    let operationStates: [String: String]
    let entryOperationMembershipCount: Int
    let entryAttachmentCount: Int
    let manifestCount: Int
    let replacementCount: Int
    let feedbackCount: Int
    let attachmentLineageCount: Int
    let producerDrainedCount: Int
    let ledgerCount: Int
    let claimedGenerationCount: Int
    let createDeliveryCount: Int
    let createDeliveryPhases: [String: String]
    let createDeliveryAmbiguousAfter: [String: String]
    let createDeliveryDispositions: [String: String]
    let conflictCount: Int
    let revokedThroughEpoch: UInt64
    let barrierCount: Int
    let nonIdleBarrierCount: Int
    let barrierPayloadFieldCount: Int
    let discardCount: Int
    let discardTombstoneCount: Int
    let reservedBytes: Int
    let stagedOwnerCount: Int
    let pendingCleanupCount: Int
    let recoveredCloseCount: Int
    let closePublicationTotal: Int
}

private struct CorrelationCapacitySummary: Decodable {
    let correlationTombstoneCount: Int
    let oldestHistoricalPresent: Bool
    let targetPhase: String?
    let entryPresent: Bool
    let discardCount: Int
}

private struct TrueSQLiteFullSummary: Decodable {
    let sqliteCode: Int32
    let maximumPageCount: Int64
    let revision: UInt64
    let entryPresent: Bool
}

private struct TransportCrashCase {
    let setupActions: [String]
    let action: String
    let extra: [String]
    let expectedPhase: String
    let expectedDisposition: String
}

private enum ExpectedTermination {
    case success
    case failed
    case killed
}

private enum FailureKind: String {
    case enospc
    case fsync
}

private enum BoundaryMode: String, CaseIterable {
    case kill
    case enospc
    case fsync

    var termination: ExpectedTermination {
        self == .kill ? .killed : .failed
    }

    func arguments(boundary: String, occurrence: Int) -> [String] {
        switch self {
        case .kill:
            [
                "--kill", boundary,
                "--kill-occurrence", String(occurrence),
            ]
        case .enospc, .fsync:
            [
                "--fail", boundary,
                "--failure", rawValue,
                "--fail-occurrence", String(occurrence),
            ]
        }
    }
}

private enum HarnessTestError: Error {
    case executableMissing([String])
    case invalidSummary(String, String)
}
#endif
