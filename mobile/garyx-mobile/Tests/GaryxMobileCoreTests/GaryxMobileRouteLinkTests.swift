import XCTest
@testable import GaryxMobileCore

final class GaryxMobileRouteLinkTests: XCTestCase {
    func testCanonicalRoutesRoundTrip() throws {
        let routes: [GaryxMobileRoute] = [
            .chat,
            .thread("thread-1"),
            .settings(.manage),
            .settings(.gateway),
            .settings(.provider),
            .settings(.channels),
            .settings(.commands),
            .settings(.mcp),
            .panel(.automations),
            .panel(.capsules),
            .capsule("01900000-0000-7000-8000-000000000001"),
            .automation("automation-1"),
            .automationThreads("automation-1"),
            .panel(.agents),
            .agent("agent-1"),
            .panel(.skills),
            .skill("skill-1"),
            .skillFile(skillId: "skill-1", path: "SKILL.md"),
            .panel(.workspaceBots),
            .workspace("/tmp/workspace"),
            .bot(channel: "channel-a", accountId: "1000000001"),
            .workspaceFile(workspaceDir: "/tmp/workspace", path: "docs/index.html"),
        ]

        for route in routes {
            let url = try XCTUnwrap(GaryxMobileRouteLink.make(route))
            XCTAssertEqual(GaryxMobileRouteLink.parse(url), route, "failed round trip for \(url)")
        }
    }

    func testLegacyThreadLinksStillParseAsRoutes() throws {
        let widgetURL = try XCTUnwrap(GaryxMobileThreadLink.make(threadId: "thread-1"))
        XCTAssertEqual(GaryxMobileRouteLink.parse(widgetURL), .thread("thread-1"))

        let hostURL = try XCTUnwrap(URL(string: "garyx://thread?thread_id=thread-2"))
        XCTAssertEqual(GaryxMobileRouteLink.parse(hostURL), .thread("thread-2"))
    }

    func testProviderSettingsWidgetLinkMatchesCanonicalRoute() throws {
        // The usage widget's whole-widget deep link (design §8/D7) must stay
        // byte-identical to the canonical provider settings route and land on
        // .settings(.provider).
        let widgetURL = try XCTUnwrap(GaryxMobileProviderSettingsLink.make())
        XCTAssertEqual(widgetURL.absoluteString, "garyx://mobile/settings/provider")
        XCTAssertEqual(widgetURL, GaryxMobileRouteLink.make(.settings(.provider)))
        XCTAssertEqual(GaryxMobileRouteLink.parse(widgetURL), .settings(.provider))
    }

    func testConnectLinksAreNotRouteLinks() throws {
        let url = try XCTUnwrap(URL(string: "garyx://mobile/connect?gatewayUrl=http%3A%2F%2F127.0.0.1%3A31337"))
        XCTAssertNil(GaryxMobileRouteLink.parse(url))
    }

    func testDetailRouteQueryAliases() throws {
        XCTAssertEqual(
            GaryxMobileRouteLink.parse(try XCTUnwrap(URL(string: "garyx://mobile/skill-file?skill_id=skill-1&file_path=docs%2Ffile.md"))),
            .skillFile(skillId: "skill-1", path: "docs/file.md")
        )
        XCTAssertEqual(
            GaryxMobileRouteLink.parse(try XCTUnwrap(URL(string: "garyx://mobile/workspace-bots?automation_id=automation-1"))),
            .automationThreads("automation-1")
        )
        XCTAssertEqual(
            GaryxMobileRouteLink.parse(try XCTUnwrap(URL(string: "garyx://mobile/capsule?capsule_id=01900000-0000-7000-8000-000000000001"))),
            .capsule("01900000-0000-7000-8000-000000000001")
        )
    }

    func testUnknownSettingsTabDoesNotParse() throws {
        let url = try XCTUnwrap(URL(string: "garyx://mobile/settings/unknown"))
        XCTAssertNil(GaryxMobileRouteLink.parse(url))
    }

    func testEveryContentRouteBuildsItsCompleteContainerChain() {
        let draftID = "draft-route"
        for tab in GaryxMobileSettingsTab.allCases {
            let overview = GaryxRouteDestination.settingsDetail("manage")
            XCTAssertEqual(
                GaryxMobileRoutePlan.destinations(
                    for: .settings(tab),
                    draftID: draftID
                ),
                tab == .manage
                    ? [overview]
                    : [overview, .settingsDetail(tab.rawValue)],
                "wrong settings occurrence chain for \(tab)"
            )
        }
        for panel in GaryxMobilePanel.allCases {
            let expected: [GaryxRouteDestination] = switch panel {
            case .chat:
                [.conversationDraft(draftID: draftID)]
            case .settings:
                [.settingsDetail("manage")]
            case .commands, .mcp:
                [.settingsDetail("manage"), .settingsDetail(panel.rawValue)]
            case .workspaces, .workspaceBots, .bots:
                [.panel("workspaceBots")]
            default:
                [.panel(panel.rawValue)]
            }
            XCTAssertEqual(
                GaryxMobileRoutePlan.destinations(
                    for: .panel(panel),
                    draftID: draftID
                ),
                expected,
                "wrong panel occurrence chain for \(panel)"
            )
        }

        let cases: [(GaryxMobileRoute, [GaryxRouteDestination])] = [
            (.chat, [.conversationDraft(draftID: draftID)]),
            (.thread(" thread-1 "), [.conversation(threadID: "thread-1")]),
            (.settings(.manage), [.settingsDetail("manage")]),
            (
                .settings(.provider),
                [.settingsDetail("manage"), .settingsDetail("provider")]
            ),
            (.panel(.automations), [.panel("automations")]),
            (.panel(.workspaceBots), [.panel("workspaceBots")]),
            (
                .panel(.commands),
                [.settingsDetail("manage"), .settingsDetail("commands")]
            ),
            (.automation("automation-1"), [.panel("automations")]),
            (
                .automationThreads(" automation-1 "),
                [.workspaceDrilldown(.automationThreads(automationID: "automation-1"))]
            ),
            (.capsule("capsule-1"), [.panel("capsules")]),
            (.agent("agent-1"), [.panel("agents")]),
            (.skill("skill-1"), [.panel("skills")]),
            (.skillFile(skillId: "skill-1", path: "SKILL.md"), [.panel("skills")]),
            (
                .workspace(" /workspace/test "),
                [.workspaceDrilldown(.workspace(path: "/workspace/test"))]
            ),
            (
                .bot(channel: "api", accountId: "1000000001"),
                [.workspaceDrilldown(.bot(accountID: "configured-group"))]
            ),
            (
                .workspaceFile(workspaceDir: "/workspace/test", path: "README.md"),
                [.panel("workspaces")]
            ),
        ]

        for (route, expected) in cases {
            XCTAssertEqual(
                GaryxMobileRoutePlan.destinations(
                    for: route,
                    draftID: draftID,
                    resolvedBotGroupID: "configured-group"
                ),
                expected,
                "wrong occurrence chain for \(route)"
            )
        }
    }

    func testDrilldownsReturnToTheirActualOpenerInsteadOfInventingOverview() {
        let panel = GaryxRouteEntry(
            id: GaryxRouteInstanceID(rawValue: "panel"),
            destination: .panel("automations")
        )
        let conversation = GaryxRouteEntry(
            id: GaryxRouteInstanceID(rawValue: "conversation"),
            destination: .conversation(threadID: "thread-1")
        )
        var state = GaryxCanonicalRouteState(path: [panel])
        _ = state.open(conversation)

        XCTAssertEqual(state.pop(), [conversation])
        XCTAssertEqual(state.path, [panel])
    }
}
