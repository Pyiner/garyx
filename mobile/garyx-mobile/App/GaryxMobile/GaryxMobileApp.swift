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
            GaryxRootView(model: model)
                .environmentObject(model)
                .environment(model.homeObservationStore)
                .environment(\.garyxAvatarImageProvider, model.avatarImageProvider)
                .environment(\.garyxAvatarScopeId, model.currentGatewayScopeId)
                .onChange(of: scenePhase) { _, phase in
                    model.handleScenePhase(phase)
                }
        }
    }
}
