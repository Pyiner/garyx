import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxMcpServersView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateMcp = false

    var body: some View {
        GaryxPanelScaffold(
            title: "MCP",
            subtitle: "\(model.mcpServers.filter(\.enabled).count) enabled / \(model.mcpServers.count) servers",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            GaryxMcpServersContent()
        } actions: {
            GaryxAddToolbarButton(label: "Add Server") {
                showsCreateMcp = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateMcp) {
            GaryxCreateMcpServerCard()
        }
    }
}

struct GaryxMcpServersContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if model.mcpServers.isEmpty, model.isRemoteStatePending {
                GaryxLoadingPanelView(title: "Loading MCP servers...")
            } else if model.mcpServers.isEmpty {
                GaryxEmptyPanelView(
                    icon: "point.3.connected.trianglepath.dotted",
                    title: "No MCP servers yet",
                    text: ""
                )
            } else {
                GaryxSectionBlock(title: "MCP Servers") {
                    GaryxCompactListGroup {
                        ForEach(Array(model.mcpServers.enumerated()), id: \.element.id) { index, server in
                            GaryxMcpServerCard(server: server)
                            if index < model.mcpServers.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

struct GaryxCreateMcpServerCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxFormSheet(
            title: "Add Server",
            canSave: canCreate,
            onSave: { Task { await createServer() } }
        ) {
            GaryxMcpServerFormFields(
                name: $model.draftMcpName,
                command: $model.draftMcpCommand,
                args: $model.draftMcpArgs,
                env: $model.draftMcpEnv,
                workingDir: $model.draftMcpWorkingDir,
                url: $model.draftMcpUrl,
                headers: $model.draftMcpHeaders,
                workspacePaths: model.userWorkspacePaths
            )
        }
    }

    private var canCreate: Bool {
        !model.draftMcpName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func createServer() async {
        guard canCreate else { return }
        if await model.createMcpServerFromDraft() {
            dismiss()
        }
    }
}

private struct GaryxMcpServerFormFields: View {
    @Binding var name: String
    @Binding var command: String
    @Binding var args: String
    @Binding var env: String
    @Binding var workingDir: String
    @Binding var url: String
    @Binding var headers: String
    let workspacePaths: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxFormGroupedSection(title: "Server") {
                TextField("Name", text: $name)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
                Divider().padding(.leading, 16)
                GaryxWorkspacePathSelectionRow(
                    title: "Working directory",
                    path: $workingDir,
                    workspacePaths: workspacePaths,
                    placeholder: "Optional",
                    allowsEmpty: true
                )
            }

            GaryxFormGroupedSection(title: "Command") {
                TextField("Start command", text: $command)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
                Divider().padding(.leading, 16)
                TextField("Arguments", text: $args)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
                Divider().padding(.leading, 16)
                TextField("Environment variables", text: $env, axis: .vertical)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .lineLimit(2...4)
                    .garyxFormTextArea(minHeight: 112)
            }

            GaryxFormGroupedSection(title: "HTTP") {
                TextField("URL", text: $url)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
                Divider().padding(.leading, 16)
                TextField("Headers", text: $headers, axis: .vertical)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .lineLimit(2...4)
                    .garyxFormTextArea(minHeight: 112)
            }
        }
    }
}

struct GaryxMcpServerCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let server: GaryxMcpServer
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var name = ""
    @State private var command = ""
    @State private var args = ""
    @State private var env = ""
    @State private var workingDir = ""
    @State private var url = ""
    @State private var headers = ""

    var body: some View {
        GaryxRowActionMenu(actions: serverSwipeActions) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "point.3.connected.trianglepath.dotted")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(server.name)
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(server.transport == "streamable_http" ? server.url ?? "HTTP" : server.command)
                            .font(GaryxFont.caption(weight: .medium))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer()
                    GaryxStatusPill(text: server.enabled ? "Enabled" : "Paused", tone: server.enabled ? .good : .muted)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(
                title: "Edit MCP Server",
                canSave: canSaveServer,
                onSave: { Task { await saveServer() } }
            ) {
                GaryxMcpServerFormFields(
                    name: $name,
                    command: $command,
                    args: $args,
                    env: $env,
                    workingDir: $workingDir,
                    url: $url,
                    headers: $headers,
                    workspacePaths: model.userWorkspacePaths
                )
            }
        }
        .confirmationDialog("Delete MCP server?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteMcpServer(server) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(server.name)
        }
    }

    private var serverSwipeActions: [GaryxRowAction] {
        [
            GaryxRowAction(title: server.enabled ? "Disable" : "Enable", systemImage: server.enabled ? "pause.fill" : "play.fill", tone: .accent) {
                Task { await model.toggleMcpServer(server) }
            },
            GaryxRowAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxRowAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }

    private func fillDraft() {
        name = server.name
        command = server.command
        args = server.args.joined(separator: ", ")
        env = server.env.map { "\($0.key)=\($0.value)" }.sorted().joined(separator: "\n")
        workingDir = server.workingDir ?? ""
        url = server.url ?? ""
        headers = server.headers.map { "\($0.key)=\($0.value)" }.sorted().joined(separator: "\n")
    }

    private var canSaveServer: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func saveServer() async {
        guard canSaveServer else { return }
        await model.updateMcpServer(
            server,
            name: name,
            command: command,
            argsText: args,
            envText: env,
            workingDir: workingDir,
            url: url,
            headersText: headers
        )
        showsEditForm = false
    }
}
