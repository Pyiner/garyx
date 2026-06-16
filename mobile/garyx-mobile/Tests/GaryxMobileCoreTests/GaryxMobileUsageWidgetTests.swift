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
              "id": "gemini",
              "name": "Gemini",
              "available": false,
              "error": "no credentials"
            }
          ],
          "refreshed_at": "2030-01-01T00:00:00+00:00"
        }
        """
        let usage = try JSONDecoder().decode(GaryxCodingUsage.self, from: Data(json.utf8))
        XCTAssertEqual(usage.providers.count, 3)
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

        let gemini = try XCTUnwrap(usage.provider(id: "gemini"))
        XCTAssertFalse(gemini.available)
        XCTAssertNil(gemini.weekly)
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
    }

    func testResetSecondsPrefersResetsAtThenFallsBack() {
        let isoWindow = GaryxUsageWindow(
            usedPercent: 10,
            remainingPercent: 90,
            resetsAt: iso8601(from: referenceNow.addingTimeInterval(3_600))
        )
        XCTAssertEqual(GaryxUsageGaugeModel.resetSeconds(for: isoWindow, now: referenceNow), 3_600)

        let secondsWindow = GaryxUsageWindow(usedPercent: 10, remainingPercent: 90, resetAfterSeconds: 120)
        XCTAssertEqual(GaryxUsageGaugeModel.resetSeconds(for: secondsWindow, now: referenceNow), 120)

        let emptyWindow = GaryxUsageWindow(usedPercent: 10, remainingPercent: 90)
        XCTAssertNil(GaryxUsageGaugeModel.resetSeconds(for: emptyWindow, now: referenceNow))
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
