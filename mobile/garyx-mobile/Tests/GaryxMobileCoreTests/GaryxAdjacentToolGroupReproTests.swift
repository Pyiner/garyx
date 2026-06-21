import XCTest
@testable import GaryxMobileCore

/// TASK-1021 — "two adjacent collapsed tool rows" reproduction + fix coverage
/// (client side).
///
/// The Rust reducer always emits a CORRECT render snapshot: the interstitial
/// assistant message sits between the two tool groups as an `.assistantMessage`
/// step, so the snapshot itself NEVER has adjacent tool groups (proved in
/// `garyx-models/tests/transcript_tool_group_adjacency_repro.rs`).
///
/// The bug was on the client: when the interstitial assistant's body was missing
/// from the local `messages` lookup, `GaryxMobileRenderStateMapper` dropped the
/// whole assistant step (`compactMap` -> `nil`) and the two tool groups rendered
/// back to back — the reported `[Used fileChange][Used fileChange]` symptom with
/// the middle message gone. Tool groups never vanish (generic fallback), so the
/// asymmetry was the failure point.
///
/// Fix: the mapper now renders a body-less placeholder for an unresolved
/// assistant step (mirroring the tool-group fallback), preserving the
/// server-owned structure. The placeholder shares the committed body's id, so it
/// upgrades in place when the body arrives.
///
/// Synthetic seqs mirror the real failing sequence (no real data):
///   seq 2   user
///   seq 3   assistant  "Segment one"     (analog of real seq 415)
///   seq 4/5 tool group A (call_a)
///   seq 7   assistant  "Segment two"     (analog of real seq 419 — used to vanish)
///   seq 8/9 tool group B (call_b)
///   seq 11  assistant  "Segment three"   (analog of real seq 423)
///   seq 12/13 tool group C (call_c)
final class GaryxAdjacentToolGroupReproTests: XCTestCase {
    private let originId = "00000000-0000-0000-0000-000000000001"
    private let interstitialText = "Segment two — resolving destinations."

    /// The structure the server snapshot defines, regardless of which assistant
    /// bodies are locally present: message, tool, message, tool, message, tool.
    private let expectedKinds = ["message", "tool", "message", "tool", "message", "tool"]

    /// RED before the fix / GREEN after: with the interstitial assistant body
    /// absent from `messages` (the runtime window where the synchronously-updated
    /// `renderSnapshotsByThread` references seq:7 before the throttled `messages`
    /// flush ingests its body), the mapper must still preserve the server
    /// structure — the two tool groups must NOT render adjacent, and the
    /// interstitial slot must remain present (as a placeholder).
    func test_interstitialAssistantMissing_keepsServerStructureWithPlaceholder() throws {
        let steps = try agentTurnSteps(GaryxMobileRenderStateMapper.rows(
            snapshot: correctServerSnapshot(),
            messages: assistantMessages(includeInterstitial: false),
            transcriptMessages: toolTranscriptMessages()
        ))

        XCTAssertFalse(
            hasAdjacentToolGroups(steps),
            "two collapsed tool rows must not render adjacent; steps=\(steps.map(blockKind))"
        )
        XCTAssertEqual(
            steps.map(blockKind), expectedKinds,
            "the interstitial assistant slot must survive as a placeholder; steps=\(steps.map(blockKind))"
        )

        // The placeholder is a body-less, loading-state assistant row.
        let placeholder = try XCTUnwrap(assistantBlock(steps, at: 2), "missing interstitial slot")
        XCTAssertEqual(placeholder.role, .assistant)
        XCTAssertEqual(placeholder.text, "")
        XCTAssertTrue(placeholder.isStreaming, "placeholder should render a loading state, not an empty bubble")
    }

    /// GREEN control: the SAME correct snapshot maps fine when the interstitial
    /// assistant body is present.
    func test_interstitialAssistantPresent_keepsToolGroupsSeparated() throws {
        let steps = try agentTurnSteps(GaryxMobileRenderStateMapper.rows(
            snapshot: correctServerSnapshot(),
            messages: assistantMessages(includeInterstitial: true),
            transcriptMessages: toolTranscriptMessages()
        ))

        XCTAssertEqual(steps.map(blockKind), expectedKinds, "steps=\(steps.map(blockKind))")
        let body = try XCTUnwrap(assistantBlock(steps, at: 2))
        XCTAssertEqual(body.text, interstitialText)
    }

    /// The placeholder upgrades to the real body IN PLACE when it arrives: same
    /// row id across the transition (no flicker / no re-insert), never adjacent,
    /// and the final body text shows. This locks the placeholder-id == body-id
    /// invariant that "smooth replace" depends on.
    func test_assistantPlaceholderUpgradesToBodyInPlace_whenInterstitialArrives() throws {
        let snapshot = correctServerSnapshot()
        let tools = toolTranscriptMessages()

        // Frame 1: snapshot references seq:7 but its body has not arrived yet.
        let before = try agentTurnSteps(GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: assistantMessages(includeInterstitial: false),
            transcriptMessages: tools
        ))
        XCTAssertFalse(hasAdjacentToolGroups(before), "frame 1 must not be adjacent; steps=\(before.map(blockKind))")
        let placeholder = try XCTUnwrap(assistantBlock(before, at: 2), "frame 1 missing interstitial slot")
        XCTAssertEqual(placeholder.text, "")
        let placeholderId = placeholder.id

        // Frame 2: the committed body lands in `messages`.
        let after = try agentTurnSteps(GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: assistantMessages(includeInterstitial: true),
            transcriptMessages: tools
        ))
        XCTAssertFalse(hasAdjacentToolGroups(after), "frame 2 must not be adjacent; steps=\(after.map(blockKind))")
        let body = try XCTUnwrap(assistantBlock(after, at: 2), "frame 2 missing interstitial slot")

        XCTAssertEqual(body.id, placeholderId, "row id must be stable across placeholder -> body upgrade")
        XCTAssertEqual(body.text, interstitialText, "final body text must show after the body arrives")
    }

    // MARK: - fixture

    private func correctServerSnapshot() -> GaryxRenderSnapshot {
        GaryxRenderSnapshot(
            basedOnSeq: 13,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "user_turn:origin:\(originId)",
                    user: ref(seq: 2, role: "user", id: "origin:\(originId)"),
                    activity: [
                        .step(GaryxRenderStepRow(
                            id: "step:assistant_step:seq:3",
                            steps: [
                                assistantStep(seq: 3),
                                toolGroupStep(call: "call_a", useSeq: 4, resultSeq: 5),
                                assistantStep(seq: 7), // interstitial — analog of real 419
                                toolGroupStep(call: "call_b", useSeq: 8, resultSeq: 9),
                                assistantStep(seq: 11),
                                toolGroupStep(call: "call_c", useSeq: 12, resultSeq: 13),
                            ],
                            running: false
                        )),
                    ]
                )),
            ]
        )
    }

    private func toolTranscriptMessages() -> [GaryxTranscriptMessage] {
        [
            toolUse(index: 3, toolUseId: "call_a"),
            toolResult(index: 4, toolUseId: "call_a"),
            toolUse(index: 7, toolUseId: "call_b"),
            toolResult(index: 8, toolUseId: "call_b"),
            toolUse(index: 11, toolUseId: "call_c"),
            toolResult(index: 12, toolUseId: "call_c"),
        ]
    }

    /// Assistant/user bodies in the global `messages` list, keyed by
    /// `historyIndex == seq - 1`. The interstitial (seq 7) is omitted in the bug
    /// case to model the snapshot-ahead-of-messages window.
    private func assistantMessages(includeInterstitial: Bool) -> [GaryxMobileMessage] {
        var messages = [
            userMessage(seq: 2),
            assistantMessage(seq: 3, text: "Segment one — adding the model."),
            assistantMessage(seq: 11, text: "Segment three — wiring the panel."),
        ]
        if includeInterstitial {
            messages.append(assistantMessage(seq: 7, text: interstitialText))
        }
        return messages
    }

    // MARK: - assertions / helpers

    private func agentTurnSteps(_ rows: [GaryxMobileTurnRow]) throws -> [GaryxMobileTranscriptBlock] {
        let row = try XCTUnwrap(rows.first, "expected one turn row, got \(rows.count)")
        let activity = try XCTUnwrap(row.activityRows.first, "expected an activity row")
        guard case .turn(let turn) = activity else {
            XCTFail("expected an agent turn, got \(activity)")
            return []
        }
        return turn.steps
    }

    private func blockKind(_ block: GaryxMobileTranscriptBlock) -> String {
        switch block {
        case .message: "message"
        case .toolGroup: "tool"
        }
    }

    private func assistantBlock(_ steps: [GaryxMobileTranscriptBlock], at index: Int) -> GaryxMobileMessage? {
        guard steps.indices.contains(index), case .message(let message) = steps[index] else { return nil }
        return message
    }

    private func hasAdjacentToolGroups(_ steps: [GaryxMobileTranscriptBlock]) -> Bool {
        for index in steps.indices.dropLast() {
            if case .toolGroup = steps[index], case .toolGroup = steps[index + 1] {
                return true
            }
        }
        return false
    }

    private func ref(seq: Int, role: String, id: String? = nil) -> GaryxRenderMessageRef {
        GaryxRenderMessageRef(id: id ?? "seq:\(seq)", seq: seq, role: role)
    }

    private func assistantStep(seq: Int) -> GaryxRenderStepItem {
        .assistantMessage(GaryxRenderAssistantStep(
            id: "assistant_step:seq:\(seq)",
            message: ref(seq: seq, role: "assistant")
        ))
    }

    private func toolGroupStep(call: String, useSeq: Int, resultSeq: Int) -> GaryxRenderStepItem {
        .toolGroup(GaryxRenderToolGroup(
            id: "tool_group:\(call)",
            status: .completed,
            entries: [
                GaryxRenderToolEntry(
                    id: "tool_entry:\(call)",
                    toolUseId: call,
                    status: .completed,
                    toolUse: ref(seq: useSeq, role: "tool_use"),
                    toolResult: ref(seq: resultSeq, role: "tool_result")
                ),
            ]
        ))
    }

    private func userMessage(seq: Int) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: "origin:\(originId)",
            role: .user,
            text: "Implement the projection.",
            timestamp: nil,
            isStreaming: false,
            localState: .remoteFinal,
            historyIndex: seq - 1
        )
    }

    private func assistantMessage(seq: Int, text: String) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: "history:\(seq - 1)",
            role: .assistant,
            text: text,
            timestamp: nil,
            isStreaming: false,
            localState: .remoteFinal,
            historyIndex: seq - 1
        )
    }

    private func toolUse(index: Int, toolUseId: String) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(
            index: index,
            role: .toolUse,
            content: json(#"{"tool":"fileChange","input":{"file_path":"/tmp/x"}}"#),
            toolUseId: toolUseId
        )
    }

    private func toolResult(index: Int, toolUseId: String) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(
            index: index,
            role: .toolResult,
            content: json(#"{"result":{"stdout":"ok"}}"#),
            toolUseId: toolUseId
        )
    }

    private func json(_ raw: String) -> GaryxJSONValue {
        try! JSONDecoder().decode(GaryxJSONValue.self, from: Data(raw.utf8))
    }
}
