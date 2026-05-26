import Foundation

extension KeyedDecodingContainer {
    func garyxDecodeFirstString(_ keys: Key...) throws -> String? {
        for key in keys {
            if let value = try decodeIfPresent(String.self, forKey: key) {
                return value
            }
        }
        return nil
    }

    func garyxDecodeFirstBool(_ keys: Key...) throws -> Bool? {
        for key in keys {
            if let value = try decodeIfPresent(Bool.self, forKey: key) {
                return value
            }
        }
        return nil
    }

    func garyxDecodeFirstInt(_ keys: Key...) throws -> Int? {
        for key in keys {
            if let value = try decodeIfPresent(Int.self, forKey: key) {
                return value
            }
        }
        return nil
    }

    func garyxDecodeFirstStringArray(_ keys: Key...) throws -> [String]? {
        for key in keys {
            if let value = try decodeIfPresent([String].self, forKey: key) {
                return value
            }
        }
        return nil
    }
}


extension GaryxJSONValue {
    static func garyxGatewayDecoded(from text: String) -> GaryxJSONValue? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{") || trimmed.hasPrefix("[") else { return nil }
        return try? JSONDecoder().decode(GaryxJSONValue.self, from: Data(trimmed.utf8))
    }

    var garyxGatewayObjectValue: [String: GaryxJSONValue]? {
        if case .object(let value) = self {
            return value
        }
        return nil
    }

    var garyxGatewayJSONStringDecodedIfNeeded: GaryxJSONValue {
        if case .string(let value) = self,
           let decoded = GaryxJSONValue.garyxGatewayDecoded(from: value) {
            return decoded
        }
        return self
    }

    var garyxGatewayStringValue: String? {
        switch self {
        case .string(let value):
            return value.garyxGatewayTrimmedNilIfEmpty
        case .number(let value):
            if value.rounded() == value,
               let exactInteger = Int(exactly: value) {
                return String(exactInteger)
            }
            return String(value).garyxGatewayTrimmedNilIfEmpty
        case .bool(let value):
            return value ? "true" : "false"
        case .null, .array, .object:
            return nil
        }
    }
}


extension Dictionary where Key == String, Value == GaryxJSONValue {
    func garyxGatewayStringValue(forKeys keys: [String]) -> String? {
        for key in keys {
            if let value = self[key]?.garyxGatewayStringValue?.garyxGatewayTrimmedNilIfEmpty {
                return value
            }
        }
        return nil
    }

    func garyxGatewayObjectValue(forKeys keys: [String]) -> [String: GaryxJSONValue]? {
        for key in keys {
            if let value = self[key]?.garyxGatewayObjectValue {
                return value
            }
        }
        return nil
    }
}

extension String {
    var garyxGatewayTrimmedNilIfEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
