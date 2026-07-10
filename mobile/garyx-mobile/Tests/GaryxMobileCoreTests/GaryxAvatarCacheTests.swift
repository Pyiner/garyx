import XCTest
@testable import GaryxMobileCore

final class GaryxAvatarCacheTests: XCTestCase {
    private let pngDataURL = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII="
    private let secondPngDataURL = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADUlEQVR42mP8z8BQDwAFgwJ/lC3h7wAAAABJRU5ErkJggg=="

    func testDataURLParsingAndFingerprintAreStableAcrossMimeAndWhitespace() throws {
        let payload = try XCTUnwrap(GaryxAvatarDataURLParser.parse(pngDataURL))
        XCTAssertEqual(payload.mediaType, "image/png")
        XCTAssertFalse(payload.data.isEmpty)
        XCTAssertEqual(payload.contentFingerprint, GaryxAvatarFingerprint.contentFingerprint(for: payload.data))

        let encoded = pngDataURL.split(separator: ",", maxSplits: 1).last.map(String.init) ?? ""
        let noisy = "  data:IMAGE/PNG;charset=utf-8;BASE64,\n\(encoded.prefix(20)) \n\(encoded.dropFirst(20))  "
        let noisyPayload = try XCTUnwrap(GaryxAvatarDataURLParser.parse(noisy))
        XCTAssertEqual(noisyPayload.data, payload.data)
        XCTAssertEqual(noisyPayload.contentFingerprint, payload.contentFingerprint)
    }

    func testDataURLParsingRejectsNonPersistentInputs() {
        XCTAssertNil(GaryxAvatarDataURLParser.parse(""))
        XCTAssertNil(GaryxAvatarDataURLParser.parse("https://example.test/avatar.png"))
        XCTAssertNil(GaryxAvatarDataURLParser.parse("data:text/plain;base64,SGVsbG8="))
        XCTAssertNil(GaryxAvatarDataURLParser.parse("data:image/png;base64,not-valid-@@@"))

        let oversized = Data(repeating: 1, count: 513 * 1024).base64EncodedString()
        XCTAssertNil(GaryxAvatarDataURLParser.parse("data:image/png;base64,\(oversized)"))
    }

    func testResolutionPriorityUsesLiveThenStoredThenPlaceholder() throws {
        let identity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-1")
        let live = try XCTUnwrap(GaryxAvatarDataURLParser.parse(pngDataURL))
        let storedPayload = try XCTUnwrap(GaryxAvatarDataURLParser.parse(secondPngDataURL))
        let stored = GaryxStoredAvatar(
            record: GaryxAvatarStoreEntry(
                identity: identity,
                fingerprint: storedPayload.contentFingerprint,
                fileName: identity.blobFileName,
                mediaType: storedPayload.mediaType,
                byteCount: storedPayload.data.count,
                updatedAt: Date(timeIntervalSince1970: 10),
                lastAccessAt: Date(timeIntervalSince1970: 10)
            ),
            payload: storedPayload
        )

        XCTAssertEqual(GaryxAvatarResolution.resolve(live: live, stored: stored), .live(live))
        XCTAssertEqual(GaryxAvatarResolution.resolve(live: nil, stored: stored), .stored(stored))
        XCTAssertEqual(GaryxAvatarResolution.resolve(live: nil, stored: nil), .placeholder)
    }

    func testWriteThroughPlanWritesOnlyValidChangedAvatars() throws {
        let identity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-1")
        let existing = try XCTUnwrap(GaryxAvatarDataURLParser.parse(pngDataURL))
        let changed = try XCTUnwrap(GaryxAvatarDataURLParser.parse(secondPngDataURL))
        let current = [identity.storageKey: existing.contentFingerprint]

        let planned = GaryxAvatarWriteThroughPlan.upserts(
            incoming: [
                GaryxAvatarUpsert(identity: identity, dataUrl: pngDataURL),
                GaryxAvatarUpsert(identity: GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-2"), dataUrl: secondPngDataURL),
                GaryxAvatarUpsert(identity: GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-3"), dataUrl: ""),
                GaryxAvatarUpsert(identity: GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-4"), dataUrl: "data:text/plain;base64,SGVsbG8="),
            ],
            currentFingerprints: current,
            validator: GaryxAvatarAlwaysValidImageValidator()
        )

        XCTAssertEqual(planned.map(\.identity.id), ["agent-test-2"])
        XCTAssertEqual(planned.first?.payload.contentFingerprint, changed.contentFingerprint)
    }

    func testValidatorDefinesLiveValidityBoundary() {
        let identity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-1")
        let planned = GaryxAvatarWriteThroughPlan.upserts(
            incoming: [GaryxAvatarUpsert(identity: identity, dataUrl: pngDataURL)],
            currentFingerprints: [:],
            validator: GaryxAvatarNeverValidImageValidator()
        )
        XCTAssertEqual(planned, [])
    }

    func testInMemoryStoreIsMembershipIndependentAndNoTombstoneOnEmptyRefresh() async throws {
        let store = GaryxInMemoryAvatarStore()
        let identity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-1")
        let writeResult = await store.upsert(
            [GaryxAvatarUpsert(identity: identity, dataUrl: pngDataURL)],
            validator: GaryxAvatarAlwaysValidImageValidator(),
            now: Date(timeIntervalSince1970: 10)
        )
        XCTAssertEqual(writeResult.written, 1)
        let storedAfterWrite = await store.storedAvatar(for: identity, now: Date(timeIntervalSince1970: 11))
        XCTAssertNotNil(storedAfterWrite)

        let emptyRefresh = await store.upsert(
            [GaryxAvatarUpsert(identity: identity, dataUrl: "")],
            validator: GaryxAvatarAlwaysValidImageValidator(),
            now: Date(timeIntervalSince1970: 12)
        )
        XCTAssertEqual(emptyRefresh.written, 0)
        XCTAssertEqual(emptyRefresh.rejected, 1)
        let storedAfterEmptyRefresh = await store.storedAvatar(for: identity, now: Date(timeIntervalSince1970: 13))
        XCTAssertNotNil(storedAfterEmptyRefresh)

        await store.upsert([], validator: GaryxAvatarAlwaysValidImageValidator(), now: Date(timeIntervalSince1970: 14))
        let storedAfterAbsentRefresh = await store.storedAvatar(for: identity, now: Date(timeIntervalSince1970: 15))
        XCTAssertNotNil(storedAfterAbsentRefresh)
    }

    func testExplicitClearAndDeleteRemoveRecords() async {
        let store = GaryxInMemoryAvatarStore()
        let identity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-1")
        await store.upsert(
            [GaryxAvatarUpsert(identity: identity, dataUrl: pngDataURL)],
            validator: GaryxAvatarAlwaysValidImageValidator(),
            now: Date(timeIntervalSince1970: 10)
        )
        let storedBeforeDelete = await store.storedAvatar(for: identity, now: Date(timeIntervalSince1970: 11))
        XCTAssertNotNil(storedBeforeDelete)

        await store.remove(identity)
        let storedAfterDelete = await store.storedAvatar(for: identity, now: Date(timeIntervalSince1970: 12))
        XCTAssertNil(storedAfterDelete)
    }

    func testIdChangeStoresNewIdentityThenRemovesOldWithoutMigratingBytes() async throws {
        let store = GaryxInMemoryAvatarStore()
        let oldIdentity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-old")
        let newIdentity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-new")
        await store.upsert(
            [GaryxAvatarUpsert(identity: oldIdentity, dataUrl: pngDataURL)],
            validator: GaryxAvatarAlwaysValidImageValidator(),
            now: Date(timeIntervalSince1970: 10)
        )
        await store.upsert(
            [GaryxAvatarUpsert(identity: newIdentity, dataUrl: secondPngDataURL)],
            validator: GaryxAvatarAlwaysValidImageValidator(),
            now: Date(timeIntervalSince1970: 11)
        )
        await store.remove(oldIdentity)

        let storedOld = await store.storedAvatar(for: oldIdentity, now: Date(timeIntervalSince1970: 12))
        XCTAssertNil(storedOld)
        let storedNew = await store.storedAvatar(for: newIdentity, now: Date(timeIntervalSince1970: 13))
        let stored = try XCTUnwrap(storedNew)
        let expected = try XCTUnwrap(GaryxAvatarDataURLParser.parse(secondPngDataURL))
        XCTAssertEqual(stored.payload.contentFingerprint, expected.contentFingerprint)
    }

    func testScopeIsolation() async throws {
        let store = GaryxInMemoryAvatarStore()
        let agentA = GaryxAvatarIdentity(scope: "gateway-a", id: "shared-id")
        let agentB = GaryxAvatarIdentity(scope: "gateway-b", id: "shared-id")
        await store.upsert(
            [
                GaryxAvatarUpsert(identity: agentA, dataUrl: pngDataURL),
                GaryxAvatarUpsert(identity: agentB, dataUrl: secondPngDataURL),
            ],
            validator: GaryxAvatarAlwaysValidImageValidator(),
            now: Date(timeIntervalSince1970: 10)
        )

        let maybeStoredAgentA = await store.storedAvatar(for: agentA, now: Date(timeIntervalSince1970: 11))
        let maybeStoredAgentB = await store.storedAvatar(for: agentB, now: Date(timeIntervalSince1970: 12))
        let storedAgentA = try XCTUnwrap(maybeStoredAgentA)
        let storedAgentB = try XCTUnwrap(maybeStoredAgentB)
        XCTAssertNotEqual(storedAgentA.payload.contentFingerprint, storedAgentB.payload.contentFingerprint)
        XCTAssertNotEqual(agentA.storageKey, agentB.storageKey)
    }

    func testLRUPruneUsesLastAccessNotMembership() async {
        let store = GaryxInMemoryAvatarStore()
        let first = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-1")
        let second = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-2")
        let third = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-3")
        await store.upsert([GaryxAvatarUpsert(identity: first, dataUrl: pngDataURL)], validator: GaryxAvatarAlwaysValidImageValidator(), now: Date(timeIntervalSince1970: 1))
        await store.upsert([GaryxAvatarUpsert(identity: second, dataUrl: pngDataURL)], validator: GaryxAvatarAlwaysValidImageValidator(), now: Date(timeIntervalSince1970: 2))
        _ = await store.storedAvatar(for: first, now: Date(timeIntervalSince1970: 10))
        await store.upsert([GaryxAvatarUpsert(identity: third, dataUrl: pngDataURL)], validator: GaryxAvatarAlwaysValidImageValidator(), now: Date(timeIntervalSince1970: 11))
        await store.prune(
            policy: GaryxAvatarPruningPolicy(maxRecords: 2, maxBytes: 16 * 1024 * 1024, maxRecordBytes: 512 * 1024),
            now: Date(timeIntervalSince1970: 12)
        )

        let storedFirst = await store.storedAvatar(for: first, now: Date(timeIntervalSince1970: 13))
        let storedSecond = await store.storedAvatar(for: second, now: Date(timeIntervalSince1970: 14))
        let storedThird = await store.storedAvatar(for: third, now: Date(timeIntervalSince1970: 15))
        XCTAssertNotNil(storedFirst)
        XCTAssertNil(storedSecond)
        XCTAssertNotNil(storedThird)
    }

    func testIndexRoundTripAndVersionGuard() throws {
        let identity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent/test:1")
        let entry = GaryxAvatarStoreEntry(
            identity: identity,
            fingerprint: "fnv1a64:1234",
            fileName: identity.blobFileName,
            mediaType: "image/png",
            byteCount: 4,
            updatedAt: Date(timeIntervalSince1970: 10),
            lastAccessAt: Date(timeIntervalSince1970: 11)
        )
        let index = GaryxAvatarStoreIndex(entries: [identity.storageKey: entry])
        let data = try JSONEncoder().encode(index)
        XCTAssertEqual(GaryxAvatarStoreIndex.decodeCurrent(from: data), index)

        let stale = GaryxAvatarStoreIndex(version: 0, entries: [identity.storageKey: entry])
        let staleData = try JSONEncoder().encode(stale)
        XCTAssertNil(GaryxAvatarStoreIndex.decodeCurrent(from: staleData))
        XCTAssertFalse(identity.blobFileName.contains("/"))
        XCTAssertFalse(identity.blobFileName.contains(":"))
    }

    func testWidgetProjectionUsesInjectedAvatarReferenceAndKeepsEmptyMapBehavior() {
        let thread = GaryxThreadSummary(
            id: "thread-test-1",
            title: "Synthetic Thread",
            createdAt: "2026-01-01T00:00:00Z",
            updatedAt: "2026-01-01T01:00:00Z",
            lastMessagePreview: "",
            workspacePath: "/Users/test/project",
            messageCount: 1,
            agentId: "agent-test-1",
            providerType: "codex",
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
        let input = GaryxRecentThreadsWidgetSnapshotInput(
            threads: [thread],
            agents: [],
            pinnedThreadIds: [],
            recentThreadIds: ["thread-test-1"],
            gatewayScopeId: "gateway-a"
        )
        let identity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-1")

        let withoutFallback = GaryxRecentThreadsWidgetSnapshotProjector.widgetThreads(from: input)
        XCTAssertEqual(withoutFallback.first?.agentId, "agent-test-1")
        XCTAssertNil(withoutFallback.first?.avatarDataUrl)
        XCTAssertNil(withoutFallback.first?.avatarScope)
        XCTAssertNil(withoutFallback.first?.avatarFingerprint)

        let withFallback = GaryxRecentThreadsWidgetSnapshotProjector.widgetThreads(
            from: input,
            avatarFallback: [identity: "fnv1a64:1234"]
        )
        XCTAssertNil(withFallback.first?.avatarDataUrl)
        XCTAssertEqual(withFallback.first?.avatarScope, "gateway-a")
        XCTAssertEqual(withFallback.first?.avatarFingerprint, "fnv1a64:1234")
        XCTAssertEqual(GaryxRecentThreadsWidgetSnapshotProjector.avatarIdentities(from: input), [identity])
    }

    func testWidgetProjectionStopsPersistingFreshBase64WhenScopeIsKnown() {
        let thread = GaryxThreadSummary(
            id: "thread-test-1",
            title: "Synthetic Thread",
            createdAt: "2026-01-01T00:00:00Z",
            updatedAt: "2026-01-01T01:00:00Z",
            lastMessagePreview: "",
            workspacePath: "/Users/test/project",
            messageCount: 1,
            agentId: "agent-test-1",
            providerType: "codex",
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
        let agent = GaryxAgentSummary(
            id: "agent-test-1",
            displayName: "Test Agent",
            providerType: "codex",
            model: "test-model",
            avatarDataUrl: pngDataURL
        )
        let input = GaryxRecentThreadsWidgetSnapshotInput(
            threads: [thread],
            agents: [agent],
            pinnedThreadIds: [],
            recentThreadIds: ["thread-test-1"],
            gatewayScopeId: "gateway-a"
        )
        let identity = GaryxAvatarIdentity(scope: "gateway-a", id: "agent-test-1")

        let projected = GaryxRecentThreadsWidgetSnapshotProjector.widgetThreads(
            from: input,
            avatarFallback: [identity: "fnv1a64:1234"]
        )

        XCTAssertNil(projected.first?.avatarDataUrl)
        XCTAssertEqual(projected.first?.avatarScope, "gateway-a")
        XCTAssertEqual(projected.first?.avatarFingerprint, "fnv1a64:1234")
    }
}
