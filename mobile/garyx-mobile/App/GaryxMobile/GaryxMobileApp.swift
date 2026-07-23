import SwiftUI

@main
struct GaryxMobileApp: App {
    @UIApplicationDelegateAdaptor(GaryxMobileAppDelegate.self) private var appDelegate
    @StateObject private var model = GaryxMobileModel()
    @StateObject private var pushNotifications = GaryxPushNotificationCoordinator()
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
                .task {
                    guard shouldStartPushNotifications else { return }
                    pushNotifications.bind(model: model, appDelegate: appDelegate)
                    await pushNotifications.startAuthorizationAndRegistration()
                }
                .onChange(of: model.currentGatewayRuntimeIdentity) { _, _ in
                    pushNotifications.gatewayRuntimeIdentityDidChange()
                }
                .onChange(of: scenePhase) { _, phase in
                    model.handleScenePhase(phase)
                    if phase == .active {
                        pushNotifications.sceneDidBecomeActive()
                    }
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
        } else if let fixture = GaryxTaskNotificationDebugFixture.current {
            fixture.view
        } else {
            GaryxRootView(model: model)
        }
        #else
        GaryxRootView(model: model)
        #endif
    }

    private var shouldStartPushNotifications: Bool {
        #if DEBUG
        GaryxFluidFakeRouteDebugFixture.current == nil
            && GaryxImagePreviewDebugFixture.current == nil
            && GaryxTaskNotificationDebugFixture.current == nil
        #else
        true
        #endif
    }
}
