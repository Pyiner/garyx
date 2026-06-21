import Foundation

public enum GaryxThreadArchiveRequestBuilder {
    public static func endpointKeys(
        threadId: String,
        endpoints: [GaryxChannelEndpoint],
        additionalEndpointKey: String? = nil
    ) -> [String] {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedThreadId.isEmpty else { return [] }

        var keys = Set<String>()
        for endpoint in endpoints {
            let endpointThreadId = endpoint.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard endpointThreadId == normalizedThreadId else { continue }
            let endpointKey = endpoint.endpointKey.trimmingCharacters(in: .whitespacesAndNewlines)
            if !endpointKey.isEmpty {
                keys.insert(endpointKey)
            }
        }

        let additional = additionalEndpointKey?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !additional.isEmpty {
            keys.insert(additional)
        }

        return keys.sorted()
    }
}
