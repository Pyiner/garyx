import Foundation
import SwiftUI

/// Root-sidebar gateway identity control. Tap opens the switcher sheet;
/// long-press lists saved gateways for one-step switching. Gateway
/// management (add/edit/delete/token) stays in Settings -> Gateway.
struct GaryxSidebarGatewayIdentityControl: View {
    let identity: GaryxGatewaySwitcherIdentity
    let rows: [GaryxGatewaySwitcherRow]
    let onSwitch: (GaryxGatewaySwitcherRow) -> Void
    let onManageGateways: () -> Void
    @Binding var debugShowsGatewaySwitcher: Bool
    @State private var showsSwitcher = false

    var body: some View {
        if identity.isInteractive {
            Menu {
                switcherMenuItems
            } label: {
                identityLabel(identity)
            } primaryAction: {
                showsSwitcher = true
            }
            .buttonStyle(.plain)
            .accessibilityLabel(accessibilityText(for: identity))
            .accessibilityHint("Opens the gateway switcher")
            .garyxSheet(isPresented: $showsSwitcher) {
                GaryxGatewaySwitcherSheet()
            }
            #if DEBUG
            .onAppear {
                presentDebugSwitcherIfNeeded()
            }
            .onChange(of: debugShowsGatewaySwitcher) { _, _ in
                presentDebugSwitcherIfNeeded()
            }
            #endif
        } else {
            Text(identity.title)
                .font(GaryxFont.system(size: 26, weight: .semibold))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .minimumScaleFactor(0.75)
        }
    }

    private func identityLabel(_ identity: GaryxGatewaySwitcherIdentity) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 6) {
                Text(identity.title)
                    .font(GaryxFont.system(size: 22, weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.75)

                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.secondary)
            }

            // Connected is the normal state and needs no callout; surface
            // the status line only when something is off.
            if identity.status != .connected, let subtitle = identity.subtitle {
                HStack(spacing: 5) {
                    Circle()
                        .fill(identity.status.garyxStatusColor)
                        .frame(width: 7, height: 7)
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
        }
        .contentShape(Rectangle())
    }

    @ViewBuilder
    private var switcherMenuItems: some View {
        ForEach(rows) { row in
            Button {
                onSwitch(row)
            } label: {
                GaryxMenuSelectionLabel(
                    title: row.title,
                    selected: row.isCurrent,
                    fallbackSystemImage: "network"
                )
            }
        }

        Divider()

        Button {
            onManageGateways()
        } label: {
            Label("Manage Gateways", systemImage: "gearshape")
        }
    }

    private func accessibilityText(for identity: GaryxGatewaySwitcherIdentity) -> String {
        if let subtitle = identity.subtitle {
            return "Gateway \(identity.title), \(subtitle)"
        }
        return "Gateway \(identity.title)"
    }

    #if DEBUG
    private func presentDebugSwitcherIfNeeded() {
        guard debugShowsGatewaySwitcher else { return }
        debugShowsGatewaySwitcher = false
        showsSwitcher = true
    }
    #endif
}

/// Half-height gateway switcher: pick a saved gateway, add a new one, or jump
/// to Settings -> Gateway. Switching stays in the sidebar so the new
/// gateway's thread list is the landing point.
struct GaryxGatewaySwitcherSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(spacing: 0) {
            header

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    let rows = model.gatewaySwitcherRows
                    ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                        gatewayRow(row)
                        if index < rows.count - 1 {
                            Divider().padding(.leading, 52)
                        }
                    }
                }
                .padding(.bottom, 12)
            }
            .scrollIndicators(.hidden)

            footerActions
        }
        .garyxWorkspacePickerSheetStyle()
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    private var header: some View {
        HStack(alignment: .center, spacing: 12) {
            Text("Gateways")
                .font(GaryxFont.callout(weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
            Spacer(minLength: 0)
        }
        .overlay(alignment: .trailing) {
            Button {
                dismiss()
            } label: {
                Image(systemName: "xmark")
                    .font(GaryxFont.system(size: 12, weight: .bold))
                    .foregroundStyle(.secondary)
                    .frame(width: 30, height: 30)
                    .background(.quaternary.opacity(0.5), in: Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Close")
        }
        .padding(.horizontal, 22)
        .padding(.top, 22)
        .padding(.bottom, 14)
    }

    private func gatewayRow(_ row: GaryxGatewaySwitcherRow) -> some View {
        Button {
            handleTap(row)
        } label: {
            HStack(spacing: 14) {
                Circle()
                    .fill(dotColor(for: row))
                    .frame(width: 8, height: 8)
                    .frame(width: 30)

                VStack(alignment: .leading, spacing: 2) {
                    Text(row.title)
                        .font(GaryxFont.callout(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(rowSubtitle(for: row))
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 0)

                if row.isCurrent {
                    GaryxSelectionCheckmark(size: 16)
                }
            }
            .frame(minHeight: 54)
            .padding(.horizontal, 20)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(row.isCurrent ? "\(row.title), current gateway" : row.title)
    }

    private var footerActions: some View {
        VStack(spacing: 0) {
            Divider()

            Button {
                dismiss()
                model.openSettings(tab: .gateway)
            } label: {
                footerRow(title: "Manage Gateways", systemImage: "gearshape")
            }
            .buttonStyle(.plain)
        }
        .padding(.bottom, 6)
    }

    private func footerRow(title: String, systemImage: String) -> some View {
        HStack(spacing: 14) {
            Image(systemName: systemImage)
                .font(GaryxFont.system(size: 17, weight: .semibold))
                .frame(width: 30)

            Text(title)
                .font(GaryxFont.callout(weight: .medium))

            Spacer(minLength: 0)
        }
        .foregroundStyle(.primary)
        .frame(height: 48)
        .padding(.horizontal, 20)
        .contentShape(Rectangle())
    }

    private func handleTap(_ row: GaryxGatewaySwitcherRow) {
        if row.isCurrent {
            if !model.isGatewayConnectionReady {
                Task { await model.connectAndRefresh() }
            }
            dismiss()
            return
        }
        guard let profile = model.gatewayProfiles.first(where: { $0.id == row.profileId }) else { return }
        dismiss()
        Task { await model.activateGatewayProfile(profile) }
    }

    private func dotColor(for row: GaryxGatewaySwitcherRow) -> Color {
        guard row.isCurrent else {
            return Color(.systemGray4)
        }
        return model.gatewaySwitcherIdentity.status.garyxStatusColor
    }

    private func rowSubtitle(for row: GaryxGatewaySwitcherRow) -> String {
        guard row.isCurrent else {
            return row.subtitle
        }
        return "\(row.subtitle) · \(GaryxGatewaySwitcherPresentation.statusLabel(for: model.connectionState))"
    }
}

extension GaryxGatewaySwitcherStatus {
    var garyxStatusColor: Color {
        switch self {
        case .connected:
            GaryxTheme.accent
        case .connecting:
            GaryxTheme.warning
        case .failed:
            GaryxTheme.danger
        case .notConnected:
            Color(.systemGray3)
        }
    }
}
