import Foundation

public struct GaryxMobileConnectPayload: Equatable, Sendable {
    public var gatewayUrl: String
    public var gatewayAuthToken: String
    public var gatewayHeaders: String

    public init(gatewayUrl: String, gatewayAuthToken: String, gatewayHeaders: String = "") {
        self.gatewayUrl = gatewayUrl
        self.gatewayAuthToken = gatewayAuthToken
        self.gatewayHeaders = GaryxGatewayHeaders.normalizedBlock(gatewayHeaders)
    }
}

public enum GaryxMobileConnectLink {
    public static func make(
        gatewayUrl: String,
        gatewayAuthToken: String,
        gatewayHeaders: String = ""
    ) -> URL? {
        var components = URLComponents()
        components.scheme = "garyx"
        components.host = "mobile"
        components.path = "/connect"
        var queryItems = [
            URLQueryItem(name: "gatewayUrl", value: gatewayUrl),
            URLQueryItem(name: "gatewayAuthToken", value: gatewayAuthToken),
        ]
        let normalizedHeaders = GaryxGatewayHeaders.normalizedBlock(gatewayHeaders)
        if !normalizedHeaders.isEmpty {
            queryItems.append(URLQueryItem(name: "gatewayHeaders", value: normalizedHeaders))
        }
        components.queryItems = queryItems
        return components.url
    }

    public static func parse(_ url: URL) -> GaryxMobileConnectPayload? {
        guard url.scheme?.lowercased() == "garyx" else {
            return nil
        }
        let host = url.host()?.lowercased()
        let path = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")).lowercased()
        guard host == "mobile" || host == "connect" || path == "mobile/connect" || path == "connect" else {
            return nil
        }
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return nil
        }
        let items = components.queryItems ?? []
        func query(_ names: String...) -> String {
            for name in names {
                if let value = items
                    .first(where: { $0.name == name })?
                    .value?
                    .trimmingCharacters(in: .whitespacesAndNewlines),
                    !value.isEmpty {
                    return value
                }
            }
            return ""
        }
        let gatewayUrl = query("gatewayUrl", "gateway_url", "url")
        guard !gatewayUrl.isEmpty else {
            return nil
        }
        return GaryxMobileConnectPayload(
            gatewayUrl: gatewayUrl,
            gatewayAuthToken: query("gatewayAuthToken", "gateway_auth_token", "token"),
            gatewayHeaders: query("gatewayHeaders", "gateway_headers", "headers")
        )
    }
}
