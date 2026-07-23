import QuartzCore
import SwiftUI
import UIKit

@MainActor
final class GaryxConversationRenderPrewarmStatus {
    struct Snapshot {
        let isReady: Bool
        let duration: CFTimeInterval?
    }

    static let shared = GaryxConversationRenderPrewarmStatus()

    private var beganAt: CFTimeInterval?
    private var completedAt: CFTimeInterval?

    private init() {}

    func began() {
        guard beganAt == nil else { return }
        beganAt = CACurrentMediaTime()
    }

    func completed() {
        guard beganAt != nil, completedAt == nil else { return }
        completedAt = CACurrentMediaTime()
    }

    var snapshot: Snapshot {
        Snapshot(
            isReady: completedAt != nil,
            duration: beganAt.flatMap { beganAt in
                completedAt.map { max(0, $0 - beganAt) }
            }
        )
    }
}

enum GaryxConversationRenderPrewarmFixture {
    static let representativeRows: [GaryxMobileTurnRow] = {
        let user = GaryxMobileMessage(
            id: "render-prewarm-user",
            role: .user,
            text: "Warm up conversation rendering.",
            timestamp: "00:00",
            isStreaming: false
        )
        let assistant = GaryxMobileMessage(
            id: "render-prewarm-assistant",
            role: .assistant,
            text: """
            Rendering is ready.

            - **Markdown** and `inline code` use the production message pipeline.
            """,
            timestamp: "00:00",
            isStreaming: false
        )
        return [
            GaryxMobileTurnRow(
                id: "render-prewarm-turn",
                userBlock: .message(user),
                activityRows: [.flat(.message(assistant))]
            ),
        ]
    }()
}

/// Startup-only copy of the live transcript's static layout seam. The real
/// message rows stay mounted behind the same opaque skeleton cover production
/// uses during materialization, so both pipelines warm without making them two
/// visible transcript treatments.
private struct GaryxConversationRenderPrewarmTranscriptView: View {
    var body: some View {
        ScrollView {
            ZStack(alignment: .topLeading) {
                Color.clear
                    .containerRelativeFrame(.vertical) { length, _ in length }
                    .allowsHitTesting(false)

                VStack(alignment: .leading) {
                    VStack(alignment: .leading, spacing: 14) {
                        Color.clear.frame(height: 1)
                        GaryxMobileTurnRowsView(
                            rows: GaryxConversationRenderPrewarmFixture.representativeRows
                        )
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 18)
                    .padding(.bottom, 24)
                    .garyxVerticalScrollContentWidth(alignment: .topLeading)

                    Color.clear
                        .frame(height: 24)
                        .accessibilityHidden(true)
                }
            }
            .background {
                GaryxTranscriptBlankSpaceTapLayer(action: {})
            }
        }
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .defaultScrollAnchor(.bottom, for: .sizeChanges)
        .scrollDismissesKeyboard(.interactively)
        .scrollDisabled(true)
    }
}

/// Startup-only compositor warm-up. Its topmost non-zero-opacity placement is
/// intentional: Core Animation may cull a hidden or fully transparent tree,
/// which would defer glass/markdown Metal pipeline compilation to the first
/// user push. Twelve consecutive delivered frames prove materialization before
/// this surface removes itself.
struct GaryxConversationRenderPrewarmer: View {
    @StateObject private var driver = GaryxConversationRenderPrewarmDriver()

    var body: some View {
        if driver.rendersWarmupSurface {
            ZStack {
                GaryxConversationRenderPrewarmTranscriptView()

                GaryxConversationOpeningTranscriptView(cover: .skeleton)
            }
            .garyxPageBackground()
            .garyxAdaptiveTopBar {
                prewarmHeader
            }
            .garyxFloatingBottomChrome {
                // The staged destination now mounts the production composer
                // on its first frame, so startup prewarming must exercise its
                // real UIKit input and shared glass card rather than a visual
                // stand-in.
                GaryxComposerRenderPrewarmSurface()
            }
            .opacity(0.01)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
            .onAppear {
                driver.start()
            }
            .onDisappear {
                driver.stop()
            }
        }
    }

    private var prewarmHeader: some View {
        GaryxAdaptiveGlassContainer(spacing: 10) {
            HStack(spacing: 12) {
                GaryxToolbarIcon(systemName: "chevron.left")

                GaryxThreadRuntimeCompactContentRow(
                    title: GaryxConversationOpeningMetadata.prewarmLocal.title,
                    target: GaryxConversationOpeningMetadata.prewarmLocal.agentTarget
                )
                .garyxAdaptiveGlass(.regular, isInteractive: false, in: Capsule())

                Spacer(minLength: 0)

                GaryxToolbarIcon {
                    GaryxInkSpinner()
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
    }
}

@MainActor
private final class GaryxConversationRenderPrewarmDriver: NSObject, ObservableObject {
    @Published private(set) var rendersWarmupSurface = true

    private var state = GaryxConversationRenderPrewarmState()
    private var displayLink: CADisplayLink?
    private var previousTimestamp: CFTimeInterval?

    func start() {
        guard rendersWarmupSurface, displayLink == nil else { return }
        state.begin()
        GaryxConversationRenderPrewarmStatus.shared.began()

        let link = CADisplayLink(target: self, selector: #selector(framePresented(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(
            minimum: 80,
            maximum: 120,
            preferred: 120
        )
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    func stop() {
        displayLink?.invalidate()
        displayLink = nil
        previousTimestamp = nil
    }

    @objc private func framePresented(_ link: CADisplayLink) {
        let frameBudget = max(1.0 / 120.0, link.targetTimestamp - link.timestamp)
        let interval = previousTimestamp.map { link.timestamp - $0 }
        previousTimestamp = link.timestamp
        guard state.presentedFrame(
            interval: interval,
            frameBudget: frameBudget
        ) == .ready else { return }

        GaryxConversationRenderPrewarmStatus.shared.completed()
        stop()
        rendersWarmupSurface = false
    }
}
