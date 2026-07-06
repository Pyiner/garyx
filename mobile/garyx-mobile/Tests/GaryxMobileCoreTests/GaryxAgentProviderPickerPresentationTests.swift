import XCTest
@testable import GaryxMobileCore

final class GaryxAgentProviderPickerPresentationTests: XCTestCase {
    // Pins the exact ids, labels, and order the picker offered before the
    // table moved into Core (labels previously lived as view-local literals).
    func testStandardOptionsPinIdsLabelsAndOrder() {
        let options = GaryxAgentProviderPickerPresentation.standardOptions
        XCTAssertEqual(options.map(\.id), [
            "claude_code",
            "codex_app_server",
            "traex",
            "gemini_cli",
            "gpt",
            "anthropic",
            "google",
        ])
        XCTAssertEqual(options.map(\.label), [
            "Claude Code",
            "Codex",
            "Traex",
            "Gemini CLI",
            "OpenAI",
            "Anthropic",
            "Google",
        ])
    }

    func testStandardOptionLabelsMatchSharedProviderDisplayName() {
        for option in GaryxAgentProviderPickerPresentation.standardOptions {
            XCTAssertEqual(option.label, GaryxProviderPresentation.displayName(for: option.id))
        }
    }

    func testOptionsWithoutCurrentProviderReturnStandardTable() {
        XCTAssertEqual(
            GaryxAgentProviderPickerPresentation.options(includingCurrent: nil),
            GaryxAgentProviderPickerPresentation.standardOptions
        )
        XCTAssertEqual(
            GaryxAgentProviderPickerPresentation.options(includingCurrent: ""),
            GaryxAgentProviderPickerPresentation.standardOptions
        )
        XCTAssertEqual(
            GaryxAgentProviderPickerPresentation.options(includingCurrent: "  \n"),
            GaryxAgentProviderPickerPresentation.standardOptions
        )
    }

    func testOptionsWithStandardCurrentProviderDoNotDuplicate() {
        for id in GaryxAgentProviderPickerPresentation.standardProviderIds {
            XCTAssertEqual(
                GaryxAgentProviderPickerPresentation.options(includingCurrent: id),
                GaryxAgentProviderPickerPresentation.standardOptions
            )
        }
        // Surrounding whitespace is trimmed before the table lookup.
        XCTAssertEqual(
            GaryxAgentProviderPickerPresentation.options(includingCurrent: " gpt "),
            GaryxAgentProviderPickerPresentation.standardOptions
        )
    }

    func testOptionsPrependNonStandardCurrentProvider() {
        let options = GaryxAgentProviderPickerPresentation.options(includingCurrent: "antigravity")
        XCTAssertEqual(options.count, 8)
        XCTAssertEqual(options.first, GaryxAgentProviderPickerOption(id: "antigravity", label: "Antigravity"))
        XCTAssertEqual(Array(options.dropFirst()), GaryxAgentProviderPickerPresentation.standardOptions)
    }

    func testLabelForEmptyProviderIsChoosePlaceholder() {
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: ""), "Choose provider")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "  \n"), "Choose provider")
    }

    func testLabelForStandardProviders() {
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "claude_code"), "Claude Code")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "codex_app_server"), "Codex")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "traex"), "Traex")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "gemini_cli"), "Gemini CLI")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "gpt"), "OpenAI")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "anthropic"), "Anthropic")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "google"), "Google")
    }

    func testLabelForNonStandardProvidersFallsBackToSharedDisplayName() {
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "antigravity"), "Antigravity")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "my_provider"), "My Provider")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: " claude_code "), "Claude Code")
    }

    // The table match is an exact, case-sensitive id comparison: case variants
    // miss the table and resolve through the shared display-name fallback.
    func testLabelMatchingIsCaseSensitive() {
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "GPT"), "GPT")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "Claude_Code"), "Claude Code")
    }

    func testOptionsMatchingIsCaseSensitive() {
        let gpt = GaryxAgentProviderPickerPresentation.options(includingCurrent: "GPT")
        XCTAssertEqual(gpt.count, 8)
        XCTAssertEqual(gpt.first, GaryxAgentProviderPickerOption(id: "GPT", label: "GPT"))

        let claude = GaryxAgentProviderPickerPresentation.options(includingCurrent: "Claude_Code")
        XCTAssertEqual(claude.count, 8)
        // The label text coincides with the standard claude_code entry, but the
        // id is the case variant and it goes through the fallback branch.
        XCTAssertEqual(claude.first, GaryxAgentProviderPickerOption(id: "Claude_Code", label: "Claude Code"))
    }
}
