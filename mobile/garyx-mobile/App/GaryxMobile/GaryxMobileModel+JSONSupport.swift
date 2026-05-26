import Foundation

extension GaryxJSONValue {
    static func decoded(from text: String) -> GaryxJSONValue? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{") || trimmed.hasPrefix("[") else { return nil }
        return try? JSONDecoder().decode(GaryxJSONValue.self, from: Data(trimmed.utf8))
    }

    var objectValue: [String: GaryxJSONValue]? {
        if case .object(let value) = self {
            return value
        }
        return nil
    }

    var arrayValue: [GaryxJSONValue]? {
        if case .array(let value) = self {
            return value
        }
        return nil
    }

    var jsonStringDecodedIfNeeded: GaryxJSONValue {
        if case .string(let value) = self,
           let decoded = GaryxJSONValue.decoded(from: value) {
            return decoded
        }
        return self
    }

    var stringValue: String? {
        switch self {
        case .string(let value):
            return value.garyxTrimmedNilIfEmpty
        case .number(let value):
            if value.rounded() == value {
                return String(Int(value))
            }
            return String(value).garyxTrimmedNilIfEmpty
        case .bool(let value):
            return value ? "true" : "false"
        case .null:
            return nil
        case .array, .object:
            return prettyPrinted
        }
    }

    var boolValue: Bool? {
        switch self {
        case .bool(let value):
            return value
        case .string(let value):
            let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if ["true", "yes", "1"].contains(normalized) {
                return true
            }
            if ["false", "no", "0"].contains(normalized) {
                return false
            }
            return nil
        default:
            return nil
        }
    }

    var prettyPrinted: String {
        if case .string(let value) = self {
            return value
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(self),
              let text = String(data: data, encoding: .utf8) else {
            return ""
        }
        return text
    }

    var isMeaningful: Bool {
        switch self {
        case .null:
            false
        case .string(let value):
            !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        case .array(let values):
            !values.isEmpty
        case .object(let values):
            !values.isEmpty
        case .number, .bool:
            true
        }
    }
}

extension Dictionary where Key == String, Value == GaryxJSONValue {
    var unwrappedToolPayloadValue: GaryxJSONValue? {
        guard let content = self["content"]?.jsonStringDecodedIfNeeded else { return nil }
        let hasEnvelopeMarkers = self["toolName"] != nil
            || self["tool_name"] != nil
            || self["toolUseId"] != nil
            || self["tool_use_id"] != nil
            || self["metadata"] != nil
            || self["role"] != nil
        return hasEnvelopeMarkers ? content : nil
    }

    func stringValue(forKeys keys: [String]) -> String? {
        for key in keys {
            if let value = self[key]?.stringValue?.garyxTrimmedNilIfEmpty {
                return value
            }
        }
        return nil
    }

    func boolValue(forKeys keys: [String]) -> Bool? {
        for key in keys {
            if let value = self[key]?.boolValue {
                return value
            }
        }
        return nil
    }

    func objectValue(forKeys keys: [String]) -> [String: GaryxJSONValue]? {
        for key in keys {
            if let value = self[key]?.objectValue {
                return value
            }
        }
        return nil
    }

    func detailText(forKeys keys: [String]) -> String? {
        for key in keys {
            guard let value = self[key], value.isMeaningful else { continue }
            if key == "message", value.objectValue != nil {
                continue
            }
            if let text = value.stringValue?.garyxTrimmedNilIfEmpty {
                return text
            }
        }
        return nil
    }
}

extension String {
    var garyxTrimmedNilIfEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

}
