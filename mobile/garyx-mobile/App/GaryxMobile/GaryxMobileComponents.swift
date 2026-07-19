import Foundation
import SwiftUI
import UIKit

private struct GaryxOpenSidebarActionKey: EnvironmentKey {
    static let defaultValue: () -> Void = {}
}

private struct GaryxLoadMoreThreadsActionKey: EnvironmentKey {
    static let defaultValue: (GaryxThreadListLoadMoreTrigger) async -> Void = { _ in }
}

private struct GaryxRetryLoadMoreThreadsActionKey: EnvironmentKey {
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

    var garyxLoadMoreThreads: (GaryxThreadListLoadMoreTrigger) async -> Void {
        get { self[GaryxLoadMoreThreadsActionKey.self] }
        set { self[GaryxLoadMoreThreadsActionKey.self] = newValue }
    }

    var garyxRetryLoadMoreThreads: () async -> Void {
        get { self[GaryxRetryLoadMoreThreadsActionKey.self] }
        set { self[GaryxRetryLoadMoreThreadsActionKey.self] = newValue }
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

    static func cachedAvatarImage(
        identity: GaryxAvatarIdentity,
        fingerprint: String,
        maxPixelSize: CGFloat
    ) -> UIImage? {
        cache.object(forKey: avatarCacheKey(identity: identity, fingerprint: fingerprint, maxPixelSize: maxPixelSize))
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

    static func image(from rawValue: String?, maxPixelSize: CGFloat) -> UIImage? {
        guard let raw = normalizedRawValue(rawValue) else { return nil }
        let cacheKey = cacheKey(for: raw, maxPixelSize: maxPixelSize)
        if let cached = cache.object(forKey: cacheKey) {
            return cached
        }
        guard let image = decodedImage(from: raw, maxPixelSize: maxPixelSize) else {
            return nil
        }
        cache.setObject(image, forKey: cacheKey, cost: cost(for: image, raw: raw))
        return image
    }

    static func image(from data: Data, cacheKey key: NSString, maxPixelSize: CGFloat) -> UIImage? {
        if let cached = cache.object(forKey: key) {
            return cached
        }
        guard let image = GaryxImageDecoder.image(from: data, maxPixelSize: maxPixelSize) else {
            return nil
        }
        cache.setObject(image, forKey: key, cost: cost(for: image, byteCount: data.count))
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

    static func avatarCacheKey(
        identity: GaryxAvatarIdentity,
        fingerprint: String,
        maxPixelSize: CGFloat
    ) -> NSString {
        NSString(string: "avatar|\(Int(maxPixelSize.rounded(.up)))|\(identity.storageKey)|\(fingerprint)")
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

    private static func cost(for image: UIImage, byteCount: Int) -> Int {
        if let cgImage = image.cgImage {
            return max(byteCount, cgImage.bytesPerRow * cgImage.height)
        }
        return byteCount
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
    @Environment(\.garyxRouteNavigationActions) private var routeNavigation

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
                    } else if let dismiss = routeNavigation.dismiss {
                        Button {
                            dismiss()
                        } label: {
                            GaryxToolbarIcon(systemName: "chevron.left")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(routeNavigation.backLabel)
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

/// Native-list sibling for thread drilldowns. Keeping this separate avoids a
/// List nested inside the non-list panel's ScrollView.
struct GaryxListPanelScaffold<Rows: View, Actions: View>: View {
    @Environment(\.garyxOpenSidebar) private var openSidebar
    @Environment(\.garyxRouteNavigationActions) private var routeNavigation

    let title: String
    let onRefresh: (() async -> Void)?
    let leadingActionLabel: String?
    let leadingAction: (() -> Void)?
    let rows: Rows
    let actions: Actions

    init(
        title: String,
        onRefresh: (() async -> Void)? = nil,
        leadingActionLabel: String? = nil,
        leadingAction: (() -> Void)? = nil,
        @ViewBuilder rows: () -> Rows,
        @ViewBuilder actions: () -> Actions
    ) {
        self.title = title
        self.onRefresh = onRefresh
        self.leadingActionLabel = leadingActionLabel
        self.leadingAction = leadingAction
        self.rows = rows()
        self.actions = actions()
    }

    var body: some View {
        List {
            rows
                .listRowSeparator(.hidden)
                .listRowInsets(EdgeInsets())
                .listRowBackground(Color.clear)
        }
        .listStyle(.plain)
        .environment(\.defaultMinListRowHeight, 0)
        .scrollContentBackground(.hidden)
        .background(GaryxTheme.background)
        .refreshable {
            if let onRefresh { await onRefresh() }
        }
        .garyxThreadActionMenuHost()
        .garyxAdaptiveTopBar {
            GaryxAdaptiveGlassContainer(spacing: 10) {
                HStack(spacing: 12) {
                    if let leadingAction {
                        Button(action: leadingAction) {
                            GaryxToolbarIcon(systemName: "chevron.left")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(leadingActionLabel ?? "Back")
                    } else if let dismiss = routeNavigation.dismiss {
                        Button(action: dismiss) {
                            GaryxToolbarIcon(systemName: "chevron.left")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(routeNavigation.backLabel)
                    } else {
                        GaryxSidebarMenuButton(action: openSidebar)
                    }

                    GaryxPanelHeaderTitle(title: title)
                        .layoutPriority(1)
                    Spacer(minLength: 0)
                    actions
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)
        }
    }
}

extension GaryxListPanelScaffold where Actions == EmptyView {
    init(
        title: String,
        onRefresh: (() async -> Void)? = nil,
        leadingActionLabel: String? = nil,
        leadingAction: (() -> Void)? = nil,
        @ViewBuilder rows: () -> Rows
    ) {
        self.init(
            title: title,
            onRefresh: onRefresh,
            leadingActionLabel: leadingActionLabel,
            leadingAction: leadingAction,
            rows: rows,
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
