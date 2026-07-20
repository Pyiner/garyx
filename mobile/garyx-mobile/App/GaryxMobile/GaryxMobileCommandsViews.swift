import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxCommandsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateCommand = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Commands",
            subtitle: "\(model.slashCommands.count) shortcuts",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            GaryxCommandsContent()
        } actions: {
            GaryxAddToolbarButton(label: "Add Command") {
                showsCreateCommand = true
            }
        }
        .garyxSheet(isPresented: $showsCreateCommand) {
            GaryxCreateSlashCommandCard()
        }
    }
}

struct GaryxCommandsContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if model.slashCommands.isEmpty, model.isRemoteStatePending {
                GaryxLoadingPanelView(title: "Loading shortcuts...")
            } else if model.slashCommands.isEmpty {
                GaryxEmptyPanelView(
                    icon: "command",
                    title: "No shortcuts yet",
                    text: ""
                )
            } else {
                GaryxSectionBlock(title: "Slash Commands") {
                    GaryxCompactListGroup {
                        ForEach(Array(model.slashCommands.enumerated()), id: \.element.id) { index, command in
                            GaryxSlashCommandCard(command: command)
                            if index < model.slashCommands.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

struct GaryxCreateSlashCommandCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxFormSheet(
            title: "Add Command",
            canSave: canCreate,
            onSave: { Task { await createCommand() } }
        ) {
            GaryxFormGroupedSection(title: "Command") {
                GaryxFormTextFieldRow(
                    title: "Command name",
                    text: $model.draftSlashName,
                    autocapitalization: .never,
                    autocorrectionDisabled: true
                )
                GaryxFormTextFieldRow(
                    title: "Description",
                    text: $model.draftSlashDescription,
                    placeholder: "Optional"
                )
                GaryxFormTextAreaRow(
                    title: "Content",
                    text: $model.draftSlashPrompt,
                    minHeight: 132,
                    lineLimits: 2...5,
                    offersFocusedEditor: true
                )
            }
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
    }

    private var canCreate: Bool {
        !model.draftSlashName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !model.draftSlashPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func createCommand() async {
        guard canCreate else { return }
        if await model.createSlashCommandFromDraft() {
            dismiss()
        }
    }
}

struct GaryxSlashCommandCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let command: GaryxSlashCommand
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var name = ""
    @State private var description = ""
    @State private var prompt = ""

    var body: some View {
        GaryxRowActionMenu(actions: commandSwipeActions) {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "command")
                        .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)

                    VStack(alignment: .leading, spacing: 3) {
                        Text("/\(command.name)")
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .garyxReadingLineLimit()
                        Text(command.description.isEmpty ? command.prompt : command.description)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .garyxReadingLineLimit(2)
                    }
                    Spacer(minLength: 8)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear {
            name = command.name
            description = command.description
            prompt = command.prompt
        }
        .garyxSheet(isPresented: $showsEditForm) {
            GaryxFormSheet(
                title: "Edit Command",
                canSave: canSaveCommand,
                onSave: { Task { await saveCommand() } }
            ) {
                GaryxFormGroupedSection(title: "Command") {
                    GaryxFormTextFieldRow(
                        title: "Name",
                        text: $name,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    GaryxFormTextFieldRow(
                        title: "Description",
                        text: $description,
                        placeholder: "Optional"
                    )
                    GaryxFormTextAreaRow(
                        title: "Prompt",
                        text: $prompt,
                        minHeight: 132,
                        lineLimits: 2...6,
                        offersFocusedEditor: true
                    )
                }
            }
            .presentationDetents([.large])
            .presentationDragIndicator(.visible)
        }
        .garyxConfirmationDialog("Delete command?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSlashCommand(command) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the slash command.")
        }
    }

    private var commandSwipeActions: [GaryxRowAction] {
        [
            GaryxRowAction(title: "Edit", systemImage: "pencil", tone: .accent) {
                name = command.name
                description = command.description
                prompt = command.prompt
                showsEditForm = true
            },
            GaryxRowAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }

    private var canSaveCommand: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func saveCommand() async {
        guard canSaveCommand else { return }
        await model.updateSlashCommand(
            command,
            name: name,
            description: description,
            prompt: prompt
        )
        showsEditForm = false
    }
}
