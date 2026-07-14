import Foundation

public struct GaryxGatewayConfiguration: Equatable, Sendable {
    public var baseURL: URL
    public var authToken: String?
    public var customHeaders: [String: String]

    public init(
        baseURL: URL,
        authToken: String? = nil,
        customHeaders: [String: String] = [:]
    ) {
        self.baseURL = baseURL
        self.authToken = authToken?.trimmingCharacters(in: .whitespacesAndNewlines)
        self.customHeaders = GaryxGatewayHeaders.normalized(customHeaders)
    }
}

public struct GaryxGatewayHeaderEntry: Equatable, Sendable {
    public var name: String
    public var value: String

    public init(name: String, value: String) {
        self.name = name
        self.value = value
    }
}

public enum GaryxGatewayHeaders {
    private static let headerNameAllowedScalars = CharacterSet.alphanumerics.union(
        CharacterSet(charactersIn: "!#$%&'*+-.^_`|~")
    )

    public static func normalizedBlock(_ value: String) -> String {
        value
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .split(whereSeparator: \.isNewline)
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }

    public static func parse(_ value: String) -> [String: String] {
        var headers: [String: String] = [:]
        for entry in parseEntries(value) where isValidHeaderName(entry.name) {
            headers[entry.name] = entry.value
        }
        return headers
    }

    public static func parseEntries(_ value: String) -> [GaryxGatewayHeaderEntry] {
        var entries: [GaryxGatewayHeaderEntry] = []
        for line in normalizedBlock(value).split(whereSeparator: \.isNewline) {
            let text = String(line)
            guard !text.hasPrefix("#"),
                  let separator = separatorIndex(in: text),
                  separator > text.startIndex else {
                continue
            }
            let name = String(text[..<separator]).trimmingCharacters(in: .whitespacesAndNewlines)
            let value = String(text[text.index(after: separator)...]).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !name.isEmpty else { continue }
            entries.append(GaryxGatewayHeaderEntry(name: name, value: value))
        }
        return entries
    }

    public static func format(_ entries: [GaryxGatewayHeaderEntry]) -> String {
        entries
            .map {
                GaryxGatewayHeaderEntry(
                    name: $0.name.trimmingCharacters(in: .whitespacesAndNewlines),
                    value: $0.value.trimmingCharacters(in: .whitespacesAndNewlines)
                )
            }
            .filter { !$0.name.isEmpty }
            .map { "\($0.name): \($0.value)" }
            .joined(separator: "\n")
    }

    public static func normalized(_ headers: [String: String]) -> [String: String] {
        var result: [String: String] = [:]
        for (name, value) in headers where isValidHeaderName(name) {
            result[name.trimmingCharacters(in: .whitespacesAndNewlines)] =
                value.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return result
    }

    private static func separatorIndex(in text: String) -> String.Index? {
        let colon = text.firstIndex(of: ":")
        let equals = text.firstIndex(of: "=")
        switch (colon, equals) {
        case (.some(let colon), .some(let equals)):
            return colon < equals ? colon : equals
        case (.some(let colon), .none):
            return colon
        case (.none, .some(let equals)):
            return equals
        case (.none, .none):
            return nil
        }
    }

    private static func isValidHeaderName(_ name: String) -> Bool {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        return trimmed.unicodeScalars.allSatisfy { headerNameAllowedScalars.contains($0) }
    }
}

public enum GaryxGatewayError: Error, Equatable, LocalizedError {
    case invalidURL(String)
    case invalidHTTPResponse
    case httpStatus(Int, String)
    case encodingFailed(String)

    public var errorDescription: String? {
        switch self {
        case .invalidURL(let value):
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty
                ? "Enter the Garyx gateway URL from the Mac app."
                : "Invalid Garyx gateway URL: \(trimmed)"
        case .invalidHTTPResponse:
            return "The Garyx gateway returned a non-HTTP response."
        case .httpStatus(let status, let body):
            let message = GaryxGatewayError.message(fromHTTPBody: body)
            return message.isEmpty ? "The Garyx gateway returned HTTP \(status)." : message
        case .encodingFailed(let message):
            return message
        }
    }

    static func message(fromHTTPBody body: String) -> String {
        let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        guard let data = trimmed.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let error = object["error"] else {
            return trimmed
        }
        if let message = error as? String,
           !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return message
        }
        if let errorObject = error as? [String: Any] {
            if let message = errorObject["message"] as? String,
               !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                return message
            }
            if let code = errorObject["code"] as? String,
               !code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                return code
            }
        }
        return trimmed
    }
}

public struct GaryxGatewayRetryPolicy: Equatable, Sendable {
    public var maxAttempts: Int
    public var initialDelay: TimeInterval
    public var maxDelay: TimeInterval
    public var backoffMultiplier: Double
    public var jitter: TimeInterval

    public init(
        maxAttempts: Int = 3,
        initialDelay: TimeInterval = 0.4,
        maxDelay: TimeInterval = 4.0,
        backoffMultiplier: Double = 2.5,
        jitter: TimeInterval = 0.2
    ) {
        self.maxAttempts = max(1, maxAttempts)
        self.initialDelay = max(0, initialDelay)
        self.maxDelay = max(self.initialDelay, maxDelay)
        self.backoffMultiplier = max(1.0, backoffMultiplier)
        self.jitter = max(0, jitter)
    }

    public static let `default` = GaryxGatewayRetryPolicy()
    public static let disabled = GaryxGatewayRetryPolicy(
        maxAttempts: 1,
        initialDelay: 0,
        maxDelay: 0,
        backoffMultiplier: 1,
        jitter: 0
    )

    public func delay(forAttempt attempt: Int) -> TimeInterval {
        guard attempt >= 1 else { return 0 }
        let exponent = Double(attempt - 1)
        let base = initialDelay * pow(backoffMultiplier, exponent)
        let capped = min(base, maxDelay)
        guard jitter > 0 else { return capped }
        let jitterAmount = Double.random(in: -jitter...jitter)
        return max(0, capped + jitterAmount)
    }
}

public enum GaryxGatewayRetryClassifier {
    /// Errors raised before the request reached the server. Always safe to retry.
    public static func isConnectionEstablishmentError(_ error: Error) -> Bool {
        let nsError = error as NSError
        guard nsError.domain == NSURLErrorDomain else { return false }
        switch nsError.code {
        case NSURLErrorCannotConnectToHost,
             NSURLErrorCannotFindHost,
             NSURLErrorDNSLookupFailed,
             NSURLErrorNotConnectedToInternet,
             NSURLErrorInternationalRoamingOff,
             NSURLErrorCallIsActive,
             NSURLErrorDataNotAllowed,
             NSURLErrorRequestBodyStreamExhausted,
             NSURLErrorNetworkConnectionLost,
             NSURLErrorResourceUnavailable:
            return true
        default:
            return false
        }
    }

    /// Errors that could have been partially processed by the server. Safe only for idempotent calls.
    public static func isAmbiguousNetworkError(_ error: Error) -> Bool {
        let nsError = error as NSError
        guard nsError.domain == NSURLErrorDomain else { return false }
        switch nsError.code {
        case NSURLErrorTimedOut,
             NSURLErrorBadServerResponse,
             NSURLErrorCannotParseResponse,
             NSURLErrorZeroByteResource:
            return true
        default:
            return false
        }
    }

    public static func isRetryableStatus(_ statusCode: Int, idempotent: Bool) -> Bool {
        switch statusCode {
        case 502:
            // Bad Gateway typically means a reverse proxy failed to reach the upstream:
            // the request was not processed, so retrying is safe for any method.
            return true
        case 503, 504:
            // Service Unavailable / Gateway Timeout might have partially processed the
            // request, so only retry for idempotent calls.
            return idempotent
        case 408, 425, 429:
            return idempotent
        default:
            return false
        }
    }

    public static func isCancellation(_ error: Error) -> Bool {
        if error is CancellationError { return true }
        let nsError = error as NSError
        return nsError.domain == NSURLErrorDomain && nsError.code == NSURLErrorCancelled
    }
}

public final class GaryxGatewayClient {
    public let configuration: GaryxGatewayConfiguration

    private let session: URLSession
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    private let retryPolicy: GaryxGatewayRetryPolicy

    public init(
        configuration: GaryxGatewayConfiguration,
        session: URLSession = .shared,
        encoder: JSONEncoder = JSONEncoder(),
        decoder: JSONDecoder = JSONDecoder(),
        retryPolicy: GaryxGatewayRetryPolicy = .default
    ) {
        self.configuration = configuration
        self.session = session
        self.encoder = encoder
        self.decoder = decoder
        self.retryPolicy = retryPolicy
    }

    public var retry: GaryxGatewayRetryPolicy { retryPolicy }

    static func encodePathSegment(_ value: String) -> String {
        value.addingPercentEncoding(withAllowedCharacters: garyxURLPathSegmentAllowed) ?? value
    }

    public func status() async throws -> GaryxSystemStatus {
        try await get("/api/status")
    }

    public func codingUsage() async throws -> GaryxCodingUsage {
        try await get("/api/usage/coding")
    }

    public func chatHealth() async throws -> GaryxChatHealth {
        try await get("/api/chat/health")
    }

    public func gatewaySettings() async throws -> [String: GaryxJSONValue] {
        try await get("/api/settings")
    }

    public func saveGatewaySettings(
        _ config: [String: GaryxJSONValue],
        merge: Bool = true
    ) async throws -> GaryxGatewaySettingsSaveResult {
        try await put("/api/settings?merge=\(merge ? "true" : "false")", body: config)
    }

    public func listThreads(limit: Int = 100, offset: Int = 0) async throws -> GaryxThreadsPage {
        try await get(
            "/api/threads",
            queryItems: [
                URLQueryItem(name: "limit", value: String(limit)),
                URLQueryItem(name: "offset", value: String(offset)),
            ]
        )
    }

    public func listRecentThreads(
        filter: GaryxRecentThreadFilter = .all,
        limit: Int = 30,
        offset: Int = 0
    ) async throws -> GaryxRecentThreadsPage {
        try await get(
            "/api/recent-threads",
            queryItems: [
                URLQueryItem(name: "tasks", value: filter.tasksQueryValue),
                URLQueryItem(name: "limit", value: String(limit)),
                URLQueryItem(name: "offset", value: String(offset)),
            ]
        )
    }

    public func getThread(threadId: String) async throws -> GaryxThreadSummary {
        try await get("/api/threads/\(threadId.urlPathEncoded)")
    }

    public func listThreadPins() async throws -> GaryxThreadPinsPage {
        try await get("/api/thread-pins")
    }

    public func setThreadPinned(threadId: String, pinned: Bool) async throws -> GaryxThreadPinsPage {
        if pinned {
            return try await put(
                "/api/thread-pins/\(threadId.urlPathEncoded)",
                body: GaryxEmptyBody()
            )
        }
        return try await delete("/api/thread-pins/\(threadId.urlPathEncoded)")
    }

    public func threadHistory(
        threadId: String,
        limit: Int = 100,
        beforeIndex: Int? = nil,
        afterIndex: Int? = nil,
        userQueryLimit: Int? = nil,
        includeToolMessages: Bool = true
    ) async throws -> GaryxThreadTranscript {
        var queryItems = [
            URLQueryItem(name: "thread_id", value: threadId),
            URLQueryItem(name: "limit", value: String(limit)),
            URLQueryItem(
                name: "include_tool_messages",
                value: includeToolMessages ? "true" : "false"
            ),
        ]
        // Forward (delta) cursor takes precedence on the gateway; only send one
        // direction at a time. `after_index` returns committed messages with
        // index > N for incremental catch-up; `before_index` loads older pages.
        if let afterIndex {
            queryItems.append(URLQueryItem(name: "after_index", value: String(afterIndex)))
        } else if let beforeIndex {
            queryItems.append(URLQueryItem(name: "before_index", value: String(beforeIndex)))
        }
        if let userQueryLimit {
            queryItems.append(URLQueryItem(name: "user_query_limit", value: String(userQueryLimit)))
        }
        return try await get(
            "/api/threads/history",
            queryItems: queryItems
        )
    }

    public func threadLogs(threadId: String, cursor: Int? = nil) async throws -> GaryxThreadLogChunk {
        var queryItems: [URLQueryItem] = []
        if let cursor {
            queryItems.append(URLQueryItem(name: "cursor", value: String(cursor)))
        }
        return try await get("/api/threads/\(threadId.urlPathEncoded)/logs", queryItems: queryItems)
    }

    public func createThread(_ request: GaryxCreateThreadRequest) async throws -> GaryxThreadSummary {
        try await post("/api/threads", body: request)
    }

    public func updateThread(
        threadId: String,
        label: String? = nil,
        workspaceDir: String? = nil,
        model: String? = nil,
        modelReasoningEffort: String? = nil,
        modelServiceTier: String? = nil
    ) async throws -> GaryxThreadSummary {
        try await patch(
            "/api/threads/\(threadId.urlPathEncoded)",
            body: GaryxUpdateThreadRequest(
                label: label,
                workspaceDir: workspaceDir,
                model: model,
                modelReasoningEffort: modelReasoningEffort,
                modelServiceTier: modelServiceTier
            )
        )
    }

    public func deleteThread(threadId: String) async throws -> GaryxDeleteResult {
        try await delete("/api/threads/\(threadId.urlPathEncoded)")
    }

    public func archiveThread(
        threadId: String,
        endpointKeys: [String] = []
    ) async throws -> GaryxArchiveThreadResult {
        try await post(
            "/api/threads/\(threadId.urlPathEncoded)/archive",
            body: GaryxArchiveThreadRequest(endpointKeys: endpointKeys),
            idempotent: true
        )
    }

    public func startChat(_ request: GaryxStartChatRequest) async throws -> GaryxStartChatResult {
        // Chat start is non-idempotent: only connection-establishment errors auto-retry.
        try await post("/api/chat/start", body: request)
    }

    public func interruptThread(threadId: String) async throws -> GaryxInterruptResult {
        // Interrupt is idempotent: repeating it for the same thread converges on the same state.
        try await post("/api/chat/interrupt", body: GaryxInterruptRequest(threadId: threadId), idempotent: true)
    }

    public func streamInput(_ request: GaryxStreamInputRequest) async throws -> GaryxStreamInputResult {
        // Gateway does not dedup by client_intent_id on the queue, so we keep this
        // non-idempotent: only connection-establishment errors auto-retry (safe because
        // the request never reached the server). 5xx/timeout surface for explicit retry.
        try await post("/api/chat/stream-input", body: request)
    }

    public func listAgents() async throws -> [GaryxAgentSummary] {
        let page: GaryxAgentsPage = try await get("/api/custom-agents")
        return page.agents
    }

    public func getAgent(agentId: String) async throws -> GaryxAgentSummary {
        try await get("/api/custom-agents/\(agentId.urlPathEncoded)")
    }

    public func providerModels(providerType: String) async throws -> GaryxProviderModels {
        try await get("/api/provider-models/\(providerType.urlPathEncoded)")
    }

    public func startClaudeCodeAuth(
        _ request: GaryxClaudeCodeAuthStartRequest = GaryxClaudeCodeAuthStartRequest()
    ) async throws -> GaryxClaudeCodeAuthSession {
        try await post(
            "/api/providers/claude_code/auth/start",
            body: request,
            timeoutInterval: 35
        )
    }

    public func submitClaudeCodeAuth(
        loginId: String,
        code: String
    ) async throws -> GaryxClaudeCodeAuthSession {
        try await post(
            "/api/providers/claude_code/auth/\(loginId.urlPathEncoded)/submit",
            body: GaryxClaudeCodeAuthSubmitRequest(code: code)
        )
    }

    public func claudeCodeAuth(loginId: String) async throws -> GaryxClaudeCodeAuthSession {
        try await get("/api/providers/claude_code/auth/\(loginId.urlPathEncoded)")
    }

    public func generateAvatar(prompt: String, timeoutSecs: Int = 600) async throws -> GaryxGeneratedAvatar {
        try await post(
            "/api/tools/image",
            body: GaryxGenerateAvatarRequest(prompt: prompt, timeoutSecs: timeoutSecs),
            timeoutInterval: TimeInterval(timeoutSecs + 30),
            allowsRetry: false
        )
    }

    public func createAgent(_ request: GaryxCustomAgentRequest) async throws -> GaryxAgentSummary {
        try await post("/api/custom-agents", body: request)
    }

    public func updateAgent(
        agentId: String,
        request: GaryxCustomAgentRequest
    ) async throws -> GaryxAgentSummary {
        try await put("/api/custom-agents/\(agentId.urlPathEncoded)", body: request)
    }

    public func deleteAgent(agentId: String) async throws -> GaryxEmptyResponse {
        try await delete("/api/custom-agents/\(agentId.urlPathEncoded)")
    }

    public func listSkills() async throws -> [GaryxSkillSummary] {
        let page: GaryxSkillsPage = try await get("/api/skills")
        return page.skills
    }

    public func listCapsules() async throws -> [GaryxCapsuleSummary] {
        let page: GaryxCapsulesPage = try await get("/api/capsules")
        return page.capsules
    }

    public func setCapsuleFavorite(
        id: String,
        favorited: Bool
    ) async throws -> GaryxCapsuleFavoriteResponse {
        let path = "/api/capsules/\(id.urlPathEncoded)/favorite"
        if favorited {
            return try await put(path, body: GaryxEmptyBody())
        }
        return try await delete(path)
    }

    public func deleteCapsule(id: String) async throws -> GaryxDeleteResult {
        try await delete("/api/capsules/\(id.urlPathEncoded)")
    }

    public func capsuleHTML(id: String) async throws -> String {
        try await getText("/api/capsules/\(id.urlPathEncoded)/serve", accept: "text/html")
    }

    public func createSkill(_ request: GaryxCreateSkillRequest) async throws -> GaryxSkillSummary {
        try await post("/api/skills", body: request)
    }

    public func updateSkill(
        skillId: String,
        request: GaryxUpdateSkillRequest
    ) async throws -> GaryxSkillSummary {
        try await patch("/api/skills/\(skillId.urlPathEncoded)", body: request)
    }

    public func toggleSkill(skillId: String) async throws -> GaryxSkillSummary {
        try await patch("/api/skills/\(skillId.urlPathEncoded)/toggle", body: GaryxEmptyBody())
    }

    public func deleteSkill(skillId: String) async throws -> GaryxDeleteResult {
        try await delete("/api/skills/\(skillId.urlPathEncoded)")
    }

    public func skillEditor(skillId: String) async throws -> GaryxSkillEditorState {
        try await get("/api/skills/\(skillId.urlPathEncoded)/tree")
    }

    public func readSkillFile(skillId: String, path: String) async throws -> GaryxSkillFileDocument {
        try await get(
            "/api/skills/\(skillId.urlPathEncoded)/file",
            queryItems: [URLQueryItem(name: "path", value: path)]
        )
    }

    public func saveSkillFile(
        skillId: String,
        request: GaryxSkillFileWriteRequest
    ) async throws -> GaryxSkillFileDocument {
        try await put("/api/skills/\(skillId.urlPathEncoded)/file", body: request)
    }

    public func createSkillEntry(
        skillId: String,
        request: GaryxSkillEntryCreateRequest
    ) async throws -> GaryxSkillEditorState {
        try await post("/api/skills/\(skillId.urlPathEncoded)/entries", body: request)
    }

    public func deleteSkillEntry(skillId: String, path: String) async throws -> GaryxSkillEditorState {
        try await delete(
            "/api/skills/\(skillId.urlPathEncoded)/entries",
            queryItems: [URLQueryItem(name: "path", value: path)]
        )
    }

    /// Anchored task forest for the conversation task-tree sidebar. The
    /// gateway owns retention and layout; callers render the page as-is.
    public func listTaskForest(anchorThreadId: String) async throws -> GaryxTaskForestPage {
        try await get(
            "/api/tasks/forest",
            queryItems: [URLQueryItem(name: "anchor_thread_id", value: anchorThreadId)]
        )
    }

    public func listAutomations() async throws -> [GaryxAutomationSummary] {
        let page: GaryxAutomationsPage = try await get("/api/automations")
        return page.automations
    }

    public func createAutomation(_ request: GaryxAutomationCreateRequest) async throws -> GaryxAutomationSummary {
        try await post("/api/automations", body: request)
    }

    public func updateAutomation(
        id: String,
        request: GaryxAutomationUpdateRequest
    ) async throws -> GaryxAutomationSummary {
        try await patch("/api/automations/\(id.urlPathEncoded)", body: request)
    }

    public func deleteAutomation(id: String) async throws -> GaryxEmptyResponse {
        try await delete("/api/automations/\(id.urlPathEncoded)")
    }

    public func automationActivity(id: String, limit: Int = 20) async throws -> GaryxAutomationActivityFeed {
        try await get(
            "/api/automations/\(id.urlPathEncoded)/activity",
            queryItems: [URLQueryItem(name: "limit", value: String(limit))]
        )
    }

    public func automationThreads(
        id: String,
        limit: Int = 50,
        offset: Int = 0
    ) async throws -> GaryxAutomationThreadsPage {
        try await get(
            "/api/automations/\(id.urlPathEncoded)/threads",
            queryItems: [
                URLQueryItem(name: "limit", value: String(limit)),
                URLQueryItem(name: "offset", value: String(offset)),
            ]
        )
    }

    public func runAutomationNow(id: String) async throws -> GaryxAutomationActivityEntry {
        try await post("/api/automations/\(id.urlPathEncoded)/run-now", body: GaryxEmptyBody())
    }

    public func updateAutomationEnabled(
        id: String,
        enabled: Bool
    ) async throws -> GaryxAutomationSummary {
        try await updateAutomation(id: id, request: GaryxAutomationUpdateRequest(enabled: enabled))
    }

    public func workspaceGitStatus(workspaceDir: String) async throws -> GaryxWorkspaceGitStatus {
        try await get(
            "/api/workspaces/git-status",
            queryItems: [URLQueryItem(name: "workspace_dir", value: workspaceDir)]
        )
    }

    public func listWorkspaces() async throws -> [GaryxWorkspaceSummary] {
        let page: GaryxWorkspacesPage = try await get("/api/workspaces")
        return page.workspaces
    }

    public func listWorkspaceDirectories(path: String? = nil) async throws -> GaryxWorkspaceDirectoryListing {
        var queryItems: [URLQueryItem] = []
        if let path, !path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            queryItems.append(URLQueryItem(name: "path", value: path))
        }
        return try await get("/api/workspaces/directories", queryItems: queryItems)
    }

    @discardableResult
    public func addWorkspace(path: String, name: String? = nil) async throws -> [GaryxWorkspaceSummary] {
        let page: GaryxWorkspacesPage = try await post(
            "/api/workspaces",
            body: GaryxWorkspaceUpsertRequest(path: path, name: name)
        )
        return page.workspaces
    }

    @discardableResult
    public func deleteWorkspace(path: String) async throws -> [GaryxWorkspaceSummary] {
        let page: GaryxWorkspacesPage = try await delete(
            "/api/workspaces",
            queryItems: [URLQueryItem(name: "path", value: path)]
        )
        return page.workspaces
    }

    public func listWorkspaceFiles(
        workspaceDir: String,
        directoryPath: String? = nil
    ) async throws -> GaryxWorkspaceFileListing {
        var queryItems = [URLQueryItem(name: "workspaceDir", value: workspaceDir)]
        if let directoryPath, !directoryPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            queryItems.append(URLQueryItem(name: "path", value: directoryPath))
        }
        return try await get("/api/workspace-files", queryItems: queryItems)
    }

    public func previewWorkspaceFile(
        workspaceDir: String,
        path: String
    ) async throws -> GaryxWorkspaceFilePreview {
        try await get(
            "/api/workspace-files/preview",
            queryItems: [
                URLQueryItem(name: "workspaceDir", value: workspaceDir),
                URLQueryItem(name: "path", value: path),
            ]
        )
    }

    public func uploadWorkspaceFiles(
        _ request: GaryxUploadWorkspaceFilesRequest
    ) async throws -> GaryxUploadWorkspaceFilesResult {
        try await post("/api/workspace-files/upload", body: request)
    }

    public func uploadChatAttachments(
        _ request: GaryxUploadChatAttachmentsRequest
    ) async throws -> GaryxUploadChatAttachmentsResult {
        try await post("/api/chat/attachments/upload", body: request)
    }

    public func listSlashCommands() async throws -> [GaryxSlashCommand] {
        let page: GaryxSlashCommandsPage = try await get("/api/commands/shortcuts")
        return page.commands
    }

    public func createSlashCommand(_ request: GaryxSlashCommandRequest) async throws -> GaryxSlashCommand {
        try await post("/api/commands/shortcuts", body: request)
    }

    public func updateSlashCommand(
        currentName: String,
        request: GaryxSlashCommandRequest
    ) async throws -> GaryxSlashCommand {
        try await put("/api/commands/shortcuts/\(currentName.urlPathEncoded)", body: request)
    }

    public func deleteSlashCommand(name: String) async throws -> GaryxDeleteResult {
        try await delete("/api/commands/shortcuts/\(name.urlPathEncoded)")
    }

    public func listMcpServers() async throws -> [GaryxMcpServer] {
        let page: GaryxMcpServersPage = try await get("/api/mcp-servers")
        return page.servers
    }

    public func createMcpServer(_ request: GaryxMcpServerRequest) async throws -> GaryxMcpServer {
        try await post("/api/mcp-servers", body: request)
    }

    public func updateMcpServer(
        currentName: String,
        request: GaryxMcpServerRequest
    ) async throws -> GaryxMcpServer {
        try await put("/api/mcp-servers/\(currentName.urlPathEncoded)", body: request)
    }

    public func deleteMcpServer(name: String) async throws -> GaryxEmptyResponse {
        try await delete("/api/mcp-servers/\(name.urlPathEncoded)")
    }

    public func toggleMcpServer(name: String, enabled: Bool) async throws -> GaryxMcpServer {
        try await patch(
            "/api/mcp-servers/\(name.urlPathEncoded)/toggle",
            body: GaryxMcpServerToggleRequest(enabled: enabled)
        )
    }

    public func listChannelEndpoints() async throws -> [GaryxChannelEndpoint] {
        let page: GaryxChannelEndpointsPage = try await get("/api/channel-endpoints")
        return page.endpoints
    }

    public func listConfiguredBots() async throws -> [GaryxConfiguredBot] {
        let page: GaryxConfiguredBotsPage = try await get("/api/configured-bots")
        return page.bots
    }

    public func listBotConsoles() async throws -> [GaryxBotConsoleSummary] {
        let page: GaryxBotConsolesPage = try await get("/api/bot-consoles")
        return page.bots
    }

    public func botStatus(botId: String) async throws -> GaryxBotBindingResult {
        try await get(
            "/api/bot/status",
            queryItems: [URLQueryItem(name: "bot_id", value: botId)]
        )
    }

    public func bindBot(botId: String, threadId: String) async throws -> GaryxBotBindingResult {
        try await post(
            "/api/bot/bind",
            body: GaryxBotBindingRequest(botId: botId, threadId: threadId)
        )
    }

    public func unbindBot(botId: String) async throws -> GaryxBotBindingResult {
        try await post(
            "/api/bot/unbind",
            body: GaryxBotBindingRequest(botId: botId)
        )
    }

    public func listChannelPlugins() async throws -> [GaryxChannelPluginCatalogEntry] {
        let page: GaryxChannelPluginCatalogPage = try await get("/api/channels/plugins")
        return page.plugins
    }

    public func startChannelAuthFlow(
        pluginId: String,
        formState: [String: GaryxJSONValue] = [:]
    ) async throws -> GaryxChannelAuthSession {
        try await post(
            "/api/channels/plugins/\(pluginId.urlPathEncoded)/auth_flow/start",
            body: GaryxChannelAuthStartRequest(formState: formState)
        )
    }

    public func pollChannelAuthFlow(
        pluginId: String,
        sessionId: String
    ) async throws -> GaryxChannelAuthPollResult {
        try await post(
            "/api/channels/plugins/\(pluginId.urlPathEncoded)/auth_flow/poll",
            body: GaryxChannelAuthPollRequest(sessionId: sessionId)
        )
    }

    public func validateChannelAccount(
        pluginId: String,
        request: GaryxChannelAccountValidationRequest
    ) async throws -> GaryxChannelAccountValidationResult {
        try await post(
            "/api/channels/plugins/\(pluginId.urlPathEncoded)/validate_account",
            body: request
        )
    }

    public func bindChannelEndpoint(endpointKey: String, threadId: String) async throws -> GaryxEmptyResponse {
        try await post(
            "/api/channel-bindings/bind",
            body: GaryxChannelEndpointBindRequest(endpointKey: endpointKey, threadId: threadId)
        )
    }

    public func detachChannelEndpoint(endpointKey: String) async throws -> GaryxEmptyResponse {
        try await post(
            "/api/channel-bindings/detach",
            body: GaryxChannelEndpointDetachRequest(endpointKey: endpointKey)
        )
    }

    public func url(for path: String, queryItems: [URLQueryItem] = []) throws -> URL {
        guard var components = URLComponents(url: configuration.baseURL, resolvingAgainstBaseURL: false) else {
            throw GaryxGatewayError.invalidURL(configuration.baseURL.absoluteString)
        }
        let pathParts = path.split(separator: "?", maxSplits: 1, omittingEmptySubsequences: false)
        let requestedPath = String(pathParts.first ?? "")
        let requestedQuery = pathParts.dropFirst().first.map(String.init)
        var requestedQueryComponents = URLComponents()
        requestedQueryComponents.percentEncodedQuery = requestedQuery
        let basePath = components.percentEncodedPath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let nextPath = requestedPath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        components.percentEncodedPath = [basePath, nextPath]
            .filter { !$0.isEmpty }
            .joined(separator: "/")
        if !components.percentEncodedPath.hasPrefix("/") {
            components.percentEncodedPath = "/" + components.percentEncodedPath
        }
        let mergedQueryItems = (requestedQueryComponents.queryItems ?? []) + queryItems
        components.queryItems = mergedQueryItems.isEmpty ? nil : mergedQueryItems

        guard let url = components.url else {
            throw GaryxGatewayError.invalidURL(path)
        }
        return url
    }

    /// Resumable per-thread transcript stream (S5): replays committed messages with
    /// `seq > afterSeq`, then streams that thread's live events.
    public func threadStreamRequest(
        threadId: String,
        afterSeq: Int,
        replayScope: GatewayThreadStreamReplayScope? = nil,
        initialUserTurns: Int? = nil,
        renderFloor: Int? = nil
    ) throws -> URLRequest {
        var queryItems = [
            URLQueryItem(name: "after_seq", value: String(max(afterSeq, 0))),
            // Windowed resume degrade is the gateway default since
            // #TASK-1956 batch 4; the old opt-in flag is gone.
            // render_mode=delta (#TASK-1956 batch 3): live frames may carry
            // `render_delta` instead of a full `render_state`;
            // GatewayStreamFrameProcessor reassembles full snapshots, so
            // everything downstream of the action stream never sees deltas.
            URLQueryItem(name: "render_mode", value: "delta"),
        ]
        if let replayScope {
            queryItems.append(URLQueryItem(name: "replay_scope", value: replayScope.rawValue))
        }
        if let initialUserTurns {
            queryItems.append(URLQueryItem(name: "initial_user_turns", value: String(max(initialUserTurns, 0))))
        }
        if let renderFloor {
            queryItems.append(URLQueryItem(name: "render_floor", value: String(max(renderFloor, 0))))
        }
        var request = try makeRequest(
            path: "/api/threads/\(threadId.urlPathEncoded)/stream",
            method: "GET",
            queryItems: queryItems
        )
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        return request
    }

    private func get<Response: Decodable>(
        _ path: String,
        queryItems: [URLQueryItem] = []
    ) async throws -> Response {
        var request = try makeRequest(path: path, method: "GET", queryItems: queryItems)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        return try await send(request, idempotent: true)
    }

    private func getText(
        _ path: String,
        accept: String = "text/plain",
        queryItems: [URLQueryItem] = []
    ) async throws -> String {
        var request = try makeRequest(path: path, method: "GET", queryItems: queryItems)
        request.setValue(accept, forHTTPHeaderField: "Accept")
        return try await sendText(request, idempotent: true)
    }

    private func post<Response: Decodable, Body: Encodable>(
        _ path: String,
        body: Body,
        idempotent: Bool = false,
        timeoutInterval: TimeInterval? = nil,
        allowsRetry: Bool = true
    ) async throws -> Response {
        var request = try makeRequest(path: path, method: "POST", timeoutInterval: timeoutInterval)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(body)
        return try await send(
            request,
            idempotent: idempotent,
            maxAttempts: allowsRetry ? nil : 1
        )
    }

    private func patch<Response: Decodable, Body: Encodable>(
        _ path: String,
        body: Body,
        idempotent: Bool = false
    ) async throws -> Response {
        var request = try makeRequest(path: path, method: "PATCH")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(body)
        return try await send(request, idempotent: idempotent)
    }

    private func put<Response: Decodable, Body: Encodable>(_ path: String, body: Body) async throws -> Response {
        var request = try makeRequest(path: path, method: "PUT")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(body)
        return try await send(request, idempotent: true)
    }

    private func delete<Response: Decodable>(
        _ path: String,
        queryItems: [URLQueryItem] = []
    ) async throws -> Response {
        var request = try makeRequest(path: path, method: "DELETE", queryItems: queryItems)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        return try await send(request, idempotent: true)
    }

    private func makeRequest(
        path: String,
        method: String,
        queryItems: [URLQueryItem] = [],
        timeoutInterval: TimeInterval? = nil
    ) throws -> URLRequest {
        var request = URLRequest(url: try url(for: path, queryItems: queryItems))
        request.httpMethod = method
        if let timeoutInterval, timeoutInterval > 0 {
            request.timeoutInterval = timeoutInterval
        }
        for (name, value) in configuration.customHeaders.sorted(by: { $0.key < $1.key }) {
            request.setValue(value, forHTTPHeaderField: name)
        }
        if let token = configuration.authToken, !token.isEmpty {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    private func send<Response: Decodable>(
        _ request: URLRequest,
        idempotent: Bool,
        maxAttempts: Int? = nil
    ) async throws -> Response {
        let data = try await sendRaw(
            request,
            idempotent: idempotent,
            maxAttempts: maxAttempts
        )
        if data.isEmpty, Response.self == GaryxEmptyResponse.self {
            return GaryxEmptyResponse() as! Response
        }
        return try decoder.decode(Response.self, from: data)
    }

    private func sendText(_ request: URLRequest, idempotent: Bool) async throws -> String {
        let data = try await sendRaw(request, idempotent: idempotent)
        guard let text = String(data: data, encoding: .utf8) else {
            throw GaryxGatewayError.encodingFailed("The Garyx gateway returned non-UTF-8 text.")
        }
        return text
    }

    /// Shared request core for the JSON and text routes: executes the request
    /// with the retry policy (status-code and transport-error classification,
    /// Retry-After handling, cancellation propagation) and returns the raw
    /// body of the first successful (2xx) response. Body decoding stays with
    /// the callers — a 2xx response always terminates the retry loop, so
    /// decode failures never re-enter it.
    private func sendRaw(
        _ request: URLRequest,
        idempotent: Bool,
        maxAttempts requestedMaxAttempts: Int? = nil
    ) async throws -> Data {
        let maxAttempts = max(1, requestedMaxAttempts ?? retryPolicy.maxAttempts)
        var attempt = 0
        while true {
            attempt += 1
            do {
                let (data, response) = try await session.data(for: request)
                guard let http = response as? HTTPURLResponse else {
                    throw GaryxGatewayError.invalidHTTPResponse
                }
                if (200..<300).contains(http.statusCode) {
                    return data
                }
                let body = String(data: data, encoding: .utf8) ?? ""
                let error = GaryxGatewayError.httpStatus(http.statusCode, body)
                if attempt < maxAttempts,
                   GaryxGatewayRetryClassifier.isRetryableStatus(http.statusCode, idempotent: idempotent) {
                    try await sleepForRetry(after: error, attempt: attempt, response: http)
                    continue
                }
                throw error
            } catch let error as GaryxGatewayError {
                throw error
            } catch {
                if GaryxGatewayRetryClassifier.isCancellation(error) {
                    throw error
                }
                let shouldRetry: Bool
                if GaryxGatewayRetryClassifier.isConnectionEstablishmentError(error) {
                    shouldRetry = true
                } else if idempotent, GaryxGatewayRetryClassifier.isAmbiguousNetworkError(error) {
                    shouldRetry = true
                } else {
                    shouldRetry = false
                }
                if attempt < maxAttempts, shouldRetry {
                    try await sleepForRetry(after: error, attempt: attempt, response: nil)
                    continue
                }
                throw error
            }
        }
    }

    private func sleepForRetry(after error: Error, attempt: Int, response: HTTPURLResponse?) async throws {
        try Task.checkCancellation()
        let retryAfter = response.flatMap { Self.retryAfterDelay(from: $0) }
        let computed = retryPolicy.delay(forAttempt: attempt)
        let delay = max(retryAfter ?? 0, computed)
        guard delay > 0 else { return }
        let nanoseconds = UInt64(delay * 1_000_000_000)
        try await Task.sleep(nanoseconds: nanoseconds)
        try Task.checkCancellation()
    }

    private static func retryAfterDelay(from response: HTTPURLResponse) -> TimeInterval? {
        guard let header = response.value(forHTTPHeaderField: "Retry-After")?
            .trimmingCharacters(in: .whitespacesAndNewlines), !header.isEmpty
        else { return nil }
        if let seconds = TimeInterval(header) {
            return max(0, seconds)
        }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(identifier: "GMT")
        formatter.dateFormat = "EEE, dd MMM yyyy HH:mm:ss zzz"
        if let date = formatter.date(from: header) {
            return max(0, date.timeIntervalSinceNow)
        }
        return nil
    }
}
private extension String {
    var urlPathEncoded: String {
        GaryxGatewayClient.encodePathSegment(self)
    }
}

private let garyxURLPathSegmentAllowed: CharacterSet = {
    CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
}()
