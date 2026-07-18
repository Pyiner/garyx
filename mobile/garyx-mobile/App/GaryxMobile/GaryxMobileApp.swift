import SwiftUI

@main
struct GaryxMobileApp: App {
    @StateObject private var model = GaryxMobileModel()
    @Environment(\.scenePhase) private var scenePhase

    init() {
        GaryxSafeAreaChrome.installWindowDefaults()
    }

    var body: some Scene {
        WindowGroup {
            rootContent
                .environmentObject(model)
                .environment(model.homeObservationStore)
                .environment(\.garyxAvatarImageProvider, model.avatarImageProvider)
                .environment(\.garyxAvatarScopeId, model.currentGatewayScopeId)
                .garyxAccessibilityPreferences()
                .onChange(of: scenePhase) { _, phase in
                    model.handleScenePhase(phase)
                }
        }
    }

    @ViewBuilder
    private var rootContent: some View {
        #if DEBUG
        if let fixture = GaryxFluidFakeRouteDebugFixture.current {
            fixture.view
        } else if let fixture = GaryxImagePreviewDebugFixture.current {
            fixture.view
        } else {
            GaryxRootView(model: model)
        }
        #else
        GaryxRootView(model: model)
        #endif
    }
}
