public enum GaryxTypographyTextStyle: String, CaseIterable, Sendable {
    case largeTitle
    case title
    case title2
    case title3
    case headline
    case body
    case callout
    case subheadline
    case footnote
    case caption
    case caption2
}

public enum GaryxTypographyTrackingIntent: String, Sendable {
    case tightened
    case neutral
    case opened
}

public enum GaryxTypographyLeadingIntent: String, Sendable {
    case tight
    case standard
    case relaxed
}

public enum GaryxTypographyScalePolicy: Equatable, Sendable {
    case unbounded
}

public struct GaryxTypographyRoleSpecification: Equatable, Sendable {
    public let textStyle: GaryxTypographyTextStyle
    public let basePointSize: Double
    public let tracking: GaryxTypographyTrackingIntent
    public let leading: GaryxTypographyLeadingIntent
    public let scalePolicy: GaryxTypographyScalePolicy

    public init(
        textStyle: GaryxTypographyTextStyle,
        basePointSize: Double,
        tracking: GaryxTypographyTrackingIntent,
        leading: GaryxTypographyLeadingIntent,
        scalePolicy: GaryxTypographyScalePolicy
    ) {
        self.textStyle = textStyle
        self.basePointSize = basePointSize
        self.tracking = tracking
        self.leading = leading
        self.scalePolicy = scalePolicy
    }
}

/// Platform-neutral typography roles used by the iOS adapter. Every role in
/// this table is reading content and therefore follows the user's full Dynamic
/// Type range. The tracking and leading intents are fulfilled by the system
/// text style's optical metrics; clients must not replace them with one global
/// kerning or line-spacing value.
public enum GaryxTypographyRole: String, CaseIterable, Sendable {
    case largeTitle
    case title
    case title2
    case title3
    case headline
    case body
    case callout
    case subheadline
    case footnote
    case caption
    case caption2

    public var specification: GaryxTypographyRoleSpecification {
        switch self {
        case .largeTitle:
            specification(size: 34, tracking: .tightened, leading: .tight)
        case .title:
            specification(size: 28, tracking: .tightened, leading: .tight)
        case .title2:
            specification(size: 22, tracking: .tightened, leading: .tight)
        case .title3:
            specification(size: 20, tracking: .tightened, leading: .tight)
        case .headline:
            specification(size: 17, tracking: .neutral, leading: .standard)
        case .body:
            specification(size: 17, tracking: .neutral, leading: .relaxed)
        case .callout:
            specification(size: 16, tracking: .neutral, leading: .relaxed)
        case .subheadline:
            specification(size: 15, tracking: .neutral, leading: .relaxed)
        case .footnote:
            specification(size: 13, tracking: .opened, leading: .relaxed)
        case .caption:
            specification(size: 12, tracking: .opened, leading: .relaxed)
        case .caption2:
            specification(size: 11, tracking: .opened, leading: .relaxed)
        }
    }

    private func specification(
        size: Double,
        tracking: GaryxTypographyTrackingIntent,
        leading: GaryxTypographyLeadingIntent
    ) -> GaryxTypographyRoleSpecification {
        GaryxTypographyRoleSpecification(
            textStyle: textStyle,
            basePointSize: size,
            tracking: tracking,
            leading: leading,
            scalePolicy: .unbounded
        )
    }

    private var textStyle: GaryxTypographyTextStyle {
        switch self {
        case .largeTitle: .largeTitle
        case .title: .title
        case .title2: .title2
        case .title3: .title3
        case .headline: .headline
        case .body: .body
        case .callout: .callout
        case .subheadline: .subheadline
        case .footnote: .footnote
        case .caption: .caption
        case .caption2: .caption2
        }
    }
}

public enum GaryxTypographyContentSizeCategory: String, Sendable {
    case extraExtraLarge
}

/// Explicit exceptions for controls whose text participates in fixed chrome
/// geometry. Reading content never uses these boundaries. Each capped surface
/// is still allowed to grow through XXL before its fixed hit target, morph
/// anchor, or compact badge geometry takes precedence.
public enum GaryxTypographyScaleBoundary: String, CaseIterable, Sendable {
    case readingSurface
    case navigationChrome
    case composerAccessoryChrome
    case segmentedControlChrome
    case compactBadgeChrome
    case compactDataVisualizationChrome
    case widgetFamilyChrome

    public var maximumCategory: GaryxTypographyContentSizeCategory? {
        switch self {
        case .readingSurface:
            nil
        case .navigationChrome,
             .composerAccessoryChrome,
             .segmentedControlChrome,
             .compactBadgeChrome,
             .compactDataVisualizationChrome,
             .widgetFamilyChrome:
            .extraExtraLarge
        }
    }

    public var rationale: String {
        switch self {
        case .readingSurface:
            "Reading surfaces follow the user's complete Dynamic Type range."
        case .navigationChrome:
            "Custom navigation titles share 44-point controls and morph anchors, so growth stops at XXL to preserve a stable single-line bar."
        case .composerAccessoryChrome:
            "Composer accessory labels live in the fixed attachment tray and chips, so growth stops at XXL while the editor itself remains unbounded."
        case .segmentedControlChrome:
            "Segment labels share a fixed system control track, so growth stops at XXL and the surrounding form content remains unbounded."
        case .compactBadgeChrome:
            "Compact status badges must remain inline with their row, so growth stops at XXL while the row's reading text remains unbounded."
        case .compactDataVisualizationChrome:
            "Labels embedded in a fixed gauge must stay inside its arc, so growth stops at XXL while surrounding usage explanations remain unbounded."
        case .widgetFamilyChrome:
            "WidgetKit snapshots have a fixed non-scrolling family canvas, so their relative typography stops at XXL to keep every static row reachable."
        }
    }
}
