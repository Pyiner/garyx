import Foundation

struct GaryxMobileCatalogCacheSnapshot: Codable, Equatable {
    static let currentVersion = 3

    var version: Int
    var savedAt: Date
    var agents: [GaryxCachedAgent]
    var workspacePaths: [String]
    var skills: [GaryxCachedSkill]
    var capsules: [GaryxCachedCapsule]
    var automations: [GaryxCachedAutomation]
    var slashCommands: [GaryxCachedSlashCommand]
    var mcpServers: [GaryxCachedMcpServer]
    var channelEndpoints: [GaryxCachedChannelEndpoint]
    var configuredBots: [GaryxCachedConfiguredBot]
    var configuredBotAccounts: [GaryxCachedConfiguredBotAccount]
    var botConsoles: [GaryxCachedBotConsole]
    var channelPlugins: [GaryxCachedChannelPlugin]

    init(
        version: Int = Self.currentVersion,
        savedAt: Date = Date(),
        agents: [GaryxCachedAgent],
        workspacePaths: [String],
        skills: [GaryxCachedSkill],
        capsules: [GaryxCachedCapsule] = [],
        automations: [GaryxCachedAutomation],
        slashCommands: [GaryxCachedSlashCommand],
        mcpServers: [GaryxCachedMcpServer],
        channelEndpoints: [GaryxCachedChannelEndpoint],
        configuredBots: [GaryxCachedConfiguredBot],
        configuredBotAccounts: [GaryxCachedConfiguredBotAccount],
        botConsoles: [GaryxCachedBotConsole],
        channelPlugins: [GaryxCachedChannelPlugin]
    ) {
        self.version = version
        self.savedAt = savedAt
        self.agents = agents
        self.workspacePaths = workspacePaths
        self.skills = skills
        self.capsules = capsules
        self.automations = automations
        self.slashCommands = slashCommands
        self.mcpServers = mcpServers
        self.channelEndpoints = channelEndpoints
        self.configuredBots = configuredBots
        self.configuredBotAccounts = configuredBotAccounts
        self.botConsoles = botConsoles
        self.channelPlugins = channelPlugins
    }

    init(
        agents: [GaryxAgentSummary],
        workspacePaths: [String],
        skills: [GaryxSkillSummary],
        capsules: [GaryxCapsuleSummary] = [],
        automations: [GaryxAutomationSummary],
        slashCommands: [GaryxSlashCommand],
        mcpServers: [GaryxMcpServer],
        channelEndpoints: [GaryxChannelEndpoint],
        configuredBots: [GaryxConfiguredBot],
        configuredBotAccounts: [GaryxConfiguredBotAccountSettings],
        botConsoles: [GaryxBotConsoleSummary],
        channelPlugins: [GaryxChannelPluginCatalogEntry],
        savedAt: Date = Date()
    ) {
        self.init(
            savedAt: savedAt,
            agents: agents.map(GaryxCachedAgent.init),
            workspacePaths: workspacePaths,
            skills: skills.map(GaryxCachedSkill.init),
            capsules: capsules.map(GaryxCachedCapsule.init),
            automations: automations.map(GaryxCachedAutomation.init),
            slashCommands: slashCommands.map(GaryxCachedSlashCommand.init),
            mcpServers: mcpServers.map(GaryxCachedMcpServer.init),
            channelEndpoints: channelEndpoints.map(GaryxCachedChannelEndpoint.init),
            configuredBots: configuredBots.map(GaryxCachedConfiguredBot.init),
            configuredBotAccounts: configuredBotAccounts.map(GaryxCachedConfiguredBotAccount.init),
            botConsoles: botConsoles.map(GaryxCachedBotConsole.init),
            channelPlugins: channelPlugins.map(GaryxCachedChannelPlugin.init)
        )
    }
}

struct GaryxCachedAgent: Codable, Equatable {
    var id: String
    var displayName: String
    var providerType: String
    var modelName: String
    var defaultWorkspaceDir: String
    var avatarDataUrl: String
    var builtIn: Bool
    var standalone: Bool
    var createdAt: String?
    var updatedAt: String?

    init(_ agent: GaryxAgentSummary) {
        id = agent.id
        displayName = agent.displayName
        providerType = agent.providerType
        modelName = agent.model
        defaultWorkspaceDir = agent.defaultWorkspaceDir
        avatarDataUrl = agent.avatarDataUrl
        builtIn = agent.builtIn
        standalone = agent.standalone
        createdAt = agent.createdAt
        updatedAt = agent.updatedAt
    }

    var model: GaryxAgentSummary {
        GaryxAgentSummary(
            id: id,
            displayName: displayName,
            providerType: providerType,
            model: modelName,
            defaultWorkspaceDir: defaultWorkspaceDir,
            avatarDataUrl: avatarDataUrl,
            builtIn: builtIn,
            standalone: standalone,
            createdAt: createdAt,
            updatedAt: updatedAt
        )
    }
}

struct GaryxCachedSkill: Codable, Equatable {
    var id: String
    var name: String
    var description: String
    var installed: Bool
    var enabled: Bool
    var sourcePath: String

    init(_ skill: GaryxSkillSummary) {
        id = skill.id
        name = skill.name
        description = skill.description
        installed = skill.installed
        enabled = skill.enabled
        sourcePath = skill.sourcePath
    }

    var model: GaryxSkillSummary {
        GaryxSkillSummary(
            id: id,
            name: name,
            description: description,
            installed: installed,
            enabled: enabled,
            sourcePath: sourcePath
        )
    }
}

struct GaryxCachedCapsule: Codable, Equatable {
    var id: String
    var title: String
    var description: String
    var threadId: String?
    var runId: String?
    var agentId: String?
    var providerType: String?
    var htmlSha256: String
    var byteSize: Int
    var revision: Int
    var createdAt: String?
    var updatedAt: String?

    init(_ capsule: GaryxCapsuleSummary) {
        id = capsule.id
        title = capsule.title
        description = capsule.description
        threadId = capsule.threadId
        runId = capsule.runId
        agentId = capsule.agentId
        providerType = capsule.providerType
        htmlSha256 = capsule.htmlSha256
        byteSize = capsule.byteSize
        revision = capsule.revision
        createdAt = capsule.createdAt
        updatedAt = capsule.updatedAt
    }

    var model: GaryxCapsuleSummary {
        GaryxCapsuleSummary(
            id: id,
            title: title,
            description: description,
            threadId: threadId,
            runId: runId,
            agentId: agentId,
            providerType: providerType,
            htmlSha256: htmlSha256,
            byteSize: byteSize,
            revision: revision,
            createdAt: createdAt,
            updatedAt: updatedAt
        )
    }
}

struct GaryxCachedAutomation: Codable, Equatable {
    var id: String
    var label: String
    var prompt: String
    var agentId: String
    var enabled: Bool
    var workspacePath: String
    var targetThreadId: String?
    var threadId: String?
    var threadMode: String
    var nextRun: String
    var lastRunAt: String?
    var lastStatus: String
    var schedule: GaryxAutomationSchedule

    enum CodingKeys: String, CodingKey {
        case id
        case label
        case prompt
        case agentId
        case enabled
        case workspacePath
        case targetThreadId
        case threadId
        case threadMode
        case nextRun
        case lastRunAt
        case lastStatus
        case schedule
    }

    init(_ automation: GaryxAutomationSummary) {
        id = automation.id
        label = automation.label
        prompt = automation.prompt
        agentId = automation.agentId
        enabled = automation.enabled
        workspacePath = automation.workspacePath
        targetThreadId = automation.targetThreadId
        threadId = automation.threadId
        threadMode = automation.threadMode
        nextRun = automation.nextRun
        lastRunAt = automation.lastRunAt
        lastStatus = automation.lastStatus
        schedule = automation.schedule
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        label = try container.decode(String.self, forKey: .label)
        prompt = try container.decode(String.self, forKey: .prompt)
        agentId = try container.decode(String.self, forKey: .agentId)
        enabled = try container.decode(Bool.self, forKey: .enabled)
        workspacePath = try container.decode(String.self, forKey: .workspacePath)
        targetThreadId = try container.decodeIfPresent(String.self, forKey: .targetThreadId)
        threadId = try container.decodeIfPresent(String.self, forKey: .threadId)
        threadMode = try container.decodeIfPresent(String.self, forKey: .threadMode)
            ?? (targetThreadId == nil ? "generated" : "target")
        nextRun = try container.decode(String.self, forKey: .nextRun)
        lastRunAt = try container.decodeIfPresent(String.self, forKey: .lastRunAt)
        lastStatus = try container.decode(String.self, forKey: .lastStatus)
        schedule = try container.decode(GaryxAutomationSchedule.self, forKey: .schedule)
    }

    var model: GaryxAutomationSummary {
        GaryxAutomationSummary(
            id: id,
            label: label,
            prompt: prompt,
            agentId: agentId,
            enabled: enabled,
            workspacePath: workspacePath,
            targetThreadId: targetThreadId,
            threadId: threadId,
            threadMode: threadMode,
            nextRun: nextRun,
            lastRunAt: lastRunAt,
            lastStatus: lastStatus,
            schedule: schedule
        )
    }
}

struct GaryxCachedSlashCommand: Codable, Equatable {
    var name: String
    var description: String
    var prompt: String

    init(_ command: GaryxSlashCommand) {
        name = command.name
        description = command.description
        prompt = command.prompt
    }

    var model: GaryxSlashCommand {
        GaryxSlashCommand(name: name, description: description, prompt: prompt)
    }
}

struct GaryxCachedMcpServer: Codable, Equatable {
    var name: String
    var transport: String
    var command: String
    var args: [String]
    var enabled: Bool
    var workingDir: String?
    var url: String?
    var bearerTokenEnv: String?

    init(_ server: GaryxMcpServer) {
        name = server.name
        transport = server.transport
        command = server.command
        args = server.args
        enabled = server.enabled
        workingDir = server.workingDir
        url = server.url
        bearerTokenEnv = server.bearerTokenEnv
    }

    var model: GaryxMcpServer {
        GaryxMcpServer(
            name: name,
            transport: transport,
            command: command,
            args: args,
            enabled: enabled,
            workingDir: workingDir,
            url: url,
            bearerTokenEnv: bearerTokenEnv
        )
    }
}

struct GaryxCachedChannelEndpoint: Codable, Equatable {
    var endpointKey: String
    var channel: String
    var accountId: String
    var displayLabel: String
    var threadId: String?
    var threadLabel: String?
    var workspaceDir: String?
    var lastInboundAt: String?
    var lastDeliveryAt: String?
    var conversationKind: String?
    var conversationLabel: String?

    init(_ endpoint: GaryxChannelEndpoint) {
        endpointKey = endpoint.endpointKey
        channel = endpoint.channel
        accountId = endpoint.accountId
        displayLabel = endpoint.displayLabel
        threadId = endpoint.threadId
        threadLabel = endpoint.threadLabel
        workspaceDir = endpoint.workspaceDir
        lastInboundAt = endpoint.lastInboundAt
        lastDeliveryAt = endpoint.lastDeliveryAt
        conversationKind = endpoint.conversationKind
        conversationLabel = endpoint.conversationLabel
    }

    var model: GaryxChannelEndpoint {
        GaryxChannelEndpoint(
            endpointKey: endpointKey,
            channel: channel,
            accountId: accountId,
            displayLabel: displayLabel,
            threadId: threadId,
            threadLabel: threadLabel,
            workspaceDir: workspaceDir,
            lastInboundAt: lastInboundAt,
            lastDeliveryAt: lastDeliveryAt,
            conversationKind: conversationKind,
            conversationLabel: conversationLabel
        )
    }
}

struct GaryxCachedConfiguredBot: Codable, Equatable {
    var channel: String
    var accountId: String
    var displayName: String
    var enabled: Bool
    var agentId: String?
    var workspaceDir: String?
    var workspaceMode: String?
    var rootBehavior: String
    var mainEndpointStatus: String
    var mainThreadId: String?
    var defaultOpenThreadId: String?

    init(_ bot: GaryxConfiguredBot) {
        channel = bot.channel
        accountId = bot.accountId
        displayName = bot.displayName
        enabled = bot.enabled
        agentId = bot.agentId
        workspaceDir = bot.workspaceDir
        workspaceMode = bot.workspaceMode
        rootBehavior = bot.rootBehavior
        mainEndpointStatus = bot.mainEndpointStatus
        mainThreadId = bot.mainThreadId
        defaultOpenThreadId = bot.defaultOpenThreadId
    }

    var model: GaryxConfiguredBot {
        GaryxConfiguredBot(
            channel: channel,
            accountId: accountId,
            displayName: displayName,
            enabled: enabled,
            agentId: agentId,
            workspaceDir: workspaceDir,
            workspaceMode: workspaceMode,
            rootBehavior: rootBehavior,
            mainEndpointStatus: mainEndpointStatus,
            mainThreadId: mainThreadId,
            defaultOpenThreadId: defaultOpenThreadId
        )
    }
}

struct GaryxCachedConfiguredBotAccount: Codable, Equatable {
    var channel: String
    var accountId: String
    var displayName: String
    var enabled: Bool
    var agentId: String?
    var workspaceDir: String?
    var workspaceMode: String?
    var config: [String: GaryxJSONValue]

    init(_ account: GaryxConfiguredBotAccountSettings) {
        channel = account.channel
        accountId = account.accountId
        displayName = account.displayName
        enabled = account.enabled
        agentId = account.agentId
        workspaceDir = account.workspaceDir
        workspaceMode = account.workspaceMode
        config = account.config
    }

    var model: GaryxConfiguredBotAccountSettings {
        GaryxConfiguredBotAccountSettings(
            channel: channel,
            accountId: accountId,
            displayName: displayName,
            enabled: enabled,
            agentId: agentId,
            workspaceDir: workspaceDir,
            workspaceMode: workspaceMode,
            config: config
        )
    }
}

struct GaryxCachedBotConsole: Codable, Equatable {
    var id: String
    var channel: String
    var accountId: String
    var title: String
    var subtitle: String
    var agentId: String?
    var rootBehavior: String
    var status: String
    var latestActivity: String?
    var endpointCount: Int
    var boundEndpointCount: Int
    var workspaceDir: String?
    var mainThreadId: String?
    var defaultOpenThreadId: String?

    init(_ console: GaryxBotConsoleSummary) {
        id = console.id
        channel = console.channel
        accountId = console.accountId
        title = console.title
        subtitle = console.subtitle
        agentId = console.agentId
        rootBehavior = console.rootBehavior
        status = console.status
        latestActivity = console.latestActivity
        endpointCount = console.endpointCount
        boundEndpointCount = console.boundEndpointCount
        workspaceDir = console.workspaceDir
        mainThreadId = console.mainThreadId
        defaultOpenThreadId = console.defaultOpenThreadId
    }

    var model: GaryxBotConsoleSummary {
        GaryxBotConsoleSummary(
            id: id,
            channel: channel,
            accountId: accountId,
            title: title,
            subtitle: subtitle,
            agentId: agentId,
            rootBehavior: rootBehavior,
            status: status,
            latestActivity: latestActivity,
            endpointCount: endpointCount,
            boundEndpointCount: boundEndpointCount,
            workspaceDir: workspaceDir,
            mainThreadId: mainThreadId,
            defaultOpenThreadId: defaultOpenThreadId,
            conversationNodes: []
        )
    }
}

struct GaryxCachedChannelPluginConfigMethod: Codable, Equatable {
    var kind: String
    var title: String?
    var description: String?

    init(_ method: GaryxChannelPluginConfigMethod) {
        kind = method.kind
        title = method.title
        description = method.description
    }

    var model: GaryxChannelPluginConfigMethod {
        GaryxChannelPluginConfigMethod(kind: kind, title: title, description: description)
    }
}

struct GaryxCachedChannelPlugin: Codable, Equatable {
    var id: String
    var displayName: String
    var description: String?
    var iconDataUrl: String?
    var schema: [String: GaryxJSONValue]
    var configMethods: [GaryxCachedChannelPluginConfigMethod]

    init(_ plugin: GaryxChannelPluginCatalogEntry) {
        id = plugin.id
        displayName = plugin.displayName
        description = plugin.description
        iconDataUrl = plugin.iconDataUrl
        schema = plugin.schema
        configMethods = plugin.configMethods.map(GaryxCachedChannelPluginConfigMethod.init)
    }

    var model: GaryxChannelPluginCatalogEntry {
        GaryxChannelPluginCatalogEntry(
            id: id,
            displayName: displayName,
            description: description,
            iconDataUrl: iconDataUrl,
            schema: schema,
            configMethods: configMethods.map(\.model)
        )
    }
}
