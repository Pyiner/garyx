import XCTest
@testable import GaryxMobileCore

final class GaryxMobileWorkspacePresentationTests: XCTestCase {
    func testKnownWorkspacePathsDedupeSortAndHideWorktrees() {
        let paths = GaryxMobileWorkspacePresentation.knownWorkspacePaths(
            threadWorkspacePaths: [
                " /workspace/project-beta ",
                "/workspace/project-alpha",
                "/workspace/.garyx/worktrees/hidden",
                "/workspace/shared-worktree",
            ],
            threadWorktreePaths: [
                "/workspace/shared-worktree",
                "C:\\workspace\\generated-worktree",
            ],
            automationWorkspacePaths: [
                "/workspace/project-alpha",
                "/workspace/project-gamma",
            ],
            autoResearchWorkspaceDirs: [
                "/workspace/.codex/worktrees/hidden",
                "C:/workspace/generated-worktree",
            ],
            additionalPaths: [
                "",
                " /workspace/project-delta ",
            ]
        )

        XCTAssertEqual(
            paths,
            [
                "/workspace/project-alpha",
                "/workspace/project-beta",
                "/workspace/project-delta",
                "/workspace/project-gamma",
            ]
        )
    }

    func testVisibleWorkspacePathRejectsGeneratedWorktreeFolders() {
        XCTAssertTrue(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath("/workspace/project-alpha"))
        XCTAssertFalse(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath(" "))
        XCTAssertFalse(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath("/workspace/.garyx/worktrees/session"))
        XCTAssertFalse(GaryxMobileWorkspacePresentation.isVisibleWorkspacePath("/workspace/.codex/worktrees/session"))
    }
}
