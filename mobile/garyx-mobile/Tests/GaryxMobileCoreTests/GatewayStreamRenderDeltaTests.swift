import XCTest
@testable import GaryxMobileCore

/// #TASK-1956 batch 3: `render_mode=delta` reassembly in
/// `GatewayStreamFrameProcessor` (the transport layer that owns the
/// `.gap(resumeAfterSeq:)` exit). The emitted action stream must always
/// carry full snapshots — the mapper, `renderEquivalent`, and the flush
/// gate never learn deltas exist. Frame fixtures mirror the gateway's
/// routes/tests.rs delta shapes; chain tokens are opaque decimal strings
/// the client compares only by equality.
final class GatewayStreamRenderDeltaTests: XCTestCase {
    private let threadId = "thread-delta"

    // MARK: - Reassembly equivalence

    func testDeltaFramesReassembleToFullSnapshotsAcrossAChain() {
        var processor = GatewayStreamFrameProcessor()

        // Full replay frame seeds the base (chain token "1001").
        let seed = processor.processPayload(
            fullFramePayload(
                basedOnSeq: 2,
                rows: [userTurnRowJSON(id: "turn-1", userSeq: 1)],
                rowsHash: "1001",
                events: [event(seq: 1, role: "user", text: "ask"),
                         event(seq: 2, role: "assistant", text: "reply")]
            ),
            threadId: threadId
        )
        XCTAssertNil(seed.reconnect)
        XCTAssertEqual(appliedSnapshots(in: seed.actions), [
            GaryxRenderSnapshot(
                basedOnSeq: 2,
                rows: [userTurnRow(id: "turn-1", userSeq: 1)],
                rowsHash: "1001"
            ),
        ])

        // Delta A: turn-1's body changes (upsert), chain 1001 -> 1002.
        let deltaA = processor.processPayload(
            deltaFramePayload(
                fromSeq: 2, fromRowsHash: "1001", basedOnSeq: 3, rowsHash: "1002",
                rowOrder: ["turn-1"],
                upsertRows: [userTurnRowJSON(id: "turn-1", userSeq: 1, replySeq: 3)],
                events: [event(seq: 3, role: "assistant", text: "more")]
            ),
            threadId: threadId
        )
        XCTAssertNil(deltaA.reconnect)
        XCTAssertEqual(actionKinds(in: deltaA.actions), ["rows", "snapshot"])
        XCTAssertEqual(processor.connectionLastSeq, 3)
        XCTAssertEqual(appliedSnapshots(in: deltaA.actions), [
            expectedDeltaSnapshot(
                basedOnSeq: 3,
                rows: [userTurnRow(id: "turn-1", userSeq: 1, replySeq: 3)],
                rowsHash: "1002"
            ),
        ])

        // Delta B anchors on delta A's token (chain continuity, not the
        // seed's), carries turn-1 forward from the held snapshot with its
        // FULL reassembled-by-A body, and upserts a new turn-2.
        let deltaB = processor.processPayload(
            deltaFramePayload(
                fromSeq: 3, fromRowsHash: "1002", basedOnSeq: 4, rowsHash: "1003",
                rowOrder: ["turn-1", "turn-2"],
                upsertRows: [userTurnRowJSON(id: "turn-2", userSeq: 4)],
                events: [event(seq: 4, role: "user", text: "second ask")]
            ),
            threadId: threadId
        )
        XCTAssertNil(deltaB.reconnect)
        XCTAssertEqual(appliedSnapshots(in: deltaB.actions), [
            expectedDeltaSnapshot(
                basedOnSeq: 4,
                rows: [
                    userTurnRow(id: "turn-1", userSeq: 1, replySeq: 3),
                    userTurnRow(id: "turn-2", userSeq: 4),
                ],
                rowsHash: "1003"
            ),
        ])
        XCTAssertEqual(processor.connectionLastSeq, 4)
    }

    func testUnknownRowKindSurvivesDeltaCarryForwardById() {
        var processor = GatewayStreamFrameProcessor()
        _ = processor.processPayload(
            fullFramePayload(
                basedOnSeq: 2,
                rows: [
                    userTurnRowJSON(id: "turn-1", userSeq: 1),
                    ["kind": "future_widget", "id": "row-x", "payload": "opaque"],
                ],
                rowsHash: "1001"
            ),
            threadId: threadId
        )

        // The delta re-references the unknown row without upserting it: its
        // preserved id must resolve from the held snapshot instead of
        // tripping the missing-row violation — and the carried-forward row
        // must still hold its FULL wire body (#TASK-2038 P2).
        let result = processor.processPayload(
            deltaFramePayload(
                fromSeq: 2, fromRowsHash: "1001", basedOnSeq: 3, rowsHash: "1002",
                rowOrder: ["turn-1", "row-x"],
                upsertRows: [userTurnRowJSON(id: "turn-1", userSeq: 1, replySeq: 3)]
            ),
            threadId: threadId
        )
        XCTAssertNil(result.reconnect)
        XCTAssertEqual(appliedSnapshots(in: result.actions), [
            expectedDeltaSnapshot(
                basedOnSeq: 3,
                rows: [
                    userTurnRow(id: "turn-1", userSeq: 1, replySeq: 3),
                    .unknown(raw: .object([
                        "kind": .string("future_widget"),
                        "id": .string("row-x"),
                        "payload": .string("opaque"),
                    ])),
                ],
                rowsHash: "1002"
            ),
        ])
    }

    func testUnknownRowCarriedForwardByDeltaRetainsItsOriginalPayload() throws {
        var processor = GatewayStreamFrameProcessor()
        let futureRow: [String: Any] = [
            "kind": "future_widget",
            "id": "row-x",
            "payload": ["answer": 42, "tags": ["a", "b"]] as [String: Any],
        ]
        _ = processor.processPayload(
            fullFramePayload(
                basedOnSeq: 2,
                rows: [userTurnRowJSON(id: "turn-1", userSeq: 1), futureRow],
                rowsHash: "1001"
            ),
            threadId: threadId
        )

        let result = processor.processPayload(
            deltaFramePayload(
                fromSeq: 2, fromRowsHash: "1001", basedOnSeq: 3, rowsHash: "1002",
                rowOrder: ["turn-1", "row-x"],
                upsertRows: [userTurnRowJSON(id: "turn-1", userSeq: 1, replySeq: 3)]
            ),
            threadId: threadId
        )
        XCTAssertNil(result.reconnect)

        // The reassembled snapshot is what `GaryxTranscriptCache` persists:
        // the carried-forward unknown row must still hold its FULL wire
        // body (#TASK-2038 P2), not a lossy `{kind:unknown,id}` husk.
        let rows = appliedSnapshots(in: result.actions).first?.rows ?? []
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(
            try JSONDecoder().decode(GaryxJSONValue.self, from: JSONEncoder().encode(rows[1])),
            try JSONDecoder().decode(
                GaryxJSONValue.self,
                from: JSONSerialization.data(withJSONObject: futureRow)
            ),
            "delta carry-forward must keep the unknown row's original payload, not just its id"
        )
    }

    // MARK: - Protocol violations ride the existing gap exit

    func testDeltaFromSeqMismatchGapsAtomically() {
        var processor = seededProcessor()

        let result = processor.processPayload(
            deltaFramePayload(
                fromSeq: 9, fromRowsHash: "1001", basedOnSeq: 10, rowsHash: "1002",
                rowOrder: ["turn-1"], upsertRows: [],
                events: [event(seq: 3, role: "assistant", text: "dropped with the frame")]
            ),
            threadId: threadId
        )

        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(result.actions, [], "violating frames are discarded atomically, events included")
        XCTAssertEqual(processor.connectionLastSeq, 2)
    }

    func testDeltaChainTokenMismatchGapsAndLeavesHeldBaseUntouched() {
        var processor = seededProcessor()

        let violation = processor.processPayload(
            deltaFramePayload(
                fromSeq: 2, fromRowsHash: "9999", basedOnSeq: 3, rowsHash: "1002",
                rowOrder: ["turn-1"], upsertRows: []
            ),
            threadId: threadId
        )
        XCTAssertEqual(violation.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(violation.actions, [])

        // Atomic discard: the held snapshot + token were not half-updated,
        // so a delta anchored on the pre-violation base still applies.
        let recovered = processor.processPayload(
            validNextDeltaPayload(),
            threadId: threadId
        )
        XCTAssertNil(recovered.reconnect)
        XCTAssertEqual(appliedSnapshots(in: recovered.actions).map(\.rowsHash), ["1002"])
    }

    func testDeltaMissingRowGapsAndLeavesHeldBaseUntouched() {
        var processor = seededProcessor()

        // "turn-ghost" resolves to neither upsert_rows nor the held rows;
        // the violation aborts mid-rebuild and must not leak a partial base.
        let violation = processor.processPayload(
            deltaFramePayload(
                fromSeq: 2, fromRowsHash: "1001", basedOnSeq: 3, rowsHash: "1002",
                rowOrder: ["turn-1", "turn-ghost"],
                upsertRows: [userTurnRowJSON(id: "turn-1", userSeq: 1, replySeq: 3)]
            ),
            threadId: threadId
        )
        XCTAssertEqual(violation.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(violation.actions, [])

        let recovered = processor.processPayload(
            validNextDeltaPayload(),
            threadId: threadId
        )
        XCTAssertNil(recovered.reconnect)
        XCTAssertEqual(appliedSnapshots(in: recovered.actions).map(\.rowsHash), ["1002"])
    }

    func testDuplicateUpsertRowGaps() {
        var processor = seededProcessor()

        let result = processor.processPayload(
            deltaFramePayload(
                fromSeq: 2, fromRowsHash: "1001", basedOnSeq: 3, rowsHash: "1002",
                rowOrder: ["turn-1"],
                upsertRows: [
                    userTurnRowJSON(id: "turn-1", userSeq: 1),
                    userTurnRowJSON(id: "turn-1", userSeq: 1, replySeq: 3),
                ]
            ),
            threadId: threadId
        )
        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(result.actions, [])
    }

    func testUnexpectedUpsertRowOutsideRowOrderGaps() {
        var processor = seededProcessor()

        let result = processor.processPayload(
            deltaFramePayload(
                fromSeq: 2, fromRowsHash: "1001", basedOnSeq: 3, rowsHash: "1002",
                rowOrder: ["turn-1"],
                upsertRows: [userTurnRowJSON(id: "turn-9", userSeq: 4)]
            ),
            threadId: threadId
        )
        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(result.actions, [])
    }

    // MARK: - Malformed delta frames gap, never vanish (#TASK-2038 P1)

    func testMalformedDeltaFrameGapsInsteadOfBeingSilentlyIgnored() {
        var processor = seededProcessor()

        // `render_delta` is present but missing its chain-critical
        // `rows_hash`, so the strict body decode fails. The frame must ride
        // the gap exit — its committed events are recovered by the replay —
        // never fall out of the envelope decode as a silent `.ignored` that
        // drops seq 3 without moving the cursor. Desktop parity:
        // `applyRenderDeltaFrame` throws its gap error on malformed bodies.
        let result = processor.processPayload(
            malformedDeltaFramePayload(
                events: [event(seq: 3, role: "assistant", text: "dropped with the frame, replayed after the gap")]
            ),
            threadId: threadId
        )

        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(result.actions, [], "the malformed frame is discarded atomically, events included")
        XCTAssertEqual(processor.connectionLastSeq, 2, "the cursor must not advance past the dropped events")
    }

    func testNonObjectDeltaPayloadGaps() {
        var processor = seededProcessor()

        let result = processor.processPayload(
            jsonString([
                "type": "thread_render_frame",
                "thread_id": threadId,
                "events": [] as [Any],
                "render_delta": "not an object",
            ]),
            threadId: threadId
        )

        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(result.actions, [])
        XCTAssertEqual(processor.connectionLastSeq, 2)
    }

    func testFrameCarryingFullStateAndMalformedDeltaReseedsFromTheFullState() {
        var processor = seededProcessor()

        // Same edge decision as the well-formed both-payloads frame: the
        // full snapshot is authoritative and reseeding is always safe, so a
        // malformed delta riding a full-state frame must not gap (and must
        // not kill the whole frame's decode either).
        let result = processor.processPayload(
            malformedDeltaFramePayload(
                renderState: renderStateJSON(
                    basedOnSeq: 3,
                    rows: [userTurnRowJSON(id: "turn-A", userSeq: 3)],
                    rowsHash: "2001"
                )
            ),
            threadId: threadId
        )

        XCTAssertNil(result.reconnect)
        XCTAssertEqual(appliedSnapshots(in: result.actions), [
            GaryxRenderSnapshot(
                basedOnSeq: 3,
                rows: [userTurnRow(id: "turn-A", userSeq: 3)],
                rowsHash: "2001"
            ),
        ])
    }

    func testNullDeltaSlotIsIgnoredNotGapped() {
        var processor = seededProcessor()

        // A JSON-null `render_delta` means "no delta", exactly like an
        // absent key (desktop treats null and undefined the same). Nothing
        // to render, nothing to gap over.
        let result = processor.processPayload(
            jsonString([
                "type": "thread_render_frame",
                "thread_id": threadId,
                "events": [] as [Any],
                "render_delta": NSNull(),
            ]),
            threadId: threadId
        )

        XCTAssertNil(result.reconnect)
        XCTAssertEqual(result.actions, [])
    }

    func testDeltaBeforeAnyFullFrameGaps() {
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 2, replayScope: .resume)

        let result = processor.processPayload(
            validNextDeltaPayload(),
            threadId: threadId
        )
        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(result.actions, [])
    }

    func testFullFrameWithoutChainTokenCannotAnchorADelta() {
        var processor = GatewayStreamFrameProcessor()
        _ = processor.processPayload(
            fullFramePayload(
                basedOnSeq: 2,
                rows: [userTurnRowJSON(id: "turn-1", userSeq: 1)],
                rowsHash: nil,
                events: [event(seq: 1, role: "user", text: "ask"),
                         event(seq: 2, role: "assistant", text: "reply")]
            ),
            threadId: threadId
        )

        let result = processor.processPayload(
            validNextDeltaPayload(),
            threadId: threadId
        )
        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(result.actions, [])
    }

    func testResetConnectionClearsTheHeldDeltaBase() {
        var processor = seededProcessor()
        processor.resetConnection(afterSeq: 2, replayScope: .resume)

        // Connection lifetime: after a reconnect the chain must restart
        // from a full frame; a delta matching the previous connection's
        // base is a violation, not a resume.
        let result = processor.processPayload(
            validNextDeltaPayload(),
            threadId: threadId
        )
        XCTAssertEqual(result.reconnect, .gap(resumeAfterSeq: 2))
        XCTAssertEqual(result.actions, [])
    }

    // MARK: - Full frames reseed unconditionally

    func testFrameCarryingBothStateAndDeltaReseedsFromTheFullState() {
        var processor = seededProcessor()

        // The gateway never produces this shape; if it ever appears, the
        // full snapshot is authoritative and reseeding is always safe —
        // same resolution as the desktop reassembler, no gap.
        let both = processor.processPayload(
            deltaFramePayload(
                fromSeq: 2, fromRowsHash: "1001", basedOnSeq: 3, rowsHash: "9999",
                rowOrder: ["turn-1"], upsertRows: [],
                renderState: renderStateJSON(
                    basedOnSeq: 3,
                    rows: [userTurnRowJSON(id: "turn-A", userSeq: 3)],
                    rowsHash: "2001"
                )
            ),
            threadId: threadId
        )
        XCTAssertNil(both.reconnect)
        XCTAssertEqual(appliedSnapshots(in: both.actions), [
            GaryxRenderSnapshot(
                basedOnSeq: 3,
                rows: [userTurnRow(id: "turn-A", userSeq: 3)],
                rowsHash: "2001"
            ),
        ])

        // The delta riding that frame was ignored: the chain anchors on the
        // full state's token, not the delta's.
        let next = processor.processPayload(
            deltaFramePayload(
                fromSeq: 3, fromRowsHash: "2001", basedOnSeq: 4, rowsHash: "2002",
                rowOrder: ["turn-A"], upsertRows: []
            ),
            threadId: threadId
        )
        XCTAssertNil(next.reconnect)
        XCTAssertEqual(appliedSnapshots(in: next.actions).map(\.rowsHash), ["2002"])
    }

    func testSnapshotOnlyAndSameSeqFullFramesReseedTheChain() {
        var processor = seededProcessor()
        _ = processor.processPayload(validNextDeltaPayload(), threadId: threadId)
        // Held is now the reassembled seq-3 snapshot (token "1002").

        // A snapshot-only full frame at the SAME seq with different content
        // (the same-seq overwrite/reseed shape) replaces base + token.
        let reseed = processor.processPayload(
            fullFramePayload(
                basedOnSeq: 3,
                rows: [userTurnRowJSON(id: "turn-1", userSeq: 1)],
                rowsHash: "3001"
            ),
            threadId: threadId
        )
        XCTAssertNil(reseed.reconnect)
        XCTAssertEqual(actionKinds(in: reseed.actions), ["snapshot"])

        // New token anchors; the pre-reseed token no longer does.
        let onNewToken = processor.processPayload(
            deltaFramePayload(
                fromSeq: 3, fromRowsHash: "3001", basedOnSeq: 4, rowsHash: "3002",
                rowOrder: ["turn-1"], upsertRows: []
            ),
            threadId: threadId
        )
        XCTAssertNil(onNewToken.reconnect)
        XCTAssertEqual(appliedSnapshots(in: onNewToken.actions).map(\.rowsHash), ["3002"])

        let onStaleToken = processor.processPayload(
            deltaFramePayload(
                fromSeq: 4, fromRowsHash: "3001", basedOnSeq: 5, rowsHash: "3003",
                rowOrder: ["turn-1"], upsertRows: []
            ),
            threadId: threadId
        )
        XCTAssertEqual(onStaleToken.reconnect, .gap(resumeAfterSeq: 2))
    }

    func testWindowedReplayFullFrameReseedsTheChain() {
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 12, replayScope: .resume)

        let windowed = processor.processPayload(
            fullFramePayload(
                basedOnSeq: 4801,
                rows: [userTurnRowJSON(id: "turn-w", userSeq: 4801)],
                rowsHash: "4001",
                events: [event(seq: 4801, role: "user", text: "window head")],
                replay: "windowed",
                window: ["floor_seq": 4801, "has_more_above": true]
            ),
            threadId: threadId
        )
        XCTAssertNil(windowed.reconnect)
        XCTAssertEqual(actionKinds(in: windowed.actions), ["reset", "rows", "snapshot"])

        let delta = processor.processPayload(
            deltaFramePayload(
                fromSeq: 4801, fromRowsHash: "4001", basedOnSeq: 4802, rowsHash: "4002",
                rowOrder: ["turn-w"], upsertRows: [],
                events: [event(seq: 4802, role: "assistant", text: "window reply")]
            ),
            threadId: threadId
        )
        XCTAssertNil(delta.reconnect)
        XCTAssertEqual(appliedSnapshots(in: delta.actions).map(\.rowsHash), ["4002"])
        XCTAssertEqual(processor.connectionLastSeq, 4802)
    }

    // MARK: - Downstream guard

    func testActionStreamAlwaysCarriesFullSnapshots() {
        var processor = GatewayStreamFrameProcessor()
        var results: [GatewayStreamPayloadResult] = []
        results.append(processor.processPayload(
            fullFramePayload(
                basedOnSeq: 2,
                rows: [userTurnRowJSON(id: "turn-1", userSeq: 1)],
                rowsHash: "1001",
                events: [event(seq: 1, role: "user", text: "ask"),
                         event(seq: 2, role: "assistant", text: "reply")]
            ),
            threadId: threadId
        ))
        results.append(processor.processPayload(
            deltaFramePayload(
                fromSeq: 2, fromRowsHash: "1001", basedOnSeq: 3, rowsHash: "1002",
                rowOrder: ["turn-1", "turn-2"],
                upsertRows: [userTurnRowJSON(id: "turn-2", userSeq: 3)],
                events: [event(seq: 3, role: "user", text: "second")]
            ),
            threadId: threadId
        ))
        results.append(processor.processPayload(
            deltaFramePayload(
                fromSeq: 3, fromRowsHash: "1002", basedOnSeq: 4, rowsHash: "1003",
                rowOrder: ["turn-1", "turn-2"],
                upsertRows: [userTurnRowJSON(id: "turn-2", userSeq: 3, replySeq: 4)],
                events: [event(seq: 4, role: "assistant", text: "answer")]
            ),
            threadId: threadId
        ))

        let expectedRows: [[GaryxRenderRow]] = [
            [userTurnRow(id: "turn-1", userSeq: 1)],
            [userTurnRow(id: "turn-1", userSeq: 1), userTurnRow(id: "turn-2", userSeq: 3)],
            [userTurnRow(id: "turn-1", userSeq: 1), userTurnRow(id: "turn-2", userSeq: 3, replySeq: 4)],
        ]
        for (index, result) in results.enumerated() {
            XCTAssertNil(result.reconnect)
            // Every rendering frame ends with exactly one snapshot action,
            // and that snapshot carries the COMPLETE row set with full
            // bodies — un-upserted rows included — so applyRenderSnapshot,
            // the mapper, renderEquivalent, and the flush gate stay
            // byte-identical to the full-frame world.
            XCTAssertEqual(actionKinds(in: result.actions), ["rows", "snapshot"], "frame \(index)")
            XCTAssertEqual(appliedSnapshots(in: result.actions).map(\.rows), [expectedRows[index]], "frame \(index)")
        }
    }

    // MARK: - Projection vocabulary skew is display-only

    func testEveryInvalidProjectionShapeIsLossyInAFullSnapshot() throws {
        for (index, invalidCase) in invalidProjectionCases().enumerated() {
            var processor = GatewayStreamFrameProcessor()
            let result = processor.processPayload(
                fullFramePayload(
                    basedOnSeq: 1,
                    rows: [toolTurnRowJSON(projection: invalidCase.projection)],
                    rowsHash: "\(5_001 + index)"
                ),
                threadId: threadId
            )

            XCTAssertNil(result.reconnect, invalidCase.name)
            let snapshot = try XCTUnwrap(appliedSnapshots(in: result.actions).only, invalidCase.name)
            let entry = try XCTUnwrap(toolEntries(in: snapshot).only, invalidCase.name)
            XCTAssertEqual(entry.id, "entry:projection", invalidCase.name)
            XCTAssertNil(entry.projection, "\(invalidCase.name) must degrade only the projection")
        }
    }

    func testEveryInvalidProjectionShapeIsLossyInDeltaAndNextValidDeltaApplies() throws {
        for invalidCase in invalidProjectionCases() {
            var processor = GatewayStreamFrameProcessor()
            let seed = processor.processPayload(
                fullFramePayload(
                    basedOnSeq: 1,
                    rows: [toolTurnRowJSON(projection: validProjectionJSON())],
                    rowsHash: "6001"
                ),
                threadId: threadId
            )
            XCTAssertNil(seed.reconnect, invalidCase.name)

            let lossy = processor.processPayload(
                deltaFramePayload(
                    fromSeq: 1,
                    fromRowsHash: "6001",
                    basedOnSeq: 2,
                    rowsHash: "6002",
                    rowOrder: ["turn:projection"],
                    upsertRows: [toolTurnRowJSON(projection: invalidCase.projection)]
                ),
                threadId: threadId
            )
            XCTAssertNil(lossy.reconnect, invalidCase.name)
            let lossySnapshot = try XCTUnwrap(appliedSnapshots(in: lossy.actions).only, invalidCase.name)
            let lossyEntry = try XCTUnwrap(toolEntries(in: lossySnapshot).only, invalidCase.name)
            XCTAssertNil(lossyEntry.projection, invalidCase.name)

            let recovered = processor.processPayload(
                deltaFramePayload(
                    fromSeq: 2,
                    fromRowsHash: "6002",
                    basedOnSeq: 3,
                    rowsHash: "6003",
                    rowOrder: ["turn:projection"],
                    upsertRows: [toolTurnRowJSON(projection: validProjectionJSON())]
                ),
                threadId: threadId
            )
            XCTAssertNil(recovered.reconnect, "\(invalidCase.name) must not enter gap/replay")
            let recoveredSnapshot = try XCTUnwrap(
                appliedSnapshots(in: recovered.actions).only,
                invalidCase.name
            )
            let recoveredEntry = try XCTUnwrap(
                toolEntries(in: recoveredSnapshot).only,
                invalidCase.name
            )
            XCTAssertNotNil(recoveredEntry.projection, invalidCase.name)
            XCTAssertEqual(recoveredSnapshot.rowsHash, "6003", invalidCase.name)
        }
    }

    // MARK: - Fixtures

    /// Processor holding the canonical seed: one `turn-1` row, seq 2,
    /// chain token "1001", committed cursor 2.
    private func seededProcessor() -> GatewayStreamFrameProcessor {
        var processor = GatewayStreamFrameProcessor()
        let seed = processor.processPayload(
            fullFramePayload(
                basedOnSeq: 2,
                rows: [userTurnRowJSON(id: "turn-1", userSeq: 1)],
                rowsHash: "1001",
                events: [event(seq: 1, role: "user", text: "ask"),
                         event(seq: 2, role: "assistant", text: "reply")]
            ),
            threadId: threadId
        )
        XCTAssertNil(seed.reconnect)
        XCTAssertEqual(processor.connectionLastSeq, 2)
        return processor
    }

    /// The valid continuation of `seededProcessor()`'s chain: 2/"1001" ->
    /// 3/"1002", turn-1 carried forward without an upsert.
    private func validNextDeltaPayload() -> String {
        deltaFramePayload(
            fromSeq: 2, fromRowsHash: "1001", basedOnSeq: 3, rowsHash: "1002",
            rowOrder: ["turn-1"], upsertRows: []
        )
    }

    private func appliedSnapshots(in actions: [GatewayStreamAction]) -> [GaryxRenderSnapshot] {
        actions.compactMap { action in
            guard case let .applyRenderSnapshot(snapshot) = action else { return nil }
            return snapshot
        }
    }

    private func actionKinds(in actions: [GatewayStreamAction]) -> [String] {
        actions.map { action in
            switch action {
            case .applyCommittedMessages:
                return "rows"
            case .applyRenderSnapshot:
                return "snapshot"
            case .resetCommittedCacheBelow:
                return "reset"
            case .refetchAfterControlRewrite:
                return "refetch"
            case .fallback:
                return "fallback"
            }
        }
    }

    private func invalidProjectionCases() -> [(name: String, projection: [String: Any])] {
        let selector: [String: Any] = [
            "root": "content",
            "path": ["input", "content"],
        ]
        let base: [String: Any] = [
            "tool_name": "Write",
            "kind": "file_write",
            "visibility": "normal",
        ]
        func withDiff(_ diff: [String: Any]) -> [String: Any] {
            var projection = base
            projection["diff"] = diff
            return projection
        }

        var legacyFormat = base
        legacyFormat["call"] = [
            "root": "content",
            "path": ["input", "content"],
            "format": "diff",
            "label": "call",
        ]
        return [
            ("legacy format diff", legacyFormat),
            ("unknown segment discriminator", withDiff([
                "source": "tool_use",
                "segments": [["future": ["text": selector]]],
            ])),
            ("unknown source", withDiff([
                "source": "future_source",
                "segments": [["unified": ["text": selector]]],
            ])),
            ("missing source", withDiff([
                "segments": [["unified": ["text": selector]]],
            ])),
            ("empty recipe", withDiff([
                "source": "tool_use",
                "segments": [] as [Any],
            ])),
            ("double-none pair", withDiff([
                "source": "tool_use",
                "segments": [["pair": ["old": NSNull(), "new": NSNull()]]],
            ])),
        ]
    }

    private func validProjectionJSON() -> [String: Any] {
        [
            "tool_name": "Write",
            "kind": "file_write",
            "visibility": "normal",
            "summary": [
                "root": "content",
                "path": ["input", "file_path"],
                "format": "path",
                "label": "file",
            ],
            "diff": [
                "source": "tool_use",
                "segments": [[
                    "pair": [
                        "old": NSNull(),
                        "new": [
                            "root": "content",
                            "path": ["input", "content"],
                        ],
                    ],
                ]],
            ],
        ]
    }

    private func toolTurnRowJSON(projection: [String: Any]) -> [String: Any] {
        [
            "kind": "user_turn",
            "id": "turn:projection",
            "user": NSNull(),
            "activity": [[
                "kind": "step",
                "id": "step:projection",
                "steps": [[
                    "kind": "tool_group",
                    "id": "group:projection",
                    "status": "completed",
                    "entries": [[
                        "id": "entry:projection",
                        "status": "completed",
                        "tool_use": NSNull(),
                        "tool_result": NSNull(),
                        "projection": projection,
                    ]],
                ]],
            ]],
        ]
    }

    private func toolEntries(in snapshot: GaryxRenderSnapshot) -> [GaryxRenderToolEntry] {
        snapshot.rows.flatMap { row -> [GaryxRenderToolEntry] in
            guard case .userTurn(let turn) = row else { return [] }
            return turn.activity.flatMap { activity -> [GaryxRenderToolEntry] in
                guard case .step(let step) = activity else { return [] }
                return step.steps.flatMap { item -> [GaryxRenderToolEntry] in
                    guard case .toolGroup(let group) = item else { return [] }
                    return group.entries
                }
            }
        }
    }

    /// Expected reassembly of a delta produced by `deltaFramePayload`:
    /// scalars replaced wholesale (`tailActivity`/`progress_locus` fixed by
    /// the fixture), the delta's `rows_hash` stored as the new token.
    private func expectedDeltaSnapshot(
        basedOnSeq: Int,
        rows: [GaryxRenderRow],
        rowsHash: String
    ) -> GaryxRenderSnapshot {
        GaryxRenderSnapshot(
            basedOnSeq: basedOnSeq,
            rows: rows,
            tailActivity: .assistantStreaming,
            activeToolGroupId: nil,
            progressLocus: .tail,
            filteredPlaceholders: [],
            rateLimit: nil,
            window: nil,
            rowsHash: rowsHash
        )
    }

    private func userTurnRowJSON(id: String, userSeq: Int, replySeq: Int? = nil) -> [String: Any] {
        var row: [String: Any] = [
            "kind": "user_turn",
            "id": id,
            "user": ["id": "history:\(userSeq - 1)", "seq": userSeq, "role": "user"],
            "activity": [] as [Any],
        ]
        if let replySeq {
            row["activity"] = [[
                "kind": "assistant_reply",
                "id": "reply:\(replySeq)",
                "message": ["id": "history:\(replySeq - 1)", "seq": replySeq, "role": "assistant"],
            ] as [String: Any]]
        }
        return row
    }

    private func userTurnRow(id: String, userSeq: Int, replySeq: Int? = nil) -> GaryxRenderRow {
        var activity: [GaryxRenderActivityRow] = []
        if let replySeq {
            activity = [.assistantReply(GaryxRenderAssistantReplyRow(
                id: "reply:\(replySeq)",
                message: GaryxRenderMessageRef(
                    id: "history:\(replySeq - 1)",
                    seq: replySeq,
                    role: "assistant"
                )
            ))]
        }
        return .userTurn(GaryxRenderUserTurnRow(
            id: id,
            user: GaryxRenderMessageRef(id: "history:\(userSeq - 1)", seq: userSeq, role: "user"),
            activity: activity
        ))
    }

    private func renderStateJSON(
        basedOnSeq: Int,
        rows: [[String: Any]],
        rowsHash: String?,
        window: [String: Any]? = nil
    ) -> [String: Any] {
        var state: [String: Any] = [
            "based_on_seq": basedOnSeq,
            "rows": rows,
            "tailActivity": "none",
            "progress_locus": "none",
            "filtered_placeholders": [] as [Any],
        ]
        if let rowsHash {
            state["rows_hash"] = rowsHash
        }
        if let window {
            state["window"] = window
        }
        return state
    }

    private func fullFramePayload(
        basedOnSeq: Int,
        rows: [[String: Any]],
        rowsHash: String?,
        events: [[String: Any]] = [],
        replay: String? = nil,
        window: [String: Any]? = nil
    ) -> String {
        var frame: [String: Any] = [
            "type": "thread_render_frame",
            "thread_id": threadId,
            "events": events,
            "render_state": renderStateJSON(
                basedOnSeq: basedOnSeq,
                rows: rows,
                rowsHash: rowsHash,
                window: window
            ),
        ]
        if let replay {
            frame["replay"] = replay
        }
        return jsonString(frame)
    }

    private func deltaFramePayload(
        fromSeq: Int,
        fromRowsHash: String,
        basedOnSeq: Int,
        rowsHash: String,
        rowOrder: [String],
        upsertRows: [[String: Any]],
        events: [[String: Any]] = [],
        renderState: [String: Any]? = nil
    ) -> String {
        var frame: [String: Any] = [
            "type": "thread_render_frame",
            "thread_id": threadId,
            "events": events,
            "render_delta": [
                "from_seq": fromSeq,
                "from_rows_hash": fromRowsHash,
                "based_on_seq": basedOnSeq,
                "rows_hash": rowsHash,
                "row_order": rowOrder,
                "upsert_rows": upsertRows,
                "tailActivity": "assistant_streaming",
                "activeToolGroupId": NSNull(),
                "progress_locus": "tail",
                "filtered_placeholders": [] as [Any],
            ] as [String: Any],
        ]
        if let renderState {
            frame["render_state"] = renderState
        }
        return jsonString(frame)
    }

    /// A frame whose `render_delta` key is present but undecodable: the
    /// chain-critical `rows_hash` is missing, so the strict body decode
    /// throws (#TASK-2038 P1 probe shape).
    private func malformedDeltaFramePayload(
        events: [[String: Any]] = [],
        renderState: [String: Any]? = nil
    ) -> String {
        var frame: [String: Any] = [
            "type": "thread_render_frame",
            "thread_id": threadId,
            "events": events,
            "render_delta": [
                "from_seq": 2,
                "from_rows_hash": "1001",
                "based_on_seq": 3,
                // "rows_hash" deliberately missing.
                "row_order": ["turn-1"],
                "upsert_rows": [] as [Any],
            ] as [String: Any],
        ]
        if let renderState {
            frame["render_state"] = renderState
        }
        return jsonString(frame)
    }

    private func event(seq: Int, role: String, text: String) -> [String: Any] {
        [
            "type": "committed_message",
            "seq": seq,
            "message": [
                "role": role,
                "text": text,
            ],
        ]
    }

    private func jsonString(_ object: [String: Any]) -> String {
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }
}

private extension Array {
    var only: Element? {
        count == 1 ? self[0] : nil
    }
}
