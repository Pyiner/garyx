import Foundation

/// Pure Markdown-to-AttributedString rendering shared by mobile presentation
/// surfaces. Link detection happens here so SwiftUI views only map the
/// resulting attributes into `Text` and route link actions.
public enum GaryxMarkdownAttributedStringRenderer {
    private struct DetectedWebLink {
        let characterOffset: Int
        let characterCount: Int
        let url: URL
    }

    private static let webLinkDetector = try? NSDataDetector(
        types: NSTextCheckingResult.CheckingType.link.rawValue
    )

    public static func attributedString(
        from markdown: String,
        linkifyFilePaths: Bool = false
    ) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .full,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        let parsed = (try? AttributedString(markdown: markdown, options: options))
            ?? AttributedString(markdown)
        var result = annotatingWebLinks(in: parsed)
        if linkifyFilePaths {
            result = annotatingFilePaths(in: result)
        }
        return result
    }

    private static func annotatingWebLinks(in attributed: AttributedString) -> AttributedString {
        let plain = String(attributed.characters)
        let candidates = detectedWebLinks(in: plain)
        guard !candidates.isEmpty else { return attributed }

        var result = attributed
        for candidate in candidates {
            let candidateRange = attributedRange(
                offset: candidate.characterOffset,
                count: candidate.characterCount,
                in: result
            )
            guard !containsCodeBlock(in: result[candidateRange]) else { continue }

            let overlappingLinks = result.runs.compactMap { run -> (Range<AttributedString.Index>, URL)? in
                guard let link = run.link,
                      rangesOverlap(run.range, candidateRange) else {
                    return nil
                }
                return (run.range, link)
            }

            if overlappingLinks.isEmpty {
                result[candidateRange].link = candidate.url
                continue
            }

            if overlappingLinks.count == 1 {
                let existing = overlappingLinks[0]
                if existing.0 == candidateRange {
                    continue
                }
                if isRepairableSelfLink(
                    range: existing.0,
                    url: existing.1,
                    candidateRange: candidateRange,
                    in: result
                ) {
                    result[existing.0].link = nil
                    result[candidateRange].link = candidate.url
                }
            }
        }
        return result
    }

    private static func detectedWebLinks(in text: String) -> [DetectedWebLink] {
        guard containsSupportedSourcePrefix(text), let webLinkDetector else { return [] }
        let nsText = text as NSString
        let fullRange = NSRange(location: 0, length: nsText.length)
        return webLinkDetector.matches(in: text, range: fullRange).compactMap { match in
            guard let url = match.url,
                  let range = Range(match.range, in: text) else {
                return nil
            }
            let source = String(text[range])
            guard hasSupportedSourcePrefix(source) else { return nil }
            return DetectedWebLink(
                characterOffset: text.distance(from: text.startIndex, to: range.lowerBound),
                characterCount: text.distance(from: range.lowerBound, to: range.upperBound),
                url: url
            )
        }
    }

    private static func hasSupportedSourcePrefix(_ source: String) -> Bool {
        let lowercase = source.lowercased()
        return lowercase.hasPrefix("http://")
            || lowercase.hasPrefix("https://")
            || lowercase.hasPrefix("www.")
    }

    private static func containsSupportedSourcePrefix(_ source: String) -> Bool {
        source.range(of: "http://", options: [.caseInsensitive, .literal]) != nil
            || source.range(of: "https://", options: [.caseInsensitive, .literal]) != nil
            || source.range(of: "www.", options: [.caseInsensitive, .literal]) != nil
    }

    /// Foundation can create an over-wide self-link for a bare URL followed
    /// immediately by CJK punctuation and prose. Repair only the provable
    /// self-link shape; labeled Markdown links keep their authored target.
    private static func isRepairableSelfLink(
        range: Range<AttributedString.Index>,
        url: URL,
        candidateRange: Range<AttributedString.Index>,
        in attributed: AttributedString
    ) -> Bool {
        guard range.lowerBound == candidateRange.lowerBound,
              candidateRange.upperBound < range.upperBound,
              ["http", "https"].contains(url.scheme?.lowercased() ?? "") else {
            return false
        }
        let visibleText = String(attributed[range].characters)
        guard hasSupportedSourcePrefix(visibleText),
              let canonical = canonicalSelfLinkURL(for: visibleText) else {
            return false
        }
        return canonical.absoluteString == url.absoluteString
    }

    private static func canonicalSelfLinkURL(for visibleText: String) -> URL? {
        let lowercase = visibleText.lowercased()
        let source = lowercase.hasPrefix("www.")
            ? "http://\(visibleText)"
            : visibleText
        return URL(string: source, encodingInvalidCharacters: true)
    }

    private static func annotatingFilePaths(in attributed: AttributedString) -> AttributedString {
        let plain = String(attributed.characters)
        let detected = GaryxFilePathLinkDetector.detect(in: plain)
        guard !detected.isEmpty else { return attributed }

        var result = attributed
        for link in detected {
            guard let url = GaryxFilePathLinkDetector.linkURL(forTarget: link.target) else {
                continue
            }
            let range = attributedRange(
                offset: link.characterOffset,
                count: link.characterCount,
                in: result
            )
            guard result[range].runs.allSatisfy({ run in
                run.link == nil && !isCodeBlock(run.presentationIntent)
            }) else {
                continue
            }
            result[range].link = url
        }
        return result
    }

    private static func attributedRange(
        offset: Int,
        count: Int,
        in attributed: AttributedString
    ) -> Range<AttributedString.Index> {
        let lower = attributed.index(attributed.startIndex, offsetByCharacters: offset)
        let upper = attributed.index(lower, offsetByCharacters: count)
        return lower..<upper
    }

    private static func containsCodeBlock(in attributed: AttributedSubstring) -> Bool {
        attributed.runs.contains { isCodeBlock($0.presentationIntent) }
    }

    private static func isCodeBlock(_ intent: PresentationIntent?) -> Bool {
        intent?.components.contains { component in
            if case .codeBlock = component.kind {
                return true
            }
            return false
        } == true
    }

    private static func rangesOverlap(
        _ lhs: Range<AttributedString.Index>,
        _ rhs: Range<AttributedString.Index>
    ) -> Bool {
        lhs.lowerBound < rhs.upperBound && rhs.lowerBound < lhs.upperBound
    }
}
