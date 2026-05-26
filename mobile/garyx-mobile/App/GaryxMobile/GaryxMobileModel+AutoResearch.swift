import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func createAutoResearchRunFromDraft() async -> Bool {
        let goal = draftAutoResearchGoal.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        let iterationsText = draftAutoResearchIterations.trimmingCharacters(in: .whitespacesAndNewlines)
        let timeBudgetText = draftAutoResearchTimeBudgetMinutes.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !goal.isEmpty,
              !workspace.isEmpty,
              let iterations = Int(iterationsText), iterations > 0,
              let timeBudgetMinutes = Int(timeBudgetText),
              timeBudgetMinutes > 0,
              timeBudgetMinutes <= Int.max / 60 else {
            return false
        }
        let timeBudgetSecs = timeBudgetMinutes * 60
        do {
            let run = try await client().createAutoResearchRun(
                GaryxAutoResearchCreateRequest(
                    goal: goal,
                    workspaceDir: workspace,
                    maxIterations: iterations,
                    timeBudgetSecs: timeBudgetSecs
                )
            )
            draftAutoResearchGoal = ""
            autoResearchRuns.insert(run, at: 0)
            activePanel = .autoResearch
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func stopAutoResearchRun(_ run: GaryxAutoResearchRun) async {
        do {
            let updated = try await client().stopAutoResearchRun(runId: run.runId, reason: "user_requested")
            replaceAutoResearchRun(updated)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func loadAutoResearchDetail(_ run: GaryxAutoResearchRun) async {
        await loadAutoResearchDetail(runId: run.runId)
    }

    func loadAutoResearchDetail(runId: String) async {
        let runId = runId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !runId.isEmpty else { return }
        do {
            let gateway = try client()
            async let detailResult = gateway.getAutoResearchRun(runId: runId)
            async let iterationsResult = gateway.listAutoResearchIterations(runId: runId)
            let detail = try await detailResult
            let iterations = try await iterationsResult
            autoResearchDetailsByRunId[runId] = detail
            autoResearchIterationsByRunId[runId] = iterations
            replaceAutoResearchRun(detail.run)
            if let page = try? await gateway.listAutoResearchCandidates(runId: runId) {
                researchCandidatesByRunId[runId] = page
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func loadAutoResearchCandidates(_ run: GaryxAutoResearchRun) async {
        do {
            researchCandidatesByRunId[run.runId] = try await client().listAutoResearchCandidates(runId: run.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func selectAutoResearchCandidate(run: GaryxAutoResearchRun, candidate: GaryxResearchCandidate) async {
        do {
            let updated = try await client().selectAutoResearchCandidate(
                runId: run.runId,
                candidateId: candidate.candidateId
            )
            replaceAutoResearchRun(updated)
            await loadAutoResearchDetail(runId: updated.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func reverifyAutoResearchCandidate(run: GaryxAutoResearchRun, candidate: GaryxResearchCandidate) async {
        do {
            let updated = try await client().reverifyAutoResearchCandidate(
                runId: run.runId,
                request: GaryxAutoResearchReverifyRequest(candidateId: candidate.candidateId)
            )
            replaceAutoResearchRun(updated)
            await loadAutoResearchDetail(runId: updated.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func sendAutoResearchFeedback(
        run: GaryxAutoResearchRun,
        candidate: GaryxResearchCandidate?,
        feedback: String
    ) async {
        let feedback = feedback.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !feedback.isEmpty else { return }
        do {
            let message: String
            if let candidate {
                message = "Candidate \(candidate.iteration): \(feedback)"
            } else {
                message = feedback
            }
            let updated = try await client().sendAutoResearchFeedback(
                runId: run.runId,
                request: GaryxAutoResearchFeedbackRequest(message: message)
            )
            replaceAutoResearchRun(updated)
            await loadAutoResearchDetail(runId: updated.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteAutoResearchRun(_ run: GaryxAutoResearchRun) async {
        do {
            _ = try await client().deleteAutoResearchRun(runId: run.runId)
            autoResearchRuns.removeAll { $0.runId == run.runId }
            researchCandidatesByRunId.removeValue(forKey: run.runId)
            autoResearchDetailsByRunId.removeValue(forKey: run.runId)
            autoResearchIterationsByRunId.removeValue(forKey: run.runId)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func replaceAutoResearchRun(_ run: GaryxAutoResearchRun) {
        if let index = autoResearchRuns.firstIndex(where: { $0.runId == run.runId }) {
            autoResearchRuns[index] = run
        } else {
            autoResearchRuns.insert(run, at: 0)
        }
        if var detail = autoResearchDetailsByRunId[run.runId] {
            detail.run = run
            autoResearchDetailsByRunId[run.runId] = detail
        }
    }
}
