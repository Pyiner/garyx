import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxWorkspacesView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var isPickingFiles = false
    @State private var showsAddWorkspace = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Workspaces",
            subtitle: subtitle,
            onRefresh: {
                await model.refreshWorkspaces()
                await model.refreshSelectedWorkspace()
            }
        ) {
            GaryxWorkspacesContent()
        } actions: {
            HStack(spacing: 8) {
                Button {
                    showsAddWorkspace = true
                } label: {
                    GaryxToolbarIcon(systemName: "plus")
                }
                .buttonStyle(GaryxPressableRowStyle())
                .accessibilityLabel("Add Workspace")

                Button {
                    isPickingFiles = true
                } label: {
                    GaryxToolbarIcon(systemName: model.isUploadingWorkspaceFiles ? "hourglass" : "square.and.arrow.up")
                }
                .buttonStyle(GaryxPressableRowStyle())
                .disabled(model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.isUploadingWorkspaceFiles)
                .accessibilityLabel("Upload Files")
            }
        }
        .task {
            await model.refreshWorkspaces()
            await model.prepareWorkspaceBrowser()
        }
        .onChange(of: model.userWorkspacePaths) { _, _ in
            Task { await model.prepareWorkspaceBrowser() }
        }
        .garyxSheet(isPresented: $showsAddWorkspace) {
            GaryxWorkspacePathPickerSheet(title: "Add Workspace") { path in
                guard garyxIsAbsoluteWorkspacePath(path) else { return }
                Task { await addWorkspace(path) }
            }
        }
        .garyxFileImporter(
            isPresented: $isPickingFiles,
            allowedContentTypes: [.item],
            allowsMultipleSelection: true
        ) { result in
            switch result {
            case .success(let urls):
                Task { await model.uploadFilesToSelectedWorkspace(from: urls) }
            case .failure(let error):
                model.lastError = error.localizedDescription
            }
        }
    }

    private var subtitle: String {
        let workspace = model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty else { return "\(model.userWorkspacePaths.count) workspaces" }
        let name = workspace.garyxLastPathComponent.isEmpty ? workspace : workspace.garyxLastPathComponent
        let directory = model.selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        return directory.isEmpty ? name : "\(name) / \(directory)"
    }

    private func addWorkspace(_ path: String) async {
        guard let addedPath = await model.addUserWorkspacePath(path) else { return }
        await model.selectWorkspace(addedPath)
    }
}

struct GaryxWorkspacesContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        let paths = model.userWorkspacePaths
        VStack(alignment: .leading, spacing: 12) {
            if paths.isEmpty {
                if model.isLoadingWorkspaces {
                    GaryxLoadingPanelView(title: "Loading workspaces...")
                } else {
                    GaryxEmptyPanelView(
                        icon: "folder",
                        title: model.workspaceRefreshFailureMessage == nil ? "No workspaces" : "Workspaces unavailable",
                        text: model.workspaceRefreshFailureMessage ?? ""
                    )
                }
            } else {
                GaryxSectionBlock(title: "Workspace") {
                    GaryxCompactListGroup {
                        ForEach(Array(paths.enumerated()), id: \.element) { index, path in
                            GaryxWorkspacePathRow(
                                path: path,
                                isSelected: model.selectedWorkspacePath == path
                            )
                            if index < paths.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }

                GaryxWorkspaceFilesSection()

                if let status = model.workspaceUploadStatus, !status.isEmpty {
                    Text(status)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 2)
                }

                if let preview = model.workspacePreview {
                    GaryxWorkspacePreviewSection(preview: preview)
                }
            }
        }
    }
}

struct GaryxWorkspacePathRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let path: String
    let isSelected: Bool

    var body: some View {
        Button {
            Task { await model.selectWorkspace(path) }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: isSelected ? "folder.fill" : "folder")
                    .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                    .foregroundStyle(isSelected ? .primary : .secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(path.garyxLastPathComponent.isEmpty ? path : path.garyxLastPathComponent)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .garyxReadingLineLimit()
                    Text(
                        GaryxMobileWorkspacePresentation.abbreviatedPath(
                            path,
                            gatewayHome: model.gatewayHomePath
                        )
                    )
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .garyxReadingLineLimit()
                        .truncationMode(.middle)
                }

                Spacer(minLength: 0)

                if isSelected {
                    GaryxSelectionCheckmark(size: 12)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .accessibilityLabel(path.garyxLastPathComponent.isEmpty ? path : path.garyxLastPathComponent)
        .accessibilityValue(
            GaryxMobileWorkspacePresentation.abbreviatedPath(
                path,
                gatewayHome: model.gatewayHomePath
            )
        )
    }
}

struct GaryxWorkspaceFilesSection: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxSectionBlock(title: "Files") {
            if let listing = model.workspaceListing {
                GaryxCompactListGroup {
                    if !model.selectedWorkspaceDirectory.isEmpty {
                        GaryxWorkspaceUpRow()
                        if !listing.entries.isEmpty {
                            GaryxCompactRowDivider()
                        }
                    }
                    ForEach(Array(listing.entries.enumerated()), id: \.element.id) { index, entry in
                        GaryxWorkspaceFileRow(entry: entry)
                        if index < listing.entries.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                    if listing.entries.isEmpty, model.selectedWorkspaceDirectory.isEmpty {
                        GaryxWorkspaceEmptyDirectoryRow()
                    }
                }
            } else {
                GaryxEmptyPanelView(
                    icon: "folder.badge.questionmark",
                    title: "Select a workspace",
                    text: ""
                )
            }
        }
    }
}

struct GaryxWorkspaceUpRow: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        Button {
            Task { await model.goUpWorkspaceDirectory() }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "arrow.turn.up.left")
                    .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)
                Text("Parent Folder")
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
    }
}

struct GaryxWorkspaceEmptyDirectoryRow: View {
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "tray")
                .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 28, height: 28)
            Text("Empty folder")
                .font(GaryxFont.subheadline(weight: .medium))
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 9)
        .frame(minHeight: 52)
    }
}

struct GaryxWorkspaceFileRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let entry: GaryxWorkspaceFileEntry

    var body: some View {
        Button {
            Task { await model.openWorkspaceEntry(entry) }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: iconName)
                    .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                    .foregroundStyle(entry.entryType == "directory" ? .primary : .secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(entry.name.isEmpty ? entry.path.garyxLastPathComponent : entry.name)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .garyxReadingLineLimit()
                    Text(detail)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .garyxReadingLineLimit()
                }

                Spacer(minLength: 0)

                Image(systemName: entry.entryType == "directory" ? "chevron.right" : "doc.text.magnifyingglass")
                    .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .accessibilityLabel(entry.name.isEmpty ? entry.path.garyxLastPathComponent : entry.name)
    }

    private var iconName: String {
        if entry.entryType == "directory" { return "folder" }
        let mediaType = entry.mediaType?.lowercased() ?? ""
        if mediaType.starts(with: "image/") { return "photo" }
        if mediaType == "application/pdf" { return "doc.richtext" }
        if mediaType.starts(with: "text/") { return "doc.text" }
        return "doc"
    }

    private var detail: String {
        if entry.entryType == "directory" {
            return entry.hasChildren ? "Folder" : "Empty folder"
        }
        var parts: [String] = []
        if let size = entry.size {
            parts.append(garyxFormattedFileSize(size))
        }
        if let modified = entry.modifiedAt, !modified.isEmpty {
            parts.append(garyxFormattedTaskTimestamp(modified))
        }
        return parts.isEmpty ? "File" : parts.joined(separator: " · ")
    }
}

struct GaryxWorkspacePreviewSection: View {
    let preview: GaryxWorkspaceFilePreview

    var body: some View {
        GaryxSectionBlock(title: "Preview") {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: previewIconName)
                        .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 28)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(preview.name)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .garyxReadingLineLimit()
                        Text(preview.path)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .garyxReadingLineLimit()
                            .truncationMode(.middle)
                    }
                    Spacer(minLength: 0)
                    GaryxStatusPill(text: preview.previewKind.capitalized, tone: .muted)
                }

                GaryxWorkspacePreviewBody(preview: preview)

                HStack(spacing: 8) {
                    Text(garyxFormattedFileSize(preview.size))
                    if preview.truncated {
                        Text("Truncated")
                    }
                }
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
            }
            .padding(12)
            .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
        }
    }

    private var previewIconName: String {
        switch preview.previewKind {
        case "image":
            "photo"
        case "pdf":
            "doc.richtext"
        case "markdown", "html", "text":
            "doc.text"
        default:
            "doc"
        }
    }
}

private func garyxFormattedFileSize(_ size: Int) -> String {
    ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file)
}

