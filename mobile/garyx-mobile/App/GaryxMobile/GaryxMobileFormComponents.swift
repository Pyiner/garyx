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
            GaryxWorkspaceSelectSheet(
                title: title,
                path: $path,
                workspacePaths: workspacePaths,
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
    @State private var showsPicker = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button {
                showsPicker = true
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: "folder")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 28)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(pathDisplayTitle)
                            .font(GaryxFont.body(weight: selectedPath.isEmpty ? .regular : .semibold))
                            .foregroundStyle(selectedPath.isEmpty ? .secondary : .primary)
                            .lineLimit(1)
                        if !selectedPath.isEmpty {
                            Text(workspacePathCompactLabel(selectedPath))
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.up.chevron.down")
                        .font(GaryxFont.system(size: 12, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .padding(.horizontal, 16)
                .frame(minHeight: 56)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if !path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
               !garyxIsAbsoluteWorkspacePath(path) {
                GaryxFormErrorText(text: "Use an absolute path.")
            }
        }
        .sheet(isPresented: $showsPicker) {
            GaryxWorkspacePathPickerSheet(
                title: "Choose workspace",
                path: $path
            )
        }
    }

    private var selectedPath: String {
        path.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var pathDisplayTitle: String {
        guard !selectedPath.isEmpty else { return placeholder }
        let tail = selectedPath.garyxLastPathComponent
        return tail.isEmpty ? selectedPath : tail
    }
}

struct GaryxWorkspaceSelectSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let title: String
    @Binding var path: String
    let workspacePaths: [String]
    let placeholder: String
    let allowsEmpty: Bool
    @State private var showsAddWorkspace = false
    @State private var addWorkspacePath = ""
    @State private var isAddingWorkspace = false

    private var trimmedPath: String {
        path.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedSelectedPath: String {
        normalizedWorkspacePath(trimmedPath)
    }

    private var visibleWorkspacePaths: [String] {
        var seen = Set<String>()
        return workspacePaths
            .compactMap { rawPath -> String? in
                let path = rawPath.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !path.isEmpty else { return nil }
                return seen.insert(normalizedWorkspacePath(path)).inserted ? path : nil
            }
    }

    private var selectedPathMissingFromOptions: Bool {
        !trimmedPath.isEmpty
            && !visibleWorkspacePaths.contains { normalizedWorkspacePath($0) == normalizedSelectedPath }
    }

    var body: some View {
        VStack(spacing: 0) {
            sheetHeader(title: title)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
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
                                if !visibleWorkspacePaths.isEmpty || selectedPathMissingFromOptions {
                                    Divider().padding(.leading, 52)
                                }
                            }
                            if selectedPathMissingFromOptions {
                                workspaceOptionRow(
                                    title: workspaceDisplayName(trimmedPath),
                                    detail: trimmedPath,
                                    systemName: "folder",
                                    isSelected: true,
                                    badge: "Current"
                                ) {
                                    dismiss()
                                }
                                if !visibleWorkspacePaths.isEmpty {
                                    Divider().padding(.leading, 52)
                                }
                            }
                            ForEach(Array(visibleWorkspacePaths.enumerated()), id: \.element) { index, workspace in
                                workspaceOptionRow(
                                    title: workspaceDisplayName(workspace),
                                    detail: workspace,
                                    systemName: "folder",
                                    isSelected: normalizedWorkspacePath(workspace) == normalizedSelectedPath
                                ) {
                                    path = workspace
                                    dismiss()
                                }
                                if index < visibleWorkspacePaths.count - 1 {
                                    Divider().padding(.leading, 52)
                                }
                            }
                        }
                        .padding(.horizontal, 10)
                        .padding(.vertical, 8)
                    }

                    Button {
                        addWorkspacePath = ""
                        showsAddWorkspace = true
                    } label: {
                        HStack(spacing: 10) {
                            Image(systemName: isAddingWorkspace ? "hourglass" : "plus.circle")
                                .font(GaryxFont.system(size: 15, weight: .semibold))
                                .frame(width: 28, height: 28)
                            Text(isAddingWorkspace ? "Adding workspace..." : "Add workspace")
                                .font(GaryxFont.body(weight: .semibold))
                            Spacer(minLength: 0)
                            Image(systemName: "chevron.right")
                                .font(GaryxFont.system(size: 12, weight: .semibold))
                                .foregroundStyle(.tertiary)
                        }
                        .foregroundStyle(.primary)
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
                    .disabled(isAddingWorkspace)
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
            }
            .scrollIndicators(.hidden)
        }
        .garyxWorkspacePickerSheetStyle()
        .sheet(isPresented: $showsAddWorkspace) {
            GaryxWorkspacePathPickerSheet(
                title: "Add workspace",
                path: $addWorkspacePath
            )
        }
        .onChange(of: addWorkspacePath) { _, newValue in
            let selected = newValue.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !selected.isEmpty else { return }
            Task { await addWorkspace(selected) }
        }
    }

    private func workspaceOptionRow(
        title: String,
        detail: String,
        systemName: String,
        isSelected: Bool,
        badge: String? = nil,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: systemName)
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(isSelected ? .primary : .secondary)
                    .frame(width: 28, height: 28)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(title)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        if let badge {
                            Text(badge)
                                .font(GaryxFont.caption(weight: .semibold))
                                .foregroundStyle(.secondary)
                        }
                    }
                    if !detail.isEmpty {
                        Text(workspacePathCompactLabel(detail))
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Spacer(minLength: 0)
                if isSelected {
                    GaryxSelectionCheckmark(size: 12)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 8)
            .frame(minHeight: 50)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
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

struct GaryxWorkspacePathPickerSheet: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    @Binding var path: String

    var body: some View {
        VStack(spacing: 0) {
            sheetHeader(title: title)

            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
                        GaryxWorkspaceDirectoryBrowser(selectedPath: path) { selectedPath in
                            path = selectedPath
                            dismiss()
                        }
                            .padding(.horizontal, 10)
                            .padding(.vertical, 8)
                    }
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
            }
            .scrollIndicators(.hidden)
        }
        .garyxWorkspacePickerSheetStyle()
    }
}

private struct GaryxWorkspaceDirectoryBrowser: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let selectedPath: String
    let onSelect: (String) -> Void
    @State private var currentPath = ""
    @State private var parentPath: String?
    @State private var entries: [GaryxWorkspaceDirectoryEntry] = []
    @State private var isLoading = true
    @State private var errorText: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                Button {
                    if let parentPath {
                        currentPath = parentPath
                        Task { await load(path: parentPath) }
                    }
                } label: {
                    Image(systemName: "chevron.left")
                        .font(GaryxFont.system(size: 13, weight: .semibold))
                        .foregroundStyle(.primary)
                        .frame(width: 32, height: 32)
                        .background(Color(.tertiarySystemFill).opacity(0.72), in: Circle())
                }
                .buttonStyle(.plain)
                .disabled(parentPath == nil || isLoading)
                .opacity(parentPath == nil ? 0.36 : 1)
                .accessibilityLabel("Back")

                VStack(alignment: .leading, spacing: 2) {
                    Text(workspaceDisplayName(currentPath).isEmpty ? "Folders" : workspaceDisplayName(currentPath))
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(currentPath.isEmpty ? "Choose a folder" : workspacePathCompactLabel(currentPath))
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 0)
                if !currentPath.isEmpty {
                    Button {
                        onSelect(currentPath)
                    } label: {
                        HStack(spacing: 5) {
                            if normalizedWorkspacePath(selectedPath) == normalizedWorkspacePath(currentPath) {
                                GaryxSelectionCheckmark(size: 11)
                            }
                            Text(normalizedWorkspacePath(selectedPath) == normalizedWorkspacePath(currentPath) ? "Selected" : "Use this folder")
                                .font(GaryxFont.caption(weight: .semibold))
                        }
                        .foregroundStyle(.primary)
                        .padding(.horizontal, 10)
                        .frame(height: 30)
                        .background(Color(.tertiarySystemFill).opacity(0.72), in: Capsule())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 8)

            Divider().padding(.leading, 8)

            if isLoading {
                Text("Loading folders...")
                    .font(GaryxFont.subheadline())
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 26)
            } else if let errorText {
                Text(errorText)
                    .font(GaryxFont.subheadline())
                    .foregroundStyle(GaryxTheme.danger)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 26)
            } else if entries.isEmpty {
                Text("No folders here.")
                    .font(GaryxFont.subheadline())
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 26)
            } else {
                ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
                    GaryxWorkspaceDirectoryBrowserRow(
                        entry: entry,
                        showsSeparator: index < entries.count - 1
                    ) {
                        currentPath = entry.path
                        Task { await load(path: entry.path) }
                    }
                }
            }
        }
        .task {
            await load(path: selectedPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : selectedPath)
        }
    }

    private func load(path: String?) async {
        isLoading = true
        errorText = nil
        do {
            let listing = try await model.listWorkspaceDirectories(path: path)
            currentPath = listing.path
            parentPath = listing.parentPath
            entries = listing.entries
        } catch {
            errorText = error.localizedDescription
            entries = []
        }
        isLoading = false
    }
}

private struct GaryxWorkspaceDirectoryBrowserRow: View {
    let entry: GaryxWorkspaceDirectoryEntry
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
                        Text(entry.name)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(workspacePathCompactLabel(entry.path))
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(GaryxFont.system(size: 12, weight: .semibold))
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
        .buttonStyle(.plain)
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

private func sheetHeader(title: String) -> some View {
    HStack(alignment: .center, spacing: 12) {
        Text(title)
            .font(GaryxFont.callout(weight: .medium))
            .foregroundStyle(.primary)
            .lineLimit(1)
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
        .buttonStyle(.plain)
        .accessibilityLabel("Close")
    }
}

private extension View {
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
