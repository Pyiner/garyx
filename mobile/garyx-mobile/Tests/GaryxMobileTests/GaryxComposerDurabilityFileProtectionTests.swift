import Foundation
import XCTest
@testable import GaryxMobile

final class GaryxComposerDurabilityFileProtectionTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "protection-gateway", epoch: 1)
    private let entryID = GaryxComposerPayloadEntryID(rawValue: "protection-entry")

    func testTemporaryAndFinalStagedPayloadUseProtectionBeforeCopyBoundary() async throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let sourceURL = directory.appendingPathComponent("source.bin")
        try Data("protected payload".utf8).write(to: sourceURL)
        try FileManager.default.setAttributes(
            [.protectionKey: FileProtectionType.complete],
            ofItemAtPath: sourceURL.path
        )
        let databaseURL = directory.appendingPathComponent("Database/composer.sqlite")
        let probe = FileProtectionProbe()
        let policy = GaryxComposerFileProtectionPolicy { url, protection in
            probe.record(url: url, protection: protection)
        }
        let store = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: databaseURL,
            fileProtectionPolicy: policy
        )
        let staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: directory,
            durability: store,
            quotaLimitBytes: 1_024,
            boundaryHook: { boundary in
                guard boundary == .copiedToTemporaryFile else { return }
                let partial = try XCTUnwrap(
                    FileManager.default.contentsOfDirectory(
                        at: directory.appendingPathComponent("Garyx/ComposerPayload"),
                        includingPropertiesForKeys: nil
                    ).first(where: { $0.lastPathComponent.contains(".partial-") })
                )
                probe.recordCopyBoundary(for: partial)
            },
            fileProtectionPolicy: policy
        )
        let entry = GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .draft("protection-draft"),
            lifecycleToken: .init(entryID: entryID, nonce: "protection-token"),
            currentGeneration: 10
        )
        let key = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entryID,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: .init(rawValue: "protection-operation")
        )
        let result = try await staging.stage(
            .init(
                expectedRevision: 0,
                sourceURL: sourceURL,
                assetID: .init(rawValue: "protected.bin"),
                entry: entry,
                context: .init(
                    key: key,
                    clientIdentity: "protection-client",
                    configurationFingerprint: "protection-config",
                    payloadLifecycle: .init(
                        token: entry.lifecycle.token,
                        revision: entry.lifecycle.revision
                    )
                )
            )
        )

        XCTAssertEqual(probe.copyBoundaryValue, .completeUntilFirstUserAuthentication)
        XCTAssertTrue(
            probe.protectedPaths.contains(where: { $0.contains(".partial-") })
        )
        XCTAssertEqual(
            probe.protection(for: result.fileURL),
            .completeUntilFirstUserAuthentication
        )
        XCTAssertEqual(
            try result.fileURL.resourceValues(forKeys: [.isExcludedFromBackupKey])
                .isExcludedFromBackup,
            true
        )
    }

    func testSQLiteReopenReappliesProtectionAndBackupExclusionToMainWALAndSHM() async throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let databaseURL = directory.appendingPathComponent("Database/composer.sqlite")
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: databaseURL)
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "create protected sqlite sidecars",
                mutations: [.setGenerationHighWatermark(32)]
            )
        )
        let paths = [
            databaseURL,
            URL(fileURLWithPath: databaseURL.path + "-wal"),
            URL(fileURLWithPath: databaseURL.path + "-shm"),
        ]
        for path in paths {
            XCTAssertTrue(FileManager.default.fileExists(atPath: path.path), path.lastPathComponent)
        }

        let sharedMemoryURL = paths[2]
        try FileManager.default.setAttributes(
            [.protectionKey: FileProtectionType.complete],
            ofItemAtPath: sharedMemoryURL.path
        )
        var unprotectedValues = URLResourceValues()
        unprotectedValues.isExcludedFromBackup = false
        var mutableSharedMemoryURL = sharedMemoryURL
        try mutableSharedMemoryURL.setResourceValues(unprotectedValues)

        let reopenProbe = FileProtectionProbe()
        _ = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: databaseURL,
            fileProtectionPolicy: .init { url, protection in
                reopenProbe.record(url: url, protection: protection)
            }
        )
        for path in paths {
            XCTAssertEqual(
                reopenProbe.protection(for: path),
                .completeUntilFirstUserAuthentication,
                path.lastPathComponent
            )
            XCTAssertEqual(
                try path.resourceValues(forKeys: [.isExcludedFromBackupKey])
                    .isExcludedFromBackup,
                true,
                path.lastPathComponent
            )
        }
    }

    private func makeTemporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("GaryxProtectionTests-(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        return directory
    }

}

private final class FileProtectionProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: FileProtectionType] = [:]
    private var storedCopyBoundaryValue: FileProtectionType?

    var copyBoundaryValue: FileProtectionType? {
        lock.withLock { storedCopyBoundaryValue }
    }

    var protectedPaths: Set<String> {
        lock.withLock { Set(values.keys) }
    }

    func record(url: URL, protection: FileProtectionType) {
        lock.withLock { values[url.path] = protection }
    }

    func recordCopyBoundary(for url: URL) {
        lock.withLock { storedCopyBoundaryValue = values[url.path] }
    }

    func protection(for url: URL) -> FileProtectionType? {
        lock.withLock { values[url.path] }
    }
}
