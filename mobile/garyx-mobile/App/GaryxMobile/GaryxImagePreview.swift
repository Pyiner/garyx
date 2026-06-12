import Foundation
import ImageIO
import SwiftUI
import UIKit

struct GaryxImagePreviewSource: Equatable {
    var title: String
    var dataUrl: String?
    var remoteUrl: String?
    var filePath: String?
    /// Gateway-readable path resolved through the host's `loadGatewayDataUrl`
    /// closure. Used by gallery pages whose data has not been fetched yet.
    var gatewayFilePath: String?
    /// Already-decoded image from the tapped thumbnail. Seeds the preview's
    /// first frame so opening never flashes an empty screen while the
    /// full-resolution decode runs; the decode replaces it seamlessly.
    var initialImage: UIImage?

    var displayTitle: String {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Image" : trimmed
    }

    var loadKey: String {
        [
            dataUrl ?? "",
            remoteUrl ?? "",
            filePath ?? "",
            gatewayFilePath ?? "",
        ].joined(separator: "|")
    }
}

struct GaryxFullscreenImagePreview: View {
    let source: GaryxImagePreviewSource
    let onDismiss: () -> Void

    var body: some View {
        GaryxFullscreenImageGalleryPreview(
            sources: [source],
            initialIndex: 0,
            onDismiss: onDismiss
        )
    }
}

/// Paged fullscreen preview: swiping left/right moves between the images of
/// the launching surface (for example one tool call's thumbnail strip)
/// without closing and reopening the preview for each image.
struct GaryxFullscreenImageGalleryPreview: View {
    let sources: [GaryxImagePreviewSource]
    let initialIndex: Int
    /// Resolves a `gatewayFilePath` source into a data URL through the
    /// gateway preview API. Injected by hosts that own gateway access.
    var loadGatewayDataUrl: ((String) async -> String?)?
    let onDismiss: () -> Void

    @State private var selection: Int
    @State private var pagingDisabled = false

    init(
        sources: [GaryxImagePreviewSource],
        initialIndex: Int,
        loadGatewayDataUrl: ((String) async -> String?)? = nil,
        onDismiss: @escaping () -> Void
    ) {
        self.sources = sources
        self.initialIndex = initialIndex
        self.loadGatewayDataUrl = loadGatewayDataUrl
        self.onDismiss = onDismiss
        _selection = State(initialValue: min(max(initialIndex, 0), max(sources.count - 1, 0)))
    }

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()

            if sources.count == 1, let onlySource = sources.first {
                GaryxImagePreviewPage(
                    source: onlySource,
                    loadGatewayDataUrl: loadGatewayDataUrl,
                    onDismiss: onDismiss
                )
            } else {
                TabView(selection: $selection) {
                    ForEach(sources.indices, id: \.self) { index in
                        GaryxImagePreviewPage(
                            source: sources[index],
                            loadGatewayDataUrl: loadGatewayDataUrl,
                            isPagedGalleryPage: true,
                            onZoomActiveChanged: { zoomed in
                                guard index == selection, pagingDisabled != zoomed else { return }
                                pagingDisabled = zoomed
                            },
                            onDismiss: onDismiss
                        )
                        .tag(index)
                    }
                }
                .tabViewStyle(.page(indexDisplayMode: .never))
                .ignoresSafeArea()
                // While zoomed in, horizontal pans move the image, not the pager.
                .scrollDisabled(pagingDisabled)
                .onChange(of: selection) { _, _ in
                    pagingDisabled = false
                }
            }
        }
        .preferredColorScheme(.dark)
        .overlay(alignment: .topTrailing) {
            closeButton
                .padding(.top, 12)
                .padding(.trailing, 16)
                .zIndex(10)
        }
        .overlay(alignment: .top) {
            if sources.count > 1 {
                pageIndexLabel
                    .padding(.top, 20)
            }
        }
    }

    private var closeButton: some View {
        Button {
            onDismiss()
        } label: {
            Image(systemName: "xmark")
                .font(GaryxFont.system(size: 16, weight: .semibold))
                .foregroundStyle(.primary)
                .frame(width: 44, height: 44)
                .garyxAdaptiveGlass(
                    .regular,
                    isInteractive: true,
                    tint: Color(.systemBackground).opacity(0.32),
                    fallbackMaterial: .ultraThinMaterial,
                    in: Circle()
                )
        }
        .buttonStyle(.plain)
        .contentShape(Circle())
        .accessibilityLabel("Close image preview")
    }

    private var pageIndexLabel: some View {
        Text("\(selection + 1) / \(sources.count)")
            .font(GaryxFont.footnote(weight: .semibold))
            .foregroundStyle(.primary)
            .padding(.horizontal, 12)
            .frame(minHeight: 28)
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: false,
                tint: Color(.systemBackground).opacity(0.32),
                fallbackMaterial: .ultraThinMaterial,
                in: Capsule()
            )
            .accessibilityLabel("Image \(selection + 1) of \(sources.count)")
    }
}

private struct GaryxImagePreviewPage: View {
    let source: GaryxImagePreviewSource
    var loadGatewayDataUrl: ((String) async -> String?)?
    var isPagedGalleryPage = false
    var onZoomActiveChanged: ((Bool) -> Void)? = nil
    let onDismiss: () -> Void

    @State private var image: UIImage?
    @State private var isLoading = false
    @State private var loadFailed = false

    init(
        source: GaryxImagePreviewSource,
        loadGatewayDataUrl: ((String) async -> String?)? = nil,
        isPagedGalleryPage: Bool = false,
        onZoomActiveChanged: ((Bool) -> Void)? = nil,
        onDismiss: @escaping () -> Void
    ) {
        self.source = source
        self.loadGatewayDataUrl = loadGatewayDataUrl
        self.isPagedGalleryPage = isPagedGalleryPage
        self.onZoomActiveChanged = onZoomActiveChanged
        self.onDismiss = onDismiss
        _image = State(initialValue: source.initialImage)
    }

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()

            if let image {
                GaryxZoomableImageCanvas(
                    image: image,
                    isPagedGalleryPage: isPagedGalleryPage,
                    onZoomActiveChanged: onZoomActiveChanged,
                    onDismiss: onDismiss
                )
                .ignoresSafeArea()
            } else if isLoading {
                ProgressView()
                    .tint(.white)
                    .controlSize(.large)
            } else {
                failureContent
            }
        }
        .task(id: source.loadKey) {
            await loadImage()
        }
    }

    @ViewBuilder
    private var failureContent: some View {
        VStack(spacing: 10) {
            Image(systemName: "photo")
                .font(GaryxFont.title2(weight: .medium))
            Text(loadFailed ? "Unable to load image" : source.displayTitle)
                .font(GaryxFont.callout(weight: .medium))
        }
        .foregroundStyle(.white.opacity(0.78))
        .padding(.horizontal, 24)
        .multilineTextAlignment(.center)
    }

    @MainActor
    private func loadImage() async {
        // Keep the seeded thumbnail on screen while the full-resolution
        // decode runs; the result replaces it without an empty frame.
        loadFailed = false
        isLoading = image == nil
        defer { isLoading = false }

        let source = source
        let loaded: UIImage? = await Task.detached(priority: .userInitiated) { () -> UIImage? in
            if let dataUrl = source.dataUrl?.trimmingCharacters(in: .whitespacesAndNewlines),
               !dataUrl.isEmpty,
               let image = GaryxImageDecoder.image(fromDataUrl: dataUrl, maxPixelSize: 4096) {
                return image
            }
            if let filePath = source.filePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !filePath.isEmpty,
               let image = GaryxImageDecoder.image(fromFile: filePath, maxPixelSize: 4096) {
                return image
            }
            return nil
        }.value

        if let loaded {
            guard !Task.isCancelled else { return }
            image = loaded
            return
        }

        if let gatewayPath = source.gatewayFilePath?.trimmingCharacters(in: .whitespacesAndNewlines),
           !gatewayPath.isEmpty,
           let loadGatewayDataUrl {
            if let resolvedDataUrl = await loadGatewayDataUrl(gatewayPath) {
                let gatewayImage = await Task.detached(priority: .userInitiated) {
                    GaryxImageDecoder.image(fromDataUrl: resolvedDataUrl, maxPixelSize: 4096)
                }.value
                guard !Task.isCancelled else { return }
                if let gatewayImage {
                    image = gatewayImage
                    return
                }
            }
            guard !Task.isCancelled else { return }
        }

        guard let url = remoteURL(from: source.remoteUrl) else {
            loadFailed = true
            return
        }

        do {
            let (data, _) = try await URLSession.shared.data(from: url)
            guard !Task.isCancelled else { return }
            let remoteImage = await Task.detached(priority: .userInitiated) {
                GaryxImageDecoder.image(from: data, maxPixelSize: 4096)
            }.value
            guard !Task.isCancelled else { return }
            if let remoteImage {
                image = remoteImage
            } else {
                loadFailed = true
            }
        } catch {
            guard !Task.isCancelled else { return }
            loadFailed = true
        }
    }

    private func remoteURL(from value: String?) -> URL? {
        guard let raw = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              raw.hasPrefix("http://") || raw.hasPrefix("https://") else {
            return nil
        }
        return URL(string: raw)
    }
}

enum GaryxImageDecoder {
    nonisolated static func image(fromDataUrl raw: String, maxPixelSize: CGFloat) -> UIImage? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let encoded = trimmed.split(separator: ",", maxSplits: 1).last.map(String.init) ?? trimmed
        guard let data = Data(base64Encoded: encoded) else { return nil }
        return image(from: data, maxPixelSize: maxPixelSize)
    }

    nonisolated static func image(from data: Data, maxPixelSize: CGFloat) -> UIImage? {
        let options = [kCGImageSourceShouldCache: false] as CFDictionary
        guard let source = CGImageSourceCreateWithData(data as CFData, options) else {
            return UIImage(data: data)
        }
        return thumbnail(from: source, maxPixelSize: maxPixelSize) ?? UIImage(data: data)
    }

    nonisolated static func image(fromFile path: String, maxPixelSize: CGFloat) -> UIImage? {
        let url = URL(fileURLWithPath: path)
        let options = [kCGImageSourceShouldCache: false] as CFDictionary
        guard let source = CGImageSourceCreateWithURL(url as CFURL, options) else {
            return UIImage(contentsOfFile: path)
        }
        return thumbnail(from: source, maxPixelSize: maxPixelSize) ?? UIImage(contentsOfFile: path)
    }

    nonisolated private static func thumbnail(from source: CGImageSource, maxPixelSize: CGFloat) -> UIImage? {
        let thumbnailOptions: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: Int(maxPixelSize),
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnailOptions as CFDictionary) else {
            return nil
        }
        return UIImage(cgImage: image)
    }
}

private struct GaryxZoomableImageCanvas: View {
    let image: UIImage
    /// True when this canvas is one page of a paged gallery. Inside a paged
    /// TabView, an attached drag gesture claims the touch stream before the
    /// pager's scroll view, so left/right page swipes stop working. Paged
    /// canvases keep the drag gesture masked off until the image is zoomed
    /// in (paging is disabled then anyway); the standalone single-image
    /// preview keeps it always for pull-down dismissal.
    var isPagedGalleryPage = false
    var onZoomActiveChanged: ((Bool) -> Void)? = nil
    let onDismiss: () -> Void

    @State private var scale: CGFloat = 1
    @State private var lastScale: CGFloat = 1
    @State private var offset: CGSize = .zero
    @State private var lastOffset: CGSize = .zero
    @State private var dismissDragOffset: CGFloat = 0

    private var dragGestureMask: GestureMask {
        if isPagedGalleryPage, scale <= 1.02 {
            return .subviews
        }
        return .all
    }

    var body: some View {
        GeometryReader { geometry in
            Image(uiImage: image)
                .resizable()
                .scaledToFit()
                .frame(width: geometry.size.width, height: geometry.size.height)
                .contentShape(Rectangle())
                .scaleEffect(scale)
                .offset(x: offset.width, y: offset.height + dismissDragOffset)
                .gesture(magnificationGesture)
                .simultaneousGesture(dragGesture, including: dragGestureMask)
                .onTapGesture(count: 2) {
                    withAnimation(.spring(response: 0.28, dampingFraction: 0.86)) {
                        if scale > 1 {
                            resetViewport()
                        } else {
                            scale = 2
                            lastScale = 2
                        }
                    }
                }
                .accessibilityLabel("Full screen image")
        }
        .background(Color.black)
        .onChange(of: scale) { _, newScale in
            onZoomActiveChanged?(newScale > 1.02)
        }
        .onDisappear {
            // Pager pages stay alive off screen; coming back shows the image
            // fit-to-screen again instead of a stale zoom viewport.
            resetViewport()
            onZoomActiveChanged?(false)
        }
    }

    private var magnificationGesture: some Gesture {
        MagnificationGesture()
            .onChanged { value in
                scale = min(max(lastScale * value, 1), 5)
                if scale <= 1 {
                    offset = .zero
                }
            }
            .onEnded { _ in
                lastScale = scale
                if scale <= 1.02 {
                    withAnimation(.easeOut(duration: 0.18)) {
                        resetViewport()
                    }
                }
            }
    }

    private var dragGesture: some Gesture {
        DragGesture()
            .onChanged { value in
                guard scale > 1 else {
                    // Only track clearly downward drags so horizontal pager
                    // swipes do not bob the image vertically.
                    let downward = value.translation.height
                    let horizontal = abs(value.translation.width)
                    dismissDragOffset = downward > 0 && downward > horizontal ? downward : 0
                    return
                }
                offset = CGSize(
                    width: lastOffset.width + value.translation.width,
                    height: lastOffset.height + value.translation.height
                )
            }
            .onEnded { value in
                guard scale > 1 else {
                    if shouldDismiss(for: value) {
                        onDismiss()
                        return
                    }
                    withAnimation(.spring(response: 0.22, dampingFraction: 0.88)) {
                        resetViewport()
                    }
                    return
                }
                lastOffset = offset
            }
    }

    private func shouldDismiss(for value: DragGesture.Value) -> Bool {
        let downward = value.translation.height
        let horizontal = abs(value.translation.width)
        return downward > 88 && downward > horizontal * 1.25
    }

    private func resetViewport() {
        scale = 1
        lastScale = 1
        offset = .zero
        lastOffset = .zero
        dismissDragOffset = 0
    }
}
