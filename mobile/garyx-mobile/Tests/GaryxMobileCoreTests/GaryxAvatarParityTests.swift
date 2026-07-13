import XCTest
@testable import GaryxMobileCore

final class GaryxAvatarParityTests: XCTestCase {
    func testSwiftStyleCatalogMatchesSharedParityFixture() throws {
        let fixture = try loadAvatarParityFixture()
        XCTAssertEqual(
            GaryxAvatarStyleOption.builtIn.map {
                AvatarParityFixture.Style(id: $0.id, label: $0.label, prompt: $0.prompt)
            },
            fixture.styles
        )
    }

    func testSwiftPromptCompositionMatchesSharedParityFixture() throws {
        let fixture = try loadAvatarParityFixture()
        for item in fixture.promptCases {
            XCTAssertEqual(
                GaryxAvatarPromptBuilder.prompt(
                    displayName: item.displayName,
                    identifier: item.identifier,
                    stylePrompt: item.stylePrompt
                ),
                item.expected
            )
        }
    }
}

private struct AvatarParityFixture: Decodable {
    struct Style: Codable, Equatable {
        let id: String
        let label: String
        let prompt: String
    }

    struct PromptCase: Decodable {
        let displayName: String
        let identifier: String
        let stylePrompt: String
        let expected: String
    }

    let styles: [Style]
    let promptCases: [PromptCase]
}

private func loadAvatarParityFixture() throws -> AvatarParityFixture {
    let url = try XCTUnwrap(
        Bundle.module.url(
            forResource: "agent-avatar-parity",
            withExtension: "json",
            subdirectory: "Fixtures"
        )
    )
    return try JSONDecoder().decode(AvatarParityFixture.self, from: Data(contentsOf: url))
}
