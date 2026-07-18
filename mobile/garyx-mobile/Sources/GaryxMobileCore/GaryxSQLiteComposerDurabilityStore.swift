import Foundation
import SQLite3

public enum GaryxComposerDurabilityRecordFamily: String, CaseIterable, Sendable {
    case payload = "composer_payload_store"
    case aliases = "composer_aliases"
    case operations = "composer_operation_capabilities"
    case manifests = "composer_operation_manifests"
    case replacements = "composer_replacement_records"
    case conflicts = "composer_payload_conflict_sets"
    case feedback = "composer_feedback_records"
    case attachmentLineages = "composer_attachment_lineages"
    case barriers = "composer_send_barriers"
    case reservationLedgers = "composer_reservation_ledgers"
    case producerDrained = "composer_producer_drained"
    case recoveredInputClosures = "composer_recovered_input_closures"
    case deliveryOutbox = "composer_delivery_outbox"
    case discardConvergence = "composer_discard_convergence"
    case createDeliveries = "composer_create_deliveries"
    case stagedAssets = "composer_staged_asset_ledger"
}

public enum GaryxComposerDurabilityStorageBoundary: Equatable, Sendable {
    case transactionBegan
    case mutationApplied(Int)
    case familyPersisted(GaryxComposerDurabilityRecordFamily)
    case metadataPersisted
    case beforeCommit
    case afterCommit
}

public enum GaryxSQLiteComposerDurabilityError: Error, Equatable, Sendable {
    case sqlite(code: Int32, message: String)
    case invalidDatabasePath
    case missingRecordFamily(String)
    case invalidMetadata(String)
    case encoding(String)
    case injectedNoSpace(GaryxComposerDurabilityStorageBoundary)
    case injectedFsyncFailure(GaryxComposerDurabilityStorageBoundary)
}

/// Concrete A4d-1 store. Every composer record family is a table in one
/// SQLite database and every publication uses one `BEGIN IMMEDIATE`/`COMMIT`
/// transaction. WAL + `synchronous=FULL` supplies the process-death boundary;
/// the shared Core reducer supplies CAS and cross-family invariants.
public actor GaryxSQLiteComposerDurabilityStore: GaryxComposerDurabilityStore {
    public typealias BoundaryHook = @Sendable (
        GaryxComposerDurabilityStorageBoundary
    ) throws -> Void

    public static let schemaVersion = 1
    public static let metadataTableName = "composer_durability_metadata"
    public static let schemaTableNames = [metadataTableName]
        + GaryxComposerDurabilityRecordFamily.allCases.map(\.rawValue)

    private let connection: GaryxSQLiteConnection
    private let boundaryHook: BoundaryHook
    private var generationAllocator: GaryxDurableHiLoAllocator
    private var reservationAllocator: GaryxDurableHiLoAllocator

    public init(
        databaseURL: URL,
        allocationBlockSize: UInt64 = 32,
        boundaryHook: @escaping BoundaryHook = { _ in },
        fileProtectionPolicy: GaryxComposerFileProtectionPolicy = .system
    ) throws {
        guard databaseURL.isFileURL, !databaseURL.path.isEmpty else {
            throw GaryxSQLiteComposerDurabilityError.invalidDatabasePath
        }
        let directory = databaseURL.deletingLastPathComponent()
        try Self.preparePrivateDirectory(
            directory,
            fileProtectionPolicy: fileProtectionPolicy
        )
        let connection = try GaryxSQLiteConnection(path: databaseURL.path)
        try Self.configure(connection)
        try Self.createSchemaIfNeeded(connection)
        try Self.protectDatabaseFiles(
            databaseURL,
            fileProtectionPolicy: fileProtectionPolicy
        )
        let snapshot = try Self.readSnapshot(connection)

        self.connection = connection
        self.boundaryHook = boundaryHook
        generationAllocator = GaryxDurableHiLoAllocator(
            persistedHighWatermark: snapshot.generationHighWatermark,
            blockSize: allocationBlockSize
        )
        reservationAllocator = GaryxDurableHiLoAllocator(
            persistedHighWatermark: snapshot.reservationHighWatermark,
            blockSize: allocationBlockSize
        )
    }

    public func load() async throws -> GaryxComposerDurabilitySnapshot {
        try Self.readSnapshot(connection)
    }

    public func allocatePayloadGeneration() async throws -> UInt64 {
        while true {
            let snapshot = try Self.readSnapshot(connection)
            if snapshot.generationHighWatermark > generationAllocator.persistedHighWatermark {
                generationAllocator = GaryxDurableHiLoAllocator(
                    persistedHighWatermark: snapshot.generationHighWatermark,
                    blockSize: generationAllocator.blockSize
                )
            }
            var candidate = generationAllocator
            let value = candidate.allocate()
            guard candidate.persistedHighWatermark != generationAllocator.persistedHighWatermark else {
                generationAllocator = candidate
                return value
            }
            do {
                _ = try commitSynchronously(
                    .init(
                        expectedRevision: snapshot.revision,
                        label: "reserve payload-generation hi-lo block",
                        mutations: [.setGenerationHighWatermark(candidate.persistedHighWatermark)]
                    )
                )
                generationAllocator = candidate
                return value
            } catch GaryxComposerDurabilityError.revisionConflict {
                continue
            }
        }
    }

    public func allocateSendReservationID() async throws -> GaryxSendReservationID {
        while true {
            let snapshot = try Self.readSnapshot(connection)
            if snapshot.reservationHighWatermark > reservationAllocator.persistedHighWatermark {
                reservationAllocator = GaryxDurableHiLoAllocator(
                    persistedHighWatermark: snapshot.reservationHighWatermark,
                    blockSize: reservationAllocator.blockSize
                )
            }
            var candidate = reservationAllocator
            let value = candidate.allocate()
            guard candidate.persistedHighWatermark != reservationAllocator.persistedHighWatermark else {
                reservationAllocator = candidate
                return GaryxSendReservationID(rawValue: value)
            }
            do {
                _ = try commitSynchronously(
                    .init(
                        expectedRevision: snapshot.revision,
                        label: "reserve send-reservation hi-lo block",
                        mutations: [.setReservationHighWatermark(candidate.persistedHighWatermark)]
                    )
                )
                reservationAllocator = candidate
                return GaryxSendReservationID(rawValue: value)
            } catch GaryxComposerDurabilityError.revisionConflict {
                continue
            }
        }
    }

    public func commit(
        _ transaction: GaryxComposerDurabilityTransaction
    ) async throws -> GaryxComposerDurabilitySnapshot {
        try commitSynchronously(transaction)
    }

    public func commitSend(
        _ send: GaryxComposerCommitSend
    ) async throws -> GaryxComposerDurabilitySnapshot {
        try commitSynchronously(send.transaction)
    }

    private func commitSynchronously(
        _ transaction: GaryxComposerDurabilityTransaction
    ) throws -> GaryxComposerDurabilitySnapshot {
        try connection.execute("BEGIN IMMEDIATE")
        var transactionOpen = true
        defer {
            if transactionOpen {
                try? connection.execute("ROLLBACK")
            }
        }
        try boundaryHook(.transactionBegan)
        let current = try Self.readSnapshot(connection)
        let candidate = try GaryxComposerDurabilityTransactionEngine.applying(
            transaction,
            to: current,
            afterApplyingMutation: { [boundaryHook] index in
                try boundaryHook(.mutationApplied(index))
            }
        )
        try Self.persist(candidate, connection: connection, boundaryHook: boundaryHook)
        try boundaryHook(.beforeCommit)
        try connection.execute("COMMIT")
        transactionOpen = false
        try boundaryHook(.afterCommit)
        return candidate
    }

    private static func configure(_ connection: GaryxSQLiteConnection) throws {
        try connection.execute("PRAGMA journal_mode=WAL")
        try connection.execute("PRAGMA synchronous=FULL")
        try connection.execute("PRAGMA foreign_keys=ON")
        try connection.execute("PRAGMA busy_timeout=5000")
        try connection.execute("PRAGMA wal_autocheckpoint=1000")
    }

    private static func createSchemaIfNeeded(_ connection: GaryxSQLiteConnection) throws {
        try connection.execute("BEGIN IMMEDIATE")
        var transactionOpen = true
        defer {
            if transactionOpen {
                try? connection.execute("ROLLBACK")
            }
        }
        try connection.execute(
            """
            CREATE TABLE IF NOT EXISTS \(metadataTableName) (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                schema_version INTEGER NOT NULL,
                revision INTEGER NOT NULL,
                generation_high_watermark INTEGER NOT NULL,
                reservation_high_watermark INTEGER NOT NULL,
                claimed_generations BLOB NOT NULL,
                tombstone_budget BLOB NOT NULL
            )
            """
        )
        for family in GaryxComposerDurabilityRecordFamily.allCases {
            try connection.execute(
                """
                CREATE TABLE IF NOT EXISTS \(family.rawValue) (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    payload BLOB NOT NULL
                )
                """
            )
        }
        let existingVersion = try connection.scalarInt64("PRAGMA user_version") ?? 0
        guard existingVersion == 0 || existingVersion == Int64(schemaVersion) else {
            throw GaryxSQLiteComposerDurabilityError.invalidMetadata(
                "unsupported composer durability schema version \(existingVersion)"
            )
        }
        try connection.execute("PRAGMA user_version=\(schemaVersion)")
        if try connection.scalarInt64(
            "SELECT COUNT(*) FROM \(metadataTableName) WHERE singleton = 1"
        ) == 0 {
            try persistInitialSnapshot(.init(), connection: connection)
        }
        try connection.execute("COMMIT")
        transactionOpen = false
    }

    private static func persistInitialSnapshot(
        _ snapshot: GaryxComposerDurabilitySnapshot,
        connection: GaryxSQLiteConnection
    ) throws {
        for family in GaryxComposerDurabilityRecordFamily.allCases {
            try connection.replaceSingletonBlob(
                table: family.rawValue,
                data: try encode(familyValue(family, from: snapshot))
            )
        }
        try persistMetadata(snapshot, connection: connection)
    }

    private static func persist(
        _ snapshot: GaryxComposerDurabilitySnapshot,
        connection: GaryxSQLiteConnection,
        boundaryHook: BoundaryHook
    ) throws {
        for family in GaryxComposerDurabilityRecordFamily.allCases {
            try connection.replaceSingletonBlob(
                table: family.rawValue,
                data: try encode(familyValue(family, from: snapshot))
            )
            try boundaryHook(.familyPersisted(family))
        }
        try persistMetadata(snapshot, connection: connection)
        try boundaryHook(.metadataPersisted)
    }

    private static func persistMetadata(
        _ snapshot: GaryxComposerDurabilitySnapshot,
        connection: GaryxSQLiteConnection
    ) throws {
        guard snapshot.revision <= UInt64(Int64.max),
              snapshot.generationHighWatermark <= UInt64(Int64.max),
              snapshot.reservationHighWatermark <= UInt64(Int64.max) else {
            throw GaryxSQLiteComposerDurabilityError.invalidMetadata(
                "durability counters exceed SQLite INTEGER range"
            )
        }
        try connection.replaceMetadata(
            schemaVersion: schemaVersion,
            revision: Int64(snapshot.revision),
            generationHighWatermark: Int64(snapshot.generationHighWatermark),
            reservationHighWatermark: Int64(snapshot.reservationHighWatermark),
            claimedGenerations: try encode(snapshot.claimedGenerations),
            tombstoneBudget: try encode(snapshot.tombstoneBudget)
        )
    }

    private static func readSnapshot(
        _ connection: GaryxSQLiteConnection
    ) throws -> GaryxComposerDurabilitySnapshot {
        let metadata = try connection.readMetadata(table: metadataTableName)
        guard metadata.schemaVersion == schemaVersion,
              metadata.revision >= 0,
              metadata.generationHighWatermark >= 0,
              metadata.reservationHighWatermark >= 0 else {
            throw GaryxSQLiteComposerDurabilityError.invalidMetadata(
                "negative counter or mismatched schema version"
            )
        }

        let payloadStore: GaryxComposerPayloadStore = try decodeFamily(.payload, connection)
        let aliases: GaryxComposerAliasTable = try decodeFamily(.aliases, connection)
        let operations: [GaryxOperationCapabilityKey: GaryxOperationCapability] =
            try decodeFamily(.operations, connection)
        let manifests: [GaryxOperationCapabilityKey: GaryxOperationManifest] =
            try decodeFamily(.manifests, connection)
        let replacements: [GaryxReplacementID: GaryxReplacementRecord] =
            try decodeFamily(.replacements, connection)
        let conflicts: [GaryxPayloadConflictSetID: GaryxPayloadConflictSet] =
            try decodeFamily(.conflicts, connection)
        let feedback: [GaryxFeedbackID: GaryxOperationFeedback] =
            try decodeFamily(.feedback, connection)
        let attachmentLineages: [
            GaryxAttachmentLineageID: GaryxAttachmentLineageTombstone
        ] = try decodeFamily(.attachmentLineages, connection)
        let barriers: [GaryxComposerPayloadEntryID: GaryxSendCommitBarrier] =
            try decodeFamily(.barriers, connection)
        let ledgers: [GaryxReservationLedgerKey: GaryxProvisionalReservationLedger] =
            try decodeFamily(.reservationLedgers, connection)
        let producerDrained: [
            GaryxSessionDescendantKey: GaryxDurableProducerDrainedRecord
        ] = try decodeFamily(.producerDrained, connection)
        let recoveredInputClosures: [
            GaryxSessionDescendantKey: GaryxRecoveredInputCloseRecord
        ] = try decodeFamily(.recoveredInputClosures, connection)
        let deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord] =
            try decodeFamily(.deliveryOutbox, connection)
        let discardConvergence: [
            GaryxComposerPayloadEntryID: GaryxPayloadDiscardConvergence
        ] = try decodeFamily(.discardConvergence, connection)
        let createDeliveries: [GaryxCreateDeliveryKey: GaryxCreateDeliveryState] =
            try decodeFamily(.createDeliveries, connection)
        let staged: GaryxStagedAssetDurabilityState =
            try decodeFamily(.stagedAssets, connection)
        let claimedGenerations: Set<UInt64> = try decode(
            metadata.claimedGenerations,
            label: "claimed generations"
        )
        let tombstoneBudget: GaryxPersistentTombstoneBudget = try decode(
            metadata.tombstoneBudget,
            label: "tombstone budget"
        )

        return GaryxComposerDurabilitySnapshot(
            revision: UInt64(metadata.revision),
            payloadStore: payloadStore,
            aliases: aliases,
            operations: operations,
            manifests: manifests,
            replacements: replacements,
            conflicts: conflicts,
            feedback: feedback,
            attachmentLineages: attachmentLineages,
            barriers: barriers,
            ledgers: ledgers,
            producerDrained: producerDrained,
            recoveredInputClosures: recoveredInputClosures,
            deliveries: deliveries,
            discardConvergence: discardConvergence,
            createDeliveries: createDeliveries,
            stagedAssetOwners: staged.owners,
            stagedAssetReservedBytes: staged.reservedBytesByAsset,
            pendingFileCleanup: staged.pendingFileCleanup,
            reservedBytes: staged.totalReservedBytes,
            generationHighWatermark: UInt64(metadata.generationHighWatermark),
            reservationHighWatermark: UInt64(metadata.reservationHighWatermark),
            claimedGenerations: claimedGenerations,
            tombstoneBudget: tombstoneBudget
        )
    }

    private static func decodeFamily<T: Decodable>(
        _ family: GaryxComposerDurabilityRecordFamily,
        _ connection: GaryxSQLiteConnection
    ) throws -> T {
        guard let data = try connection.singletonBlob(table: family.rawValue) else {
            throw GaryxSQLiteComposerDurabilityError.missingRecordFamily(family.rawValue)
        }
        return try decode(data, label: family.rawValue)
    }

    private static func familyValue(
        _ family: GaryxComposerDurabilityRecordFamily,
        from snapshot: GaryxComposerDurabilitySnapshot
    ) -> any Encodable {
        switch family {
        case .payload: snapshot.payloadStore
        case .aliases: snapshot.aliases
        case .operations: snapshot.operations
        case .manifests: snapshot.manifests
        case .replacements: snapshot.replacements
        case .conflicts: snapshot.conflicts
        case .feedback: snapshot.feedback
        case .attachmentLineages: snapshot.attachmentLineages
        case .barriers: snapshot.barriers
        case .reservationLedgers: snapshot.ledgers
        case .producerDrained: snapshot.producerDrained
        case .recoveredInputClosures: snapshot.recoveredInputClosures
        case .deliveryOutbox: snapshot.deliveries
        case .discardConvergence: snapshot.discardConvergence
        case .createDeliveries: snapshot.createDeliveries
        case .stagedAssets:
            GaryxStagedAssetDurabilityState(
                owners: snapshot.stagedAssetOwners,
                reservedBytesByAsset: snapshot.stagedAssetReservedBytes,
                pendingFileCleanup: snapshot.pendingFileCleanup,
                totalReservedBytes: snapshot.reservedBytes
            )
        }
    }

    private static func encode(_ value: any Encodable) throws -> Data {
        do {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            return try encoder.encode(value)
        } catch {
            throw GaryxSQLiteComposerDurabilityError.encoding(String(describing: error))
        }
    }

    private static func decode<T: Decodable>(_ data: Data, label: String) throws -> T {
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw GaryxSQLiteComposerDurabilityError.encoding(
                "\(label): \(String(describing: error))"
            )
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

    private static func protectDatabaseFiles(
        _ databaseURL: URL,
        fileProtectionPolicy: GaryxComposerFileProtectionPolicy
    ) throws {
        let paths = [
            databaseURL,
            URL(fileURLWithPath: databaseURL.path + "-wal"),
            URL(fileURLWithPath: databaseURL.path + "-shm"),
        ]
        for path in paths where FileManager.default.fileExists(atPath: path.path) {
            try FileManager.default.setAttributes(
                [.posixPermissions: 0o600],
                ofItemAtPath: path.path
            )
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            var mutablePath = path
            try mutablePath.setResourceValues(values)
            try fileProtectionPolicy.apply(to: path)
        }
    }
}

private struct GaryxStagedAssetDurabilityState: Codable {
    let owners: [GaryxStagedAssetID: GaryxOperationCapabilityKey]
    let reservedBytesByAsset: [GaryxStagedAssetID: Int]
    let pendingFileCleanup: [GaryxStagedAssetID: GaryxOperationCapabilityKey]
    let totalReservedBytes: Int

    init(
        owners: [GaryxStagedAssetID: GaryxOperationCapabilityKey] = [:],
        reservedBytesByAsset: [GaryxStagedAssetID: Int] = [:],
        pendingFileCleanup: [GaryxStagedAssetID: GaryxOperationCapabilityKey] = [:],
        totalReservedBytes: Int = 0
    ) {
        self.owners = owners
        self.reservedBytesByAsset = reservedBytesByAsset
        self.pendingFileCleanup = pendingFileCleanup
        self.totalReservedBytes = totalReservedBytes
    }
}

private struct GaryxSQLiteMetadataRow {
    let schemaVersion: Int
    let revision: Int64
    let generationHighWatermark: Int64
    let reservationHighWatermark: Int64
    let claimedGenerations: Data
    let tombstoneBudget: Data
}

private final class GaryxSQLiteConnection: @unchecked Sendable {
    private var handle: OpaquePointer?

    init(path: String) throws {
        var database: OpaquePointer?
        let code = sqlite3_open_v2(
            path,
            &database,
            SQLITE_OPEN_CREATE | SQLITE_OPEN_READWRITE | SQLITE_OPEN_FULLMUTEX,
            nil
        )
        guard code == SQLITE_OK, let database else {
            let message = database.flatMap(sqlite3_errmsg).map(String.init(cString:))
                ?? "unable to open SQLite database"
            if let database { sqlite3_close_v2(database) }
            throw GaryxSQLiteComposerDurabilityError.sqlite(code: code, message: message)
        }
        handle = database
        sqlite3_extended_result_codes(database, 1)
    }

    deinit {
        if let handle { sqlite3_close_v2(handle) }
    }

    func execute(_ sql: String) throws {
        guard let handle else {
            throw GaryxSQLiteComposerDurabilityError.invalidDatabasePath
        }
        var errorMessage: UnsafeMutablePointer<CChar>?
        let code = sqlite3_exec(handle, sql, nil, nil, &errorMessage)
        guard code == SQLITE_OK else {
            let message = errorMessage.map { String(cString: $0) }
                ?? String(cString: sqlite3_errmsg(handle))
            sqlite3_free(errorMessage)
            throw GaryxSQLiteComposerDurabilityError.sqlite(code: code, message: message)
        }
    }

    func scalarInt64(_ sql: String) throws -> Int64? {
        let statement = try prepare(sql)
        defer { sqlite3_finalize(statement) }
        switch sqlite3_step(statement) {
        case SQLITE_ROW:
            return sqlite3_column_int64(statement, 0)
        case SQLITE_DONE:
            return nil
        default:
            throw lastError()
        }
    }

    func replaceSingletonBlob(table: String, data: Data) throws {
        let statement = try prepare(
            "INSERT INTO \(table) (singleton, payload) VALUES (1, ?) "
                + "ON CONFLICT(singleton) DO UPDATE SET payload = excluded.payload"
        )
        defer { sqlite3_finalize(statement) }
        try bind(data, at: 1, to: statement)
        guard sqlite3_step(statement) == SQLITE_DONE else { throw lastError() }
    }

    func singletonBlob(table: String) throws -> Data? {
        let statement = try prepare("SELECT payload FROM \(table) WHERE singleton = 1")
        defer { sqlite3_finalize(statement) }
        switch sqlite3_step(statement) {
        case SQLITE_ROW:
            return data(from: statement, column: 0)
        case SQLITE_DONE:
            return nil
        default:
            throw lastError()
        }
    }

    func replaceMetadata(
        schemaVersion: Int,
        revision: Int64,
        generationHighWatermark: Int64,
        reservationHighWatermark: Int64,
        claimedGenerations: Data,
        tombstoneBudget: Data
    ) throws {
        let statement = try prepare(
            """
            INSERT INTO composer_durability_metadata (
                singleton, schema_version, revision, generation_high_watermark,
                reservation_high_watermark, claimed_generations, tombstone_budget
            ) VALUES (1, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(singleton) DO UPDATE SET
                schema_version = excluded.schema_version,
                revision = excluded.revision,
                generation_high_watermark = excluded.generation_high_watermark,
                reservation_high_watermark = excluded.reservation_high_watermark,
                claimed_generations = excluded.claimed_generations,
                tombstone_budget = excluded.tombstone_budget
            """
        )
        defer { sqlite3_finalize(statement) }
        sqlite3_bind_int64(statement, 1, Int64(schemaVersion))
        sqlite3_bind_int64(statement, 2, revision)
        sqlite3_bind_int64(statement, 3, generationHighWatermark)
        sqlite3_bind_int64(statement, 4, reservationHighWatermark)
        try bind(claimedGenerations, at: 5, to: statement)
        try bind(tombstoneBudget, at: 6, to: statement)
        guard sqlite3_step(statement) == SQLITE_DONE else { throw lastError() }
    }

    func readMetadata(table: String) throws -> GaryxSQLiteMetadataRow {
        let statement = try prepare(
            """
            SELECT schema_version, revision, generation_high_watermark,
                   reservation_high_watermark, claimed_generations, tombstone_budget
            FROM \(table) WHERE singleton = 1
            """
        )
        defer { sqlite3_finalize(statement) }
        guard sqlite3_step(statement) == SQLITE_ROW,
              let claimed = data(from: statement, column: 4),
              let budget = data(from: statement, column: 5) else {
            throw GaryxSQLiteComposerDurabilityError.invalidMetadata(
                "composer durability metadata row is missing"
            )
        }
        return GaryxSQLiteMetadataRow(
            schemaVersion: Int(sqlite3_column_int64(statement, 0)),
            revision: sqlite3_column_int64(statement, 1),
            generationHighWatermark: sqlite3_column_int64(statement, 2),
            reservationHighWatermark: sqlite3_column_int64(statement, 3),
            claimedGenerations: claimed,
            tombstoneBudget: budget
        )
    }

    private func prepare(_ sql: String) throws -> OpaquePointer {
        guard let handle else {
            throw GaryxSQLiteComposerDurabilityError.invalidDatabasePath
        }
        var statement: OpaquePointer?
        let code = sqlite3_prepare_v2(handle, sql, -1, &statement, nil)
        guard code == SQLITE_OK, let statement else { throw lastError(code: code) }
        return statement
    }

    private func bind(_ data: Data, at index: Int32, to statement: OpaquePointer) throws {
        let code = data.withUnsafeBytes { bytes in
            sqlite3_bind_blob(
                statement,
                index,
                bytes.baseAddress,
                Int32(bytes.count),
                unsafeBitCast(-1, to: sqlite3_destructor_type.self)
            )
        }
        guard code == SQLITE_OK else { throw lastError(code: code) }
    }

    private func data(from statement: OpaquePointer, column: Int32) -> Data? {
        let count = Int(sqlite3_column_bytes(statement, column))
        guard count >= 0 else { return nil }
        if count == 0 { return Data() }
        guard let bytes = sqlite3_column_blob(statement, column) else { return nil }
        return Data(bytes: bytes, count: count)
    }

    private func lastError(code: Int32? = nil) -> GaryxSQLiteComposerDurabilityError {
        guard let handle else { return .invalidDatabasePath }
        return .sqlite(
            code: code ?? sqlite3_extended_errcode(handle),
            message: String(cString: sqlite3_errmsg(handle))
        )
    }
}
