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
    @State private var notices = Self.initialNoticesForEnvironment
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
                        notices = Self.initialNoticesForEnvironment
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
            status = "restore:automatic"
        case .resendDeliveryCopy:
            notices.removeAll { $0.id == Self.ambiguousNoticeID }
            status = "resend:new-client-intent"
        case .acknowledgeFeedback(let id):
            notices.removeAll { $0.id == "feedback:\(id.rawValue)" }
            status = "feedback:acknowledged"
        case .retryUpload(let id):
            notices.removeAll { $0.id == "feedback:\(id.rawValue)" }
            status = "upload:retried"
        case .removeUpload(let id):
            notices.removeAll { $0.id == "feedback:\(id.rawValue)" }
            status = "upload:removed"
        case .restoreCreate:
            notices.removeAll { $0.id == Self.ambiguousCreateNoticeID }
            status = "create:restore:automatic"
        case .rebuildCreateCopy:
            notices.removeAll { $0.id == Self.ambiguousCreateNoticeID }
            status = "create:rebuild:new-client-intent"
        }
    }

    private static let ambiguousNoticeID = "delivery:fixture-send"
    private static let ambiguousCreateNoticeID = "create:fixture-create"
    private static let backpressureFeedbackID = GaryxFeedbackID(
        rawValue: "fixture-backpressure"
    )
    private static let uploadFeedbackID = GaryxFeedbackID(rawValue: "fixture-upload")

    private static var initialNoticesForEnvironment: [GaryxComposerDurableNotice] {
        ProcessInfo.processInfo.environment["GARYX_MOBILE_DURABLE_DELIVERY_SCENARIO"]
            == "create"
            ? [ambiguousCreateNotice]
            : initialNotices
    }

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

    private static let ambiguousCreateNotice = GaryxComposerDurableNotice(
        id: ambiguousCreateNoticeID,
        kind: .ambiguousCreate,
        title: "Conversation creation status unknown",
        detail: "The conversation may already exist. Rebuilding can create another conversation.",
        actions: [
            .restoreCreate(
                .init(
                    scope: .init(identity: "fixture-gateway", epoch: 1),
                    createIntentID: "fixture-create"
                )
            ),
            .rebuildCreateCopy(
                .init(
                    scope: .init(identity: "fixture-gateway", epoch: 1),
                    createIntentID: "fixture-create"
                )
            ),
        ]
    )
}
#endif
