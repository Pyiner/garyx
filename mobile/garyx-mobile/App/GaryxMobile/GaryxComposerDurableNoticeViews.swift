import SwiftUI

struct GaryxComposerDurableNoticeStack: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let notices: [GaryxComposerDurableNotice]
    private let actionHandler: (@MainActor (GaryxComposerDurableNoticeAction) async -> Void)?

    @State private var pendingDuplicateRiskAction: GaryxComposerDurableNoticeAction?
    @State private var inFlightActionID: String?

    init(
        notices: [GaryxComposerDurableNotice],
        actionHandler: (@MainActor (GaryxComposerDurableNoticeAction) async -> Void)? = nil
    ) {
        self.notices = notices
        self.actionHandler = actionHandler
    }

    var body: some View {
        VStack(spacing: 8) {
            ForEach(notices) { notice in
                noticeCard(notice)
            }
        }
        .padding(.horizontal, 10)
        .padding(.top, 10)
        .garyxAlert(
            "This may create a duplicate",
            isPresented: Binding(
                get: { pendingDuplicateRiskAction != nil },
                set: { if !$0 { pendingDuplicateRiskAction = nil } }
            )
        ) {
            Button("Cancel", role: .cancel) {
                pendingDuplicateRiskAction = nil
            }
            Button("Send duplicate-risk copy", role: .destructive) {
                guard let action = pendingDuplicateRiskAction else { return }
                pendingDuplicateRiskAction = nil
                perform(action)
            }
        } message: {
            Text("The original message or conversation may already exist. The copy uses a new intent ID, but the gateway cannot prevent a duplicate yet.")
        }
    }

    private func noticeCard(_ notice: GaryxComposerDurableNotice) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: iconName(for: notice.kind))
                .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                .foregroundStyle(tint(for: notice.kind))
                .frame(width: 22, height: 22)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 5) {
                Text(notice.title)
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.primary)

                Text(notice.detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                actionLayout(notice.actions)
                    .padding(.top, 2)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(tint(for: notice.kind).opacity(0.09), in: noticeShape)
        .overlay {
            noticeShape.stroke(tint(for: notice.kind).opacity(0.22), lineWidth: 0.75)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("composer-durable-notice-\(notice.id)")
    }

    @ViewBuilder
    private func actionLayout(
        _ actions: [GaryxComposerDurableNoticeAction]
    ) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                    actionButton(action)
                }
            }

            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                    actionButton(action)
                }
            }
        }
    }

    private func actionButton(_ action: GaryxComposerDurableNoticeAction) -> some View {
        let identifier = actionIdentifier(action)
        return Button {
            if isDuplicateRisk(action) {
                pendingDuplicateRiskAction = action
            } else {
                perform(action)
            }
        } label: {
            HStack(spacing: 5) {
                if inFlightActionID == identifier {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityHidden(true)
                }
                Text(actionTitle(action))
                    .font(.caption.weight(.semibold))
            }
            // Keep the semantic button frame safely above the 44 pt minimum
            // after SwiftUI converts between logical and simulator pixels.
            .frame(minHeight: 46)
            .padding(.horizontal, 10)
            .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .foregroundStyle(actionRoleTint(action))
        .disabled(inFlightActionID != nil)
        .accessibilityLabel(actionAccessibilityLabel(action))
        .accessibilityHint(actionAccessibilityHint(action))
        .accessibilityIdentifier("composer-durable-action-\(identifier)")
    }

    private func perform(_ action: GaryxComposerDurableNoticeAction) {
        let identifier = actionIdentifier(action)
        guard inFlightActionID == nil else { return }
        inFlightActionID = identifier
        Task { @MainActor in
            if let actionHandler {
                await actionHandler(action)
            } else {
                await model.performComposerDurableNoticeAction(action)
            }
            inFlightActionID = nil
        }
    }

    private var noticeShape: RoundedRectangle {
        RoundedRectangle(cornerRadius: 14, style: .continuous)
    }

    private func iconName(for kind: GaryxComposerDurableNoticeKind) -> String {
        switch kind {
        case .ambiguousDelivery, .ambiguousCreate: "questionmark.circle.fill"
        case .payloadConflict: "arrow.triangle.branch"
        case .feedback: "exclamationmark.circle.fill"
        }
    }

    private func tint(for kind: GaryxComposerDurableNoticeKind) -> Color {
        switch kind {
        case .ambiguousDelivery, .ambiguousCreate: .orange
        case .payloadConflict: .blue
        case .feedback: .yellow
        }
    }

    private func actionTitle(_ action: GaryxComposerDurableNoticeAction) -> String {
        switch action {
        case .restoreDelivery, .restoreCreate: "Restore draft"
        case .resendDeliveryCopy: "Resend copy"
        case .rebuildCreateCopy: "Rebuild copy"
        case .useRecoveredDraft: "Use recovered"
        case .keepCurrentDraft: "Keep current"
        case .acknowledgeFeedback: "Got it"
        case .retryUpload: "Retry upload"
        case .removeUpload: "Remove"
        }
    }

    private func actionAccessibilityLabel(
        _ action: GaryxComposerDurableNoticeAction
    ) -> String {
        switch action {
        case .restoreDelivery, .restoreCreate: "Restore uncertain send as draft"
        case .resendDeliveryCopy: "Resend a duplicate-risk copy"
        case .rebuildCreateCopy: "Rebuild a duplicate-risk conversation copy"
        case .useRecoveredDraft: "Use recovered message draft"
        case .keepCurrentDraft: "Keep current message draft"
        case .acknowledgeFeedback: "Dismiss this durable notice"
        case .retryUpload: "Retry the failed attachment upload"
        case .removeUpload: "Remove the failed attachment"
        }
    }

    private func actionAccessibilityHint(
        _ action: GaryxComposerDurableNoticeAction
    ) -> String {
        switch action {
        case .restoreDelivery, .restoreCreate:
            "Keeps the current draft and offers the recovered message separately."
        case .resendDeliveryCopy, .rebuildCreateCopy:
            "Shows a warning before sending with a new intent ID because a duplicate is possible."
        case .useRecoveredDraft:
            "Replaces the current composer draft with the recovered message."
        case .keepCurrentDraft:
            "Discards the recovered candidate and leaves the current draft unchanged."
        case .acknowledgeFeedback:
            "Acknowledges this notice in durable storage."
        case .retryUpload:
            "Transfers the staged attachment to a new upload attempt."
        case .removeUpload:
            "Acknowledges the notice and removes the staged attachment together."
        }
    }

    private func actionRoleTint(_ action: GaryxComposerDurableNoticeAction) -> Color {
        switch action {
        case .removeUpload: .red
        default: .accentColor
        }
    }

    private func isDuplicateRisk(_ action: GaryxComposerDurableNoticeAction) -> Bool {
        switch action {
        case .resendDeliveryCopy, .rebuildCreateCopy: true
        default: false
        }
    }

    private func actionIdentifier(_ action: GaryxComposerDurableNoticeAction) -> String {
        switch action {
        case .restoreDelivery(let id): "restore-delivery-\(id.rawValue)"
        case .resendDeliveryCopy(let id): "resend-delivery-\(id.rawValue)"
        case .restoreCreate(let key): "restore-create-\(key.createIntentID)"
        case .rebuildCreateCopy(let key): "rebuild-create-\(key.createIntentID)"
        case .useRecoveredDraft(let conflict, let entry):
            "use-recovered-\(conflict.rawValue)-\(entry.rawValue)"
        case .keepCurrentDraft(let conflict, let entry):
            "keep-current-\(conflict.rawValue)-\(entry.rawValue)"
        case .acknowledgeFeedback(let id): "ack-feedback-\(id.rawValue)"
        case .retryUpload(let id): "retry-upload-\(id.rawValue)"
        case .removeUpload(let id): "remove-upload-\(id.rawValue)"
        }
    }
}
