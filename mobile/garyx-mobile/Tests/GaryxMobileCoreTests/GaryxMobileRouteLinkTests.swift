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
            .panel(.tasks),
            .task("task-1"),
            .panel(.automations),
            .automation("automation-1"),
            .automationThreads("automation-1"),
            .panel(.agents),
            .agent("agent-1"),
            .team("team-1"),
            .panel(.skills),
            .skill("skill-1"),
            .skillFile(skillId: "skill-1", path: "SKILL.md"),
            .panel(.workspaceBots),
            .workspace("/tmp/workspace"),
            .bot(channel: "channel-a", accountId: "1000000001"),
            .workspaceFile(workspaceDir: "/tmp/workspace", path: "docs/index.html"),
            .panel(.dreams),
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

    func testConnectLinksAreNotRouteLinks() throws {
        let url = try XCTUnwrap(URL(string: "garyx://mobile/connect?gatewayUrl=http%3A%2F%2F127.0.0.1%3A31337"))
        XCTAssertNil(GaryxMobileRouteLink.parse(url))
    }

    func testDetailRouteQueryAliases() throws {
        XCTAssertEqual(
            GaryxMobileRouteLink.parse(try XCTUnwrap(URL(string: "garyx://mobile/task?task_id=task-1"))),
            .task("task-1")
        )
        XCTAssertEqual(
            GaryxMobileRouteLink.parse(try XCTUnwrap(URL(string: "garyx://mobile/skill-file?skill_id=skill-1&file_path=docs%2Ffile.md"))),
            .skillFile(skillId: "skill-1", path: "docs/file.md")
        )
        XCTAssertEqual(
            GaryxMobileRouteLink.parse(try XCTUnwrap(URL(string: "garyx://mobile/workspace-bots?automation_id=automation-1"))),
            .automationThreads("automation-1")
        )
    }

    func testUnknownSettingsTabDoesNotParse() throws {
        let url = try XCTUnwrap(URL(string: "garyx://mobile/settings/unknown"))
        XCTAssertNil(GaryxMobileRouteLink.parse(url))
    }
}
