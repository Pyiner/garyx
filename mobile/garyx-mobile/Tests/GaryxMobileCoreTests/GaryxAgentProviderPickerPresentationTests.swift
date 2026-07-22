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
            "antigravity",
            "grok_build",
        ])
        XCTAssertEqual(options.map(\.label), [
            "Claude Code",
            "Codex",
            "Traex",
            "Antigravity",
            "Grok",
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
            GaryxAgentProviderPickerPresentation.options(includingCurrent: " antigravity "),
            GaryxAgentProviderPickerPresentation.standardOptions
        )
    }

    func testOptionsPrependNonStandardCurrentProvider() {
        let options = GaryxAgentProviderPickerPresentation.options(includingCurrent: "custom_provider")
        XCTAssertEqual(options.count, 6)
        XCTAssertEqual(options.first, GaryxAgentProviderPickerOption(id: "custom_provider", label: "Custom Provider"))
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
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "antigravity"), "Antigravity")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "grok_build"), "Grok")
    }

    func testLabelForNonStandardProvidersFallsBackToSharedDisplayName() {
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "my_provider"), "My Provider")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: " claude_code "), "Claude Code")
    }

    // Picker ids remain exact, but every label resolves through the shared
    // provider presentation so known case aliases keep the canonical brand.
    func testLabelFallbackCanonicalizesKnownProviderCaseAliases() {
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "TRAE"), "Traex")
        XCTAssertEqual(GaryxAgentProviderPickerPresentation.label(for: "Claude_Code"), "Claude Code")
    }

    func testOptionsKeepCaseVariantIdsButUseSharedLabels() {
        let traex = GaryxAgentProviderPickerPresentation.options(includingCurrent: "TRAE")
        XCTAssertEqual(traex.count, 6)
        XCTAssertEqual(traex.first, GaryxAgentProviderPickerOption(id: "TRAE", label: "Traex"))

        let claude = GaryxAgentProviderPickerPresentation.options(includingCurrent: "Claude_Code")
        XCTAssertEqual(claude.count, 6)
        // The id stays the case variant while the label uses the same canonical
        // presentation as the standard claude_code entry.
        XCTAssertEqual(claude.first, GaryxAgentProviderPickerOption(id: "Claude_Code", label: "Claude Code"))
    }
}
