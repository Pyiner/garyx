import Foundation
import SwiftUI
import UIKit

struct GaryxSkillsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateSkill = false

    private var skillEditorPresented: Binding<Bool> {
        Binding(
            get: { model.selectedSkillEditor != nil },
            set: { isPresented in
                if !isPresented {
                    closeSkillEditor()
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
        .garyxFullScreenCover(isPresented: $showsCreateSkill) {
            GaryxCreateSkillCard()
        }
        .garyxFullScreenCover(isPresented: skillEditorPresented) {
            GaryxFormSheet(title: "Skill Detail", onDone: closeSkillEditor) {
                GaryxSkillDetailCard()
            }
        }
    }

    private func closeSkillEditor() {
        model.closeSkillDetail()
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
            Group {
                GaryxFormGroupedSection(title: "Identity") {
                    GaryxFormTextFieldRow(
                        title: "ID",
                        text: $model.draftSkillId,
                        valuePlacement: .below,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    GaryxFormTextFieldRow(
                        title: "Name",
                        text: $model.draftSkillName,
                        placeholder: "Required"
                    )
                }

                GaryxFormGroupedSection(title: "Content") {
                    GaryxFormTextAreaRow(
                        title: "Description",
                        text: $model.draftSkillDescription,
                        minHeight: 104,
                        lineLimits: 2...4
                    )
                    GaryxFormTextAreaRow(
                        title: "Body",
                        text: $model.draftSkillBody,
                        minHeight: 220,
                        lineLimits: 6...14,
                        offersFocusedEditor: true
                    )
                }
            }
        }
    }

    private var canCreate: Bool {
        !model.draftSkillId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !model.draftSkillName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
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
    @State private var optimisticEnabled: Bool?
    @State private var name = ""
    @State private var description = ""

    var body: some View {
        GaryxRowActionMenu(actions: skillSwipeActions) {
            HStack(alignment: .center, spacing: 10) {
                Button {
                    Task { await model.openSkillEditor(skill) }
                } label: {
                    HStack(alignment: .center, spacing: 10) {
                        Image(systemName: "wand.and.stars")
                            .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: 24, height: 24)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(skill.name)
                                .font(GaryxFont.body(weight: .semibold))
                                .foregroundStyle(.primary)
                                .garyxReadingLineLimit()
                            Text(skill.description.isEmpty ? skill.id : skill.description)
                                .font(GaryxFont.caption(weight: .medium))
                                .foregroundStyle(.secondary)
                                .garyxReadingLineLimit(2)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(GaryxPressableRowStyle())

                Toggle("", isOn: skillEnabledBinding)
                    .labelsHidden()
                    .tint(GaryxTheme.controlTint)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .onChange(of: skill.enabled) { _, newValue in
            if optimisticEnabled == newValue {
                optimisticEnabled = nil
            }
        }
        .garyxFullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(
                title: "Edit Skill Info",
                canSave: canSaveSkill,
                onSave: { Task { await saveSkill() } }
            ) {
                GaryxSkillMetadataFields(
                    skill: skill,
                    name: $name,
                    description: $description,
                    mode: .editable
                )
            }
        }
        .garyxConfirmationDialog("Delete skill?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSkill(skill) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the skill directory.")
        }
    }

    private var skillEnabledBinding: Binding<Bool> {
        Binding {
            optimisticEnabled ?? skill.enabled
        } set: { nextValue in
            let currentValue = optimisticEnabled ?? skill.enabled
            guard nextValue != currentValue else { return }
            optimisticEnabled = nextValue
            Task {
                if await model.toggleSkill(skill) == false {
                    optimisticEnabled = nil
                }
            }
        }
    }

    private var skillSwipeActions: [GaryxRowAction] {
        [
            GaryxRowAction(title: "Edit Info", systemImage: "pencil") {
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

struct GaryxSkillDetailCard: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        if let editor = model.selectedSkillEditor {
            Group {
                GaryxSkillMetadataFields(
                    skill: editor.skill,
                    name: .constant(editor.skill.name),
                    description: .constant(editor.skill.description),
                    mode: .readOnly
                )

                GaryxFormGroupedSection(title: "Files") {
                    if editor.entries.isEmpty {
                        Text("No files in this skill.")
                            .font(GaryxFont.callout())
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(16)
                    } else {
                        VStack(alignment: .leading, spacing: 4) {
                            ForEach(editor.entries) { node in
                                GaryxSkillEntryRow(node: node, depth: 0) { path in
                                    Task { await model.openSkillFile(skillId: editor.skill.id, path: path) }
                                }
                            }
                        }
                        .padding(12)
                    }
                }

                if let document = model.selectedSkillDocument {
                    GaryxFormGroupedSection(title: document.path) {
                        GaryxSkillDocumentPreview(document: document)
                            .padding(16)
                    }
                } else {
                    GaryxEmptyPanelView(
                        icon: "doc.text.magnifyingglass",
                        title: "Select a file to inspect this skill.",
                        text: ""
                    )
                }
            }
        } else {
            GaryxLoadingPanelView(title: "Loading skill...")
        }
    }
}

private struct GaryxSkillMetadataFields: View {
    enum Mode {
        case readOnly
        case editable
    }

    let skill: GaryxSkillSummary
    @Binding var name: String
    @Binding var description: String
    let mode: Mode

    var body: some View {
        GaryxFormGroupedSection(title: "Skill") {
            switch mode {
            case .readOnly:
                GaryxFormReadOnlyRow(title: "Name", value: skill.name)
            case .editable:
                GaryxFormTextFieldRow(title: "Name", text: $name)
            }

            switch mode {
            case .readOnly:
                GaryxFormReadOnlyMultilineRow(
                    title: "Description",
                    value: skill.description,
                    placeholder: "No description provided.",
                    minHeight: 86,
                    valuePlacement: .below
                )
            case .editable:
                GaryxFormTextAreaRow(
                    title: "Description",
                    text: $description,
                    placeholder: "No description provided.",
                    minHeight: 86,
                    lineLimits: 2...5
                )
            }
        }
    }
}

struct GaryxSkillEntryRow: View {
    @Environment(\.isEnabled) private var isEnabled
    @EnvironmentObject private var model: GaryxMobileModel
    let node: GaryxSkillEntryNode
    let depth: Int
    let onOpenFile: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: node.entryType == "directory" ? "folder.fill" : "doc.text")
                    .font(GaryxFont.fixedSystem(size: 14, weight: .semibold))
                    .foregroundStyle(node.entryType == "directory" ? .secondary : .primary)
                    .frame(width: 18)
                VStack(alignment: .leading, spacing: 2) {
                    Text(node.name)
                        .font(GaryxFont.callout(weight: .medium))
                        .foregroundStyle(.primary)
                        .garyxReadingLineLimit()
                    if node.entryType == "file", node.path != node.name {
                        Text(node.path)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .garyxReadingLineLimit()
                            .truncationMode(.middle)
                    }
                }
                Spacer(minLength: 0)
                if isSelected {
                    GaryxSelectionCheckmark(size: 13)
                }
            }
            .padding(.vertical, 7)
            .padding(.horizontal, 8)
            .padding(.leading, CGFloat(depth) * 14)
            .background {
                if isSelected {
                    Color(.tertiarySystemFill).opacity(0.56)
                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
            }
            .contentShape(Rectangle())
            .onTapGesture {
                guard isEnabled else { return }
                if node.entryType == "file" {
                    onOpenFile(node.path)
                }
            }

            ForEach(node.children) { child in
                GaryxSkillEntryRow(node: child, depth: depth + 1, onOpenFile: onOpenFile)
            }
        }
    }

    private var isSelected: Bool {
        node.entryType == "file" && model.selectedSkillDocument?.path == node.path
    }
}

private struct GaryxSkillDocumentPreview: View {
    let document: GaryxSkillFileDocument

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Image(systemName: previewIcon)
                    .font(GaryxFont.fixedSystem(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
                Text(previewLabel)
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
                if !document.mediaType.isEmpty {
                    Text(document.mediaType)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.tertiary)
                        .garyxReadingLineLimit()
                }
            }

            switch document.previewKind {
            case "markdown":
                if document.content.isEmpty {
                    GaryxSkillPreviewUnavailableView(title: "Empty markdown file.")
                } else {
                    GaryxMarkdownText(text: document.content)
                        .textSelection(.enabled)
                        // Hosts in-place long-press menus for code blocks and
                        // inline images in the skill document preview.
                        .garyxMessageMenuHost()
                }
            case "text":
                GaryxSkillPlainTextPreview(content: document.content)
            case "image":
                if let image = GaryxDataURLImageCache.image(from: document.dataBase64) {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .frame(maxWidth: .infinity, alignment: .center)
                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                } else {
                    GaryxSkillPreviewUnavailableView(title: "Image preview is unavailable.")
                }
            default:
                if !document.content.isEmpty {
                    GaryxSkillPlainTextPreview(content: document.content)
                } else {
                    GaryxSkillPreviewUnavailableView(title: "Preview unavailable for this file type.")
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var previewIcon: String {
        switch document.previewKind {
        case "markdown":
            "doc.richtext"
        case "text":
            "doc.plaintext"
        case "image":
            "photo"
        default:
            "doc"
        }
    }

    private var previewLabel: String {
        switch document.previewKind {
        case "markdown":
            "Markdown"
        case "text":
            "Text"
        case "image":
            "Image"
        default:
            document.previewKind.capitalized
        }
    }
}

private struct GaryxSkillPlainTextPreview: View {
    let content: String

    var body: some View {
        Text(content.isEmpty ? "Empty file." : content)
            .font(.system(.footnote, design: .monospaced))
            .foregroundStyle(.primary)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct GaryxSkillPreviewUnavailableView: View {
    let title: String

    var body: some View {
        Text(title)
            .font(GaryxFont.callout())
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, minHeight: 96, alignment: .center)
    }
}
