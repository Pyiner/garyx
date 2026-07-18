import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func createSkillFromDraft() async -> Bool {
        let id = draftSkillId.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = draftSkillName.trimmingCharacters(in: .whitespacesAndNewlines)
        let description = draftSkillDescription.trimmingCharacters(in: .whitespacesAndNewlines)
        let body = draftSkillBody.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty, !name.isEmpty else { return false }
        let runtimeGeneration = gatewayRequestToken
        do {
            let skill = try await client().createSkill(
                GaryxCreateSkillRequest(
                    id: id,
                    name: name,
                    description: description,
                    body: body.isEmpty ? "" : body
                )
            )
            guard runtimeGeneration == gatewayRequestToken else { return false }
            draftSkillId = ""
            draftSkillName = ""
            draftSkillDescription = ""
            draftSkillBody = ""
            skills.insert(skill, at: 0)
            persistCatalogCacheSnapshot()
            return true
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    @discardableResult
    func toggleSkill(_ skill: GaryxSkillSummary) async -> Bool {
        let runtimeGeneration = gatewayRequestToken
        do {
            let updated = try await client().toggleSkill(skillId: skill.id)
            guard runtimeGeneration == gatewayRequestToken else { return false }
            replaceSkill(updated)
            return true
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    func deleteSkill(_ skill: GaryxSkillSummary) async {
        let runtimeGeneration = gatewayRequestToken
        do {
            _ = try await client().deleteSkill(skillId: skill.id)
            guard runtimeGeneration == gatewayRequestToken else { return }
            skills.removeAll { $0.id == skill.id }
            if selectedSkillEditor?.skill.id == skill.id {
                closeSkillDetail()
            }
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func updateSkill(_ skill: GaryxSkillSummary, name: String, description: String) async {
        let runtimeGeneration = gatewayRequestToken
        do {
            let updated = try await client().updateSkill(
                skillId: skill.id,
                request: GaryxUpdateSkillRequest(name: name, description: description)
            )
            guard runtimeGeneration == gatewayRequestToken else { return }
            replaceSkill(updated)
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func openSkillEditor(_ skill: GaryxSkillSummary, selecting requestedPath: String? = nil) async {
        let runtimeGeneration = gatewayRequestToken
        let editorRequestId = UUID()
        skillEditorLoadRequestId = editorRequestId
        skillFileLoadRequestId = nil
        do {
            let gateway = try client()
            let editor = try await gateway.skillEditor(skillId: skill.id)
            guard runtimeGeneration == gatewayRequestToken,
                  skillEditorLoadRequestId == editorRequestId else { return }
            selectedSkillEditor = editor
            selectedSkillDocument = nil
            let normalizedRequestedPath = requestedPath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let documentPath = normalizedRequestedPath.isEmpty
                ? Self.preferredSkillFilePath(in: editor.entries)
                : normalizedRequestedPath
            guard let preferredPath = documentPath, !preferredPath.isEmpty else {
                return
            }
            let fileRequestId = UUID()
            skillFileLoadRequestId = fileRequestId
            do {
                let document = try await gateway.readSkillFile(skillId: skill.id, path: preferredPath)
                guard runtimeGeneration == gatewayRequestToken,
                      skillEditorLoadRequestId == editorRequestId,
                      skillFileLoadRequestId == fileRequestId,
                      selectedSkillEditor?.skill.id == skill.id else { return }
                selectedSkillDocument = document
            } catch {
                guard runtimeGeneration == gatewayRequestToken,
                      skillEditorLoadRequestId == editorRequestId,
                      skillFileLoadRequestId == fileRequestId,
                      selectedSkillEditor?.skill.id == skill.id else { return }
                lastError = displayMessage(for: error)
            }
        } catch {
            guard runtimeGeneration == gatewayRequestToken,
                  skillEditorLoadRequestId == editorRequestId else { return }
            lastError = displayMessage(for: error)
        }
    }

    func openSkillFile(skillId: String, path: String) async {
        let runtimeGeneration = gatewayRequestToken
        let fileRequestId = UUID()
        skillFileLoadRequestId = fileRequestId
        do {
            let document = try await client().readSkillFile(skillId: skillId, path: path)
            guard runtimeGeneration == gatewayRequestToken,
                  skillFileLoadRequestId == fileRequestId,
                  selectedSkillEditor?.skill.id == skillId else { return }
            selectedSkillDocument = document
        } catch {
            guard runtimeGeneration == gatewayRequestToken,
                  skillFileLoadRequestId == fileRequestId,
                  selectedSkillEditor?.skill.id == skillId else { return }
            lastError = displayMessage(for: error)
        }
    }

    func closeSkillDetail() {
        skillEditorLoadRequestId = nil
        skillFileLoadRequestId = nil
        selectedSkillEditor = nil
        selectedSkillDocument = nil
    }

    private static func preferredSkillFilePath(in entries: [GaryxSkillEntryNode]) -> String? {
        let filePaths = skillFilePaths(in: entries)
        return filePaths.first { $0 == "SKILL.md" }
            ?? filePaths.first { $0.localizedCaseInsensitiveCompare("SKILL.md") == .orderedSame }
            ?? filePaths.first
    }

    private static func skillFilePaths(in entries: [GaryxSkillEntryNode]) -> [String] {
        entries.flatMap { entry -> [String] in
            var paths: [String] = []
            if entry.entryType == "file" {
                paths.append(entry.path)
            }
            paths.append(contentsOf: skillFilePaths(in: entry.children))
            return paths
        }
    }

    func createSlashCommandFromDraft() async -> Bool {
        let name = draftSlashName.trimmingCharacters(in: .whitespacesAndNewlines)
        let description = draftSlashDescription.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = draftSlashPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, !description.isEmpty, !prompt.isEmpty else { return false }
        let runtimeGeneration = gatewayRequestToken
        do {
            let command = try await client().createSlashCommand(
                GaryxSlashCommandRequest(name: name, description: description, prompt: prompt)
            )
            guard runtimeGeneration == gatewayRequestToken else { return false }
            draftSlashName = ""
            draftSlashDescription = ""
            draftSlashPrompt = ""
            slashCommands.append(command)
            slashCommands.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            persistCatalogCacheSnapshot()
            return true
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    func deleteSlashCommand(_ command: GaryxSlashCommand) async {
        let runtimeGeneration = gatewayRequestToken
        do {
            _ = try await client().deleteSlashCommand(name: command.name)
            guard runtimeGeneration == gatewayRequestToken else { return }
            slashCommands.removeAll { $0.name == command.name }
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func updateSlashCommand(_ command: GaryxSlashCommand, name: String, description: String, prompt: String) async {
        let nextName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextDescription = description.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextPrompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextName.isEmpty, !nextDescription.isEmpty else { return }
        let runtimeGeneration = gatewayRequestToken
        do {
            let updated = try await client().updateSlashCommand(
                currentName: command.name,
                request: GaryxSlashCommandRequest(
                    name: nextName,
                    description: nextDescription,
                    prompt: nextPrompt.isEmpty ? nil : nextPrompt
                )
            )
            guard runtimeGeneration == gatewayRequestToken else { return }
            replaceSlashCommand(updated, previousName: command.name)
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func createMcpServerFromDraft() async -> Bool {
        let name = draftMcpName.trimmingCharacters(in: .whitespacesAndNewlines)
        let command = draftMcpCommand.trimmingCharacters(in: .whitespacesAndNewlines)
        let url = draftMcpUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, !command.isEmpty || !url.isEmpty else { return false }
        let runtimeGeneration = gatewayRequestToken
        do {
            let request = GaryxMcpServerRequest(
                name: name,
                transport: url.isEmpty ? "stdio" : "streamable_http",
                command: command,
                args: splitShellLikeList(draftMcpArgs),
                env: keyValueDictionary(from: draftMcpEnv),
                enabled: true,
                workingDir: draftMcpWorkingDir.trimmingCharacters(in: .whitespacesAndNewlines).garyxTrimmedNilIfEmpty,
                url: url.isEmpty ? nil : url,
                headers: keyValueDictionary(from: draftMcpHeaders)
            )
            let server = try await client().createMcpServer(request)
            guard runtimeGeneration == gatewayRequestToken else { return false }
            draftMcpName = ""
            draftMcpCommand = ""
            draftMcpArgs = ""
            draftMcpEnv = ""
            draftMcpWorkingDir = ""
            draftMcpUrl = ""
            draftMcpHeaders = ""
            mcpServers.append(server)
            mcpServers.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            persistCatalogCacheSnapshot()
            return true
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    func toggleMcpServer(_ server: GaryxMcpServer) async {
        let runtimeGeneration = gatewayRequestToken
        do {
            let updated = try await client().toggleMcpServer(name: server.name, enabled: !server.enabled)
            guard runtimeGeneration == gatewayRequestToken else { return }
            replaceMcpServer(updated)
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func deleteMcpServer(_ server: GaryxMcpServer) async {
        let runtimeGeneration = gatewayRequestToken
        do {
            _ = try await client().deleteMcpServer(name: server.name)
            guard runtimeGeneration == gatewayRequestToken else { return }
            mcpServers.removeAll { $0.name == server.name }
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    func updateMcpServer(
        _ server: GaryxMcpServer,
        name: String,
        command: String,
        argsText: String,
        envText: String,
        workingDir: String,
        url: String,
        headersText: String
    ) async {
        let nextName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextCommand = command.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextUrl = url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextName.isEmpty, !nextCommand.isEmpty || !nextUrl.isEmpty else { return }
        let runtimeGeneration = gatewayRequestToken
        do {
            var baseServer = server
            if catalogSnapshotRestored {
                let latestServers = try await client().listMcpServers()
                guard runtimeGeneration == gatewayRequestToken else { return }
                guard let latestServer = latestServers.first(where: { $0.name == server.name }) else {
                    lastError = "MCP server details are still loading. Try again after refresh."
                    return
                }
                baseServer = latestServer
            }
            let parsedEnv = keyValueDictionary(from: envText)
            let parsedHeaders = keyValueDictionary(from: headersText)
            let nextEnv = server.env.isEmpty && parsedEnv.isEmpty && !baseServer.env.isEmpty
                ? baseServer.env
                : parsedEnv
            let nextHeaders = server.headers.isEmpty && parsedHeaders.isEmpty && !baseServer.headers.isEmpty
                ? baseServer.headers
                : parsedHeaders
            let updated = try await client().updateMcpServer(
                currentName: server.name,
                request: GaryxMcpServerRequest(
                    name: nextName,
                    transport: nextUrl.isEmpty ? "stdio" : "streamable_http",
                    command: nextCommand,
                    args: splitShellLikeList(argsText),
                    env: nextEnv,
                    enabled: server.enabled,
                    workingDir: workingDir.trimmingCharacters(in: .whitespacesAndNewlines).garyxTrimmedNilIfEmpty,
                    url: nextUrl.isEmpty ? nil : nextUrl,
                    bearerTokenEnv: baseServer.bearerTokenEnv,
                    headers: nextHeaders
                )
            )
            guard runtimeGeneration == gatewayRequestToken else { return }
            replaceMcpServer(updated, previousName: server.name)
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }


    func replaceSkill(_ skill: GaryxSkillSummary) {
        if let index = skills.firstIndex(where: { $0.id == skill.id }) {
            skills[index] = skill
        } else {
            skills.insert(skill, at: 0)
        }
        persistCatalogCacheSnapshot()
    }

    func replaceSlashCommand(_ command: GaryxSlashCommand, previousName: String? = nil) {
        if let previousName, previousName != command.name {
            slashCommands.removeAll { $0.name == previousName }
        }
        if let index = slashCommands.firstIndex(where: { $0.name == command.name }) {
            slashCommands[index] = command
        } else {
            slashCommands.append(command)
        }
        slashCommands.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        persistCatalogCacheSnapshot()
    }

    func replaceMcpServer(_ server: GaryxMcpServer, previousName: String? = nil) {
        if let previousName, previousName != server.name {
            mcpServers.removeAll { $0.name == previousName }
        }
        if let index = mcpServers.firstIndex(where: { $0.name == server.name }) {
            mcpServers[index] = server
        } else {
            mcpServers.append(server)
        }
        mcpServers.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        persistCatalogCacheSnapshot()
    }
}
