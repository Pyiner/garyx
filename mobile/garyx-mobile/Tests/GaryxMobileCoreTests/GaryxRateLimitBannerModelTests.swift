import XCTest
@testable import GaryxMobileCore

final class GaryxRateLimitBannerModelTests: XCTestCase {
    private func date(_ iso: String) -> Date {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.date(from: iso)!
    }

    func testNilRateLimitProducesNoBanner() {
        XCTAssertNil(GaryxRateLimitBannerModel.make(from: nil))
    }

    private let utc = TimeZone(identifier: "UTC")!

    func testPrimaryWindowAutoResendShowsResetClockAndCountdown() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex_app_server",
            resetAt: "2030-01-01T06:05:30+00:00",
            window: "primary",
            willAutoResend: true
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T06:00:00+00:00"),
            timeZone: utc
        )
        XCTAssertEqual(model?.title, "Codex 5-hour limit reached")
        XCTAssertEqual(model?.detail, "Auto-resend at 06:05 · 05:30 left")
        XCTAssertEqual(model?.isResending, false)
        XCTAssertEqual(model?.showContinue, false)
    }

    func testWeeklyWindowLabelIncludesDateWhenNotToday() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex",
            resetAt: "2030-01-08T00:00:00+00:00",
            window: "secondary",
            willAutoResend: true
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T00:00:00+00:00"),
            timeZone: utc
        )
        XCTAssertEqual(model?.title, "Codex weekly limit reached")
        XCTAssertEqual(model?.detail, "Auto-resend at Jan 8 00:00 · 168:00:00 left")
    }

    func testRecoveredWindowShowsResending() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex",
            resetAt: "2030-01-01T06:00:00+00:00",
            window: "primary",
            willAutoResend: true
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T06:00:05+00:00"),
            timeZone: utc
        )
        XCTAssertEqual(model?.detail, "Quota recovered — resending…")
        XCTAssertEqual(model?.isResending, true)
        XCTAssertEqual(model?.showContinue, false)
    }

    func testNoAutoResendShowsResetClockCountdownAndContinue() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex",
            resetAt: "2030-01-01T06:01:00+00:00",
            window: "primary",
            willAutoResend: false
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T06:00:00+00:00"),
            timeZone: utc
        )
        XCTAssertEqual(model?.detail, "Resets at 06:01 · 01:00 left")
        XCTAssertEqual(model?.isResending, false)
        XCTAssertEqual(model?.showContinue, true)
    }

    func testRecoveredWithoutAutoResendOffersContinue() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex",
            resetAt: "2030-01-01T06:00:00+00:00",
            window: "primary",
            willAutoResend: false
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T06:10:00+00:00"),
            timeZone: utc
        )
        XCTAssertEqual(
            model?.detail,
            "Reset at 06:00 — quota should be available again."
        )
        XCTAssertEqual(model?.showContinue, true)
    }

    func testMissingResetFallsBackToProviderMessage() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex_app_server",
            resetAt: nil,
            window: nil,
            message: "You've hit your usage limit. Visit https://example.com/usage to purchase more credits or try again at 9:42 PM.",
            willAutoResend: false
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T06:00:00+00:00"),
            timeZone: utc
        )
        XCTAssertEqual(model?.title, "Codex usage limit reached")
        XCTAssertEqual(
            model?.detail,
            "You've hit your usage limit. Visit https://example.com/usage to purchase more credits or try again at 9:42 PM."
        )
        XCTAssertEqual(model?.showContinue, true)
    }

    func testMissingResetAndMessageShowsTryAgainShortly() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex",
            resetAt: nil,
            window: nil,
            willAutoResend: false
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T06:00:00+00:00"),
            timeZone: utc
        )
        XCTAssertEqual(model?.detail, "Try again shortly.")
        XCTAssertEqual(model?.showContinue, true)
    }

    func testDecodesFromRenderSnapshotJSON() throws {
        let json = """
        {
            "based_on_seq": 12,
            "rows": [],
            "tailActivity": "none",
            "progress_locus": "none",
            "filtered_placeholders": [],
            "rateLimit": {
                "provider": "codex_app_server",
                "resetAt": "2030-01-01T06:00:00+00:00",
                "window": "primary",
                "message": "You've hit your usage limit.",
                "willAutoResend": true
            }
        }
        """
        let snapshot = try JSONDecoder().decode(
            GaryxRenderSnapshot.self,
            from: Data(json.utf8)
        )
        XCTAssertEqual(snapshot.rateLimit?.provider, "codex_app_server")
        XCTAssertEqual(snapshot.rateLimit?.window, "primary")
        XCTAssertEqual(snapshot.rateLimit?.willAutoResend, true)
    }

    func testSnapshotWithoutRateLimitDecodesToNil() throws {
        let json = """
        {
            "based_on_seq": 1,
            "rows": [],
            "tailActivity": "none",
            "progress_locus": "none",
            "filtered_placeholders": []
        }
        """
        let snapshot = try JSONDecoder().decode(
            GaryxRenderSnapshot.self,
            from: Data(json.utf8)
        )
        XCTAssertNil(snapshot.rateLimit)
    }
}
