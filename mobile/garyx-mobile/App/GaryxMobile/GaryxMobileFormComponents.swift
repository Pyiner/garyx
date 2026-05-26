import Foundation
import SwiftUI
import UIKit

struct GaryxFormSheet<Content: View>: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    let canSave: Bool?
    let onCancel: (() -> Void)?
    let onSave: (() -> Void)?
    let onDone: (() -> Void)?
    let content: Content

    init(title: String, onDone: (() -> Void)? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.canSave = nil
        self.onCancel = nil
        self.onSave = nil
        self.onDone = onDone
        self.content = content()
    }

    init(
        title: String,
        canSave: Bool,
        onCancel: (() -> Void)? = nil,
        onSave: @escaping () -> Void,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.canSave = canSave
        self.onCancel = onCancel
        self.onSave = onSave
        self.onDone = nil
        self.content = content()
    }

    var body: some View {
        ZStack(alignment: .top) {
            GaryxFormPalette.pageBackground
                .ignoresSafeArea()

            ScrollView {
                content
                    .padding(.horizontal, 18)
                    .padding(.top, 92)
                    .padding(.bottom, 28)
                    .frame(maxWidth: 560, alignment: .leading)
                    .frame(maxWidth: .infinity)
            }

            ZStack {
                HStack {
                    Button(action: cancel) {
                        GaryxToolbarIcon(systemName: "xmark")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Cancel")

                    Spacer(minLength: 0)

                    if let onSave {
                        Button(action: onSave) {
                            GaryxToolbarIcon(systemName: "checkmark")
                                .opacity(canSave == false ? 0.42 : 1)
                        }
                        .buttonStyle(.plain)
                        .disabled(canSave == false)
                        .accessibilityLabel("Save")
                    }
                }

                Text(title)
                    .font(GaryxFont.title3(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 18)
            .padding(.top, 10)
        }
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
}

struct GaryxFormGroupedSection<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
                .padding(.horizontal, 14)

            VStack(alignment: .leading, spacing: 0) {
                content
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(GaryxFormPalette.cardBackground, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        }
    }
}

struct GaryxFormRow<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        HStack(spacing: 12) {
            Text(title)
                .font(GaryxFont.body())
                .foregroundStyle(.primary)
            Spacer(minLength: 8)
            content
                .font(GaryxFont.body())
                .foregroundStyle(.primary)
                .multilineTextAlignment(.trailing)
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 52)
    }
}

struct GaryxFormReadOnlyRow: View {
    let title: String
    let value: String

    var body: some View {
        GaryxFormRow(title: title) {
            Text(value)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}

struct GaryxFormMenuValueLabel: View {
    let value: String

    var body: some View {
        HStack(spacing: 6) {
            Text(value)
                .font(GaryxFont.body(weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Image(systemName: "chevron.up.chevron.down")
                .font(GaryxFont.system(size: 11, weight: .semibold))
                .foregroundStyle(.tertiary)
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
        Button(action: action) {
            HStack(spacing: 12) {
                Text(title)
                    .font(GaryxFont.body())
                    .foregroundStyle(.primary)
                Spacer(minLength: 8)
                Text(displayValue)
                    .font(GaryxFont.body())
                    .foregroundStyle(isPlaceholder ? .secondary : .primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var displayValue: String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? placeholder : value
    }

    private var isPlaceholder: Bool {
        value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

struct GaryxFormErrorText: View {
    let text: String

    var body: some View {
        Text(text)
            .font(GaryxFont.caption(weight: .medium))
            .foregroundStyle(GaryxTheme.danger)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 14)
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
    let workspacePaths: [String]
    var placeholder: String = "Choose workspace"
    var allowsEmpty: Bool = true
    @State private var showsPicker = false

    var body: some View {
        Button {
            showsPicker = true
        } label: {
            HStack(spacing: 12) {
                Text(title)
                    .font(GaryxFont.body())
                    .foregroundStyle(.primary)
                Spacer(minLength: 8)
                Text(displayValue)
                    .font(GaryxFont.body(weight: path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .regular : .medium))
                    .foregroundStyle(path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .secondary : .primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .sheet(isPresented: $showsPicker) {
            GaryxWorkspacePathPickerSheet(
                title: title,
                path: $path,
                workspacePaths: workspacePaths,
                placeholder: placeholder,
                allowsEmpty: allowsEmpty
            )
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
        }
    }

    private var displayValue: String {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return placeholder }
        return trimmed.garyxLastPathComponent.isEmpty ? trimmed : trimmed.garyxLastPathComponent
    }
}

struct GaryxWorkspacePathPickerField: View {
    @Binding var path: String
    let workspacePaths: [String]
    var placeholder: String = "/path/to/project"

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField(placeholder, text: $path)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxFormTextField()
            if !path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
               !garyxIsAbsoluteWorkspacePath(path) {
                GaryxFormErrorText(text: "Use an absolute directory path.")
            }
            GaryxWorkspacePathTree(
                paths: workspacePaths,
                selectedPath: path,
                onSelect: { path = $0 }
            )
            .padding(.horizontal, 8)
            .padding(.bottom, 8)
        }
    }
}

private struct GaryxWorkspacePathPickerSheet: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    @Binding var path: String
    let workspacePaths: [String]
    let placeholder: String
    let allowsEmpty: Bool
    @State private var draft = ""

    var body: some View {
        GaryxFormSheet(
            title: title,
            canSave: canSave,
            onSave: {
                path = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                dismiss()
            }
        ) {
            VStack(alignment: .leading, spacing: 22) {
                GaryxFormGroupedSection(title: "Directory") {
                    GaryxWorkspacePathPickerField(
                        path: $draft,
                        workspacePaths: workspacePaths,
                        placeholder: placeholder
                    )
                }
                if allowsEmpty {
                    Button {
                        path = ""
                        dismiss()
                    } label: {
                        HStack {
                            Image(systemName: "xmark.circle")
                            Text("No workspace")
                            Spacer(minLength: 0)
                        }
                        .font(GaryxFont.body(weight: .medium))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 16)
                        .frame(minHeight: 52)
                        .background(GaryxFormPalette.cardBackground, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .onAppear { draft = path.trimmingCharacters(in: .whitespacesAndNewlines) }
    }

    private var canSave: Bool {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return allowsEmpty }
        return garyxIsAbsoluteWorkspacePath(trimmed)
    }
}

private struct GaryxWorkspacePathTree: View {
    let paths: [String]
    let selectedPath: String
    let onSelect: (String) -> Void

    var body: some View {
        let rows = workspacePathTreeRows(paths)
        if !rows.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                    Button {
                        if row.isSelectable {
                            onSelect(row.path)
                        }
                    } label: {
                        HStack(spacing: 10) {
                            Spacer(minLength: CGFloat(row.depth) * 14)
                                .frame(width: CGFloat(row.depth) * 14)
                            Image(systemName: row.isSelectable ? "folder.fill" : "folder")
                                .font(GaryxFont.system(size: 14, weight: .semibold))
                                .foregroundStyle(row.isSelectable ? .primary : .secondary)
                                .frame(width: 22)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(row.name)
                                    .font(GaryxFont.subheadline(weight: row.isSelectable ? .semibold : .regular))
                                    .foregroundStyle(row.isSelectable ? .primary : .secondary)
                                    .lineLimit(1)
                                if row.isSelectable {
                                    Text(workspacePathCompactLabel(row.path))
                                        .font(GaryxFont.caption())
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                        .truncationMode(.middle)
                                }
                            }
                            Spacer(minLength: 0)
                            if normalizedWorkspacePath(selectedPath) == normalizedWorkspacePath(row.path), row.isSelectable {
                                GaryxSelectionCheckmark(size: 12)
                            }
                        }
                        .padding(.horizontal, 8)
                        .frame(minHeight: row.isSelectable ? 50 : 38)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(!row.isSelectable)

                    if index < rows.count - 1 {
                        Divider().padding(.leading, 42 + CGFloat(row.depth) * 14)
                    }
                }
            }
        }
    }
}

private struct GaryxWorkspacePathTreeRow: Identifiable {
    let id: String
    let name: String
    let path: String
    let depth: Int
    let isSelectable: Bool
}

private func workspacePathTreeRows(_ paths: [String]) -> [GaryxWorkspacePathTreeRow] {
    let normalized = paths
        .map(normalizedWorkspacePath)
        .filter { garyxIsAbsoluteWorkspacePath($0) }
    var seen = Set<String>()
    var rows: [GaryxWorkspacePathTreeRow] = []

    for path in normalized.sorted(by: { $0.localizedStandardCompare($1) == .orderedAscending }) {
        let parts = pathComponentsForWorkspaceTree(path)
        var current = parts.root
        appendWorkspaceTreeRow(
            &rows,
            seen: &seen,
            name: parts.root,
            path: current,
            depth: 0,
            isSelectable: path == current
        )
        for (index, part) in parts.segments.enumerated() {
            current = childWorkspacePath(parent: current, segment: part)
            appendWorkspaceTreeRow(
                &rows,
                seen: &seen,
                name: part,
                path: current,
                depth: index + 1,
                isSelectable: current == path
            )
        }
    }
    return rows
}

private func appendWorkspaceTreeRow(
    _ rows: inout [GaryxWorkspacePathTreeRow],
    seen: inout Set<String>,
    name: String,
    path: String,
    depth: Int,
    isSelectable: Bool
) {
    let id = "\(path)|\(isSelectable ? "selected" : "folder")"
    if let index = rows.firstIndex(where: { $0.path == path }) {
        if isSelectable, !rows[index].isSelectable {
            rows[index] = GaryxWorkspacePathTreeRow(
                id: id,
                name: rows[index].name,
                path: path,
                depth: rows[index].depth,
                isSelectable: true
            )
        }
        return
    }
    guard seen.insert(path).inserted else { return }
    rows.append(GaryxWorkspacePathTreeRow(
        id: id,
        name: name,
        path: path,
        depth: depth,
        isSelectable: isSelectable
    ))
}

private func normalizedWorkspacePath(_ path: String) -> String {
    var trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines).replacingOccurrences(of: "\\", with: "/")
    while trimmed.count > 1, trimmed.hasSuffix("/") {
        if trimmed.count == 3, Array(trimmed)[1] == ":" { break }
        trimmed.removeLast()
    }
    return trimmed
}

private func pathComponentsForWorkspaceTree(_ path: String) -> (root: String, segments: [String]) {
    let normalized = normalizedWorkspacePath(path)
    if normalized.hasPrefix("/") {
        return ("/", normalized.dropFirst().split(separator: "/").map(String.init))
    }
    let chars = Array(normalized)
    if chars.count >= 2, chars[1] == ":" {
        let root = String(chars[0...1])
        let rest = String(chars.dropFirst(2)).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        return (root, rest.split(separator: "/").map(String.init))
    }
    return ("/", normalized.split(separator: "/").map(String.init))
}

private func childWorkspacePath(parent: String, segment: String) -> String {
    if parent == "/" { return "/\(segment)" }
    if parent.hasSuffix(":") { return "\(parent)/\(segment)" }
    return "\(parent)/\(segment)"
}

private func workspacePathCompactLabel(_ path: String) -> String {
    let normalized = normalizedWorkspacePath(path)
    let parts = normalized.split(separator: "/").map(String.init)
    guard parts.count > 2 else { return normalized }
    return ".../\(parts.suffix(2).joined(separator: "/"))"
}

enum GaryxFormPalette {
    static let pageBackground = Color(.systemGroupedBackground).opacity(0.72)
    static let cardBackground = Color(.systemBackground)
}

extension View {
    func garyxFormTextField(minHeight: CGFloat = 52, horizontalPadding: CGFloat = 16) -> some View {
        self
            .font(GaryxFont.body())
            .foregroundStyle(.primary)
            .padding(.horizontal, horizontalPadding)
            .frame(minHeight: minHeight, alignment: .leading)
    }

    func garyxFormTextArea(minHeight: CGFloat = 132) -> some View {
        self
            .font(GaryxFont.body())
            .foregroundStyle(.primary)
            .padding(16)
            .frame(minHeight: minHeight, alignment: .topLeading)
    }
}
