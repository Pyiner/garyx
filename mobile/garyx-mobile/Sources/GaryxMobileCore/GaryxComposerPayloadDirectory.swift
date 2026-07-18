import Foundation

public struct GaryxComposerPayloadProjection: Equatable, Sendable {
    public let scope: GaryxGatewayScope
    public let key: GaryxComposerKey
    public let entryID: GaryxComposerPayloadEntryID
    public let lifecycle: GaryxPayloadLifecycleSnapshot
    public let generation: UInt64
    public let text: String
    public let attachments: [GaryxComposerAttachment]
    public let readiness: GaryxComposerSendReadiness

    public init(
        scope: GaryxGatewayScope,
        key: GaryxComposerKey,
        entryID: GaryxComposerPayloadEntryID,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        generation: UInt64,
        text: String,
        attachments: [GaryxComposerAttachment],
        readiness: GaryxComposerSendReadiness
    ) {
        self.scope = scope
        self.key = key
        self.entryID = entryID
        self.lifecycle = lifecycle
        self.generation = generation
        self.text = text
        self.attachments = attachments
        self.readiness = readiness
    }
}

public enum GaryxComposerPayloadActivationDisposition: Equatable, Sendable {
    case restored(GaryxComposerPayloadEntryID)
    case created(GaryxComposerPayloadEntryID)
    case destinationCollision([GaryxComposerPayloadEntryID])
}

/// Scope-partitioned, stable-Entry directory used by the real composer host.
/// Active selection is presentation-only and is rebuilt from the route top;
/// the payload values themselves remain in the durable store when a key or
/// gateway partition is no longer visible.
public struct GaryxComposerPayloadDirectory: Equatable, Sendable {
    public private(set) var store: GaryxComposerPayloadStore
    public private(set) var activeScope: GaryxGatewayScope?
    public private(set) var activeKey: GaryxComposerKey?
    public private(set) var activeEntryID: GaryxComposerPayloadEntryID?

    public init(store: GaryxComposerPayloadStore = .init()) {
        self.store = store
        activeScope = nil
        activeKey = nil
        activeEntryID = nil
    }

    public func projection(
        scope: GaryxGatewayScope,
        key: GaryxComposerKey,
        operations: [GaryxOperationCapabilityKey: GaryxOperationCapability] = [:]
    ) -> GaryxComposerPayloadProjection? {
        let matches = entries(scope: scope, key: key)
        guard matches.count == 1, let entry = matches.first else { return nil }
        return Self.projection(entry: entry, operations: operations)
    }

    public var activeProjection: GaryxComposerPayloadProjection? {
        guard let activeScope,
              let activeKey,
              let activeEntryID,
              let entry = store.entry(activeEntryID, scope: activeScope),
              entry.destination == activeKey else {
            return nil
        }
        return Self.projection(entry: entry, operations: [:])
    }

    @discardableResult
    public mutating func activate(
        scope: GaryxGatewayScope,
        key: GaryxComposerKey,
        creating entryID: GaryxComposerPayloadEntryID,
        generation: UInt64,
        lifecycleNonce: String
    ) -> GaryxComposerPayloadActivationDisposition {
        let matches = entries(scope: scope, key: key)
        guard matches.count <= 1 else {
            return .destinationCollision(matches.map(\.id).sorted {
                $0.rawValue < $1.rawValue
            })
        }
        if let existing = matches.first {
            activeScope = scope
            activeKey = key
            activeEntryID = existing.id
            return .restored(existing.id)
        }

        let entry = GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: key,
            lifecycleToken: GaryxPayloadLifecycleToken(
                entryID: entryID,
                nonce: lifecycleNonce
            ),
            currentGeneration: generation
        )
        precondition(store.insert(entry), "fresh composer Entry identity collided")
        activeScope = scope
        activeKey = key
        activeEntryID = entryID
        return .created(entryID)
    }

    public mutating func suspendPresentation() {
        activeScope = nil
        activeKey = nil
        activeEntryID = nil
    }

    @discardableResult
    public mutating func updateActiveText(_ text: String, generation: UInt64) -> Bool {
        guard var entry = activeEntry(), generation == entry.currentGeneration else {
            return false
        }
        entry.setText(text, generation: generation)
        store.update(entry)
        return true
    }

    @discardableResult
    public mutating func addActiveAttachment(_ attachment: GaryxComposerAttachment) -> Bool {
        guard var entry = activeEntry(), attachment.generation == entry.currentGeneration else {
            return false
        }
        entry.addAttachment(attachment)
        store.update(entry)
        return true
    }

    @discardableResult
    public mutating func removeActiveAttachment(_ id: GaryxAttachmentID) -> Bool {
        guard var entry = activeEntry(), entry.attachments[id] != nil else { return false }
        entry.removeAttachment(id)
        store.update(entry)
        return true
    }

    @discardableResult
    public mutating func beginFreshActiveGeneration(_ generation: UInt64) -> Bool {
        guard var entry = activeEntry(), entry.beginFreshGeneration(generation) else {
            return false
        }
        store.update(entry)
        return true
    }

    @discardableResult
    public mutating func promoteActive(to destination: GaryxComposerKey) -> Bool {
        guard let activeScope, let activeEntryID,
              store.promote(entryID: activeEntryID, scope: activeScope, to: destination) else {
            return false
        }
        activeKey = destination
        return true
    }

    public mutating func replaceStore(_ replacement: GaryxComposerPayloadStore) {
        store = replacement
        guard let activeScope, let activeKey,
              entries(scope: activeScope, key: activeKey).count == 1,
              let restored = entries(scope: activeScope, key: activeKey).first else {
            suspendPresentation()
            return
        }
        activeEntryID = restored.id
    }

    private func activeEntry() -> GaryxComposerPayloadEntry? {
        guard let activeScope, let activeEntryID else { return nil }
        return store.entry(activeEntryID, scope: activeScope)
    }

    private func entries(
        scope: GaryxGatewayScope,
        key: GaryxComposerKey
    ) -> [GaryxComposerPayloadEntry] {
        store.entriesByScope[scope]?.values.filter { $0.destination == key } ?? []
    }

    private static func projection(
        entry: GaryxComposerPayloadEntry,
        operations: [GaryxOperationCapabilityKey: GaryxOperationCapability]
    ) -> GaryxComposerPayloadProjection {
        let entryOperations = entry.operationKeys.compactMap { operations[$0] }
        return GaryxComposerPayloadProjection(
            scope: entry.scope,
            key: entry.destination,
            entryID: entry.id,
            lifecycle: entry.lifecycle.snapshot,
            generation: entry.currentGeneration,
            text: entry.currentText,
            attachments: entry.attachments.values.sorted { $0.id.rawValue < $1.id.rawValue },
            readiness: GaryxComposerSendReadinessPolicy.evaluate(entryOperations)
        )
    }
}
