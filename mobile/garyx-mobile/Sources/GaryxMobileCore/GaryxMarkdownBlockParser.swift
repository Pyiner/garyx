import Foundation

struct GaryxMarkdownParsedBlock: Equatable {
    enum Kind: Equatable {
        case markdown(String)
        case code(language: String?, text: String)
        case image(alt: String, source: String)
        case table(GaryxMarkdownParsedTable)
    }

    let kind: Kind
}

struct GaryxMarkdownParsedTable: Equatable {
    enum ColumnAlignment: Equatable {
        case leading
        case center
        case trailing
    }

    struct Column: Equatable {
        let title: String
        let alignment: ColumnAlignment
    }

    let columns: [Column]
    let rows: [[String]]
}

enum GaryxMarkdownBlockParser {
    static func blocks(from text: String) -> [GaryxMarkdownParsedBlock] {
        var blocks: [GaryxMarkdownParsedBlock] = []
        var markdownLines: [String] = []
        var codeLines: [String] = []
        var codeLanguage: String?
        var activeFence: Fence?
        let lines = text.components(separatedBy: "\n")

        func appendMarkdown() {
            let value = markdownLines.joined(separator: "\n")
            markdownLines.removeAll(keepingCapacity: true)
            guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
            blocks.append(GaryxMarkdownParsedBlock(kind: .markdown(value)))
        }

        func appendCode() {
            let value = codeLines.joined(separator: "\n")
            codeLines.removeAll(keepingCapacity: true)
            guard !value.isEmpty else { return }
            blocks.append(GaryxMarkdownParsedBlock(kind: .code(language: codeLanguage, text: value)))
            codeLanguage = nil
        }

        var index = 0
        while index < lines.count {
            let line = lines[index]
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            let fenceLine = fenceCandidateLine(from: line)
            if let fence = activeFence {
                if isClosingFence(fenceLine, for: fence) {
                    appendCode()
                    activeFence = nil
                } else {
                    codeLines.append(line)
                }
                index += 1
                continue
            }

            if let image = parseStandaloneImage(trimmed) {
                appendMarkdown()
                blocks.append(GaryxMarkdownParsedBlock(kind: .image(alt: image.alt, source: image.source)))
                index += 1
                continue
            }

            if let fence = openingFence(from: fenceLine) {
                appendMarkdown()
                activeFence = fence
                codeLanguage = fence.info.isEmpty ? nil : fence.info
                index += 1
                continue
            }

            if let parsedTable = parseTable(lines: lines, startIndex: index) {
                appendMarkdown()
                blocks.append(GaryxMarkdownParsedBlock(kind: .table(parsedTable.table)))
                index = parsedTable.nextIndex
            } else {
                markdownLines.append(line)
                index += 1
            }
        }

        if activeFence != nil {
            appendCode()
        }
        appendMarkdown()

        if blocks.isEmpty {
            blocks.append(GaryxMarkdownParsedBlock(kind: .markdown(text)))
        }
        return blocks
    }

    private struct Fence {
        let marker: Character
        let length: Int
        let info: String
    }

    private static func openingFence(from trimmed: String) -> Fence? {
        guard let marker = trimmed.first, marker == "`" || marker == "~" else { return nil }
        let length = fenceMarkerLength(in: trimmed, marker: marker)
        guard length >= 3 else { return nil }
        let infoStart = trimmed.index(trimmed.startIndex, offsetBy: length)
        let info = trimFenceWhitespace(String(trimmed[infoStart...]))
        return Fence(marker: marker, length: length, info: info)
    }

    private static func isClosingFence(_ trimmed: String, for fence: Fence) -> Bool {
        guard trimmed.first == fence.marker else { return false }
        let length = fenceMarkerLength(in: trimmed, marker: fence.marker)
        guard length >= fence.length else { return false }
        let restStart = trimmed.index(trimmed.startIndex, offsetBy: length)
        return isFenceWhitespaceOnly(trimmed[restStart...])
    }

    private static func fenceMarkerLength(in value: String, marker: Character) -> Int {
        var count = 0
        for character in value {
            guard character == marker else { break }
            count += 1
        }
        return count
    }

    private static func parseTable(
        lines: [String],
        startIndex: Int
    ) -> (table: GaryxMarkdownParsedTable, nextIndex: Int)? {
        guard startIndex + 1 < lines.count,
              let headerCells = splitTableRow(lines[startIndex]),
              headerCells.count >= 2,
              headerCells.contains(where: { !$0.isEmpty }),
              let separatorCells = splitTableRow(lines[startIndex + 1]),
              separatorCells.count == headerCells.count,
              separatorCells.allSatisfy(isTableSeparator) else {
            return nil
        }

        var nextIndex = startIndex + 2
        var rows: [[String]] = []
        while nextIndex < lines.count {
            let line = lines[nextIndex]
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, openingFence(from: fenceCandidateLine(from: line)) == nil,
                  let rowCells = splitTableRow(line),
                  rowCells.count >= 2 else {
                break
            }
            rows.append(normalizedTableRow(rowCells, columnCount: headerCells.count))
            nextIndex += 1
        }

        let columns = zip(headerCells, separatorCells).map { header, separator in
            GaryxMarkdownParsedTable.Column(
                title: header,
                alignment: tableAlignment(from: separator)
            )
        }
        return (GaryxMarkdownParsedTable(columns: columns, rows: rows), nextIndex)
    }

    private static func splitTableRow(_ line: String) -> [String]? {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard containsUnescapedPipe(trimmed) else { return nil }

        var row = trimmed
        if row.first == "|" {
            row.removeFirst()
        }
        if let lastIndex = row.indices.last, row[lastIndex] == "|", !isEscapedPipe(in: row, at: lastIndex) {
            row.removeLast()
        }

        var cells: [String] = []
        var current = ""
        var isEscaping = false
        for character in row {
            if isEscaping {
                if character == "|" {
                    current.append(character)
                } else {
                    current.append("\\")
                    current.append(character)
                }
                isEscaping = false
            } else if character == "\\" {
                isEscaping = true
            } else if character == "|" {
                cells.append(current.trimmingCharacters(in: .whitespaces))
                current.removeAll(keepingCapacity: true)
            } else {
                current.append(character)
            }
        }
        if isEscaping {
            current.append("\\")
        }
        cells.append(current.trimmingCharacters(in: .whitespaces))
        return cells.count >= 2 ? cells : nil
    }

    private static func containsUnescapedPipe(_ value: String) -> Bool {
        for index in value.indices where value[index] == "|" {
            if !isEscapedPipe(in: value, at: index) {
                return true
            }
        }
        return false
    }

    private static func isEscapedPipe(in value: String, at index: String.Index) -> Bool {
        var slashCount = 0
        var cursor = index
        while cursor > value.startIndex {
            cursor = value.index(before: cursor)
            if value[cursor] == "\\" {
                slashCount += 1
            } else {
                break
            }
        }
        return slashCount % 2 == 1
    }

    private static func isTableSeparator(_ cell: String) -> Bool {
        let trimmed = cell.trimmingCharacters(in: .whitespaces)
        let body = trimmed.trimmingCharacters(in: CharacterSet(charactersIn: ":"))
        guard body.count >= 3 else { return false }
        return body.allSatisfy { $0 == "-" }
    }

    private static func tableAlignment(from separator: String) -> GaryxMarkdownParsedTable.ColumnAlignment {
        let trimmed = separator.trimmingCharacters(in: .whitespaces)
        let hasLeadingColon = trimmed.hasPrefix(":")
        let hasTrailingColon = trimmed.hasSuffix(":")
        if hasLeadingColon && hasTrailingColon {
            return .center
        }
        if hasTrailingColon {
            return .trailing
        }
        return .leading
    }

    private static func normalizedTableRow(_ cells: [String], columnCount: Int) -> [String] {
        if cells.count == columnCount {
            return cells
        }
        if cells.count > columnCount {
            var normalized = Array(cells.prefix(max(columnCount - 1, 0)))
            let remaining = cells.dropFirst(max(columnCount - 1, 0)).joined(separator: " | ")
            normalized.append(remaining)
            return normalized
        }
        return cells + Array(repeating: "", count: columnCount - cells.count)
    }

    private static func parseStandaloneImage(_ trimmed: String) -> (alt: String, source: String)? {
        guard trimmed.hasPrefix("!["), trimmed.hasSuffix(")") else { return nil }
        let afterBang = trimmed.index(trimmed.startIndex, offsetBy: 2)
        guard let altEnd = trimmed[afterBang...].firstIndex(of: "]") else { return nil }
        let parenStart = trimmed.index(after: altEnd)
        guard parenStart < trimmed.endIndex, trimmed[parenStart] == "(" else { return nil }
        let sourceStart = trimmed.index(after: parenStart)
        let sourceEnd = trimmed.index(before: trimmed.endIndex)
        guard sourceStart < sourceEnd else { return nil }
        let alt = String(trimmed[afterBang..<altEnd]).trimmingCharacters(in: .whitespaces)
        let rawSource = String(trimmed[sourceStart..<sourceEnd]).trimmingCharacters(in: .whitespaces)
        guard !rawSource.isEmpty else { return nil }
        let source = rawSource
            .split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
            .first
            .map(String.init) ?? rawSource
        return (alt, source)
    }

    private static func fenceCandidateLine(from line: String) -> String {
        var lowerBound = line.startIndex
        while lowerBound < line.endIndex, isFenceWhitespace(line[lowerBound]) {
            lowerBound = line.index(after: lowerBound)
        }

        var upperBound = line.endIndex
        while upperBound > lowerBound {
            let index = line.index(before: upperBound)
            guard isFenceWhitespace(line[index]) else { break }
            upperBound = index
        }

        return String(line[lowerBound..<upperBound])
    }

    private static func trimFenceWhitespace(_ value: String) -> String {
        fenceCandidateLine(from: value)
    }

    private static func isFenceWhitespaceOnly(_ value: Substring) -> Bool {
        value.allSatisfy(isFenceWhitespace)
    }

    private static func isFenceWhitespace(_ character: Character) -> Bool {
        // Do not use Foundation's Unicode whitespace set here: U+200B is used
        // to escape literal fences in prompts and must not be stripped.
        character == " " || character == "\t" || character == "\r"
    }
}
