import SwiftUI
import UIKit

struct GaryxComposerAdapterCloseSnapshot: Equatable {
    let finalSequence: UInt64
    let text: String
    let pendingProducers: Set<GaryxInputProducerKind>
    let wasFocused: Bool
}

@MainActor
protocol GaryxComposerInputAdapter: AnyObject {
    var occurrenceID: GaryxRouteInstanceID { get }
    var composerKey: GaryxComposerKey { get }
    var isLive: Bool { get }
    var isInputReady: Bool { get }
    func grantLive(_ configuration: GaryxComposerInputConfiguration)
    func makeReadOnly()
    func requestFocus()
    func replaceLiveText(_ text: String)
    func finalizeInput() -> GaryxComposerAdapterCloseSnapshot
}

@MainActor
final class GaryxComposerProducerRegistry {
    private(set) var active: Set<GaryxInputProducerKind> = []

    func contains(_ producer: GaryxInputProducerKind) -> Bool {
        active.contains(producer)
    }

    func began(_ producer: GaryxInputProducerKind) {
        active.insert(producer)
    }

    @discardableResult
    func reachedTerminal(_ producer: GaryxInputProducerKind) -> Bool {
        active.remove(producer) != nil
    }
}

@MainActor
final class GaryxComposerOrderedTextView: UITextView, GaryxComposerInputAdapter {
    let occurrenceID: GaryxRouteInstanceID
    private(set) var composerKey: GaryxComposerKey
    private(set) var isLive = false

    var onOrderedText: ((String, GaryxComposerInputEventIdentity) -> Void)?
    var onProducerTerminal: ((GaryxInputProducerKind) -> Void)?
    var onFocusChanged: ((Bool) -> Void)?
    var onSubmit: (() -> Void)?

    private(set) var inputConfiguration: GaryxComposerInputConfiguration?
    var isInputReady: Bool { inputConfiguration?.isReadOnly == false }
    private let producers = GaryxComposerProducerRegistry()
    private var nextSequence: UInt64 = 1
    private var lastPublishedText = ""
    private var observedMarkedText = false
    private var isFinalizing = false

    init(occurrenceID: GaryxRouteInstanceID, composerKey: GaryxComposerKey) {
        self.occurrenceID = occurrenceID
        self.composerKey = composerKey
        super.init(frame: .zero, textContainer: nil)
        backgroundColor = .clear
        textContainerInset = .zero
        textContainer.lineFragmentPadding = 0
        font = UIFont.preferredFont(forTextStyle: .subheadline)
        adjustsFontForContentSizeCategory = true
        autocorrectionType = .default
        smartDashesType = .default
        smartQuotesType = .default
        returnKeyType = .send
        keyboardDismissMode = .interactive
        accessibilityIdentifier = "garyx-composer-uikit-input"
        accessibilityTraits.insert(.notEnabled)
        updateDebugAccessibilityState()
        addInteraction(UIScribbleInteraction(delegate: self))
    }

    required init?(coder: NSCoder) {
        nil
    }

    func grantLive(_ configuration: GaryxComposerInputConfiguration) {
        let startsNewSession = inputConfiguration?.sessionID != configuration.sessionID
            || inputConfiguration?.epoch != configuration.epoch
        composerKey = configuration.composerKey
        inputConfiguration = configuration
        isLive = !configuration.isReadOnly
        isFinalizing = false
        if startsNewSession {
            nextSequence = configuration.nextInputSequence
            observedMarkedText = false
        } else {
            // A route payload rebind may arrive after the reducer has already
            // admitted an event from this session. Never move the adapter's
            // sequence cursor backwards.
            nextSequence = max(nextSequence, configuration.nextInputSequence)
        }
        if text != configuration.initialText {
            text = configuration.initialText
        }
        lastPublishedText = text
        isEditable = isLive
        isSelectable = true
        if isLive {
            accessibilityTraits.remove(.notEnabled)
        } else {
            accessibilityTraits.insert(.notEnabled)
        }
        updateDebugAccessibilityState()
    }

    func makeReadOnly() {
        isLive = false
        isEditable = false
        accessibilityTraits.insert(.notEnabled)
        updateDebugAccessibilityState()
        if isFirstResponder {
            resignFirstResponder()
        }
    }

    func requestFocus() {
        guard isLive, !isFinalizing else { return }
        if window != nil {
            becomeFirstResponder()
        } else {
            DispatchQueue.main.async { [weak self] in
                guard let self, self.window != nil, self.isLive, !self.isFinalizing else { return }
                self.becomeFirstResponder()
            }
        }
    }

    private func updateDebugAccessibilityState() {
        #if DEBUG
        guard ProcessInfo.processInfo.environment["GARYX_MOBILE_PRODUCTION_ROUTE_DIAGNOSTICS"] == "1"
        else { return }
        accessibilityLabel = isLive ? "composer-live" : "composer-read-only"
        #endif
    }

    func replaceLiveText(_ text: String) {
        guard isLive, !isFinalizing else { return }
        self.text = text
        publishCurrentText(force: true)
    }

    func applyTextContainerInsets(_ insets: UIEdgeInsets) {
        guard textContainerInset != insets else { return }
        textContainerInset = insets
        invalidateIntrinsicContentSize()
        setNeedsLayout()
    }

    /// Main-actor critical section used by route commit-release:
    /// freeze admission, unmark synchronously, publish the exact resulting
    /// sequence, then resign focus. Async dictation/scribble producers keep
    /// their lease and may report terminal after this method returns.
    func finalizeInput() -> GaryxComposerAdapterCloseSnapshot {
        let wasFocused = isFirstResponder
        guard !isFinalizing else {
            return GaryxComposerAdapterCloseSnapshot(
                finalSequence: nextSequence &- 1,
                text: text,
                pendingProducers: producers.active,
                wasFocused: wasFocused
            )
        }
        isFinalizing = true
        isLive = false
        accessibilityTraits.insert(.notEnabled)
        updateDebugAccessibilityState()
        if markedTextRange != nil {
            producers.began(.markedText)
            unmarkText()
            publishCurrentText(force: true)
            if producers.reachedTerminal(.markedText) {
                onProducerTerminal?(.markedText)
            }
        } else {
            publishCurrentText(force: true)
        }
        isEditable = false
        resignFirstResponder()
        return GaryxComposerAdapterCloseSnapshot(
            finalSequence: nextSequence &- 1,
            text: text,
            pendingProducers: producers.active,
            wasFocused: wasFocused
        )
    }

    func observedTextDidChange() {
        let hasMarkedText = markedTextRange != nil
        if hasMarkedText {
            producers.began(.markedText)
        }
        publishCurrentText(force: false)
        if observedMarkedText, !hasMarkedText,
           producers.reachedTerminal(.markedText) {
            onProducerTerminal?(.markedText)
        }
        observedMarkedText = hasMarkedText
    }

    override func insertDictationResult(_ dictationResult: [UIDictationPhrase]) {
        guard !isFinalizing || producers.contains(.dictation) else { return }
        if !isFinalizing {
            producers.began(.dictation)
        }
        super.insertDictationResult(dictationResult)
        publishCurrentText(force: true)
        if producers.reachedTerminal(.dictation) {
            onProducerTerminal?(.dictation)
        }
    }

    override func dictationRecordingDidEnd() {
        super.dictationRecordingDidEnd()
        // Recording completion begins the asynchronous recognition window.
        // The producer reaches terminal only when UIKit supplies a result or
        // reports recognition failure.
        if !isFinalizing {
            producers.began(.dictation)
        }
    }

    func beginDictationRecognitionForTesting() {
        guard !isFinalizing else { return }
        producers.began(.dictation)
    }

    func failDictationRecognitionForTesting() {
        if producers.reachedTerminal(.dictation) {
            onProducerTerminal?(.dictation)
        }
    }

    override func dictationRecognitionFailed() {
        super.dictationRecognitionFailed()
        if producers.reachedTerminal(.dictation) {
            onProducerTerminal?(.dictation)
        }
    }

    /// Mirrors UIKit's result boundary without manufacturing private
    /// UIDictationPhrase instances. App-target tests use this to exercise the
    /// result-before-terminal branch against the real UITextView adapter.
    func acceptRecognizedDictationTextForTesting(_ recognizedText: String) {
        guard !isFinalizing || producers.contains(.dictation) else { return }
        text = recognizedText
        publishCurrentText(force: true)
        if producers.reachedTerminal(.dictation) {
            onProducerTerminal?(.dictation)
        }
    }

    private func publishCurrentText(force: Bool) {
        guard let configuration = inputConfiguration else { return }
        guard force || text != lastPublishedText else { return }
        let sequence = nextSequence
        nextSequence &+= 1
        lastPublishedText = text
        onOrderedText?(
            text,
            GaryxComposerInputEventIdentity(
                composerKey: configuration.composerKey,
                sessionID: configuration.sessionID,
                inputSessionEpoch: configuration.epoch,
                payloadGeneration: configuration.payloadGeneration,
                reservationID: configuration.reservationID,
                inputSequence: sequence
            )
        )
    }
}

extension GaryxComposerOrderedTextView: UIScribbleInteractionDelegate {
    func scribbleInteractionWillBeginWriting(_ interaction: UIScribbleInteraction) {
        producers.began(.scribble)
    }

    func scribbleInteractionDidFinishWriting(_ interaction: UIScribbleInteraction) {
        publishCurrentText(force: true)
        if producers.reachedTerminal(.scribble) {
            onProducerTerminal?(.scribble)
        }
    }

    func scribbleInteraction(
        _ interaction: UIScribbleInteraction,
        shouldBeginAt location: CGPoint
    ) -> Bool {
        inputConfiguration?.isReadOnly == false && !isFinalizing
    }
}

struct GaryxComposerTextLayout {
    let textContainerInsets: UIEdgeInsets
    let minimumTextHeight: CGFloat
    let maximumLineCount: Int

    var minimumControlHeight: CGFloat {
        minimumTextHeight + textContainerInsets.top + textContainerInsets.bottom
    }

    func controlHeight(fittedHeight: CGFloat, lineHeight: CGFloat) -> CGFloat {
        let verticalInsets = textContainerInsets.top + textContainerInsets.bottom
        let fittedTextHeight = max(0, fittedHeight - verticalInsets)
        let minimum = max(minimumTextHeight, lineHeight)
        let maximum = max(minimum, lineHeight * CGFloat(maximumLineCount))
        return min(max(fittedTextHeight, minimum), maximum) + verticalInsets
    }
}

struct GaryxComposerUIKitField: UIViewRepresentable {
    let occurrenceID: GaryxRouteInstanceID
    let configuration: GaryxComposerInputConfiguration
    let layout: GaryxComposerTextLayout
    let isFocused: FocusState<Bool>.Binding
    let onRegister: @MainActor (GaryxComposerInputAdapter) -> Void
    let onUnregister: @MainActor (GaryxComposerInputAdapter) -> Void
    let onOrderedText: @MainActor (String, GaryxComposerInputEventIdentity) -> Void
    let onProducerTerminal: @MainActor (GaryxInputProducerKind) -> Void
    let onSubmit: @MainActor () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIView(context: Context) -> GaryxComposerOrderedTextView {
        let view = GaryxComposerOrderedTextView(
            occurrenceID: occurrenceID,
            composerKey: configuration.composerKey
        )
        view.delegate = context.coordinator
        view.applyTextContainerInsets(layout.textContainerInsets)
        context.coordinator.installCallbacks(on: view)
        view.grantLive(configuration)
        onRegister(view)
        requestDebugFocusIfNeeded(on: view)
        return view
    }

    func updateUIView(_ view: GaryxComposerOrderedTextView, context: Context) {
        context.coordinator.parent = self
        view.applyTextContainerInsets(layout.textContainerInsets)
        context.coordinator.installCallbacks(on: view)
        if view.inputConfiguration != configuration {
            view.grantLive(configuration)
        } else if configuration.isReadOnly {
            view.makeReadOnly()
        }
        // Route lifecycle/top ownership arrives through the updated SwiftUI
        // environment while the UIView identity remains stable.
        context.coordinator.parent.onRegister(view)
        requestDebugFocusIfNeeded(on: view)
        if isFocused.wrappedValue,
           !configuration.isReadOnly,
           !view.isFirstResponder {
            DispatchQueue.main.async {
                guard view.window != nil else { return }
                view.becomeFirstResponder()
            }
        }
    }

    static func dismantleUIView(
        _ view: GaryxComposerOrderedTextView,
        coordinator: Coordinator
    ) {
        coordinator.parent.onUnregister(view)
        view.makeReadOnly()
    }

    private func requestDebugFocusIfNeeded(on view: GaryxComposerOrderedTextView) {
        #if DEBUG
        guard ProcessInfo.processInfo.environment["GARYX_MOBILE_PRODUCTION_ROUTE_AUTO_FOCUS"] == "1"
        else { return }
        DispatchQueue.main.async { [weak view] in
            view?.requestFocus()
        }
        #endif
    }

    func sizeThatFits(
        _ proposal: ProposedViewSize,
        uiView: GaryxComposerOrderedTextView,
        context: Context
    ) -> CGSize? {
        let width = proposal.width ?? uiView.bounds.width
        guard width > 0 else { return nil }
        let fitted = uiView.sizeThatFits(
            CGSize(width: width, height: .greatestFiniteMagnitude)
        )
        let lineHeight = uiView.font?.lineHeight ?? 20
        let height = layout.controlHeight(
            fittedHeight: fitted.height,
            lineHeight: lineHeight
        )
        uiView.isScrollEnabled = fitted.height > height
        return CGSize(width: width, height: height)
    }

    @MainActor
    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: GaryxComposerUIKitField

        init(parent: GaryxComposerUIKitField) {
            self.parent = parent
        }

        func installCallbacks(on view: GaryxComposerOrderedTextView) {
            view.onOrderedText = parent.onOrderedText
            view.onProducerTerminal = parent.onProducerTerminal
            view.onSubmit = parent.onSubmit
            view.onFocusChanged = { [weak self] focused in
                DispatchQueue.main.async { [weak self] in
                    guard let self,
                          self.parent.isFocused.wrappedValue != focused else { return }
                    self.parent.isFocused.wrappedValue = focused
                }
            }
        }

        func textViewDidBeginEditing(_ textView: UITextView) {
            (textView as? GaryxComposerOrderedTextView)?.onFocusChanged?(true)
        }

        func textViewDidEndEditing(_ textView: UITextView) {
            (textView as? GaryxComposerOrderedTextView)?.onFocusChanged?(false)
        }

        func textViewDidChange(_ textView: UITextView) {
            (textView as? GaryxComposerOrderedTextView)?.observedTextDidChange()
        }

        func textView(
            _ textView: UITextView,
            shouldChangeTextIn range: NSRange,
            replacementText text: String
        ) -> Bool {
            guard text == "\n",
                  textView.markedTextRange == nil else { return true }
            parent.onSubmit()
            return false
        }
    }
}
