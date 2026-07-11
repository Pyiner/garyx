import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    static var defaultGatewayURL: String {
        #if targetEnvironment(simulator)
        "http://127.0.0.1:31337"
        #else
        ""
        #endif
    }

    static func firstNonEmpty(_ values: String?...) -> String? {
        values
            .compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }
    }

    static func normalizedWorkspaceMode(_ value: String?) -> String {
        let normalized = value?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        return normalized == "worktree" ? "worktree" : "local"
    }

    static func botSelectorId(channel: String, accountId: String) -> String {
        "\(channel.trimmingCharacters(in: .whitespacesAndNewlines)):\(accountId.trimmingCharacters(in: .whitespacesAndNewlines))"
    }

    nonisolated static func isVisibleMobileWorkspacePath(_ path: String) -> Bool {
        GaryxMobileWorkspacePresentation.isVisibleWorkspacePath(path)
    }

    func splitShellLikeList(_ value: String) -> [String] {
        value
            .split { $0 == "," || $0 == "\n" }
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    func keyValueDictionary(from value: String) -> [String: String] {
        var result: [String: String] = [:]
        for line in value.split(whereSeparator: \.isNewline) {
            let text = String(line).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { continue }
            let parts = text.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            guard let key = parts.first.map(String.init)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !key.isEmpty else {
                continue
            }
            let rawValue = parts.dropFirst().first.map(String.init) ?? ""
            result[key] = rawValue.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return result
    }

    func client() throws -> GaryxGatewayClient {
        let normalized = normalizedGatewayURL(gatewayURL)
        guard let url = parsedGatewayURL(from: normalized) else {
            throw GaryxGatewayError.invalidURL(normalized)
        }
        let configuration = GaryxGatewayConfiguration(
            baseURL: url,
            authToken: gatewayAuthToken,
            customHeaders: GaryxGatewayHeaders.parse(gatewayHeaders)
        )
        return gatewayClientFactory?(configuration)
            ?? GaryxGatewayClient(configuration: configuration)
    }

    func parsedGatewayURL(from value: String) -> URL? {
        let normalized = normalizedGatewayURL(value)
        guard !normalized.isEmpty else { return nil }
        guard
            let components = URLComponents(string: normalized),
            let scheme = components.scheme?.lowercased(),
            scheme == "http" || scheme == "https",
            let host = components.host,
            !host.isEmpty,
            let url = components.url
        else {
            return nil
        }
        return url
    }

    func normalizedGatewayURL(_ value: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return trimmed }
        if trimmed.hasPrefix("http://") || trimmed.hasPrefix("https://") {
            return trimmed.replacingOccurrences(
                of: "/+$",
                with: "",
                options: .regularExpression
            )
        }
        let withoutTrailingSlash = trimmed.replacingOccurrences(
            of: "/+$",
            with: "",
            options: .regularExpression
        )
        return "http://\(withoutTrailingSlash)"
    }

    func mobileRole(for role: GaryxTranscriptRole) -> GaryxMobileMessage.Role {
        switch role {
        case .assistant:
            .assistant
        case .user:
            .user
        case .tool, .toolUse, .toolResult:
            .tool
        case .system, .unknown:
            .system
        }
    }

    func displayMessage(for error: Error) -> String {
        if Self.isCancellationError(error) {
            return ""
        }
        if let localized = (error as? LocalizedError)?.errorDescription, !localized.isEmpty {
            return localized
        }
        return error.localizedDescription
    }

    static func presentableErrorMessage(_ message: String?) -> String? {
        let trimmed = message?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !trimmed.isEmpty else { return nil }
        guard !isCancellationMessage(trimmed) else { return nil }
        return trimmed
    }

    static func isCancellationError(_ error: Error) -> Bool {
        if error is CancellationError {
            return true
        }
        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain && nsError.code == NSURLErrorCancelled {
            return true
        }
        return isCancellationMessage(error.localizedDescription)
    }

    static func isCancellationMessage(_ message: String) -> Bool {
        let normalized = message
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        return normalized == "cancel"
            || normalized == "cancel."
            || normalized == "cancelled"
            || normalized == "canceled"
            || normalized == "cancelled."
            || normalized == "canceled."
            || normalized == "the operation was cancelled."
            || normalized == "the operation was canceled."
            || normalized == "the operation couldn’t be completed. (nsurlerrordomain error -999.)"
            || normalized == "the operation could not be completed. (nsurlerrordomain error -999.)"
    }
}
