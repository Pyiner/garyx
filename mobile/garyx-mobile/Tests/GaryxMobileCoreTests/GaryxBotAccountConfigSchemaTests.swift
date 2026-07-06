import XCTest
@testable import GaryxMobileCore

/// Characterization tests for the bot account form logic (TASK-1753):
/// schema-field parsing, config-value coercion, and default account id
/// generation previously inlined in `GaryxMobileBotSettingsViews.swift`.
final class GaryxBotAccountConfigSchemaTests: XCTestCase {
    private func field(
        key: String,
        kind: GaryxBotSchemaField.Kind,
        required: Bool = false,
        defaultValue: GaryxJSONValue? = nil
    ) -> GaryxBotSchemaField {
        GaryxBotSchemaField(
            key: key,
            label: key,
            kind: kind,
            required: required,
            secret: false,
            enumValues: [],
            defaultValue: defaultValue,
            description: nil,
            placeholder: ""
        )
    }

    // MARK: Schema parsing

    func testFieldsParsing() {
        let schema: [String: GaryxJSONValue] = [
            "properties": .object([
                "bot_token": .object([
                    "type": .string("string"),
                    "description": .string("Token from BotFather"),
                    "placeholder": .string("123:abc"),
                    "x-garyx": .object(["secret": .bool(true)]),
                ]),
                "enabled_flag": .object([
                    "type": .string("boolean"),
                    "default": .bool(true),
                ]),
                "poll_interval": .object([
                    "type": .string("integer"),
                    "default": .number(30),
                ]),
                "mode": .object([
                    "type": .string("string"),
                    "enum": .array([.string("polling"), .string("webhook")]),
                    "default": .string("polling"),
                ]),
                "weird": .object([
                    "type": .string("unknown-type"),
                ]),
                "skipped": .string("not-an-object"),
            ]),
            "required": .array([.string("bot_token")]),
        ]

        let fields = GaryxBotSchemaField.fields(from: schema)
        // Required first, then case-insensitive key order; non-object
        // properties are skipped.
        XCTAssertEqual(fields.map(\.key), ["bot_token", "enabled_flag", "mode", "poll_interval", "weird"])

        let token = fields[0]
        XCTAssertEqual(token.kind, .string)
        XCTAssertTrue(token.required)
        XCTAssertTrue(token.secret)
        XCTAssertEqual(token.label, "Bot Token")
        XCTAssertEqual(token.description, "Token from BotFather")
        XCTAssertEqual(token.placeholder, "123:abc")
        XCTAssertEqual(token.enumValues, [])

        let enabled = fields[1]
        XCTAssertEqual(enabled.kind, .boolean)
        XCTAssertFalse(enabled.required)
        XCTAssertFalse(enabled.secret)
        XCTAssertEqual(enabled.label, "Enabled Flag")
        XCTAssertEqual(enabled.defaultValue, .bool(true))

        let mode = fields[2]
        XCTAssertEqual(mode.kind, .string)
        XCTAssertEqual(mode.enumValues, ["polling", "webhook"])
        XCTAssertEqual(mode.defaultValue, .string("polling"))

        let interval = fields[3]
        XCTAssertEqual(interval.kind, .number, "integer schema type maps to number kind")
        XCTAssertEqual(interval.defaultValue, .number(30))

        XCTAssertEqual(fields[4].kind, .string, "unknown schema type maps to string kind")
    }

    func testFieldsPlaceholderDefaultsByRequiredness() {
        let schema: [String: GaryxJSONValue] = [
            "properties": .object([
                "needed": .object(["type": .string("string")]),
                "extra": .object(["type": .string("string")]),
            ]),
            "required": .array([.string("needed")]),
        ]
        let fields = GaryxBotSchemaField.fields(from: schema)
        XCTAssertEqual(fields.map(\.key), ["needed", "extra"])
        XCTAssertEqual(fields[0].placeholder, "Required")
        XCTAssertEqual(fields[1].placeholder, "Optional")
    }

    func testFieldsFromEmptySchema() {
        XCTAssertEqual(GaryxBotSchemaField.fields(from: [:]), [])
    }

    // MARK: Value coercion

    func testStringValue() {
        XCTAssertEqual(GaryxBotConfigValues.stringValue(.string("x")), "x")
        XCTAssertEqual(GaryxBotConfigValues.stringValue(.number(2)), "2")
        XCTAssertEqual(GaryxBotConfigValues.stringValue(.number(2.5)), "2.5")
        XCTAssertEqual(GaryxBotConfigValues.stringValue(.bool(true)), "true")
        XCTAssertEqual(GaryxBotConfigValues.stringValue(.bool(false)), "false")
        XCTAssertEqual(GaryxBotConfigValues.stringValue(.null), "")
        XCTAssertEqual(GaryxBotConfigValues.stringValue(.array([.string("a")])), "")
        XCTAssertEqual(GaryxBotConfigValues.stringValue(.object([:])), "")
        XCTAssertEqual(GaryxBotConfigValues.stringValue(nil), "")
    }

    func testBoolValue() {
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.bool(true)), true)
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.bool(false)), false)
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.string(" YES ")), true)
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.string("true")), true)
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.string("1")), true)
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.string("false")), false)
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.string("No")), false)
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.string("0")), false)
        XCTAssertNil(GaryxBotConfigValues.boolValue(.string("maybe")))
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.number(0)), false)
        XCTAssertEqual(GaryxBotConfigValues.boolValue(.number(2)), true)
        XCTAssertNil(GaryxBotConfigValues.boolValue(.null))
        XCTAssertNil(GaryxBotConfigValues.boolValue(.array([])))
        XCTAssertNil(GaryxBotConfigValues.boolValue(.object([:])))
        XCTAssertNil(GaryxBotConfigValues.boolValue(nil))
    }

    func testEditorValue() {
        let stringField = field(key: "token", kind: .string)
        let boolField = field(key: "flag", kind: .boolean)
        let defaulted = field(key: "mode", kind: .string, defaultValue: .string("polling"))

        XCTAssertEqual(
            GaryxBotConfigValues.editorValue(for: stringField, config: ["token": .string("abc")]),
            .string("abc")
        )
        XCTAssertEqual(GaryxBotConfigValues.editorValue(for: defaulted, config: [:]), .string("polling"))
        XCTAssertEqual(GaryxBotConfigValues.editorValue(for: boolField, config: [:]), .bool(false))
        XCTAssertEqual(GaryxBotConfigValues.editorValue(for: stringField, config: [:]), .string(""))
    }

    func testFieldValueFromEditorText() {
        XCTAssertEqual(GaryxBotConfigValues.fieldValue(fromEditorText: "1.5", kind: .number), .number(1.5))
        XCTAssertEqual(GaryxBotConfigValues.fieldValue(fromEditorText: "junk", kind: .number), .number(0))
        XCTAssertEqual(GaryxBotConfigValues.fieldValue(fromEditorText: "abc", kind: .string), .string("abc"))
        // Boolean fields never render text editors, but the mapping stays
        // string for non-number kinds.
        XCTAssertEqual(GaryxBotConfigValues.fieldValue(fromEditorText: "true", kind: .boolean), .string("true"))
    }

    func testApplyingSchemaDefaults() {
        let fields = [
            field(key: "mode", kind: .string, defaultValue: .string("polling")),
            field(key: "token", kind: .string),
        ]
        let existing: [String: GaryxJSONValue] = [
            "mode": .string("webhook"),
            "legacy": .string("keep-me"),
        ]

        // Merging keeps user values and only fills missing defaults.
        let merged = GaryxBotConfigValues.applyingSchemaDefaults(to: existing, fields: fields, replacing: false)
        XCTAssertEqual(merged, ["mode": .string("webhook"), "legacy": .string("keep-me")])

        let seeded = GaryxBotConfigValues.applyingSchemaDefaults(to: [:], fields: fields, replacing: false)
        XCTAssertEqual(seeded, ["mode": .string("polling")])

        // Replacing resets to schema defaults only (channel switch).
        let replaced = GaryxBotConfigValues.applyingSchemaDefaults(to: existing, fields: fields, replacing: true)
        XCTAssertEqual(replaced, ["mode": .string("polling")])
    }

    // MARK: Save normalization

    func testNormalizedBoolean() {
        let flag = field(key: "flag", kind: .boolean)
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["flag": .string("yes")], fields: [flag]),
            ["flag": .bool(true)]
        )
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["flag": .string("junk")], fields: [flag]),
            ["flag": .bool(false)]
        )
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["flag": .number(1)], fields: [flag]),
            ["flag": .bool(true)]
        )
        // Missing value coerces to false; a schema default wins first.
        XCTAssertEqual(GaryxBotConfigValues.normalized(config: [:], fields: [flag]), ["flag": .bool(false)])
        let defaulted = field(key: "flag", kind: .boolean, defaultValue: .bool(true))
        XCTAssertEqual(GaryxBotConfigValues.normalized(config: [:], fields: [defaulted]), ["flag": .bool(true)])
    }

    func testNormalizedNumber() {
        let optionalPort = field(key: "port", kind: .number)
        let requiredPort = field(key: "port", kind: .number, required: true)

        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["port": .string(" 42 ")], fields: [optionalPort]),
            ["port": .number(42)]
        )
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["port": .number(2.5)], fields: [optionalPort]),
            ["port": .number(2.5)]
        )
        // Empty optional numbers drop out of the payload entirely.
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["port": .string("")], fields: [optionalPort]),
            [:]
        )
        XCTAssertEqual(GaryxBotConfigValues.normalized(config: [:], fields: [optionalPort]), [:])
        // Empty or unparseable required numbers persist as 0.
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["port": .string("")], fields: [requiredPort]),
            ["port": .number(0)]
        )
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["port": .string("junk")], fields: [requiredPort]),
            ["port": .number(0)]
        )
    }

    func testNormalizedEnumValuedNumberFieldSavesAsNumber() {
        // Reachable combination: schema type number/integer with enum. The
        // picker writes .string(option) into the editor draft; save-time
        // normalization coerces it to a number (editor state stays string,
        // save normalizes as number).
        var port = field(key: "port", kind: .number, required: true)
        port.enumValues = ["8080", "9090"]
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["port": .string("8080")], fields: [port]),
            ["port": .number(8080)]
        )
    }

    func testNormalizedString() {
        let optionalName = field(key: "name", kind: .string)
        let requiredName = field(key: "name", kind: .string, required: true)

        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["name": .string("  x  ")], fields: [optionalName]),
            ["name": .string("x")]
        )
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["name": .string("   ")], fields: [optionalName]),
            [:]
        )
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["name": .string("")], fields: [requiredName]),
            ["name": .string("")]
        )
        // Non-string stored values re-render through the editor string form.
        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: ["name": .number(5)], fields: [optionalName]),
            ["name": .string("5")]
        )
    }

    func testNormalizedPreservesUndeclaredKeysAndUsesDefaults() {
        let mode = field(key: "mode", kind: .string, defaultValue: .string("polling"))
        let interval = field(key: "interval", kind: .number, defaultValue: .number(30))
        let config: [String: GaryxJSONValue] = ["extra": .string("keep")]

        XCTAssertEqual(
            GaryxBotConfigValues.normalized(config: config, fields: [mode, interval]),
            [
                "extra": .string("keep"),
                "mode": .string("polling"),
                "interval": .number(30),
            ]
        )
    }

    // MARK: Default account id

    func testDefaultAccountIdSlugging() {
        XCTAssertEqual(
            GaryxBotAccountIdDefaults.defaultAccountId(channel: "Telegram", existingAccountIds: []),
            "telegram-main"
        )
        XCTAssertEqual(
            GaryxBotAccountIdDefaults.defaultAccountId(channel: " My_Channel! ", existingAccountIds: []),
            "my-channel-main"
        )
        XCTAssertEqual(
            GaryxBotAccountIdDefaults.defaultAccountId(channel: "discord2", existingAccountIds: []),
            "discord2-main"
        )
        XCTAssertEqual(
            GaryxBotAccountIdDefaults.defaultAccountId(channel: "!!!", existingAccountIds: []),
            "bot-main"
        )
    }

    func testDefaultAccountIdUniquification() {
        XCTAssertEqual(
            GaryxBotAccountIdDefaults.defaultAccountId(
                channel: "telegram",
                existingAccountIds: ["telegram-main"]
            ),
            "telegram-main-2"
        )
        XCTAssertEqual(
            GaryxBotAccountIdDefaults.defaultAccountId(
                channel: "telegram",
                existingAccountIds: ["telegram-main", "telegram-main-2", "telegram-main-3"]
            ),
            "telegram-main-4"
        )

        var exhausted: Set<String> = ["telegram-main"]
        for index in 2...99 {
            exhausted.insert("telegram-main-\(index)")
        }
        XCTAssertEqual(
            GaryxBotAccountIdDefaults.defaultAccountId(channel: "telegram", existingAccountIds: exhausted),
            "telegram-main-new"
        )
    }
}
