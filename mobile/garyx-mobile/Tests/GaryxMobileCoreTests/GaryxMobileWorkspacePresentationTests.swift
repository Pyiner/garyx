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

    func testUserWorkspacePathsOnlyUseExplicitSavedValues() {
        let paths = GaryxMobileWorkspacePresentation.userWorkspacePaths(
            savedWorkspacePaths: [
                " /workspace/project-beta ",
                "/workspace/project-alpha",
                "/workspace/project-beta",
            ]
        )

        XCTAssertEqual(
            paths,
            [
                "/workspace/project-alpha",
                "/workspace/project-beta",
            ]
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
