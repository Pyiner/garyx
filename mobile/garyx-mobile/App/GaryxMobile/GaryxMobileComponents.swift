import Foundation
import SwiftUI
import UIKit

private struct GaryxOpenSidebarActionKey: EnvironmentKey {
    static let defaultValue: () -> Void = {}
}

private struct GaryxLoadMoreThreadsActionKey: EnvironmentKey {
    static let defaultValue: () async -> Void = {}
}

private struct GaryxSidebarDragActiveKey: EnvironmentKey {
    static let defaultValue: Bool = false
}

extension EnvironmentValues {
    var garyxOpenSidebar: () -> Void {
        get { self[GaryxOpenSidebarActionKey.self] }
        set { self[GaryxOpenSidebarActionKey.self] = newValue }
    }

    var garyxLoadMoreThreads: () async -> Void {
        get { self[GaryxLoadMoreThreadsActionKey.self] }
        set { self[GaryxLoadMoreThreadsActionKey.self] = newValue }
    }

    // True while a horizontal sidebar open/close drag is in progress. Surfaces
    // inside the drawer disable their own vertical scrolling so the swipe only
    // moves the sidebar, never the thread list or transcript behind it.
    var garyxSidebarDragActive: Bool {
        get { self[GaryxSidebarDragActiveKey.self] }
        set { self[GaryxSidebarDragActiveKey.self] = newValue }
    }
}

enum GaryxDataURLImageCache {
    static let channelIconMaxPixelSize: CGFloat = 128
    static let agentAvatarMaxPixelSize: CGFloat = 288

    private static let cache: NSCache<NSString, UIImage> = {
        let cache = NSCache<NSString, UIImage>()
        cache.countLimit = 128
        cache.totalCostLimit = 32 * 1024 * 1024
        return cache
    }()
    private static let predecodeQueue = DispatchQueue(
        label: "com.garyx.mobile.data-url-image-cache.predecode",
        qos: .utility
    )
    private static let predecodeState = GaryxDataURLImagePredecodeState()

    static func cachedImage(from rawValue: String?, maxPixelSize: CGFloat) -> UIImage? {
        guard let raw = normalizedRawValue(rawValue) else { return nil }
        return cache.object(forKey: cacheKey(for: raw, maxPixelSize: maxPixelSize))
    }

    static func image(from rawValue: String?) -> UIImage? {
        guard let raw = normalizedRawValue(rawValue) else { return nil }
        let cacheKey = cacheKey(for: raw, maxPixelSize: nil)
        if let cached = cache.object(forKey: cacheKey) {
            return cached
        }
        guard let image = decodedImage(from: raw, maxPixelSize: nil) else {
            return nil
        }
        cache.setObject(image, forKey: cacheKey, cost: raw.utf8.count)
        return image
    }

    static func predecodeAgentAvatars(from rawValues: [String?]) {
        predecode(rawValues, maxPixelSize: agentAvatarMaxPixelSize)
    }

    static func predecodeAgentAvatar(from rawValue: String?) async {
        await predecodeOne(rawValue, maxPixelSize: agentAvatarMaxPixelSize)
    }

    static func predecodeChannelIcons(from rawValues: [String?]) {
        predecode(rawValues, maxPixelSize: channelIconMaxPixelSize)
    }

    static func predecode(_ rawValues: [String?], maxPixelSize: CGFloat) {
        let jobs = rawValues.compactMap { predecodeJob(for: $0, maxPixelSize: maxPixelSize) }
        guard !jobs.isEmpty else { return }

        predecodeQueue.async {
            for job in jobs {
                performPredecode(job, maxPixelSize: maxPixelSize)
            }
        }
    }

    private static func predecodeOne(_ rawValue: String?, maxPixelSize: CGFloat) async {
        guard let job = predecodeJob(for: rawValue, maxPixelSize: maxPixelSize) else { return }
        await Task.detached(priority: .utility) {
            performPredecode(job, maxPixelSize: maxPixelSize)
        }.value
    }

    private static func normalizedRawValue(_ rawValue: String?) -> String? {
        let raw = (rawValue ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return raw.isEmpty ? nil : raw
    }

    private static func cacheKey(for raw: String, maxPixelSize: CGFloat?) -> NSString {
        if let maxPixelSize {
            return NSString(string: "\(Int(maxPixelSize.rounded(.up)))|\(raw)")
        }
        return NSString(string: "full|\(raw)")
    }

    private static func predecodeJob(for rawValue: String?, maxPixelSize: CGFloat) -> PredecodeJob? {
        guard let raw = normalizedRawValue(rawValue),
              !isRemoteURL(raw) else {
            return nil
        }
        let keyString = cacheKey(for: raw, maxPixelSize: maxPixelSize) as String
        guard cache.object(forKey: NSString(string: keyString)) == nil else { return nil }
        guard reserveScheduledKey(keyString) else { return nil }
        return PredecodeJob(raw: raw, keyString: keyString)
    }

    private static func performPredecode(_ job: PredecodeJob, maxPixelSize: CGFloat) {
        let cacheKey = NSString(string: job.keyString)
        autoreleasepool {
            if cache.object(forKey: cacheKey) == nil,
               let image = decodedImage(from: job.raw, maxPixelSize: maxPixelSize) {
                cache.setObject(image, forKey: cacheKey, cost: cost(for: image, raw: job.raw))
            }
        }
        releaseScheduledKey(job.keyString)
    }

    private static func reserveScheduledKey(_ key: String) -> Bool {
        predecodeState.reserve(key)
    }

    private static func releaseScheduledKey(_ key: String) {
        predecodeState.release(key)
    }

    private static func isRemoteURL(_ raw: String) -> Bool {
        raw.hasPrefix("http://") || raw.hasPrefix("https://")
    }

    private static func decodedImage(from raw: String, maxPixelSize: CGFloat?) -> UIImage? {
        let encoded = raw.split(separator: ",", maxSplits: 1).last.map(String.init) ?? raw
        guard let data = Data(base64Encoded: encoded) else {
            return nil
        }
        if let maxPixelSize {
            return GaryxImageDecoder.image(from: data, maxPixelSize: maxPixelSize)
        }
        return UIImage(data: data)
    }

    private static func cost(for image: UIImage, raw: String) -> Int {
        if let cgImage = image.cgImage {
            return max(raw.utf8.count, cgImage.bytesPerRow * cgImage.height)
        }
        return raw.utf8.count
    }

    private struct PredecodeJob: Sendable {
        var raw: String
        var keyString: String
    }
}

private final class GaryxDataURLImagePredecodeState: @unchecked Sendable {
    private let lock = NSLock()
    private var scheduledKeys = Set<String>()

    func reserve(_ key: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard !scheduledKeys.contains(key) else { return false }
        scheduledKeys.insert(key)
        return true
    }

    func release(_ key: String) {
        lock.lock()
        scheduledKeys.remove(key)
        lock.unlock()
    }
}

struct GaryxPanelScaffold<Content: View, Actions: View>: View {
    @Environment(\.garyxOpenSidebar) private var openSidebar
    @EnvironmentObject private var model: GaryxMobileModel

    let title: String
    let subtitle: String
    let onRefresh: (() async -> Void)?
    let showsRefreshButton: Bool
    let leadingActionLabel: String?
    let leadingActionSystemName: String
    let leadingAction: (() -> Void)?
    let background: Color
    let contentHorizontalPadding: CGFloat
    let content: Content
    let actions: Actions

    init(
        title: String,
        subtitle: String,
        onRefresh: (() async -> Void)? = nil,
        showsRefreshButton: Bool? = nil,
        leadingActionLabel: String? = nil,
        leadingActionSystemName: String = "chevron.left",
        leadingAction: (() -> Void)? = nil,
        background: Color = GaryxTheme.background,
        contentHorizontalPadding: CGFloat = 16,
        @ViewBuilder content: () -> Content,
        @ViewBuilder actions: () -> Actions
    ) {
        self.title = title
        self.subtitle = subtitle
        self.onRefresh = onRefresh
        // Pull-to-refresh, on-appear loads, and the per-gateway event
        // streams already cover freshness — header refresh buttons are
        // opt-in chrome, not the default.
        self.showsRefreshButton = showsRefreshButton ?? false
        self.leadingActionLabel = leadingActionLabel
        self.leadingActionSystemName = leadingActionSystemName
        self.leadingAction = leadingAction
        self.background = background
        // Pages whose content is sidebar-style row sections (which carry
        // their own outer row padding, matching the home pinned+recent
        // list) pass 0 so rows keep the same edge geometry as home.
        self.contentHorizontalPadding = contentHorizontalPadding
        self.content = content()
        self.actions = actions()
    }

    var body: some View {
        ScrollView {
            content
                .padding(.horizontal, contentHorizontalPadding)
                .padding(.vertical, 10)
                .frame(maxWidth: 560, alignment: .leading)
                .garyxVerticalScrollContentWidth(maxWidth: 560)
        }
        .refreshable {
            if let onRefresh {
                await onRefresh()
            }
        }
        .background(background)
        .garyxAdaptiveTopBar {
            GaryxAdaptiveGlassContainer(spacing: 10) {
                HStack(spacing: 12) {
                    if let leadingAction {
                        Button {
                            leadingAction()
                        } label: {
                            GaryxToolbarIcon(systemName: leadingActionSystemName)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(leadingActionLabel ?? "Back")
                    } else if model.mainPanelLeadingEdgeAction != .openSidebar {
                        Button {
                            model.performMainPanelLeadingEdgeAction()
                        } label: {
                            GaryxToolbarIcon(systemName: "chevron.left")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(model.mainPanelLeadingEdgeActionLabel)
                    } else {
                        GaryxSidebarMenuButton {
                            openSidebar()
                        }
                    }

                    GaryxPanelHeaderTitle(title: title, subtitle: subtitle)
                        .layoutPriority(1)

                    Spacer(minLength: 0)

                    if let onRefresh, showsRefreshButton {
                        Button {
                            Task { await onRefresh() }
                        } label: {
                            GaryxToolbarIcon(systemName: "arrow.clockwise")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Refresh")
                    }

                    actions
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)
        }
    }
}

struct GaryxPanelHeaderTitle: View {
    let title: String

    init(title: String, subtitle: String = "") {
        self.title = title
    }

    var body: some View {
        Text(title)
            .font(GaryxFont.callout(weight: .medium))
            .foregroundStyle(.primary)
            .lineLimit(1)
            .truncationMode(.tail)
            .padding(.horizontal, 14)
            .frame(height: 44, alignment: .leading)
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: false,
                fallbackMaterial: .ultraThinMaterial,
                in: Capsule()
            )
    }
}

extension GaryxPanelScaffold where Actions == EmptyView {
    init(
        title: String,
        subtitle: String,
        onRefresh: (() async -> Void)? = nil,
        showsRefreshButton: Bool? = nil,
        leadingActionLabel: String? = nil,
        leadingActionSystemName: String = "chevron.left",
        leadingAction: (() -> Void)? = nil,
        background: Color = GaryxTheme.background,
        @ViewBuilder content: () -> Content
    ) {
        self.init(
            title: title,
            subtitle: subtitle,
            onRefresh: onRefresh,
            showsRefreshButton: showsRefreshButton,
            leadingActionLabel: leadingActionLabel,
            leadingActionSystemName: leadingActionSystemName,
            leadingAction: leadingAction,
            background: background,
            content: content,
            actions: { EmptyView() }
        )
    }
}

struct GaryxAddToolbarButton: View {
    let label: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            GaryxToolbarIcon(systemName: "plus")
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}
