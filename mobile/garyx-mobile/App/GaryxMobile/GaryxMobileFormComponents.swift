import Foundation
import SwiftUI
import UIKit

struct GaryxFormSheet<Content: View>: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    let canSave: Bool?
    let saveTitle: String
    let isSaving: Bool
    let onCancel: (() -> Void)?
    let onSave: (() -> Void)?
    let onDone: (() -> Void)?
    let content: Content

    init(title: String, onDone: (() -> Void)? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.canSave = nil
        self.saveTitle = "Save"
        self.isSaving = false
        self.onCancel = nil
        self.onSave = nil
        self.onDone = onDone
        self.content = content()
    }

    init(
        title: String,
        canSave: Bool,
        saveTitle: String = "Save",
        isSaving: Bool = false,
        onCancel: (() -> Void)? = nil,
        onSave: @escaping () -> Void,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.canSave = canSave
        self.saveTitle = saveTitle
        self.isSaving = isSaving
        self.onCancel = onCancel
        self.onSave = onSave
        self.onDone = nil
        self.content = content()
    }

    var body: some View {
        NavigationStack {
            Form {
                content
            }
            .formStyle(.grouped)
            .scrollDismissesKeyboard(.interactively)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if let onSave {
                    ToolbarItem(placement: .cancellationAction) {
                        Button(action: cancel) {
                            Text("Cancel")
                                .foregroundStyle(.primary)
                        }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button(action: onSave) {
                            ZStack {
                                Text(saveTitle)
                                    .fontWeight(.semibold)
                                    .opacity(isSaving ? 0 : 1)
                                if isSaving {
                                    ProgressView()
                                        .controlSize(.small)
                                }
                            }
                            .foregroundStyle(canSave == false ? Color.secondary : Color.primary)
                        }
                        .disabled(canSave == false)
                        .accessibilityLabel(isSaving ? "Saving" : saveTitle)
                    }
                } else {
                    ToolbarItem(placement: .confirmationAction) {
                        Button(action: finish) {
                            Text("Done")
                                .fontWeight(.semibold)
                                .foregroundStyle(.primary)
                        }
                    }
                }
            }
        }
        .tint(GaryxTheme.controlTint)
    }

    private func cancel() {
        if let onCancel {
            onCancel()
        } else if let onDone {
            onDone()
        } else {
            dismiss()
        }
    }

    private func finish() {
        if let onDone {
            onDone()
        } else {
            dismiss()
        }
    }
}

struct GaryxFormGroupedSection<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        Section {
            content
        } header: {
            Text(title)
                .textCase(nil)
        }
    }
}

enum GaryxFormValuePlacement {
    case trailing
    case below
}

private struct GaryxFormFieldTitle: View {
    let title: String
    var required = false

    var body: some View {
        HStack(spacing: 4) {
            Text(title)
                .font(Font.callout)
                .foregroundStyle(.primary)
            if required {
                Text("*")
                    .font(Font.callout.weight(.semibold))
                    .foregroundStyle(GaryxTheme.danger)
            }
        }
        .garyxReadingLineLimit(2)
        .fixedSize(horizontal: false, vertical: true)
    }
}

struct GaryxFormRow<Content: View>: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let title: String
    let required: Bool
    let valuePlacement: GaryxFormValuePlacement
    /// When set, the whole row becomes a tap target that runs `onTap` — used for
    /// navigation / present rows so the title and surrounding whitespace remain
    /// hittable. Editable rows leave this `nil` so caret placement keeps working.
    let onTap: (() -> Void)?
    let content: Content

    init(
        title: String,
        required: Bool = false,
        valuePlacement: GaryxFormValuePlacement = .trailing,
        onTap: (() -> Void)? = nil,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.required = required
        self.valuePlacement = valuePlacement
        self.onTap = onTap
        self.content = content()
    }

    var body: some View {
        if let onTap {
            Button(action: onTap) {
                rowLayout
                    .contentShape(Rectangle())
            }
            .buttonStyle(GaryxPressableRowStyle())
        } else {
            rowLayout
        }
    }

    @ViewBuilder
    private var rowLayout: some View {
        if valuePlacement == .below || dynamicTypeSize.garyxUsesExpandedReadingLayout {
            stackedRow
        } else {
            trailingRow
        }
    }

    private var trailingRow: some View {
        LabeledContent {
            content
                .font(.body)
                .foregroundStyle(.primary)
                .multilineTextAlignment(.trailing)
                .frame(maxWidth: .infinity, alignment: .trailing)
        } label: {
            GaryxFormFieldTitle(title: title, required: required)
        }
    }

    private var stackedRow: some View {
        VStack(alignment: .leading, spacing: 8) {
            GaryxFormFieldTitle(title: title, required: required)
            content
                .font(.body)
                .foregroundStyle(.primary)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.vertical, 3)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct GaryxFormReadOnlyRow: View {
    let title: String
    let value: String

    var body: some View {
        GaryxFormRow(title: title) {
            Text(value)
                .foregroundStyle(.secondary)
                .garyxReadingLineLimit(2)
                .truncationMode(.middle)
        }
    }
}

struct GaryxFormReadOnlyMultilineRow: View {
    let title: String
    let value: String
    var placeholder: String = ""
    var minHeight: CGFloat = 72
    var valuePlacement: GaryxFormValuePlacement = .trailing

    var body: some View {
        GaryxFormRow(
            title: title,
            valuePlacement: valuePlacement
        ) {
            valueText
                .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .topLeading)
        }
    }

    private var valueText: some View {
        Text(displayValue)
            .font(Font.callout)
            .foregroundStyle(isEmpty ? .secondary : .primary)
            .multilineTextAlignment(.leading)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
    }

    private var isEmpty: Bool {
        value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var displayValue: String {
        if isEmpty {
            return placeholder
        }
        return value
    }
}

struct GaryxFormTextFieldRow: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let title: String
    @Binding var text: String
    var placeholder: String = ""
    var valuePlacement: GaryxFormValuePlacement = .trailing
    var keyboardType: UIKeyboardType = .default
    var textContentType: UITextContentType?
    var autocapitalization: TextInputAutocapitalization?
    var autocorrectionDisabled = false
    var isReadOnly = false
    /// Long values like gateway URLs wrap onto extra lines instead of
    /// truncating, keeping the field name on the left.
    var wrapsValue = false
    /// Tapping the label or surrounding row focuses the field. The field keeps
    /// its own tap handling for caret placement, so
    /// text rows are focus-on-tap, never wrapped in a `Button`.
    @FocusState private var isFocused: Bool

    var body: some View {
        GaryxFormRow(title: title, valuePlacement: valuePlacement) {
            if isReadOnly {
                Text(displayValue)
                    .foregroundStyle(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .secondary : .primary)
                    .garyxReadingLineLimit(wrapsValue ? 3 : 1)
                    .truncationMode(.middle)
            } else {
                editableField
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            guard !isReadOnly else { return }
            isFocused = true
        }
    }

    @ViewBuilder
    private var editableField: some View {
        if wrapsValue || dynamicTypeSize.garyxUsesExpandedReadingLayout {
            TextField(placeholder, text: $text, axis: .vertical)
                .lineLimit(1...(wrapsValue ? 6 : 3))
                .multilineTextAlignment(.leading)
                .fixedSize(horizontal: false, vertical: true)
                .focused($isFocused)
                .accessibilityLabel(title)
                .textFieldStyle(.plain)
                .textContentType(textContentType)
                .textInputAutocapitalization(autocapitalization)
                .autocorrectionDisabled(autocorrectionDisabled)
                .keyboardType(keyboardType)
        } else {
            TextField(placeholder, text: $text)
                .multilineTextAlignment(valuePlacement == .trailing ? .trailing : .leading)
                .focused($isFocused)
                .accessibilityLabel(title)
                .textFieldStyle(.plain)
                .textContentType(textContentType)
                .textInputAutocapitalization(autocapitalization)
                .autocorrectionDisabled(autocorrectionDisabled)
                .keyboardType(keyboardType)
        }
    }

    private var displayValue: String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? placeholder : text
    }
}

struct GaryxFormSecureFieldRow: View {
    let title: String
    @Binding var text: String
    var placeholder: String = ""
    var valuePlacement: GaryxFormValuePlacement = .trailing
    var textContentType: UITextContentType?
    var autocapitalization: TextInputAutocapitalization?
    var autocorrectionDisabled = false
    /// Tap-to-focus on the full row; the field keeps its own tap
    /// handling, so secure rows are never wrapped in a `Button`.
    @FocusState private var isFocused: Bool

    var body: some View {
        GaryxFormRow(title: title, valuePlacement: valuePlacement) {
            SecureField(placeholder, text: $text)
                .textContentType(textContentType)
                .textInputAutocapitalization(autocapitalization)
                .autocorrectionDisabled(autocorrectionDisabled)
                .garyxReadingLineLimit()
                .multilineTextAlignment(valuePlacement == .trailing ? .trailing : .leading)
                .focused($isFocused)
                .accessibilityLabel(title)
                .textFieldStyle(.plain)
        }
        .contentShape(Rectangle())
        .onTapGesture { isFocused = true }
    }
}

struct GaryxFormTextAreaRow: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let title: String
    @Binding var text: String
    var placeholder: String = ""
    var minHeight: CGFloat = 112
    var lineLimits: ClosedRange<Int> = 2...6
    var autocapitalization: TextInputAutocapitalization?
    var autocorrectionDisabled = false
    var isDisabled = false
    var offersFocusedEditor = false
    @FocusState private var isFocused: Bool
    @State private var showsFocusedEditor = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            editorHeader

            editor
                .frame(minHeight: minHeight, alignment: .topLeading)
        }
        .padding(.vertical, 3)
        .frame(maxWidth: .infinity, alignment: .leading)
        .garyxFullScreenCover(isPresented: $showsFocusedEditor) {
            GaryxFocusedTextEditorSheet(
                title: title,
                text: $text,
                placeholder: placeholder,
                autocapitalization: autocapitalization,
                autocorrectionDisabled: autocorrectionDisabled
            )
        }
    }

    @ViewBuilder
    private var editorHeader: some View {
        if offersFocusedEditor, !isDisabled, dynamicTypeSize.isAccessibilitySize {
            VStack(alignment: .leading, spacing: 5) {
                GaryxFormFieldTitle(title: title)
                focusedEditorButton
            }
        } else {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                GaryxFormFieldTitle(title: title)
                Spacer(minLength: 0)
                if offersFocusedEditor, !isDisabled {
                    focusedEditorButton
                }
            }
        }
    }

    private var focusedEditorButton: some View {
        Button("Full Screen") {
            isFocused = false
            showsFocusedEditor = true
        }
        .font(.subheadline)
        .buttonStyle(GaryxPressableRowStyle())
        .foregroundStyle(.secondary)
    }

    private var editor: some View {
        TextField(placeholder, text: $text, axis: .vertical)
            .textInputAutocapitalization(autocapitalization)
            .autocorrectionDisabled(autocorrectionDisabled)
            .font(Font.callout)
            .foregroundStyle(.primary)
            .multilineTextAlignment(.leading)
            .lineLimit(lineLimits)
            .focused($isFocused)
            .accessibilityLabel(title)
            .textFieldStyle(.plain)
            .disabled(isDisabled)
    }
}

private struct GaryxFocusedTextEditorSheet: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    @Binding var text: String
    let placeholder: String
    let autocapitalization: TextInputAutocapitalization?
    let autocorrectionDisabled: Bool
    @State private var draft: String
    @FocusState private var isFocused: Bool

    init(
        title: String,
        text: Binding<String>,
        placeholder: String,
        autocapitalization: TextInputAutocapitalization?,
        autocorrectionDisabled: Bool
    ) {
        self.title = title
        self._text = text
        self.placeholder = placeholder
        self.autocapitalization = autocapitalization
        self.autocorrectionDisabled = autocorrectionDisabled
        self._draft = State(initialValue: text.wrappedValue)
    }

    var body: some View {
        NavigationStack {
            ZStack(alignment: .topLeading) {
                Color(.systemBackground)
                    .ignoresSafeArea()

                if draft.isEmpty, !placeholder.isEmpty {
                    Text(placeholder)
                        .foregroundStyle(.tertiary)
                        .padding(.horizontal, 20)
                        .padding(.top, 16)
                        .allowsHitTesting(false)
                }

                TextEditor(text: $draft)
                    .textInputAutocapitalization(autocapitalization)
                    .autocorrectionDisabled(autocorrectionDisabled)
                    .font(.body)
                    .scrollContentBackground(.hidden)
                    .focused($isFocused)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(action: { dismiss() }) {
                        Text("Cancel")
                            .foregroundStyle(.primary)
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        text = draft
                        dismiss()
                    } label: {
                        Text("Done")
                            .fontWeight(.semibold)
                            .foregroundStyle(.primary)
                    }
                }
            }
            .onAppear {
                draft = text
                isFocused = true
            }
        }
        .tint(GaryxTheme.controlTint)
    }
}

private struct GaryxGatewayHeaderDraftRow: Identifiable {
    let id: UUID
    var name: String
    var value: String

    init(id: UUID = UUID(), name: String = "", value: String = "") {
        self.id = id
        self.name = name
        self.value = value
    }

    init(entry: GaryxGatewayHeaderEntry) {
        self.id = UUID()
        self.name = entry.name
        self.value = entry.value
    }

    var entry: GaryxGatewayHeaderEntry {
        GaryxGatewayHeaderEntry(name: name, value: value)
    }
}

struct GaryxGatewayHeadersEditor: View {
    @Environment(\.garyxMotion) private var motion
    @Binding var text: String
    @State private var rows: [GaryxGatewayHeaderDraftRow] = []
    @State private var lastText = ""
    // Headers are an advanced field, so keep them collapsed by default; only
    // start expanded when the profile already carries configured headers.
    @State private var isExpanded = false

    var body: some View {
        DisclosureGroup(isExpanded: expandedBinding) {
            VStack(alignment: .leading, spacing: 14) {
                ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                    VStack(alignment: .leading, spacing: 10) {
                        HStack {
                            Text("Header \(index + 1)")
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(.secondary)
                            Spacer(minLength: 0)
                            Button(role: .destructive) {
                                removeRow(row.id)
                            } label: {
                                Image(systemName: "trash")
                                    .font(GaryxFont.fixedSystem(size: 13, weight: .semibold))
                                    .frame(width: 32, height: 32)
                            }
                            .buttonStyle(GaryxPressableRowStyle())
                            .disabled(rows.count == 1 && row.name.isEmpty && row.value.isEmpty)
                            .accessibilityLabel("Remove header")
                        }

                        GaryxInlineFormTextField(
                            title: "Name",
                            placeholder: "Header name",
                            accessibilityLabel: "Header \(index + 1) name",
                            text: Binding(
                                get: { value(for: row.id).name },
                                set: { updateRow(row.id, name: $0) }
                            )
                        )

                        GaryxInlineFormTextField(
                            title: "Value",
                            placeholder: "Header value",
                            accessibilityLabel: "Header \(index + 1) value",
                            text: Binding(
                                get: { value(for: row.id).value },
                                set: { updateRow(row.id, value: $0) }
                            )
                        )
                    }

                    if index < rows.count - 1 {
                        Divider()
                    }
                }

                Button(action: addRow) {
                    Label("Add Header", systemImage: "plus")
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                }
                .buttonStyle(GaryxPressableRowStyle())
                .accessibilityLabel("Add header")
            }
            .padding(.top, 8)
        } label: {
            LabeledContent("Headers") {
                if configuredHeaderCount > 0 {
                    Text("\(configuredHeaderCount)")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .onAppear {
            resetRows(from: text)
            isExpanded = configuredHeaderCount > 0
        }
        .onChange(of: text) { _, newValue in
            if newValue != lastText {
                resetRows(from: newValue)
            }
        }
    }

    private var expandedBinding: Binding<Bool> {
        Binding(
            get: { isExpanded },
            set: { next in
                withAnimation(motion.animation(.formDisclosure)) {
                    isExpanded = next
                }
            }
        )
    }

    private var configuredHeaderCount: Int {
        GaryxGatewayHeaders.parseEntries(text).count
    }

    private func value(for id: UUID) -> GaryxGatewayHeaderDraftRow {
        rows.first(where: { $0.id == id }) ?? GaryxGatewayHeaderDraftRow(id: id)
    }

    private func updateRow(_ id: UUID, name: String? = nil, value: String? = nil) {
        var nextRows = rows
        guard let index = nextRows.firstIndex(where: { $0.id == id }) else { return }
        if let name {
            nextRows[index].name = name
        }
        if let value {
            nextRows[index].value = value
        }
        setRowsAndEmit(nextRows)
    }

    private func addRow() {
        rows.append(GaryxGatewayHeaderDraftRow())
    }

    private func removeRow(_ id: UUID) {
        let nextRows = rows.filter { $0.id != id }
        setRowsAndEmit(nextRows.isEmpty ? [GaryxGatewayHeaderDraftRow()] : nextRows)
    }

    private func resetRows(from value: String) {
        lastText = value
        let parsedRows = GaryxGatewayHeaders.parseEntries(value).map(GaryxGatewayHeaderDraftRow.init(entry:))
        rows = parsedRows.isEmpty ? [GaryxGatewayHeaderDraftRow()] : parsedRows
    }

    private func setRowsAndEmit(_ nextRows: [GaryxGatewayHeaderDraftRow]) {
        rows = nextRows
        let nextText = GaryxGatewayHeaders.format(nextRows.map(\.entry))
        lastText = nextText
        text = nextText
    }
}

private struct GaryxInlineFormTextField: View {
    let title: String
    let placeholder: String
    let accessibilityLabel: String
    @Binding var text: String

    var body: some View {
        LabeledContent(title) {
            TextField(placeholder, text: $text)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled(true)
                .multilineTextAlignment(.trailing)
                .textFieldStyle(.plain)
                .accessibilityLabel(accessibilityLabel)
        }
    }
}

struct GaryxFormMenuValueLabel: View {
    let value: String

    var body: some View {
        HStack(spacing: 6) {
            Text(value)
                .font(Font.callout.weight(.medium))
                .foregroundStyle(.primary)
                .garyxReadingLineLimit(2)
                .truncationMode(.middle)
            Image(systemName: "chevron.down")
                .font(GaryxFont.fixedSystem(size: 10, weight: .semibold))
                .foregroundStyle(.tertiary)
                .accessibilityHidden(true)
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

struct GaryxFormSelectionRow: View {
    let title: String
    let value: String
    let placeholder: String
    let action: () -> Void

    var body: some View {
        GaryxFormRow(title: title, onTap: action) {
            HStack(spacing: 7) {
                Text(displayValue)
                    .font(Font.callout.weight(isPlaceholder ? .regular : .medium))
                    .foregroundStyle(isPlaceholder ? .secondary : .primary)
                    .garyxReadingLineLimit(2)
                    .truncationMode(.middle)
                Image(systemName: "chevron.right")
                    .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
        .accessibilityValue(displayValue)
    }

    private var displayValue: String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? placeholder : value
    }

    private var isPlaceholder: Bool {
        value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

/// A form row whose entire width is the label of a `Menu` (D10). A `Menu` cannot
/// be nested inside an outer `Button`, so the correct full-row fix for menu rows
/// is to make the whole row the menu label, replacing the dead-click
/// `GaryxFormRow { Menu { … } }` anti-pattern. `menuContent` is the menu body
/// (buttons or an inline `Picker`); `valueLabel` is the trailing value shown in
/// the row.
struct GaryxFormMenuRow<MenuContent: View, ValueLabel: View>: View {
    let title: String
    let required: Bool
    let menuContent: MenuContent
    let valueLabel: ValueLabel

    init(
        title: String,
        required: Bool = false,
        @ViewBuilder menuContent: () -> MenuContent,
        @ViewBuilder valueLabel: () -> ValueLabel
    ) {
        self.title = title
        self.required = required
        self.menuContent = menuContent()
        self.valueLabel = valueLabel()
    }

    var body: some View {
        Menu {
            menuContent
        } label: {
            GaryxFormRow(title: title, required: required) {
                valueLabel
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
            // Own the native Form cell's full width, including its default
            // leading/trailing inset. Content keeps the same 16-point visual
            // alignment while the Menu hit region covers the white row edge.
            .padding(.horizontal, 16)
            .frame(maxWidth: .infinity, minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .listRowInsets(EdgeInsets())
    }
}

extension GaryxFormMenuRow where ValueLabel == GaryxFormMenuValueLabel {
    /// Convenience for the common "single value + chevron" trailing label.
    init(
        title: String,
        value: String,
        required: Bool = false,
        @ViewBuilder menuContent: () -> MenuContent
    ) {
        self.init(title: title, required: required, menuContent: menuContent) {
            GaryxFormMenuValueLabel(value: value)
        }
    }
}

struct GaryxFormErrorText: View {
    let text: String

    var body: some View {
        Text(text)
            .font(Font.caption.weight(.medium))
            .foregroundStyle(GaryxTheme.danger)
            .fixedSize(horizontal: false, vertical: true)
    }
}

func garyxIsAbsoluteWorkspacePath(_ path: String) -> Bool {
    let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return false }
    if trimmed.hasPrefix("/") || trimmed.hasPrefix("\\\\") { return true }
    let chars = Array(trimmed)
    guard chars.count >= 3 else { return false }
    let first = chars[0]
    let second = chars[1]
    let third = chars[2]
    return first.isLetter && second == ":" && (third == "/" || third == "\\")
}

struct GaryxWorkspacePathSelectionRow: View {
    let title: String
    @Binding var path: String
    var placeholder: String = "Choose workspace"
    var allowsEmpty: Bool = true
    @State private var showsPicker = false

    var body: some View {
        GaryxFormRow(title: title, onTap: { showsPicker = true }) {
            HStack(spacing: 7) {
                Text(displayValue)
                    .font(Font.callout.weight(path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .regular : .medium))
                    .foregroundStyle(path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .secondary : .primary)
                    .garyxReadingLineLimit()
                    .truncationMode(.tail)
                    .multilineTextAlignment(.trailing)
                Image(systemName: "chevron.right")
                    .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
        .accessibilityValue(displayValue)
        .garyxSheet(isPresented: $showsPicker) {
            GaryxWorkspaceSelectSheet(
                title: title,
                path: $path,
                placeholder: placeholder,
                allowsEmpty: allowsEmpty
            )
        }
    }

    private var displayValue: String {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return placeholder }
        return trimmed.garyxLastPathComponent.isEmpty ? trimmed : trimmed.garyxLastPathComponent
    }
}

/// The shared workspace picker content: the server-ordered catalog rendered
/// verbatim (pinned rows first as delivered), a search field over name and
/// path, an explicit "No workspace" choice, and the add-workspace flow.
struct GaryxWorkspaceSelectSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let title: String
    @Binding var path: String
    let placeholder: String
    let allowsEmpty: Bool
    @State private var searchText = ""
    @State private var showsAddWorkspace = false
    @State private var isAddingWorkspace = false

    private var trimmedPath: String {
        path.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedSelectedPath: String {
        normalizedWorkspacePath(trimmedPath)
    }

    /// Server order verbatim — the catalog arrives pre-sorted
    /// (pinned → activity → name → path); the client never re-sorts.
    private var visibleWorkspaces: [GaryxWorkspaceSummary] {
        let needle = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspaces = model.workspaceCatalog.workspaces
        guard !needle.isEmpty else { return workspaces }
        return workspaces.filter {
            $0.name.localizedCaseInsensitiveContains(needle)
                || $0.path.localizedCaseInsensitiveContains(needle)
        }
    }

    private var selectedPathMissingFromOptions: Bool {
        !trimmedPath.isEmpty
            && !model.workspaceCatalog.workspaces.contains {
                normalizedWorkspacePath($0.path) == normalizedSelectedPath
            }
    }

    var body: some View {
        VStack(spacing: 0) {
            sheetHeader(title: title)
            searchField
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    VStack(spacing: 0) {
                        if allowsEmpty {
                            workspaceOptionRow(
                                title: "No workspace",
                                detail: "",
                                systemName: "minus.circle",
                                isSelected: trimmedPath.isEmpty
                            ) {
                                path = ""
                                dismiss()
                            }
                            if !visibleWorkspaces.isEmpty || selectedPathMissingFromOptions {
                                Divider().padding(.leading, 52)
                            }
                        }
                        if selectedPathMissingFromOptions {
                            workspaceOptionRow(
                                title: workspaceDisplayName(trimmedPath),
                                detail: abbreviatedDetail(trimmedPath),
                                systemName: "folder",
                                isSelected: true,
                                badge: "Current"
                            ) {
                                dismiss()
                            }
                            if !visibleWorkspaces.isEmpty {
                                Divider().padding(.leading, 52)
                            }
                        }
                        ForEach(Array(visibleWorkspaces.enumerated()), id: \.element.path) { index, workspace in
                            workspaceOptionRow(
                                title: workspace.name,
                                detail: abbreviatedDetail(workspace.path),
                                systemName: "folder",
                                isSelected: normalizedWorkspacePath(workspace.path) == normalizedSelectedPath,
                                pinned: workspace.pinned
                            ) {
                                path = workspace.path
                                dismiss()
                            }
                            if index < visibleWorkspaces.count - 1 {
                                Divider().padding(.leading, 52)
                            }
                        }
                        Divider().padding(.leading, 52)
                        workspaceOptionRow(
                            title: isAddingWorkspace ? "Adding workspace..." : "Add workspace",
                            detail: "",
                            systemName: isAddingWorkspace ? "hourglass" : "plus.circle",
                            isSelected: false,
                            showsChevron: true
                        ) {
                            showsAddWorkspace = true
                        }
                        .disabled(isAddingWorkspace)
                    }
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
                .garyxVerticalScrollContentWidth()
            }
            .scrollIndicators(.hidden)
        }
        .garyxWorkspacePickerSheetStyle()
        .garyxSheet(isPresented: $showsAddWorkspace) {
            GaryxWorkspacePathPickerSheet(title: "Add Workspace") { selectedPath in
                Task { await addWorkspace(selectedPath) }
            }
        }
        .task {
            await model.refreshWorkspaces()
        }
        .onChange(of: model.workspaceCatalogState.phase) { _, phase in
            // A gateway switch resets the catalog to idle; the picker belongs
            // to the previous universe and must not survive it.
            if phase == .idle {
                dismiss()
            }
        }
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .font(GaryxFont.fixedSystem(size: 13, weight: .semibold))
                .foregroundStyle(.secondary)
            TextField("Search workspaces", text: $searchText)
                .font(Font.subheadline)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            if !searchText.isEmpty {
                Button {
                    searchText = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(GaryxFont.fixedSystem(size: 14, weight: .regular))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 10)
        .frame(minHeight: 36)
        .background(Color(.tertiarySystemFill).opacity(0.72), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .padding(.horizontal, 22)
        .padding(.bottom, 12)
    }

    private func abbreviatedDetail(_ path: String) -> String {
        GaryxMobileWorkspacePresentation.abbreviatedPath(
            normalizedWorkspacePath(path),
            gatewayHome: model.gatewayHomePath
        )
    }

    private func workspaceOptionRow(
        title: String,
        detail: String,
        systemName: String,
        isSelected: Bool,
        badge: String? = nil,
        pinned: Bool = false,
        showsChevron: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: systemName)
                    .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                    .foregroundStyle(isSelected ? .primary : .secondary)
                    .frame(width: 28, height: 28)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(title)
                            .font(Font.subheadline.weight(.semibold))
                            .foregroundStyle(.primary)
                            .garyxReadingLineLimit()
                        if pinned {
                            Image(systemName: "pin.fill")
                                .font(GaryxFont.fixedSystem(size: 10, weight: .semibold))
                                .foregroundStyle(.secondary)
                                .accessibilityLabel("Pinned")
                        }
                        if let badge {
                            Text(badge)
                                .font(Font.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .garyxReadingLineLimit()
                                .padding(.horizontal, 6)
                                .padding(.vertical, 1.5)
                                .background(
                                    Color(.secondarySystemFill),
                                    in: RoundedRectangle(cornerRadius: 5, style: .continuous)
                                )
                                // The option badge stays inline beside the
                                // workspace name, so its growth stops at XXL.
                                .garyxTypographyBoundary(.compactBadgeChrome)
                        }
                    }
                    if !detail.isEmpty {
                        Text(detail)
                            .font(Font.caption)
                            .foregroundStyle(.secondary)
                            .garyxReadingLineLimit()
                            .truncationMode(.middle)
                    }
                }
                Spacer(minLength: 0)
                if isSelected {
                    GaryxSelectionCheckmark(size: 12)
                } else if showsChevron {
                    Image(systemName: "chevron.right")
                        .font(GaryxFont.fixedSystem(size: 12, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, minHeight: 50, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
    }

    private func addWorkspace(_ selectedPath: String) async {
        guard !isAddingWorkspace else { return }
        isAddingWorkspace = true
        defer { isAddingWorkspace = false }
        if let addedPath = await model.addUserWorkspacePath(selectedPath) {
            path = addedPath
            showsAddWorkspace = false
            dismiss()
        }
    }
}

/// The remote directory chooser sheet. It writes the chosen folder into
/// `path` or reports it through `onConfirm`. Add-workspace flows name the
/// workspace by folder basename; renaming lives in the workspace menus.
struct GaryxWorkspacePathPickerSheet: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    var path: Binding<String>? = nil
    var onConfirm: ((String) -> Void)? = nil

    init(title: String, path: Binding<String>) {
        self.title = title
        self.path = path
        self.onConfirm = nil
    }

    init(title: String, onConfirm: @escaping (String) -> Void) {
        self.title = title
        self.path = nil
        self.onConfirm = onConfirm
    }

    var body: some View {
        VStack(spacing: 0) {
            sheetHeader(title: title)

            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    GaryxWorkspaceDirectoryBrowser(
                        selectedPath: path?.wrappedValue ?? "",
                        onSelect: { selectedPath in
                            if let onConfirm {
                                onConfirm(selectedPath)
                            } else {
                                path?.wrappedValue = selectedPath
                            }
                            dismiss()
                        }
                    )
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
                .garyxVerticalScrollContentWidth()
            }
            .scrollIndicators(.hidden)
        }
        .garyxWorkspacePickerSheetStyle()
    }
}

/// Remote directory browser v2: editable path bar with breadcrumb segment
/// jumps, a local filter, git-repo badges, and typed-error inline rendering
/// that stays put on failure (`GaryxWorkspaceDirectoryBrowserState`).
private struct GaryxWorkspaceDirectoryBrowser: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let selectedPath: String
    let onSelect: (String) -> Void
    @State private var browser = GaryxWorkspaceDirectoryBrowserState()
    @State private var isEditingPath = false
    @State private var pathDraft = ""
    @FocusState private var pathFieldFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            headerRow
                .padding(.horizontal, 8)
                .padding(.vertical, 8)

            if let inlineError = browser.inlineError {
                Text(inlineError.message)
                    .font(Font.caption.weight(.medium))
                    .foregroundStyle(GaryxTheme.danger)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 8)
                    .padding(.bottom, 8)
            }

            filterField
                .padding(.horizontal, 8)
                .padding(.bottom, 8)

            Divider().padding(.leading, 8)

            if browser.isLoading, browser.listing == nil {
                Text("Loading folders...")
                    .font(Font.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 26)
            } else if browser.filteredEntries.isEmpty {
                Text(browser.filterText.isEmpty ? "No folders here." : "No matching folders.")
                    .font(Font.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 26)
            } else {
                ForEach(Array(browser.filteredEntries.enumerated()), id: \.element.id) { index, entry in
                    GaryxWorkspaceDirectoryBrowserRow(
                        entry: entry,
                        detail: abbreviated(entry.path),
                        showsSeparator: index < browser.filteredEntries.count - 1
                    ) {
                        navigate(to: entry.path)
                    }
                }
            }
        }
        .task {
            let initial = selectedPath.trimmingCharacters(in: .whitespacesAndNewlines)
            await load(path: initial.isEmpty ? nil : initial)
        }
    }

    private var headerRow: some View {
        HStack(spacing: 10) {
            Button {
                if let parentPath = browser.parentPath {
                    navigate(to: parentPath)
                }
            } label: {
                Image(systemName: "chevron.left")
                    .font(GaryxFont.fixedSystem(size: 13, weight: .semibold))
                    .foregroundStyle(.primary)
                    .frame(width: 32, height: 32)
                    .background(Color(.tertiarySystemFill).opacity(0.72), in: Circle())
            }
            .buttonStyle(GaryxPressableRowStyle())
            .disabled(browser.parentPath == nil || browser.isLoading)
            .opacity(browser.parentPath == nil ? 0.36 : 1)
            .accessibilityLabel("Back")

            if isEditingPath {
                pathField
            } else {
                breadcrumb
            }

            Spacer(minLength: 0)

            if let currentPath = browser.currentPath, !isEditingPath {
                Button {
                    onSelect(currentPath)
                } label: {
                    HStack(spacing: 5) {
                        if normalizedWorkspacePath(selectedPath) == normalizedWorkspacePath(currentPath) {
                            GaryxSelectionCheckmark(size: 11)
                        }
                        Text(normalizedWorkspacePath(selectedPath) == normalizedWorkspacePath(currentPath) ? "Selected" : "Use this folder")
                            .font(Font.caption.weight(.semibold))
                    }
                    .foregroundStyle(.primary)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .frame(minHeight: 30)
                    .background(Color(.tertiarySystemFill).opacity(0.72), in: Capsule())
                }
                .buttonStyle(GaryxPressableRowStyle())
            }
        }
    }

    private var breadcrumb: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal) {
                HStack(spacing: 2) {
                    ForEach(browser.pathSegments) { segment in
                        if segment.path != "/" {
                            Text("/")
                                .font(Font.caption)
                                .foregroundStyle(.tertiary)
                        }
                        Button {
                            navigate(to: segment.path)
                        } label: {
                            Text(segment.label)
                                .font(Font.caption.weight(segment.path == browser.currentPath ? .semibold : .regular))
                                .foregroundStyle(segment.path == browser.currentPath ? .primary : .secondary)
                                .padding(.vertical, 6)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .id(segment.path)
                    }
                    Button {
                        beginEditingPath()
                    } label: {
                        Image(systemName: "pencil")
                            .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: 24, height: 24)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Edit path")
                }
            }
            .scrollIndicators(.hidden)
            .onChange(of: browser.currentPath) { _, currentPath in
                guard let currentPath else { return }
                proxy.scrollTo(currentPath, anchor: .trailing)
            }
        }
        .onTapGesture {
            beginEditingPath()
        }
    }

    private var pathField: some View {
        TextField("/absolute/path", text: $pathDraft)
            .font(Font.caption.monospaced())
            .autocorrectionDisabled()
            .textInputAutocapitalization(.never)
            .keyboardType(.URL)
            .focused($pathFieldFocused)
            .onSubmit {
                submitTypedPath()
            }
            .onChange(of: pathFieldFocused) { _, focused in
                if !focused {
                    isEditingPath = false
                }
            }
            .padding(.horizontal, 8)
            .frame(minHeight: 32)
            .background(Color(.tertiarySystemFill).opacity(0.72), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .accessibilityLabel("Directory path")
    }

    private var filterField: some View {
        HStack(spacing: 6) {
            Image(systemName: "line.3.horizontal.decrease")
                .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                .foregroundStyle(.tertiary)
            TextField("Filter folders", text: $browser.filterText)
                .font(Font.caption)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            if !browser.filterText.isEmpty {
                Button {
                    browser.filterText = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(GaryxFont.fixedSystem(size: 12, weight: .regular))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear filter")
            }
        }
        .padding(.horizontal, 8)
        .frame(minHeight: 30)
        .background(Color(.tertiarySystemFill).opacity(0.5), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    private func beginEditingPath() {
        pathDraft = browser.currentPath ?? ""
        isEditingPath = true
        pathFieldFocused = true
    }

    private func submitTypedPath() {
        guard let target = browser.normalizeTypedPath(pathDraft) else {
            isEditingPath = false
            return
        }
        isEditingPath = false
        navigate(to: target)
    }

    private func navigate(to path: String) {
        Task { await load(path: path) }
    }

    private func load(path: String?) async {
        browser.beginLoad()
        do {
            let listing = try await model.listWorkspaceDirectories(path: path)
            browser.apply(listing)
        } catch {
            browser.fail(error)
        }
    }

    private func abbreviated(_ path: String) -> String {
        GaryxMobileWorkspacePresentation.abbreviatedPath(
            normalizedWorkspacePath(path),
            gatewayHome: model.gatewayHomePath
        )
    }
}

private struct GaryxWorkspaceDirectoryBrowserRow: View {
    let entry: GaryxWorkspaceDirectoryEntry
    let detail: String
    let showsSeparator: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    Image(systemName: "folder")
                        .font(GaryxFont.fixedSystem(size: 15, weight: .medium))
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 28)
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(entry.name)
                                .font(Font.subheadline.weight(.semibold))
                                .foregroundStyle(.primary)
                                .garyxReadingLineLimit()
                            if entry.gitRepo {
                                Image(systemName: "arrow.triangle.branch")
                                    .font(GaryxFont.fixedSystem(size: 10, weight: .semibold))
                                    .foregroundStyle(.secondary)
                                    .accessibilityLabel("Git repository")
                            }
                        }
                        Text(detail)
                            .font(Font.caption)
                            .foregroundStyle(.secondary)
                            .garyxReadingLineLimit()
                            .truncationMode(.middle)
                    }
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(GaryxFont.fixedSystem(size: 12, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 8)
                .frame(minHeight: 50)
                if showsSeparator {
                    Divider().padding(.leading, 46)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
    }
}

private func normalizedWorkspacePath(_ path: String) -> String {
    var trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines).replacingOccurrences(of: "\\", with: "/")
    while trimmed.count > 1, trimmed.hasSuffix("/") {
        if trimmed == "//" { break }
        if trimmed.count == 3, Array(trimmed)[1] == ":" { break }
        trimmed.removeLast()
    }
    return trimmed
}

private func workspaceDisplayName(_ path: String) -> String {
    let tail = path.garyxLastPathComponent
    return tail.isEmpty ? path : tail
}

private func sheetHeader(title: String) -> some View {
    HStack(alignment: .center, spacing: 12) {
        Text(title)
            .font(Font.callout.weight(.medium))
            .foregroundStyle(.primary)
            .garyxReadingLineLimit()
        Spacer(minLength: 0)
        Button {
        } label: {
            EmptyView()
        }
        .hidden()
    }
    .overlay(alignment: .trailing) {
        GaryxDismissButton()
    }
    .padding(.horizontal, 22)
    .padding(.top, 22)
    .padding(.bottom, 14)
}

private struct GaryxDismissButton: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Button {
            dismiss()
        } label: {
            GaryxCompactGlassIcon(systemName: "xmark")
        }
        .buttonStyle(GaryxPressableRowStyle())
        .accessibilityLabel("Close")
    }
}

extension View {
    func garyxWorkspacePickerSheetStyle() -> some View {
        self
            .background {
                Rectangle()
                    .fill(Color(.systemBackground).opacity(0.98))
                    .overlay {
                        LinearGradient(
                            colors: [
                                Color.white.opacity(0.28),
                                Color.white.opacity(0.10)
                            ],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    }
                    .ignoresSafeArea()
            }
            .presentationBackground(.clear)
            .presentationBackgroundInteraction(.enabled)
            .presentationDetents([.fraction(0.93), .large])
            .presentationDragIndicator(.hidden)
            .presentationCornerRadius(38)
    }
}

enum GaryxFormMetrics {
    static let rowInset: CGFloat = 16
    static let rowMinHeight: CGFloat = 54
}

enum GaryxFormPalette {
    static let pageBackground = Color(.systemGroupedBackground)
    static let cardBackground = Color(.secondarySystemGroupedBackground)
}
