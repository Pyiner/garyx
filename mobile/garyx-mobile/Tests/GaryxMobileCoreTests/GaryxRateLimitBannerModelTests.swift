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

    func testPrimaryWindowAutoResendShowsCountdown() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex_app_server",
            resetAt: "2030-01-01T06:05:30+00:00",
            window: "primary",
            willAutoResend: true
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T06:00:00+00:00")
        )
        XCTAssertEqual(model?.title, "Codex 5-hour limit reached")
        XCTAssertEqual(model?.detail, "Auto-resend in 05:30")
        XCTAssertEqual(model?.isResending, false)
    }

    func testWeeklyWindowLabel() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex",
            resetAt: "2030-01-08T00:00:00+00:00",
            window: "secondary",
            willAutoResend: true
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T00:00:00+00:00")
        )
        XCTAssertEqual(model?.title, "Codex weekly limit reached")
        XCTAssertEqual(model?.detail, "Auto-resend in 168:00:00")
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
            now: date("2030-01-01T06:00:05+00:00")
        )
        XCTAssertEqual(model?.detail, "Quota recovered — resending…")
        XCTAssertEqual(model?.isResending, true)
    }

    func testNoAutoResendShowsPlainCountdown() {
        let rateLimit = GaryxRenderRateLimit(
            provider: "codex",
            resetAt: "2030-01-01T06:01:00+00:00",
            window: "primary",
            willAutoResend: false
        )
        let model = GaryxRateLimitBannerModel.make(
            from: rateLimit,
            now: date("2030-01-01T06:00:00+00:00")
        )
        XCTAssertEqual(model?.detail, "Resets in 01:00")
        XCTAssertEqual(model?.isResending, false)
    }

    func testDecodesFromRenderSnapshotJSON() throws {
        let json = """
        {
            "based_on_seq": 12,
            "rows": [],
            "tailActivity": "none",
            "progress_locus": "none",
            "visibleMessageIds": [],
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
            "visibleMessageIds": [],
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
