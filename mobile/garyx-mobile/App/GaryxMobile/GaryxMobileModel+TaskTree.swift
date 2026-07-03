import Foundation
import SwiftUI

// Conversation task-tree sidebar state: anchored forest fetch/cache with
// stale-response gating, open/close, row navigation, and the poll policy.
// All mapping decisions live in GaryxTaskTreeSidebarPresentation (Core).
extension GaryxMobileModel {
    var taskTreeSidebarRows: [GaryxTaskTreeRow] {
        guard let page = taskTreeForestPage else { return [] }
        return GaryxTaskTreeSidebarPresentation.rows(
            page: page,
            currentThreadId: selectedThread?.id
        )
    }

    var taskTreeActiveBadgeCount: Int {
        guard let page = taskTreeForestPage else { return 0 }
        return GaryxTaskTreeSidebarPresentation.activeBadgeCount(page: page)
    }

    /// Entry points (header button, edge gesture) exist only when the tree is
    /// known non-empty, matching the Mac popover's hidden-when-empty rule.
    var isTaskTreeSidebarAvailable: Bool {
        guard let page = taskTreeForestPage else { return false }
        return GaryxTaskTreeSidebarPresentation.isSidebarAvailable(page: page)
    }

    /// While the very first fetch for a thread is in flight the edge gesture
    /// may still open the panel onto a loading state.
    var isTaskTreeFirstLoadInFlight: Bool {
        taskTreeForestPage == nil && taskTreeLoadPhase.isLoading
    }

    var shouldContinueTaskTreePolling: Bool {
        GaryxTaskTreeSidebarPresentation.shouldContinuePolling(page: taskTreeForestPage)
    }

    /// Re-anchor the sidebar to the selected thread: restore the cached
    /// snapshot for instant rendering and close the panel when the
    /// conversation is left entirely.
    func syncTaskTreeSidebarAnchor() {
        let anchor = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !anchor.isEmpty else {
            taskTreeForestPage = nil
            taskTreeLoadPhase = .idle
            isTaskTreeSidebarOpen = false
            return
        }
        let cached = taskTreeSnapshotsByThread[anchor]
        taskTreeForestPage = cached
        taskTreeLoadPhase = cached == nil ? .idle : .loaded
    }

    func refreshSelectedThreadTaskForest() async {
        guard hasGatewaySettings,
              let anchor = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
              !anchor.isEmpty else {
            return
        }
        let token = taskTreeRequestGate.begin(
            gatewayKey: activeGatewayScopeId,
            anchorThreadId: anchor
        )
        if taskTreeForestPage == nil {
            taskTreeLoadPhase = .loading
        }
        do {
            let page = try await client().listTaskForest(anchorThreadId: anchor)
            guard taskTreeRequestGate.accepts(
                token: token,
                gatewayKey: activeGatewayScopeId,
                anchorThreadId: anchor
            ), selectedThread?.id == anchor else {
                return
            }
            taskTreeForestPage = page
            taskTreeSnapshotsByThread[anchor] = page
            taskTreeLoadPhase = .loaded
            taskTreePollSuspendedThreadId =
                GaryxTaskTreeSidebarPresentation.isSidebarAvailable(page: page) ? nil : anchor
        } catch {
            guard taskTreeRequestGate.accepts(
                token: token,
                gatewayKey: activeGatewayScopeId,
                anchorThreadId: anchor
            ), selectedThread?.id == anchor else {
                return
            }
            // Transient errors keep the previous snapshot and retry silently
            // on the next poll tick (desktop popover parity).
            if taskTreeForestPage == nil {
                taskTreeLoadPhase = .failed(displayMessage(for: error))
            }
        }
    }

    func openTaskTreeSidebar() {
        guard !isTaskTreeSidebarOpen else { return }
        dismissKeyboardForTaskTreeSidebar()
        isTaskTreeSidebarOpen = true
        Task { [weak self] in
            await self?.refreshSelectedThreadTaskForest()
        }
    }

    func closeTaskTreeSidebar() {
        isTaskTreeSidebarOpen = false
    }

    func toggleTaskTreeSidebar() {
        if isTaskTreeSidebarOpen {
            closeTaskTreeSidebar()
        } else {
            openTaskTreeSidebar()
        }
    }

    /// Row tap: the current thread's row only closes the panel; any other row
    /// closes and routes through the shared `openThread` path.
    func handleTaskTreeRowTap(_ row: GaryxTaskTreeRow) async {
        let currentThreadId = selectedThread?.id
        closeTaskTreeSidebar()
        guard GaryxTaskTreeSidebarPresentation.shouldNavigate(
            currentThreadId: currentThreadId,
            targetThreadId: row.threadId
        ) else {
            return
        }
        await openThread(id: row.threadId, source: .replace)
    }

    /// Local task mutations (create/status/title/assign/stop/delete) resume a
    /// poll suspended on a known-empty tree and refresh the open conversation's
    /// snapshot so the badge and sidebar catch the change immediately.
    func noteTaskTreeLocalMutation() {
        taskTreePollSuspendedThreadId = nil
        guard selectedThread != nil else { return }
        Task { [weak self] in
            await self?.refreshSelectedThreadTaskForest()
        }
    }

    /// Identity hint for a sidebar row resolved against the loaded agent
    /// targets so avatars reuse the shared cache and presentation.
    func taskTreeRowAvatar(for row: GaryxTaskTreeRow) -> GaryxSidebarThreadRowAvatar {
        let identityId = row.identityAgentId
        if !identityId.isEmpty,
           let target = agentTargets.first(where: { $0.id == identityId }) {
            return GaryxSidebarThreadRowAvatar(
                agentId: target.id,
                avatarDataUrl: target.avatarDataUrl,
                kind: target.kind,
                label: target.title,
                providerType: target.providerType,
                builtIn: target.builtIn
            )
        }
        return GaryxSidebarThreadRowAvatar(
            agentId: identityId,
            avatarDataUrl: "",
            kind: row.identityIsTeam ? .team : .agent,
            label: identityId.isEmpty ? row.title : identityId,
            providerType: row.providerType,
            builtIn: false
        )
    }

    private func dismissKeyboardForTaskTreeSidebar() {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil,
            from: nil,
            for: nil
        )
    }
}
