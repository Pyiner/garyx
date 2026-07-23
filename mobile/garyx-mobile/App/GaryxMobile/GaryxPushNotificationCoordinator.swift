import Foundation
import SwiftUI
import UIKit
import UserNotifications

@MainActor
final class GaryxMobileAppDelegate: NSObject, UIApplicationDelegate {
    private var deviceTokenHandler: ((Data) -> Void)?
    private var registrationFailureHandler: ((Error) -> Void)?
    private var pendingDeviceToken: Data?
    private var pendingRegistrationFailure: Error?

    func bind(
        deviceTokenHandler: @escaping (Data) -> Void,
        registrationFailureHandler: @escaping (Error) -> Void
    ) {
        self.deviceTokenHandler = deviceTokenHandler
        self.registrationFailureHandler = registrationFailureHandler
        if let pendingDeviceToken {
            self.pendingDeviceToken = nil
            deviceTokenHandler(pendingDeviceToken)
        }
        if let pendingRegistrationFailure {
            self.pendingRegistrationFailure = nil
            registrationFailureHandler(pendingRegistrationFailure)
        }
    }

    func application(
        _: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        if let deviceTokenHandler {
            deviceTokenHandler(deviceToken)
        } else {
            pendingDeviceToken = deviceToken
        }
    }

    func application(
        _: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        if let registrationFailureHandler {
            registrationFailureHandler(error)
        } else {
            pendingRegistrationFailure = error
        }
    }
}

@MainActor
final class GaryxPushNotificationCoordinator: NSObject, ObservableObject {
    private let notificationCenter: UNUserNotificationCenter
    private weak var model: GaryxMobileModel?
    private var registrationState = GaryxPushRegistrationState()
    private var gatewayConfigurations: [String: GaryxGatewayConfiguration] = [:]
    private var registrationTask: Task<Void, Never>?
    private var pendingTappedThreadID: String?
    private var didStartAuthorization = false

    init(notificationCenter: UNUserNotificationCenter = .current()) {
        self.notificationCenter = notificationCenter
        super.init()
        notificationCenter.delegate = self
    }

    func bind(model: GaryxMobileModel, appDelegate: GaryxMobileAppDelegate) {
        self.model = model
        refreshGatewayTarget()
        if let pendingTappedThreadID {
            self.pendingTappedThreadID = nil
            Task {
                await model.openThread(id: pendingTappedThreadID)
            }
        }
        appDelegate.bind(
            deviceTokenHandler: { [weak self] data in
                self?.receivedDeviceToken(data)
            },
            registrationFailureHandler: { _ in
                // Registration is best effort. A later foreground transition
                // asks iOS to register again without surfacing an app error.
            }
        )
    }

    func startAuthorizationAndRegistration() async {
        guard !didStartAuthorization else { return }
        didStartAuthorization = true
        let settings = await notificationCenter.notificationSettings()
        switch settings.authorizationStatus {
        case .notDetermined:
            do {
                let granted = try await notificationCenter.requestAuthorization(
                    options: [.alert, .sound]
                )
                if granted {
                    UIApplication.shared.registerForRemoteNotifications()
                }
            } catch {
                // Authorization failures never affect ordinary app use.
            }
        case .authorized, .provisional, .ephemeral:
            UIApplication.shared.registerForRemoteNotifications()
        case .denied:
            break
        @unknown default:
            break
        }
    }

    func gatewayRuntimeIdentityDidChange() {
        refreshGatewayTarget()
        perform(registrationState.handle(.foregrounded))
    }

    func sceneDidBecomeActive() {
        guard didStartAuthorization else { return }
        perform(registrationState.handle(.foregrounded))
        Task { [weak self] in
            guard let self else { return }
            let settings = await self.notificationCenter.notificationSettings()
            switch settings.authorizationStatus {
            case .authorized, .provisional, .ephemeral:
                UIApplication.shared.registerForRemoteNotifications()
            case .notDetermined, .denied:
                break
            @unknown default:
                break
            }
        }
    }

    private func refreshGatewayTarget() {
        let targetID: String?
        let configuration: GaryxGatewayConfiguration?
        if let model,
           model.currentGatewayScopeId != "unconfigured",
           let client = try? model.client() {
            targetID = model.currentGatewayScopeId
            configuration = client.configuration
        } else {
            targetID = nil
            configuration = nil
        }
        if targetID != registrationState.currentTargetID {
            registrationTask?.cancel()
            registrationTask = nil
        }
        if let targetID, let configuration {
            gatewayConfigurations[targetID] = configuration
        }
        perform(registrationState.handle(.gatewayChanged(targetID)))
    }

    private func receivedDeviceToken(_ data: Data) {
        let environment: GaryxPushEnvironment
        #if DEBUG
        environment = .forBuild(isDebugBuild: true)
        #else
        environment = .forBuild(isDebugBuild: false)
        #endif
        let device = GaryxPushDeviceRegistration(
            token: GaryxPushDeviceToken.hexadecimal(data),
            environment: environment,
            bundleID: Bundle.main.bundleIdentifier ?? "com.garyx.mobile",
            deviceName: UIDevice.current.name
        )
        perform(registrationState.handle(.deviceTokenReceived(device)))
    }

    private func perform(_ actions: [GaryxPushRegistrationAction]) {
        for action in actions {
            switch action {
            case .register(let key):
                register(key)
            case .unregister(let targetID, let token):
                unregister(targetID: targetID, token: token)
            }
        }
    }

    private func register(_ key: GaryxPushRegistrationKey) {
        registrationTask?.cancel()
        guard let configuration = gatewayConfigurations[key.targetID] else {
            perform(
                registrationState.handle(
                    .registrationFinished(key, succeeded: false)
                )
            )
            return
        }
        registrationTask = Task { [weak self] in
            let succeeded: Bool
            do {
                let client = GaryxGatewayClient(configuration: configuration)
                _ = try await client.registerPushDevice(
                    GaryxPushDeviceRegistrationRequest(device: key.device)
                )
                succeeded = true
            } catch {
                succeeded = false
            }
            guard let self else { return }
            self.perform(
                self.registrationState.handle(
                    .registrationFinished(key, succeeded: succeeded)
                )
            )
        }
    }

    private func unregister(targetID: String, token: String) {
        guard let configuration = gatewayConfigurations[targetID] else { return }
        Task {
            let client = GaryxGatewayClient(configuration: configuration)
            _ = try? await client.unregisterPushDevice(token: token)
        }
    }

    private func presentationOptions(
        for userInfo: [AnyHashable: Any]
    ) -> UNNotificationPresentationOptions {
        let payload = GaryxPushPayloadParser.parse(userInfo: userInfo)
        let decision = GaryxPushPayloadParser.foregroundPresentation(
            for: payload,
            openThreadID: model?.selectedThread?.id
        )
        return decision == .suppress ? [] : [.banner, .list, .sound]
    }

    private func handleNotificationTap(userInfo: [AnyHashable: Any]) async {
        guard let payload = GaryxPushPayloadParser.parse(userInfo: userInfo),
              case .thread(let threadID) = GaryxPushPayloadParser.route(for: payload) else {
            return
        }
        guard let model else {
            pendingTappedThreadID = threadID
            return
        }
        await model.openThread(id: threadID)
    }
}

extension GaryxPushNotificationCoordinator: UNUserNotificationCenterDelegate {
    nonisolated func userNotificationCenter(
        _: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        let userInfo = notification.request.content.userInfo
        return await presentationOptions(for: userInfo)
    }

    nonisolated func userNotificationCenter(
        _: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let userInfo = response.notification.request.content.userInfo
        await handleNotificationTap(userInfo: userInfo)
    }
}
