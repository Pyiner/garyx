import Foundation

/// Per-thread composer drafts. The composer is one text field reused across
/// threads; this store binds the unsent text to the context it was typed in, so
/// switching threads no longer discards what the other thread had in progress.
///
/// Pure value type so the rule lives in `GaryxMobileCore` with tests; the model
/// holds one instance and drives the field through `composerContextVersion`.
public struct GaryxComposerDraftStore: Equatable {
    /// Context key for the new-thread composer (no thread selected yet).
    public static let newThreadKey = "__new_thread__"

    public private(set) var drafts: [String: String]
    /// The context the live composer text currently belongs to.
    public private(set) var activeKey: String

    public init(
        activeKey: String = GaryxComposerDraftStore.newThreadKey,
        drafts: [String: String] = [:]
    ) {
        self.activeKey = activeKey
        self.drafts = drafts
    }

    /// The saved draft for the active context (empty when none).
    public var current: String {
        drafts[activeKey] ?? ""
    }

    /// Persist the live text for the active context. Called on every edit, so an
    /// empty value drops the entry rather than storing "".
    public mutating func setCurrent(_ text: String) {
        if text.isEmpty {
            drafts.removeValue(forKey: activeKey)
        } else {
            drafts[activeKey] = text
        }
    }

    /// Switch the active context, preserving every other context's draft.
    /// Returns `true` when the active key actually changed, i.e. the caller
    /// should reload the field from `current`.
    @discardableResult
    public mutating func switchTo(_ key: String) -> Bool {
        guard key != activeKey else { return false }
        activeKey = key
        return true
    }

    /// Clear only the active context's draft — used after a successful send.
    public mutating func reset() {
        drafts.removeValue(forKey: activeKey)
    }

    /// Drop a thread's draft entirely (the thread was deleted or unbound).
    /// Returns `true` when the dropped thread was active and the field must
    /// reload (it falls back to the new-thread context).
    @discardableResult
    public mutating func discard(threadId: String) -> Bool {
        drafts.removeValue(forKey: threadId)
        guard activeKey == threadId else { return false }
        activeKey = Self.newThreadKey
        return true
    }

    /// Drop every draft — used when the whole context changes (gateway switch).
    public mutating func clearAll() {
        drafts.removeAll()
        activeKey = Self.newThreadKey
    }
}
