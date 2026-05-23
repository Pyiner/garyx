import XCTest
@testable import GaryxMobileCore

final class GaryxMobileLiveUpdateSupportTests: XCTestCase {
    func testChannelIconResolverUsesGatewayPluginCatalogCaseInsensitively() throws {
        let page = try JSONDecoder().decode(
            GaryxChannelPluginCatalogPage.self,
            from: Data(
                """
                {
                  "plugins": [
                    {
                      "id": "discord",
                      "display_name": "Discord",
                      "icon_data_url": " data:image/png;base64,ZGlzY29yZA== ",
                      "schema": {},
                      "config_methods": []
                    }
                  ]
                }
                """.utf8
            )
        )

        XCTAssertEqual(
            GaryxChannelIconResolver.iconDataUrl(for: " Discord ", plugins: page.plugins),
            "data:image/png;base64,ZGlzY29yZA=="
        )
    }

    func testChannelIconResolverPreservesGatewaySVGDataUrl() throws {
        let page = try JSONDecoder().decode(
            GaryxChannelPluginCatalogPage.self,
            from: Data(
                """
                {
                  "plugins": [
                    {
                      "id": "discord",
                      "display_name": "Discord",
                      "icon_data_url": "data:image/svg+xml;base64,PHN2Zy8+",
                      "schema": {},
                      "config_methods": []
                    }
                  ]
                }
                """.utf8
            )
        )

        XCTAssertEqual(
            GaryxChannelIconResolver.iconDataUrl(for: "discord", plugins: page.plugins),
            "data:image/svg+xml;base64,PHN2Zy8+"
        )
    }

    func testThreadActivitySignatureTracksPassiveTranscriptChanges() throws {
        let base = try decodeTranscript(
            """
            {
              "ok": true,
              "messages": [
                {
                  "index": 1,
                  "role": "user",
                  "text": "Start work",
                  "timestamp": "2026-05-23T10:00:00Z"
                }
              ],
              "pending_user_inputs": [],
              "thread_runtime": {
                "active_run": {
                  "run_id": "run-test",
                  "assistant_response": "",
                  "updated_at": "2026-05-23T10:00:01Z",
                  "pending_user_input_count": 0
                }
              }
            }
            """
        )
        let updatedResponse = try decodeTranscript(
            """
            {
              "ok": true,
              "messages": [
                {
                  "index": 1,
                  "role": "user",
                  "text": "Start work",
                  "timestamp": "2026-05-23T10:00:00Z"
                }
              ],
              "pending_user_inputs": [],
              "thread_runtime": {
                "active_run": {
                  "run_id": "run-test",
                  "assistant_response": "Working on it",
                  "updated_at": "2026-05-23T10:00:02Z",
                  "pending_user_input_count": 0
                }
              }
            }
            """
        )
        let completed = try decodeTranscript(
            """
            {
              "ok": true,
              "messages": [
                {
                  "index": 1,
                  "role": "user",
                  "text": "Start work",
                  "timestamp": "2026-05-23T10:00:00Z"
                },
                {
                  "index": 2,
                  "role": "assistant",
                  "text": "Done",
                  "timestamp": "2026-05-23T10:00:05Z"
                }
              ],
              "pending_user_inputs": [],
              "thread_runtime": null
            }
            """
        )

        XCTAssertNotEqual(
            GaryxThreadActivitySignature.make(from: base),
            GaryxThreadActivitySignature.make(from: updatedResponse)
        )
        XCTAssertNotEqual(
            GaryxThreadActivitySignature.make(from: updatedResponse),
            GaryxThreadActivitySignature.make(from: completed)
        )
    }

    private func decodeTranscript(_ json: String) throws -> GaryxThreadTranscript {
        try JSONDecoder().decode(GaryxThreadTranscript.self, from: Data(json.utf8))
    }
}
