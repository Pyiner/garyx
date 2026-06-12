import Foundation
import SwiftUI
import UIKit

private struct GaryxOpenSidebarActionKey: EnvironmentKey {
    static let defaultValue: () -> Void = {}
}

private struct GaryxSidebarDragActiveKey: EnvironmentKey {
    static let defaultValue: Bool = false
}

extension EnvironmentValues {
    var garyxOpenSidebar: () -> Void {
        get { self[GaryxOpenSidebarActionKey.self] }
        set { self[GaryxOpenSidebarActionKey.self] = newValue }
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
    private static let cache: NSCache<NSString, UIImage> = {
        let cache = NSCache<NSString, UIImage>()
        cache.countLimit = 128
        cache.totalCostLimit = 32 * 1024 * 1024
        return cache
    }()

    static func image(from rawValue: String?) -> UIImage? {
        let raw = (rawValue ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return nil }
        let cacheKey = NSString(string: raw)
        if let cached = cache.object(forKey: cacheKey) {
            return cached
        }
        let encoded = raw.split(separator: ",", maxSplits: 1).last.map(String.init) ?? raw
        guard let data = Data(base64Encoded: encoded),
              let image = UIImage(data: data) else {
            return nil
        }
        cache.setObject(image, forKey: cacheKey, cost: data.count)
        return image
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
