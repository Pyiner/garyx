import SwiftUI

struct GaryxToolTraceGroupView: View {
    let group: GaryxMobileToolTraceGroup

    @State private var expanded: Bool
    @State private var userControlled = false

    init(group: GaryxMobileToolTraceGroup) {
        self.group = group
        _expanded = State(initialValue: group.defaultExpanded)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                userControlled = true
                withAnimation(.easeOut(duration: 0.19)) {
                    expanded.toggle()
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "terminal")
                        .font(GaryxFont.system(size: 13, weight: .regular))
                        .frame(width: 16, height: 16)

                    Text(group.summary)
                        .font(GaryxFont.footnote())
                        .lineLimit(1)
                        .truncationMode(.tail)

                    if group.isActive {
                        ProgressView()
                            .scaleEffect(0.62)
                    }

                    Image(systemName: "chevron.down")
                        .font(GaryxFont.system(size: 10, weight: .semibold))
                        .rotationEffect(.degrees(expanded ? 0 : -90))
                        .opacity(0.74)
                }
                .foregroundStyle(group.isActive ? GaryxTheme.primaryText : GaryxTheme.secondaryText)
                .frame(minHeight: 22)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(expanded ? "Collapse tool calls" : "Expand tool calls")
            .accessibilityAddTraits(.isButton)

            if expanded {
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(GaryxToolTraceTreeNode.roots(from: group.entries)) { node in
                        GaryxToolTraceTreeNodeView(node: node, depth: 0)
                    }
                }
                .padding(.top, 5)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onChange(of: group.defaultExpanded) { _, shouldExpand in
            guard !userControlled else { return }
            withAnimation(.easeOut(duration: 0.21)) {
                expanded = shouldExpand
            }
        }
    }
}

private struct GaryxToolTraceTreeNode: Identifiable, Equatable {
    let entry: GaryxMobileToolTraceEntry
    var children: [GaryxToolTraceTreeNode]

    var id: String {
        entry.id
    }

    static func roots(from entries: [GaryxMobileToolTraceEntry]) -> [GaryxToolTraceTreeNode] {
        let childrenByParent = Dictionary(grouping: entries) { entry in
            Self.trimmedNilIfEmpty(entry.parentToolUseId)
        }
        let toolUseIds = Set(entries.compactMap { entry in
            Self.trimmedNilIfEmpty(entry.toolUseId)
        })

        func build(_ entry: GaryxMobileToolTraceEntry, seen: Set<String>) -> GaryxToolTraceTreeNode {
            let entryToolUseId = Self.trimmedNilIfEmpty(entry.toolUseId)
            guard let entryToolUseId, !seen.contains(entryToolUseId) else {
                return GaryxToolTraceTreeNode(entry: entry, children: [])
            }
            let children = (childrenByParent[entryToolUseId] ?? []).map { child in
                build(child, seen: seen.union([entryToolUseId]))
            }
            return GaryxToolTraceTreeNode(entry: entry, children: children)
        }

        let rootEntries = entries.filter { entry in
            guard let parentId = Self.trimmedNilIfEmpty(entry.parentToolUseId) else {
                return true
            }
            return !toolUseIds.contains(parentId)
        }
        return (rootEntries.isEmpty ? entries : rootEntries).map { build($0, seen: []) }
    }

    private static func trimmedNilIfEmpty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }
}

private struct GaryxToolTraceTreeNodeView: View {
    let node: GaryxToolTraceTreeNode
    let depth: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            GaryxToolTraceEntryView(entry: node.entry)
                .padding(.leading, CGFloat(depth) * 14)

            ForEach(node.children) { child in
                GaryxToolTraceTreeNodeView(node: child, depth: depth + 1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct GaryxToolTraceEntryView: View {
    let entry: GaryxMobileToolTraceEntry

    @State private var expanded = false

    private var hasDetails: Bool {
        entry.inputText != nil || entry.resultText != nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            header

            if expanded && hasDetails {
                VStack(alignment: .leading, spacing: 4) {
                    if let inputText = entry.inputText {
                        GaryxToolTraceDetailSection(label: entry.inputLabel, text: inputText)
                    }
                    if let resultText = entry.resultText {
                        GaryxToolTraceDetailSection(label: entry.resultLabel, text: resultText)
                    }
                }
                .padding(.top, 1)
                .transition(.opacity)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .transaction { transaction in
            transaction.animation = nil
        }
    }

    @ViewBuilder
    private var header: some View {
        let content = HStack(spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: iconName)
                    .font(GaryxFont.system(size: 12, weight: .regular))
                    .foregroundStyle(GaryxTheme.secondaryText)
                    .frame(width: 16, height: 16)

                Text(entry.title)
                    .font(GaryxFont.footnote())
                    .foregroundStyle(GaryxTheme.secondaryText)
                    .lineLimit(1)

                if let previewText = entry.previewText {
                    Text(previewText)
                        .font(GaryxFont.system(size: 11))
                        .foregroundStyle(GaryxTheme.secondaryText)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Text(entry.status.label)
                .font(GaryxFont.system(size: 11))
                .foregroundStyle(statusColor)
                .textCase(.lowercase)

            if hasDetails {
                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 10, weight: .semibold))
                    .foregroundStyle(Color(.tertiaryLabel))
                    .rotationEffect(.degrees(expanded ? 90 : 0))
            }
        }
        .frame(minHeight: 20)

        if hasDetails {
            Button {
                withAnimation(.easeOut(duration: 0.16)) {
                    expanded.toggle()
                }
            } label: {
                content
            }
            .buttonStyle(.plain)
            .accessibilityLabel(expanded ? "Collapse tool details" : "Expand tool details")
        } else {
            content
        }
    }

    private var iconName: String {
        switch entry.status {
        case .running:
            "circle.dotted"
        case .completed:
            entry.isCommand ? "terminal" : "checkmark.circle"
        case .failed:
            "exclamationmark.triangle"
        }
    }

    private var statusColor: Color {
        switch entry.status {
        case .running:
            GaryxTheme.accent
        case .completed:
            GaryxTheme.secondaryText.opacity(0.5)
        case .failed:
            GaryxTheme.danger
        }
    }
}

struct GaryxToolTraceDetailSection: View {
    let label: String
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(GaryxTheme.secondaryText)
                .textCase(.uppercase)

            Text(text)
                .font(.system(size: 12, weight: .regular, design: .monospaced))
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .stroke(GaryxTheme.hairline, lineWidth: 1)
                }
        }
    }
}
