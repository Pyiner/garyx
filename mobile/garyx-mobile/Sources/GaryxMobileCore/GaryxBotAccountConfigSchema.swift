import Foundation

/// One manual channel-auth field declared by a channel plugin's JSON schema
/// (`properties` + `required` + `enum` + `default` + `x-garyx` annotations),
/// as presented in the bot account form (TASK-1753).
public struct GaryxBotSchemaField: Identifiable, Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case string
        case boolean
        case number
    }

    public var id: String { key }
    public var key: String
    public var label: String
    public var kind: Kind
    public var required: Bool
    public var secret: Bool
    public var enumValues: [String]
    public var defaultValue: GaryxJSONValue?
    public var description: String?
    public var placeholder: String

    /// Parses the plugin config schema into ordered form fields: kind from
    /// the JSON-schema `type` (number/integer → number, unknown → string),
    /// label title-cased from the snake_case key, placeholder from the schema
    /// or the Required/Optional default, sorted required-first then by key.
    public static func fields(from schema: [String: GaryxJSONValue]) -> [GaryxBotSchemaField] {
        let properties = garyxBotObjectValue(schema["properties"]) ?? [:]
        let required = Set((garyxBotArrayValue(schema["required"]) ?? []).compactMap(garyxBotStringValueIfPresent))
        return properties
            .compactMap { key, rawValue -> GaryxBotSchemaField? in
                guard let object = garyxBotObjectValue(rawValue) else { return nil }
                let type = garyxBotStringValueIfPresent(object["type"]) ?? "string"
                let enumValues = (garyxBotArrayValue(object["enum"]) ?? []).compactMap(garyxBotStringValueIfPresent)
                let kind: Kind
                switch type {
                case "boolean":
                    kind = .boolean
                case "number", "integer":
                    kind = .number
                default:
                    kind = .string
                }
                let xGaryx = garyxBotObjectValue(object["x-garyx"]) ?? [:]
                let secret = GaryxBotConfigValues.boolValue(xGaryx["secret"]) ?? false
                return GaryxBotSchemaField(
                    key: key,
                    label: key
                        .replacingOccurrences(of: "_", with: " ")
                        .split(separator: " ")
                        .map { $0.prefix(1).uppercased() + $0.dropFirst() }
                        .joined(separator: " "),
                    kind: kind,
                    required: required.contains(key),
                    secret: secret,
                    enumValues: enumValues,
                    defaultValue: object["default"],
                    description: garyxBotStringValueIfPresent(object["description"]),
                    placeholder: garyxBotStringValueIfPresent(object["placeholder"])
                        ?? (required.contains(key) ? "Required" : "Optional")
                )
            }
            .sorted { lhs, rhs in
                if lhs.required != rhs.required {
                    return lhs.required
                }
                return lhs.key.localizedCaseInsensitiveCompare(rhs.key) == .orderedAscending
            }
    }
}

/// Config-value coercion rules shared by the bot-form field editors and the
/// save path.
public enum GaryxBotConfigValues {
    /// The text an editor shows for a stored config value: integral numbers
    /// drop the decimal point, booleans stringify, containers/null are empty.
    public static func stringValue(_ value: GaryxJSONValue?) -> String {
        guard let value else { return "" }
        switch value {
        case .string(let text):
            return text
        case .number(let number):
            if number.rounded() == number {
                return String(Int(number))
            }
            return String(number)
        case .bool(let flag):
            return flag ? "true" : "false"
        case .null:
            return ""
        case .array, .object:
            return ""
        }
    }

    /// Lenient boolean read: real bools, "true"/"yes"/"1" and
    /// "false"/"no"/"0" strings, and numeric zero/non-zero; anything else is
    /// indeterminate (nil).
    public static func boolValue(_ value: GaryxJSONValue?) -> Bool? {
        guard let value else { return nil }
        switch value {
        case .bool(let flag):
            return flag
        case .string(let text):
            let normalized = text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if ["true", "yes", "1"].contains(normalized) {
                return true
            }
            if ["false", "no", "0"].contains(normalized) {
                return false
            }
            return nil
        case .number(let number):
            return number != 0
        case .null, .array, .object:
            return nil
        }
    }

    /// The value a field editor binds before the user touches it: the stored
    /// config value, then the schema default, then the kind fallback.
    public static func editorValue(
        for field: GaryxBotSchemaField,
        config: [String: GaryxJSONValue]
    ) -> GaryxJSONValue {
        config[field.key] ?? field.defaultValue ?? (field.kind == .boolean ? .bool(false) : .string(""))
    }

    /// Text-editor input mapped into the field's JSON type: number editors
    /// coerce per keystroke (unparseable text becomes 0), everything else
    /// stays a string.
    public static func fieldValue(fromEditorText text: String, kind: GaryxBotSchemaField.Kind) -> GaryxJSONValue {
        kind == .number ? .number(Double(text) ?? 0) : .string(text)
    }

    /// Seeds schema defaults into a config draft without overwriting existing
    /// values; `replacing` starts from an empty draft (channel switch).
    public static func applyingSchemaDefaults(
        to config: [String: GaryxJSONValue],
        fields: [GaryxBotSchemaField],
        replacing: Bool
    ) -> [String: GaryxJSONValue] {
        var next = replacing ? [:] : config
        for field in fields {
            if next[field.key] == nil, let defaultValue = field.defaultValue {
                next[field.key] = defaultValue
            }
        }
        return next
    }

    /// Save-time normalization: booleans coerce leniently (indeterminate →
    /// false), numbers/strings trim and drop empty optional entries while
    /// empty required entries persist as typed empties; keys not declared by
    /// the schema pass through untouched; a missing value falls back to the
    /// schema default before coercion.
    public static func normalized(
        config: [String: GaryxJSONValue],
        fields: [GaryxBotSchemaField]
    ) -> [String: GaryxJSONValue] {
        var next = config
        for field in fields {
            let value = config[field.key] ?? field.defaultValue ?? .string("")
            switch field.kind {
            case .boolean:
                next[field.key] = .bool(boolValue(value) ?? false)
            case .number:
                let text = stringValue(value).trimmingCharacters(in: .whitespacesAndNewlines)
                if text.isEmpty, !field.required {
                    next.removeValue(forKey: field.key)
                } else {
                    next[field.key] = .number(Double(text) ?? 0)
                }
            case .string:
                let text = stringValue(value).trimmingCharacters(in: .whitespacesAndNewlines)
                if text.isEmpty, !field.required {
                    next.removeValue(forKey: field.key)
                } else {
                    next[field.key] = .string(text)
                }
            }
        }
        return next
    }
}

/// Default bot account id generation for the add-bot form.
public enum GaryxBotAccountIdDefaults {
    /// Channel slug + "-main", uniquified against existing ids with a
    /// `-2`…`-99` suffix walk and a final `-new` fallback.
    public static func defaultAccountId(
        channel: String,
        existingAccountIds: Set<String>
    ) -> String {
        let slug = channel
            .lowercased()
            .map { $0.isLetter || $0.isNumber ? String($0) : "-" }
            .joined()
            .split(separator: "-")
            .joined(separator: "-")
        let base = "\(slug.isEmpty ? "bot" : slug)-main"
        if !existingAccountIds.contains(base) {
            return base
        }
        for index in 2...99 {
            let candidate = "\(base)-\(index)"
            if !existingAccountIds.contains(candidate) {
                return candidate
            }
        }
        return "\(base)-new"
    }
}

private func garyxBotObjectValue(_ value: GaryxJSONValue?) -> [String: GaryxJSONValue]? {
    guard case .object(let object) = value else { return nil }
    return object
}

private func garyxBotArrayValue(_ value: GaryxJSONValue?) -> [GaryxJSONValue]? {
    guard case .array(let values) = value else { return nil }
    return values
}

private func garyxBotStringValueIfPresent(_ value: GaryxJSONValue?) -> String? {
    let text = GaryxBotConfigValues.stringValue(value).trimmingCharacters(in: .whitespacesAndNewlines)
    return text.isEmpty ? nil : text
}
