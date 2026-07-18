import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxComposerStagedAssetStoreTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "staging-gateway", epoch: 1)
    private let entryID = GaryxComposerPayloadEntryID(rawValue: "staging-entry")
    private let assetID = GaryxStagedAssetID(rawValue: "asset.bin")

    func testQuotaOwnerCommitsBeforeCopyAndFileIsPrivateAtomicAndExcludedFromBackup() async throws {
        let fixture = try makeFixture(bytes: Data("payload".utf8))
        let durability = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let probe = StagingProbe()
        let expectedDestination = fixture.applicationSupport
            .appendingPathComponent("Garyx/ComposerPayload/asset.bin")
        let staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: fixture.applicationSupport,
            durability: durability,
            quotaLimitBytes: 1_024,
            boundaryHook: { boundary in
                if boundary == .quotaReserved {
                    probe.sawQuotaBeforeFile = !FileManager.default.fileExists(
                        atPath: expectedDestination.path
                    )
                }
            }
        )

        let result = try await staging.stage(
            makeAdmission(sourceURL: fixture.sourceURL, expectedRevision: 0)
        )
        XCTAssertTrue(probe.sawQuotaBeforeFile)
        XCTAssertEqual(try Data(contentsOf: result.fileURL), Data("payload".utf8))
        let values = try result.fileURL.resourceValues(forKeys: [.isExcludedFromBackupKey])
        XCTAssertEqual(values.isExcludedFromBackup, true)
        let attributes = try FileManager.default.attributesOfItem(atPath: result.fileURL.path)
        XCTAssertEqual((attributes[.posixPermissions] as? NSNumber)?.intValue, 0o600)

        let snapshot = try await durability.load()
        XCTAssertEqual(snapshot.reservedBytes, 7)
        XCTAssertEqual(snapshot.stagedAssetOwners[assetID], result.operation.context.key)
        XCTAssertEqual(snapshot.manifests[result.operation.context.key], result.manifest)
        XCTAssertEqual(
            snapshot.payloadStore.entry(entryID, scope: scope)?.operationKeys,
            [result.operation.context.key]
        )
    }

    func testQuotaFailureLeavesDatabaseAndFilesystemUntouched() async throws {
        let fixture = try makeFixture(bytes: Data(repeating: 1, count: 8))
        let durability = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: fixture.applicationSupport,
            durability: durability,
            quotaLimitBytes: 4
        )

        do {
            _ = try await staging.stage(
                makeAdmission(sourceURL: fixture.sourceURL, expectedRevision: 0)
            )
            XCTFail("quota must fail before copy")
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerStagingError,
                .quotaExceeded(limit: 4, requested: 8, alreadyReserved: 0)
            )
        }
        let snapshot = try await durability.load()
        XCTAssertEqual(snapshot, GaryxComposerDurabilitySnapshot())
        XCTAssertEqual(try stagedFiles(in: staging.rootURL), [])
    }

    func testEveryCopyAndFsyncFailpointCompensatesOwnerFileAndQuota() async throws {
        let boundaries: [GaryxComposerStagingBoundary] = [
            .quotaReserved,
            .beforeCopy,
            .copiedToTemporaryFile,
            .beforeFileSync,
            .fileSynced,
            .atomicallyRenamed,
            .directorySynced,
        ]
        for boundary in boundaries {
            let fixture = try makeFixture(bytes: Data("recoverable".utf8))
            let durability = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
            let staging = try GaryxComposerStagedAssetStore(
                applicationSupportDirectory: fixture.applicationSupport,
                durability: durability,
                quotaLimitBytes: 1_024,
                boundaryHook: { observed in
                    if observed == boundary {
                        if observed == .beforeFileSync {
                            throw GaryxComposerStagingError.injectedFsyncFailure(observed)
                        }
                        throw GaryxComposerStagingError.injectedNoSpace(observed)
                    }
                }
            )

            do {
                _ = try await staging.stage(
                    makeAdmission(sourceURL: fixture.sourceURL, expectedRevision: 0)
                )
                XCTFail("expected staging failure at \(boundary)")
            } catch {
                let expected: GaryxComposerStagingError = boundary == .beforeFileSync
                    ? .injectedFsyncFailure(boundary)
                    : .injectedNoSpace(boundary)
                XCTAssertEqual(error as? GaryxComposerStagingError, expected)
            }

            let relaunched = try GaryxSQLiteComposerDurabilityStore(
                databaseURL: fixture.databaseURL
            )
            let snapshot = try await relaunched.load()
            XCTAssertEqual(snapshot.reservedBytes, 0, "boundary \(boundary)")
            XCTAssertTrue(snapshot.stagedAssetOwners.isEmpty, "boundary \(boundary)")
            XCTAssertTrue(snapshot.pendingFileCleanup.isEmpty, "boundary \(boundary)")
            XCTAssertTrue(snapshot.operations.isEmpty, "boundary \(boundary)")
            XCTAssertTrue(snapshot.manifests.isEmpty, "boundary \(boundary)")
            XCTAssertEqual(try stagedFiles(in: staging.rootURL), [], "boundary \(boundary)")
        }
    }

    func testReservationScopedManifestCannotPrecedeLedger() async throws {
        let fixture = try makeFixture(bytes: Data("payload".utf8))
        let durability = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: fixture.applicationSupport,
            durability: durability,
            quotaLimitBytes: 1_024
        )
        var admission = makeAdmission(sourceURL: fixture.sourceURL, expectedRevision: 0)
        let reservationContext = GaryxScopeBoundOperationContext(
            key: GaryxOperationCapabilityKey(
                scope: scope,
                entryID: entryID,
                generation: 10,
                reservationID: GaryxSendReservationID(rawValue: 1),
                branch: .followup,
                operationID: admission.context.key.operationID
            ),
            clientIdentity: "staging-client",
            configurationFingerprint: "staging-config",
            payloadLifecycle: admission.context.payloadLifecycle
        )
        admission = GaryxComposerStagedAssetAdmission(
            expectedRevision: admission.expectedRevision,
            sourceURL: admission.sourceURL,
            assetID: admission.assetID,
            entry: admission.entry,
            context: reservationContext
        )

        do {
            _ = try await staging.stage(admission)
            XCTFail("reservation descendant must require its ledger")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected durability error \(error)")
            }
        }
        let snapshot = try await durability.load()
        XCTAssertEqual(snapshot, GaryxComposerDurabilitySnapshot())
        XCTAssertEqual(try stagedFiles(in: staging.rootURL), [])
    }

    private func makeAdmission(
        sourceURL: URL,
        expectedRevision: UInt64
    ) -> GaryxComposerStagedAssetAdmission {
        let entry = GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .draft("staging-draft"),
            lifecycleToken: .init(entryID: entryID, nonce: "staging-token"),
            currentGeneration: 10
        )
        let context = GaryxScopeBoundOperationContext(
            key: GaryxOperationCapabilityKey(
                scope: scope,
                entryID: entryID,
                generation: 10,
                reservationID: nil,
                branch: .followup,
                operationID: GaryxOperationID(rawValue: "staging-operation")
            ),
            clientIdentity: "staging-client",
            configurationFingerprint: "staging-config",
            payloadLifecycle: .init(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        return GaryxComposerStagedAssetAdmission(
            expectedRevision: expectedRevision,
            sourceURL: sourceURL,
            assetID: assetID,
            entry: entry,
            context: context
        )
    }

    private func makeFixture(bytes: Data) throws -> (
        directory: URL,
        applicationSupport: URL,
        databaseURL: URL,
        sourceURL: URL
    ) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("garyx-staging-tests-\(UUID().uuidString)", isDirectory: true)
        let applicationSupport = directory.appendingPathComponent("ApplicationSupport")
        try FileManager.default.createDirectory(
            at: applicationSupport,
            withIntermediateDirectories: true
        )
        let sourceURL = directory.appendingPathComponent("source.bin")
        try bytes.write(to: sourceURL, options: .atomic)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return (
            directory,
            applicationSupport,
            applicationSupport.appendingPathComponent("composer.sqlite3"),
            sourceURL
        )
    }

    private func stagedFiles(in directory: URL) throws -> [String] {
        guard FileManager.default.fileExists(atPath: directory.path) else { return [] }
        return try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted()
    }
}

private final class StagingProbe: @unchecked Sendable {
    var sawQuotaBeforeFile = false
}
