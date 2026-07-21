import XCTest
@testable import GaryxMobileCore

final class GaryxWorkspaceManagementCoreTests: XCTestCase {
    // MARK: - Catalog model decoding (server wire shapes)

    func testWorkspacesPageDecodesFullServerShapeVerbatim() throws {
        let json = """
        {
            "workspace_state_initialized": true,
            "gateway_home": "/Users/test",
            "workspaces": [
                {
                    "name": "garyx",
                    "path": "/Users/test/repos/garyx",
                    "pinned": true,
                    "thread_count": 42,
                    "last_activity_at": "2026-07-20T16:44:00Z",
                    "git_repo": true
                },
                {
                    "name": "notes",
                    "path": "/Users/test/notes",
                    "pinned": false,
                    "thread_count": 0,
                    "last_activity_at": null,
                    "git_repo": false
                }
            ]
        }
        """
        let page = try JSONDecoder().decode(GaryxWorkspacesPage.self, from: Data(json.utf8))
        XCTAssertEqual(page.gatewayHome, "/Users/test")
        XCTAssertTrue(page.workspaceStateInitialized)
        XCTAssertEqual(page.workspaces.count, 2)
        let first = page.workspaces[0]
        XCTAssertEqual(first.name, "garyx")
        XCTAssertEqual(first.path, "/Users/test/repos/garyx")
        XCTAssertTrue(first.pinned)
        XCTAssertEqual(first.threadCount, 42)
        XCTAssertEqual(first.lastActivityAt, "2026-07-20T16:44:00Z")
        XCTAssertTrue(first.gitRepo)
        let second = page.workspaces[1]
        XCTAssertFalse(second.pinned)
        XCTAssertEqual(second.threadCount, 0)
        XCTAssertNil(second.lastActivityAt)
        XCTAssertFalse(second.gitRepo)
        // Server order is delivered verbatim; the catalog never re-sorts.
        XCTAssertEqual(
            GaryxWorkspaceCatalog(page: page).paths,
            ["/Users/test/repos/garyx", "/Users/test/notes"]
        )
    }

    func testDirectoryEntryDecodesCamelCaseGitRepo() throws {
        let json = """
        {
            "path": "/Users/test",
            "parentPath": "/Users",
            "entries": [
                {"name": "repos", "path": "/Users/test/repos", "gitRepo": false},
                {"name": "garyx", "path": "/Users/test/garyx", "gitRepo": true}
            ]
        }
        """
        let listing = try JSONDecoder().decode(
            GaryxWorkspaceDirectoryListing.self, from: Data(json.utf8)
        )
        XCTAssertEqual(listing.path, "/Users/test")
        XCTAssertEqual(listing.parentPath, "/Users")
        XCTAssertEqual(listing.entries.map(\.gitRepo), [false, true])
    }

    func testDirectoryTypedErrorDecodesOnlyKnownFourHundredCodes() {
        let body = Data(#"{"error": "path must be absolute", "code": "invalid_path"}"#.utf8)
        let typed = GaryxWorkspaceDirectoryError.decode(statusCode: 400, body: body)
        XCTAssertEqual(typed?.code, .invalidPath)
        XCTAssertEqual(typed?.message, "path must be absolute")

        for code in ["not_found", "not_a_directory", "permission_denied"] {
            let data = Data(#"{"error": "x", "code": "\#(code)"}"#.utf8)
            XCTAssertNotNil(
                GaryxWorkspaceDirectoryError.decode(statusCode: 400, body: data),
                code
            )
        }

        // Non-400s and unknown codes are transport failures, not typed errors.
        XCTAssertNil(GaryxWorkspaceDirectoryError.decode(statusCode: 500, body: body))
        XCTAssertNil(
            GaryxWorkspaceDirectoryError.decode(
                statusCode: 400,
                body: Data(#"{"error": "x", "code": "mystery"}"#.utf8)
            )
        )
        XCTAssertNil(GaryxWorkspaceDirectoryError.decode(statusCode: 400, body: Data("nonsense".utf8)))
    }

    func testThreadSummaryDecodesMembershipAndProvenance() throws {
        let json = """
        {
            "thread_id": "thread::wt",
            "title": "Worktree thread",
            "workspace_dir": "/workspace/root-worktrees/wt1",
            "root_workspace_path": "/workspace/root",
            "workspace_origin": "explicit",
            "worktree": {"path": "/workspace/root-worktrees/wt1"}
        }
        """
        let summary = try JSONDecoder().decode(GaryxThreadSummary.self, from: Data(json.utf8))
        XCTAssertEqual(summary.rootWorkspacePath, "/workspace/root")
        XCTAssertEqual(summary.workspaceOrigin, "explicit")
        XCTAssertEqual(summary.workspacePath, "/workspace/root-worktrees/wt1")
    }

    // MARK: - Draft tri-state

    private let catalog = GaryxWorkspaceCatalog(
        gatewayHome: "/Users/test",
        workspaces: [
            GaryxWorkspaceSummary(name: "pinned-first", path: "/w/pinned", pinned: true),
            GaryxWorkspaceSummary(name: "active", path: "/w/active"),
        ]
    )

    func testUnresolvedDraftResolvesOnceToFirstServerRow() {
        let resolved = GaryxDraftWorkspaceSelection.unresolved
            .resolved(against: catalog, catalogLoaded: true)
        XCTAssertEqual(resolved, .path("/w/pinned"))
    }

    func testUnresolvedDraftWaitsForCatalogArrival() {
        let pending = GaryxDraftWorkspaceSelection.unresolved
            .resolved(against: .empty, catalogLoaded: false)
        XCTAssertEqual(pending, .unresolved)
    }

    func testEmptyLoadedCatalogResolvesToExplicitNone() {
        let resolved = GaryxDraftWorkspaceSelection.unresolved
            .resolved(against: .empty, catalogLoaded: true)
        XCTAssertEqual(resolved, GaryxDraftWorkspaceSelection.none)
    }

    func testExplicitNoneIsNeverOverridden() {
        let resolved = GaryxDraftWorkspaceSelection.none
            .resolved(against: catalog, catalogLoaded: true)
        XCTAssertEqual(resolved, GaryxDraftWorkspaceSelection.none)
    }

    func testResolvedPathNeverDriftsOnRefresh() {
        let selection = GaryxDraftWorkspaceSelection.path("/w/active")
        XCTAssertEqual(selection.resolved(against: catalog, catalogLoaded: true), selection)
    }

    func testChosenPathIsNeverAutoReplaced() {
        // Catalog membership is not a validity test: an agent default
        // directory or a freshly removed workspace stays selected and the
        // picker presents it as the "Current" row.
        let selection = GaryxDraftWorkspaceSelection.path("/w/not-in-catalog")
        XCTAssertEqual(selection.resolved(against: catalog, catalogLoaded: true), selection)
        XCTAssertEqual(selection.resolved(against: .empty, catalogLoaded: true), selection)
        XCTAssertEqual(selection.resolved(against: .empty, catalogLoaded: false), selection)
    }

    func testCreatePayloadMapping() {
        XCTAssertEqual(GaryxDraftWorkspaceSelection.path("/w/a").createPayloadWorkspaceDir, "/w/a")
        XCTAssertNil(GaryxDraftWorkspaceSelection.none.createPayloadWorkspaceDir)
        XCTAssertNil(GaryxDraftWorkspaceSelection.unresolved.createPayloadWorkspaceDir)
        // Only the explicit choice raises the wire bit — an unresolved draft
        // still lets the agent default substitute.
        XCTAssertTrue(GaryxDraftWorkspaceSelection.none.isExplicitNoWorkspace)
        XCTAssertFalse(GaryxDraftWorkspaceSelection.unresolved.isExplicitNoWorkspace)
        XCTAssertFalse(GaryxDraftWorkspaceSelection.path("/w/a").isExplicitNoWorkspace)
    }

    func testCreateThreadRequestEncodesExplicitNoWorkspaceBit() throws {
        // Explicit No workspace: no workspaceDir, noWorkspace=true so the
        // gateway provisions the managed directory instead of the agent
        // default substituting.
        let noWorkspace = GaryxCreateThreadRequest(noWorkspace: true, agentId: "reviewer")
        let encoded = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(noWorkspace)
        ) as? [String: Any]
        XCTAssertEqual(encoded?["noWorkspace"] as? Bool, true)
        XCTAssertNil(encoded?["workspaceDir"])

        // No explicit choice: the bit is omitted entirely.
        let unresolved = GaryxCreateThreadRequest(agentId: "reviewer")
        let unresolvedEncoded = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(unresolved)
        ) as? [String: Any]
        XCTAssertNil(unresolvedEncoded?["noWorkspace"])

        // A chosen path never raises the bit.
        let picked = GaryxCreateThreadRequest(workspaceDir: "/w/a")
        let pickedEncoded = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(picked)
        ) as? [String: Any]
        XCTAssertEqual(pickedEncoded?["workspaceDir"] as? String, "/w/a")
        XCTAssertNil(pickedEncoded?["noWorkspace"])
    }

    func testThreadSummariesPageStrictRowsCarryProvenance() throws {
        // The strict summaries-page adapter (production list path) must not
        // drop the membership/provenance fields.
        let json = """
        {
            "store_incarnation_id": "inc-1",
            "server_boot_id": "boot-1",
            "threads": [
                {
                    "thread_id": "thread::wt",
                    "title": "Worktree thread",
                    "workspace_dir": "/workspace/root-worktrees/wt1",
                    "thread_type": "chat",
                    "message_count": 2,
                    "root_workspace_path": "/workspace/root",
                    "workspace_origin": "explicit",
                    "worktree": {"path": "/workspace/root-worktrees/wt1"}
                }
            ],
            "has_more": false,
            "next_cursor": null
        }
        """
        let page = try JSONDecoder().decode(GaryxThreadSummariesPage.self, from: Data(json.utf8))
        let summary = try XCTUnwrap(page.threads.first)
        XCTAssertEqual(summary.rootWorkspacePath, "/workspace/root")
        XCTAssertEqual(summary.workspaceOrigin, "explicit")
        XCTAssertEqual(summary.workspacePath, "/workspace/root-worktrees/wt1")
        XCTAssertEqual(summary.worktreePath, "/workspace/root-worktrees/wt1")
    }

    func testLegacyThreadRecordAdapterCarriesProvenance() throws {
        // The point-read adapter (GET /api/threads/:id) must surface the
        // server-owned provenance for the thread settings panel.
        let json = """
        {
            "thread_id": "thread::implicit",
            "label": "Implicit thread",
            "workspace_dir": "/data/thread-workspaces/thread--implicit",
            "root_workspace_path": null,
            "workspace_origin": "implicit"
        }
        """
        let record = try JSONDecoder().decode(GaryxLegacyThreadRecordDTO.self, from: Data(json.utf8))
        let summary = GaryxThreadSummaryAdapter.summary(record)
        XCTAssertNil(summary.rootWorkspacePath)
        XCTAssertEqual(summary.workspaceOrigin, "implicit")
        XCTAssertEqual(summary.workspacePath, "/data/thread-workspaces/thread--implicit")
    }

    func testPersistedEncodingDistinguishesNoneFromUnresolved() {
        XCTAssertNil(GaryxDraftWorkspaceSelection.unresolved.persistedValue)
        XCTAssertEqual(GaryxDraftWorkspaceSelection.none.persistedValue, "none")
        XCTAssertEqual(GaryxDraftWorkspaceSelection.path("/w/a").persistedValue, "path:/w/a")

        XCTAssertEqual(GaryxDraftWorkspaceSelection.fromPersistedValue(nil), .unresolved)
        XCTAssertEqual(GaryxDraftWorkspaceSelection.fromPersistedValue(""), .unresolved)
        XCTAssertEqual(GaryxDraftWorkspaceSelection.fromPersistedValue("none"), GaryxDraftWorkspaceSelection.none)
        XCTAssertEqual(GaryxDraftWorkspaceSelection.fromPersistedValue("path:/w/a"), .path("/w/a"))
        // Legacy bare strings are not a recognized encoding; they read as
        // unresolved exactly once and re-resolve from the catalog.
        XCTAssertEqual(GaryxDraftWorkspaceSelection.fromPersistedValue("/w/legacy"), .unresolved)
        XCTAssertEqual(GaryxDraftWorkspaceSelection.fromPersistedValue("path:   "), .unresolved)
    }

    // MARK: - Directory browser reducer

    private func listing(
        path: String,
        parent: String?,
        entries: [GaryxWorkspaceDirectoryEntry]
    ) -> GaryxWorkspaceDirectoryListing {
        GaryxWorkspaceDirectoryListing(path: path, parentPath: parent, entries: entries)
    }

    func testBrowserAppliesListingAndResetsFilter() {
        var state = GaryxWorkspaceDirectoryBrowserState()
        state.beginLoad()
        XCTAssertTrue(state.isLoading)
        state.apply(listing(
            path: "/Users/test",
            parent: "/Users",
            entries: [
                GaryxWorkspaceDirectoryEntry(name: "repos", path: "/Users/test/repos"),
                GaryxWorkspaceDirectoryEntry(name: "garyx", path: "/Users/test/garyx", gitRepo: true),
            ]
        ))
        XCTAssertFalse(state.isLoading)
        XCTAssertEqual(state.currentPath, "/Users/test")
        XCTAssertEqual(state.filteredEntries.count, 2)

        state.filterText = "gar"
        XCTAssertEqual(state.filteredEntries.map(\.name), ["garyx"])

        // Navigating away replaces the listing and clears the filter.
        state.apply(listing(path: "/Users/test/garyx", parent: "/Users/test", entries: []))
        XCTAssertEqual(state.filterText, "")
    }

    func testBrowserFailureStaysPutWithInlineError() {
        var state = GaryxWorkspaceDirectoryBrowserState()
        state.apply(listing(
            path: "/Users/test",
            parent: "/Users",
            entries: [GaryxWorkspaceDirectoryEntry(name: "repos", path: "/Users/test/repos")]
        ))

        state.beginLoad()
        state.fail(GaryxWorkspaceDirectoryError(code: .notFound, message: "missing"))
        // Stay-put: the previous listing is still on screen.
        XCTAssertEqual(state.currentPath, "/Users/test")
        XCTAssertEqual(state.filteredEntries.count, 1)
        XCTAssertEqual(
            state.inlineError,
            .typed(GaryxWorkspaceDirectoryError(code: .notFound, message: "missing"))
        )

        // The next successful navigation clears the error.
        state.beginLoad()
        XCTAssertNil(state.inlineError)
    }

    func testBrowserPathSegmentsJumpTargets() {
        var state = GaryxWorkspaceDirectoryBrowserState()
        state.apply(listing(path: "/Users/test/repos", parent: "/Users/test", entries: []))
        XCTAssertEqual(
            state.pathSegments.map(\.path),
            ["/", "/Users", "/Users/test", "/Users/test/repos"]
        )
        XCTAssertEqual(state.pathSegments.map(\.label), ["/", "Users", "test", "repos"])
    }

    func testBrowserTypedPathNormalization() {
        var state = GaryxWorkspaceDirectoryBrowserState()
        XCTAssertNil(state.normalizeTypedPath("   "))
        XCTAssertNil(state.inlineError)

        XCTAssertEqual(state.normalizeTypedPath("/Users/test/repos/"), "/Users/test/repos")
        XCTAssertEqual(state.normalizeTypedPath("  /Users/test  "), "/Users/test")
        XCTAssertEqual(state.normalizeTypedPath("/"), "/")

        // Relative input short-circuits to the server's invalid_path contract.
        XCTAssertNil(state.normalizeTypedPath("repos/garyx"))
        XCTAssertEqual(state.inlineError?.message, GaryxWorkspaceDirectoryErrorCode.invalidPath.userMessage)
    }
}
