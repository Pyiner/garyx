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
    var savedWorkspacePaths: [String]? = nil
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
                savedWorkspacePaths: savedWorkspacePaths ?? workspacePaths,
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
                GaryxFormErrorText(text: "Use an absolute path.")
            }
            GaryxWorkspacePathBrowser(path: $path, paths: workspacePaths)
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
    let savedWorkspacePaths: [String]
    let placeholder: String
    let allowsEmpty: Bool
    @State private var draft = ""

    var body: some View {
        let trimmedDraft = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        let isInvalidDraft = !trimmedDraft.isEmpty && !garyxIsAbsoluteWorkspacePath(trimmedDraft)
        let noWorkspaceSelected = trimmedDraft.isEmpty

        VStack(spacing: 0) {
            HStack(alignment: .center, spacing: 12) {
                Text(title)
                    .font(GaryxFont.callout(weight: .medium))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                Spacer(minLength: 0)
                Button {
                    saveAndDismiss()
                } label: {
                    GaryxCompactGlassIcon(systemName: "checkmark")
                        .opacity(canSave ? 1 : 0.38)
                }
                .buttonStyle(.plain)
                .disabled(!canSave)
                .accessibilityLabel("Save")
                Button {
                    dismiss()
                } label: {
                    GaryxCompactGlassIcon(systemName: "xmark")
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close")
            }
            .padding(.horizontal, 22)
            .padding(.top, 22)
            .padding(.bottom, 14)

            VStack(alignment: .leading, spacing: 6) {
                GaryxGlassPathField(placeholder: placeholder, path: $draft)

                if isInvalidDraft {
                    GaryxFormErrorText(text: "Use an absolute path.")
                }
            }
                .padding(.horizontal, 22)
                .padding(.bottom, 14)

            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
                        GaryxWorkspacePathBrowser(path: $draft, paths: workspacePaths, savedPaths: savedWorkspacePaths)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 8)
                    }
                    if allowsEmpty {
                        Button {
                            path = ""
                            dismiss()
                        } label: {
                            HStack(spacing: 10) {
                                Image(systemName: "xmark.circle")
                                    .font(GaryxFont.system(size: 14, weight: .semibold))
                                Text("No workspace")
                                    .font(GaryxFont.body(weight: .medium))
                                Spacer(minLength: 0)
                                if noWorkspaceSelected {
                                    GaryxSelectionCheckmark(size: 12)
                                }
                            }
                            .foregroundStyle(noWorkspaceSelected ? .primary : .secondary)
                            .padding(.horizontal, 18)
                            .frame(minHeight: 50)
                            .garyxAdaptiveGlass(
                                .regular,
                                isInteractive: true,
                                fallbackMaterial: .ultraThinMaterial,
                                in: RoundedRectangle(cornerRadius: 20, style: .continuous)
                            )
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
            }
            .scrollIndicators(.hidden)
        }
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
        .onAppear { draft = path.trimmingCharacters(in: .whitespacesAndNewlines) }
    }

    private func saveAndDismiss() {
        path = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        dismiss()
    }

    private var canSave: Bool {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return allowsEmpty }
        return garyxIsAbsoluteWorkspacePath(trimmed)
    }
}

private struct GaryxGlassPathField: View {
    let placeholder: String
    @Binding var path: String

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 22, style: .continuous)

        HStack(spacing: 10) {
            Image(systemName: "folder")
                .font(GaryxFont.system(size: 15, weight: .medium))
                .foregroundStyle(.secondary)

            TextField(placeholder, text: $path)
                .font(GaryxFont.subheadline())
                .foregroundStyle(.primary)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(1)
                .accessibilityLabel("Workspace path")

            if !path.isEmpty {
                Button {
                    path = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(GaryxFont.system(size: 15, weight: .medium))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear path")
            }
        }
        .padding(.horizontal, 14)
        .frame(height: 38)
        .garyxAdaptiveGlass(
            .regular,
            isInteractive: true,
            tint: Color(.systemBackground).opacity(0.92),
            fallbackMaterial: .ultraThinMaterial,
            in: shape
        )
        .overlay {
            shape
                .stroke(Color.white.opacity(0.34), lineWidth: 0.7)
        }
        .overlay {
            shape
                .stroke(Color.primary.opacity(0.055), lineWidth: 1)
        }
    }
}

private struct GaryxWorkspacePathBrowser: View {
    @Binding var path: String
    let paths: [String]
    var savedPaths: [String]? = nil
    @State private var currentPath = ""

    private var entries: [GaryxWorkspacePathEntry] {
        workspacePathEntries(paths, savedPaths: savedPaths ?? paths)
    }

    private var rows: [GaryxWorkspaceDirectoryCandidate] {
        workspaceDirectoryChildren(currentPath: currentPath, entries: entries)
    }

    private var currentEntry: GaryxWorkspacePathEntry? {
        workspacePathEntry(for: currentPath, entries: entries)
    }

    private var normalizedSelectedPath: String {
        normalizedWorkspacePath(path)
    }

    private var normalizedCurrentPath: String {
        normalizedWorkspacePath(currentEntry?.originalPath ?? currentPath)
    }

    private var canUseCurrentPath: Bool {
        !currentPath.isEmpty && garyxIsAbsoluteWorkspacePath(currentPath)
    }

    var body: some View {
        if !entries.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    Button {
                        currentPath = parentWorkspacePath(currentPath)
                    } label: {
                        Image(systemName: "chevron.left")
                            .font(GaryxFont.system(size: 13, weight: .semibold))
                            .foregroundStyle(.primary)
                            .frame(width: 32, height: 32)
                            .background(Color(.tertiarySystemFill).opacity(0.72), in: Circle())
                    }
                    .buttonStyle(.plain)
                    .disabled(currentPath.isEmpty)
                    .opacity(currentPath.isEmpty ? 0.36 : 1)
                    .accessibilityLabel("Back")
                    .accessibilityHint("Go to parent folder")

                    VStack(alignment: .leading, spacing: 2) {
                        Text(currentPathTitle)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(currentPath.isEmpty ? "Workspace folders" : workspacePathCompactLabel(currentPath))
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    Spacer(minLength: 0)
                    if canUseCurrentPath {
                        Button {
                            path = currentEntry?.originalPath ?? currentPath
                        } label: {
                            HStack(spacing: 5) {
                                if normalizedSelectedPath == normalizedCurrentPath {
                                    GaryxSelectionCheckmark(size: 11)
                                }
                                Text(normalizedSelectedPath == normalizedCurrentPath ? "Selected" : "Use this folder")
                                    .font(GaryxFont.caption(weight: .semibold))
                            }
                            .foregroundStyle(.primary)
                            .padding(.horizontal, 10)
                            .frame(height: 30)
                            .background(Color(.tertiarySystemFill).opacity(0.72), in: Capsule())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(normalizedSelectedPath == normalizedCurrentPath ? "Current path selected" : "Use current folder")
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 8)

                Divider().padding(.leading, 8)

                if rows.isEmpty {
                    Text("No folders here.")
                        .font(GaryxFont.subheadline())
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 26)
                } else {
                    ForEach(Array(rows.enumerated()), id: \.element.id) { index, candidate in
                        GaryxWorkspacePathBrowserRow(
                            candidate: candidate,
                            isSelected: normalizedSelectedPath == normalizedWorkspacePath(candidate.originalPath ?? candidate.path),
                            showsSeparator: index < rows.count - 1
                        ) {
                            currentPath = candidate.path
                        }
                    }
                }
            }
            .onAppear {
                currentPath = initialWorkspaceBrowserPath(entries: entries, selectedPath: path)
            }
            .onChange(of: paths) { _, _ in
                currentPath = initialWorkspaceBrowserPath(entries: entries, selectedPath: path)
            }
        } else {
            Text("No saved workspaces. Enter an absolute path manually.")
                .font(GaryxFont.subheadline())
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 24)
        }
    }

    private var currentPathTitle: String {
        guard !currentPath.isEmpty else { return "Folders" }
        return workspaceDirectoryName(currentPath)
    }
}

private struct GaryxWorkspacePathBrowserRow: View {
    let candidate: GaryxWorkspaceDirectoryCandidate
    let isSelected: Bool
    let showsSeparator: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    Image(systemName: "folder")
                        .font(GaryxFont.system(size: 15, weight: .medium))
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 28)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(candidate.name)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(workspacePathCompactLabel(candidate.path))
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    Spacer(minLength: 0)
                    if isSelected {
                        GaryxSelectionCheckmark(size: 12)
                    }
                    Image(systemName: "chevron.right")
                        .font(GaryxFont.system(size: 12, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 8)
                .frame(minHeight: 50)
                .background {
                    if isSelected {
                        Color(.tertiarySystemFill).opacity(0.56)
                            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    }
                }
                if showsSeparator {
                    Divider().padding(.leading, 46)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
        .accessibilityValue(isSelected ? "Selected" : "")
        .accessibilityHint("Open folder")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var accessibilityLabel: String {
        [
            candidate.name,
            workspacePathCompactLabel(candidate.path)
        ].joined(separator: ", ")
    }
}

private struct GaryxWorkspacePathEntry {
    let normalizedPath: String
    let originalPath: String
}

private struct GaryxWorkspaceDirectoryCandidate: Identifiable {
    let path: String
    let name: String
    let originalPath: String?

    var id: String { path }
}

private func workspacePathEntries(_ paths: [String], savedPaths: [String]) -> [GaryxWorkspacePathEntry] {
    let savedOriginalByNormalized = Dictionary(
        savedPaths.compactMap { rawPath -> (String, String)? in
            let original = rawPath.trimmingCharacters(in: .whitespacesAndNewlines)
            let normalized = normalizedWorkspacePath(original)
            guard garyxIsAbsoluteWorkspacePath(normalized) else { return nil }
            return (normalized, original)
        },
        uniquingKeysWith: { first, _ in first }
    )
    var seen = Set<String>()
    return paths.compactMap { rawPath -> GaryxWorkspacePathEntry? in
        let original = rawPath.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalized = normalizedWorkspacePath(original)
        guard garyxIsAbsoluteWorkspacePath(normalized),
              seen.insert(normalized).inserted else {
            return nil
        }
        if let savedOriginal = savedOriginalByNormalized[normalized] {
            return GaryxWorkspacePathEntry(
                normalizedPath: normalized,
                originalPath: savedOriginal
            )
        }
        return GaryxWorkspacePathEntry(
            normalizedPath: normalized,
            originalPath: original
        )
    }
    .sorted { $0.normalizedPath.localizedStandardCompare($1.normalizedPath) == .orderedAscending }
}

private func workspaceDirectoryChildren(
    currentPath: String,
    entries: [GaryxWorkspacePathEntry]
) -> [GaryxWorkspaceDirectoryCandidate] {
    var candidates: [String: GaryxWorkspaceDirectoryCandidate] = [:]
    for entry in entries {
        guard let childPath = workspaceImmediateChildPath(
            parentPath: currentPath,
            descendantPath: entry.normalizedPath
        ) else { continue }
        let exactEntry = workspacePathEntry(for: childPath, entries: entries)
        candidates[childPath] = GaryxWorkspaceDirectoryCandidate(
            path: childPath,
            name: workspaceDirectoryName(childPath),
            originalPath: exactEntry?.originalPath
        )
    }
    return candidates.values.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
}

private func workspacePathEntry(
    for path: String,
    entries: [GaryxWorkspacePathEntry]
) -> GaryxWorkspacePathEntry? {
    let normalized = normalizedWorkspacePath(path)
    return entries.first { $0.normalizedPath == normalized }
}

private func initialWorkspaceBrowserPath(entries: [GaryxWorkspacePathEntry], selectedPath: String) -> String {
    let normalized = normalizedWorkspacePath(selectedPath)
    if !normalized.isEmpty, garyxIsAbsoluteWorkspacePath(normalized) {
        return parentWorkspacePath(normalized)
    }
    guard let first = entries.first?.normalizedPath else { return "" }
    return parentWorkspacePath(first)
}

private func parentWorkspacePath(_ path: String) -> String {
    let normalized = normalizedWorkspacePath(path)
    let parts = pathComponentsForWorkspacePath(normalized)
    guard !parts.segments.isEmpty else { return "" }
    guard parts.segments.count > 1 else { return parts.root }
    return parts.segments.dropLast().reduce(parts.root) { current, segment in
        childWorkspacePath(parent: current, segment: segment)
    }
}

private func workspaceImmediateChildPath(parentPath: String, descendantPath: String) -> String? {
    let parent = normalizedWorkspacePath(parentPath)
    let descendant = normalizedWorkspacePath(descendantPath)
    guard !descendant.isEmpty, descendant != parent else { return nil }
    if parent.isEmpty {
        let parts = pathComponentsForWorkspacePath(descendant)
        guard let first = parts.segments.first else { return parts.root }
        return childWorkspacePath(parent: parts.root, segment: first)
    }
    if parent == "/" || parent == "//" || parent.hasSuffix(":") {
        let parts = pathComponentsForWorkspacePath(descendant)
        guard parts.root == parent, let first = parts.segments.first else { return nil }
        return childWorkspacePath(parent: parent, segment: first)
    }
    let prefix = parent.hasSuffix("/") ? parent : "\(parent)/"
    guard descendant.hasPrefix(prefix) else { return nil }
    let remainder = String(descendant.dropFirst(prefix.count))
    guard let nextSegment = remainder.split(separator: "/", maxSplits: 1).first else { return nil }
    return childWorkspacePath(parent: parent, segment: String(nextSegment))
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

private func pathComponentsForWorkspacePath(_ path: String) -> (root: String, segments: [String]) {
    let normalized = normalizedWorkspacePath(path)
    if normalized.hasPrefix("//") {
        return ("//", normalized.dropFirst(2).split(separator: "/").map(String.init))
    }
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

private func workspaceDirectoryName(_ path: String) -> String {
    let tail = path.garyxLastPathComponent
    return tail.isEmpty ? path : tail
}

private func childWorkspacePath(parent: String, segment: String) -> String {
    if parent == "/" { return "/\(segment)" }
    if parent == "//" { return "//\(segment)" }
    if parent.hasSuffix(":") { return "\(parent)/\(segment)" }
    return "\(parent)/\(segment)"
}

private func workspacePathCompactLabel(_ path: String) -> String {
    let normalized = normalizedWorkspacePath(path)
    if normalized.hasPrefix("//") {
        let parts = normalized.dropFirst(2).split(separator: "/").map(String.init)
        guard parts.count > 2 else { return normalized }
        return ".../\(parts.suffix(2).joined(separator: "/"))"
    }
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
