import XCTest
@testable import GaryxMobileCore

final class GaryxNewThreadWorkspaceModePolicyTests: XCTestCase {
    func testEmptyWorkspaceIsLocal() {
        XCTAssertEqual(
            GaryxNewThreadWorkspaceModePolicy.workspaceMode(
                workspace: "   ",
                preferredMode: "worktree",
                gitStatuses: ["/repo": gitStatus(isGitRepo: true)]
            ),
            "local"
        )
    }

    func testLocalPreferenceStaysLocalEvenForGitRepo() {
        XCTAssertEqual(
            GaryxNewThreadWorkspaceModePolicy.workspaceMode(
                workspace: "/repo",
                preferredMode: "local",
                gitStatuses: ["/repo": gitStatus(isGitRepo: true)]
            ),
            "local"
        )
    }

    func testWorktreePreferenceWithoutGitStatusIsLocal() {
        XCTAssertEqual(
            GaryxNewThreadWorkspaceModePolicy.workspaceMode(
                workspace: "/repo",
                preferredMode: "worktree",
                gitStatuses: [:]
            ),
            "local"
        )
    }

    func testWorktreePreferenceWithNonRepoWorkspaceIsLocal() {
        XCTAssertEqual(
            GaryxNewThreadWorkspaceModePolicy.workspaceMode(
                workspace: "/repo",
                preferredMode: "worktree",
                gitStatuses: ["/repo": gitStatus(isGitRepo: false)]
            ),
            "local"
        )
    }

    func testWorktreePreferenceWithGitRepoIsWorktree() {
        XCTAssertEqual(
            GaryxNewThreadWorkspaceModePolicy.workspaceMode(
                workspace: "/repo",
                preferredMode: "worktree",
                gitStatuses: ["/repo": gitStatus(isGitRepo: true)]
            ),
            "worktree"
        )
    }

    func testPreferredModeIsTrimmedAndCaseInsensitive() {
        XCTAssertEqual(
            GaryxNewThreadWorkspaceModePolicy.workspaceMode(
                workspace: "/repo",
                preferredMode: "  Worktree ",
                gitStatuses: ["/repo": gitStatus(isGitRepo: true)]
            ),
            "worktree"
        )
    }

    func testWorkspaceIsTrimmedForStatusLookup() {
        XCTAssertEqual(
            GaryxNewThreadWorkspaceModePolicy.workspaceMode(
                workspace: "  /repo  ",
                preferredMode: "worktree",
                gitStatuses: ["/repo": gitStatus(isGitRepo: true)]
            ),
            "worktree"
        )
    }

    func testNormalizedWorkspaceModeDefaultsToLocal() {
        XCTAssertEqual(GaryxNewThreadWorkspaceModePolicy.normalizedWorkspaceMode(nil), "local")
        XCTAssertEqual(GaryxNewThreadWorkspaceModePolicy.normalizedWorkspaceMode("anything"), "local")
        XCTAssertEqual(GaryxNewThreadWorkspaceModePolicy.normalizedWorkspaceMode(" WORKTREE "), "worktree")
    }

    private func gitStatus(isGitRepo: Bool) -> GaryxWorkspaceGitStatus {
        GaryxWorkspaceGitStatus(workspaceDir: "/repo", isGitRepo: isGitRepo)
    }
}
