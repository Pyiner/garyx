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
        do {
            let skill = try await client().createSkill(
                GaryxCreateSkillRequest(
                    id: id,
                    name: name,
                    description: description,
                    body: body.isEmpty ? "" : body
                )
            )
            draftSkillId = ""
            draftSkillName = ""
            draftSkillDescription = ""
            draftSkillBody = ""
            skills.insert(skill, at: 0)
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func toggleSkill(_ skill: GaryxSkillSummary) async {
        do {
            let updated = try await client().toggleSkill(skillId: skill.id)
            replaceSkill(updated)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteSkill(_ skill: GaryxSkillSummary) async {
        do {
            _ = try await client().deleteSkill(skillId: skill.id)
            skills.removeAll { $0.id == skill.id }
            if selectedSkillEditor?.skill.id == skill.id {
                selectedSkillEditor = nil
                selectedSkillDocument = nil
                selectedSkillFileContent = ""
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateSkill(_ skill: GaryxSkillSummary, name: String, description: String) async {
        do {
            let updated = try await client().updateSkill(
                skillId: skill.id,
                request: GaryxUpdateSkillRequest(name: name, description: description)
            )
            replaceSkill(updated)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func openSkillEditor(_ skill: GaryxSkillSummary) async {
        do {
            selectedSkillEditor = try await client().skillEditor(skillId: skill.id)
            selectedSkillDocument = nil
            selectedSkillFileContent = ""
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func openSkillFile(skillId: String, path: String) async {
        do {
            let document = try await client().readSkillFile(skillId: skillId, path: path)
            selectedSkillDocument = document
            selectedSkillFileContent = document.content
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func saveSelectedSkillFile() async {
        guard let document = selectedSkillDocument else { return }
        do {
            let saved = try await client().saveSkillFile(
                skillId: document.skill.id,
                request: GaryxSkillFileWriteRequest(path: document.path, content: selectedSkillFileContent)
            )
            selectedSkillDocument = saved
            selectedSkillFileContent = saved.content
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func createSkillEntry() async {
        guard let editor = selectedSkillEditor else { return }
        let path = draftSkillEntryPath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else { return }
        do {
            selectedSkillEditor = try await client().createSkillEntry(
                skillId: editor.skill.id,
                request: GaryxSkillEntryCreateRequest(path: path, entryType: draftSkillEntryType)
            )
            draftSkillEntryPath = ""
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteSkillEntry(skillId: String, path: String) async {
        do {
            selectedSkillEditor = try await client().deleteSkillEntry(skillId: skillId, path: path)
            if selectedSkillDocument?.path == path {
                selectedSkillDocument = nil
                selectedSkillFileContent = ""
            }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func createSlashCommandFromDraft() async -> Bool {
        let name = draftSlashName.trimmingCharacters(in: .whitespacesAndNewlines)
        let description = draftSlashDescription.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = draftSlashPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, !description.isEmpty, !prompt.isEmpty else { return false }
        do {
            let command = try await client().createSlashCommand(
                GaryxSlashCommandRequest(name: name, description: description, prompt: prompt)
            )
            draftSlashName = ""
            draftSlashDescription = ""
            draftSlashPrompt = ""
            slashCommands.append(command)
            slashCommands.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func deleteSlashCommand(_ command: GaryxSlashCommand) async {
        do {
            _ = try await client().deleteSlashCommand(name: command.name)
            slashCommands.removeAll { $0.name == command.name }
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func updateSlashCommand(_ command: GaryxSlashCommand, name: String, description: String, prompt: String) async {
        let nextName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextDescription = description.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextPrompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextName.isEmpty, !nextDescription.isEmpty else { return }
        do {
            let updated = try await client().updateSlashCommand(
                currentName: command.name,
                request: GaryxSlashCommandRequest(
                    name: nextName,
                    description: nextDescription,
                    prompt: nextPrompt.isEmpty ? nil : nextPrompt
                )
            )
            replaceSlashCommand(updated, previousName: command.name)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func createMcpServerFromDraft() async -> Bool {
        let name = draftMcpName.trimmingCharacters(in: .whitespacesAndNewlines)
        let command = draftMcpCommand.trimmingCharacters(in: .whitespacesAndNewlines)
        let url = draftMcpUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, !command.isEmpty || !url.isEmpty else { return false }
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
            draftMcpName = ""
            draftMcpCommand = ""
            draftMcpArgs = ""
            draftMcpEnv = ""
            draftMcpWorkingDir = ""
            draftMcpUrl = ""
            draftMcpHeaders = ""
            mcpServers.append(server)
            mcpServers.sort { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func toggleMcpServer(_ server: GaryxMcpServer) async {
        do {
            let updated = try await client().toggleMcpServer(name: server.name, enabled: !server.enabled)
            replaceMcpServer(updated)
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func deleteMcpServer(_ server: GaryxMcpServer) async {
        do {
            _ = try await client().deleteMcpServer(name: server.name)
            mcpServers.removeAll { $0.name == server.name }
        } catch {
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
        do {
            let updated = try await client().updateMcpServer(
                currentName: server.name,
                request: GaryxMcpServerRequest(
                    name: nextName,
                    transport: nextUrl.isEmpty ? "stdio" : "streamable_http",
                    command: nextCommand,
                    args: splitShellLikeList(argsText),
                    env: keyValueDictionary(from: envText),
                    enabled: server.enabled,
                    workingDir: workingDir.trimmingCharacters(in: .whitespacesAndNewlines).garyxTrimmedNilIfEmpty,
                    url: nextUrl.isEmpty ? nil : nextUrl,
                    headers: keyValueDictionary(from: headersText)
                )
            )
            replaceMcpServer(updated, previousName: server.name)
        } catch {
            lastError = displayMessage(for: error)
        }
    }


    func replaceSkill(_ skill: GaryxSkillSummary) {
        if let index = skills.firstIndex(where: { $0.id == skill.id }) {
            skills[index] = skill
        } else {
            skills.insert(skill, at: 0)
        }
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
    }
}
