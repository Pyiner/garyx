import XCTest
@testable import GaryxMobileCore

final class GaryxMobileUsageWidgetTests: XCTestCase {
    private let referenceNow = Date(timeIntervalSince1970: 1_900_000_000)

    // MARK: - Decoding the gateway payload

    func testDecodesGatewayUsagePayload() throws {
        let json = """
        {
          "providers": [
            {
              "id": "claude_code",
              "name": "Claude Code",
              "available": true,
              "plan": "max",
              "weekly": {"used_percent": 27.0, "remaining_percent": 73.0, "resets_at": "2030-01-07T11:00:00+00:00"},
              "session": {"used_percent": 11.0, "remaining_percent": 89.0, "resets_at": "2030-01-01T05:00:00+00:00"}
            },
            {
              "id": "codex",
              "name": "Codex",
              "available": true,
              "stale": true,
              "plan": "pro",
              "weekly": {"used_percent": 89.0, "remaining_percent": 11.0, "reset_after_seconds": 140803},
              "error": "Codex usage request returned HTTP 500"
            },
            {
              "id": "other",
              "name": "Other",
              "available": false,
              "error": "no credentials"
            },
            {
              "id": "antigravity",
              "name": "Antigravity",
              "available": true,
              "models": [
                {
                  "id": "claude-opus-4-6-thinking",
                  "name": "Claude Opus 4.6 (Thinking)",
                  "remaining_fraction": 0.985,
                  "remaining_percent": 98.5,
                  "used_percent": 1.5,
                  "resets_at": "2030-01-01T08:00:00Z",
                  "reset_after_seconds": 3600,
                  "description": "Quota resets in 1 hour."
                }
              ]
            }
          ],
          "refreshed_at": "2030-01-01T00:00:00+00:00"
        }
        """
        let usage = try JSONDecoder().decode(GaryxCodingUsage.self, from: Data(json.utf8))
        XCTAssertEqual(usage.providers.count, 4)
        XCTAssertEqual(usage.refreshedAt, "2030-01-01T00:00:00+00:00")

        let claude = try XCTUnwrap(usage.provider(id: "claude_code"))
        XCTAssertTrue(claude.available)
        XCTAssertFalse(claude.stale)
        XCTAssertEqual(claude.plan, "max")
        XCTAssertEqual(claude.weekly?.usedPercent, 27.0)
        XCTAssertEqual(claude.weekly?.remainingPercent, 73.0)
        XCTAssertEqual(claude.weekly?.resetsAt, "2030-01-07T11:00:00+00:00")
        XCTAssertEqual(claude.session?.remainingPercent, 89.0)

        let codex = try XCTUnwrap(usage.provider(id: "codex"))
        XCTAssertTrue(codex.stale)
        XCTAssertEqual(codex.weekly?.remainingPercent, 11.0)
        XCTAssertEqual(codex.weekly?.resetAfterSeconds, 140_803)
        XCTAssertEqual(codex.error, "Codex usage request returned HTTP 500")

        let other = try XCTUnwrap(usage.provider(id: "other"))
        XCTAssertFalse(other.available)
        XCTAssertNil(other.weekly)

        let antigravity = try XCTUnwrap(usage.provider(id: "antigravity"))
        XCTAssertTrue(antigravity.available)
        XCTAssertNil(antigravity.weekly)
        XCTAssertEqual(antigravity.models.count, 1)
        XCTAssertEqual(antigravity.models[0].name, "Claude Opus 4.6 (Thinking)")
        XCTAssertEqual(antigravity.models[0].remainingPercent, 98.5)
        XCTAssertEqual(antigravity.models[0].description, "Quota resets in 1 hour.")
    }

    func testDecodesWindowRemainingFallbackFromUsed() throws {
        let json = #"{"used_percent": 40}"#
        let window = try JSONDecoder().decode(GaryxUsageWindow.self, from: Data(json.utf8))
        XCTAssertEqual(window.usedPercent, 40)
        XCTAssertEqual(window.remainingPercent, 60)
    }

    // MARK: - Gauge presentation

    func testGaugeModelForAvailableProvider() {
        let provider = GaryxProviderUsage(
            id: "claude_code",
            name: "Claude Code",
            available: true,
            weekly: GaryxUsageWindow(usedPercent: 27, remainingPercent: 73, resetAfterSeconds: 2 * 86_400)
        )
        let model = GaryxUsageGaugeModel.make(from: provider, now: referenceNow)
        XCTAssertTrue(model.available)
        XCTAssertEqual(model.title, "Claude Code")
        XCTAssertEqual(model.remainingText, "73%")
        XCTAssertEqual(model.fillFraction, 0.73, accuracy: 0.0001)
        XCTAssertEqual(model.level, .healthy)
        XCTAssertEqual(model.detailText, "resets in 2d")
        XCTAssertNotNil(model.symbolName)
    }

    func testGaugeModelForStaleCriticalProvider() {
        let provider = GaryxProviderUsage(
            id: "codex",
            name: "Codex",
            available: true,
            stale: true,
            weekly: GaryxUsageWindow(usedPercent: 89, remainingPercent: 11, resetAfterSeconds: 100)
        )
        let model = GaryxUsageGaugeModel.make(from: provider, now: referenceNow)
        XCTAssertEqual(model.remainingText, "11%")
        XCTAssertEqual(model.level, .critical)
        XCTAssertEqual(model.detailText, "stale data")
    }

    func testGaugeModelForUnavailableProvider() {
        let provider = GaryxProviderUsage(
            id: "codex",
            name: "Codex",
            available: false,
            error: "Codex auth missing tokens.access_token"
        )
        let model = GaryxUsageGaugeModel.make(from: provider, now: referenceNow)
        XCTAssertFalse(model.available)
        XCTAssertEqual(model.remainingText, "—")
        XCTAssertEqual(model.level, .unavailable)
        XCTAssertEqual(model.detailText, "Unavailable")
        XCTAssertEqual(model.fillFraction, 0)
    }

    func testGaugeModelAvailableButMissingWeeklyIsUnavailable() {
        let provider = GaryxProviderUsage(id: "codex", name: "Codex", available: true)
        let model = GaryxUsageGaugeModel.make(from: provider, now: referenceNow)
        XCTAssertFalse(model.available)
        XCTAssertEqual(model.detailText, "No data")
    }

    func testLevelThresholds() {
        XCTAssertEqual(GaryxUsageGaugeModel.level(forRemaining: 100), .healthy)
        XCTAssertEqual(GaryxUsageGaugeModel.level(forRemaining: 50), .healthy)
        XCTAssertEqual(GaryxUsageGaugeModel.level(forRemaining: 49), .warning)
        XCTAssertEqual(GaryxUsageGaugeModel.level(forRemaining: 20), .warning)
        XCTAssertEqual(GaryxUsageGaugeModel.level(forRemaining: 19), .critical)
        XCTAssertEqual(GaryxUsageGaugeModel.level(forRemaining: 0), .critical)
    }

    func testFormatDuration() {
        XCTAssertEqual(GaryxUsageGaugeModel.formatDuration(2 * 86_400), "2d")
        XCTAssertEqual(GaryxUsageGaugeModel.formatDuration(5 * 3_600), "5h")
        XCTAssertEqual(GaryxUsageGaugeModel.formatDuration(12 * 60), "12m")
        XCTAssertEqual(GaryxUsageGaugeModel.formatDuration(30), "<1m")
        XCTAssertEqual(GaryxUsageGaugeModel.formatDuration(-100), "<1m")
        // Two-segment precision per the shared §4 spec (`2d 4h`, `1h 12m`),
        // matching the desktop formatUsageDuration.
        XCTAssertEqual(GaryxUsageGaugeModel.formatDuration(2 * 86_400 + 4 * 3_600), "2d 4h")
        XCTAssertEqual(GaryxUsageGaugeModel.formatDuration(3_600 + 12 * 60), "1h 12m")
        XCTAssertEqual(GaryxUsageGaugeModel.formatDuration(2 * 86_400 + 59 * 60), "2d")
    }

    func testResetSecondsUsesEitherSourceAndPrefersConservativeMinimum() {
        let isoWindow = GaryxUsageWindow(
            usedPercent: 10,
            remainingPercent: 90,
            resetsAt: iso8601(from: referenceNow.addingTimeInterval(3_600))
        )
        XCTAssertEqual(GaryxUsageGaugeModel.resetSeconds(for: isoWindow, now: referenceNow), 3_600)

        let secondsWindow = GaryxUsageWindow(usedPercent: 10, remainingPercent: 90, resetAfterSeconds: 120)
        XCTAssertEqual(GaryxUsageGaugeModel.resetSeconds(for: secondsWindow, now: referenceNow), 120)

        // When both sources are present and disagree, prefer the shorter
        // conservative value (design §4), whichever side it comes from.
        let secondsShorter = GaryxUsageWindow(
            usedPercent: 10,
            remainingPercent: 90,
            resetsAt: iso8601(from: referenceNow.addingTimeInterval(7_200)),
            resetAfterSeconds: 600
        )
        XCTAssertEqual(GaryxUsageGaugeModel.resetSeconds(for: secondsShorter, now: referenceNow), 600)

        let isoShorter = GaryxUsageWindow(
            usedPercent: 10,
            remainingPercent: 90,
            resetsAt: iso8601(from: referenceNow.addingTimeInterval(600)),
            resetAfterSeconds: 7_200
        )
        XCTAssertEqual(GaryxUsageGaugeModel.resetSeconds(for: isoShorter, now: referenceNow), 600)

        let emptyWindow = GaryxUsageWindow(usedPercent: 10, remainingPercent: 90)
        XCTAssertNil(GaryxUsageGaugeModel.resetSeconds(for: emptyWindow, now: referenceNow))
    }

    func testUsageUpdatedText() {
        XCTAssertEqual(
            GaryxUsageGaugeModel.usageUpdatedText(
                refreshedAt: iso8601(from: referenceNow.addingTimeInterval(-3 * 60)),
                now: referenceNow
            ),
            "updated 3m ago"
        )
        XCTAssertNil(GaryxUsageGaugeModel.usageUpdatedText(refreshedAt: nil, now: referenceNow))
        XCTAssertNil(GaryxUsageGaugeModel.usageUpdatedText(refreshedAt: "not a date", now: referenceNow))
    }

    func testFillFractionClampsOutOfRange() {
        let provider = GaryxProviderUsage(
            id: "claude_code",
            name: "Claude Code",
            available: true,
            weekly: GaryxUsageWindow(usedPercent: -50, remainingPercent: 150)
        )
        let model = GaryxUsageGaugeModel.make(from: provider, now: referenceNow)
        XCTAssertEqual(model.fillFraction, 1.0, accuracy: 0.0001)
        XCTAssertEqual(model.remainingText, "100%")
    }

    func testWidgetModelsOnlyShowsClaudeCodeAndCodexInOrder() {
        let usage = GaryxCodingUsage(providers: [
            GaryxProviderUsage(
                id: "antigravity",
                name: "Antigravity",
                available: true,
                models: [
                    GaryxModelUsage(
                        id: "claude-opus-4-6-thinking",
                        name: "Claude Opus 4.6 (Thinking)",
                        remainingFraction: 0.98,
                        remainingPercent: 98,
                        usedPercent: 2
                    )
                ]
            ),
            GaryxProviderUsage(
                id: "codex",
                name: "Codex",
                available: true,
                weekly: GaryxUsageWindow(usedPercent: 20, remainingPercent: 80)
            ),
            GaryxProviderUsage(
                id: "claude_code",
                name: "Claude Code",
                available: true,
                weekly: GaryxUsageWindow(usedPercent: 10, remainingPercent: 90)
            ),
        ])

        let models = GaryxUsageGaugeModel.widgetModels(from: usage, now: referenceNow)
        XCTAssertEqual(models.map(\.providerId), ["claude_code", "codex"])
        XCTAssertEqual(models.map(\.remainingText), ["90%", "80%"])
    }

    func testWidgetModelsDoesNotFallbackToAntigravityOnlyUsage() {
        let usage = GaryxCodingUsage(providers: [
            GaryxProviderUsage(
                id: "antigravity",
                name: "Antigravity",
                available: true,
                models: [
                    GaryxModelUsage(
                        id: "claude-opus-4-6-thinking",
                        name: "Claude Opus 4.6 (Thinking)",
                        remainingFraction: 0.98,
                        remainingPercent: 98,
                        usedPercent: 2
                    )
                ]
            ),
        ])

        XCTAssertTrue(GaryxUsageGaugeModel.widgetModels(from: usage, now: referenceNow).isEmpty)
    }

    func testHeroModelsKeepFixedProviderColumnsWithPlaceholders() {
        let usage = GaryxCodingUsage(providers: [
            GaryxProviderUsage(
                id: "claude_code",
                name: "Claude Code",
                available: true,
                weekly: GaryxUsageWindow(usedPercent: 27, remainingPercent: 73, resetAfterSeconds: 2 * 86_400)
            ),
        ])

        let models = GaryxUsageGaugeModel.heroModels(from: usage, now: referenceNow)
        XCTAssertEqual(models.map(\.providerId), ["claude_code", "codex", "antigravity"])
        XCTAssertEqual(models[0].remainingText, "73%")
        XCTAssertTrue(models[0].available)
        // Missing providers hold their column as "No data" placeholders with
        // presentation-resolved titles, so the hero layout stays stable.
        XCTAssertEqual(models[1].title, "Codex")
        XCTAssertEqual(models[2].title, "Antigravity")
        for placeholder in models.dropFirst() {
            XCTAssertFalse(placeholder.available)
            XCTAssertEqual(placeholder.remainingText, "—")
            XCTAssertEqual(placeholder.detailText, "No data")
            XCTAssertEqual(placeholder.level, .unavailable)
        }

        let emptyModels = GaryxUsageGaugeModel.heroModels(from: nil, now: referenceNow)
        XCTAssertEqual(emptyModels.map(\.providerId), ["claude_code", "codex", "antigravity"])
        XCTAssertTrue(emptyModels.allSatisfy { !$0.available })
    }

    func testHeroModelAntigravityGaugesTightestModelBucket() {
        let provider = GaryxProviderUsage(
            id: "antigravity",
            name: "Antigravity",
            available: true,
            models: [
                GaryxModelUsage(
                    id: "gemini-3-flash",
                    name: "Gemini 3 Flash",
                    remainingFraction: 0.84,
                    remainingPercent: 84,
                    usedPercent: 16
                ),
                GaryxModelUsage(
                    id: "claude-opus-4-6-thinking",
                    name: "Claude Opus 4.6 (Thinking)",
                    remainingFraction: 0.15,
                    remainingPercent: 15,
                    usedPercent: 85,
                    resetAfterSeconds: 3_600
                ),
            ]
        )

        let model = GaryxUsageGaugeModel.heroModel(from: provider, now: referenceNow)
        XCTAssertTrue(model.available)
        XCTAssertEqual(model.title, "Antigravity")
        XCTAssertEqual(model.remainingText, "15%")
        XCTAssertEqual(model.fillFraction, 0.15, accuracy: 0.0001)
        XCTAssertEqual(model.level, .critical)
        XCTAssertEqual(model.detailText, "resets in 1h")
        XCTAssertFalse(model.stale)
    }

    func testHeroModelAntigravityStaleReadsStaleData() {
        let provider = GaryxProviderUsage(
            id: "antigravity",
            name: "Antigravity",
            available: true,
            stale: true,
            models: [
                GaryxModelUsage(
                    id: "gemini-3-flash",
                    name: "Gemini 3 Flash",
                    remainingFraction: 0.84,
                    remainingPercent: 84,
                    usedPercent: 16,
                    resetAfterSeconds: 3_600
                ),
            ]
        )

        let model = GaryxUsageGaugeModel.heroModel(from: provider, now: referenceNow)
        XCTAssertEqual(model.detailText, "stale data")
        XCTAssertTrue(model.stale)
    }

    func testHeroModelPrefersWeeklyWindowOverModelBuckets() {
        let provider = GaryxProviderUsage(
            id: "claude_code",
            name: "Claude Code",
            available: true,
            weekly: GaryxUsageWindow(usedPercent: 27, remainingPercent: 73, resetAfterSeconds: 2 * 86_400),
            models: [
                GaryxModelUsage(
                    id: "claude-opus-4-6",
                    name: "Claude Opus 4.6",
                    remainingFraction: 0.05,
                    remainingPercent: 5,
                    usedPercent: 95
                ),
            ]
        )

        let model = GaryxUsageGaugeModel.heroModel(from: provider, now: referenceNow)
        XCTAssertEqual(model.remainingText, "73%")
        XCTAssertEqual(model.detailText, "resets in 2d")
    }

    func testHeroModelUnavailableAndBucketlessFallsBackToMake() {
        let unavailable = GaryxProviderUsage(id: "codex", name: "Codex", available: false)
        XCTAssertEqual(GaryxUsageGaugeModel.heroModel(from: unavailable, now: referenceNow).level, .unavailable)

        let bucketless = GaryxProviderUsage(id: "antigravity", name: "Antigravity", available: true)
        let model = GaryxUsageGaugeModel.heroModel(from: bucketless, now: referenceNow)
        XCTAssertFalse(model.available)
        XCTAssertEqual(model.detailText, "No data")
    }

    func testGaugeModelCarriesStaleFlag() {
        let stale = GaryxProviderUsage(
            id: "codex",
            name: "Codex",
            available: true,
            stale: true,
            weekly: GaryxUsageWindow(usedPercent: 89, remainingPercent: 11)
        )
        XCTAssertTrue(GaryxUsageGaugeModel.make(from: stale, now: referenceNow).stale)

        let fresh = GaryxProviderUsage(
            id: "codex",
            name: "Codex",
            available: true,
            weekly: GaryxUsageWindow(usedPercent: 89, remainingPercent: 11)
        )
        XCTAssertFalse(GaryxUsageGaugeModel.make(from: fresh, now: referenceNow).stale)
    }

    func testProviderUsageDisplayModelsAntigravityBuckets() throws {
        let provider = GaryxProviderUsage(
            id: "antigravity",
            name: "Antigravity",
            available: true,
            models: [
                GaryxModelUsage(
                    id: "claude-opus-4-6-thinking",
                    name: "Claude Opus 4.6 (Thinking)",
                    remainingFraction: 0.985,
                    remainingPercent: 98.5,
                    usedPercent: 1.5,
                    description: "Quota resets in 1 hour."
                )
            ]
        )

        let display = try XCTUnwrap(GaryxProviderUsageDisplayModel.make(from: provider, now: referenceNow))
        XCTAssertEqual(display.summaryText, "1 model quota")
        XCTAssertEqual(display.detailText, "Per-model quota")
        XCTAssertEqual(display.models.count, 1)
        XCTAssertEqual(display.models[0].title, "Claude Opus 4.6 (Thinking)")
        XCTAssertEqual(display.models[0].remainingText, "99% left")
        XCTAssertEqual(display.models[0].detailText, "Quota resets in 1 hour.")
        XCTAssertEqual(display.models[0].remainingPercent, 98.5, accuracy: 0.0001)
        XCTAssertTrue(display.windows.isEmpty)
    }

    func testProviderUsageDisplayModelSurfacesPlanSessionAndStale() throws {
        let provider = GaryxProviderUsage(
            id: "claude_code",
            name: "Claude Code",
            available: true,
            stale: true,
            plan: " max ",
            weekly: GaryxUsageWindow(usedPercent: 27, remainingPercent: 73, resetAfterSeconds: 2 * 86_400 + 4 * 3_600),
            session: GaryxUsageWindow(usedPercent: 89, remainingPercent: 11, resetAfterSeconds: 3_600 + 12 * 60)
        )

        let display = try XCTUnwrap(
            GaryxProviderUsageDisplayModel.make(
                from: provider,
                refreshedAt: iso8601(from: referenceNow.addingTimeInterval(-3 * 60)),
                now: referenceNow
            )
        )
        XCTAssertEqual(display.plan, "max")
        XCTAssertTrue(display.stale)
        XCTAssertEqual(display.updatedText, "updated 3m ago")
        XCTAssertEqual(display.windows.map(\.label), ["Session", "Weekly"])

        let session = display.windows[0]
        XCTAssertEqual(session.remainingPercent, 11, accuracy: 0.0001)
        XCTAssertEqual(session.remainingText, "11%")
        XCTAssertEqual(session.detailText, "resets in 1h 12m")
        XCTAssertEqual(session.level, .critical)

        let weekly = display.windows[1]
        XCTAssertEqual(weekly.remainingPercent, 73, accuracy: 0.0001)
        XCTAssertEqual(weekly.remainingText, "73%")
        XCTAssertEqual(weekly.detailText, "resets in 2d 4h")
        XCTAssertEqual(weekly.level, .healthy)

        // Summary keeps the weekly-first legacy semantics for existing callers,
        // and the stale flag folds into the caption as before.
        XCTAssertEqual(display.summaryText, "73% left")
        XCTAssertEqual(display.detailText, "stale data")
    }

    func testProviderUsageDisplayModelSortsModelBucketsTightestFirst() throws {
        let provider = GaryxProviderUsage(
            id: "antigravity",
            name: "Antigravity",
            available: true,
            models: [
                GaryxModelUsage(
                    id: "gemini-3-flash",
                    name: "Gemini 3 Flash",
                    remainingFraction: 0.84,
                    remainingPercent: 84,
                    usedPercent: 16
                ),
                GaryxModelUsage(
                    id: "claude-opus-4-6-thinking",
                    name: "Claude Opus 4.6 (Thinking)",
                    remainingFraction: 0.12,
                    remainingPercent: 12,
                    usedPercent: 88
                ),
            ]
        )

        let display = try XCTUnwrap(GaryxProviderUsageDisplayModel.make(from: provider, now: referenceNow))
        XCTAssertEqual(display.models.map(\.id), ["claude-opus-4-6-thinking", "gemini-3-flash"])
        XCTAssertEqual(display.models[0].level, .critical)
        XCTAssertEqual(display.models[1].level, .healthy)
        XCTAssertEqual(display.summaryText, "2 model quotas")
    }

    func testProviderUsageDisplayModelSessionOnlyWindowStillRenders() throws {
        let provider = GaryxProviderUsage(
            id: "codex",
            name: "Codex",
            available: true,
            session: GaryxUsageWindow(usedPercent: 40, remainingPercent: 60, resetAfterSeconds: 900)
        )

        let display = try XCTUnwrap(GaryxProviderUsageDisplayModel.make(from: provider, now: referenceNow))
        XCTAssertTrue(display.available)
        XCTAssertEqual(display.windows.map(\.label), ["Session"])
        XCTAssertEqual(display.summaryText, "60% left")
        XCTAssertEqual(display.detailText, "resets in 15m")
    }

    func testProviderUsageDisplayModelUnavailableCarriesPlanAndStale() throws {
        let provider = GaryxProviderUsage(
            id: "codex",
            name: "Codex",
            available: false,
            stale: true,
            plan: "pro",
            error: "Codex usage request returned HTTP 500"
        )

        let display = try XCTUnwrap(GaryxProviderUsageDisplayModel.make(from: provider, now: referenceNow))
        XCTAssertFalse(display.available)
        XCTAssertEqual(display.summaryText, "Unavailable")
        XCTAssertEqual(display.plan, "pro")
        XCTAssertTrue(display.stale)
        XCTAssertTrue(display.windows.isEmpty)
    }

    // MARK: - Store + snapshot

    func testSnapshotRoundTripStoresOnlyUsageNoToken() throws {
        let suite = "GaryxUsageWidgetTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }

        XCTAssertTrue(GaryxUsageWidgetStore.loadSnapshot(defaults: defaults).isEmpty)

        let snapshot = GaryxUsageWidgetSnapshot(
            usage: GaryxCodingUsage(providers: [
                GaryxProviderUsage(
                    id: "claude_code",
                    name: "Claude Code",
                    available: true,
                    weekly: GaryxUsageWindow(usedPercent: 27, remainingPercent: 73)
                )
            ]),
            fetchedAt: referenceNow
        )
        GaryxUsageWidgetStore.saveSnapshot(snapshot, defaults: defaults)
        XCTAssertEqual(GaryxUsageWidgetStore.loadSnapshot(defaults: defaults), snapshot)

        GaryxUsageWidgetStore.clear(defaults: defaults)
        XCTAssertTrue(GaryxUsageWidgetStore.loadSnapshot(defaults: defaults).isEmpty)
    }

    func testSnapshotAgeText() {
        let snapshot = GaryxUsageWidgetSnapshot(
            usage: GaryxCodingUsage(providers: []),
            fetchedAt: referenceNow
        )
        XCTAssertEqual(snapshot.ageText(asOf: referenceNow.addingTimeInterval(30)), "updated just now")
        XCTAssertEqual(snapshot.ageText(asOf: referenceNow.addingTimeInterval(180)), "updated 3m ago")
        XCTAssertEqual(snapshot.ageText(asOf: referenceNow.addingTimeInterval(7_200)), "updated 2h ago")
        XCTAssertNil(GaryxUsageWidgetSnapshot.empty.ageText(asOf: referenceNow))
    }

    // MARK: - Helpers

    private func iso8601(from date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.string(from: date)
    }
}
