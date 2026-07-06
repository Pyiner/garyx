import Foundation

public struct GaryxMobileWorkspaceFileTarget: Equatable, Sendable {
    public var workspaceDir: String
    public var path: String

    public init(workspaceDir: String, path: String) {
        self.workspaceDir = workspaceDir
        self.path = path
    }
}

public enum GaryxMobileFileLink {
    public static func localFilePath(from target: String) -> String? {
        let trimmed = target.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        if trimmed.hasPrefix("/") {
            return normalizeLocalFilePath(trimmed)
        }

        if let url = URL(string: trimmed), url.isFileURL {
            return localFilePath(from: url)
        }

        return nil
    }

    public static func localFilePath(from url: URL) -> String? {
        if url.isFileURL {
            return normalizeLocalFilePath(url.path.removingPercentEncoding ?? url.path)
        }

        let raw = url.absoluteString.trimmingCharacters(in: .whitespacesAndNewlines)
        if raw.hasPrefix("/") {
            return normalizeLocalFilePath(raw)
        }

        return nil
    }

    public static func previewTarget(
        forLocalFilePath absolutePath: String,
        workspacePaths: [String]
    ) -> GaryxMobileWorkspaceFileTarget? {
        let normalizedAbsolutePath = normalizeLocalFilePath(absolutePath) ?? ""
        guard normalizedAbsolutePath.hasPrefix("/") else { return nil }

        let candidates = workspacePaths.compactMap { workspacePath -> GaryxMobileWorkspaceFileTarget? in
            let workspace = normalizeWorkspaceRootPath(workspacePath)
            guard !workspace.isEmpty,
                  let filePath = relativeWorkspaceFilePath(
                    workspacePath: workspace,
                    absolutePath: normalizedAbsolutePath
                  ),
                  !filePath.isEmpty else {
                return nil
            }
            return GaryxMobileWorkspaceFileTarget(workspaceDir: workspace, path: filePath)
        }

        if let match = candidates.sorted(by: { $0.workspaceDir.count > $1.workspaceDir.count }).first {
            return match
        }

        guard let parent = absoluteFileParentPath(normalizedAbsolutePath),
              let fileName = absoluteFileName(normalizedAbsolutePath) else {
            return nil
        }
        return GaryxMobileWorkspaceFileTarget(workspaceDir: parent, path: fileName)
    }

    public static func previewTarget(
        fromLink target: String,
        workspacePaths: [String],
        currentWorkspaceDir: String? = nil,
        currentFilePath: String? = nil
    ) -> GaryxMobileWorkspaceFileTarget? {
        if let absolutePath = localFilePath(from: target) {
            return previewTarget(forLocalFilePath: absolutePath, workspacePaths: workspacePaths)
        }

        guard let workspace = currentWorkspaceDir?.trimmingCharacters(in: .whitespacesAndNewlines),
              !workspace.isEmpty,
              let relativePath = workspaceRelativePath(
                from: target,
                currentFilePath: currentFilePath
              ),
              !relativePath.isEmpty else {
            return nil
        }

        return GaryxMobileWorkspaceFileTarget(workspaceDir: workspace, path: relativePath)
    }

    private static func normalizeLocalFilePath(_ path: String) -> String? {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let withoutQuery = trimmed.split(separator: "?", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? ""
        let withoutFragment = withoutQuery.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? ""
        let decoded = withoutFragment.removingPercentEncoding ?? withoutFragment
        let withoutLineSuffix = stripTrailingLineColumnSuffix(decoded)
        return withoutLineSuffix.hasPrefix("/") ? withoutLineSuffix : nil
    }

    private static func stripTrailingLineColumnSuffix(_ path: String) -> String {
        guard let range = path.range(of: #":\d+(?::\d+)?$"#, options: .regularExpression) else {
            return path
        }
        return String(path[..<range.lowerBound])
    }

    private static func normalizeWorkspaceRootPath(_ path: String) -> String {
        var trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        while trimmed.count > 1, trimmed.hasSuffix("/") {
            trimmed.removeLast()
        }
        return trimmed
    }

    private static func relativeWorkspaceFilePath(
        workspacePath: String,
        absolutePath: String
    ) -> String? {
        let workspace = normalizeWorkspaceRootPath(workspacePath)
        let absolute = absolutePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !workspace.isEmpty, absolute.hasPrefix("/") else { return nil }
        if absolute == workspace { return "" }
        let prefix = workspace == "/" ? "/" : "\(workspace)/"
        guard absolute.hasPrefix(prefix) else { return nil }
        return String(absolute.dropFirst(prefix.count))
    }

    private static func absoluteFileParentPath(_ path: String) -> String? {
        let normalized = normalizeWorkspaceRootPath(path)
        guard normalized.hasPrefix("/") else { return nil }
        let parent = (normalized as NSString).deletingLastPathComponent
        return parent.isEmpty ? "/" : parent
    }

    private static func absoluteFileName(_ path: String) -> String? {
        let normalized = normalizeWorkspaceRootPath(path)
        let name = (normalized as NSString).lastPathComponent
        return name.isEmpty || name == "/" ? nil : name
    }

    private static func workspaceRelativePath(
        from target: String,
        currentFilePath: String?
    ) -> String? {
        let trimmed = stripTrailingLineColumnSuffix(
            target.trimmingCharacters(in: .whitespacesAndNewlines)
        )
        guard !trimmed.isEmpty,
              !trimmed.hasPrefix("#"),
              !trimmed.hasPrefix("?") else {
            return nil
        }

        if let url = URL(string: trimmed),
           let scheme = url.scheme?.lowercased(),
           !scheme.isEmpty {
            guard scheme == "file" else { return nil }
        }

        let withoutQuery = trimmed.split(separator: "?", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? ""
        let withoutFragment = withoutQuery.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? ""
        let decoded = (withoutFragment.removingPercentEncoding ?? withoutFragment)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !decoded.isEmpty,
              !decoded.hasPrefix("/") else {
            return nil
        }

        let currentDirectory = parentRelativeDirectory(currentFilePath)
        let combined = currentDirectory.isEmpty ? decoded : "\(currentDirectory)/\(decoded)"
        return normalizeRelativePath(combined)
    }

    private static func parentRelativeDirectory(_ path: String?) -> String {
        let normalized = normalizeRelativePath(path ?? "") ?? ""
        guard !normalized.isEmpty else { return "" }
        let parent = (normalized as NSString).deletingLastPathComponent
        return parent == "." ? "" : parent
    }

    private static func normalizeRelativePath(_ path: String) -> String? {
        let parts = path
            .replacingOccurrences(of: "\\", with: "/")
            .split(separator: "/", omittingEmptySubsequences: false)
            .map(String.init)
        var stack: [String] = []
        for part in parts {
            if part.isEmpty || part == "." {
                continue
            }
            if part == ".." {
                // Folding below the workspace root means the link escapes the
                // workspace. The gateway rejects any `..` component, so treat
                // the link as unresolvable instead of collapsing it onto a
                // root-level file of the same name.
                guard !stack.isEmpty else { return nil }
                stack.removeLast()
                continue
            }
            stack.append(part)
        }
        let normalized = stack.joined(separator: "/")
        return normalized.isEmpty ? nil : normalized
    }
}
