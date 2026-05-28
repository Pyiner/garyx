import Foundation
import SwiftUI
import UIKit

struct GaryxMarkdownText: View {
    let text: String
    var foreground: Color = .primary
    var codeBackground: Color = GaryxTheme.surface
    var codeBorder: Color = GaryxTheme.hairline
    var fillsAvailableWidth = true
    var allowsRelativeFileLinks = false
    var onFileLinkTap: ((String) -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(GaryxMarkdownRenderCache.shared.blocks(from: text)) { block in
                switch block.kind {
                case .markdown(let markdown):
                    GaryxMarkdownParagraphView(
                        markdown: markdown,
                        foreground: foreground,
                        allowsRelativeFileLinks: allowsRelativeFileLinks,
                        onFileLinkTap: onFileLinkTap
                    )
                case .code(let language, let code):
                    GaryxCodeBlockView(
                        language: language,
                        code: code,
                        foreground: foreground,
                        background: codeBackground,
                        border: codeBorder,
                        fillsAvailableWidth: fillsAvailableWidth
                    )
                case .image(let alt, let source):
                    GaryxMarkdownImageView(alt: alt, source: source)
                case .table(let table):
                    GaryxMarkdownTableView(
                        table: table,
                        foreground: foreground,
                        background: codeBackground,
                        border: codeBorder,
                        fillsAvailableWidth: fillsAvailableWidth,
                        allowsRelativeFileLinks: allowsRelativeFileLinks,
                        onFileLinkTap: onFileLinkTap
                    )
                }
            }
        }
        .frame(maxWidth: fillsAvailableWidth ? .infinity : nil, alignment: .leading)
    }

    fileprivate static func attributedString(from markdown: String) -> AttributedString {
        GaryxMarkdownRenderCache.shared.attributedString(from: markdown)
    }
}

private struct GaryxMarkdownParagraphView: View {
    let markdown: String
    let foreground: Color
    var allowsRelativeFileLinks = false
    var onFileLinkTap: ((String) -> Void)?

    private var lines: [String] {
        markdown.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                if line.trimmingCharacters(in: .whitespaces).isEmpty {
                    Color.clear.frame(height: 8)
                } else if let bullet = Self.bulletText(from: line) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Circle()
                            .fill(foreground)
                            .frame(width: 4, height: 4)
                            .offset(y: -2)

                        Text(GaryxMarkdownText.attributedString(from: bullet))
                            .font(GaryxFont.body())
                            .foregroundStyle(foreground)
                            .tint(GaryxTheme.accent)
                            .environment(\.openURL, openURLAction)
                            .textSelection(.enabled)
                            .lineSpacing(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                } else if let numbered = Self.numberedList(from: line) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(numbered.label)
                            .font(GaryxFont.body(weight: .medium))
                            .foregroundStyle(foreground)
                            .textSelection(.enabled)

                        Text(GaryxMarkdownText.attributedString(from: numbered.text))
                            .font(GaryxFont.body())
                            .foregroundStyle(foreground)
                            .tint(GaryxTheme.accent)
                            .environment(\.openURL, openURLAction)
                            .textSelection(.enabled)
                            .lineSpacing(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                } else {
                    Text(GaryxMarkdownText.attributedString(from: line))
                        .font(GaryxFont.body())
                        .foregroundStyle(foreground)
                        .tint(GaryxTheme.accent)
                        .environment(\.openURL, openURLAction)
                        .textSelection(.enabled)
                        .lineSpacing(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    private var openURLAction: OpenURLAction {
        OpenURLAction { url in
            guard let onFileLinkTap else { return .systemAction }
            let target = GaryxMarkdownLinkTarget.fileTarget(
                from: url,
                allowsRelativeFileLinks: allowsRelativeFileLinks
            )
            guard !target.isEmpty else { return .systemAction }
            onFileLinkTap(target)
            return .handled
        }
    }

    private static func bulletText(from line: String) -> String? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        if trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ") {
            return String(trimmed.dropFirst(2))
        }
        return nil
    }

    private static func numberedList(from line: String) -> (label: String, text: String)? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        var digitPrefix = ""
        var cursor = trimmed.startIndex
        while cursor < trimmed.endIndex, trimmed[cursor].isNumber {
            digitPrefix.append(trimmed[cursor])
            cursor = trimmed.index(after: cursor)
        }
        guard !digitPrefix.isEmpty, cursor < trimmed.endIndex, trimmed[cursor] == "." else {
            return nil
        }
        let afterDot = trimmed.index(after: cursor)
        guard afterDot < trimmed.endIndex, trimmed[afterDot] == " " else {
            return nil
        }
        let textStart = trimmed.index(after: afterDot)
        return ("\(digitPrefix).", String(trimmed[textStart...]))
    }
}

private enum GaryxMarkdownLinkTarget {
    static func fileTarget(
        from url: URL,
        allowsRelativeFileLinks: Bool
    ) -> String {
        if let path = GaryxMobileFileLink.localFilePath(from: url) {
            return path
        }
        guard allowsRelativeFileLinks else { return "" }

        let raw = url.relativeString.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty,
              !raw.hasPrefix("#"),
              !raw.hasPrefix("?"),
              url.scheme == nil else {
            return ""
        }
        return raw
    }
}

private struct GaryxCodeBlockView: View {
    let language: String?
    let code: String
    let foreground: Color
    let background: Color
    let border: Color
    let fillsAvailableWidth: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let language, !language.isEmpty {
                Text(language)
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 10)
                    .padding(.top, 8)
                    .padding(.bottom, 4)
            }

            ScrollView(.horizontal, showsIndicators: false) {
                Text(code.isEmpty ? " " : code)
                    .font(.system(size: 12.5, weight: .regular, design: .monospaced))
                    .foregroundStyle(foreground)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: true, vertical: true)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
            }
        }
        .frame(maxWidth: fillsAvailableWidth ? .infinity : nil, alignment: .leading)
        .background(background, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(border, lineWidth: 1)
        }
    }
}

private enum GaryxMarkdownTableColumnAlignment {
    case leading
    case center
    case trailing

    var frameAlignment: Alignment {
        switch self {
        case .leading:
            return .leading
        case .center:
            return .center
        case .trailing:
            return .trailing
        }
    }

    var textAlignment: TextAlignment {
        switch self {
        case .leading:
            return .leading
        case .center:
            return .center
        case .trailing:
            return .trailing
        }
    }
}

private struct GaryxMarkdownTable {
    struct Column {
        let title: String
        let alignment: GaryxMarkdownTableColumnAlignment
    }

    let columns: [Column]
    let rows: [[String]]
}

private struct GaryxMarkdownTableView: View {
    let table: GaryxMarkdownTable
    let foreground: Color
    let background: Color
    let border: Color
    let fillsAvailableWidth: Bool
    var allowsRelativeFileLinks = false
    var onFileLinkTap: ((String) -> Void)?

    private var columnWidths: [CGFloat] {
        table.columns.indices.map { index in
            let headerLength = table.columns[index].title.count
            let rowLength = table.rows
                .compactMap { index < $0.count ? $0[index].count : nil }
                .max() ?? 0
            let maxLength = max(headerLength, rowLength)
            return min(max(CGFloat(maxLength) * 7.2 + 32, 86), 220)
        }
    }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: true) {
            VStack(alignment: .leading, spacing: 0) {
                rowView(
                    cells: table.columns.map(\.title),
                    isHeader: true,
                    rowIndex: -1
                )

                if !table.rows.isEmpty {
                    Rectangle()
                        .fill(border)
                        .frame(height: 1)
                }

                ForEach(Array(table.rows.enumerated()), id: \.offset) { rowIndex, cells in
                    rowView(cells: cells, isHeader: false, rowIndex: rowIndex)

                    if rowIndex < table.rows.count - 1 {
                        Rectangle()
                            .fill(border.opacity(0.72))
                            .frame(height: 1)
                    }
                }
            }
            .background(background.opacity(0.58), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .stroke(border, lineWidth: 1)
            }
        }
        .frame(maxWidth: fillsAvailableWidth ? .infinity : nil, alignment: .leading)
    }

    private func rowView(cells: [String], isHeader: Bool, rowIndex: Int) -> some View {
        HStack(alignment: .top, spacing: 0) {
            ForEach(table.columns.indices, id: \.self) { columnIndex in
                let column = table.columns[columnIndex]
                let text = columnIndex < cells.count ? cells[columnIndex] : ""

                cellView(
                    text: text,
                    column: column,
                    width: columnWidths[columnIndex],
                    isHeader: isHeader
                )

                if columnIndex < table.columns.count - 1 {
                    Rectangle()
                        .fill(border.opacity(0.72))
                        .frame(width: 1)
                }
            }
        }
        .background(rowBackground(isHeader: isHeader, rowIndex: rowIndex))
    }

    private func cellView(
        text: String,
        column: GaryxMarkdownTable.Column,
        width: CGFloat,
        isHeader: Bool
    ) -> some View {
        Text(GaryxMarkdownText.attributedString(from: text.isEmpty ? " " : text))
            .font(isHeader ? GaryxFont.callout(weight: .semibold) : GaryxFont.callout())
            .foregroundStyle(foreground)
            .tint(GaryxTheme.accent)
            .multilineTextAlignment(column.alignment.textAlignment)
            .environment(\.openURL, openURLAction)
            .textSelection(.enabled)
            .lineSpacing(2)
            .fixedSize(horizontal: false, vertical: true)
            .frame(width: width, alignment: column.alignment.frameAlignment)
            .padding(.horizontal, 8)
            .padding(.vertical, 7)
    }

    private func rowBackground(isHeader: Bool, rowIndex: Int) -> Color {
        if isHeader {
            return background.opacity(0.88)
        }
        if rowIndex.isMultiple(of: 2) {
            return Color.clear
        }
        return background.opacity(0.26)
    }

    private var openURLAction: OpenURLAction {
        OpenURLAction { url in
            guard let onFileLinkTap else { return .systemAction }
            let target = GaryxMarkdownLinkTarget.fileTarget(
                from: url,
                allowsRelativeFileLinks: allowsRelativeFileLinks
            )
            guard !target.isEmpty else { return .systemAction }
            onFileLinkTap(target)
            return .handled
        }
    }
}

private struct GaryxMarkdownImageView: View {
    let alt: String
    let source: String

    @State private var localImage: UIImage?
    @State private var loadFailed = false
    @State private var showsPreview = false

    private var maxDisplayWidth: CGFloat {
        min(UIScreen.main.bounds.width * 0.76, 320)
    }

    private var maxDisplayHeight: CGFloat {
        260
    }

    private var resolvedURL: URL? {
        let trimmed = source.trimmingCharacters(in: .whitespaces)
        if let url = URL(string: trimmed), let scheme = url.scheme?.lowercased(),
           ["http", "https"].contains(scheme) {
            return url
        }
        return nil
    }

    private var localFilePath: String? {
        let trimmed = source.trimmingCharacters(in: .whitespaces)
        if trimmed.hasPrefix("file://") {
            return URL(string: trimmed)?.path
        }
        if trimmed.hasPrefix("/") {
            return trimmed
        }
        return nil
    }

    var body: some View {
        Button {
            showsPreview = true
        } label: {
            if let image = localImage {
                let size = displaySize(for: image.size)
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .frame(width: size.width, height: size.height)
                    .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .stroke(Color.primary.opacity(0.08), lineWidth: 1)
                    }
            } else {
                Group {
                    if loadFailed {
                        failurePlaceholder
                    } else {
                        loadingPlaceholder
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .stroke(Color.primary.opacity(0.08), lineWidth: 1)
                }
            }
        }
        .buttonStyle(.plain)
        .fixedSize()
        .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .disabled(localImage == nil && loadFailed)
        .fullScreenCover(isPresented: $showsPreview) {
            GaryxFullscreenImagePreview(
                source: GaryxImagePreviewSource(
                    title: alt.isEmpty ? "Image" : alt,
                    dataUrl: sourceDataUrl,
                    remoteUrl: resolvedURL?.absoluteString,
                    filePath: localFilePath
                )
            ) {
                showsPreview = false
            }
        }
        .accessibilityLabel(alt.isEmpty ? "Image" : alt)
        .accessibilityHint("Opens full screen preview")
        .task(id: source) {
            await loadImageIfPossible()
        }
    }

    private func displaySize(for rawSize: CGSize) -> CGSize {
        let rawWidth = max(rawSize.width, 1)
        let rawHeight = max(rawSize.height, 1)
        let scale = min(maxDisplayWidth / rawWidth, maxDisplayHeight / rawHeight, 1)
        return CGSize(width: rawWidth * scale, height: rawHeight * scale)
    }

    @ViewBuilder
    private var loadingPlaceholder: some View {
        ZStack {
            Color(.secondarySystemFill)
            ProgressView()
                .scaleEffect(0.78)
        }
        .frame(width: maxDisplayWidth, height: 160)
    }

    @ViewBuilder
    private var failurePlaceholder: some View {
        HStack(spacing: 10) {
            Image(systemName: "photo")
                .font(GaryxFont.system(size: 18, weight: .medium))
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(alt.isEmpty ? "Image" : alt)
                    .font(GaryxFont.callout(weight: .medium))
                    .foregroundStyle(.primary)
                Text(source)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .frame(width: maxDisplayWidth, alignment: .leading)
        .background(Color(.secondarySystemFill))
    }

    @MainActor
    private func loadImageIfPossible() async {
        localImage = nil
        loadFailed = false
        if let sourceDataUrl {
            let image = await Task.detached(priority: .utility) {
                GaryxImageDecoder.image(fromDataUrl: sourceDataUrl, maxPixelSize: 720)
            }.value
            guard !Task.isCancelled else { return }
            if let image {
                localImage = image
            } else {
                loadFailed = true
            }
            return
        }
        if let path = localFilePath {
            let image = await Task.detached(priority: .utility) {
                GaryxImageDecoder.image(fromFile: path, maxPixelSize: 720)
            }.value
            guard !Task.isCancelled else { return }
            if let image {
                localImage = image
            } else {
                loadFailed = true
            }
            return
        }
        guard let url = resolvedURL else {
            loadFailed = true
            return
        }
        do {
            let (data, _) = try await URLSession.shared.data(from: url)
            let image = await Task.detached(priority: .utility) {
                GaryxImageDecoder.image(from: data, maxPixelSize: 720)
            }.value
            guard !Task.isCancelled else { return }
            if let image {
                localImage = image
            } else {
                loadFailed = true
            }
        } catch {
            guard !Task.isCancelled else { return }
            loadFailed = true
        }
    }

    private var sourceDataUrl: String? {
        let trimmed = source.trimmingCharacters(in: .whitespaces)
        return trimmed.hasPrefix("data:") ? trimmed : nil
    }
}

private struct GaryxMarkdownBlock: Identifiable {
    enum Kind {
        case markdown(String)
        case code(language: String?, text: String)
        case image(alt: String, source: String)
        case table(GaryxMarkdownTable)
    }

    let id: Int
    let kind: Kind

    static func blocks(from text: String) -> [GaryxMarkdownBlock] {
        GaryxMarkdownBlockParser.blocks(from: text).enumerated().map { index, parsed in
            GaryxMarkdownBlock(id: index, kind: kind(from: parsed.kind))
        }
    }

    private static func kind(from parsed: GaryxMarkdownParsedBlock.Kind) -> Kind {
        switch parsed {
        case .markdown(let value):
            return .markdown(value)
        case .code(let language, let text):
            return .code(language: language, text: text)
        case .image(let alt, let source):
            return .image(alt: alt, source: source)
        case .table(let table):
            return .table(
                GaryxMarkdownTable(
                    columns: table.columns.map {
                        GaryxMarkdownTable.Column(
                            title: $0.title,
                            alignment: GaryxMarkdownTableColumnAlignment($0.alignment)
                        )
                    },
                    rows: table.rows
                )
            )
        }
    }
}

private extension GaryxMarkdownTableColumnAlignment {
    init(_ alignment: GaryxMarkdownParsedTable.ColumnAlignment) {
        switch alignment {
        case .leading:
            self = .leading
        case .center:
            self = .center
        case .trailing:
            self = .trailing
        }
    }
}

private final class GaryxMarkdownRenderCache {
    static let shared = GaryxMarkdownRenderCache()

    private let maxCacheableBlockBytes = 16 * 1024
    private let maxCacheableAttributedBytes = 8 * 1024
    private let blockCache: NSCache<NSString, GaryxMarkdownBlockCacheEntry>
    private let attributedCache: NSCache<NSString, GaryxMarkdownAttributedCacheEntry>
    private let attributedOptions = AttributedString.MarkdownParsingOptions(
        interpretedSyntax: .full,
        failurePolicy: .returnPartiallyParsedIfPossible
    )

    private init() {
        let blockCache = NSCache<NSString, GaryxMarkdownBlockCacheEntry>()
        blockCache.countLimit = 256
        blockCache.totalCostLimit = 2 * 1024 * 1024
        self.blockCache = blockCache

        let attributedCache = NSCache<NSString, GaryxMarkdownAttributedCacheEntry>()
        attributedCache.countLimit = 1_024
        attributedCache.totalCostLimit = 4 * 1024 * 1024
        self.attributedCache = attributedCache
    }

    func blocks(from text: String) -> [GaryxMarkdownBlock] {
        let byteCount = text.utf8.count
        guard byteCount <= maxCacheableBlockBytes else {
            return GaryxMarkdownBlock.blocks(from: text)
        }
        let key = NSString(string: text)
        if let cached = blockCache.object(forKey: key) {
            return cached.blocks
        }
        let blocks = GaryxMarkdownBlock.blocks(from: text)
        blockCache.setObject(GaryxMarkdownBlockCacheEntry(blocks: blocks), forKey: key, cost: max(1, byteCount))
        return blocks
    }

    func attributedString(from markdown: String) -> AttributedString {
        let byteCount = markdown.utf8.count
        guard byteCount <= maxCacheableAttributedBytes else {
            return (try? AttributedString(markdown: markdown, options: attributedOptions)) ?? AttributedString(markdown)
        }
        let key = NSString(string: markdown)
        if let cached = attributedCache.object(forKey: key) {
            return cached.value
        }
        let value = (try? AttributedString(markdown: markdown, options: attributedOptions)) ?? AttributedString(markdown)
        attributedCache.setObject(GaryxMarkdownAttributedCacheEntry(value: value), forKey: key, cost: max(1, byteCount))
        return value
    }
}

private final class GaryxMarkdownBlockCacheEntry {
    let blocks: [GaryxMarkdownBlock]

    init(blocks: [GaryxMarkdownBlock]) {
        self.blocks = blocks
    }
}

private final class GaryxMarkdownAttributedCacheEntry {
    let value: AttributedString

    init(value: AttributedString) {
        self.value = value
    }
}
