import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func refreshDreams() async {
        guard hasGatewaySettings, dreamsAutoScanEnabled else {
            dreams = []
            latestDreamScan = nil
            return
        }
        do {
            let page = try await client().listDreams(sinceHours: 24, limit: 80)
            dreams = page.dreams
            latestDreamScan = page.scan ?? page.latestScan
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func scanDreams() async {
        guard hasGatewaySettings, dreamsAutoScanEnabled, !isScanningDreams else { return }
        isScanningDreams = true
        defer { isScanningDreams = false }
        do {
            let page = try await client().scanDreams(
                request: GaryxDreamScanRequest(sinceHours: 24, mode: "auto", limit: 600)
            )
            dreams = page.dreams
            latestDreamScan = page.scan ?? page.latestScan
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func setDreamsAutoScanEnabled(_ enabled: Bool) async {
        guard hasGatewaySettings, dreamsAutoScanEnabled != enabled, !isSavingDreamsSettings else {
            return
        }
        let previous = dreamsAutoScanEnabled
        dreamsAutoScanEnabled = enabled
        isSavingDreamsSettings = true
        defer { isSavingDreamsSettings = false }
        do {
            _ = try await client().saveGatewaySettings([
                "dreams": .object([
                    "enabled": .bool(enabled)
                ])
            ])
            gatewaySettingsStatus = "Saved"
            if !enabled {
                dreams = []
                latestDreamScan = nil
                if activePanel == .dreams {
                    activePanel = .chat
                }
            } else {
                await refreshDreams()
            }
        } catch {
            dreamsAutoScanEnabled = previous
            lastError = displayMessage(for: error)
        }
    }

    func openDreamSpan(_ span: GaryxDreamSpan) async {
        await openThread(id: span.threadId)
    }

    func runAutomation(_ automation: GaryxAutomationSummary) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let run = try await client().runAutomationNow(id: automation.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastAutomationRun = run
            await refreshRemoteState()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            if !run.threadId.isEmpty {
                await openThread(id: run.threadId)
            } else if let targetThreadId = automation.targetThreadId, !targetThreadId.isEmpty {
                await openThread(id: targetThreadId)
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func toggleAutomation(_ automation: GaryxAutomationSummary) async {
        await setAutomationEnabled(automation, enabled: !automation.enabled)
    }

    @discardableResult
    func setAutomationEnabled(_ automation: GaryxAutomationSummary, enabled: Bool) async -> Bool {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().updateAutomationEnabled(
                id: automation.id,
                enabled: enabled
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            await refreshRemoteState()
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    @discardableResult
    func updateAutomation(
        _ automation: GaryxAutomationSummary,
        label: String,
        prompt: String,
        agentId rawAgentId: String,
        schedule: GaryxAutomationSchedule,
        targetsExistingThread: Bool,
        targetThreadId: String,
        workspacePath: String
    ) async -> Bool {
        let nextLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextPrompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextTargetThreadId = targetsExistingThread
            ? targetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            : ""
        let nextAgentId = rawAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextWorkspacePath = workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextLabel.isEmpty, !nextPrompt.isEmpty else { return false }
        if targetsExistingThread {
            guard !nextTargetThreadId.isEmpty else { return false }
        } else {
            guard !nextAgentId.isEmpty else { return false }
            guard !nextWorkspacePath.isEmpty else { return false }
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let updated = try await client().updateAutomation(
                id: automation.id,
                request: GaryxAutomationUpdateRequest(
                    label: nextLabel,
                    prompt: nextPrompt,
                    agentId: nextTargetThreadId.isEmpty ? nextAgentId : nil,
                    workspaceDir: nextTargetThreadId.isEmpty ? nextWorkspacePath : nil,
                    targetThreadId: nextTargetThreadId.isEmpty ? nil : nextTargetThreadId,
                    clearsTargetThreadId: nextTargetThreadId.isEmpty,
                    schedule: schedule
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            replaceAutomation(updated)
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }


    func createAutomation(
        label rawLabel: String,
        prompt rawPrompt: String,
        agentId rawAgentId: String,
        workspacePath rawWorkspacePath: String,
        targetThreadId rawTargetThreadId: String,
        schedule: GaryxAutomationSchedule
    ) async -> Bool {
        let label = rawLabel.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = rawPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let agentId = rawAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = rawWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        let targetThreadId = rawTargetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !label.isEmpty, !prompt.isEmpty else { return false }
        guard !targetThreadId.isEmpty || !workspace.isEmpty else { return false }
        if targetThreadId.isEmpty {
            guard !agentId.isEmpty else { return false }
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let automation = try await client().createAutomation(
                GaryxAutomationCreateRequest(
                    label: label,
                    prompt: prompt,
                    agentId: agentId.isEmpty ? nil : agentId,
                    workspaceDir: targetThreadId.isEmpty && !workspace.isEmpty ? workspace : nil,
                    targetThreadId: targetThreadId.isEmpty ? nil : targetThreadId,
                    schedule: schedule,
                    enabled: true
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            automations.insert(automation, at: 0)
            activePanel = .automations
            persistCatalogCacheSnapshot()
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    func deleteAutomation(_ automation: GaryxAutomationSummary) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().deleteAutomation(id: automation.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            automations.removeAll { $0.id == automation.id }
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func replaceAutomation(_ automation: GaryxAutomationSummary) {
        if let index = automations.firstIndex(where: { $0.id == automation.id }) {
            automations[index] = automation
        } else {
            automations.insert(automation, at: 0)
        }
        persistCatalogCacheSnapshot()
    }
}
