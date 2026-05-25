import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

enum GaryxMobileMotion {
    static let sidebar = Animation.interactiveSpring(response: 0.28, dampingFraction: 0.92, blendDuration: 0.08)
    static let sidebarDrilldown = Animation.easeOut(duration: 0.16)
    static let rowSwipe = Animation.interactiveSpring(response: 0.22, dampingFraction: 0.92, blendDuration: 0.04)
}

struct GaryxRootView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        ZStack {
            GaryxTheme.background.ignoresSafeArea()

            if model.hasGatewaySettings, case .ready = model.connectionState {
                GaryxShellView()
            } else {
                GaryxGatewaySetupView()
            }
        }
        .overlay(alignment: .top) {
            GaryxGlobalErrorToastHost(topOffset: 72)
        }
        .environment(\.garyxOpenSidebar) {
            model.setSidebarVisible(true)
        }
        .task {
            #if DEBUG
            guard !model.debugSnapshotActive else { return }
            #endif
            if model.canConnectGateway {
                await model.connectAndRefresh()
            }
        }
        .onOpenURL { url in
            #if DEBUG
            if model.applyDebugURL(url) {
                return
            }
            #endif
            Task { await model.applyMobileConnectLink(url) }
        }
        .sheet(isPresented: $model.showsSettings) {
            GaryxGatewaySetupView(isSheet: true)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
        }
    }
}

struct GaryxGatewaySetupView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    var isSheet = false
    var startsEmpty = false
    @State private var draftGatewayURL = ""
    @State private var draftGatewayAuthToken = ""
    @State private var didInitializeDraft = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Spacer(minLength: 32)

                VStack(spacing: 20) {
                    GaryxAppLogo(size: 88)

                    GaryxConnectionPill(
                        label: setupStatusLabel,
                        color: setupStatusColor,
                        isBusy: setupIsBusy
                    )

                    VStack(spacing: 10) {
                        Text("Gary X")
                            .font(GaryxFont.largeTitle(weight: .semibold))
                            .foregroundStyle(.primary)

                        Text("Set the gateway address and token, then save. Saving verifies the gateway before continuing.")
                            .font(GaryxFont.callout())
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxWidth: 280)

                    VStack(spacing: 10) {
                        HStack(spacing: 8) {
                            TextField("Gateway URL", text: $draftGatewayURL)
                                .textContentType(.URL)
                                .keyboardType(.URL)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .garyxInputStyle()

                            GaryxGatewayProfileMenuButton { profile in
                                model.selectGatewayProfile(profile)
                                draftGatewayURL = model.gatewayURL
                                draftGatewayAuthToken = model.gatewayAuthToken
                            }
                        }

                        SecureField("Gateway Token", text: $draftGatewayAuthToken)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .garyxInputStyle()
                    }

                    GaryxPrimaryCapsuleButton(
                        title: setupIsBusy ? "Saving..." : "Save and Continue",
                        systemImage: setupIsBusy ? nil : "checkmark.circle.fill"
                    ) {
                        Task {
                            model.gatewayURL = draftGatewayURL
                            model.gatewayAuthToken = draftGatewayAuthToken
                            await model.connectAndRefresh()
                            if isSheet, case .ready = model.connectionState {
                                dismiss()
                            }
                        }
                    }
                    .disabled(!canSaveGateway || setupIsBusy)
                    .opacity(canSaveGateway && !setupIsBusy ? 1 : 0.45)
                }
                .frame(maxWidth: 320)
                .padding(.horizontal, 24)

                Spacer(minLength: 24)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(GaryxTheme.background)
            .navigationTitle("Gary X")
            .navigationBarTitleDisplayMode(.inline)
            .onAppear(perform: initializeDraft)
            .toolbar {
                if isSheet {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Done") {
                            model.showsSettings = false
                            dismiss()
                        }
                    }
                }
            }
            .overlay(alignment: .top) {
                if isSheet {
                    GaryxGlobalErrorToastHost(topOffset: 8)
                }
            }
        }
    }

    private var canSaveGateway: Bool {
        let trimmed = draftGatewayURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              ["http", "https"].contains(scheme),
              components.host != nil else {
            return false
        }
        return true
    }

    private func initializeDraft() {
        guard !didInitializeDraft else { return }
        draftGatewayURL = startsEmpty ? "" : model.gatewayURL
        draftGatewayAuthToken = startsEmpty ? "" : model.gatewayAuthToken
        didInitializeDraft = true
    }

    private var setupIsBusy: Bool {
        if case .checking = model.connectionState {
            return true
        }
        return false
    }

    private var setupStatusLabel: String {
        if startsEmpty && draftGatewayURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return "Add Gateway"
        }
        switch model.connectionState {
        case .disconnected:
            return "Not connected"
        case .checking:
            return "Connecting"
        case .ready:
            return "Connected"
        case .failed:
            return "Offline"
        }
    }

    private var setupStatusColor: Color {
        if startsEmpty && draftGatewayURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return Color(.tertiaryLabel)
        }
        switch model.connectionState {
        case .checking:
            return .orange
        case .ready:
            return .green
        case .disconnected, .failed:
            return Color(.tertiaryLabel)
        }
    }
}

struct GaryxShellView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.colorScheme) private var colorScheme

    @State private var sidebarDragOffset: CGFloat = 0
    @State private var sidebarDragAxis: GaryxSidebarDragAxis?

    private let sidebarWidth: CGFloat = 330
    private let sidebarEdgeGestureWidth: CGFloat = 24
    private let sidebarAxisDecisionDistance: CGFloat = 14
    private let sidebarAxisDecisionRatio: CGFloat = 1.5

    var body: some View {
        GeometryReader { proxy in
            let usePersistentSidebar = proxy.size.width > 760 && horizontalSizeClass != .compact
            let currentSidebarWidth = min(sidebarWidth, proxy.size.width)

            Group {
                if usePersistentSidebar {
                    HStack(spacing: 0) {
                        GaryxThreadSidebar(showsInlineCloseButton: false)
                            .frame(width: currentSidebarWidth)

                        GaryxMainPanelView()
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                    .background(GaryxTheme.background)
                } else {
                    drawerBody(width: drawerSidebarWidth(for: proxy.size), containerSize: proxy.size)
                }
            }
            .onChange(of: usePersistentSidebar) { _, isPersistent in
                sidebarDragOffset = 0
                if isPersistent {
                    model.setSidebarVisible(false, animated: false)
                }
            }
        }
        .onChange(of: horizontalSizeClass) { _, _ in
            sidebarDragOffset = 0
        }
    }

    private func drawerSidebarWidth(for containerSize: CGSize) -> CGFloat {
        if horizontalSizeClass == .compact {
            return containerSize.width
        }
        return min(sidebarWidth, containerSize.width * 0.92)
    }

    private func drawerBody(width: CGFloat, containerSize: CGSize) -> some View {
        let revealWidth = sidebarRevealWidth(for: width)
        let drawerOffset = revealWidth - width
        let closeStripX = max(0, min(revealWidth, max(0, containerSize.width - 28)))

        return ZStack(alignment: .topLeading) {
            GaryxMainPanelView()
                .frame(width: containerSize.width, height: containerSize.height)
                .contentShape(Rectangle())
                .simultaneousGesture(openingSidebarGesture(sidebarWidth: width))
                .zIndex(0)

            (colorScheme == .dark ? Color.white : Color.black)
                .opacity(contentDimOpacity(for: width))
                .frame(width: containerSize.width, height: containerSize.height)
                .ignoresSafeArea()
                .contentShape(Rectangle())
                .allowsHitTesting(revealWidth > 1)
                .onTapGesture { closeSidebar() }
                .gesture(closingSidebarGesture(sidebarWidth: width))
                .zIndex(1)

            GaryxThreadSidebar(showsInlineCloseButton: true)
                .frame(width: width)
                .frame(maxHeight: .infinity)
                .offset(x: drawerOffset)
                .allowsHitTesting(revealWidth > width * 0.82)
                .zIndex(2)

            if revealWidth > 1 {
                Color.clear
                    .frame(width: 28, height: containerSize.height)
                    .offset(x: closeStripX)
                    .contentShape(Rectangle())
                    .simultaneousGesture(closingSidebarGesture(sidebarWidth: width))
                    .zIndex(3)
                    .accessibilityHidden(true)
            }
        }
        .background(GaryxTheme.background)
    }

    private func sidebarRevealWidth(for width: CGFloat) -> CGFloat {
        if model.sidebarVisible {
            return max(0, min(width, width + sidebarDragOffset))
        }
        return max(0, min(width, sidebarDragOffset))
    }

    private func contentDimOpacity(for width: CGFloat) -> Double {
        guard width > 0 else { return 0 }
        return 0.12 * min(1, sidebarRevealWidth(for: width) / width)
    }

    private func openingSidebarGesture(sidebarWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .global)
            .onChanged { value in
                guard !model.sidebarVisible else { return }
                if sidebarDragAxis == nil {
                    sidebarDragAxis = decideSidebarAxis(
                        translation: value.translation,
                        startLocation: value.startLocation,
                        opening: true
                    )
                }
                guard sidebarDragAxis == .horizontal else { return }
                switch model.mainPanelLeadingEdgeAction {
                case .openSidebar:
                    sidebarDragOffset = max(0, min(sidebarWidth, value.translation.width))
                case .settingsOverview, .workspaceBotsOverview:
                    sidebarDragOffset = 0
                }
            }
            .onEnded { value in
                defer {
                    sidebarDragAxis = nil
                }
                guard !model.sidebarVisible, sidebarDragAxis == .horizontal else {
                    resetSidebarDrag()
                    return
                }
                let shouldOpen = value.translation.width > sidebarWidth * 0.22
                    || value.predictedEndTranslation.width > sidebarWidth * 0.35
                switch model.mainPanelLeadingEdgeAction {
                case .openSidebar:
                    finishGesture(open: shouldOpen)
                case .settingsOverview, .workspaceBotsOverview:
                    resetSidebarDrag()
                    if shouldOpen {
                        hideKeyboard()
                        withAnimation(GaryxMobileMotion.sidebarDrilldown) {
                            model.performMainPanelLeadingEdgeAction()
                        }
                    }
                }
            }
    }

    private func closingSidebarGesture(sidebarWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .global)
            .onChanged { value in
                guard model.sidebarVisible else { return }
                if sidebarDragAxis == nil {
                    sidebarDragAxis = decideSidebarAxis(
                        translation: value.translation,
                        startLocation: value.startLocation,
                        opening: false
                    )
                }
                guard sidebarDragAxis == .horizontal else { return }
                sidebarDragOffset = min(0, max(-sidebarWidth, value.translation.width))
            }
            .onEnded { value in
                defer {
                    sidebarDragAxis = nil
                }
                guard model.sidebarVisible, sidebarDragAxis == .horizontal else {
                    resetSidebarDrag()
                    return
                }
                let shouldClose = -value.translation.width > sidebarWidth * 0.22
                    || -value.predictedEndTranslation.width > sidebarWidth * 0.35
                finishGesture(open: !shouldClose)
            }
    }

    private func decideSidebarAxis(
        translation: CGSize,
        startLocation: CGPoint,
        opening: Bool
    ) -> GaryxSidebarDragAxis? {
        let horizontal = translation.width
        let vertical = translation.height
        let horizontalMag = abs(horizontal)
        let verticalMag = abs(vertical)
        let dominant = max(horizontalMag, verticalMag)
        guard dominant >= sidebarAxisDecisionDistance else { return nil }
        guard horizontalMag > verticalMag * sidebarAxisDecisionRatio else {
            return .vertical
        }
        if opening {
            guard horizontal > 0,
                  startLocation.x <= sidebarEdgeGestureWidth else {
                return .vertical
            }
        } else {
            guard horizontal < 0 else { return .vertical }
        }
        return .horizontal
    }

    private func finishGesture(open: Bool) {
        hideKeyboard()
        withAnimation(GaryxMobileMotion.sidebar) {
            model.setSidebarVisible(open, animated: false)
            sidebarDragOffset = 0
        }
    }

    private func resetSidebarDrag() {
        withAnimation(GaryxMobileMotion.sidebar) {
            sidebarDragOffset = 0
        }
    }

    private func closeSidebar() {
        finishGesture(open: false)
    }

    private func hideKeyboard() {
        UIApplication.shared.sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil,
            from: nil,
            for: nil
        )
    }
}

struct GaryxWorkspacesView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var isPickingFiles = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Workspaces",
            subtitle: subtitle,
            onRefresh: { await model.refreshSelectedWorkspace() }
        ) {
            GaryxWorkspacesContent()
        } actions: {
            Button {
                isPickingFiles = true
            } label: {
                GaryxToolbarIcon(systemName: model.isUploadingWorkspaceFiles ? "hourglass" : "square.and.arrow.up")
            }
            .buttonStyle(.plain)
            .disabled(model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.isUploadingWorkspaceFiles)
            .accessibilityLabel("Upload Files")
        }
        .task {
            await model.prepareWorkspaceBrowser()
        }
        .onChange(of: model.knownWorkspacePaths) { _, _ in
            Task { await model.prepareWorkspaceBrowser() }
        }
        .fileImporter(
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
        guard !workspace.isEmpty else { return "\(model.knownWorkspacePaths.count) workspaces" }
        let name = workspace.garyxLastPathComponent.isEmpty ? workspace : workspace.garyxLastPathComponent
        let directory = model.selectedWorkspaceDirectory.trimmingCharacters(in: .whitespacesAndNewlines)
        return directory.isEmpty ? name : "\(name) / \(directory)"
    }
}

struct GaryxWorkspacesContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        let paths = model.knownWorkspacePaths
        VStack(alignment: .leading, spacing: 12) {
            if paths.isEmpty {
                GaryxEmptyPanelView(
                    icon: "folder",
                    title: "No workspaces",
                    text: ""
                )
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
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(isSelected ? .primary : .secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(path.garyxLastPathComponent.isEmpty ? path : path.garyxLastPathComponent)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(garyxCompactPathLabel(path))
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 0)

                if isSelected {
                    Image(systemName: "checkmark")
                        .font(GaryxFont.system(size: 12, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(path.garyxLastPathComponent.isEmpty ? path : path.garyxLastPathComponent)
        .accessibilityValue(garyxCompactPathLabel(path))
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
                    .font(GaryxFont.system(size: 15, weight: .semibold))
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
        .buttonStyle(.plain)
    }
}

struct GaryxWorkspaceEmptyDirectoryRow: View {
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "tray")
                .font(GaryxFont.system(size: 15, weight: .semibold))
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
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(entry.entryType == "directory" ? .primary : .secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(entry.name.isEmpty ? entry.path.garyxLastPathComponent : entry.name)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(detail)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                Image(systemName: entry.entryType == "directory" ? "chevron.right" : "doc.text.magnifyingglass")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
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
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 28, height: 28)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(preview.name)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(preview.path)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    Spacer(minLength: 0)
                    GaryxStatusPill(text: preview.previewKind.capitalized, tone: .muted)
                }

                if let text = preview.text, !text.isEmpty {
                    ScrollView([.vertical, .horizontal], showsIndicators: true) {
                        Text(text)
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(.primary)
                            .textSelection(.enabled)
                            .padding(10)
                    }
                    .frame(maxHeight: 240, alignment: .topLeading)
                    .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                } else if let image {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .frame(maxWidth: .infinity, maxHeight: 260)
                        .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                } else {
                    Text(preview.previewKind == "pdf" ? "PDF preview available on desktop." : "No inline preview available.")
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                        .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }

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

    private var image: UIImage? {
        guard preview.previewKind == "image",
              let dataBase64 = preview.dataBase64 else {
            return nil
        }
        return GaryxDataURLImageCache.image(from: dataBase64)
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

private func garyxCompactPathLabel(_ path: String) -> String {
    let normalized = path
        .trimmingCharacters(in: .whitespacesAndNewlines)
        .replacingOccurrences(of: "\\", with: "/")
    guard !normalized.isEmpty else { return "" }
    let parts = normalized
        .split(separator: "/", omittingEmptySubsequences: true)
        .map(String.init)
    if parts.count >= 2,
       parts[0] == "Users" {
        let remainder = Array(parts.dropFirst(2))
        guard !remainder.isEmpty else { return "Home folder" }
        return "~/" + remainder.prefix(2).joined(separator: "/")
    }
    if parts.count > 2 {
        return parts.suffix(2).joined(separator: "/")
    }
    return normalized
}

struct GaryxTasksView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateTask = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Tasks",
            subtitle: "\(model.activeTaskCount) active / \(model.tasks.count) total",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                if model.tasks.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "checklist",
                        title: "No tasks yet.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Tasks") {
                        GaryxCompactListGroup {
                            GaryxTaskList(tasks: model.tasks)
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Task") {
                showsCreateTask = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateTask) {
            GaryxFormSheet(title: "New Task") {
                GaryxCreateTaskCard()
            }
        }
    }
}

struct GaryxDreamsView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxPanelScaffold(
            title: "Dreams",
            subtitle: subtitle,
            onRefresh: { await model.refreshDreams() }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                GaryxSectionBlock(title: "Settings") {
                    GaryxCompactListGroup {
                        GaryxDreamsAutoScanRow()
                    }
                }

                if model.dreams.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "moon.stars",
                        title: "No dreams yet.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Last 24 Hours") {
                        GaryxCompactListGroup {
                            GaryxDreamTopicList(dreams: model.dreams)
                        }
                    }
                }
            }
        } actions: {
            Button {
                Task { await model.scanDreams() }
            } label: {
                GaryxToolbarIcon(systemName: model.isScanningDreams ? "hourglass" : "sparkles")
            }
            .buttonStyle(.plain)
            .disabled(model.isScanningDreams)
            .accessibilityLabel("Scan dreams")
        }
    }

    private var subtitle: String {
        if let scan = model.latestDreamScan {
            let status = scan.status.trimmingCharacters(in: .whitespacesAndNewlines)
            let updated = garyxFormattedTaskTimestamp(scan.createdAt)
            let statusText = status.isEmpty ? "scan" : status
            return updated.isEmpty
                ? "\(model.dreams.count) topics / \(statusText)"
                : "\(model.dreams.count) topics / \(statusText) \(updated)"
        }
        return "\(model.dreams.count) topics"
    }
}

struct GaryxDreamsAutoScanRow: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "clock.arrow.2.circlepath")
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 24, height: 24)

            VStack(alignment: .leading, spacing: 3) {
                Text("Dreams")
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                Text("Shows Dreams in the app and runs periodic scans when recent user messages exist.")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }

            Spacer(minLength: 0)

            Toggle(
                "Dreams",
                isOn: Binding(
                    get: { model.dreamsAutoScanEnabled },
                    set: { nextValue in
                        Task { await model.setDreamsAutoScanEnabled(nextValue) }
                    }
                )
            )
            .labelsHidden()
            .toggleStyle(.switch)
            .disabled(model.isSavingDreamsSettings)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }
}

struct GaryxDreamTopicList: View {
    let dreams: [GaryxDreamTopic]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(dreams.enumerated()), id: \.element.id) { index, dream in
                GaryxDreamTopicRow(dream: dream)
                if index < dreams.count - 1 {
                    GaryxCompactRowDivider()
                }
            }
        }
    }
}

struct GaryxDreamTopicRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let dream: GaryxDreamTopic

    var body: some View {
        Button {
            if let firstSpan = dream.spans.first {
                Task { await model.openDreamSpan(firstSpan) }
            }
        } label: {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(dream.title)
                        .font(GaryxFont.body(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)

                    Spacer(minLength: 8)

                    Text("\(dream.messageCount)")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .frame(height: 24)
                        .background(Color(.tertiarySystemFill), in: Capsule())
                }

                if !dream.summary.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(dream.summary)
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                }

                VStack(alignment: .leading, spacing: 6) {
                    ForEach(dream.spans.prefix(3)) { span in
                        GaryxDreamSpanRow(span: span)
                    }
                }

                HStack(spacing: 8) {
                    Text(dream.sourceDisplayLabel)
                    Spacer(minLength: 8)
                    Text(dream.formattedLastMessageAt)
                }
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(dream.spans.isEmpty)
    }
}

struct GaryxDreamSpanRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let span: GaryxDreamSpan

    var body: some View {
        Button {
            Task { await model.openDreamSpan(span) }
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "arrow.turn.down.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .frame(width: 14)

                VStack(alignment: .leading, spacing: 2) {
                    Text(span.excerpt.isEmpty ? span.threadId : span.excerpt)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                    Text(span.threadDisplayLabel)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 6)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct GaryxTaskList: View {
    let tasks: [GaryxTaskSummary]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(tasks.enumerated()), id: \.element.id) { index, task in
                GaryxTaskListRow(task: task)
                if index < tasks.count - 1 {
                    GaryxCompactRowDivider()
                }
            }
        }
    }
}

struct GaryxCreateTaskCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var workspacePath = ""
    @State private var startImmediately = true
    @State private var notificationTargetId = "none"

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Task")
            TextField("Title", text: $model.draftTaskTitle)
                .garyxInputStyle()
            TextField("Details", text: $model.draftTaskBody, axis: .vertical)
                .lineLimit(3...8)
                .garyxInputStyle()

            GaryxFieldLabel("Assignee")
                .padding(.top, 4)
            Menu {
                ForEach(model.agentTargets) { target in
                    Button {
                        model.setSelectedAgentTarget(target.id)
                    } label: {
                        Label(target.title, systemImage: target.kind == .team ? "person.3" : "person")
                    }
                }
            } label: {
                GaryxAgentPickerLabel(
                    target: model.selectedAgentTarget,
                    title: model.selectedAgentLabel,
                    showsChevron: true,
                    style: .compact
                )
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(GaryxTheme.input, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
            .buttonStyle(.plain)
            .disabled(model.agentTargets.isEmpty)

            GaryxFieldLabel("Workspace")
                .padding(.top, 4)
            TextField("Workspace directory", text: $workspacePath)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()

            GaryxFieldLabel("Notification")
                .padding(.top, 4)
            Menu {
                Button {
                    notificationTargetId = "none"
                } label: {
                    Label("Do not notify", systemImage: notificationTargetId == "none" ? "checkmark" : "bell.slash")
                }
                if !model.mobileBotGroups.isEmpty {
                    Divider()
                    ForEach(model.mobileBotGroups) { group in
                        Button {
                            notificationTargetId = group.id
                        } label: {
                            Label(group.title, systemImage: notificationTargetId == group.id ? "checkmark" : "bell")
                        }
                    }
                }
            } label: {
                HStack(spacing: 8) {
                    Text(notificationTargetLabel)
                        .font(GaryxFont.callout(weight: .medium))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.down")
                        .font(GaryxFont.system(size: 10, weight: .bold))
                        .foregroundStyle(.tertiary)
                }
                .padding(.horizontal, 12)
                .frame(height: 42)
                .background(GaryxTheme.input, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
            .buttonStyle(.plain)

            Toggle("Start immediately", isOn: $startImmediately)
                .font(GaryxFont.callout(weight: .medium))

            Button {
                Task {
                    model.setNewThreadWorkspace(workspacePath)
                    await model.createTaskFromDraft(
                        start: startImmediately,
                        notificationTarget: notificationTargetRequest
                    )
                    if model.draftTaskTitle.isEmpty, model.draftTaskBody.isEmpty {
                        dismiss()
                    }
                }
            } label: {
                Label("Create Task", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
            .disabled(!canCreate)
        }
        .garyxCardStyle()
        .onAppear {
            workspacePath = model.newThreadWorkspace
        }
    }

    private var canCreate: Bool {
        !model.draftTaskTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !model.draftTaskBody.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var selectedNotificationGroup: GaryxMobileBotGroup? {
        model.mobileBotGroups.first { $0.id == notificationTargetId }
    }

    private var notificationTargetLabel: String {
        selectedNotificationGroup?.title ?? "Do not notify"
    }

    private var notificationTargetRequest: GaryxTaskNotificationTargetRequest {
        guard let group = selectedNotificationGroup else { return .none }
        return .bot(channel: group.channel, accountId: group.accountId)
    }
}

struct GaryxTaskListRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let task: GaryxTaskSummary
    @State private var showsAssignSheet = false
    @State private var showsDeleteConfirmation = false
    @State private var showsMoreActions = false
    @State private var showsRenamePrompt = false
    @State private var showsStatusActions = false
    @State private var showsTaskDetails = false
    @State private var renameDraftTitle = ""

    var body: some View {
        GaryxSwipeActionRow(actions: taskSwipeActions) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top, spacing: 8) {
                    Button {
                        if task.threadId.isEmpty {
                            showsTaskDetails = true
                        } else {
                            Task { await model.openThread(id: task.threadId) }
                        }
                    } label: {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(task.title)
                                .font(GaryxFont.subheadline(weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(2)
                                .multilineTextAlignment(.leading)
                            Text(task.displayId)
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)

                    GaryxStatusPill(text: task.status.label, tone: task.status.tone)
                        .fixedSize(horizontal: true, vertical: false)
                }

                HStack(spacing: 8) {
                    Text(task.assigneeDisplayLabel)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: 8)
                    Text(task.formattedUpdatedAt)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .fullScreenCover(isPresented: $showsAssignSheet) {
            GaryxFormSheet(title: "Assign Task") {
                GaryxTaskAssignCard(task: task)
            }
        }
        .fullScreenCover(isPresented: $showsTaskDetails) {
            GaryxFormSheet(title: "Task Details") {
                GaryxTaskDetailCard(task: task)
            }
        }
        .alert("Rename Task", isPresented: $showsRenamePrompt) {
            TextField("Task title", text: $renameDraftTitle)
            Button("Cancel", role: .cancel) {}
            Button("Save") {
                Task { await model.updateTaskTitle(task, title: renameDraftTitle) }
            }
        }
        .confirmationDialog("Task Actions", isPresented: $showsMoreActions, titleVisibility: .visible) {
            Button("Rename") {
                openRenamePrompt()
            }
            if !model.agentTargets.isEmpty {
                Button("Assign") {
                    showsAssignSheet = true
                }
            }
            Button("Details") {
                showsTaskDetails = true
            }
            if task.assignee != nil || !task.assigneeLabel.isEmpty {
                Button("Unassign") {
                    Task { await model.unassignTask(task) }
                }
            }
            Button("Delete", role: .destructive) {
                showsDeleteConfirmation = true
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog("Set Status", isPresented: $showsStatusActions, titleVisibility: .visible) {
            ForEach(task.status.allowedTransitions, id: \.rawValue) { status in
                Button {
                    Task { await model.updateTask(task, to: status) }
                } label: {
                    Label(status.label, systemImage: status.systemImage)
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog("Delete task?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteTask(task) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the task from the task list.")
        }
    }

    private var taskSwipeActions: [GaryxSwipeAction] {
        var actions: [GaryxSwipeAction] = []
        if !task.threadId.isEmpty {
            actions.append(
                GaryxSwipeAction(title: "Open", systemImage: "message", tone: .accent) {
                    Task { await model.openThread(id: task.threadId) }
                }
            )
        }
        if task.threadId.isEmpty {
            actions.append(
                GaryxSwipeAction(title: "Details", systemImage: "info.circle", tone: .accent) {
                    showsTaskDetails = true
                }
            )
        }
        if task.status == .inProgress {
            actions.append(
                GaryxSwipeAction(title: "Stop", systemImage: "stop.fill", tone: .warning) {
                    Task { await model.stopTask(task) }
                }
            )
        }
        actions.append(
            GaryxSwipeAction(title: "Status", systemImage: "arrow.left.arrow.right.circle") {
                showsStatusActions = true
            }
        )
        actions.append(
            GaryxSwipeAction(title: "More", systemImage: "ellipsis.circle") {
                showsMoreActions = true
            }
        )
        return actions
    }

    private func openRenamePrompt() {
        renameDraftTitle = task.title
        showsRenamePrompt = true
    }
}

struct GaryxTaskDetailCard: View {
    let task: GaryxTaskSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(task.title)
                        .font(GaryxFont.title3(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(3)
                    Spacer(minLength: 0)
                    GaryxStatusPill(text: task.status.label, tone: task.status.tone)
                }
                Text(task.displayId)
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
            }

            GaryxCompactListGroup {
                GaryxTaskMetaLine(label: "Assignee", value: task.assigneeDisplayLabel)
                GaryxCompactRowDivider()
                GaryxTaskMetaLine(label: "Runtime", value: task.runtimeAgentId.isEmpty ? "Not assigned" : task.runtimeAgentId)
                GaryxCompactRowDivider()
                GaryxTaskMetaLine(label: "Thread", value: task.threadId.isEmpty ? "No thread" : task.threadId)
                GaryxCompactRowDivider()
                GaryxTaskMetaLine(label: "Replies", value: "\(task.replyCount)")
                GaryxCompactRowDivider()
                GaryxTaskMetaLine(label: "Updated", value: task.formattedUpdatedAt)
                if let creator = task.creator {
                    GaryxCompactRowDivider()
                    GaryxTaskMetaLine(label: "Creator", value: creator.label)
                }
                if let updatedBy = task.updatedBy {
                    GaryxCompactRowDivider()
                    GaryxTaskMetaLine(label: "Updated by", value: updatedBy.label)
                }
                if let source = task.source {
                    GaryxCompactRowDivider()
                    GaryxTaskMetaLine(label: "Source", value: source.detailLabel)
                }
            }

            if task.threadId.isEmpty {
                GaryxNotice(
                    title: "No chat thread yet",
                    text: "Assign or start this task to create a runnable thread."
                )
            }
        }
        .garyxCardStyle()
    }
}

struct GaryxTaskAssignCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let task: GaryxTaskSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Assign To")
            if model.agentTargets.isEmpty {
                GaryxEmptyPanelView(
                    icon: "person.crop.circle.badge.exclamationmark",
                    title: "No agents available.",
                    text: ""
                )
            } else {
                GaryxCompactListGroup {
                    ForEach(Array(model.agentTargets.enumerated()), id: \.element.id) { index, target in
                        Button {
                            Task {
                                await model.assignTask(task, agentId: target.id)
                                dismiss()
                            }
                        } label: {
                            GaryxAgentIdentityRow(
                                id: target.id,
                                title: target.title,
                                subtitle: target.subtitle,
                                kind: target.kind,
                                avatarDataUrl: target.avatarDataUrl,
                                providerType: target.providerType,
                                builtIn: target.builtIn,
                                selected: task.assignee?.agentId == target.id
                                    || task.assigneeLabel == target.id
                                    || task.runtimeAgentId == target.id
                            )
                        }
                        .buttonStyle(.plain)
                        if index < model.agentTargets.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                }
            }
        }
        .garyxCardStyle()
    }
}

struct GaryxTaskMetaLine: View {
    let label: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
                .textCase(.lowercase)
                .frame(width: 76, alignment: .leading)
            Text(value.isEmpty ? "Unknown" : value)
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}

private enum GaryxAgentCreationSheet: String, Identifiable {
    case agent
    case team

    var id: String { rawValue }

    var title: String {
        switch self {
        case .agent:
            "New Agent"
        case .team:
            "New Team"
        }
    }
}

private enum GaryxAgentsTab: String, CaseIterable, Identifiable {
    case agents = "Agents"
    case teams = "Teams"

    var id: String { rawValue }
}

struct GaryxAgentsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var creationSheet: GaryxAgentCreationSheet?
    @State private var selectedTab: GaryxAgentsTab = .agents

    var body: some View {
        GaryxPanelScaffold(
            title: "Agents",
            subtitle: "\(model.agents.count) agents / \(model.teams.count) teams",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                Picker("Agent type", selection: $selectedTab) {
                    ForEach(GaryxAgentsTab.allCases) { tab in
                        Text(tab.rawValue).tag(tab)
                    }
                }
                .pickerStyle(.segmented)

                switch selectedTab {
                case .agents:
                    GaryxSectionBlock(title: "Agents") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.agents.enumerated()), id: \.element.id) { index, agent in
                                GaryxAgentCard(agent: agent)
                                if index < model.agents.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                case .teams:
                    GaryxSectionBlock(title: "Teams") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.teams.enumerated()), id: \.element.id) { index, team in
                                GaryxTeamCard(team: team)
                                if index < model.teams.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: selectedTab == .agents ? "New Agent" : "New Team") {
                creationSheet = selectedTab == .agents ? .agent : .team
            }
        }
        .fullScreenCover(item: $creationSheet) { sheet in
            GaryxFormSheet(title: sheet.title) {
                switch sheet {
                case .agent:
                    GaryxCreateAgentCard()
                case .team:
                    GaryxCreateTeamCard()
                }
            }
        }
    }
}

struct GaryxCreateAgentCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("New Agent")
            TextField("Agent ID", text: $model.draftAgentId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Display name", text: $model.draftAgentName)
                .garyxInputStyle()
            TextField("Provider", text: $model.draftAgentProvider)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Model", text: $model.draftAgentModel)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Default workspace directory", text: $model.draftAgentWorkspace)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("System Prompt", text: $model.draftAgentPrompt, axis: .vertical)
                .lineLimit(2...6)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createAgentFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Create Agent", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxCreateTeamCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("New Team")
            TextField("Team ID", text: $model.draftTeamId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Display name", text: $model.draftTeamName)
                .garyxInputStyle()
            TextField("Leader Agent", text: $model.draftTeamLeaderId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Members", text: $model.draftTeamMemberIds)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Workflow", text: $model.draftTeamWorkflow, axis: .vertical)
                .lineLimit(2...6)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createTeamFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Create Team", systemImage: "person.2.badge.plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxAgentCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let agent: GaryxAgentSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var agentId = ""
    @State private var displayName = ""
    @State private var providerType = ""
    @State private var modelName = ""
    @State private var workspace = ""
    @State private var systemPrompt = ""

    var body: some View {
        GaryxSwipeActionRow(actions: agentSwipeActions) {
            VStack(alignment: .leading, spacing: 10) {
                GaryxAgentIdentityRow(
                    id: agent.id,
                    title: agent.displayName,
                    subtitle: "",
                    kind: .agent,
                    avatarDataUrl: agent.avatarDataUrl,
                    providerType: agent.providerType,
                    builtIn: agent.builtIn,
                    selected: model.selectedAgentTargetId == agent.id
                )
            }
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Agent") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Agent")
                    TextField("Agent ID", text: $agentId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Display name", text: $displayName)
                        .garyxInputStyle()
                    TextField("Provider", text: $providerType)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Model", text: $modelName)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Default workspace directory", text: $workspace)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("System Prompt", text: $systemPrompt, axis: .vertical)
                        .lineLimit(2...6)
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateAgent(
                                agent,
                                agentId: agentId,
                                displayName: displayName,
                                providerType: providerType,
                                modelName: modelName,
                                workspace: workspace,
                                systemPrompt: systemPrompt
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save Agent", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
        .confirmationDialog("Delete agent?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteAgent(agent) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the custom agent configuration.")
        }
    }

    private var agentSwipeActions: [GaryxSwipeAction] {
        var actions = [
            GaryxSwipeAction(title: "Chat", systemImage: "message", tone: .accent) {
                model.setSelectedAgentTarget(agent.id)
                Task { await model.createThread() }
            },
            GaryxSwipeAction(title: "Use", systemImage: "checkmark.circle") {
                model.setSelectedAgentTarget(agent.id)
            }
        ]
        if !agent.builtIn {
            actions.append(
                GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                    fillDraft()
                    showsEditForm = true
                }
            )
            actions.append(
                GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                    showsDeleteConfirmation = true
                }
            )
        }
        return actions
    }

    private func fillDraft() {
        agentId = agent.id
        displayName = agent.displayName
        providerType = agent.providerType
        modelName = agent.model
        workspace = agent.defaultWorkspaceDir
        systemPrompt = agent.systemPrompt
    }
}

struct GaryxTeamCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let team: GaryxTeamSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var teamId = ""
    @State private var displayName = ""
    @State private var leaderAgentId = ""
    @State private var memberAgentIds = ""
    @State private var workflowText = ""

    var body: some View {
        GaryxSwipeActionRow(actions: teamSwipeActions) {
            VStack(alignment: .leading, spacing: 10) {
                GaryxAgentIdentityRow(
                    id: team.id,
                    title: team.displayName,
                    subtitle: "",
                    kind: .team,
                    avatarDataUrl: team.avatarDataUrl,
                    providerType: "",
                    selected: model.selectedAgentTargetId == team.id
                )
                if !team.workflowText.isEmpty {
                    Text(team.workflowText)
                        .font(GaryxFont.footnote())
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .padding(.horizontal, 10)
                }
            }
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Team") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Team")
                    TextField("Team ID", text: $teamId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Display name", text: $displayName)
                        .garyxInputStyle()
                    TextField("Leader Agent", text: $leaderAgentId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Members", text: $memberAgentIds)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Workflow", text: $workflowText, axis: .vertical)
                        .lineLimit(2...6)
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateTeam(
                                team,
                                teamId: teamId,
                                displayName: displayName,
                                leaderAgentId: leaderAgentId,
                                memberAgentIds: memberAgentIds,
                                workflowText: workflowText
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save Team", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
        .confirmationDialog("Delete team?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteTeam(team) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the team configuration.")
        }
    }

    private var teamSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "Chat", systemImage: "message", tone: .accent) {
                model.setSelectedAgentTarget(team.id)
                Task { await model.createThread() }
            },
            GaryxSwipeAction(title: "Use", systemImage: "checkmark.circle") {
                model.setSelectedAgentTarget(team.id)
            },
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }

    private func fillDraft() {
        teamId = team.id
        displayName = team.displayName
        leaderAgentId = team.leaderAgentId
        memberAgentIds = team.memberAgentIds.joined(separator: ", ")
        workflowText = team.workflowText
    }
}

struct GaryxSkillsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateSkill = false
    @State private var showsDiscardSkillEditorConfirmation = false

    private var skillEditorPresented: Binding<Bool> {
        Binding(
            get: { model.selectedSkillEditor != nil },
            set: { isPresented in
                if !isPresented {
                    requestCloseSkillEditor()
                }
            }
        )
    }

    var body: some View {
        GaryxPanelScaffold(
            title: "Skills",
            subtitle: "\(model.skills.filter(\.enabled).count) enabled / \(model.skills.count) total",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                if model.skills.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "wand.and.stars",
                        title: "No skills installed. Create your first skill.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Skills") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.skills.enumerated()), id: \.element.id) { index, skill in
                                GaryxSkillCard(skill: skill)
                                if index < model.skills.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Skill") {
                showsCreateSkill = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateSkill) {
            GaryxFormSheet(title: "New Skill") {
                GaryxCreateSkillCard()
            }
        }
        .fullScreenCover(isPresented: skillEditorPresented) {
            GaryxFormSheet(title: "Skill Editor", onDone: requestCloseSkillEditor) {
                GaryxSkillEditorCard()
            }
            .interactiveDismissDisabled(skillEditorHasUnsavedChanges)
            .confirmationDialog(
                "Discard unsaved skill changes?",
                isPresented: $showsDiscardSkillEditorConfirmation,
                titleVisibility: .visible
            ) {
                Button("Discard", role: .destructive) {
                    closeSkillEditor()
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Your current file edits have not been saved.")
            }
        }
    }

    private var skillEditorHasUnsavedChanges: Bool {
        guard let document = model.selectedSkillDocument, document.editable else { return false }
        return model.selectedSkillFileContent != document.content
    }

    private func requestCloseSkillEditor() {
        if skillEditorHasUnsavedChanges {
            showsDiscardSkillEditorConfirmation = true
        } else {
            closeSkillEditor()
        }
    }

    private func closeSkillEditor() {
        model.selectedSkillEditor = nil
        model.selectedSkillDocument = nil
        model.selectedSkillFileContent = ""
    }
}

struct GaryxCreateSkillCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("New Skill")
            TextField("ID", text: $model.draftSkillId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Name", text: $model.draftSkillName)
                .garyxInputStyle()
            TextField("Description", text: $model.draftSkillDescription, axis: .vertical)
                .lineLimit(2...4)
                .garyxInputStyle()
            TextField("Body", text: $model.draftSkillBody, axis: .vertical)
                .lineLimit(2...5)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createSkillFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Create Skill", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxSkillCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let skill: GaryxSkillSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var name = ""
    @State private var description = ""

    var body: some View {
        GaryxSwipeActionRow(actions: skillSwipeActions) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "wand.and.stars")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(skill.name)
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(skill.description.isEmpty ? skill.sourcePath.garyxLastPathComponent : skill.description)
                            .font(GaryxFont.caption(weight: .medium))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                    Spacer()
                    GaryxStatusPill(text: skill.enabled ? "Enabled" : "Paused", tone: skill.enabled ? .good : .muted)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Skill") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Skill")
                    TextField("Name", text: $name)
                        .garyxInputStyle()
                    TextField("Description", text: $description, axis: .vertical)
                        .lineLimit(2...4)
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateSkill(skill, name: name, description: description)
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
        .confirmationDialog("Delete skill?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSkill(skill) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the skill directory.")
        }
    }

    private var skillSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "Open", systemImage: "doc.text", tone: .accent) {
                Task { await model.openSkillEditor(skill) }
            },
            GaryxSwipeAction(title: skill.enabled ? "Disable" : "Enable", systemImage: skill.enabled ? "pause.fill" : "play.fill") {
                Task { await model.toggleSkill(skill) }
            },
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }

    private func fillDraft() {
        name = skill.name
        description = skill.description
    }
}

struct GaryxSkillEditorCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsDiscardFileSwitchConfirmation = false
    @State private var pendingFileSkillId = ""
    @State private var pendingFilePath = ""

    var body: some View {
        if let editor = model.selectedSkillEditor {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    GaryxFieldLabel("Skill Editor")
                    Spacer()
                    Text(editor.skill.name)
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.secondary)
                }

                ForEach(editor.entries) { node in
                    GaryxSkillEntryRow(skillId: editor.skill.id, node: node, depth: 0) { path in
                        requestOpenSkillFile(skillId: editor.skill.id, path: path)
                    }
                }

                HStack(spacing: 8) {
                    TextField("path/to/file.md", text: $model.draftSkillEntryPath)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    Picker("Type", selection: $model.draftSkillEntryType) {
                        Text("New File").tag("file")
                        Text("New Folder").tag("directory")
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 148)
                }
                Button {
                    Task { await model.createSkillEntry() }
                } label: {
                    Label("Create", systemImage: "plus")
                }
                .buttonStyle(GaryxSecondaryButtonStyle())

                if let document = model.selectedSkillDocument {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(document.path)
                            .font(GaryxFont.caption(weight: .semibold))
                            .foregroundStyle(.secondary)
                        TextField("Content", text: $model.selectedSkillFileContent, axis: .vertical)
                            .lineLimit(6...16)
                            .garyxInputStyle()
                            .disabled(!document.editable)
                        Button {
                            Task { await model.saveSelectedSkillFile() }
                        } label: {
                            Label("Save", systemImage: "square.and.arrow.down")
                        }
                        .buttonStyle(GaryxPrimaryCompactButtonStyle())
                        .disabled(!document.editable)
                    }
                }
            }
            .garyxCardStyle()
            .confirmationDialog(
                "Discard unsaved skill changes?",
                isPresented: $showsDiscardFileSwitchConfirmation,
                titleVisibility: .visible
            ) {
                Button("Discard", role: .destructive) {
                    openPendingSkillFile()
                }
                Button("Cancel", role: .cancel) {
                    clearPendingSkillFile()
                }
            } message: {
                Text("Your current file edits have not been saved.")
            }
        }
    }

    private var skillEditorHasUnsavedChanges: Bool {
        guard let document = model.selectedSkillDocument, document.editable else { return false }
        return model.selectedSkillFileContent != document.content
    }

    private func requestOpenSkillFile(skillId: String, path: String) {
        if model.selectedSkillDocument?.path == path {
            return
        }
        if skillEditorHasUnsavedChanges {
            pendingFileSkillId = skillId
            pendingFilePath = path
            showsDiscardFileSwitchConfirmation = true
        } else {
            Task { await model.openSkillFile(skillId: skillId, path: path) }
        }
    }

    private func openPendingSkillFile() {
        let skillId = pendingFileSkillId
        let path = pendingFilePath
        clearPendingSkillFile()
        guard !skillId.isEmpty, !path.isEmpty else { return }
        Task { await model.openSkillFile(skillId: skillId, path: path) }
    }

    private func clearPendingSkillFile() {
        pendingFileSkillId = ""
        pendingFilePath = ""
    }
}

struct GaryxSkillEntryRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let skillId: String
    let node: GaryxSkillEntryNode
    let depth: Int
    let onOpenFile: (String) -> Void
    @State private var showsDeleteConfirmation = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: node.entryType == "directory" ? "folder.fill" : "doc.text")
                    .frame(width: 18)
                Button {
                    if node.entryType == "file" {
                        onOpenFile(node.path)
                    }
                } label: {
                    Text(node.name)
                        .font(GaryxFont.callout(weight: .medium))
                        .lineLimit(1)
                }
                .buttonStyle(.plain)
                Spacer(minLength: 0)
                Button(role: .destructive) {
                    showsDeleteConfirmation = true
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(GaryxMiniIconButtonStyle())
            }
            .padding(.leading, CGFloat(depth) * 14)

            ForEach(node.children) { child in
                GaryxSkillEntryRow(skillId: skillId, node: child, depth: depth + 1, onOpenFile: onOpenFile)
            }
        }
        .confirmationDialog("Delete skill entry?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSkillEntry(skillId: skillId, path: node.path) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(node.path)
        }
    }
}

struct GaryxCommandsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateCommand = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Commands",
            subtitle: "\(model.slashCommands.count) shortcuts",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            GaryxCommandsContent()
        } actions: {
            GaryxAddToolbarButton(label: "Add Command") {
                showsCreateCommand = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateCommand) {
            GaryxFormSheet(title: "Add Command") {
                GaryxCreateSlashCommandCard()
            }
        }
    }
}

struct GaryxCommandsContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if model.slashCommands.isEmpty {
                GaryxEmptyPanelView(
                    icon: "command",
                    title: "No shortcuts yet",
                    text: ""
                )
            } else {
                GaryxSectionBlock(title: "Slash Commands") {
                    GaryxCompactListGroup {
                        ForEach(Array(model.slashCommands.enumerated()), id: \.element.id) { index, command in
                            GaryxSlashCommandCard(command: command)
                            if index < model.slashCommands.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

struct GaryxCreateSlashCommandCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Add Command")
            TextField("Command name", text: $model.draftSlashName)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Description", text: $model.draftSlashDescription)
                .garyxInputStyle()
            TextField("Content", text: $model.draftSlashPrompt, axis: .vertical)
                .lineLimit(2...5)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createSlashCommandFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Save Command", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxSlashCommandCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let command: GaryxSlashCommand
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var name = ""
    @State private var description = ""
    @State private var prompt = ""

    var body: some View {
        GaryxSwipeActionRow(actions: commandSwipeActions) {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "command")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)

                    VStack(alignment: .leading, spacing: 3) {
                        Text("/\(command.name)")
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(command.description.isEmpty ? command.prompt : command.description)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                    Spacer(minLength: 8)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear {
            name = command.name
            description = command.description
            prompt = command.prompt
        }
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Command") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Command")
                    TextField("name", text: $name)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Description", text: $description)
                        .garyxInputStyle()
                    TextField("Prompt", text: $prompt, axis: .vertical)
                        .lineLimit(2...6)
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateSlashCommand(
                                command,
                                name: name,
                                description: description,
                                prompt: prompt
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
        .confirmationDialog("Delete command?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteSlashCommand(command) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the slash command.")
        }
    }

    private var commandSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "Edit", systemImage: "pencil", tone: .accent) {
                name = command.name
                description = command.description
                prompt = command.prompt
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }
}

struct GaryxMcpServersView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateMcp = false

    var body: some View {
        GaryxPanelScaffold(
            title: "MCP",
            subtitle: "\(model.mcpServers.filter(\.enabled).count) enabled / \(model.mcpServers.count) servers",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            GaryxMcpServersContent()
        } actions: {
            GaryxAddToolbarButton(label: "Add Server") {
                showsCreateMcp = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateMcp) {
            GaryxFormSheet(title: "Add Server") {
                GaryxCreateMcpServerCard()
            }
        }
    }
}

struct GaryxMcpServersContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if model.mcpServers.isEmpty {
                GaryxEmptyPanelView(
                    icon: "point.3.connected.trianglepath.dotted",
                    title: "No MCP servers yet",
                    text: ""
                )
            } else {
                GaryxSectionBlock(title: "MCP Servers") {
                    GaryxCompactListGroup {
                        ForEach(Array(model.mcpServers.enumerated()), id: \.element.id) { index, server in
                            GaryxMcpServerCard(server: server)
                            if index < model.mcpServers.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

struct GaryxCreateMcpServerCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Add Server")
            TextField("Name", text: $model.draftMcpName)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Start command", text: $model.draftMcpCommand)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Arguments", text: $model.draftMcpArgs)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Environment variables", text: $model.draftMcpEnv, axis: .vertical)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(2...4)
                .garyxInputStyle()
            TextField("Working directory", text: $model.draftMcpWorkingDir)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("URL", text: $model.draftMcpUrl)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .garyxInputStyle()
            TextField("Headers", text: $model.draftMcpHeaders, axis: .vertical)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(2...4)
                .garyxInputStyle()
            Button {
                Task {
                    if await model.createMcpServerFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Save", systemImage: "plus")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
        }
        .garyxCardStyle()
    }
}

struct GaryxMcpServerCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let server: GaryxMcpServer
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var name = ""
    @State private var command = ""
    @State private var args = ""
    @State private var env = ""
    @State private var workingDir = ""
    @State private var url = ""
    @State private var headers = ""

    var body: some View {
        GaryxSwipeActionRow(actions: serverSwipeActions) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    Image(systemName: "point.3.connected.trianglepath.dotted")
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 24, height: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(server.name)
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(server.transport == "streamable_http" ? server.url ?? "HTTP" : server.command)
                            .font(GaryxFont.caption(weight: .medium))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer()
                    GaryxStatusPill(text: server.enabled ? "Enabled" : "Paused", tone: server.enabled ? .good : .muted)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit MCP Server") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("MCP Server")
                    TextField("Name", text: $name)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Start command", text: $command)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Arguments", text: $args)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Environment variables", text: $env, axis: .vertical)
                        .lineLimit(2...4)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Working directory", text: $workingDir)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("URL", text: $url)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    TextField("Headers", text: $headers, axis: .vertical)
                        .lineLimit(2...4)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    Button {
                        Task {
                            await model.updateMcpServer(
                                server,
                                name: name,
                                command: command,
                                argsText: args,
                                envText: env,
                                workingDir: workingDir,
                                url: url,
                                headersText: headers
                            )
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                }
                .garyxCardStyle()
            }
        }
        .confirmationDialog("Delete MCP server?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteMcpServer(server) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(server.name)
        }
    }

    private var serverSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: server.enabled ? "Disable" : "Enable", systemImage: server.enabled ? "pause.fill" : "play.fill", tone: .accent) {
                Task { await model.toggleMcpServer(server) }
            },
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }

    private func fillDraft() {
        name = server.name
        command = server.command
        args = server.args.joined(separator: ", ")
        env = server.env.map { "\($0.key)=\($0.value)" }.sorted().joined(separator: "\n")
        workingDir = server.workingDir ?? ""
        url = server.url ?? ""
        headers = server.headers.map { "\($0.key)=\($0.value)" }.sorted().joined(separator: "\n")
    }
}

struct GaryxAutoResearchView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateRun = false
    @State private var detailRun: GaryxAutoResearchRun?

    var body: some View {
        GaryxPanelScaffold(
            title: "Auto Research",
            subtitle: "\(model.runningResearchCount) active / \(model.autoResearchRuns.count) total",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                if model.autoResearchRuns.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "atom",
                        title: "No Auto Research runs",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Auto Research") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.autoResearchRuns.enumerated()), id: \.element.id) { index, run in
                                GaryxAutoResearchRunCard(run: run) {
                                    detailRun = run
                                    Task { await model.loadAutoResearchDetail(run) }
                                }
                                if index < model.autoResearchRuns.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Auto Research Run") {
                showsCreateRun = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateRun) {
            GaryxFormSheet(title: "Create Auto Research Run") {
                GaryxCreateAutoResearchCard()
            }
        }
        .sheet(item: $detailRun) { run in
            GaryxAutoResearchDetailSheet(run: run)
        }
    }
}

struct GaryxCreateAutoResearchCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFieldLabel("Create Auto Research Run")
            TextField("Goal", text: $model.draftAutoResearchGoal, axis: .vertical)
                .lineLimit(2...5)
                .garyxInputStyle()
            if workspacePaths.isEmpty {
                Text("No workspaces available")
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
            } else {
                Picker("Workspace", selection: workspaceSelection) {
                    ForEach(workspacePaths, id: \.self) { path in
                        Text(path.garyxLastPathComponent).tag(path)
                    }
                }
                .pickerStyle(.menu)
                .garyxInputStyle()
            }
            HStack {
                TextField("Iterations", text: $model.draftAutoResearchIterations)
                    .keyboardType(.numberPad)
                    .garyxInputStyle()
                TextField("Budget min", text: $model.draftAutoResearchTimeBudgetMinutes)
                    .keyboardType(.numberPad)
                    .garyxInputStyle()
            }
            HStack {
                Spacer(minLength: 0)
                Button {
                    Task {
                        if await model.createAutoResearchRunFromDraft() {
                            dismiss()
                        }
                    }
                } label: {
                    Label("Start", systemImage: "play.fill")
                }
                .buttonStyle(GaryxPrimaryCompactButtonStyle())
                .disabled(!canStart)
            }
        }
        .garyxCardStyle()
        .onAppear(perform: ensureWorkspaceSelection)
    }

    private var workspacePaths: [String] {
        model.knownWorkspacePaths
    }

    private var workspaceSelection: Binding<String> {
        Binding {
            effectiveWorkspacePath
        } set: { value in
            model.selectedWorkspacePath = value
        }
    }

    private var effectiveWorkspacePath: String {
        let selected = model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, workspacePaths.contains(selected) {
            return selected
        }
        return workspacePaths.first ?? ""
    }

    private var canStart: Bool {
        !model.draftAutoResearchGoal.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !effectiveWorkspacePath.isEmpty
            && positiveInteger(model.draftAutoResearchIterations) != nil
            && positiveAutoResearchBudgetMinutes(model.draftAutoResearchTimeBudgetMinutes) != nil
    }

    private func ensureWorkspaceSelection() {
        let nextSelection = effectiveWorkspacePath
        if model.selectedWorkspacePath != nextSelection {
            model.selectedWorkspacePath = nextSelection
        }
    }

    private func positiveInteger(_ value: String) -> Int? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = Int(trimmed), parsed > 0 else { return nil }
        return parsed
    }

    private func positiveAutoResearchBudgetMinutes(_ value: String) -> Int? {
        guard let parsed = positiveInteger(value), parsed <= Int.max / 60 else { return nil }
        return parsed
    }
}

struct GaryxAutoResearchRunCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let run: GaryxAutoResearchRun
    let onOpenDetail: () -> Void
    @State private var showsDeleteConfirmation = false

    var body: some View {
        GaryxSwipeActionRow(actions: researchSwipeActions) {
            Button(action: onOpenDetail) {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(alignment: .center, spacing: 10) {
                        Image(systemName: "atom")
                            .font(GaryxFont.system(size: 15, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: 24, height: 24)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(run.goal.isEmpty ? run.runId : run.goal)
                                .font(GaryxFont.body(weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(2)
                            Text(run.workspaceDir?.garyxLastPathComponent ?? run.runId)
                                .font(GaryxFont.caption(weight: .medium))
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        GaryxStatusPill(text: garyxAutoResearchStateLabel(run.state), tone: researchTone)
                    }
                    Text("\(run.iterationsUsed) of \(run.maxIterations) iterations")
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 11)
            }
            .buttonStyle(.plain)
            .accessibilityHint("Open Auto Research details")
        }
        .confirmationDialog("Delete Auto Research run?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteAutoResearchRun(run) }
            }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("This removes the run, iterations, and candidates.")
        }
    }

    private var researchSwipeActions: [GaryxSwipeAction] {
        var actions: [GaryxSwipeAction] = []
        if !garyxAutoResearchIsTerminal(run.state) {
            actions.append(
                GaryxSwipeAction(title: "Stop", systemImage: "stop.fill", tone: .warning) {
                    Task { await model.stopAutoResearchRun(run) }
                }
            )
        }
        actions.append(
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        )
        return actions
    }

    private var researchTone: GaryxStatusPill.Tone {
        garyxAutoResearchTone(run)
    }
}

struct GaryxAutoResearchDetailSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let run: GaryxAutoResearchRun
    @State private var feedbackCandidate: GaryxResearchCandidate?
    @State private var feedbackDraft = ""

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    summaryBlock
                    iterationBlock
                    if orphanCandidates.count > 0 {
                        candidateBlock
                    }
                }
                .padding(12)
                .frame(maxWidth: 620, alignment: .leading)
                .frame(maxWidth: .infinity)
            }
            .background(GaryxTheme.background)
            .refreshable {
                await model.loadAutoResearchDetail(runId: run.runId)
            }
            .navigationTitle("Auto Research")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Done") {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    if let activeThreadId {
                        Button {
                            openThread(activeThreadId)
                        } label: {
                            Label("Open", systemImage: "arrow.up.right")
                        }
                    }
                }
            }
        }
        .task {
            await model.loadAutoResearchDetail(runId: run.runId)
        }
        .sheet(item: $feedbackCandidate, onDismiss: {
            feedbackDraft = ""
        }) { candidate in
            GaryxAutoResearchFeedbackSheet(candidate: candidate, feedback: $feedbackDraft) { feedback in
                let current = currentRun
                feedbackCandidate = nil
                feedbackDraft = ""
                Task {
                    await model.sendAutoResearchFeedback(
                        run: current,
                        candidate: candidate,
                        feedback: feedback
                    )
                }
            }
        }
    }

    private var currentRun: GaryxAutoResearchRun {
        model.autoResearchDetailsByRunId[run.runId]?.run
            ?? model.autoResearchRuns.first { $0.runId == run.runId }
            ?? run
    }

    private var detail: GaryxAutoResearchDetail? {
        model.autoResearchDetailsByRunId[run.runId]
    }

    private var candidatesPage: GaryxAutoResearchCandidatesPage? {
        model.researchCandidatesByRunId[run.runId]
    }

    private var candidates: [GaryxResearchCandidate] {
        candidatesPage?.candidates ?? []
    }

    private var candidatesByIteration: [Int: GaryxResearchCandidate] {
        var result: [Int: GaryxResearchCandidate] = [:]
        for candidate in candidates {
            result[candidate.iteration] = candidate
        }
        return result
    }

    private var displayIterations: [GaryxAutoResearchIteration] {
        var items = model.autoResearchIterationsByRunId[run.runId] ?? []
        if let latest = detail?.latestIteration,
           !items.contains(where: { $0.iterationIndex == latest.iterationIndex }) {
            items.append(latest)
        }
        return items.sorted { $0.iterationIndex < $1.iterationIndex }
    }

    private var orphanCandidates: [GaryxResearchCandidate] {
        let iterationIds = Set(displayIterations.map(\.iterationIndex))
        return candidates
            .filter { !iterationIds.contains($0.iteration) }
            .sorted { $0.iteration > $1.iteration }
    }

    private var summaryBlock: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "atom")
                    .font(GaryxFont.system(size: 16, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)
                    .background(Color(.secondarySystemGroupedBackground), in: Circle())
                VStack(alignment: .leading, spacing: 5) {
                    Text(currentRun.goal.isEmpty ? currentRun.runId : currentRun.goal)
                        .font(GaryxFont.body(weight: .semibold))
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(summarySubtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Spacer(minLength: 0)
                GaryxStatusPill(text: garyxAutoResearchStateLabel(currentRun.state), tone: garyxAutoResearchTone(currentRun))
            }
            if let terminalReason {
                Text(terminalReason)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            HStack(spacing: 8) {
                GaryxAutoResearchMetricPill(
                    title: "Iterations",
                    value: "\(currentRun.iterationsUsed) of \(currentRun.maxIterations)"
                )
                if let selectedCandidate {
                    GaryxAutoResearchMetricPill(
                        title: "Winner",
                        value: candidateMetricValue(selectedCandidate)
                    )
                } else if let bestCandidate {
                    GaryxAutoResearchMetricPill(
                        title: "Best",
                        value: candidateMetricValue(bestCandidate)
                    )
                }
                Spacer(minLength: 0)
            }
            if let activeThreadId {
                Button {
                    openThread(activeThreadId)
                } label: {
                    Label("Open Active Thread", systemImage: "arrow.up.right")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(GaryxSecondaryButtonStyle())
            }
        }
        .padding(12)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }

    private var iterationBlock: some View {
        GaryxSectionBlock(title: "Iterations") {
            if displayIterations.isEmpty {
                Text("No iteration records yet.")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 4)
            } else {
                GaryxCompactListGroup {
                    ForEach(Array(displayIterations.enumerated()), id: \.element.id) { index, iteration in
                        let candidate = candidatesByIteration[iteration.iterationIndex]
                        GaryxResearchIterationRow(
                            iteration: iteration,
                            candidate: candidate,
                            isBest: candidate?.candidateId == candidatesPage?.bestCandidateId,
                            isSelected: candidate?.candidateId == currentRun.selectedCandidate,
                            isRunTerminal: garyxAutoResearchIsTerminal(currentRun.state),
                            onSelect: { candidate in
                                Task { await model.selectAutoResearchCandidate(run: currentRun, candidate: candidate) }
                            },
                            onReverify: { candidate in
                                Task { await model.reverifyAutoResearchCandidate(run: currentRun, candidate: candidate) }
                            },
                            onFeedback: openFeedback,
                            onOpenThread: openThread
                        )
                        if index < displayIterations.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                }
            }
        }
    }

    private var candidateBlock: some View {
        GaryxSectionBlock(title: "Candidates") {
            GaryxCompactListGroup {
                ForEach(Array(orphanCandidates.enumerated()), id: \.element.id) { index, candidate in
                    GaryxResearchCandidateRow(
                        candidate: candidate,
                        isBest: candidate.candidateId == candidatesPage?.bestCandidateId,
                        isSelected: candidate.candidateId == currentRun.selectedCandidate,
                        isRunTerminal: garyxAutoResearchIsTerminal(currentRun.state),
                        onSelect: {
                            Task { await model.selectAutoResearchCandidate(run: currentRun, candidate: candidate) }
                        },
                        onReverify: {
                            Task { await model.reverifyAutoResearchCandidate(run: currentRun, candidate: candidate) }
                        },
                        onFeedback: {
                            openFeedback(candidate)
                        }
                    )
                    if index < orphanCandidates.count - 1 {
                        GaryxCompactRowDivider()
                    }
                }
            }
        }
    }

    private var summarySubtitle: String {
        let workspace = currentRun.workspaceDir?.garyxLastPathComponent ?? "No workspace"
        let updated = garyxFormattedTaskTimestamp(currentRun.updatedAt)
        return updated.isEmpty ? workspace : "\(workspace) · updated \(updated)"
    }

    private var activeThreadId: String? {
        let value = detail?.activeThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    private var terminalReason: String? {
        let value = currentRun.terminalReason?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : garyxAutoResearchReasonLabel(value)
    }

    private var selectedCandidate: GaryxResearchCandidate? {
        let selectedId = currentRun.selectedCandidate?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !selectedId.isEmpty else { return nil }
        return candidates.first { $0.candidateId == selectedId }
    }

    private var bestCandidate: GaryxResearchCandidate? {
        let bestId = candidatesPage?.bestCandidateId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !bestId.isEmpty else { return nil }
        return candidates.first { $0.candidateId == bestId }
    }

    private func candidateMetricValue(_ candidate: GaryxResearchCandidate) -> String {
        if let score = candidate.verdict?.score {
            return String(format: "%.1f/10", score)
        }
        return "Candidate \(candidate.iteration)"
    }

    private func openFeedback(_ candidate: GaryxResearchCandidate) {
        feedbackCandidate = candidate
        feedbackDraft = ""
    }

    private func openThread(_ threadId: String?) {
        let threadId = threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !threadId.isEmpty else { return }
        dismiss()
        Task { await model.openThread(id: threadId) }
    }
}

struct GaryxAutoResearchMetricPill: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
            Text(value)
                .font(GaryxFont.footnote(weight: .semibold))
                .foregroundStyle(.primary)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

struct GaryxAutoResearchFeedbackSheet: View {
    @Environment(\.dismiss) private var dismiss
    let candidate: GaryxResearchCandidate
    @Binding var feedback: String
    let onSend: (String) -> Void

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 12) {
                TextEditor(text: $feedback)
                    .font(GaryxFont.body())
                    .scrollContentBackground(.hidden)
                    .padding(8)
                    .frame(minHeight: 160)
                    .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .stroke(GaryxTheme.hairline, lineWidth: 1)
                    }
                Text("\(feedback.trimmingCharacters(in: .whitespacesAndNewlines).count) characters")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
            .padding(16)
            .background(GaryxTheme.background)
            .navigationTitle("Feedback on Candidate \(candidate.iteration)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Cancel") {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Send") {
                        let value = feedback.trimmingCharacters(in: .whitespacesAndNewlines)
                        onSend(value)
                        dismiss()
                    }
                    .disabled(feedback.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}

struct GaryxResearchIterationRow: View {
    let iteration: GaryxAutoResearchIteration
    let candidate: GaryxResearchCandidate?
    let isBest: Bool
    let isSelected: Bool
    let isRunTerminal: Bool
    let onSelect: (GaryxResearchCandidate) -> Void
    let onReverify: (GaryxResearchCandidate) -> Void
    let onFeedback: (GaryxResearchCandidate) -> Void
    let onOpenThread: (String?) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 8) {
                Text("Iteration \(iteration.iterationIndex)")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                GaryxStatusPill(
                    text: garyxAutoResearchStateLabel(iteration.state.isEmpty ? "pending" : iteration.state),
                    tone: garyxAutoResearchTone(iteration.state)
                )
                if isSelected {
                    GaryxStatusPill(text: "Winner", tone: .good)
                } else if isBest {
                    GaryxStatusPill(text: "Current best", tone: .good)
                }
                Spacer(minLength: 0)
            }
            if let candidate {
                GaryxResearchCandidateContent(candidate: candidate)
                GaryxResearchCandidateActions(
                    candidate: candidate,
                    isSelected: isSelected,
                    isRunTerminal: isRunTerminal,
                    onSelect: { onSelect(candidate) },
                    onReverify: { onReverify(candidate) },
                    onFeedback: { onFeedback(candidate) }
                )
            } else {
                Text(iteration.state.lowercased() == "completed" ? "No candidate recorded for this iteration." : "This iteration is still running.")
                    .font(GaryxFont.footnote())
                    .foregroundStyle(.secondary)
            }
            if hasThreadLinks {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) {
                        threadLinkControls
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        threadLinkControls
                    }
                }
            }
        }
        .padding(10)
    }

    private var workThreadId: String? {
        let value = iteration.workThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    private var verifyThreadId: String? {
        let value = iteration.verifyThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    private var hasThreadLinks: Bool {
        workThreadId != nil || verifyThreadId != nil
    }

    @ViewBuilder
    private var threadLinkControls: some View {
        if let workThreadId {
            Button {
                onOpenThread(workThreadId)
            } label: {
                Label("Work", systemImage: "doc.text")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
        if let verifyThreadId {
            Button {
                onOpenThread(verifyThreadId)
            } label: {
                Label("Verify", systemImage: "checkmark.seal")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
    }
}

struct GaryxResearchCandidateRow: View {
    let candidate: GaryxResearchCandidate
    let isBest: Bool
    let isSelected: Bool
    let isRunTerminal: Bool
    let onSelect: () -> Void
    let onReverify: () -> Void
    let onFeedback: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack {
                Text("Candidate \(candidate.iteration)")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                if isSelected {
                    GaryxStatusPill(text: "Winner", tone: .good)
                } else if isBest {
                    GaryxStatusPill(text: "Current best", tone: .good)
                }
                Spacer(minLength: 0)
            }
            GaryxResearchCandidateContent(candidate: candidate)
            GaryxResearchCandidateActions(
                candidate: candidate,
                isSelected: isSelected,
                isRunTerminal: isRunTerminal,
                onSelect: onSelect,
                onReverify: onReverify,
                onFeedback: onFeedback
            )
        }
        .padding(10)
    }
}

struct GaryxResearchCandidateContent: View {
    let candidate: GaryxResearchCandidate

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(candidate.output.isEmpty ? "No candidate output yet." : candidate.output)
                .font(GaryxFont.footnote())
                .foregroundStyle(.secondary)
                .lineLimit(8)
            if let verdict = candidate.verdict {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Score \(String(format: "%.1f", verdict.score))/10")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.primary)
                    if !verdict.feedback.isEmpty {
                        Text(verdict.feedback)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(3)
                    }
                }
            }
        }
    }
}

struct GaryxResearchCandidateActions: View {
    let candidate: GaryxResearchCandidate
    let isSelected: Bool
    let isRunTerminal: Bool
    let onSelect: () -> Void
    let onReverify: () -> Void
    let onFeedback: () -> Void

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                controls
            }
            VStack(alignment: .leading, spacing: 8) {
                controls
            }
        }
    }

    @ViewBuilder
    private var controls: some View {
        if isSelected {
            GaryxStatusPill(text: "Selected Winner", tone: .good)
                .fixedSize(horizontal: true, vertical: false)
        } else {
            Button {
                onSelect()
            } label: {
                Label("Select", systemImage: "checkmark")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
        if !isRunTerminal {
            Button {
                onReverify()
            } label: {
                Label("Reverify", systemImage: "arrow.clockwise")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
            Button {
                onFeedback()
            } label: {
                Label("Feedback", systemImage: "text.bubble")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
    }
}

func garyxAutoResearchIsTerminal(_ state: String) -> Bool {
    switch state.lowercased() {
    case "user_stopped", "budget_exhausted", "blocked":
        true
    default:
        false
    }
}

func garyxAutoResearchStateLabel(_ state: String) -> String {
    switch state.lowercased() {
    case "queued":
        "Queued"
    case "researching":
        "Researching"
    case "judging":
        "Judging"
    case "budget_exhausted":
        "Budget exhausted"
    case "blocked":
        "Blocked"
    case "user_stopped":
        "Stopped"
    case "completed":
        "Completed"
    case "pending":
        "Pending"
    default:
        state
            .split(separator: "_")
            .map { word in
                word.prefix(1).uppercased() + String(word.dropFirst())
            }
            .joined(separator: " ")
    }
}

func garyxAutoResearchReasonLabel(_ reason: String) -> String {
    let normalized = reason.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !normalized.isEmpty else { return "" }
    switch normalized.lowercased() {
    case "user_requested", "user_stopped":
        return "Stopped by user"
    case "time_budget_exhausted":
        return "Time budget exhausted"
    case "budget_exhausted":
        return "Budget exhausted"
    case "blocked":
        return "Blocked"
    default:
        return normalized
            .split(separator: "_")
            .map { word in
                word.prefix(1).uppercased() + String(word.dropFirst())
            }
            .joined(separator: " ")
    }
}

func garyxAutoResearchTone(_ run: GaryxAutoResearchRun) -> GaryxStatusPill.Tone {
    let selected = run.selectedCandidate?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    if !selected.isEmpty {
        return .good
    }
    return garyxAutoResearchTone(run.state)
}

func garyxAutoResearchTone(_ state: String) -> GaryxStatusPill.Tone {
    switch state.lowercased() {
    case "completed":
        .good
    case "blocked":
        .danger
    case "user_stopped", "budget_exhausted":
        .muted
    default:
        .warning
    }
}

struct GaryxMobileSettingsPanel: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsGatewaySetup = false
    @State private var showsCreateBot = false
    @State private var showsCreateCommand = false
    @State private var showsCreateMcp = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Settings",
            subtitle: model.activeSettingsTab.label,
            onRefresh: { await model.connectAndRefresh() },
            leadingActionLabel: settingsLeadingActionLabel,
            leadingActionSystemName: "chevron.left",
            leadingAction: settingsLeadingAction,
            background: GaryxTheme.background
        ) {
            VStack(alignment: .leading, spacing: 12) {
                GaryxSettingsTabContent()
            }
        } actions: {
            HStack(spacing: 8) {
                switch model.activeSettingsTab {
                case .gateway:
                    GaryxAddToolbarButton(label: "Add Gateway") {
                        model.gatewaySettingsStatus = nil
                        model.lastError = nil
                        showsGatewaySetup = true
                    }
                case .commands:
                    GaryxAddToolbarButton(label: "Add Command") {
                        showsCreateCommand = true
                    }
                case .mcp:
                    GaryxAddToolbarButton(label: "Add Server") {
                        showsCreateMcp = true
                    }
                case .channels:
                    GaryxAddToolbarButton(label: "Add Bot") {
                        showsCreateBot = true
                    }
                case .manage, .provider:
                    EmptyView()
                }
            }
        }
        .fullScreenCover(isPresented: $showsGatewaySetup) {
            GaryxGatewaySetupView(isSheet: true, startsEmpty: true)
        }
        .fullScreenCover(isPresented: $showsCreateBot) {
            GaryxFormSheet(title: "Add Bot") {
                GaryxBotAccountForm(account: nil)
            }
        }
        .fullScreenCover(isPresented: $showsCreateCommand) {
            GaryxFormSheet(title: "Add Command") {
                GaryxCreateSlashCommandCard()
            }
        }
        .fullScreenCover(isPresented: $showsCreateMcp) {
            GaryxFormSheet(title: "Add Server") {
                GaryxCreateMcpServerCard()
            }
        }
    }

    private var settingsLeadingActionLabel: String? {
        model.activeSettingsTab == .manage ? nil : "All Settings"
    }

    private var settingsLeadingAction: (() -> Void)? {
        guard model.activeSettingsTab != .manage else { return nil }
        return {
            model.showSettingsOverview()
        }
    }
}

struct GaryxSettingsTabContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        switch model.activeSettingsTab {
        case .manage:
            GaryxSettingsOverviewContent()
        case .gateway:
            GaryxSettingsDetailContent {
                GaryxSettingsGatewayContent()
            }
        case .provider:
            GaryxSettingsDetailContent {
                GaryxSettingsProviderContent()
            }
        case .channels:
            GaryxSettingsDetailContent {
                GaryxBotsContent()
            }
        case .commands:
            GaryxSettingsDetailContent {
                GaryxCommandsContent()
            }
        case .mcp:
            GaryxSettingsDetailContent {
                GaryxMcpServersContent()
            }
        }
    }
}

struct GaryxSettingsOverviewContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    private var managementPanels: [GaryxMobilePanel] {
        [
            model.dreamsAutoScanEnabled ? .dreams : nil,
            .tasks,
            .autoResearch,
            .agents,
            .skills,
        ].compactMap { $0 }
    }
    private let settingsTabs: [GaryxMobileSettingsTab] = [
        .gateway,
        .provider,
        .channels,
        .commands,
        .mcp,
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            GaryxSettingsOverviewSection(title: "Manage") {
                ForEach(Array(managementPanels.enumerated()), id: \.element.id) { index, panel in
                    GaryxSettingsPanelLinkRow(panel: panel)
                    if index < managementPanels.count - 1 {
                        Divider()
                            .padding(.leading, 54)
                    }
                }
            }

            GaryxSettingsOverviewSection(title: "Settings") {
                GaryxDreamsAutoScanRow()
                Divider()
                    .padding(.leading, 54)

                ForEach(Array(settingsTabs.enumerated()), id: \.element.id) { index, tab in
                    GaryxSettingsTabLinkRow(tab: tab)
                    if index < settingsTabs.count - 1 {
                        Divider()
                            .padding(.leading, 54)
                    }
                }
            }
        }
    }
}

struct GaryxSettingsOverviewSection<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 16)

            VStack(spacing: 0) {
                content
            }
            .background(GaryxTheme.surface)
        }
    }
}

struct GaryxSettingsDetailContent<Content: View>: View {
    let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            content
        }
    }
}

struct GaryxSettingsPanelLinkRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let panel: GaryxMobilePanel

    var body: some View {
        Button {
            model.openPanel(panel)
        } label: {
            HStack(spacing: 10) {
                Image(systemName: panel.iconName)
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(panel.label)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(panel.label)
    }

    private var subtitle: String {
        switch panel {
        case .workspaces:
            "\(model.knownWorkspacePaths.count) workspaces"
        case .dreams:
            "\(model.dreams.count) topics"
        case .tasks:
            "\(model.activeTaskCount) active / \(model.tasks.count) total"
        case .autoResearch:
            "\(model.runningResearchCount) active / \(model.autoResearchRuns.count) total"
        case .workspaceBots:
            "\(model.mobileBotGroups.count) bots / \(visibleWorkspaceCount) workspaces"
        case .agents:
            "\(model.agents.count) agents / \(model.teams.count) teams"
        case .skills:
            "\(model.skills.filter(\.enabled).count) enabled / \(model.skills.count) total"
        default:
            ""
        }
    }

    private var visibleWorkspaceCount: Int {
        model.knownWorkspacePaths
            .filter(GaryxMobileModel.isVisibleMobileWorkspacePath)
            .count
    }
}

struct GaryxSettingsTabLinkRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let tab: GaryxMobileSettingsTab

    var body: some View {
        Button {
            model.activeSettingsTab = tab
        } label: {
            HStack(spacing: 10) {
                Image(systemName: tab.iconName)
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)

                VStack(alignment: .leading, spacing: 2) {
                    Text(tab.label)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(tab.label)
    }

    private var subtitle: String {
        switch tab {
        case .manage:
            "All mobile settings"
        case .gateway:
            model.gatewayURL.isEmpty ? "Connection and saved gateways" : model.gatewayURL
        case .provider:
            model.providerModelsByType.isEmpty ? "Model providers" : "\(model.providerModelsByType.count) provider types"
        case .channels:
            "\(model.configuredBots.count) configured bots"
        case .commands:
            "\(model.slashCommands.count) slash commands"
        case .mcp:
            "\(model.mcpServers.count) servers"
        }
    }
}

struct GaryxSettingsGatewayContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxSectionBlock(title: "Current") {
                GaryxGatewayCurrentRow()
            }

            if !model.gatewayProfiles.isEmpty {
                GaryxSectionBlock(title: "Gateways") {
                    GaryxCompactListGroup {
                        ForEach(Array(model.gatewayProfiles.enumerated()), id: \.element.id) { index, profile in
                            GaryxSavedGatewayProfileRow(
                                profile: profile,
                                isCurrent: model.currentGatewayProfile?.id == profile.id
                            )
                            if index < model.gatewayProfiles.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            } else {
                GaryxSectionBlock(title: "Gateways") {
                    GaryxGatewayEmptyProfilesRow()
                }
            }

            if let status = model.gatewaySettingsStatus, !status.isEmpty {
                Text(status)
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(GaryxTheme.accent)
                    .padding(.horizontal, 2)
            }
        }
    }
}

struct GaryxGatewayCurrentRow: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: currentIcon)
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(currentColor)
                .frame(width: 22, height: 22)

            VStack(alignment: .leading, spacing: 2) {
                Text(currentTitle)
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                Text(model.gatewayURL.isEmpty ? "No gateway selected" : model.gatewayURL)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 0)

            Button {
                Task { await model.connectAndRefresh() }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(GaryxFont.system(size: 13, weight: .semibold))
                    .frame(width: 34, height: 34)
                    .background(Color(.secondarySystemFill), in: Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Reconnect gateway")
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 8)
    }

    private var currentTitle: String {
        switch model.connectionState {
        case .ready(let version):
            if let version, !version.isEmpty {
                return "Connected \(version)"
            }
            return "Connected"
        case .checking:
            return "Connecting"
        case .failed:
            return "Connection failed"
        case .disconnected:
            return "Not connected"
        }
    }

    private var currentIcon: String {
        switch model.connectionState {
        case .ready:
            return "checkmark.circle.fill"
        case .checking:
            return "arrow.triangle.2.circlepath"
        case .failed:
            return "exclamationmark.circle.fill"
        case .disconnected:
            return "network"
        }
    }

    private var currentColor: Color {
        switch model.connectionState {
        case .ready:
            return GaryxTheme.accent
        case .checking:
            return .secondary
        case .failed:
            return GaryxTheme.danger
        case .disconnected:
            return .secondary
        }
    }
}

struct GaryxGatewayEmptyProfilesRow: View {
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "network")
                .font(GaryxFont.system(size: 14, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 22, height: 22)
            Text("No saved gateways")
                .font(GaryxFont.subheadline(weight: .medium))
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 9)
    }
}

struct GaryxGatewayProfileMenuButton: View {
    @EnvironmentObject private var model: GaryxMobileModel
    var onSelect: ((GaryxGatewayProfile) -> Void)?

    var body: some View {
        if model.gatewayProfiles.isEmpty {
            EmptyView()
        } else {
            Menu {
                ForEach(model.gatewayProfiles) { profile in
                    Button {
                        if let onSelect {
                            onSelect(profile)
                        } else {
                            Task { await model.activateGatewayProfile(profile) }
                        }
                    } label: {
                        Label(profile.gatewayUrl, systemImage: profile.hasToken ? "key.fill" : "network")
                    }
                }
            } label: {
                GaryxToolbarIcon(systemName: "clock.arrow.circlepath")
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Choose gateway")
        }
    }
}

struct GaryxSavedGatewayProfileRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let profile: GaryxGatewayProfile
    let isCurrent: Bool
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var label = ""
    @State private var gatewayUrl = ""
    @State private var token = ""

    var body: some View {
        GaryxSwipeActionRow(actions: profileSwipeActions) {
            HStack(spacing: 9) {
                Image(systemName: isCurrent ? "checkmark.circle.fill" : "network")
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(isCurrent ? GaryxTheme.accent : .secondary)
                    .frame(width: 20, height: 20)

                VStack(alignment: .leading, spacing: 2) {
                    Text(profile.label)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(profile.gatewayUrl)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                if profile.hasToken {
                    Image(systemName: "key.fill")
                        .font(GaryxFont.system(size: 11, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 7)
            .contentShape(Rectangle())
            .onTapGesture {
                Task { await model.activateGatewayProfile(profile) }
            }
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Gateway") {
                VStack(alignment: .leading, spacing: 12) {
                    GaryxFieldLabel("Gateway")
                    TextField("Name", text: $label)
                        .garyxInputStyle()
                    TextField("Gateway URL", text: $gatewayUrl)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    SecureField("Gateway Token", text: $token)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    Button {
                        if model.updateGatewayProfile(
                            profile,
                            label: label,
                            gatewayUrl: gatewayUrl,
                            token: token
                        ) {
                            showsEditForm = false
                        }
                    } label: {
                        Label("Save Gateway", systemImage: "checkmark")
                    }
                    .buttonStyle(GaryxPrimaryCompactButtonStyle())
                    .disabled(gatewayUrl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                .garyxCardStyle()
            }
        }
        .confirmationDialog("Delete gateway?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                model.removeGatewayProfile(profile)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the saved gateway profile from this device.")
        }
    }

    private var profileSwipeActions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(title: "Switch", systemImage: "arrow.triangle.2.circlepath", tone: .accent) {
                Task { await model.activateGatewayProfile(profile) }
            },
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }

    private func fillDraft() {
        label = profile.label
        gatewayUrl = profile.gatewayUrl
        token = model.gatewayProfileToken(profile)
    }
}

struct GaryxSettingsProviderContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if !model.providerModelsByType.isEmpty {
                GaryxSectionBlock(title: "Model Providers") {
                    GaryxCompactListGroup {
                        let providers = model.providerModelsByType
                            .values
                            .sorted { lhs, rhs in
                                let lhsName = garyxProviderDisplayName(lhs.providerType)
                                let rhsName = garyxProviderDisplayName(rhs.providerType)
                                if lhsName != rhsName {
                                    return lhsName < rhsName
                                }
                                return lhs.providerType < rhs.providerType
                            }
                        ForEach(Array(providers.enumerated()), id: \.element.providerType) { index, provider in
                            GaryxProviderModelsRow(provider: provider)
                            if index < providers.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }

            GaryxSectionBlock(title: "Default Agent") {
                GaryxCompactListGroup {
                    ForEach(Array(model.agentTargets.enumerated()), id: \.element.id) { index, target in
                        Button {
                            model.setSelectedAgentTarget(target.id)
                        } label: {
                            GaryxAgentIdentityRow(
                                id: target.id,
                                title: target.title,
                                subtitle: target.subtitle,
                                kind: target.kind,
                                avatarDataUrl: target.avatarDataUrl,
                                providerType: target.providerType,
                                builtIn: target.builtIn,
                                selected: model.selectedAgentTargetId == target.id
                            )
                        }
                        .buttonStyle(.plain)
                        if index < model.agentTargets.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                }
            }
        }
    }
}

struct GaryxProviderModelsRow: View {
    let provider: GaryxProviderModels

    var body: some View {
        HStack(spacing: 9) {
            Image(systemName: iconName)
                .font(GaryxFont.system(size: 14, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 20, height: 20)

            VStack(alignment: .leading, spacing: 2) {
                Text(garyxProviderDisplayName(provider.providerType))
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                Text(detail)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            GaryxStatusPill(text: hasError ? "Error" : "Ready", tone: hasError ? .danger : .good)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
    }

    private var iconName: String {
        let source = provider.providerType.lowercased()
        if source.contains("codex") {
            return "chevron.left.forwardslash.chevron.right"
        }
        if source.contains("claude") || source.contains("anthropic") {
            return "sparkles"
        }
        if source.contains("gemini") || source.contains("google") {
            return "diamond.fill"
        }
        if source.contains("gpt") || source.contains("openai") {
            return "circle.hexagongrid.fill"
        }
        return "cpu"
    }

    private var hasError: Bool {
        let error = provider.error?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return !error.isEmpty
    }

    private var detail: String {
        var parts: [String] = []
        if let defaultModel = provider.defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines), !defaultModel.isEmpty {
            parts.append("Default \(defaultModel)")
        }
        if provider.supportsModelSelection {
            parts.append("\(provider.models.count) models")
        }
        if provider.supportsReasoningEffortSelection {
            parts.append("\(provider.reasoningEfforts.count) reasoning")
        }
        if provider.supportsServiceTierSelection {
            parts.append("\(provider.serviceTiers.count) tiers")
        }
        if parts.isEmpty {
            if hasError {
                return "Model metadata unavailable"
            }
            return provider.source.isEmpty ? "Provider metadata" : provider.source.capitalized
        }
        return parts.joined(separator: " · ")
    }
}

private func garyxProviderDisplayName(_ providerType: String) -> String {
    switch providerType {
    case "codex_app_server":
        return "Codex"
    case "claude_code":
        return "Claude Code"
    case "gemini_cli":
        return "Gemini CLI"
    case "gpt":
        return "OpenAI"
    case "anthropic", "claude_llm":
        return "Anthropic"
    case "google", "gemini_llm":
        return "Google"
    default:
        let words = providerType
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")
            .split(separator: " ")
            .map { word in
                word.prefix(1).uppercased() + word.dropFirst()
            }
        return words.isEmpty ? "Provider" : words.joined(separator: " ")
    }
}

private extension GaryxTaskStatus {
    var label: String {
        switch self {
        case .todo:
            "Todo"
        case .inProgress:
            "In Progress"
        case .inReview:
            "In Review"
        case .done:
            "Done"
        }
    }

    var systemImage: String {
        switch self {
        case .todo:
            "circle"
        case .inProgress:
            "play.circle.fill"
        case .inReview:
            "arrowshape.turn.up.right.circle.fill"
        case .done:
            "checkmark.circle.fill"
        }
    }

    var allowedTransitions: [GaryxTaskStatus] {
        switch self {
        case .todo:
            [.inProgress]
        case .inProgress:
            [.inReview, .todo]
        case .inReview:
            [.done, .inProgress]
        case .done:
            [.todo]
        }
    }

    var tone: GaryxStatusPill.Tone {
        switch self {
        case .todo:
            .muted
        case .inProgress:
            .warning
        case .inReview:
            .danger
        case .done:
            .good
        }
    }
}

private extension GaryxTaskSummary {
    var displayId: String {
        if !id.isEmpty {
            id
        } else if number > 0 {
            "#TASK-\(number)"
        } else {
            "Task"
        }
    }

    var assigneeDisplayLabel: String {
        if let assignee {
            return assignee.garyxDisplayLabel
        }
        if !assigneeLabel.isEmpty {
            return assigneeLabel
        }
        return "Unassigned"
    }

    var formattedUpdatedAt: String {
        garyxFormattedTaskTimestamp(updatedAt)
    }
}

private extension GaryxTaskPrincipal {
    var garyxDisplayLabel: String {
        if kind == "human", let userId, !userId.isEmpty {
            return "@\(userId)"
        }
        if kind == "agent", let agentId, !agentId.isEmpty {
            return agentId
        }
        if let agentId, !agentId.isEmpty {
            return agentId
        }
        if let userId, !userId.isEmpty {
            return "@\(userId)"
        }
        return kind.isEmpty ? "Unknown" : kind
    }

}

private extension GaryxTaskSource {
    var detailLabel: String {
        if let taskId, !taskId.isEmpty {
            return taskId
        }
        if let taskThreadId, !taskThreadId.isEmpty {
            return taskThreadId
        }
        if let threadId, !threadId.isEmpty {
            return threadId
        }
        if let botId, !botId.isEmpty {
            return botId
        }
        let channel = channel ?? ""
        let account = accountId ?? ""
        if !channel.isEmpty, !account.isEmpty {
            return "\(channel) / \(account)"
        }
        if !channel.isEmpty {
            return channel
        }
        return "Unknown"
    }
}

private extension GaryxDreamTopic {
    var sourceDisplayLabel: String {
        let normalized = source.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return "unknown" }
        return normalized.replacingOccurrences(of: "_", with: " ")
    }

    var formattedLastMessageAt: String {
        garyxFormattedTaskTimestamp(lastMessageAt)
    }
}

private extension GaryxDreamSpan {
    var threadDisplayLabel: String {
        let seqLabel = startSeq == endSeq ? "#\(startSeq)" : "#\(startSeq)-#\(endSeq)"
        let workspace = workspacePath?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .garyxLastPathComponent ?? ""
        if workspace.isEmpty {
            return "\(threadId) \(seqLabel)"
        }
        return "\(workspace) / \(seqLabel)"
    }
}
