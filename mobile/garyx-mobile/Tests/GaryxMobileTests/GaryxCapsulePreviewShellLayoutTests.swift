import SwiftUI
import UIKit
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxCapsulePreviewShellLayoutTests: XCTestCase {
    func testChatCardImagePreviewOwnsFullWidthInLongEagerTranscript() async throws {
        let (model, defaults, suiteName) = try makeModel()
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let capsuleID = "synthetic-layout-capsule"
        let rendition = GaryxCapsuleThumbnailRendition.chatCard
        model.capsuleThumbnailMemory.set(
            makeThumbnailImage(),
            for: GaryxCapsuleThumbnailCacheKey(
                id: capsuleID,
                revision: 1,
                rendition: rendition
            )
        )

        let recorder = GaryxCapsulePreviewSizeRecorder()
        let controller = UIHostingController(
            rootView: GaryxCapsuleLongTranscriptLayoutHarness(
                model: model,
                capsuleID: capsuleID,
                rendition: rendition,
                sizeRecorder: recorder
            )
            .environmentObject(model)
        )
        let window = try present(controller)
        defer {
            controller.dismiss(animated: false)
            window.isHidden = true
            window.rootViewController = nil
        }

        let resolved = await waitUntil {
            controller.view.setNeedsLayout()
            controller.view.layoutIfNeeded()
            return recorder.sizes.contains { $0.height < 20 }
                || recorder.sizes.last.map {
                    abs($0.width - 320) < 0.001 && abs($0.height - 180) < 0.001
                } == true
        }
        XCTAssertTrue(resolved, "the eager transcript must resolve the cached image phase")

        let measured = try XCTUnwrap(recorder.sizes.last)
        print("CAPSULE_CHAT_PREVIEW_IMAGE_SIZE measured=\(measured) all=\(recorder.sizes)")
        XCTAssertEqual(measured.width, 320, accuracy: 0.001)
        XCTAssertEqual(measured.height, 180, accuracy: 0.001)
    }

    func testChatCardPreviewKeepsGeometryAcrossIdleImageFailedAndDeleted() async throws {
        let (model, defaults, suiteName) = try makeModel()
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let driver = GaryxCapsulePreviewPhaseDriver(phase: .idle)
        let recorder = GaryxCapsulePreviewPhaseRecorder()
        let controller = UIHostingController(
            rootView: GaryxCapsulePhaseTransitionLayoutHarness(
                model: model,
                driver: driver,
                rendition: .chatCard,
                recorder: recorder
            )
            .environmentObject(model)
        )
        let window = try present(controller)
        defer {
            controller.dismiss(animated: false)
            window.isHidden = true
            window.rootViewController = nil
        }

        let phases: [(String, GaryxCapsulePreviewThumbnail.Phase)] = [
            ("idle", .idle),
            ("image", .image(makeThumbnailImage())),
            ("failed", .failed),
            ("deleted", .deleted),
        ]
        for (name, phase) in phases {
            driver.phase = phase
            let measured = await waitForMeasurement(
                named: name,
                recorder: recorder,
                controller: controller
            )
            let size = try XCTUnwrap(measured?.size, "missing \(name) geometry")
            print("CAPSULE_CHAT_PREVIEW_PHASE_SIZE phase=\(name) measured=\(size)")
            XCTAssertEqual(size.width, 320, accuracy: 0.001, "\(name) width")
            XCTAssertEqual(size.height, 180, accuracy: 0.001, "\(name) height")
        }
    }

    func testGalleryPreviewKeepsExistingSixteenByTenGeometryAcrossPhases() {
        let phases: [GaryxCapsulePreviewThumbnail.Phase] = [
            .idle,
            .image(makeThumbnailImage()),
            .failed,
            .deleted,
        ]

        for phase in phases {
            let controller = UIHostingController(
                rootView: GaryxCapsulePreviewShell(
                    phase: phase,
                    rendition: .gallery,
                    cornerRadius: 0,
                    showsBorder: false
                )
            )
            let measured = controller.sizeThatFits(
                in: CGSize(width: 320, height: 1)
            )
            XCTAssertEqual(measured.width, 320, accuracy: 0.001)
            XCTAssertEqual(measured.height, 200, accuracy: 0.001)
        }
    }

    private func makeModel() throws -> (GaryxMobileModel, UserDefaults, String) {
        let suiteName = "GaryxCapsulePreviewShellLayoutTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        return (GaryxMobileModel(defaults: defaults), defaults, suiteName)
    }

    private func makeThumbnailImage() -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let renderer = UIGraphicsImageRenderer(
            size: CGSize(width: 1_170, height: 657),
            format: format
        )
        return renderer.image { context in
            UIColor.systemTeal.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 1_170, height: 657))
        }
    }

    private func present<Content: View>(
        _ controller: UIHostingController<Content>
    ) throws -> UIWindow {
        let windowScene = try XCTUnwrap(
            UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .first
        )
        let window = UIWindow(windowScene: windowScene)
        window.frame = CGRect(x: 0, y: 0, width: 440, height: 956)
        window.rootViewController = controller
        window.overrideUserInterfaceStyle = .light
        window.isHidden = false
        controller.view.frame = window.bounds
        window.layoutIfNeeded()
        controller.view.layoutIfNeeded()
        return window
    }

    private func waitForMeasurement<Content: View>(
        named name: String,
        recorder: GaryxCapsulePreviewPhaseRecorder,
        controller: UIHostingController<Content>
    ) async -> GaryxCapsulePreviewPhaseMeasurement? {
        _ = await waitUntil {
            controller.view.setNeedsLayout()
            controller.view.layoutIfNeeded()
            return recorder.measurement(named: name) != nil
        }
        return recorder.measurement(named: name)
    }

    private func waitUntil(
        timeout: Duration = .seconds(5),
        condition: @escaping @MainActor () -> Bool
    ) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            if condition() { return true }
            await Task.yield()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return condition()
    }
}

private final class GaryxCapsulePreviewSizeRecorder {
    private(set) var sizes: [CGSize] = []

    func record(_ size: CGSize) {
        guard sizes.last != size else { return }
        sizes.append(size)
    }
}

private struct GaryxCapsulePreviewPhaseMeasurement: Equatable {
    let name: String
    let size: CGSize
}

private final class GaryxCapsulePreviewPhaseRecorder {
    private var measurements: [GaryxCapsulePreviewPhaseMeasurement] = []

    func record(_ measurement: GaryxCapsulePreviewPhaseMeasurement) {
        guard measurements.last != measurement else { return }
        measurements.append(measurement)
    }

    func measurement(named name: String) -> GaryxCapsulePreviewPhaseMeasurement? {
        measurements.last { $0.name == name }
    }
}

@MainActor
private final class GaryxCapsulePreviewPhaseDriver: ObservableObject {
    @Published var phase: GaryxCapsulePreviewThumbnail.Phase

    init(phase: GaryxCapsulePreviewThumbnail.Phase) {
        self.phase = phase
    }
}

private struct GaryxCapsulePhaseTransitionLayoutHarness: View {
    @ObservedObject var model: GaryxMobileModel
    @ObservedObject var driver: GaryxCapsulePreviewPhaseDriver
    let rendition: GaryxCapsuleThumbnailRendition
    let recorder: GaryxCapsulePreviewPhaseRecorder
    private let sizeRecorder = GaryxCapsulePreviewSizeRecorder()

    var body: some View {
        GaryxCapsuleLongTranscriptLayoutHarness(
            model: model,
            capsuleID: "synthetic-phase-capsule",
            rendition: rendition,
            sizeRecorder: sizeRecorder,
            phase: driver.phase,
            phaseRecorder: recorder
        )
    }
}

private struct GaryxCapsuleLongTranscriptLayoutHarness: View {
    @ObservedObject var model: GaryxMobileModel
    let capsuleID: String
    let rendition: GaryxCapsuleThumbnailRendition
    let sizeRecorder: GaryxCapsulePreviewSizeRecorder
    var phase: GaryxCapsulePreviewThumbnail.Phase?
    var phaseRecorder: GaryxCapsulePreviewPhaseRecorder?

    init(
        model: GaryxMobileModel,
        capsuleID: String,
        rendition: GaryxCapsuleThumbnailRendition,
        sizeRecorder: GaryxCapsulePreviewSizeRecorder,
        phase: GaryxCapsulePreviewThumbnail.Phase? = nil,
        phaseRecorder: GaryxCapsulePreviewPhaseRecorder? = nil
    ) {
        self.model = model
        self.capsuleID = capsuleID
        self.rendition = rendition
        self.sizeRecorder = sizeRecorder
        self.phase = phase
        self.phaseRecorder = phaseRecorder
    }

    var body: some View {
        ScrollView {
            ZStack(alignment: .topLeading) {
                Color.clear
                    .containerRelativeFrame(.vertical) { length, _ in length }
                    .allowsHitTesting(false)

                VStack(alignment: .leading) {
                    VStack(alignment: .leading, spacing: 14) {
                        GaryxMobileTurnRowsView(rows: transcriptRows(range: 0..<18))

                        Button(action: {}) {
                            VStack(alignment: .leading, spacing: 0) {
                                preview
                                    .onGeometryChange(
                                        for: GaryxCapsulePreviewPhaseMeasurement.self
                                    ) { geometry in
                                        GaryxCapsulePreviewPhaseMeasurement(
                                            name: phaseName,
                                            size: geometry.size
                                        )
                                    } action: { size in
                                        sizeRecorder.record(size.size)
                                        phaseRecorder?.record(size)
                                    }

                                VStack(alignment: .leading, spacing: 2) {
                                    Text("Synthetic Capsule")
                                    Text("Created")
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(.horizontal, 12)
                                .padding(.vertical, 8)
                            }
                        }
                        .buttonStyle(GaryxPressableRowStyle())
                        .frame(maxWidth: 320, alignment: .leading)

                        GaryxMobileTurnRowsView(rows: transcriptRows(range: 18..<30))
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 18)
                    .padding(.bottom, 24)
                    .garyxVerticalScrollContentWidth(alignment: .topLeading)

                    Color.clear.frame(height: 24)
                    Color.clear.frame(height: 1)
                }
                // Mirror the finite iPhone 17 Pro Max viewport proposal that
                // exposed the intrinsic-size collapse in the eager transcript.
                .frame(height: 956, alignment: .topLeading)
            }
        }
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .defaultScrollAnchor(.bottom, for: .sizeChanges)
    }

    @ViewBuilder
    private var preview: some View {
        if let phase {
            GaryxCapsulePreviewShell(
                phase: phase,
                rendition: rendition,
                cornerRadius: 0,
                showsBorder: false
            )
        } else {
            GaryxCapsulePreviewThumbnail(
                capsuleId: capsuleID,
                revision: 1,
                rendition: rendition,
                cacheEpoch: model.capsuleHTMLCacheEpoch,
                cornerRadius: 0,
                showsBorder: false
            )
        }
    }

    private var phaseName: String {
        guard let phase else { return "thumbnail" }
        switch phase {
        case .idle:
            return "idle"
        case .image:
            return "image"
        case .failed:
            return "failed"
        case .deleted:
            return "deleted"
        }
    }

    private func transcriptRows(range: Range<Int>) -> [GaryxMobileTurnRow] {
        range.map { index in
            let user = GaryxMobileMessage(
                id: "synthetic-user-\(index)",
                role: .user,
                text: """
                Synthetic question \(index) with enough words to wrap across the transcript.
                """,
                isStreaming: false
            )
            let assistant = GaryxMobileMessage(
                id: "synthetic-assistant-\(index)",
                role: .assistant,
                text: """
                Synthetic response \(index) exercises the production Markdown row.

                - First deterministic transcript line
                - Second deterministic transcript line
                - Third deterministic transcript line
                """,
                isStreaming: false
            )
            return GaryxMobileTurnRow(
                id: "synthetic-turn-\(index)",
                userBlock: .message(user),
                activityRows: [.flat(.message(assistant))]
            )
        }
    }
}
