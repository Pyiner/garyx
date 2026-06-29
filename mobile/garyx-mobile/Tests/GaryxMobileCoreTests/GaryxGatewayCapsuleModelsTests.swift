import XCTest
@testable import GaryxMobileCore

final class GaryxGatewayCapsuleModelsTests: XCTestCase {
    func testCapsuleSummaryDecodesSnakeCaseGatewayShape() throws {
        let page = try JSONDecoder().decode(
            GaryxCapsulesPage.self,
            from: Data(
                """
                {
                  "capsules": [
                    {
                      "id": "01900000-0000-7000-8000-000000000001",
                      "title": "Synthetic Capsule",
                      "description": "A safe demo.",
                      "thread_id": "thread::capsule",
                      "run_id": "run-capsule",
                      "agent_id": "codex",
                      "provider_type": "codex_app_server",
                      "html_sha256": "abc123",
                      "byte_size": 42,
                      "revision": 3,
                      "created_at": "2026-06-28T10:00:00Z",
                      "updated_at": "2026-06-28T11:00:00Z"
                    }
                  ]
                }
                """.utf8
            )
        )

        let capsule = try XCTUnwrap(page.capsules.first)
        XCTAssertEqual(capsule.id, "01900000-0000-7000-8000-000000000001")
        XCTAssertEqual(capsule.title, "Synthetic Capsule")
        XCTAssertEqual(capsule.description, "A safe demo.")
        XCTAssertEqual(capsule.threadId, "thread::capsule")
        XCTAssertEqual(capsule.runId, "run-capsule")
        XCTAssertEqual(capsule.agentId, "codex")
        XCTAssertEqual(capsule.providerType, "codex_app_server")
        XCTAssertEqual(capsule.htmlSha256, "abc123")
        XCTAssertEqual(capsule.byteSize, 42)
        XCTAssertEqual(capsule.revision, 3)
        XCTAssertEqual(capsule.createdAt, "2026-06-28T10:00:00Z")
        XCTAssertEqual(capsule.updatedAt, "2026-06-28T11:00:00Z")
    }

    func testCapsuleSummaryDecodesCamelCaseGatewayShape() throws {
        let capsule = try JSONDecoder().decode(
            GaryxCapsuleSummary.self,
            from: Data(
                """
                {
                  "id": "01900000-0000-7000-8000-000000000002",
                  "title": "Camel Capsule",
                  "threadId": "thread::camel",
                  "runId": "run-camel",
                  "agentId": "claude",
                  "providerType": "claude_code",
                  "htmlSha256": "def456",
                  "byteSize": 100,
                  "revision": 2,
                  "createdAt": "2026-06-28T12:00:00Z",
                  "updatedAt": "2026-06-28T13:00:00Z"
                }
                """.utf8
            )
        )

        XCTAssertEqual(capsule.id, "01900000-0000-7000-8000-000000000002")
        XCTAssertEqual(capsule.description, "")
        XCTAssertEqual(capsule.threadId, "thread::camel")
        XCTAssertEqual(capsule.runId, "run-camel")
        XCTAssertEqual(capsule.agentId, "claude")
        XCTAssertEqual(capsule.providerType, "claude_code")
        XCTAssertEqual(capsule.htmlSha256, "def456")
        XCTAssertEqual(capsule.byteSize, 100)
        XCTAssertEqual(capsule.revision, 2)
    }

    func testCapsulesPanelPresentationMatchesTopLevelDrawerContract() {
        XCTAssertEqual(GaryxMobilePanel.capsules.label, "Capsules")
        XCTAssertEqual(GaryxMobilePanel.capsules.iconName, "capsule.fill")
    }

    // MARK: - GaryxCapsuleHTMLCacheKey (shared (id, revision) preview cache key)

    func testHTMLCacheKeyIsIdAndRevisionOnly() {
        // The chat-card wire carries no html_sha256; the key must be shared
        // across gallery/focused/chat, so the sha must not participate.
        let sameSha = GaryxCapsuleHTMLCacheKey(capsule: capsule(id: "c-1", revision: 1, htmlSha256: "aaa"))
        let diffSha = GaryxCapsuleHTMLCacheKey(capsule: capsule(id: "c-1", revision: 1, htmlSha256: "bbb"))
        let diffRev = GaryxCapsuleHTMLCacheKey(capsule: capsule(id: "c-1", revision: 2, htmlSha256: "aaa"))

        XCTAssertEqual(sameSha, diffSha, "differing sha at same (id, revision) must be the same cache entry")
        XCTAssertNotEqual(sameSha, diffRev, "revision must distinguish the cache key")
        XCTAssertEqual(sameSha.id, "c-1")
        XCTAssertEqual(sameSha.revision, 1)
        XCTAssertEqual(GaryxCapsuleHTMLCacheKey(id: "  c-1  ", revision: 1), sameSha, "id is trimmed")
    }

    func testHTMLCachePrunerEvictsDeletedAndSupersededRevisions() {
        let cache: [GaryxCapsuleHTMLCacheKey: String] = [
            GaryxCapsuleHTMLCacheKey(id: "live", revision: 2): "<live/>",
            GaryxCapsuleHTMLCacheKey(id: "live", revision: 1): "<old/>",   // superseded revision
            GaryxCapsuleHTMLCacheKey(id: "gone", revision: 1): "<gone/>",  // deleted capsule
        ]
        let result = GaryxCapsuleHTMLCachePruner.pruned(
            cache: cache,
            validCapsules: [capsule(id: "live", revision: 2, htmlSha256: "x")]
        )
        XCTAssertTrue(result.didEvict)
        XCTAssertEqual(Set(result.cache.keys), [GaryxCapsuleHTMLCacheKey(id: "live", revision: 2)])
    }

    func testHTMLCachePrunerReportsNoEvictionWhenNothingChanges() {
        let cache = [GaryxCapsuleHTMLCacheKey(id: "live", revision: 2): "<live/>"]
        let result = GaryxCapsuleHTMLCachePruner.pruned(
            cache: cache,
            validCapsules: [capsule(id: "live", revision: 2, htmlSha256: "x")]
        )
        XCTAssertFalse(result.didEvict)
        XCTAssertEqual(result.cache, cache)
    }

    private func capsule(id: String, revision: Int, htmlSha256: String) -> GaryxCapsuleSummary {
        GaryxCapsuleSummary(id: id, title: "Capsule", htmlSha256: htmlSha256, byteSize: 10, revision: revision)
    }
}
