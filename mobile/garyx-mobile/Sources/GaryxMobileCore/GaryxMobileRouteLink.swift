import Foundation

public enum GaryxMobileRoute: Equatable, Sendable {
    case chat
    case thread(String)
    case settings(GaryxMobileSettingsTab)
    case panel(GaryxMobilePanel)
    case task(String)
    case automation(String)
    case automationThreads(String)
    case capsule(String)
    case agent(String)
    case team(String)
    case skill(String)
    case skillFile(skillId: String, path: String)
    case workspace(String)
    case bot(channel: String, accountId: String)
    case workspaceFile(workspaceDir: String, path: String)
}

public enum GaryxMobileRouteLink {
    public static func make(_ route: GaryxMobileRoute) -> URL? {
        var components = URLComponents()
        components.scheme = "garyx"
        components.host = "mobile"

        switch route {
        case .chat:
            components.path = "/chat"
        case let .thread(threadId):
            components.path = "/thread"
            components.queryItems = [URLQueryItem(name: "threadId", value: normalized(threadId))]
        case let .settings(tab):
            components.path = tab == .manage ? "/settings" : "/settings/\(tab.rawValue)"
        case let .panel(panel):
            components.path = "/\(pathComponent(for: panel))"
        case let .task(id):
            components.path = "/task"
            components.queryItems = [URLQueryItem(name: "id", value: normalized(id))]
        case let .automation(id):
            components.path = "/automation"
            components.queryItems = [URLQueryItem(name: "id", value: normalized(id))]
        case let .automationThreads(id):
            components.path = "/workspace-bots"
            components.queryItems = [URLQueryItem(name: "automationThreads", value: normalized(id))]
        case let .capsule(id):
            components.path = "/capsule"
            components.queryItems = [URLQueryItem(name: "id", value: normalized(id))]
        case let .agent(id):
            components.path = "/agent"
            components.queryItems = [URLQueryItem(name: "id", value: normalized(id))]
        case let .team(id):
            components.path = "/team"
            components.queryItems = [URLQueryItem(name: "id", value: normalized(id))]
        case let .skill(id):
            components.path = "/skill"
            components.queryItems = [URLQueryItem(name: "id", value: normalized(id))]
        case let .skillFile(skillId, path):
            components.path = "/skill-file"
            components.queryItems = [
                URLQueryItem(name: "skillId", value: normalized(skillId)),
                URLQueryItem(name: "path", value: normalized(path)),
            ]
        case let .workspace(path):
            components.path = "/workspace"
            components.queryItems = [URLQueryItem(name: "path", value: normalized(path))]
        case let .bot(channel, accountId):
            components.path = "/bot"
            components.queryItems = [
                URLQueryItem(name: "channel", value: normalized(channel)),
                URLQueryItem(name: "accountId", value: normalized(accountId)),
            ]
        case let .workspaceFile(workspaceDir, path):
            components.path = "/workspace-file"
            components.queryItems = [
                URLQueryItem(name: "workspace", value: normalized(workspaceDir)),
                URLQueryItem(name: "path", value: normalized(path)),
            ]
        }

        return components.url
    }

    public static func parse(_ url: URL) -> GaryxMobileRoute? {
        guard url.scheme?.lowercased() == "garyx",
              let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return nil
        }

        let host = url.host()?.lowercased()
        var parts = url.path
            .split(separator: "/", omittingEmptySubsequences: true)
            .map { $0.lowercased() }
        if parts.first == "mobile" {
            parts.removeFirst()
        }

        if host == "thread" {
            return parseThreadRoute(components)
        }
        guard host == nil || host == "mobile" else {
            return nil
        }
        guard let first = parts.first else {
            return nil
        }

        switch first {
        case "chat":
            return .chat
        case "thread":
            return parseThreadRoute(components)
        case "settings":
            if parts.count > 1 {
                guard let tab = GaryxMobileSettingsTab(rawValue: parts[1]) else {
                    return nil
                }
                return .settings(tab)
            }
            return .settings(.manage)
        case "tasks":
            return .panel(.tasks)
        case "task":
            return queryValue(components, "id", "taskId", "task_id").map(GaryxMobileRoute.task)
        case "automations", "automation-list":
            return .panel(.automations)
        case "capsules":
            return .panel(.capsules)
        case "capsule":
            return queryValue(components, "id", "capsuleId", "capsule_id").map(GaryxMobileRoute.capsule)
        case "automation":
            return queryValue(components, "id", "automationId", "automation_id").map(GaryxMobileRoute.automation)
        case "agents":
            return .panel(.agents)
        case "agent":
            return queryValue(components, "id", "agentId", "agent_id").map(GaryxMobileRoute.agent)
        case "team":
            return queryValue(components, "id", "teamId", "team_id").map(GaryxMobileRoute.team)
        case "skills":
            return .panel(.skills)
        case "skill":
            return queryValue(components, "id", "skillId", "skill_id").map(GaryxMobileRoute.skill)
        case "skill-file":
            guard let skillId = queryValue(components, "skillId", "skill_id", "id"),
                  let path = queryValue(components, "path", "file", "filePath", "file_path") else {
                return nil
            }
            return .skillFile(skillId: skillId, path: path)
        case "workspace-bots", "workspacebots":
            if let id = queryValue(
                components,
                "automationThreads",
                "automation_threads",
                "automationId",
                "automation_id"
            ) {
                return .automationThreads(id)
            }
            return .panel(.workspaceBots)
        case "workspaces":
            return .panel(.workspaceBots)
        case "workspace":
            return queryValue(components, "path", "workspace", "workspaceDir", "workspace_dir")
                .map(GaryxMobileRoute.workspace)
        case "bots":
            return .panel(.workspaceBots)
        case "bot":
            guard let channel = queryValue(components, "channel", "channelId", "channel_id"),
                  let accountId = queryValue(components, "accountId", "account_id", "id") else {
                return nil
            }
            return .bot(channel: channel, accountId: accountId)
        case "workspace-file":
            guard let workspace = queryValue(components, "workspace", "workspaceDir", "workspace_dir"),
                  let path = queryValue(components, "path", "file", "filePath", "file_path") else {
                return nil
            }
            return .workspaceFile(workspaceDir: workspace, path: path)
        case "dreams":
            return .panel(.dreams)
        default:
            return nil
        }
    }

    private static func parseThreadRoute(_ components: URLComponents) -> GaryxMobileRoute? {
        queryValue(components, "threadId", "thread_id", "id").map(GaryxMobileRoute.thread)
    }

    private static func queryValue(_ components: URLComponents, _ names: String...) -> String? {
        for name in names {
            if let value = components.queryItems?
                .first(where: { $0.name == name })?
                .value?
                .trimmingCharacters(in: .whitespacesAndNewlines),
                !value.isEmpty {
                return value
            }
        }
        return nil
    }

    private static func normalized(_ value: String) -> String {
        value.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func pathComponent(for panel: GaryxMobilePanel) -> String {
        switch panel {
        case .chat:
            "chat"
        case .dreams:
            "dreams"
        case .tasks:
            "tasks"
        case .workspaces, .workspaceBots, .bots:
            "workspace-bots"
        case .automations:
            "automations"
        case .capsules:
            "capsules"
        case .agents:
            "agents"
        case .skills:
            "skills"
        case .commands:
            "settings/commands"
        case .mcp:
            "settings/mcp"
        case .settings:
            "settings"
        }
    }
}
