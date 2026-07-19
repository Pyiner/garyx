#if DEBUG
import SwiftUI

@MainActor
struct GaryxComposerDurableNoticeDebugFixture {
    static var current: Self? {
        ProcessInfo.processInfo.environment["GARYX_MOBILE_DURABLE_DELIVERY_FIXTURE"] == "1"
            ? Self()
            : nil
    }

    var view: some View {
        GaryxComposerDurableNoticeDebugFixtureView()
    }
}

@MainActor
private struct GaryxComposerDurableNoticeDebugFixtureView: View {
    @State private var notices = Self.initialNotices
    @State private var status = "ready"

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text("Durable delivery")
                        .font(.title2.bold())
                        .accessibilityAddTraits(.isHeader)

                    Text("Fixture composer")
                        .font(.body)
                        .foregroundStyle(.secondary)

                    Button {
                        notices = Self.initialNotices
                        status = "send:ambiguous"
                    } label: {
                        Text("Send fixture message")
                            .frame(minHeight: 46)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.borderedProminent)
                    .accessibilityHint(
                        "Creates the same unknown-send surface shown after a lost transport response."
                    )
                    .accessibilityIdentifier("durable.fixture.send")

                    GaryxComposerDurableNoticeStack(
                        notices: notices,
                        actionHandler: perform
                    )

                    Text(status)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("durable.fixture.status")
                }
                .padding(16)
            }
            .background(Color(uiColor: .systemGroupedBackground))
            .navigationTitle("Delivery fixture")
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private func perform(_ action: GaryxComposerDurableNoticeAction) async {
        switch action {
        case .restoreDelivery:
            notices.removeAll { $0.id == Self.ambiguousNoticeID }
            notices.insert(Self.conflictNotice, at: 0)
            status = "restore:conflict"
        case .resendDeliveryCopy:
            notices.removeAll { $0.id == Self.ambiguousNoticeID }
            status = "resend:new-client-intent"
        case .useRecoveredDraft:
            notices.removeAll { $0.id == Self.conflictNoticeID }
            status = "conflict:recovered"
        case .keepCurrentDraft:
            notices.removeAll { $0.id == Self.conflictNoticeID }
            status = "conflict:current"
        case .acknowledgeFeedback(let id):
            notices.removeAll { $0.id == "feedback:\(id.rawValue)" }
            status = "feedback:acknowledged"
        case .retryUpload(let id):
            notices.removeAll { $0.id == "feedback:\(id.rawValue)" }
            status = "upload:retried"
        case .removeUpload(let id):
            notices.removeAll { $0.id == "feedback:\(id.rawValue)" }
            status = "upload:removed"
        case .restoreCreate, .rebuildCreateCopy:
            status = "create:handled"
        }
    }

    private static let ambiguousNoticeID = "delivery:fixture-send"
    private static let conflictNoticeID = "conflict:fixture-conflict"
    private static let backpressureFeedbackID = GaryxFeedbackID(
        rawValue: "fixture-backpressure"
    )
    private static let uploadFeedbackID = GaryxFeedbackID(rawValue: "fixture-upload")

    private static let initialNotices: [GaryxComposerDurableNotice] = [
        GaryxComposerDurableNotice(
            id: ambiguousNoticeID,
            kind: .ambiguousDelivery,
            title: "Send status unknown",
            detail: "The gateway may have accepted this message. Resending can create a duplicate.",
            actions: [
                .restoreDelivery(.init(rawValue: "fixture-send")),
                .resendDeliveryCopy(.init(rawValue: "fixture-send")),
            ]
        ),
        GaryxComposerDurableNotice(
            id: "feedback:\(backpressureFeedbackID.rawValue)",
            kind: .feedback,
            title: "Too many sends awaiting confirmation",
            detail: "This draft was kept. Resolve an unknown send before trying again.",
            actions: [.acknowledgeFeedback(backpressureFeedbackID)]
        ),
        GaryxComposerDurableNotice(
            id: "feedback:\(uploadFeedbackID.rawValue)",
            kind: .feedback,
            title: "Upload did not finish",
            detail: "Retry the upload or remove this attachment.",
            actions: [.retryUpload(uploadFeedbackID), .removeUpload(uploadFeedbackID)]
        ),
    ]

    private static let conflictNotice = GaryxComposerDurableNotice(
        id: conflictNoticeID,
        kind: .payloadConflict,
        title: "Recovered message is ready",
        detail: "Choose which draft should remain in the composer.",
        actions: [
            .useRecoveredDraft(
                .init(rawValue: "fixture-conflict"),
                .init(rawValue: "fixture-recovered-entry")
            ),
            .keepCurrentDraft(
                .init(rawValue: "fixture-conflict"),
                .init(rawValue: "fixture-recovered-entry")
            ),
        ]
    )
}
#endif
