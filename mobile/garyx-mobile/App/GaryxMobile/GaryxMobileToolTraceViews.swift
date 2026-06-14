import SwiftUI

/// Transcript surface for one tool-call group: a compact natural-language
/// summary row that opens the call list sheet, plus inline thumbnails for
/// images the calls read or produced. Call details live behind the sheet
/// (list → per-call parameters/output), not inline in the transcript.
struct GaryxToolTraceGroupView: View {
    let group: GaryxMobileToolTraceGroup

    @State private var showsCallList = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Button {
                showsCallList = true
            } label: {
                HStack(spacing: 8) {
                    summaryText

                    Image(systemName: "chevron.right")
                        .font(GaryxFont.system(size: 11, weight: .semibold))
                        .opacity(0.74)
                }
                .foregroundStyle(group.isActive ? GaryxTheme.primaryText : GaryxTheme.secondaryText)
                .frame(minHeight: 22)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Show tool calls")
            .accessibilityAddTraits(.isButton)

            let imageRefs = GaryxToolCallPresentation.imageRefs(from: group.entries)
            if !imageRefs.isEmpty {
                GaryxToolImageThumbnailStrip(refs: imageRefs)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .sheet(isPresented: $showsCallList) {
            GaryxToolCallListSheet(group: group)
                .presentationDetents([.fraction(0.72), .large])
                .presentationDragIndicator(.visible)
        }
    }

    @ViewBuilder
    private var summaryText: some View {
        if group.isActive {
            GaryxShimmerText(
                text: group.summary,
                font: GaryxFont.subheadline(),
                baseColor: GaryxTheme.secondaryText,
                peakColor: GaryxTheme.primaryText
            )
            .lineLimit(1)
            .truncationMode(.tail)
        } else {
            Text(group.summary)
                .font(GaryxFont.subheadline())
                .lineLimit(1)
                .truncationMode(.tail)
        }
    }
}

// MARK: - Call list sheet

struct GaryxToolCallListSheet: View {
    let group: GaryxMobileToolTraceGroup

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            // A plain scroll list, not `List`: the reference design has no
            // separators and no disclosure chevrons, just airy icon+text
            // rows that push the call detail.
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(zip(group.entries, GaryxToolCallPresentation.listRows(from: group.entries))), id: \.1.id) { entry, row in
                        NavigationLink {
                            GaryxToolCallDetailView(entry: entry)
                        } label: {
                            GaryxToolCallRowLabel(row: row)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.top, 6)
                .padding(.bottom, 16)
            }
            .navigationTitle(group.summary)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        dismiss()
                    } label: {
                        Image(systemName: "xmark")
                            .font(GaryxFont.system(size: 15, weight: .medium))
                            .foregroundStyle(.primary)
                    }
                    .accessibilityLabel("Close tool calls")
                }
            }
        }
    }
}

private struct GaryxToolCallRowLabel: View {
    let row: GaryxToolCallListRow

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            Image(systemName: iconName)
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(row.isError ? GaryxTheme.danger : GaryxTheme.secondaryText)
                .frame(width: 24, height: 24)

            if row.isRunning {
                GaryxShimmerText(
                    text: [row.verb, row.detail].compactMap(\.self).joined(separator: " "),
                    font: GaryxFont.subheadline(),
                    baseColor: GaryxTheme.secondaryText,
                    peakColor: GaryxTheme.primaryText
                )
                .lineLimit(1)
                .truncationMode(.middle)
            } else {
                (Text(row.verb).foregroundStyle(GaryxTheme.secondaryText)
                    + Text(row.detail.map { " \($0)" } ?? "").foregroundStyle(.primary))
                    .font(GaryxFont.subheadline())
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Spacer(minLength: 0)
        }
        .padding(.vertical, 11)
        .contentShape(Rectangle())
    }

    private var iconName: String {
        switch row.icon {
        case .command: "terminal"
        case .read: "doc.text"
        case .edit: "pencil"
        case .search: "magnifyingglass"
        case .web: "globe"
        case .generic: "gearshape"
        }
    }
}

// MARK: - Call detail

struct GaryxToolCallDetailView: View {
    let entry: GaryxMobileToolTraceEntry

    var body: some View {
        let detail = GaryxToolCallPresentation.detail(for: entry)
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                ForEach(detail.sections) { section in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(section.label)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)

                        switch section.content {
                        case .plainMonospace(let text):
                            Text(text)
                                .font(.system(size: 13, weight: .regular, design: .monospaced))
                                .foregroundStyle(.primary)
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                        case .codeCard(let text):
                            GaryxToolCallCodeCard(text: text)
                        case .diff(let lines):
                            GaryxToolCallDiffView(lines: lines)
                        case .imagePreview(let path):
                            GaryxToolImageThumbnail(
                                ref: GaryxToolCallImageRef(id: "detail:\(path)", path: path)
                            )
                        }
                    }
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigationTitle(detail.title)
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct GaryxToolCallCodeCard: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(size: 13, weight: .regular, design: .monospaced))
            .foregroundStyle(.primary)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

private struct GaryxToolCallDiffView: View {
    let lines: [GaryxToolCallDiffLine]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(lines) { line in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(marker(for: line.kind))
                            .font(.system(size: 12, weight: .regular, design: .monospaced))
                            .foregroundStyle(markerColor(for: line.kind))

                        Text(line.text.isEmpty ? " " : line.text)
                            .font(.system(size: 12, weight: .regular, design: .monospaced))
                            .foregroundStyle(.primary)
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 2)
                    .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
                    .background(background(for: line.kind))
                }
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }

    private func marker(for kind: GaryxToolCallDiffLine.Kind) -> String {
        switch kind {
        case .added: "+"
        case .removed: "-"
        case .context: " "
        }
    }

    private func markerColor(for kind: GaryxToolCallDiffLine.Kind) -> Color {
        switch kind {
        case .added: Color(.systemGreen)
        case .removed: Color(.systemRed)
        case .context: GaryxTheme.secondaryText
        }
    }

    private func background(for kind: GaryxToolCallDiffLine.Kind) -> Color {
        switch kind {
        case .added: Color(.systemGreen).opacity(0.12)
        case .removed: Color(.systemRed).opacity(0.12)
        case .context: Color.clear
        }
    }
}

// MARK: - Inline image thumbnails

private struct GaryxToolImageThumbnailStrip: View {
    let refs: [GaryxToolCallImageRef]

    @EnvironmentObject private var model: GaryxMobileModel
    @State private var loadedByPath: [String: GaryxToolImageLoadedPreview] = [:]
    @State private var previewSelection: GaryxToolImagePreviewSelection?

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(Array(refs.enumerated()), id: \.element.id) { index, ref in
                    GaryxToolImageThumbnail(
                        ref: ref,
                        onLoaded: { image, dataUrl in
                            loadedByPath[ref.path] = GaryxToolImageLoadedPreview(image: image, dataUrl: dataUrl)
                        },
                        onTap: {
                            previewSelection = GaryxToolImagePreviewSelection(index: index)
                        }
                    )
                }
            }
        }
        .scrollClipDisabled()
        // One shared gallery cover for the whole strip so the fullscreen
        // preview can swipe between this tool group's images.
        .fullScreenCover(item: $previewSelection) { selection in
            GaryxFullscreenImageGalleryPreview(
                sources: refs.map { ref in
                    let loaded = loadedByPath[ref.path]
                    return GaryxImagePreviewSource(
                        title: ref.fileName,
                        dataUrl: loaded?.dataUrl,
                        remoteUrl: nil,
                        filePath: nil,
                        gatewayFilePath: ref.path,
                        initialImage: loaded?.image
                    )
                },
                initialIndex: selection.index,
                loadGatewayDataUrl: { path in
                    await loadGatewayImageDataUrl(path)
                },
                onDismiss: { previewSelection = nil }
            )
        }
    }

    private func loadGatewayImageDataUrl(_ path: String) async -> String? {
        guard let preview = await model.localFilePreview(path, reportsError: false),
              let base64 = preview.dataBase64,
              !base64.isEmpty else {
            return nil
        }
        let mediaType = preview.mediaType.isEmpty ? "image/png" : preview.mediaType
        return "data:\(mediaType);base64,\(base64)"
    }
}

private struct GaryxToolImageLoadedPreview {
    let image: UIImage
    let dataUrl: String
}

private struct GaryxToolImagePreviewSelection: Identifiable {
    let index: Int
    var id: Int { index }
}

private struct GaryxToolImageThumbnail: View {
    let ref: GaryxToolCallImageRef
    /// Reports the decoded image and data URL up to a hosting strip so its
    /// gallery preview can seed every page that has already loaded.
    var onLoaded: ((UIImage, String) -> Void)? = nil
    /// When set, tapping delegates to the host's shared gallery instead of
    /// this thumbnail's own single-image cover.
    var onTap: (() -> Void)? = nil

    @EnvironmentObject private var model: GaryxMobileModel
    @State private var image: UIImage?
    @State private var dataUrl: String?
    @State private var loadFailed = false
    @State private var showsPreview = false

    // Fixed cell sized so four thumbnails fit one transcript row (screen
    // minus the 16pt transcript margins and three 8pt gaps). Sources of any
    // aspect ratio center-crop fill the cell, matching the reference design.
    private var thumbnailWidth: CGFloat {
        let contentWidth = UIScreen.main.bounds.width - 32
        return ((contentWidth - 3 * 8) / 4).rounded(.down)
    }

    private var thumbnailHeight: CGFloat {
        (thumbnailWidth * 1.25).rounded(.down)
    }

    var body: some View {
        Button {
            guard image != nil else { return }
            if let onTap {
                onTap()
            } else {
                showsPreview = true
            }
        } label: {
            Group {
                if let image {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFill()
                        .frame(width: thumbnailWidth, height: thumbnailHeight)
                } else if loadFailed {
                    // Unreadable image paths render nothing rather than a
                    // dead placeholder: the path stays visible in the call
                    // list sheet.
                    EmptyView()
                } else {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(GaryxTheme.surface)
                        .frame(width: thumbnailWidth, height: thumbnailHeight)
                        .overlay {
                            ProgressView()
                                .controlSize(.small)
                        }
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(Color.primary.opacity(0.08), lineWidth: 1)
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Preview \(ref.fileName)")
        .task(id: ref.path) {
            await loadThumbnail()
        }
        .fullScreenCover(isPresented: $showsPreview) {
            GaryxFullscreenImagePreview(
                source: GaryxImagePreviewSource(
                    title: ref.fileName,
                    dataUrl: dataUrl,
                    remoteUrl: nil,
                    filePath: nil,
                    initialImage: image
                )
            ) {
                showsPreview = false
            }
        }
    }

    private func loadThumbnail() async {
        guard image == nil, !loadFailed else { return }
        guard let preview = await model.localFilePreview(ref.path, reportsError: false),
              let base64 = preview.dataBase64,
              let data = Data(base64Encoded: base64),
              let loaded = UIImage(data: data) else {
            loadFailed = true
            return
        }
        image = loaded
        let mediaType = preview.mediaType.isEmpty ? "image/png" : preview.mediaType
        let resolvedDataUrl = "data:\(mediaType);base64,\(base64)"
        dataUrl = resolvedDataUrl
        onLoaded?(loaded, resolvedDataUrl)
    }
}
