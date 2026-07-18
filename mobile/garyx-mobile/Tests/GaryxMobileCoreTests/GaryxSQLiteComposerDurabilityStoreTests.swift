import Foundation
import SQLite3
import XCTest
@testable import GaryxMobileCore

final class GaryxSQLiteComposerDurabilityStoreTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "sqlite-gateway", epoch: 1)
    private let entryID = GaryxComposerPayloadEntryID(rawValue: "sqlite-entry")
    private let reservationID = GaryxSendReservationID(rawValue: 7)

    func testSchemaKeepsEveryRecordFamilyInOneSQLiteDatabase() async throws {
        let fixture = try makeDatabaseFixture()
        _ = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)

        var database: OpaquePointer?
        XCTAssertEqual(sqlite3_open_v2(fixture.databaseURL.path, &database, SQLITE_OPEN_READONLY, nil), SQLITE_OK)
        let handle = try XCTUnwrap(database)
        defer { sqlite3_close_v2(handle) }

        let tables = try queryStrings(
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
            database: handle
        )
        XCTAssertEqual(
            Set(GaryxSQLiteComposerDurabilityStore.schemaTableNames).subtracting(tables),
            []
        )
        let databasePaths = try queryStrings("PRAGMA database_list", column: 2, database: handle)
            .map { URL(fileURLWithPath: $0).resolvingSymlinksInPath().path }
        XCTAssertEqual(databasePaths, [fixture.databaseURL.resolvingSymlinksInPath().path])
        XCTAssertEqual(
            try queryStrings("PRAGMA journal_mode", database: handle).map { $0.lowercased() },
            ["wal"]
        )

        for table in GaryxComposerDurabilityRecordFamily.allCases.map(\.rawValue) {
            XCTAssertEqual(try scalarInt("SELECT COUNT(*) FROM \(table)", database: handle), 1)
        }
    }

    func testCommitSendPublishesOutboxGenerationAndPayloadClearAcrossRelaunch() async throws {
        let fixture = try makeDatabaseFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let send = try makeCommitSend()

        let committed = try await store.commitSend(send)
        XCTAssertEqual(committed.revision, 1)
        XCTAssertEqual(committed.payloadStore.entry(entryID, scope: scope)?.currentGeneration, 11)
        XCTAssertEqual(committed.payloadStore.entry(entryID, scope: scope)?.currentText, "follow-up")
        XCTAssertNil(committed.payloadStore.entry(entryID, scope: scope)?.textByGeneration[10])
        XCTAssertEqual(committed.deliveries[send.delivery.id], send.delivery)

        let relaunched = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let restored = try await relaunched.load()
        XCTAssertEqual(restored, committed)
        XCTAssertEqual(restored.ledgers[send.ledger.key]?.terminalOutcome, .committed)
        XCTAssertEqual(restored.barriers[entryID]?.phase, .durableCommitted)
    }

    func testEveryPreCommitStorageBoundaryRollsBackWholeCommitSend() async throws {
        let rollbackBoundaries: [GaryxComposerDurabilityStorageBoundary] = [
            .transactionBegan,
            .mutationApplied(0),
            .mutationApplied(1),
            .mutationApplied(2),
            .mutationApplied(3),
        ] + GaryxComposerDurabilityRecordFamily.allCases.map {
            .familyPersisted($0)
        } + [
            .metadataPersisted,
            .beforeCommit,
        ]

        for boundary in rollbackBoundaries {
            let fixture = try makeDatabaseFixture()
            let store = try GaryxSQLiteComposerDurabilityStore(
                databaseURL: fixture.databaseURL,
                boundaryHook: { observed in
                    if observed == boundary {
                        throw GaryxSQLiteComposerDurabilityError.injectedNoSpace(observed)
                    }
                }
            )
            do {
                _ = try await store.commitSend(makeCommitSend())
                XCTFail("expected rollback at \(boundary)")
            } catch {
                XCTAssertEqual(
                    error as? GaryxSQLiteComposerDurabilityError,
                    .injectedNoSpace(boundary)
                )
            }

            let relaunched = try GaryxSQLiteComposerDurabilityStore(
                databaseURL: fixture.databaseURL
            )
            let restored = try await relaunched.load()
            XCTAssertEqual(restored, GaryxComposerDurabilitySnapshot())
        }
    }

    func testAfterCommitFailureReportsAmbiguousButRelaunchSeesWholeSend() async throws {
        let fixture = try makeDatabaseFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL,
            boundaryHook: { boundary in
                if boundary == .afterCommit {
                    throw GaryxSQLiteComposerDurabilityError.injectedFsyncFailure(boundary)
                }
            }
        )
        let send = try makeCommitSend()
        do {
            _ = try await store.commitSend(send)
            XCTFail("after-commit acknowledgement should be lost")
        } catch {
            XCTAssertEqual(
                error as? GaryxSQLiteComposerDurabilityError,
                .injectedFsyncFailure(.afterCommit)
            )
        }

        let relaunched = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let restored = try await relaunched.load()
        XCTAssertEqual(restored.revision, 1)
        XCTAssertEqual(restored.deliveries[send.delivery.id], send.delivery)
        XCTAssertEqual(restored.payloadStore.entry(entryID, scope: scope)?.currentText, "follow-up")
    }

    func testFsyncFailureBeforeCommitLeavesSealedPayloadUnpublished() async throws {
        let fixture = try makeDatabaseFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL,
            boundaryHook: { boundary in
                if boundary == .beforeCommit {
                    throw GaryxSQLiteComposerDurabilityError.injectedFsyncFailure(boundary)
                }
            }
        )
        do {
            _ = try await store.commitSend(makeCommitSend())
            XCTFail("expected fsync failpoint")
        } catch {
            XCTAssertEqual(
                error as? GaryxSQLiteComposerDurabilityError,
                .injectedFsyncFailure(.beforeCommit)
            )
        }
        let relaunched = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let restored = try await relaunched.load()
        XCTAssertEqual(restored, GaryxComposerDurabilitySnapshot())
    }

    func testHiLoAllocatorPreRaisesOncePerBlockAndNeverReusesAfterRelaunch() async throws {
        let fixture = try makeDatabaseFixture()
        let firstProcess = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL,
            allocationBlockSize: 4
        )

        let firstGeneration = try await firstProcess.allocatePayloadGeneration()
        XCTAssertEqual(firstGeneration, 1)
        let afterFirstGeneration = try await firstProcess.load()
        XCTAssertEqual(afterFirstGeneration.generationHighWatermark, 4)
        XCTAssertEqual(afterFirstGeneration.revision, 1)
        let secondGeneration = try await firstProcess.allocatePayloadGeneration()
        let thirdGeneration = try await firstProcess.allocatePayloadGeneration()
        XCTAssertEqual(secondGeneration, 2)
        XCTAssertEqual(thirdGeneration, 3)
        let afterInBlockGenerations = try await firstProcess.load()
        XCTAssertEqual(afterInBlockGenerations.revision, 1, "in-block seal performs no DB commit")

        let firstReservation = try await firstProcess.allocateSendReservationID()
        XCTAssertEqual(firstReservation.rawValue, 1)
        let afterFirstReservation = try await firstProcess.load()
        XCTAssertEqual(afterFirstReservation.reservationHighWatermark, 4)
        XCTAssertEqual(afterFirstReservation.revision, 2)
        let secondReservation = try await firstProcess.allocateSendReservationID()
        XCTAssertEqual(secondReservation.rawValue, 2)
        let afterInBlockReservations = try await firstProcess.load()
        XCTAssertEqual(afterInBlockReservations.revision, 2)

        let relaunched = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL,
            allocationBlockSize: 4
        )
        let relaunchedGeneration = try await relaunched.allocatePayloadGeneration()
        let relaunchedReservation = try await relaunched.allocateSendReservationID()
        XCTAssertEqual(relaunchedGeneration, 5)
        XCTAssertEqual(relaunchedReservation.rawValue, 5)
        let restored = try await relaunched.load()
        XCTAssertEqual(restored.generationHighWatermark, 8)
        XCTAssertEqual(restored.reservationHighWatermark, 8)
        XCTAssertEqual(restored.revision, 4)
    }

    func testGenerationClaimFloorSurvivesSQLiteRelaunchAndRejectsOldBlock() async throws {
        let fixture = try makeDatabaseFixture()
        let first = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL,
            allocationBlockSize: 4
        )
        let firstGeneration = try await first.allocatePayloadGeneration()
        XCTAssertEqual(firstGeneration, 1)
        var snapshot = try await first.load()
        snapshot = try await first.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "claim first allocation block identity",
                mutations: [.claimGeneration(firstGeneration)]
            )
        )
        XCTAssertEqual(snapshot.claimedGenerations, [1])
        XCTAssertEqual(snapshot.generationClaimFloor, 0)

        let second = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL,
            allocationBlockSize: 4
        )
        let nextProcessGeneration = try await second.allocatePayloadGeneration()
        XCTAssertEqual(nextProcessGeneration, 5)
        snapshot = try await second.load()
        XCTAssertEqual(snapshot.generationHighWatermark, 8)
        XCTAssertEqual(snapshot.generationClaimFloor, 4)
        XCTAssertTrue(snapshot.claimedGenerations.isEmpty)

        let third = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL,
            allocationBlockSize: 4
        )
        do {
            _ = try await third.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "reject compacted generation after relaunch",
                    mutations: [.claimGeneration(firstGeneration)]
                )
            )
            XCTFail("an old allocation block must remain fenced after relaunch")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchanged = try await third.load()
        XCTAssertEqual(unchanged, snapshot)
    }

    func testLegacyExactClaimSetMetadataDecodesWithZeroFloor() async throws {
        let fixture = try makeDatabaseFixture()
        var creatingStore: GaryxSQLiteComposerDurabilityStore? = try .init(
            databaseURL: fixture.databaseURL
        )
        _ = creatingStore
        creatingStore = nil

        var database: OpaquePointer?
        XCTAssertEqual(
            sqlite3_open_v2(
                fixture.databaseURL.path,
                &database,
                SQLITE_OPEN_READWRITE | SQLITE_OPEN_FULLMUTEX,
                nil
            ),
            SQLITE_OK
        )
        let handle = try XCTUnwrap(database)
        XCTAssertEqual(
            sqlite3_exec(
                handle,
                "UPDATE composer_durability_metadata "
                    + "SET generation_high_watermark = 4, claimed_generations = '[3]' "
                    + "WHERE singleton = 1",
                nil,
                nil,
                nil
            ),
            SQLITE_OK
        )
        XCTAssertEqual(sqlite3_close_v2(handle), SQLITE_OK)
        database = nil

        let migrated = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL,
            allocationBlockSize: 4
        )
        let snapshot = try await migrated.load()
        XCTAssertEqual(snapshot.generationClaimFloor, 0)
        XCTAssertEqual(snapshot.claimedGenerations, [3])
    }

    func testConcreteStoreEnforcesLedgerBeforeDurableDescendant() async throws {
        let fixture = try makeDatabaseFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let drained = makeProducerDrained()
        do {
            _ = try await store.commit(
                .init(
                    expectedRevision: 0,
                    label: "descendant before ledger",
                    mutations: [.upsertProducerDrained(drained.key, drained.value)]
                )
            )
            XCTFail("ledger-first admission must be enforced by concrete store")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error: \(error)")
            }
        }
        let relaunched = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let restored = try await relaunched.load()
        XCTAssertEqual(restored, GaryxComposerDurabilitySnapshot())
    }

    func testReplacementSwapClearsSupersededOwnerShapeInSameSQLiteTransaction() async throws {
        let entry = makeEntry()
        let capture = GaryxPayloadLifecycleCapture(
            token: entry.lifecycle.token,
            revision: entry.lifecycle.revision
        )
        let oldKey = operationKey("old-swap")
        let successorKey = operationKey("new-swap")
        let assetID = GaryxStagedAssetID(rawValue: "replacement.bin")
        var old = GaryxOperationCapability(
            context: .init(
                key: oldKey,
                clientIdentity: "sqlite-client",
                configurationFingerprint: "sqlite-config",
                payloadLifecycle: capture
            ),
            state: .failedRetryable,
            stagedAssetID: assetID,
            reservedBytes: 101,
            uploadAttempted: true
        )
        var successor = GaryxOperationCapability(
            context: .init(
                key: successorKey,
                clientIdentity: "sqlite-client",
                configurationFingerprint: "sqlite-config",
                payloadLifecycle: capture
            ),
            state: .requested
        )
        var replacement = GaryxReplacementRecord(
            id: .init(rawValue: "sqlite-replacement"),
            scope: scope,
            entryID: entryID,
            oldKey: oldKey,
            reservationID: nil,
            branch: .followup,
            stagedAssetID: assetID,
            reservedBytes: 101
        )
        XCTAssertEqual(
            GaryxReplacementSwapReducer.commit(
                old: &old,
                successor: &successor,
                record: &replacement,
                lifecycle: entry.lifecycle.snapshot,
                scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
            ),
            .committed
        )
        var committedEntry = entry
        committedEntry.addOperation(oldKey)
        committedEntry.addOperation(successorKey)
        let transaction = GaryxComposerDurabilityTransaction(
            expectedRevision: 0,
            label: "concrete replacement swap",
            mutations: [
                .upsertEntry(committedEntry),
                .upsertOperation(old),
                .upsertOperation(successor),
                .upsertManifest(
                    .init(
                        key: successorKey,
                        stagedPath: assetID.rawValue,
                        state: successor.state,
                        uploadAttempted: false
                    )
                ),
                .upsertReplacement(replacement),
                .reserveStagedAsset(assetID: assetID, owner: successorKey, bytes: 101),
            ]
        )

        let failedFixture = try makeDatabaseFixture()
        let failedStore = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: failedFixture.databaseURL,
            boundaryHook: { boundary in
                if boundary == .beforeCommit {
                    throw GaryxSQLiteComposerDurabilityError.injectedFsyncFailure(boundary)
                }
            }
        )
        do {
            _ = try await failedStore.commit(transaction)
            XCTFail("swap must roll back as one transaction")
        } catch {
            XCTAssertEqual(
                error as? GaryxSQLiteComposerDurabilityError,
                .injectedFsyncFailure(.beforeCommit)
            )
        }
        let failedRelaunch = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: failedFixture.databaseURL
        )
        let failedRestored = try await failedRelaunch.load()
        XCTAssertEqual(failedRestored, GaryxComposerDurabilitySnapshot())

        let committedFixture = try makeDatabaseFixture()
        let committedStore = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: committedFixture.databaseURL
        )
        _ = try await committedStore.commit(transaction)
        let relaunched = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: committedFixture.databaseURL
        )
        let restored = try await relaunched.load()
        XCTAssertEqual(restored.operations[oldKey]?.state, .superseded)
        XCTAssertNil(restored.operations[oldKey]?.stagedAssetID)
        XCTAssertEqual(restored.operations[oldKey]?.reservedBytes, 0)
        XCTAssertEqual(restored.operations[successorKey]?.stagedAssetID, assetID)
        XCTAssertEqual(restored.stagedAssetOwners[assetID], successorKey)
        XCTAssertEqual(restored.reservedBytes, 101)
        XCTAssertEqual(restored.replacements[replacement.id]?.phase, .committed)
    }

    private func makeCommitSend() throws -> GaryxComposerCommitSend {
        var entry = makeEntry(text: "sealed")
        entry.setText("follow-up", generation: 11)
        let lifecycle = entry.lifecycle.snapshot
        let envelope = GaryxDeliveryEnvelope(
            text: "sealed",
            attachmentIDs: [],
            generation: 10,
            clientIntentID: "sqlite-intent"
        )
        var barrier = GaryxSendCommitBarrier(
            entryID: entryID,
            scope: scope,
            payloadLifecycle: .init(token: lifecycle.token, revision: lifecycle.revision)
        )
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservationID,
                envelope: envelope,
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: lifecycle
            ),
            .sealed
        )
        XCTAssertTrue(barrier.replaceProvisionalText("follow-up", lifecycle: lifecycle))
        let settlement = try XCTUnwrap(
            barrier.durableCommit(
                deliveryID: GaryxDeliveryRecordID(rawValue: "sqlite-delivery"),
                correlationID: "sqlite-correlation",
                clientIntentID: "sqlite-intent",
                lifecycle: lifecycle
            )
        )
        var ledger = GaryxProvisionalReservationLedger(
            key: .init(scope: scope, entryID: entryID, reservationID: reservationID),
            envelopeGeneration: 10,
            followupGeneration: 11
        )
        XCTAssertTrue(ledger.settle(.committed, targetGeneration: 11))
        return try GaryxComposerCommitSend(
            expectedRevision: 0,
            ledger: ledger,
            sealedPayloadEntry: entry,
            barrier: barrier,
            settlement: settlement
        )
    }

    private func makeEntry(text: String = "") -> GaryxComposerPayloadEntry {
        GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .draft("sqlite-draft"),
            lifecycleToken: .init(entryID: entryID, nonce: "sqlite-token"),
            currentGeneration: 10,
            text: text
        )
    }

    private func operationKey(_ rawValue: String) -> GaryxOperationCapabilityKey {
        GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entryID,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: .init(rawValue: rawValue)
        )
    }

    private func makeProducerDrained() -> (
        key: GaryxSessionDescendantKey,
        value: GaryxDurableProducerDrainedRecord
    ) {
        let entry = makeEntry()
        let key = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: GaryxComposerInputSessionID(rawValue: "sqlite-session"),
            epoch: 1
        )
        return (
            key,
            GaryxDurableProducerDrainedRecord(
                scope: scope,
                entryID: entryID,
                reservationID: reservationID,
                record: GaryxProducerDrainedRecord(
                    sessionID: key.sessionID,
                    epoch: key.epoch,
                    finalSequence: 3,
                    bufferedText: "buffer"
                )
            )
        )
    }

    private func makeDatabaseFixture() throws -> (directory: URL, databaseURL: URL) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("garyx-sqlite-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return (directory, directory.appendingPathComponent("composer.sqlite3"))
    }

    private func queryStrings(
        _ sql: String,
        column: Int32 = 0,
        database: OpaquePointer
    ) throws -> [String] {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK,
              let statement else {
            throw NSError(domain: "SQLiteTest", code: 1)
        }
        defer { sqlite3_finalize(statement) }
        var values: [String] = []
        while sqlite3_step(statement) == SQLITE_ROW {
            if let value = sqlite3_column_text(statement, column) {
                values.append(String(cString: value))
            }
        }
        return values
    }

    private func scalarInt(_ sql: String, database: OpaquePointer) throws -> Int64 {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK,
              let statement else {
            throw NSError(domain: "SQLiteTest", code: 2)
        }
        defer { sqlite3_finalize(statement) }
        guard sqlite3_step(statement) == SQLITE_ROW else {
            throw NSError(domain: "SQLiteTest", code: 3)
        }
        return sqlite3_column_int64(statement, 0)
    }
}
