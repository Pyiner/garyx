import XCTest
@testable import GaryxMobileCore

final class GaryxMobileWorkspacePresentationTests: XCTestCase {
    func testWorkspacePathSuggestionsDedupeSortAndHideDynamicPaths() {
        let paths = GaryxMobileWorkspacePresentation.workspacePathSuggestions(
            threadWorkspacePaths: [
                " /workspace/project-beta ",
                "/workspace/project-alpha",
                "/workspace/.garyx/worktrees/hidden",
                "/workspace/shared-worktree",
                "/tmp",
            ],
            threadWorktreePaths: [
                "/workspace/shared-worktree",
                "C:\\workspace\\generated-worktree",
            ],
            automationWorkspacePaths: [
                "/workspace/project-alpha",
                "/workspace/project-gamma",
            ],
            savedWorkspacePaths: [
                "/workspace/project-saved",
            ],
            additionalPaths: [
                "",
                " /workspace/project-delta ",
                "/private/tmp",
            ]
        )

        XCTAssertEqual(
            paths,
            [
                "/workspace/project-alpha",
                "/workspace/project-beta",
                "/workspace/project-delta",
                "/workspace/project-gamma",
                "/workspace/project-saved",
            ]
        )
    }

    func testAbbreviatedPathUsesGatewayHomeOnly() {
        // The gateway machine's home, never the device-local HOME.
        XCTAssertEqual(
            GaryxMobileWorkspacePresentation.abbreviatedPath(
                "/Users/test/repos/garyx",
                gatewayHome: "/Users/test"
            ),
            "~/repos/garyx"
        )
        XCTAssertEqual(
            GaryxMobileWorkspacePresentation.abbreviatedPath(
                "/Users/test",
                gatewayHome: "/Users/test/"
            ),
            "~"
        )
        // A sibling prefix must not match: /Users/testing is not under home.
        XCTAssertEqual(
            GaryxMobileWorkspacePresentation.abbreviatedPath(
                "/Users/testing/repos",
                gatewayHome: "/Users/test"
            ),
            "/Users/testing/repos"
        )
        // No gateway home known -> path passes through untouched.
        XCTAssertEqual(
            GaryxMobileWorkspacePresentation.abbreviatedPath(
                "/Users/test/repos",
                gatewayHome: nil
            ),
            "/Users/test/repos"
        )
        // A root home never abbreviates.
        XCTAssertEqual(
            GaryxMobileWorkspacePresentation.abbreviatedPath(
                "/anything",
                gatewayHome: "/"
            ),
            "/anything"
        )
    }

    func testVisibleWorkspacePathRejectsGeneratedWorktreeFolders() {
        XCTAssertTrue(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath("/workspace/project-alpha"))
        XCTAssertFalse(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath(" "))
        XCTAssertFalse(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath("/workspace/.garyx/worktrees/session"))
        XCTAssertFalse(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath("/workspace/.codex/worktrees/session"))
        XCTAssertFalse(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath("/tmp"))
        XCTAssertFalse(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath("/private/tmp"))
    }
}
