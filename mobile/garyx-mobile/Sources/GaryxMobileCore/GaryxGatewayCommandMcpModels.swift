import Foundation

public struct GaryxSlashCommandsPage: Decodable, Equatable, Sendable {
    public var commands: [GaryxSlashCommand]
}


public struct GaryxSlashCommand: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { name }
    public var name: String
    public var description: String
    public var prompt: String

    enum CodingKeys: String, CodingKey {
        case name
        case description
        case prompt
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        name = try container.garyxDecodeFirstString(.name) ?? ""
        description = try container.garyxDecodeFirstString(.description) ?? ""
        prompt = try container.garyxDecodeFirstString(.prompt) ?? ""
    }
}


public struct GaryxSlashCommandRequest: Encodable, Equatable, Sendable {
    public var name: String
    public var description: String
    public var prompt: String?

    public init(name: String, description: String, prompt: String?) {
        self.name = name
        self.description = description
        self.prompt = prompt
    }
}


public struct GaryxMcpServersPage: Decodable, Equatable, Sendable {
    public var servers: [GaryxMcpServer]
}


public struct GaryxMcpServer: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { name }
    public var name: String
    public var transport: String
    public var command: String
    public var args: [String]
    public var env: [String: String]
    public var enabled: Bool
    public var workingDir: String?
    public var url: String?
    public var bearerTokenEnv: String?
    public var headers: [String: String]

    enum CodingKeys: String, CodingKey {
        case name
        case transport
        case command
        case args
        case env
        case enabled
        case workingDir
        case workingDirSnake = "working_dir"
        case url
        case bearerTokenEnv
        case bearerTokenEnvSnake = "bearer_token_env"
        case headers
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        name = try container.garyxDecodeFirstString(.name) ?? ""
        transport = try container.garyxDecodeFirstString(.transport) ?? "stdio"
        command = try container.garyxDecodeFirstString(.command) ?? ""
        args = try container.decodeIfPresent([String].self, forKey: .args) ?? []
        env = try container.decodeIfPresent([String: String].self, forKey: .env) ?? [:]
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        workingDir = try container.garyxDecodeFirstString(.workingDir, .workingDirSnake)
        url = try container.garyxDecodeFirstString(.url)
        bearerTokenEnv = try container.garyxDecodeFirstString(.bearerTokenEnv, .bearerTokenEnvSnake)
        headers = try container.decodeIfPresent([String: String].self, forKey: .headers) ?? [:]
    }
}


public struct GaryxMcpServerRequest: Encodable, Equatable, Sendable {
    public var name: String
    public var transport: String
    public var command: String
    public var args: [String]
    public var env: [String: String]
    public var enabled: Bool
    public var workingDir: String?
    public var url: String?
    public var bearerTokenEnv: String?
    public var headers: [String: String]

    public init(
        name: String,
        transport: String = "stdio",
        command: String = "",
        args: [String] = [],
        env: [String: String] = [:],
        enabled: Bool = true,
        workingDir: String? = nil,
        url: String? = nil,
        bearerTokenEnv: String? = nil,
        headers: [String: String] = [:]
    ) {
        self.name = name
        self.transport = transport
        self.command = command
        self.args = args
        self.env = env
        self.enabled = enabled
        self.workingDir = workingDir
        self.url = url
        self.bearerTokenEnv = bearerTokenEnv
        self.headers = headers
    }

    enum CodingKeys: String, CodingKey {
        case name
        case transport
        case command
        case args
        case env
        case enabled
        case workingDir = "working_dir"
        case url
        case bearerTokenEnv = "bearer_token_env"
        case headers
    }
}


public struct GaryxMcpServerToggleRequest: Encodable, Equatable, Sendable {
    public var enabled: Bool

    public init(enabled: Bool) {
        self.enabled = enabled
    }
}
