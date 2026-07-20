import XCTest
@testable import GaryxMobileCore

/// Conformance suite for the cross-platform conversation state contract.
/// Runs the shared fixtures in spec/conversation-state against the iOS
/// implementation. The desktop twin is
/// desktop/garyx-desktop/src/renderer/src/conversation-state-conformance.test.mjs.
/// See docs/agents/conversation-state.md.
final class GaryxConversationStateConformanceTests: XCTestCase {
    private static let fixedNow = "2026-01-01T00:00:00.000Z"

    private func specURL(_ relativePath: String) -> URL {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 {
            url.deleteLastPathComponent()
        }
        return url
            .appendingPathComponent("spec")
            .appendingPathComponent("conversation-state")
            .appendingPathComponent(relativePath)
    }

    private func loadJSONObject(_ relativePath: String) throws -> [String: Any] {
        let data = try Data(contentsOf: specURL(relativePath))
        return try XCTUnwrap(
            try JSONSerialization.jsonObject(with: data) as? [String: Any],
            "fixture \(relativePath) should decode to an object"
        )
    }

    // MARK: Vocabulary

    func testEnumVocabulariesMatchSharedSchema() throws {
        let states = try loadJSONObject("states.json")
        XCTAssertEqual(GaryxIntentState.allCases.map(\.rawValue), states["intentState"] as? [String])
        XCTAssertEqual(GaryxIntentSource.allCases.map(\.rawValue), states["intentSource"] as? [String])
        XCTAssertEqual(
            GaryxIntentDispatchMode.allCases.map(\.rawValue),
            states["intentDispatchMode"] as? [String]
        )
        XCTAssertEqual(
            GaryxThreadRuntimeState.allCases.map(\.rawValue),
            states["threadRuntimeState"] as? [String]
        )
        XCTAssertEqual(
            GaryxLiveStreamStatus.allCases.map(\.rawValue),
            states["liveStreamStatus"] as? [String]
        )
        XCTAssertEqual(
            GaryxTranscriptEntryState.allCases.map(\.rawValue),
            states["transcriptEntryState"] as? [String]
        )
        XCTAssertEqual(GaryxComposerPhase.allCases.map(\.rawValue), states["composerPhase"] as? [String])
        XCTAssertEqual(
            GaryxDurableDeliveryState.allCases.map(\.rawValue),
            states["durableDeliveryState"] as? [String]
        )
        XCTAssertEqual(
            GaryxDeliveryEvidence.allCases.map(\.rawValue),
            states["durableDeliveryEvidence"] as? [String]
        )
        XCTAssertEqual(
            GaryxDeliveryUserDisposition.allCases.map(\.rawValue),
            states["durableDeliveryUserDisposition"] as? [String]
        )
        XCTAssertEqual(
            GaryxCreateDeliveryPhase.allCases.map(\.rawValue),
            states["durableCreateDeliveryPhase"] as? [String]
        )
        XCTAssertEqual(
            GaryxCreateAmbiguousDisposition.allCases.map(\.rawValue),
            states["durableCreateUserDisposition"] as? [String]
        )
    }

    // MARK: Machine scenarios

    func testMachineScenarioFixtures() throws {
        let fixtures = try loadJSONObject("scenarios/machine.json")
        let scenarios = try XCTUnwrap(fixtures["scenarios"] as? [[String: Any]])
        XCTAssertFalse(scenarios.isEmpty)

        for scenario in scenarios {
            let name = scenario["name"] as? String ?? "unnamed"
            let steps = try XCTUnwrap(scenario["steps"] as? [[String: Any]], "\(name): steps")
            var state = GaryxConversationMachineState()
            for (index, step) in steps.enumerated() {
                if let rawAction = step["action"] as? [String: Any] {
                    let action = try machineAction(from: rawAction, label: "\(name) step \(index)")
                    state.apply(action, now: { Self.fixedNow })
                }
                if let expectation = step["expect"] as? [String: Any] {
                    assertSnapshot(state, expectation, label: "\(name) step \(index)")
                }
            }
        }
    }

    // MARK: Activity model

    func testActivityModelFixtures() throws {
        let fixtures = try loadJSONObject("scenarios/activity.json")
        let cases = try XCTUnwrap(fixtures["cases"] as? [[String: Any]])
        XCTAssertFalse(cases.isEmpty)

        for fixtureCase in cases {
            let name = fixtureCase["name"] as? String ?? "unnamed"
            let input = try XCTUnwrap(fixtureCase["input"] as? [String: Any], "\(name): input")
            let expect = try XCTUnwrap(fixtureCase["expect"] as? [String: Any], "\(name): expect")

            let rawMessages = try XCTUnwrap(input["messages"] as? [[String: Any]], "\(name): messages")
            let messages = try rawMessages.map { raw -> GaryxActivityMessage in
                let roleValue = try XCTUnwrap(raw["role"] as? String, "\(name): message role")
                let role = try XCTUnwrap(GaryxTranscriptRole(rawValue: roleValue), "\(name): role \(roleValue)")
                let isLoopContinuation = (raw["internal"] as? Bool ?? false)
                    && (raw["internalKind"] as? String) == "loop_continuation"
                return GaryxActivityMessage(
                    role: role,
                    isLoopContinuation: isLoopContinuation
                )
            }

            let model = GaryxThreadActivityModel.derive(
                messages: messages,
                runtimeBusy: input["runtimeBusy"] as? Bool ?? false,
                pendingAckIntentCount: input["pendingAckIntentCount"] as? Int ?? 0,
                remoteAwaitingAckInputCount: input["remoteAwaitingAckInputCount"] as? Int ?? 0,
                pendingHistoryIntent: input["pendingHistoryIntent"] as? Bool ?? false
            )

            XCTAssertEqual(model.runActive, expect["runActive"] as? Bool, "\(name): runActive")
            XCTAssertEqual(
                model.showPendingAckLoading,
                expect["showPendingAckLoading"] as? Bool,
                "\(name): showPendingAckLoading"
            )
            XCTAssertEqual(
                model.canSteerQueuedPrompt,
                expect["canSteerQueuedPrompt"] as? Bool,
                "\(name): canSteerQueuedPrompt"
            )
        }
    }

    // MARK: Function cases

    func testPendingAckIndexFixtures() throws {
        let fixtures = try loadJSONObject("scenarios/function-cases.json")
        let cases = try XCTUnwrap(fixtures["pendingAckIndex"] as? [[String: Any]])
        XCTAssertFalse(cases.isEmpty)

        for fixtureCase in cases {
            let name = fixtureCase["name"] as? String ?? "unnamed"
            let pendingIds = try XCTUnwrap(fixtureCase["pendingAckIntentIds"] as? [String], name)
            let ackId = try XCTUnwrap(fixtureCase["acknowledgedPendingInputId"] as? String, name)
            let rawIntents = try XCTUnwrap(fixtureCase["intents"] as? [String: Any], name)
            var intentsById: [String: GaryxMessageIntent] = [:]
            for (intentId, rawAny) in rawIntents {
                let raw = rawAny as? [String: Any] ?? [:]
                intentsById[intentId] = GaryxMessageIntent(
                    intentId: intentId,
                    threadId: "t1",
                    text: "",
                    state: .awaitingProviderAck,
                    source: .queueSteer,
                    pendingInputId: raw["pendingInputId"] as? String
                )
            }
            XCTAssertEqual(
                garyxFindPendingAckIntentIndex(
                    pendingAckIntentIds: pendingIds,
                    acknowledgedPendingInputId: ackId,
                    intentsById: intentsById
                ),
                fixtureCase["expect"] as? Int,
                name
            )
        }
    }

    func testProviderAckTrackingFixtures() throws {
        let fixtures = try loadJSONObject("scenarios/function-cases.json")
        let cases = try XCTUnwrap(fixtures["providerAckTracking"] as? [[String: Any]])
        XCTAssertFalse(cases.isEmpty)

        for fixtureCase in cases {
            let intentState = (fixtureCase["intentState"] as? String)
                .flatMap(GaryxIntentState.init(rawValue:))
            if fixtureCase["intentState"] is NSNull {
                XCTAssertNil(intentState)
            }
            XCTAssertEqual(
                garyxShouldTrackProviderAckAfterStreamInputResponse(intentState: intentState),
                fixtureCase["expect"] as? Bool,
                "intentState=\(String(describing: fixtureCase["intentState"]))"
            )
        }
    }

    func testComposerPhaseFixtures() throws {
        let fixtures = try loadJSONObject("scenarios/function-cases.json")
        let cases = try XCTUnwrap(fixtures["composerPhase"] as? [[String: Any]])
        XCTAssertFalse(cases.isEmpty)

        for fixtureCase in cases {
            let phase = garyxNextComposerPhase(
                hasText: fixtureCase["hasText"] as? Bool ?? false,
                isComposing: fixtureCase["isComposing"] as? Bool ?? false,
                locked: fixtureCase["locked"] as? Bool ?? false
            )
            XCTAssertEqual(phase.rawValue, fixtureCase["expect"] as? String)
        }
    }

    // MARK: Durable delivery scenarios

    func testDurableDeliveryScenarioFixtures() throws {
        let fixtures = try loadJSONObject("scenarios/durable-delivery.json")
        let consumers = try XCTUnwrap(fixtures["platformConsumers"] as? [String: String])
        XCTAssertEqual(consumers["ios"], "implemented")
        XCTAssertEqual(consumers["mac"], "implemented")
        let scenarios = try XCTUnwrap(fixtures["scenarios"] as? [[String: Any]])
        XCTAssertFalse(scenarios.isEmpty)

        for scenario in scenarios {
            let name = scenario["name"] as? String ?? "unnamed durable delivery"
            var records = ["delivery": makeDurableDeliveryRecord()]
            for action in scenario["actions"] as? [String] ?? [] {
                try applyDurableDeliveryAction(
                    action,
                    records: &records,
                    label: name
                )
            }
            let expectation = try XCTUnwrap(
                scenario["expect"] as? [String: Any],
                "\(name): expectation"
            )
            for (recordID, expectedAny) in expectation {
                let expected = try XCTUnwrap(
                    expectedAny as? [String: Any],
                    "\(name): \(recordID) expectation"
                )
                let record = try XCTUnwrap(records[recordID], "\(name): \(recordID)")
                assertDurableDelivery(record, expected: expected, label: "\(name): \(recordID)")
            }
        }
    }

    func testDurableCreateDeliveryScenarioFixtures() throws {
        let fixtures = try loadJSONObject("scenarios/durable-delivery.json")
        let scenarios = try XCTUnwrap(fixtures["createScenarios"] as? [[String: Any]])
        XCTAssertFalse(scenarios.isEmpty)
        let scope = durableFixtureScope

        for scenario in scenarios {
            let name = scenario["name"] as? String ?? "unnamed durable create"
            var state = GaryxCreateDeliveryState(
                scope: scope,
                createIntentID: "create-intent",
                entryID: durableFixtureEntryID
            )
            for action in scenario["actions"] as? [String] ?? [] {
                switch action {
                case "created": state.created(threadID: "thread-1")
                case "bound": state.bound()
                case "chatAttempted": state.chatStartAttempted()
                case "responseLost": state.responseLost()
                case "acknowledged": state.acknowledged()
                case "scopeRevoke": state.settleForScopeRevoke()
                default: XCTFail("\(name): unsupported create action \(action)")
                }
            }
            let expected = try XCTUnwrap(
                scenario["expect"] as? [String: Any],
                "\(name): expectation"
            )
            XCTAssertEqual(state.phase.rawValue, expected["phase"] as? String, "\(name): phase")
            assertNullableString(
                state.ambiguousAfter?.rawValue,
                expected["ambiguousAfter"],
                "\(name): ambiguousAfter"
            )
            XCTAssertEqual(
                state.userDisposition.rawValue,
                expected["userDisposition"] as? String,
                "\(name): userDisposition"
            )
            assertNullableString(state.threadID, expected["threadID"], "\(name): threadID")
        }
    }

    // MARK: Fixture decoding helpers

    private var durableFixtureScope: GaryxGatewayScope {
        GaryxGatewayScope(identity: "fixture-gateway", epoch: 1)
    }

    private var durableFixtureEntryID: GaryxComposerPayloadEntryID {
        GaryxComposerPayloadEntryID(rawValue: "fixture-entry")
    }

    private func makeDurableDeliveryRecord(
        id: String = "delivery",
        correlationID: String = "intent-original",
        envelope: GaryxDeliveryEnvelope? = nil
    ) -> GaryxDeliveryRecord {
        GaryxDeliveryRecord(
            id: .init(rawValue: id),
            scope: durableFixtureScope,
            entryID: durableFixtureEntryID,
            reservationID: .init(rawValue: 1),
            correlationID: correlationID,
            envelope: envelope ?? .init(
                text: "fixture message",
                attachmentIDs: [],
                generation: 1,
                clientIntentID: correlationID
            )
        )
    }

    private func applyDurableDeliveryAction(
        _ action: String,
        records: inout [String: GaryxDeliveryRecord],
        label: String
    ) throws {
        guard var delivery = records["delivery"] else {
            return XCTFail("\(label): original delivery missing")
        }
        switch action {
        case "attempt":
            XCTAssertTrue(delivery.markTransportAttempted(), label)
            records["delivery"] = delivery
        case "ambiguous":
            XCTAssertTrue(delivery.markAmbiguous(), label)
            records["delivery"] = delivery
        case "acknowledge":
            delivery.recordServerAcknowledgement()
            records["delivery"] = delivery
        case "evidence", "evidenceOtherScope":
            var keyed = Dictionary(uniqueKeysWithValues: records.map {
                (GaryxDeliveryRecordID(rawValue: $0.key), $0.value)
            })
            let source = action == "evidence"
                ? durableFixtureScope
                : GaryxGatewayScope(identity: "other-gateway", epoch: 1)
            let disposition = GaryxDeliveryEvidenceIngress.acknowledge(
                correlationID: "intent-original",
                authenticatedScope: source,
                records: &keyed
            )
            if action == "evidenceOtherScope" {
                XCTAssertEqual(disposition, .rejectedAuthenticationSource, label)
            } else if case .updated = disposition {
                // Expected.
            } else {
                XCTFail("\(label): evidence was not accepted: \(disposition)")
            }
            records = Dictionary(uniqueKeysWithValues: keyed.map {
                ($0.key.rawValue, $0.value)
            })
        case "restoreDraft":
            let disposition = GaryxDeliveryDraftRecoveryReducer.restore(
                record: &delivery
            )
            guard case .restored = disposition else {
                return XCTFail("\(label): restore was rejected: \(disposition)")
            }
            records["delivery"] = delivery
        case "recoverUndispatchedDraft":
            let disposition = GaryxDeliveryDraftRecoveryReducer.restore(
                record: &delivery,
                allowingUndispatched: true
            )
            guard case .restored = disposition else {
                return XCTFail("\(label): undispatched recovery was rejected: \(disposition)")
            }
            records["delivery"] = delivery
        case "resendCopy":
            let copyID = GaryxDeliveryRecordID(rawValue: "delivery-copy")
            guard let envelope = delivery.resendAsDuplicate(
                newRecordID: copyID,
                newClientIntentID: "intent-copy"
            ) else {
                return XCTFail("\(label): duplicate resend was rejected")
            }
            records["delivery"] = delivery
            records["delivery-copy"] = makeDurableDeliveryRecord(
                id: copyID.rawValue,
                correlationID: "intent-copy",
                envelope: envelope
            )
        case "scopeRevoke":
            for key in records.keys {
                var record = records[key]
                record?.settleForScopeRevoke()
                records[key] = record
            }
        default:
            XCTFail("\(label): unsupported durable delivery action \(action)")
        }
    }

    private func assertDurableDelivery(
        _ record: GaryxDeliveryRecord,
        expected: [String: Any],
        label: String
    ) {
        XCTAssertEqual(record.phase.rawValue, expected["state"] as? String, "\(label): state")
        XCTAssertEqual(record.evidence.rawValue, expected["evidence"] as? String, "\(label): evidence")
        XCTAssertEqual(
            record.userDisposition.rawValue,
            expected["userDisposition"] as? String,
            "\(label): userDisposition"
        )
        XCTAssertEqual(record.envelope != nil, expected["envelopePresent"] as? Bool, "\(label): envelope")
        if expected.keys.contains("duplicateRecordID") {
            assertNullableString(
                record.duplicateRecordID?.rawValue,
                expected["duplicateRecordID"],
                "\(label): duplicateRecordID"
            )
        }
        if expected.keys.contains("clientIntentID") {
            assertNullableString(
                record.envelope?.clientIntentID,
                expected["clientIntentID"],
                "\(label): clientIntentID"
            )
        }
    }

    private func machineAction(
        from raw: [String: Any],
        label: String
    ) throws -> GaryxConversationAction {
        let type = try XCTUnwrap(raw["type"] as? String, "\(label): action type")
        switch type {
        case "composer/sync":
            return .composerSync(
                hasText: raw["hasText"] as? Bool ?? false,
                isComposing: raw["isComposing"] as? Bool ?? false,
                locked: raw["locked"] as? Bool ?? false
            )
        case "intent/created":
            let rawIntent = try XCTUnwrap(raw["intent"] as? [String: Any], "\(label): intent")
            return .intentCreated(
                intent: try fixtureIntent(from: rawIntent, label: label),
                enqueue: raw["enqueue"] as? Bool ?? false
            )
        case "intent/request-dispatch":
            return .intentRequestDispatch(
                threadId: try XCTUnwrap(raw["threadId"] as? String, label),
                intentId: try XCTUnwrap(raw["intentId"] as? String, label),
                mode: try XCTUnwrap(
                    (raw["mode"] as? String).flatMap(GaryxIntentDispatchMode.init(rawValue:)),
                    label
                ),
                source: try XCTUnwrap(
                    (raw["source"] as? String).flatMap(GaryxIntentSource.init(rawValue:)),
                    label
                ),
                removeFromQueue: raw["removeFromQueue"] as? Bool ?? false
            )
        case "intent/dispatch-started":
            return .intentDispatchStarted(intentId: try XCTUnwrap(raw["intentId"] as? String, label))
        case "intent/remote-accepted":
            return .intentRemoteAccepted(
                intentId: try XCTUnwrap(raw["intentId"] as? String, label),
                runId: try XCTUnwrap(raw["runId"] as? String, label),
                threadId: try XCTUnwrap(raw["threadId"] as? String, label),
                pendingInputId: raw["pendingInputId"] as? String,
                responseText: raw["responseText"] as? String,
                removeFromQueue: raw["removeFromQueue"] as? Bool ?? false,
                awaitProviderAck: raw["awaitProviderAck"] as? Bool ?? false
            )
        case "intent/awaiting-response":
            return .intentAwaitingResponse(intentId: try XCTUnwrap(raw["intentId"] as? String, label))
        case "intent/awaiting-history":
            return .intentAwaitingHistory(
                intentId: try XCTUnwrap(raw["intentId"] as? String, label),
                responseText: raw["responseText"] as? String
            )
        case "intent/completed":
            return .intentCompleted(intentId: try XCTUnwrap(raw["intentId"] as? String, label))
        case "intent/failed":
            return .intentFailed(
                intentId: try XCTUnwrap(raw["intentId"] as? String, label),
                error: try XCTUnwrap(raw["error"] as? String, label)
            )
        case "intent/interrupted":
            return .intentInterrupted(
                intentId: try XCTUnwrap(raw["intentId"] as? String, label),
                error: raw["error"] as? String
            )
        case "intent/cancelled":
            return .intentCancelled(
                threadId: try XCTUnwrap(raw["threadId"] as? String, label),
                intentId: try XCTUnwrap(raw["intentId"] as? String, label)
            )
        case "intent/requeue-front":
            return .intentRequeueFront(
                threadId: try XCTUnwrap(raw["threadId"] as? String, label),
                intentId: try XCTUnwrap(raw["intentId"] as? String, label),
                error: raw["error"] as? String,
                source: (raw["source"] as? String).flatMap(GaryxIntentSource.init(rawValue:))
            )
        case "intent/reorder":
            return .intentReorder(
                threadId: try XCTUnwrap(raw["threadId"] as? String, label),
                intentId: try XCTUnwrap(raw["intentId"] as? String, label),
                toIndex: try XCTUnwrap(raw["toIndex"] as? Int, label)
            )
        case "thread/runtime":
            return .threadRuntime(
                threadId: try XCTUnwrap(raw["threadId"] as? String, label),
                state: try XCTUnwrap(
                    (raw["runtimeState"] as? String).flatMap(GaryxThreadRuntimeState.init(rawValue:)),
                    label
                ),
                activeIntentId: raw["activeIntentId"] as? String,
                remoteRunId: raw["remoteRunId"] as? String,
                error: raw["error"] as? String
            )
        case "thread/clear":
            return .threadClear(threadId: try XCTUnwrap(raw["threadId"] as? String, label))
        case "thread/replace-id":
            return .threadReplaceId(
                fromThreadId: try XCTUnwrap(raw["fromThreadId"] as? String, label),
                toThreadId: try XCTUnwrap(raw["toThreadId"] as? String, label)
            )
        case "thread/delete":
            return .threadDelete(threadId: try XCTUnwrap(raw["threadId"] as? String, label))
        default:
            XCTFail("\(label): unsupported action type \(type)")
            throw NSError(domain: "GaryxConversationStateConformanceTests", code: 1)
        }
    }

    private func fixtureIntent(
        from raw: [String: Any],
        label: String
    ) throws -> GaryxMessageIntent {
        GaryxMessageIntent(
            intentId: try XCTUnwrap(raw["intentId"] as? String, label),
            threadId: try XCTUnwrap(raw["threadId"] as? String, label),
            text: raw["text"] as? String ?? "",
            createdAt: Self.fixedNow,
            updatedAt: Self.fixedNow,
            state: try XCTUnwrap(
                (raw["state"] as? String).flatMap(GaryxIntentState.init(rawValue:)),
                label
            ),
            source: try XCTUnwrap(
                (raw["source"] as? String).flatMap(GaryxIntentSource.init(rawValue:)),
                label
            ),
            dispatchMode: (raw["dispatchMode"] as? String).flatMap(GaryxIntentDispatchMode.init(rawValue:)),
            remoteRunId: raw["remoteRunId"] as? String,
            pendingInputId: raw["pendingInputId"] as? String,
            responseText: raw["responseText"] as? String
        )
    }

    // MARK: Snapshot assertions

    private func assertSnapshot(
        _ state: GaryxConversationMachineState,
        _ expectation: [String: Any],
        label: String
    ) {
        if let intents = expectation["intents"] as? [String: Any] {
            for (intentId, expectedAny) in intents {
                if expectedAny is NSNull {
                    XCTAssertNil(state.intentsById[intentId], "\(label): intent \(intentId) should be absent")
                    continue
                }
                guard let intent = state.intentsById[intentId] else {
                    XCTFail("\(label): intent \(intentId) should exist")
                    continue
                }
                guard let expected = expectedAny as? [String: Any] else {
                    XCTFail("\(label): intent expectation for \(intentId) should be an object")
                    continue
                }
                for (field, value) in expected {
                    assertIntentField(intent, field: field, value: value, label: label)
                }
            }
        }
        if let queues = expectation["queues"] as? [String: Any] {
            for (threadId, queueAny) in queues {
                XCTAssertEqual(
                    state.queueByThread[threadId] ?? [],
                    queueAny as? [String] ?? [],
                    "\(label): queue for \(threadId)"
                )
            }
        }
        if let runtimes = expectation["runtimes"] as? [String: Any] {
            for (threadId, expectedAny) in runtimes {
                guard let expected = expectedAny as? [String: Any] else {
                    XCTFail("\(label): runtime expectation for \(threadId) should be an object")
                    continue
                }
                if (expected["exists"] as? Bool) == false {
                    XCTAssertNil(
                        state.threadRuntimeByThread[threadId],
                        "\(label): runtime \(threadId) should be absent"
                    )
                    continue
                }
                guard let runtime = state.threadRuntimeByThread[threadId] else {
                    XCTFail("\(label): runtime \(threadId) should exist")
                    continue
                }
                if let stateValue = expected["state"] as? String {
                    XCTAssertEqual(runtime.state.rawValue, stateValue, "\(label): runtime \(threadId).state")
                }
                if let busy = expected["busy"] as? Bool {
                    XCTAssertEqual(garyxIsRuntimeBusy(runtime.state), busy, "\(label): runtime \(threadId) busy")
                }
                if expected.keys.contains("activeIntentId") {
                    assertNullableString(
                        runtime.activeIntentId,
                        expected["activeIntentId"],
                        "\(label): runtime \(threadId).activeIntentId"
                    )
                }
                if expected.keys.contains("remoteRunId") {
                    assertNullableString(
                        runtime.remoteRunId,
                        expected["remoteRunId"],
                        "\(label): runtime \(threadId).remoteRunId"
                    )
                }
            }
        }
        if let phase = expectation["composerPhase"] as? String {
            XCTAssertEqual(state.composerPhase.rawValue, phase, "\(label): composerPhase")
        }
    }

    private func assertIntentField(
        _ intent: GaryxMessageIntent,
        field: String,
        value: Any,
        label: String
    ) {
        let actual: String?
        switch field {
        case "state": actual = intent.state.rawValue
        case "threadId": actual = intent.threadId
        case "text": actual = intent.text
        case "source": actual = intent.source.rawValue
        case "dispatchMode": actual = intent.dispatchMode?.rawValue
        case "remoteRunId": actual = intent.remoteRunId
        case "remoteThreadKey": actual = intent.remoteThreadKey
        case "pendingInputId": actual = intent.pendingInputId
        case "responseText": actual = intent.responseText
        case "error": actual = intent.error
        default:
            XCTFail("\(label): unsupported intent field \(field)")
            return
        }
        assertNullableString(actual, value, "\(label): intent \(intent.intentId).\(field)")
    }

    private func assertNullableString(_ actual: String?, _ expected: Any?, _ label: String) {
        if expected is NSNull {
            XCTAssertNil(actual, label)
            return
        }
        XCTAssertEqual(actual, expected as? String, label)
    }
}
