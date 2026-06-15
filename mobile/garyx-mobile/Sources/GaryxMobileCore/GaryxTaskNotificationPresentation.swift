import Foundation

public struct GaryxTaskNotification: Equatable, Sendable {
    public var event: String
    public var status: String
    public var taskId: String
    public var title: String
    public var finalMessage: String

    public init(
        event: String,
        status: String,
        taskId: String,
        title: String,
        finalMessage: String
    ) {
        self.event = event
        self.status = status
        self.taskId = taskId
        self.title = title
        self.finalMessage = finalMessage
    }
}

public enum GaryxTaskNotificationPresentation {
    public static func parse(_ text: String) -> GaryxTaskNotification? {
        guard let envelope = stripOuterEnvelope(from: text) else { return nil }

        let lines = envelope.body.components(separatedBy: CharacterSet.newlines)
        let firstLine = lines.first { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            ?? ""
        let titleParts = readyForReviewTitleParts(from: firstLine)
        let taskId = envelope.attributes["task_id"] ?? titleParts?.taskId ?? ""
        let title = titleParts?.title.isEmpty == false
            ? titleParts?.title ?? ""
            : (taskId.isEmpty ? "Task ready for review" : taskId)
        let finalMessage = finalMessage(from: envelope.body, firstLine: firstLine)

        return GaryxTaskNotification(
            event: envelope.attributes["event"] ?? "ready_for_review",
            status: envelope.attributes["status"] ?? "in_review",
            taskId: taskId,
            title: title,
            finalMessage: finalMessage
        )
    }

    public static func statusLabel(for status: String) -> String {
        if status == "in_review" {
            return "In review"
        }
        return status
            .split { $0 == "_" || $0 == "-" }
            .map { part in
                part.prefix(1).uppercased() + String(part.dropFirst())
            }
            .joined(separator: " ")
    }

    private static func stripOuterEnvelope(
        from text: String
    ) -> (attributes: [String: String], body: String)? {
        guard let openMatch = firstMatch(
            #"^\s*<garyx_task_notification\b([^>]*)>\s*"#,
            in: text
        ) else {
            return nil
        }
        let closeTag = "</garyx_task_notification>"
        guard let closeRange = text.range(of: closeTag, options: .backwards) else {
            return nil
        }

        guard let openRange = Range(openMatch.range, in: text) else { return nil }
        let openEnd = openRange.upperBound
        guard openEnd <= closeRange.lowerBound else { return nil }

        let rawAttributes = substring(from: text, range: openMatch.range(at: 1)) ?? ""
        let body = text[openEnd..<closeRange.lowerBound]
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return (parseAttributes(rawAttributes), body)
    }

    private static func readyForReviewTitleParts(
        from line: String
    ) -> (taskId: String, title: String)? {
        guard let match = firstMatch(#"^Task\s+(.+?)\s+is ready for review:\s*(.*)$"#, in: line),
              let taskId = substring(from: line, range: match.range(at: 1))?.trimmingCharacters(in: .whitespacesAndNewlines),
              let title = substring(from: line, range: match.range(at: 2))?.trimmingCharacters(in: .whitespacesAndNewlines) else {
            return nil
        }
        return (taskId, title)
    }

    private static func finalMessage(from body: String, firstLine: String) -> String {
        let messageEnd = firstMatch(#"\r?\nView details:"#, in: body).map { match in
            Range(match.range, in: body)?.lowerBound ?? body.endIndex
        } ?? body.endIndex
        let bodyAfterFirstLine: Substring
        if !firstLine.isEmpty, body.hasPrefix(firstLine) {
            let start = body.index(body.startIndex, offsetBy: firstLine.count)
            bodyAfterFirstLine = body[start..<messageEnd]
        } else {
            bodyAfterFirstLine = body[..<messageEnd]
        }

        let final = bodyAfterFirstLine.trimmingCharacters(in: .whitespacesAndNewlines)
        if !final.isEmpty {
            return final
        }

        let fallback = body
            .dropFirst(firstLine.count)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return fallback.isEmpty ? "The task is ready for review." : fallback
    }

    private static func parseAttributes(_ raw: String) -> [String: String] {
        matches(#"([\w:-]+)\s*=\s*"([^"]*)""#, in: raw).reduce(into: [:]) { result, match in
            guard let key = substring(from: raw, range: match.range(at: 1)),
                  let value = substring(from: raw, range: match.range(at: 2)) else {
                return
            }
            result[key] = decodeXMLAttribute(value)
        }
    }

    private static func decodeXMLAttribute(_ value: String) -> String {
        value
            .replacingOccurrences(of: "&quot;", with: "\"")
            .replacingOccurrences(of: "&apos;", with: "'")
            .replacingOccurrences(of: "&lt;", with: "<")
            .replacingOccurrences(of: "&gt;", with: ">")
            .replacingOccurrences(of: "&amp;", with: "&")
    }

    private static func firstMatch(
        _ pattern: String,
        in text: String
    ) -> NSTextCheckingResult? {
        matches(pattern, in: text).first
    }

    private static func matches(
        _ pattern: String,
        in text: String
    ) -> [NSTextCheckingResult] {
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return [] }
        return regex.matches(
            in: text,
            range: NSRange(text.startIndex..<text.endIndex, in: text)
        )
    }

    private static func substring(from text: String, range: NSRange) -> String? {
        guard range.location != NSNotFound,
              let swiftRange = Range(range, in: text) else {
            return nil
        }
        return String(text[swiftRange])
    }
}
