import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

@MainActor
final class GaryxPresentationLeaseCoordinator {
    private weak var container: GaryxRouteStackContainer?
    private weak var routeStore: GaryxProductionRouteStore?

    func attach(
        container: GaryxRouteStackContainer,
        routeStore: GaryxProductionRouteStore
    ) {
        self.container = container
        self.routeStore = routeStore
        synchronizeBarrier()
    }

    func detach(container: GaryxRouteStackContainer) {
        guard self.container === container else { return }
        self.container = nil
        routeStore?.presentationBarrierStateChanged(
            false,
            observableSettlement: .afterViewGraphUpdate
        )
        routeStore = nil
    }

    @discardableResult
    func acquire(
        _ token: GaryxPresentationLeaseToken,
        parent: GaryxPresentationLeaseToken?,
        resultBearing: Bool
    ) -> Bool {
        _ = container?.reclaimReleasedPresentationLeases()
        let acquired = container?.acquirePresentationLease(
            token,
            parent: parent,
            resultBearing: resultBearing
        ) ?? false
        synchronizeBarrier()
        return acquired
    }

    func markPresented(_ token: GaryxPresentationLeaseToken) {
        container?.markPresentationLeasePresented(token)
    }

    func markDismissing(_ token: GaryxPresentationLeaseToken) {
        container?.markPresentationLeaseDismissing(token)
    }

    func recordResult(_ token: GaryxPresentationLeaseToken) {
        container?.recordPresentationResult(token)
        synchronizeBarrier()
        routeStore?.presentationBarrierDidChange()
    }

    func recordNoResult(_ token: GaryxPresentationLeaseToken) {
        container?.recordPresentationNoResult(token)
        synchronizeBarrier()
        routeStore?.presentationBarrierDidChange()
    }

    func dismissalCompleted(_ token: GaryxPresentationLeaseToken) {
        container?.presentationDismissalCompleted(token)
        synchronizeBarrier()
        routeStore?.presentationBarrierDidChange()
    }

    func presentationFailed(_ token: GaryxPresentationLeaseToken) {
        container?.presentationFailed(token)
        synchronizeBarrier()
        routeStore?.presentationBarrierDidChange()
    }

    func ownerPresentationEnded(_ token: GaryxPresentationLeaseToken) {
        container?.ownerPresentationEnded(token)
        synchronizeBarrier()
        routeStore?.presentationBarrierDidChange()
    }

    func presentationHasEnded(_ witness: GaryxPresentationControllerWitness) -> Bool {
        guard witness.didResolveController, let container else { return false }
        guard let controller = witness.controller else { return true }
        return !container.containsControllerInPresentedHierarchy(controller)
    }

    func record(for token: GaryxPresentationLeaseToken) -> GaryxPresentationLeaseRecord? {
        container?.presentationLeaseRecord(token)
    }

    private func synchronizeBarrier() {
        routeStore?.presentationBarrierStateChanged(
            container?.hasPresentationBarrier ?? false
        )
    }
}

/// Stable, imperative UIKit identity captured by presented content. This state
/// is intentionally not observable: representable hierarchy callbacks may
/// update it without publishing into the active SwiftUI graph.
@MainActor
final class GaryxPresentationControllerWitness {
    private(set) weak var controller: UIViewController?
    private(set) var didResolveController = false

    func resolve(from view: UIView) {
        var responder: UIResponder? = view
        while let next = responder?.next {
            if let controller = next as? UIViewController {
                self.controller = controller
                didResolveController = true
                return
            }
            responder = next
        }
    }

    func reset() {
        controller = nil
        didResolveController = false
    }
}

private struct GaryxPresentationControllerWitnessReader: UIViewRepresentable {
    let witness: GaryxPresentationControllerWitness

    func makeUIView(context: Context) -> ProbeView {
        let view = ProbeView()
        view.isUserInteractionEnabled = false
        view.isAccessibilityElement = false
        view.witness = witness
        return view
    }

    func updateUIView(_ uiView: ProbeView, context: Context) {
        uiView.witness = witness
    }

    final class ProbeView: UIView {
        weak var witness: GaryxPresentationControllerWitness?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            witness?.resolve(from: self)
        }

        override func didMoveToSuperview() {
            super.didMoveToSuperview()
            witness?.resolve(from: self)
        }

        override func layoutSubviews() {
            super.layoutSubviews()
            witness?.resolve(from: self)
        }
    }
}

@MainActor
final class GaryxPresentationLeaseSession: ObservableObject {
    private let resultBearing: Bool
    private var coordinator: GaryxPresentationLeaseCoordinator?
    @Published private(set) var token: GaryxPresentationLeaseToken?
    private(set) var operationContext: GaryxPresentationOperationContext?
    let presentationControllerWitness = GaryxPresentationControllerWitness()
    private var appeared = false
    private var didCompleteDismissal = false
    private var receivedTerminalCallback = false
    private var cycleFinished = false

    init(resultBearing: Bool = false) {
        self.resultBearing = resultBearing
    }

    func acquireIfNeeded(
        coordinator: GaryxPresentationLeaseCoordinator?,
        parent: GaryxPresentationLeaseToken?,
        operationContext: @MainActor () -> GaryxPresentationOperationContext?
    ) {
        if let token {
            let record = self.coordinator?.record(for: token)
            if record == nil || record?.released == true {
                // Another presentation may already have reclaimed the
                // released audit forest. An unknown old token is terminal,
                // never an active lease that should block reacquisition.
                cycleFinished = true
            }
        }
        if cycleFinished {
            resetForNextCycle()
        }
        guard token == nil, let coordinator else { return }
        let token = GaryxPresentationLeaseToken(
            rawValue: UUID().uuidString.lowercased()
        )
        guard coordinator.acquire(
            token,
            parent: parent,
            resultBearing: resultBearing
        ) else { return }
        self.coordinator = coordinator
        self.token = token
        self.operationContext = operationContext()
    }

    func markPresented() {
        guard let token else { return }
        appeared = true
        coordinator?.markPresented(token)
    }

    func markDismissing() {
        guard !didCompleteDismissal, let token else { return }
        coordinator?.markDismissing(token)
    }

    func recordResult() {
        guard resultBearing, let token else { return }
        coordinator?.recordResult(token)
        updateFinishedState()
    }

    func recordNoResult() {
        guard resultBearing, let token else { return }
        coordinator?.recordNoResult(token)
        updateFinishedState()
    }

    func completeDismissal() {
        guard !didCompleteDismissal, let token else { return }
        receivedTerminalCallback = true
        didCompleteDismissal = true
        coordinator?.dismissalCompleted(token)
        updateFinishedState()
    }

    /// SwiftUI can tear down presented content with no binding write-back and
    /// no onDismiss callback when its presenting owner disappears. Defer long
    /// enough for ordinary callbacks in the same lifecycle turn to win, then
    /// settle only from precise UIKit controller-chain evidence.
    func presentedContentDisappeared() {
        guard let token else { return }
        Task { @MainActor [weak self] in
            await Task.yield()
            await Task.yield()
            guard let self,
                  self.token == token,
                  !self.receivedTerminalCallback,
                  let record = self.coordinator?.record(for: token),
                  !record.released,
                  self.coordinator?.presentationHasEnded(
                      self.presentationControllerWitness
                  ) == true else {
                return
            }
            self.didCompleteDismissal = true
            self.coordinator?.ownerPresentationEnded(token)
            self.updateFinishedState()
        }
    }

    /// Picker frameworks do not all expose an explicit cancellation callback.
    /// Defer the no-result edge one main-actor turn so a result delivered in
    /// either order in the same frame wins the join without being discarded.
    func scheduleNoResultIfPending() {
        guard resultBearing, let token else { return }
        Task { @MainActor [weak self] in
            await Task.yield()
            guard let self,
                  self.token == token,
                  self.coordinator?.record(for: token)?.result == .pending else {
                return
            }
            self.coordinator?.recordNoResult(token)
            self.updateFinishedState()
        }
    }

    func bindingBecameFalse(completesDismissal: Bool) {
        guard token != nil, !didCompleteDismissal else { return }
        receivedTerminalCallback = true
        markDismissing()
        if completesDismissal || !appeared {
            completeDismissal()
        }
    }

    func bindingSetterBecameFalse() {
        guard token != nil, !didCompleteDismissal else { return }
        receivedTerminalCallback = true
        markDismissing()
    }

    private func resetForNextCycle() {
        guard token == nil || cycleFinished else { return }
        coordinator = nil
        token = nil
        operationContext = nil
        presentationControllerWitness.reset()
        appeared = false
        didCompleteDismissal = false
        receivedTerminalCallback = false
        cycleFinished = false
    }

    private func updateFinishedState() {
        guard let token else { return }
        cycleFinished = coordinator?.record(for: token)?.released == true
    }
}

/// Immutable bridge from the presentation acceptance boundary to the eventual
/// attachment operation. The Core capability chooses the durable origin while
/// the frozen request token and client keep transport on that same gateway.
struct GaryxPresentationOperationContext {
    let capability: GaryxScopeBoundOperationContext
    let requestToken: GaryxGatewayRequestToken
    let gatewayClient: GaryxGatewayClient
}

struct GaryxPresentationOperationContextProvider {
    let make: @MainActor () -> GaryxPresentationOperationContext?

    init(_ make: @escaping @MainActor () -> GaryxPresentationOperationContext?) {
        self.make = make
    }
}

struct GaryxPresentationResultActions {
    let operationContext: GaryxPresentationOperationContext?
    let recordResult: @MainActor () -> Void
    let recordNoResult: @MainActor () -> Void
}

private struct GaryxPresentationLeaseCoordinatorKey: EnvironmentKey {
    static let defaultValue: GaryxPresentationLeaseCoordinator? = nil
}

private struct GaryxPresentationParentLeaseKey: EnvironmentKey {
    static let defaultValue: GaryxPresentationLeaseToken? = nil
}

private struct GaryxPresentationOperationContextProviderKey: EnvironmentKey {
    static let defaultValue: GaryxPresentationOperationContextProvider? = nil
}

extension EnvironmentValues {
    var garyxPresentationLeaseCoordinator: GaryxPresentationLeaseCoordinator? {
        get { self[GaryxPresentationLeaseCoordinatorKey.self] }
        set { self[GaryxPresentationLeaseCoordinatorKey.self] = newValue }
    }

    var garyxPresentationParentLease: GaryxPresentationLeaseToken? {
        get { self[GaryxPresentationParentLeaseKey.self] }
        set { self[GaryxPresentationParentLeaseKey.self] = newValue }
    }

    var garyxPresentationOperationContextProvider: GaryxPresentationOperationContextProvider? {
        get { self[GaryxPresentationOperationContextProviderKey.self] }
        set { self[GaryxPresentationOperationContextProviderKey.self] = newValue }
    }
}

@MainActor
private protocol GaryxPresentationLeaseModifierSupport {
    var coordinator: GaryxPresentationLeaseCoordinator? { get }
    var parent: GaryxPresentationLeaseToken? { get }
    var operationProvider: GaryxPresentationOperationContextProvider? { get }
    var session: GaryxPresentationLeaseSession { get }
}

@MainActor
private extension GaryxPresentationLeaseModifierSupport {
    func prepare() {
        session.acquireIfNeeded(
            coordinator: coordinator,
            parent: parent,
            operationContext: { operationProvider?.make() }
        )
    }

    func leasedBinding(
        _ binding: Binding<Bool>,
        completesDismissalOnFalse: Bool
    ) -> Binding<Bool> {
        Binding(
            get: { binding.wrappedValue },
            set: { presented in
                if presented {
                    prepare()
                } else {
                    session.bindingSetterBecameFalse()
                }
                binding.wrappedValue = presented
                if !presented, completesDismissalOnFalse {
                    session.completeDismissal()
                }
            }
        )
    }

    func leasedItemBinding<Item>(
        _ binding: Binding<Item?>,
        completesDismissalOnFalse: Bool = false
    ) -> Binding<Item?> {
        Binding(
            get: { binding.wrappedValue },
            set: { item in
                if item != nil {
                    prepare()
                } else {
                    session.bindingSetterBecameFalse()
                }
                binding.wrappedValue = item
                if item == nil, completesDismissalOnFalse {
                    session.completeDismissal()
                }
            }
        )
    }

    func observingLease<Content: View>(
        _ content: Content,
        isPresented: Bool,
        completesDismissalOnFalse: Bool,
        marksPresentedOnTrue: Bool = false
    ) -> some View {
        content.onChange(of: isPresented, initial: true) { _, presented in
            if presented {
                prepare()
                if marksPresentedOnTrue {
                    session.markPresented()
                }
            } else {
                session.bindingBecameFalse(
                    completesDismissal: completesDismissalOnFalse
                )
            }
        }
    }

    func presented<Content: View>(_ content: Content) -> some View {
        content
            .environment(\.garyxPresentationParentLease, session.token)
            .background {
                GaryxPresentationControllerWitnessReader(
                    witness: session.presentationControllerWitness
                )
                .frame(width: 0, height: 0)
                .accessibilityHidden(true)
            }
            .onAppear { session.markPresented() }
            .onDisappear { session.presentedContentDisappeared() }
    }

    var resultActions: GaryxPresentationResultActions {
        GaryxPresentationResultActions(
            operationContext: session.operationContext,
            recordResult: { session.recordResult() },
            recordNoResult: { session.recordNoResult() }
        )
    }
}

private struct GaryxSheetModifier<Presented: View>: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession()
    let isPresented: Binding<Bool>
    let onDismiss: (() -> Void)?
    let presentedContent: () -> Presented

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: isPresented.wrappedValue,
            completesDismissalOnFalse: false
        )
        .sheet(
            isPresented: leasedBinding(isPresented, completesDismissalOnFalse: false),
            onDismiss: {
                session.completeDismissal()
                session.scheduleNoResultIfPending()
                onDismiss?()
            }
        ) {
            presented(presentedContent())
        }
    }
}

private struct GaryxItemSheetModifier<Item: Identifiable, Presented: View>: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession()
    let item: Binding<Item?>
    let onDismiss: (() -> Void)?
    let presentedContent: (Item) -> Presented

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: item.wrappedValue != nil,
            completesDismissalOnFalse: false
        )
        .sheet(
            item: leasedItemBinding(item),
            onDismiss: {
                session.completeDismissal()
                session.scheduleNoResultIfPending()
                onDismiss?()
            }
        ) { value in
            presented(presentedContent(value))
        }
    }
}

/// Registers a custom in-place modal surface with the same route barrier used
/// by system sheets and covers. The view hierarchy stays attached for anchor
/// morphs, while navigation admission and nested presentation ownership keep
/// their existing modal semantics.
private struct GaryxInPlacePresentationBarrierModifier: ViewModifier {
    @Environment(\.garyxPresentationLeaseCoordinator) private var coordinator
    @Environment(\.garyxPresentationParentLease) private var parent
    @Environment(\.garyxPresentationOperationContextProvider) private var operationProvider
    @StateObject private var session = GaryxPresentationLeaseSession()
    @State private var activeLeaseToken: GaryxPresentationLeaseToken?

    let isPresented: Bool

    func body(content: Content) -> some View {
        content
            .environment(
                \.garyxPresentationParentLease,
                isPresented ? (activeLeaseToken ?? parent) : parent
            )
            .onChange(of: isPresented, initial: true) { _, presented in
                if presented {
                    session.acquireIfNeeded(
                        coordinator: coordinator,
                        parent: parent,
                        operationContext: { operationProvider?.make() }
                    )
                    session.markPresented()
                    activeLeaseToken = session.token
                } else {
                    session.bindingBecameFalse(completesDismissal: true)
                    activeLeaseToken = nil
                }
            }
            .onDisappear {
                guard isPresented else { return }
                session.markDismissing()
                session.completeDismissal()
                activeLeaseToken = nil
            }
    }
}

private struct GaryxFullScreenModifier<Presented: View>: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session: GaryxPresentationLeaseSession
    let isPresented: Binding<Bool>
    let onDismiss: (() -> Void)?
    let presentedContent: (GaryxPresentationResultActions) -> Presented

    init(
        isPresented: Binding<Bool>,
        resultBearing: Bool,
        onDismiss: (() -> Void)?,
        presentedContent: @escaping (GaryxPresentationResultActions) -> Presented
    ) {
        self.isPresented = isPresented
        self.onDismiss = onDismiss
        self.presentedContent = presentedContent
        _session = StateObject(
            wrappedValue: GaryxPresentationLeaseSession(resultBearing: resultBearing)
        )
    }

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: isPresented.wrappedValue,
            completesDismissalOnFalse: false
        )
        .fullScreenCover(
            isPresented: leasedBinding(isPresented, completesDismissalOnFalse: false),
            onDismiss: {
                session.completeDismissal()
                session.scheduleNoResultIfPending()
                onDismiss?()
            }
        ) {
            presented(presentedContent(resultActions))
        }
    }
}

private struct GaryxItemFullScreenModifier<Item: Identifiable, Presented: View>: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession()
    let item: Binding<Item?>
    let onDismiss: (() -> Void)?
    let presentedContent: (Item) -> Presented

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: item.wrappedValue != nil,
            completesDismissalOnFalse: false
        )
        .fullScreenCover(
            item: leasedItemBinding(item),
            onDismiss: {
                session.completeDismissal()
                onDismiss?()
            }
        ) { value in
            presented(presentedContent(value))
        }
    }
}

private struct GaryxPopoverModifier<Presented: View>: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession()
    let isPresented: Binding<Bool>
    let attachmentAnchor: PopoverAttachmentAnchor
    let arrowEdge: Edge?
    let presentedContent: () -> Presented

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: isPresented.wrappedValue,
            completesDismissalOnFalse: false
        )
        .popover(
            isPresented: leasedBinding(isPresented, completesDismissalOnFalse: false),
            attachmentAnchor: attachmentAnchor,
            arrowEdge: arrowEdge
        ) {
            presented(presentedContent())
                .onDisappear { session.completeDismissal() }
        }
    }
}

private struct GaryxFileImporterModifier: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession(resultBearing: true)
    let isPresented: Binding<Bool>
    let allowedContentTypes: [UTType]
    let allowsMultipleSelection: Bool
    let onCompletion: (Result<[URL], Error>, GaryxPresentationOperationContext?) -> Void

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: isPresented.wrappedValue,
            completesDismissalOnFalse: true,
            marksPresentedOnTrue: true
        )
        .fileImporter(
            isPresented: leasedBinding(isPresented, completesDismissalOnFalse: true),
            allowedContentTypes: allowedContentTypes,
            allowsMultipleSelection: allowsMultipleSelection
        ) { result in
            switch result {
            case .success:
                session.recordResult()
            case .failure:
                session.recordNoResult()
            }
            onCompletion(result, session.operationContext)
        }
    }
}

private struct GaryxPhotosPickerModifier: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession(resultBearing: true)
    let isPresented: Binding<Bool>
    let selection: Binding<[PhotosPickerItem]>
    let maxSelectionCount: Int?
    let matching: PHPickerFilter?
    let onSelection: ([PhotosPickerItem], GaryxPresentationOperationContext?) -> Void

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: isPresented.wrappedValue,
            completesDismissalOnFalse: true,
            marksPresentedOnTrue: true
        )
        .photosPicker(
            isPresented: leasedBinding(isPresented, completesDismissalOnFalse: true),
            selection: selection,
            maxSelectionCount: maxSelectionCount,
            matching: matching
        )
        .onChange(of: selection.wrappedValue) { _, items in
            guard !items.isEmpty else { return }
            session.recordResult()
            onSelection(items, session.operationContext)
        }
        .onChange(of: isPresented.wrappedValue) { _, presented in
            guard !presented else { return }
            session.scheduleNoResultIfPending()
        }
    }
}

private struct GaryxSinglePhotosPickerModifier: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession(resultBearing: true)
    let isPresented: Binding<Bool>
    let selection: Binding<PhotosPickerItem?>
    let matching: PHPickerFilter?

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: isPresented.wrappedValue,
            completesDismissalOnFalse: true,
            marksPresentedOnTrue: true
        )
        .photosPicker(
            isPresented: leasedBinding(isPresented, completesDismissalOnFalse: true),
            selection: selection,
            matching: matching
        )
        .onChange(of: selection.wrappedValue) { _, item in
            guard item != nil else { return }
            session.recordResult()
        }
        .onChange(of: isPresented.wrappedValue) { _, presented in
            guard !presented else { return }
            session.scheduleNoResultIfPending()
        }
    }
}

private struct GaryxConfirmationDialogModifier<Actions: View, Message: View>: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession()
    let title: LocalizedStringKey
    let isPresented: Binding<Bool>
    let titleVisibility: Visibility
    let actions: () -> Actions
    let message: () -> Message

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: isPresented.wrappedValue,
            completesDismissalOnFalse: true,
            marksPresentedOnTrue: true
        )
        .confirmationDialog(
            title,
            isPresented: leasedBinding(isPresented, completesDismissalOnFalse: true),
            titleVisibility: titleVisibility,
            actions: actions,
            message: message
        )
    }
}

private struct GaryxAlertModifier<Actions: View, Message: View>: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession()
    let title: LocalizedStringKey
    let isPresented: Binding<Bool>
    let actions: () -> Actions
    let message: () -> Message

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: isPresented.wrappedValue,
            completesDismissalOnFalse: true,
            marksPresentedOnTrue: true
        )
        .alert(
            title,
            isPresented: leasedBinding(isPresented, completesDismissalOnFalse: true),
            actions: actions,
            message: message
        )
    }
}

private struct GaryxItemAlertModifier<Item: Identifiable>: ViewModifier,
    GaryxPresentationLeaseModifierSupport {
    @Environment(\.garyxPresentationLeaseCoordinator) var coordinator
    @Environment(\.garyxPresentationParentLease) var parent
    @Environment(\.garyxPresentationOperationContextProvider) var operationProvider
    @StateObject var session = GaryxPresentationLeaseSession()
    let item: Binding<Item?>
    let alert: (Item) -> Alert

    func body(content: Content) -> some View {
        observingLease(
            content,
            isPresented: item.wrappedValue != nil,
            completesDismissalOnFalse: true,
            marksPresentedOnTrue: true
        )
        .alert(
            item: leasedItemBinding(
                item,
                completesDismissalOnFalse: true
            )
        ) { value in alert(value) }
    }
}

extension View {
    func garyxInPlacePresentationBarrier(isPresented: Bool) -> some View {
        modifier(GaryxInPlacePresentationBarrierModifier(isPresented: isPresented))
    }

    func garyxSheet<Presented: View>(
        isPresented: Binding<Bool>,
        onDismiss: (() -> Void)? = nil,
        @ViewBuilder content: @escaping () -> Presented
    ) -> some View {
        modifier(
            GaryxSheetModifier(
                isPresented: isPresented,
                onDismiss: onDismiss,
                presentedContent: content
            )
        )
    }

    func garyxSheet<Item: Identifiable, Presented: View>(
        item: Binding<Item?>,
        onDismiss: (() -> Void)? = nil,
        @ViewBuilder content: @escaping (Item) -> Presented
    ) -> some View {
        modifier(
            GaryxItemSheetModifier(
                item: item,
                onDismiss: onDismiss,
                presentedContent: content
            )
        )
    }

    func garyxFullScreenCover<Presented: View>(
        isPresented: Binding<Bool>,
        onDismiss: (() -> Void)? = nil,
        @ViewBuilder content: @escaping () -> Presented
    ) -> some View {
        modifier(
            GaryxFullScreenModifier(
                isPresented: isPresented,
                resultBearing: false,
                onDismiss: onDismiss,
                presentedContent: { _ in content() }
            )
        )
    }

    func garyxResultFullScreenCover<Presented: View>(
        isPresented: Binding<Bool>,
        onDismiss: (() -> Void)? = nil,
        @ViewBuilder content: @escaping (GaryxPresentationResultActions) -> Presented
    ) -> some View {
        modifier(
            GaryxFullScreenModifier(
                isPresented: isPresented,
                resultBearing: true,
                onDismiss: onDismiss,
                presentedContent: content
            )
        )
    }

    func garyxFullScreenCover<Item: Identifiable, Presented: View>(
        item: Binding<Item?>,
        onDismiss: (() -> Void)? = nil,
        @ViewBuilder content: @escaping (Item) -> Presented
    ) -> some View {
        modifier(
            GaryxItemFullScreenModifier(
                item: item,
                onDismiss: onDismiss,
                presentedContent: content
            )
        )
    }

    func garyxPopover<Presented: View>(
        isPresented: Binding<Bool>,
        attachmentAnchor: PopoverAttachmentAnchor = .rect(.bounds),
        arrowEdge: Edge? = nil,
        @ViewBuilder content: @escaping () -> Presented
    ) -> some View {
        modifier(
            GaryxPopoverModifier(
                isPresented: isPresented,
                attachmentAnchor: attachmentAnchor,
                arrowEdge: arrowEdge,
                presentedContent: content
            )
        )
    }

    func garyxFileImporter(
        isPresented: Binding<Bool>,
        allowedContentTypes: [UTType],
        allowsMultipleSelection: Bool = false,
        onCompletion: @escaping (Result<[URL], Error>) -> Void
    ) -> some View {
        modifier(
            GaryxFileImporterModifier(
                isPresented: isPresented,
                allowedContentTypes: allowedContentTypes,
                allowsMultipleSelection: allowsMultipleSelection,
                onCompletion: { result, _ in onCompletion(result) }
            )
        )
    }

    func garyxResultFileImporter(
        isPresented: Binding<Bool>,
        allowedContentTypes: [UTType],
        allowsMultipleSelection: Bool = false,
        onCompletion: @escaping (
            Result<[URL], Error>,
            GaryxPresentationOperationContext?
        ) -> Void
    ) -> some View {
        modifier(
            GaryxFileImporterModifier(
                isPresented: isPresented,
                allowedContentTypes: allowedContentTypes,
                allowsMultipleSelection: allowsMultipleSelection,
                onCompletion: onCompletion
            )
        )
    }

    func garyxPhotosPicker(
        isPresented: Binding<Bool>,
        selection: Binding<[PhotosPickerItem]>,
        maxSelectionCount: Int? = nil,
        matching: PHPickerFilter? = nil,
        onSelection: @escaping (
            [PhotosPickerItem],
            GaryxPresentationOperationContext?
        ) -> Void = { _, _ in }
    ) -> some View {
        modifier(
            GaryxPhotosPickerModifier(
                isPresented: isPresented,
                selection: selection,
                maxSelectionCount: maxSelectionCount,
                matching: matching,
                onSelection: onSelection
            )
        )
    }

    func garyxPhotosPicker(
        isPresented: Binding<Bool>,
        selection: Binding<PhotosPickerItem?>,
        matching: PHPickerFilter? = nil
    ) -> some View {
        modifier(
            GaryxSinglePhotosPickerModifier(
                isPresented: isPresented,
                selection: selection,
                matching: matching
            )
        )
    }

    func garyxConfirmationDialog<Actions: View, Message: View>(
        _ title: LocalizedStringKey,
        isPresented: Binding<Bool>,
        titleVisibility: Visibility = .automatic,
        @ViewBuilder actions: @escaping () -> Actions,
        @ViewBuilder message: @escaping () -> Message
    ) -> some View {
        modifier(
            GaryxConfirmationDialogModifier(
                title: title,
                isPresented: isPresented,
                titleVisibility: titleVisibility,
                actions: actions,
                message: message
            )
        )
    }

    func garyxAlert<Actions: View, Message: View>(
        _ title: LocalizedStringKey,
        isPresented: Binding<Bool>,
        @ViewBuilder actions: @escaping () -> Actions,
        @ViewBuilder message: @escaping () -> Message
    ) -> some View {
        modifier(
            GaryxAlertModifier(
                title: title,
                isPresented: isPresented,
                actions: actions,
                message: message
            )
        )
    }

    func garyxAlert<Actions: View>(
        _ title: LocalizedStringKey,
        isPresented: Binding<Bool>,
        @ViewBuilder actions: @escaping () -> Actions
    ) -> some View {
        garyxAlert(
            title,
            isPresented: isPresented,
            actions: actions,
            message: { EmptyView() }
        )
    }

    func garyxAlert<Item: Identifiable>(
        item: Binding<Item?>,
        content: @escaping (Item) -> Alert
    ) -> some View {
        modifier(GaryxItemAlertModifier(item: item, alert: content))
    }
}
