import Foundation

public struct GaryxImageSaveRequest: Equatable, Sendable {
    public let title: String
    public let dataURL: String?
    public let filePath: String?
    public let gatewayFilePath: String?
    public let remoteURL: String?

    public init(
        title: String,
        dataURL: String? = nil,
        filePath: String? = nil,
        gatewayFilePath: String? = nil,
        remoteURL: String? = nil
    ) {
        self.title = title
        self.dataURL = dataURL
        self.filePath = filePath
        self.gatewayFilePath = gatewayFilePath
        self.remoteURL = remoteURL
    }

    /// Mirrors preview decoding priority so Save always targets the same bytes
    /// the user is viewing. Invalid or unavailable candidates fall through to
    /// the next source rather than making a lower-priority source unreachable.
    public var candidates: [GaryxImageSaveCandidate] {
        var result: [GaryxImageSaveCandidate] = []
        if let value = Self.nonEmpty(dataURL) {
            result.append(.inlineData(value))
        }
        if let value = Self.nonEmpty(filePath) {
            result.append(.localFile(value))
        }
        if let value = Self.nonEmpty(gatewayFilePath) {
            result.append(.gatewayFile(value))
        }
        if let value = Self.nonEmpty(remoteURL),
           let url = URL(string: value),
           let scheme = url.scheme?.lowercased(),
           scheme == "http" || scheme == "https" {
            result.append(.remoteURL(value))
        }
        return result
    }

    private static func nonEmpty(_ value: String?) -> String? {
        guard let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              !trimmed.isEmpty else {
            return nil
        }
        return trimmed
    }
}

public enum GaryxImageSaveCandidate: Equatable, Sendable {
    case inlineData(String)
    case localFile(String)
    case gatewayFile(String)
    case remoteURL(String)
}

public struct GaryxImageSaveRemoteResource: Equatable, Sendable {
    public let data: Data
    public let mimeType: String?
    public let suggestedFilename: String?

    public init(data: Data, mimeType: String? = nil, suggestedFilename: String? = nil) {
        self.data = data
        self.mimeType = mimeType
        self.suggestedFilename = suggestedFilename
    }
}

public struct GaryxImageSavePayload: Equatable, Sendable {
    public let data: Data
    public let originalFilename: String
    public let uniformTypeIdentifier: String

    public init(data: Data, originalFilename: String, uniformTypeIdentifier: String) {
        self.data = data
        self.originalFilename = originalFilename
        self.uniformTypeIdentifier = uniformTypeIdentifier
    }
}

public enum GaryxImageSaveLoadError: Error, Equatable, Sendable {
    case noUsableImageSource
}

public enum GaryxImageSaveLoader {
    public typealias FileReader = (String) async throws -> Data
    public typealias GatewayDataURLResolver = (String) async throws -> String?
    public typealias RemoteFetcher = (String) async throws -> GaryxImageSaveRemoteResource

    public static func load(
        _ request: GaryxImageSaveRequest,
        readFile: FileReader,
        resolveGatewayDataURL: GatewayDataURLResolver,
        fetchRemote: RemoteFetcher
    ) async throws -> GaryxImageSavePayload {
        for candidate in request.candidates {
            do {
                let payload: GaryxImageSavePayload?
                switch candidate {
                case let .inlineData(raw):
                    payload = GaryxImageSavePayloadFactory.makeInlinePayload(
                        raw,
                        title: request.title,
                        sourceName: nil
                    )
                case let .localFile(path):
                    let data = try await readFile(path)
                    payload = GaryxImageSavePayloadFactory.makePayload(
                        data: data,
                        title: request.title,
                        sourceName: path,
                        mediaType: nil
                    )
                case let .gatewayFile(path):
                    guard let raw = try await resolveGatewayDataURL(path) else {
                        payload = nil
                        break
                    }
                    payload = GaryxImageSavePayloadFactory.makeInlinePayload(
                        raw,
                        title: request.title,
                        sourceName: path
                    )
                case let .remoteURL(url):
                    let resource = try await fetchRemote(url)
                    payload = GaryxImageSavePayloadFactory.makePayload(
                        data: resource.data,
                        title: request.title,
                        sourceName: resource.suggestedFilename ?? url,
                        mediaType: resource.mimeType
                    )
                }
                if let payload {
                    return payload
                }
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                // A preview may carry several representations. Match the
                // display loader by trying the next representation when one
                // path is stale, malformed, or temporarily unavailable.
                continue
            }
        }
        throw GaryxImageSaveLoadError.noUsableImageSource
    }
}

public enum GaryxImageSavePayloadFactory {
    public static func makeInlinePayload(
        _ raw: String,
        title: String,
        sourceName: String?
    ) -> GaryxImageSavePayload? {
        guard let decoded = GaryxInlineImageData.decode(raw) else { return nil }
        return makePayload(
            data: decoded.data,
            title: title,
            sourceName: sourceName,
            mediaType: decoded.mediaType
        )
    }

    public static func makePayload(
        data: Data,
        title: String,
        sourceName: String?,
        mediaType: String?
    ) -> GaryxImageSavePayload? {
        guard !data.isEmpty,
              let format = GaryxImageFileFormat.infer(
                  data: data,
                  mediaType: mediaType,
                  sourceName: sourceName ?? title
              ) else {
            return nil
        }
        return GaryxImageSavePayload(
            data: data,
            originalFilename: filename(
                title: title,
                sourceName: sourceName,
                fileExtension: format.fileExtension
            ),
            uniformTypeIdentifier: format.uniformTypeIdentifier
        )
    }

    private static func filename(title: String, sourceName: String?, fileExtension: String) -> String {
        let trimmedTitle = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let preferredRaw: String
        if trimmedTitle.isEmpty || trimmedTitle.caseInsensitiveCompare("Image") == .orderedSame {
            preferredRaw = sourceName ?? "Image"
        } else {
            preferredRaw = trimmedTitle
        }

        var leaf = sourceLeafName(preferredRaw)
        let currentExtension = URL(fileURLWithPath: leaf).pathExtension.lowercased()
        if GaryxImageFileFormat.knownExtensions.contains(currentExtension) {
            leaf = (leaf as NSString).deletingPathExtension
        }
        let invalid = CharacterSet.controlCharacters.union(CharacterSet(charactersIn: #"/:\"?*<>|"#))
        leaf = leaf.components(separatedBy: invalid).joined(separator: "-")
        leaf = leaf.trimmingCharacters(in: .whitespacesAndNewlines.union(CharacterSet(charactersIn: ".")))
        if leaf.isEmpty {
            leaf = "Image"
        }
        leaf = String(leaf.prefix(120))
        return "\(leaf).\(fileExtension)"
    }

    private static func sourceLeafName(_ raw: String) -> String {
        if let url = URL(string: raw),
           let scheme = url.scheme?.lowercased(),
           scheme == "http" || scheme == "https" || scheme == "file",
           !url.lastPathComponent.isEmpty {
            return url.lastPathComponent.removingPercentEncoding ?? url.lastPathComponent
        }
        let leaf = URL(fileURLWithPath: raw).lastPathComponent
        return leaf.isEmpty ? raw : leaf
    }
}

public struct GaryxImageFileFormat: Equatable, Sendable {
    public let fileExtension: String
    public let uniformTypeIdentifier: String

    public init(fileExtension: String, uniformTypeIdentifier: String) {
        self.fileExtension = fileExtension
        self.uniformTypeIdentifier = uniformTypeIdentifier
    }

    public static let knownExtensions: Set<String> = [
        "png", "jpg", "jpeg", "gif", "heic", "heif", "webp", "tif", "tiff", "bmp", "avif",
    ]

    public static func infer(data: Data, mediaType: String?, sourceName: String) -> Self? {
        sniff(data)
            ?? fromMediaType(mediaType)
            ?? fromFileExtension(URL(fileURLWithPath: sourceName).pathExtension)
    }

    private static func sniff(_ data: Data) -> Self? {
        let bytes = [UInt8](data.prefix(16))
        if bytes.starts(with: [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            return Self(fileExtension: "png", uniformTypeIdentifier: "public.png")
        }
        if bytes.starts(with: [0xFF, 0xD8, 0xFF]) {
            return Self(fileExtension: "jpg", uniformTypeIdentifier: "public.jpeg")
        }
        if bytes.starts(with: Array("GIF87a".utf8)) || bytes.starts(with: Array("GIF89a".utf8)) {
            return Self(fileExtension: "gif", uniformTypeIdentifier: "com.compuserve.gif")
        }
        if bytes.starts(with: [0x49, 0x49, 0x2A, 0x00]) || bytes.starts(with: [0x4D, 0x4D, 0x00, 0x2A]) {
            return Self(fileExtension: "tiff", uniformTypeIdentifier: "public.tiff")
        }
        if bytes.starts(with: [0x42, 0x4D]) {
            return Self(fileExtension: "bmp", uniformTypeIdentifier: "com.microsoft.bmp")
        }
        if bytes.count >= 12,
           String(bytes: bytes[0..<4], encoding: .ascii) == "RIFF",
           String(bytes: bytes[8..<12], encoding: .ascii) == "WEBP" {
            return Self(fileExtension: "webp", uniformTypeIdentifier: "org.webmproject.webp")
        }
        if bytes.count >= 12,
           String(bytes: bytes[4..<8], encoding: .ascii) == "ftyp" {
            let brand = String(bytes: bytes[8..<12], encoding: .ascii)?.lowercased()
            if ["heic", "heix", "hevc", "hevx"].contains(brand) {
                return Self(fileExtension: "heic", uniformTypeIdentifier: "public.heic")
            }
            if ["mif1", "msf1"].contains(brand) {
                return Self(fileExtension: "heif", uniformTypeIdentifier: "public.heif")
            }
            if ["avif", "avis"].contains(brand) {
                return Self(fileExtension: "avif", uniformTypeIdentifier: "public.avif")
            }
        }
        return nil
    }

    private static func fromMediaType(_ raw: String?) -> Self? {
        switch raw?.split(separator: ";", maxSplits: 1).first?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "image/png": return Self(fileExtension: "png", uniformTypeIdentifier: "public.png")
        case "image/jpeg", "image/jpg": return Self(fileExtension: "jpg", uniformTypeIdentifier: "public.jpeg")
        case "image/gif": return Self(fileExtension: "gif", uniformTypeIdentifier: "com.compuserve.gif")
        case "image/heic": return Self(fileExtension: "heic", uniformTypeIdentifier: "public.heic")
        case "image/heif": return Self(fileExtension: "heif", uniformTypeIdentifier: "public.heif")
        case "image/webp": return Self(fileExtension: "webp", uniformTypeIdentifier: "org.webmproject.webp")
        case "image/tiff": return Self(fileExtension: "tiff", uniformTypeIdentifier: "public.tiff")
        case "image/bmp": return Self(fileExtension: "bmp", uniformTypeIdentifier: "com.microsoft.bmp")
        case "image/avif": return Self(fileExtension: "avif", uniformTypeIdentifier: "public.avif")
        default: return nil
        }
    }

    private static func fromFileExtension(_ raw: String) -> Self? {
        switch raw.lowercased() {
        case "png": return Self(fileExtension: "png", uniformTypeIdentifier: "public.png")
        case "jpg", "jpeg": return Self(fileExtension: "jpg", uniformTypeIdentifier: "public.jpeg")
        case "gif": return Self(fileExtension: "gif", uniformTypeIdentifier: "com.compuserve.gif")
        case "heic": return Self(fileExtension: "heic", uniformTypeIdentifier: "public.heic")
        case "heif": return Self(fileExtension: "heif", uniformTypeIdentifier: "public.heif")
        case "webp": return Self(fileExtension: "webp", uniformTypeIdentifier: "org.webmproject.webp")
        case "tif", "tiff": return Self(fileExtension: "tiff", uniformTypeIdentifier: "public.tiff")
        case "bmp": return Self(fileExtension: "bmp", uniformTypeIdentifier: "com.microsoft.bmp")
        case "avif": return Self(fileExtension: "avif", uniformTypeIdentifier: "public.avif")
        default: return nil
        }
    }
}

private struct GaryxInlineImageData {
    let data: Data
    let mediaType: String?

    static func decode(_ raw: String) -> Self? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        guard trimmed.lowercased().hasPrefix("data:") else {
            guard let data = Data(base64Encoded: trimmed, options: .ignoreUnknownCharacters) else {
                return nil
            }
            return Self(data: data, mediaType: nil)
        }

        guard let comma = trimmed.firstIndex(of: ",") else { return nil }
        let metadata = String(trimmed[trimmed.index(trimmed.startIndex, offsetBy: 5)..<comma])
        let encoded = String(trimmed[trimmed.index(after: comma)...])
        let parts = metadata.split(separator: ";", omittingEmptySubsequences: false).map(String.init)
        let mediaType = parts.first.flatMap { $0.contains("/") ? $0.lowercased() : nil }
        let isBase64 = parts.dropFirst().contains { $0.caseInsensitiveCompare("base64") == .orderedSame }
        let data = isBase64
            ? Data(base64Encoded: encoded, options: .ignoreUnknownCharacters)
            : decodePercentEncodedBytes(encoded)
        guard let data else { return nil }
        return Self(data: data, mediaType: mediaType)
    }

    private static func decodePercentEncodedBytes(_ value: String) -> Data? {
        let bytes = Array(value.utf8)
        var output: [UInt8] = []
        output.reserveCapacity(bytes.count)
        var index = 0
        while index < bytes.count {
            if bytes[index] == 0x25 {
                guard index + 2 < bytes.count,
                      let high = hex(bytes[index + 1]),
                      let low = hex(bytes[index + 2]) else {
                    return nil
                }
                output.append(high << 4 | low)
                index += 3
            } else {
                output.append(bytes[index])
                index += 1
            }
        }
        return Data(output)
    }

    private static func hex(_ byte: UInt8) -> UInt8? {
        switch byte {
        case 0x30...0x39: return byte - 0x30
        case 0x41...0x46: return byte - 0x41 + 10
        case 0x61...0x66: return byte - 0x61 + 10
        default: return nil
        }
    }
}
