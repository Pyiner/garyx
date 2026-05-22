import SwiftUI

@main
struct GaryxMobileApp: App {
    @StateObject private var model = GaryxMobileModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            GaryxRootView()
                .environmentObject(model)
                .onChange(of: scenePhase) { _, phase in
                    model.handleScenePhase(phase)
                }
        }
    }
}
