import Foundation

public struct GaryxSkillsPage: Decodable, Equatable, Sendable {
    public var skills: [GaryxSkillSummary]
}


public struct GaryxSkillSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var description: String
    public var installed: Bool
    public var enabled: Bool
    public var sourcePath: String

    public init(
        id: String,
        name: String,
        description: String = "",
        installed: Bool = true,
        enabled: Bool = true,
        sourcePath: String = ""
    ) {
        self.id = id
        self.name = name
        self.description = description
        self.installed = installed
        self.enabled = enabled
        self.sourcePath = sourcePath
    }

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case description
        case installed
        case enabled
        case sourcePath = "source_path"
        case sourcePathCamel = "sourcePath"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.id) ?? ""
        name = try container.garyxDecodeFirstString(.name) ?? id
        description = try container.garyxDecodeFirstString(.description) ?? ""
        installed = try container.garyxDecodeFirstBool(.installed) ?? false
        enabled = try container.garyxDecodeFirstBool(.enabled) ?? true
        sourcePath = try container.garyxDecodeFirstString(.sourcePath, .sourcePathCamel) ?? ""
    }
}


public struct GaryxCreateSkillRequest: Encodable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var description: String
    public var body: String

    public init(id: String, name: String, description: String, body: String) {
        self.id = id
        self.name = name
        self.description = description
        self.body = body
    }
}


public struct GaryxUpdateSkillRequest: Encodable, Equatable, Sendable {
    public var name: String
    public var description: String

    public init(name: String, description: String) {
        self.name = name
        self.description = description
    }
}


public struct GaryxSkillEntryNode: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { path }
    public var path: String
    public var name: String
    public var entryType: String
    public var children: [GaryxSkillEntryNode]

    enum CodingKeys: String, CodingKey {
        case path
        case name
        case entryType
        case entryTypeSnake = "entry_type"
        case children
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        path = try container.garyxDecodeFirstString(.path) ?? ""
        name = try container.garyxDecodeFirstString(.name) ?? path.garyxLastPathComponent
        entryType = try container.garyxDecodeFirstString(.entryType, .entryTypeSnake) ?? "file"
        children = try container.decodeIfPresent([GaryxSkillEntryNode].self, forKey: .children) ?? []
    }
}


public struct GaryxSkillEditorState: Decodable, Equatable, Sendable {
    public var skill: GaryxSkillSummary
    public var entries: [GaryxSkillEntryNode]
}


public struct GaryxSkillFileDocument: Decodable, Equatable, Sendable {
    public var skill: GaryxSkillSummary
    public var path: String
    public var content: String
    public var mediaType: String
    public var previewKind: String
    public var dataBase64: String?
    public var editable: Bool

    enum CodingKeys: String, CodingKey {
        case skill
        case path
        case content
        case mediaType
        case mediaTypeSnake = "media_type"
        case previewKind
        case previewKindSnake = "preview_kind"
        case dataBase64
        case dataBase64Snake = "data_base64"
        case editable
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        skill = try container.decode(GaryxSkillSummary.self, forKey: .skill)
        path = try container.garyxDecodeFirstString(.path) ?? ""
        content = try container.garyxDecodeFirstString(.content) ?? ""
        mediaType = try container.garyxDecodeFirstString(.mediaType, .mediaTypeSnake) ?? "text/plain"
        previewKind = try container.garyxDecodeFirstString(.previewKind, .previewKindSnake) ?? "text"
        dataBase64 = try container.garyxDecodeFirstString(.dataBase64, .dataBase64Snake)
        editable = try container.decodeIfPresent(Bool.self, forKey: .editable) ?? false
    }
}


public struct GaryxSkillFileWriteRequest: Encodable, Equatable, Sendable {
    public var path: String
    public var content: String

    public init(path: String, content: String) {
        self.path = path
        self.content = content
    }
}


public struct GaryxSkillEntryCreateRequest: Encodable, Equatable, Sendable {
    public var path: String
    public var entryType: String

    public init(path: String, entryType: String) {
        self.path = path
        self.entryType = entryType
    }
}
