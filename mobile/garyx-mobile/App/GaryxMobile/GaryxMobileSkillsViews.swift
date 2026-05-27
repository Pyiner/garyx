import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxSkillsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateSkill = false
    @State private var showsDiscardSkillEditorConfirmation = false

    private var skillEditorPresented: Binding<Bool> {
        Binding(
            get: { model.selectedSkillEditor != nil },
            set: { isPresented in
                if !isPresented {
                    requestCloseSkillEditor()
                }
            }
        )
    }

    var body: some View {
        GaryxPanelScaffold(
            title: "Skills",
            subtitle: "\(model.skills.filter(\.enabled).count) enabled / \(model.skills.count) total",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                if model.skills.isEmpty, model.isRemoteStatePending {
                    GaryxLoadingPanelView(title: "Loading skills...")
                } else if model.skills.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "wand.and.stars",
                        title: "No skills installed. Create your first skill.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Skills") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.skills.enumerated()), id: \.element.id) { index, skill in
                                GaryxSkillCard(skill: skill)
                                if index < model.skills.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Skill") {
                showsCreateSkill = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateSkill) {
            GaryxCreateSkillCard()
        }
        .fullScreenCover(isPresented: skillEditorPresented) {
            GaryxFormSheet(title: "Skill Editor", onDone: requestCloseSkillEditor) {
                GaryxSkillEditorCard()
            }
            .interactiveDismissDisabled(skillEditorHasUnsavedChanges)
            .confirmationDialog(
                "Discard unsaved skill changes?",
                isPresented: $showsDiscardSkillEditorConfirmation,
                titleVisibility: .visible
            ) {
                Button("Discard", role: .destructive) {
                    closeSkillEditor()
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Your current file edits have not been saved.")
            }
        }
    }

    private var skillEditorHasUnsavedChanges: Bool {
        guard let document = model.selectedSkillDocument, document.editable else { return false }
        return model.selectedSkillFileContent != document.content
    }

    private func requestCloseSkillEditor() {
        if skillEditorHasUnsavedChanges {
            showsDiscardSkillEditorConfirmation = true
        } else {
            closeSkillEditor()
        }
    }

    private func closeSkillEditor() {
        model.selectedSkillEditor = nil
        model.selectedSkillDocument = nil
        model.selectedSkillFileContent = ""
    }
}

struct GaryxCreateSkillCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxFormSheet(
            title: "New Skill",
            canSave: canCreate,
            onSave: { Task { await createSkill() } }
        ) {
            VStack(alignment: .leading, spacing: 22) {
                GaryxFormGroupedSection(title: "Identity") {
                    GaryxFormTextFieldRow(
                        title: "ID",
                        text: $model.draftSkillId,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    Divider().padding(.leading, 16)
                    GaryxFormTextFieldRow(
                        title: "Name",
                        text: $model.draftSkillName,
                        placeholder: "Optional"
                    )
                }

                GaryxFormGroupedSection(title: "Content") {
                    GaryxFormTextAreaRow(
                        title: "Description",
                        text: $model.draftSkillDescription,
                        minHeight: 104,
                        lineLimits: 2...4
                    )
                    Divider().padding(.leading, 16)
                    GaryxFormTextAreaRow(
                        title: "Body",
                        text: $model.draftSkillBody,
                        minHeight: 132,
                        lineLimits: 2...5
                    )
                }
            }
        }
    }

    private var canCreate: Bool {
        !model.draftSkillId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func createSkill() async {
        guard canCreate else { return }
        if await model.createSkillFromDraft() {
            dismiss()
        }
    }
}

struct GaryxSkillCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let skill: GaryxSkillSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var name = ""
    @State private var description = ""

    var body: some View {
        GaryxRowActionMenu(actions: skillSwipeActions) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "wand.and.stars")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(skill.name)
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(skill.description.isEmpty ? skill.sourcePath.garyxLastPathComponent : skill.description)
                            .font(GaryxFont.caption(weight: .medium))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                    Spacer()
                    GaryxStatusPill(text: skill.enabled ? "Enabled" : "Paused", tone: skill.enabled ? .good : .muted)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(
                title: "Edit Skill",
                canSave: canSaveSkill,
                onSave: { Task { await saveSkill() } }
            ) {
                VStack(alignment: .leading, spacing: 22) {
                    GaryxFormGroupedSection(title: "Skill") {
                        GaryxFormTextFieldRow(title: "Name", text: $name)
                        Divider().padding(.leading, 16)
                        GaryxFormTextAreaRow(
                            title: "Description",
                            text: $description,
                            minHeight: 112,
                            lineLimits: 2...4
                        )
                    }
                }
            }
        }
        .confirmationDialog("Delete skill?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSkill(skill) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the skill directory.")
        }
    }

    private var skillSwipeActions: [GaryxRowAction] {
        [
            GaryxRowAction(title: "Open", systemImage: "doc.text", tone: .accent) {
                Task { await model.openSkillEditor(skill) }
            },
            GaryxRowAction(title: skill.enabled ? "Disable" : "Enable", systemImage: skill.enabled ? "pause.fill" : "play.fill") {
                Task { await model.toggleSkill(skill) }
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
        name = skill.name
        description = skill.description
    }

    private var canSaveSkill: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func saveSkill() async {
        guard canSaveSkill else { return }
        await model.updateSkill(skill, name: name, description: description)
        showsEditForm = false
    }
}

struct GaryxSkillEditorCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsDiscardFileSwitchConfirmation = false
    @State private var pendingFileSkillId = ""
    @State private var pendingFilePath = ""

    var body: some View {
        if let editor = model.selectedSkillEditor {
            VStack(alignment: .leading, spacing: 22) {
                GaryxFormGroupedSection(title: editor.skill.name) {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(editor.entries) { node in
                            GaryxSkillEntryRow(skillId: editor.skill.id, node: node, depth: 0) { path in
                                requestOpenSkillFile(skillId: editor.skill.id, path: path)
                            }
                        }
                    }
                    .padding(16)
                }

                GaryxFormGroupedSection(title: "New Entry") {
                    VStack(alignment: .leading, spacing: 12) {
                        GaryxFormTextFieldRow(
                            title: "Path",
                            text: $model.draftSkillEntryPath,
                            placeholder: "path/to/file.md",
                            autocapitalization: .never,
                            autocorrectionDisabled: true
                        )
                        Divider().padding(.leading, 16)
                        Picker("Type", selection: $model.draftSkillEntryType) {
                            Text("New File").tag("file")
                            Text("New Folder").tag("directory")
                        }
                        .pickerStyle(.segmented)
                        .padding(.horizontal, 12)
                        .padding(.bottom, 12)
                        Button {
                            Task { await model.createSkillEntry() }
                        } label: {
                            Label("Create", systemImage: "plus")
                        }
                        .buttonStyle(GaryxSecondaryButtonStyle())
                        .padding(.horizontal, 12)
                        .padding(.bottom, 12)
                    }
                }

                if let document = model.selectedSkillDocument {
                    GaryxFormGroupedSection(title: document.path) {
                        VStack(alignment: .leading, spacing: 12) {
                            GaryxFormTextAreaRow(
                                title: "Content",
                                text: $model.selectedSkillFileContent,
                                minHeight: 220,
                                lineLimits: 6...16,
                                isDisabled: !document.editable
                            )
                            Button {
                                Task { await model.saveSelectedSkillFile() }
                            } label: {
                                Label("Save", systemImage: "square.and.arrow.down")
                            }
                            .buttonStyle(GaryxPrimaryCompactButtonStyle())
                            .disabled(!document.editable)
                            .padding(.horizontal, 12)
                            .padding(.bottom, 12)
                        }
                    }
                }
            }
            .confirmationDialog(
                "Discard unsaved skill changes?",
                isPresented: $showsDiscardFileSwitchConfirmation,
                titleVisibility: .visible
            ) {
                Button("Discard", role: .destructive) {
                    openPendingSkillFile()
                }
                Button("Cancel", role: .cancel) {
                    clearPendingSkillFile()
                }
            } message: {
                Text("Your current file edits have not been saved.")
            }
        }
    }

    private var skillEditorHasUnsavedChanges: Bool {
        guard let document = model.selectedSkillDocument, document.editable else { return false }
        return model.selectedSkillFileContent != document.content
    }

    private func requestOpenSkillFile(skillId: String, path: String) {
        if model.selectedSkillDocument?.path == path {
            return
        }
        if skillEditorHasUnsavedChanges {
            pendingFileSkillId = skillId
            pendingFilePath = path
            showsDiscardFileSwitchConfirmation = true
        } else {
            Task { await model.openSkillFile(skillId: skillId, path: path) }
        }
    }

    private func openPendingSkillFile() {
        let skillId = pendingFileSkillId
        let path = pendingFilePath
        clearPendingSkillFile()
        guard !skillId.isEmpty, !path.isEmpty else { return }
        Task { await model.openSkillFile(skillId: skillId, path: path) }
    }

    private func clearPendingSkillFile() {
        pendingFileSkillId = ""
        pendingFilePath = ""
    }
}

struct GaryxSkillEntryRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let skillId: String
    let node: GaryxSkillEntryNode
    let depth: Int
    let onOpenFile: (String) -> Void
    @State private var showsDeleteConfirmation = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: node.entryType == "directory" ? "folder.fill" : "doc.text")
                    .frame(width: 18)
                Button {
                    if node.entryType == "file" {
                        onOpenFile(node.path)
                    }
                } label: {
                    Text(node.name)
                        .font(GaryxFont.callout(weight: .medium))
                        .lineLimit(1)
                }
                .buttonStyle(.plain)
                Spacer(minLength: 0)
                Button(role: .destructive) {
                    showsDeleteConfirmation = true
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(GaryxMiniIconButtonStyle())
            }
            .padding(.leading, CGFloat(depth) * 14)

            ForEach(node.children) { child in
                GaryxSkillEntryRow(skillId: skillId, node: child, depth: depth + 1, onOpenFile: onOpenFile)
            }
        }
        .confirmationDialog("Delete skill entry?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSkillEntry(skillId: skillId, path: node.path) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(node.path)
        }
    }
}
