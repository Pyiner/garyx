import CoreGraphics

public struct GaryxAnchoredFullscreenMorphLayout: Equatable, Sendable {
    public let frame: CGRect
    public let cornerRadius: CGFloat
    public let contentOpacity: Double
    public let scrimOpacity: Double

    public init(
        frame: CGRect,
        cornerRadius: CGFloat,
        contentOpacity: Double,
        scrimOpacity: Double
    ) {
        self.frame = frame
        self.cornerRadius = cornerRadius
        self.contentOpacity = contentOpacity
        self.scrimOpacity = scrimOpacity
    }
}

/// Headless geometry for a full-canvas surface that grows out of a source
/// anchor. Opening and closing both sample this one progress function, so the
/// return path is the exact reverse of the arrival path.
public enum GaryxAnchoredFullscreenMorphGeometry {
    public static func layout(
        progress rawProgress: CGFloat,
        sourceRect rawSourceRect: CGRect,
        containerSize rawContainerSize: CGSize,
        sourceCornerRadius: CGFloat = 12,
        destinationCornerRadius: CGFloat = 0,
        maximumScrimOpacity: Double = 0.12
    ) -> GaryxAnchoredFullscreenMorphLayout {
        let progress = min(1, max(0, rawProgress))
        let containerSize = CGSize(
            width: max(0, rawContainerSize.width),
            height: max(0, rawContainerSize.height)
        )
        let destinationRect = CGRect(origin: .zero, size: containerSize)
        let sourceRect = normalizedSourceRect(
            rawSourceRect,
            destinationRect: destinationRect
        )

        return GaryxAnchoredFullscreenMorphLayout(
            frame: CGRect(
                x: interpolate(sourceRect.minX, destinationRect.minX, progress: progress),
                y: interpolate(sourceRect.minY, destinationRect.minY, progress: progress),
                width: interpolate(sourceRect.width, destinationRect.width, progress: progress),
                height: interpolate(sourceRect.height, destinationRect.height, progress: progress)
            ),
            cornerRadius: interpolate(
                max(0, sourceCornerRadius),
                max(0, destinationCornerRadius),
                progress: progress
            ),
            contentOpacity: Double(progress),
            scrimOpacity: max(0, maximumScrimOpacity) * Double(progress)
        )
    }

    /// Deep links can present a Capsule before its lazy grid cell exists. A
    /// stable centered 16:10 origin keeps that non-tap path deterministic;
    /// ordinary gallery taps always supply their real thumbnail anchor.
    public static func fallbackSourceRect(containerSize rawContainerSize: CGSize) -> CGRect {
        let containerSize = CGSize(
            width: max(0, rawContainerSize.width),
            height: max(0, rawContainerSize.height)
        )
        let width = min(180, containerSize.width * 0.46)
        let height = width * 10 / 16
        return CGRect(
            x: (containerSize.width - width) / 2,
            y: (containerSize.height - height) / 2,
            width: width,
            height: height
        )
    }

    private static func normalizedSourceRect(
        _ rawSourceRect: CGRect,
        destinationRect: CGRect
    ) -> CGRect {
        guard destinationRect.width > 0, destinationRect.height > 0 else {
            return .zero
        }
        let sourceRect = rawSourceRect.standardized
        guard sourceRect.width > 0, sourceRect.height > 0 else {
            return fallbackSourceRect(containerSize: destinationRect.size)
        }
        let clipped = sourceRect.intersection(destinationRect)
        guard !clipped.isNull, clipped.width > 0, clipped.height > 0 else {
            return fallbackSourceRect(containerSize: destinationRect.size)
        }
        return clipped
    }

    private static func interpolate(
        _ source: CGFloat,
        _ destination: CGFloat,
        progress: CGFloat
    ) -> CGFloat {
        source + (destination - source) * progress
    }
}
