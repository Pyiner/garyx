import Darwin
import Foundation

public enum GaryxComposerStagingBoundary: Equatable, Sendable {
    case quotaReserved
    case beforeCopy
    case copiedToTemporaryFile
    case beforeFileSync
    case fileSynced
    case atomicallyRenamed
    case directorySynced
}

public enum GaryxComposerStagingError: Error, Equatable, Sendable {
    case invalidSource
    case invalidOperationIdentity
    case quotaExceeded(limit: Int, requested: Int, alreadyReserved: Int)
    case destinationAlreadyExists
    case sourceChanged(expectedBytes: Int, actualBytes: Int)
    case filesystem(code: Int32, operation: String)
    case injectedNoSpace(GaryxComposerStagingBoundary)
    case injectedFsyncFailure(GaryxComposerStagingBoundary)
}

public struct GaryxComposerStagedAssetAdmission: Sendable {
    public let expectedRevision: UInt64
    public let sourceURL: URL
    public let assetID: GaryxStagedAssetID
    public let entry: GaryxComposerPayloadEntry
    public let context: GaryxScopeBoundOperationContext

    public init(
        expectedRevision: UInt64,
        sourceURL: URL,
        assetID: GaryxStagedAssetID,
        entry: GaryxComposerPayloadEntry,
        context: GaryxScopeBoundOperationContext
    ) {
        self.expectedRevision = expectedRevision
        self.sourceURL = sourceURL
        self.assetID = assetID
        self.entry = entry
        self.context = context
    }
}

public struct GaryxComposerStagedAssetResult: Sendable {
    public let fileURL: URL
    public let operation: GaryxOperationCapability
    public let manifest: GaryxOperationManifest
    public let snapshot: GaryxComposerDurabilitySnapshot
}

/// Fixed application-data protection policy with an audit seam used by the
/// app-hosted iOS tests. The audit fires only after Foundation accepts the
/// required protection attribute for that path.
public struct GaryxComposerFileProtectionPolicy: Sendable {
    public typealias Audit = @Sendable (URL, FileProtectionType) -> Void

    public static let system = Self()
    private let audit: Audit

    public init(audit: @escaping Audit = { _, _ in }) {
        self.audit = audit
    }

    func apply(to url: URL) throws {
        let protection = FileProtectionType.completeUntilFirstUserAuthentication
        #if os(iOS)
        try FileManager.default.setAttributes(
            [.protectionKey: protection],
            ofItemAtPath: url.path
        )
        #endif
        audit(url, protection)
    }
}

/// Protected, app-private payload staging. Quota/owner metadata is committed
/// before a source byte is copied. A process death in the reservation→rename
/// interval is recovered from the durable manifest and owner ledger.
public actor GaryxComposerStagedAssetStore {
    public typealias BoundaryHook = @Sendable (GaryxComposerStagingBoundary) throws -> Void

    public nonisolated let rootURL: URL
    public nonisolated let quotaLimitBytes: Int

    private let durability: any GaryxComposerDurabilityStore
    private let boundaryHook: BoundaryHook
    private let fileProtectionPolicy: GaryxComposerFileProtectionPolicy

    public init(
        applicationSupportDirectory: URL,
        durability: any GaryxComposerDurabilityStore,
        quotaLimitBytes: Int,
        boundaryHook: @escaping BoundaryHook = { _ in },
        fileProtectionPolicy: GaryxComposerFileProtectionPolicy = .system
    ) throws {
        precondition(quotaLimitBytes >= 0)
        rootURL = applicationSupportDirectory
            .appendingPathComponent("Garyx", isDirectory: true)
            .appendingPathComponent("ComposerPayload", isDirectory: true)
        self.durability = durability
        self.quotaLimitBytes = quotaLimitBytes
        self.boundaryHook = boundaryHook
        self.fileProtectionPolicy = fileProtectionPolicy
        try Self.preparePrivateDirectory(rootURL, fileProtectionPolicy: fileProtectionPolicy)
    }

    public func stage(
        _ admission: GaryxComposerStagedAssetAdmission
    ) async throws -> GaryxComposerStagedAssetResult {
        let sourceValues = try admission.sourceURL.resourceValues(
            forKeys: [.isRegularFileKey, .fileSizeKey]
        )
        guard sourceValues.isRegularFile == true,
              let byteCount = sourceValues.fileSize,
              byteCount >= 0 else {
            throw GaryxComposerStagingError.invalidSource
        }
        guard admission.context.key.scope == admission.entry.scope,
              admission.context.key.entryID == admission.entry.id,
              admission.context.payloadLifecycle.token == admission.entry.lifecycle.token,
              admission.context.payloadLifecycle.revision == admission.entry.lifecycle.revision,
              admission.entry.lifecycle.phase == .active else {
            throw GaryxComposerStagingError.invalidOperationIdentity
        }

        let destinationURL = rootURL.appendingPathComponent(
            admission.assetID.rawValue,
            isDirectory: false
        )
        guard !FileManager.default.fileExists(atPath: destinationURL.path) else {
            throw GaryxComposerStagingError.destinationAlreadyExists
        }
        let snapshot = try await durability.load()
        guard snapshot.revision == admission.expectedRevision else {
            throw GaryxComposerDurabilityError.revisionConflict(
                expected: admission.expectedRevision,
                actual: snapshot.revision
            )
        }
        guard byteCount <= quotaLimitBytes,
              snapshot.reservedBytes <= quotaLimitBytes - byteCount else {
            throw GaryxComposerStagingError.quotaExceeded(
                limit: quotaLimitBytes,
                requested: byteCount,
                alreadyReserved: snapshot.reservedBytes
            )
        }

        var entry = admission.entry
        let operation = GaryxOperationCapability(
            context: admission.context,
            state: .preparing,
            stagedAssetID: admission.assetID,
            reservedBytes: byteCount
        )
        entry.addOperation(admission.context.key)
        let manifest = GaryxOperationManifest(
            key: admission.context.key,
            stagedPath: destinationURL.lastPathComponent,
            state: .preparing,
            uploadAttempted: false
        )
        let reservedSnapshot = try await durability.commit(
            .init(
                expectedRevision: admission.expectedRevision,
                label: "reserve staged payload before copy",
                mutations: [
                    .upsertEntry(entry),
                    .upsertOperation(operation),
                    .upsertManifest(manifest),
                    .reserveStagedAsset(
                        assetID: admission.assetID,
                        owner: admission.context.key,
                        bytes: byteCount
                    ),
                ]
            )
        )

        do {
            try boundaryHook(.quotaReserved)
            try boundaryHook(.beforeCopy)
            try copyAtomically(
                sourceURL: admission.sourceURL,
                destinationURL: destinationURL,
                expectedBytes: byteCount
            )
        } catch {
            await compensateFailedCopy(
                entry: entry,
                operation: operation,
                assetID: admission.assetID,
                destinationURL: destinationURL,
                expectedRevision: reservedSnapshot.revision
            )
            throw error
        }

        return GaryxComposerStagedAssetResult(
            fileURL: destinationURL,
            operation: operation,
            manifest: manifest,
            snapshot: reservedSnapshot
        )
    }

    public func settleCondemnedFiles() async throws -> GaryxComposerDurabilitySnapshot {
        var snapshot = try await durability.load()
        for assetID in snapshot.pendingFileCleanup.keys.sorted(by: { $0.rawValue < $1.rawValue }) {
            let fileURL = rootURL.appendingPathComponent(assetID.rawValue)
            try removeIfPresent(fileURL)
            snapshot = try await durability.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "confirm condemned staged-file deletion",
                    mutations: [.completeFileCleanup(assetID)]
                )
            )
        }
        try removeInterruptedCopies()
        return snapshot
    }

    /// A SIGKILL bypasses the staging call's `defer`, so launch recovery must
    /// remove copy-on-write temporaries independently of the domain tombstone.
    /// Only files created by `copyAtomically` match this private name shape.
    private func removeInterruptedCopies() throws {
        let children: [URL]
        do {
            children = try FileManager.default.contentsOfDirectory(
                at: rootURL,
                includingPropertiesForKeys: [.isRegularFileKey],
                options: [.skipsSubdirectoryDescendants]
            )
        } catch let error as NSError {
            throw Self.filesystemError(error, operation: "enumerate staged payloads")
        }
        var removedAny = false
        for child in children where
            child.lastPathComponent.hasPrefix(".")
                && child.lastPathComponent.contains(".partial-") {
            do {
                try FileManager.default.removeItem(at: child)
                removedAny = true
            } catch let error as NSError {
                throw Self.filesystemError(error, operation: "delete interrupted staged payload")
            }
        }
        if removedAny {
            try Self.synchronizeDirectory(rootURL)
        }
    }

    private func copyAtomically(
        sourceURL: URL,
        destinationURL: URL,
        expectedBytes: Int
    ) throws {
        let temporaryURL = rootURL.appendingPathComponent(
            ".\(destinationURL.lastPathComponent).partial-\(UUID().uuidString)"
        )
        defer { try? FileManager.default.removeItem(at: temporaryURL) }
        let descriptor = Darwin.open(
            temporaryURL.path,
            O_WRONLY | O_CREAT | O_EXCL,
            S_IRUSR | S_IWUSR
        )
        guard descriptor >= 0 else {
            throw GaryxComposerStagingError.filesystem(
                code: errno,
                operation: "create protected staging temporary"
            )
        }
        Darwin.close(descriptor)
        try protectFile(temporaryURL)
        do {
            let source = try FileHandle(forReadingFrom: sourceURL)
            let destination = try FileHandle(forWritingTo: temporaryURL)
            defer {
                try? source.close()
                try? destination.close()
            }
            while let chunk = try source.read(upToCount: 1_048_576), !chunk.isEmpty {
                try destination.write(contentsOf: chunk)
            }
            try source.close()
            try destination.close()
        } catch let error as NSError {
            throw Self.filesystemError(error, operation: "copy staged payload")
        }
        try boundaryHook(.copiedToTemporaryFile)
        let copiedValues = try temporaryURL.resourceValues(forKeys: [.fileSizeKey])
        let actualBytes = copiedValues.fileSize ?? -1
        guard actualBytes == expectedBytes else {
            throw GaryxComposerStagingError.sourceChanged(
                expectedBytes: expectedBytes,
                actualBytes: actualBytes
            )
        }

        try boundaryHook(.beforeFileSync)
        do {
            let file = try FileHandle(forWritingTo: temporaryURL)
            try file.synchronize()
            try file.close()
        } catch let error as NSError {
            throw Self.filesystemError(error, operation: "fsync staged payload")
        }
        try boundaryHook(.fileSynced)
        do {
            try FileManager.default.moveItem(at: temporaryURL, to: destinationURL)
        } catch let error as NSError {
            throw Self.filesystemError(error, operation: "rename staged payload")
        }
        try protectFile(destinationURL)
        try boundaryHook(.atomicallyRenamed)
        try Self.synchronizeDirectory(rootURL)
        try boundaryHook(.directorySynced)
    }

    private func compensateFailedCopy(
        entry originalEntry: GaryxComposerPayloadEntry,
        operation originalOperation: GaryxOperationCapability,
        assetID: GaryxStagedAssetID,
        destinationURL: URL,
        expectedRevision: UInt64
    ) async {
        var operation = originalOperation
        operation.settleIdentityDiscard()
        var entry = originalEntry
        entry.removeOperation(originalOperation.context.key)
        do {
            let cleanup = try await durability.commit(
                .init(
                    expectedRevision: expectedRevision,
                    label: "record failed staged-copy cleanup",
                    mutations: [
                        .upsertOperation(operation),
                        .registerFileCleanup(
                            assetID: assetID,
                            owner: originalOperation.context.key
                        ),
                        .releaseStagedAsset(assetID),
                        .removeManifest(originalOperation.context.key),
                        .removeOperation(originalOperation.context.key),
                        .upsertEntry(entry),
                    ]
                )
            )
            try removeIfPresent(destinationURL)
            _ = try await durability.commit(
                .init(
                    expectedRevision: cleanup.revision,
                    label: "confirm failed staged-copy deletion",
                    mutations: [.completeFileCleanup(assetID)]
                )
            )
        } catch {
            // The durable manifest/owner from the reservation transaction is
            // intentionally left for launch recovery if compensation itself
            // cannot commit. Never erase the only recovery evidence here.
        }
    }

    private func removeIfPresent(_ fileURL: URL) throws {
        guard FileManager.default.fileExists(atPath: fileURL.path) else { return }
        do {
            try FileManager.default.removeItem(at: fileURL)
            try Self.synchronizeDirectory(rootURL)
        } catch let error as NSError {
            throw Self.filesystemError(error, operation: "delete condemned staged payload")
        }
    }

    private static func preparePrivateDirectory(
        _ directory: URL,
        fileProtectionPolicy: GaryxComposerFileProtectionPolicy
    ) throws {
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        var mutableDirectory = directory
        try mutableDirectory.setResourceValues(values)
        try fileProtectionPolicy.apply(to: directory)
    }

    private func protectFile(_ fileURL: URL) throws {
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o600],
            ofItemAtPath: fileURL.path
        )
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        var mutableURL = fileURL
        try mutableURL.setResourceValues(values)
        try fileProtectionPolicy.apply(to: fileURL)
    }

    private static func synchronizeDirectory(_ directory: URL) throws {
        let descriptor = Darwin.open(directory.path, O_RDONLY)
        guard descriptor >= 0 else {
            throw GaryxComposerStagingError.filesystem(
                code: errno,
                operation: "open staging directory"
            )
        }
        defer { Darwin.close(descriptor) }
        guard Darwin.fsync(descriptor) == 0 else {
            throw GaryxComposerStagingError.filesystem(
                code: errno,
                operation: "fsync staging directory"
            )
        }
    }

    private static func filesystemError(
        _ error: NSError,
        operation: String
    ) -> GaryxComposerStagingError {
        .filesystem(code: Int32(error.code), operation: operation)
    }
}
