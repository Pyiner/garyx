import Foundation

public enum GaryxJSONValue: Codable, Equatable, Sendable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: GaryxJSONValue])
    case array([GaryxJSONValue])
    case null

    public init(from decoder: Decoder) throws {
        let single = try decoder.singleValueContainer()
        if single.decodeNil() {
            self = .null
        } else if let value = try? single.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? single.decode(Double.self) {
            self = .number(value)
        } else if let value = try? single.decode(String.self) {
            self = .string(value)
        } else if let value = try? single.decode([GaryxJSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try single.decode([String: GaryxJSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var single = encoder.singleValueContainer()
        switch self {
        case .string(let value):
            try single.encode(value)
        case .number(let value):
            try single.encode(value)
        case .bool(let value):
            try single.encode(value)
        case .object(let value):
            try single.encode(value)
        case .array(let value):
            try single.encode(value)
        case .null:
            try single.encodeNil()
        }
    }
}


public struct GaryxContentAttachmentDescriptor: Equatable, Sendable {
    public var id: String
    public var kind: String
    public var name: String
    public var mediaType: String
    public var path: String?
    public var dataUrl: String?
    public var remoteUrl: String?

    public init(
        id: String,
        kind: String,
        name: String,
        mediaType: String,
        path: String? = nil,
        dataUrl: String? = nil,
        remoteUrl: String? = nil
    ) {
        self.id = id
        self.kind = kind
        self.name = name
        self.mediaType = mediaType
        self.path = path
        self.dataUrl = dataUrl
        self.remoteUrl = remoteUrl
    }

    public var isImage: Bool {
        kind.caseInsensitiveCompare("image") == .orderedSame || mediaType.hasPrefix("image/")
    }
}


public enum GaryxStructuredContentRenderer {
    public static func attachments(from content: GaryxJSONValue?) -> [GaryxContentAttachmentDescriptor] {
        guard let content else { return [] }
        var attachments: [GaryxContentAttachmentDescriptor] = []

        @discardableResult
        func appendAttachment(from object: [String: GaryxJSONValue], fallbackIndex: Int) -> Bool {
            let type = object.garyxGatewayStringValue(forKeys: ["type", "kind"])?.lowercased() ?? ""
            let mediaType = object.garyxGatewayStringValue(forKeys: ["media_type", "mediaType"])
                ?? object.garyxGatewayObjectValue(forKeys: ["source"])?.garyxGatewayStringValue(forKeys: ["media_type", "mediaType"])
                ?? ""
            let path = object.garyxGatewayStringValue(forKeys: ["path", "file_path", "filePath"])
            let name = object.garyxGatewayStringValue(forKeys: ["name", "filename", "file_name"])
                ?? path?.garyxLastPathComponent
                ?? (type.contains("image") || mediaType.hasPrefix("image/") ? "Image" : "Attachment")
            let source = object.garyxGatewayObjectValue(forKeys: ["source"])
            let base64 = source?.garyxGatewayStringValue(forKeys: ["data"])
                ?? object.garyxGatewayStringValue(forKeys: ["data", "base64"])
            let attachmentDataUrl: String?
            if let base64 {
                attachmentDataUrl = base64.hasPrefix("data:")
                    ? base64
                    : makeDataUrl(mediaType: mediaType.isEmpty ? "image/jpeg" : mediaType, base64: base64)
            } else {
                attachmentDataUrl = nil
            }
            let remoteUrl = object.garyxGatewayStringValue(forKeys: ["url", "image_url", "imageUrl"])
                ?? source?.garyxGatewayStringValue(forKeys: ["url"])
            let isImage = type.contains("image")
                || mediaType.hasPrefix("image/")
                || attachmentDataUrl != nil
                || remoteUrl != nil
            guard isImage || type == "file" || type == "attachment" || path != nil else { return false }
            let attachmentIdBase = object.garyxGatewayStringValue(forKeys: ["id"])
                ?? path
                ?? remoteUrl
                ?? (type.isEmpty ? "attachment" : type)
            attachments.append(
                GaryxContentAttachmentDescriptor(
                    id: "\(attachmentIdBase)-\(fallbackIndex)",
                    kind: isImage ? "image" : "file",
                    name: name,
                    mediaType: mediaType,
                    path: path,
                    dataUrl: attachmentDataUrl,
                    remoteUrl: remoteUrl
                )
            )
            return true
        }

        func inspect(_ value: GaryxJSONValue) {
            switch value.garyxGatewayJSONStringDecodedIfNeeded {
            case .array(let items):
                items.forEach(inspect)
            case .object(let object):
                if appendAttachment(from: object, fallbackIndex: attachments.count) {
                    return
                }
                if let nested = object["content"]?.garyxGatewayJSONStringDecodedIfNeeded {
                    inspect(nested)
                }
            case .string, .number, .bool, .null:
                break
            }
        }

        inspect(content)
        return attachments
    }

    public static func text(from value: GaryxJSONValue) -> String? {
        var parts: [String] = []

        func inspect(_ value: GaryxJSONValue) {
            switch value.garyxGatewayJSONStringDecodedIfNeeded {
            case .string(let text):
                let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty {
                    parts.append(trimmed)
                }
            case .array(let items):
                items.forEach(inspect)
            case .object(let object):
                let type = object.garyxGatewayStringValue(forKeys: ["type", "kind"])?.lowercased() ?? ""
                if type == "text" || type == "input_text" {
                    if let text = object.garyxGatewayStringValue(forKeys: ["text", "content"]) {
                        parts.append(text.trimmingCharacters(in: .whitespacesAndNewlines))
                    }
                    return
                }
                if let nested = object["content"]?.garyxGatewayJSONStringDecodedIfNeeded,
                   type != "image",
                   type != "file" {
                    inspect(nested)
                }
            case .number, .bool, .null:
                break
            }
        }

        inspect(value)
        let text = parts.joined(separator: "\n\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }

    public static func summaryText(from value: GaryxJSONValue) -> String? {
        let text = text(from: value)
        let attachments = attachments(from: value)
        guard let summary = attachmentSummary(from: attachments) else {
            return text
        }
        if let text, !text.isEmpty {
            return "\(text)\n\n\(summary)"
        }
        return summary
    }

    public static func userMergeKey(
        text: String,
        attachments: [GaryxContentAttachmentDescriptor]
    ) -> String {
        let normalizedText = normalizedMergeText(text)
        guard !attachments.isEmpty,
              let attachmentSummary = attachmentSummary(from: attachments) else {
            return normalizedText
        }
        if normalizedText.isEmpty || normalizedText == attachmentSummary {
            return attachmentSummary
        }
        return normalizedText
    }

    public static func attachmentSummary(from attachments: [GaryxContentAttachmentDescriptor]) -> String? {
        let imageCount = attachments.filter(\.isImage).count
        let fileCount = max(attachments.count - imageCount, 0)
        return attachmentSummary(imageCount: imageCount, fileCount: fileCount)
    }

    public static func attachmentSummary(imageCount: Int, fileCount: Int) -> String? {
        var parts: [String] = []
        if imageCount > 0 {
            parts.append("\(imageCount) image\(imageCount == 1 ? "" : "s")")
        }
        if fileCount > 0 {
            parts.append("\(fileCount) file\(fileCount == 1 ? "" : "s")")
        }
        return parts.isEmpty ? nil : "[\(parts.joined(separator: ", "))]"
    }

    private static func normalizedMergeText(_ text: String) -> String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\r\n", with: "\n")
    }

    private static func makeDataUrl(mediaType: String, base64: String) -> String {
        let normalizedType = mediaType.trimmingCharacters(in: .whitespacesAndNewlines)
        let type = normalizedType.isEmpty ? "application/octet-stream" : normalizedType
        return "data:\(type);base64,\(base64)"
    }
}
