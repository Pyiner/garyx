import SwiftUI

/// The Capsule product glyph — a small vial holding a faceted gem ("a container
/// of something precious"), replacing the old `capsule.fill` horizontal pill.
/// Authored as vector paths in a 24×24 box so it stays crisp at every size and
/// tints like an SF Symbol: `.monochrome` inherits the ambient foreground style
/// (nav / placeholder), `.accent` fills with the capsule teal→indigo→fuchsia
/// gradient (empty-state / hero).
struct GaryxCapsuleGlyph: View {
    enum Style { case monochrome, accent }
    var style: Style = .monochrome

    var body: some View {
        GeometryReader { geo in
            let s = min(geo.size.width, geo.size.height) / 24
            let stroke = StrokeStyle(lineWidth: 1.6 * s, lineCap: .round, lineJoin: .round)
            ZStack {
                if style == .accent {
                    Self.vessel(s).stroke(Self.accentGradient, style: stroke)
                    Self.lip(s).stroke(Self.accentGradient, style: stroke)
                    Self.gem(s).fill(Self.accentGradient)
                } else {
                    Self.vessel(s).stroke(style: stroke)
                    Self.lip(s).stroke(style: stroke)
                    Self.gem(s).fill()
                }
            }
            .frame(width: geo.size.width, height: geo.size.height)
        }
        .aspectRatio(1, contentMode: .fit)
        .accessibilityHidden(true)
    }

    static let accentGradient = LinearGradient(
        colors: [
            Color(red: 0.369, green: 0.918, blue: 0.831), // #5eead4
            Color(red: 0.506, green: 0.549, blue: 0.973), // #818cf8
            Color(red: 0.941, green: 0.671, blue: 0.988)  // #f0abfc
        ],
        startPoint: .topLeading,
        endPoint: .bottomTrailing
    )

    /// Vial body: neck → shoulders → rounded base, open at the top (capped by the
    /// `lip`). Rounded shoulders/base use quadratic curves (no SVG arcs).
    private static func vessel(_ s: CGFloat) -> Path {
        func pt(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: x * s, y: y * s) }
        var path = Path()
        path.move(to: pt(10, 3))
        path.addLine(to: pt(10, 6.4))
        path.addQuadCurve(to: pt(7.5, 12), control: pt(7.7, 8.7))
        path.addLine(to: pt(7.5, 17.5))
        path.addQuadCurve(to: pt(10.5, 20.5), control: pt(7.5, 20.5))
        path.addLine(to: pt(13.5, 20.5))
        path.addQuadCurve(to: pt(16.5, 17.5), control: pt(16.5, 20.5))
        path.addLine(to: pt(16.5, 12))
        path.addQuadCurve(to: pt(14, 6.4), control: pt(16.3, 8.7))
        path.addLine(to: pt(14, 3))
        return path
    }

    /// The vial lip / cap line across the mouth.
    private static func lip(_ s: CGFloat) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: 9 * s, y: 3 * s))
        path.addLine(to: CGPoint(x: 15 * s, y: 3 * s))
        return path
    }

    /// The gem suspended inside the vial.
    private static func gem(_ s: CGFloat) -> Path {
        func pt(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: x * s, y: y * s) }
        var path = Path()
        path.move(to: pt(12, 11))
        path.addLine(to: pt(14, 13))
        path.addLine(to: pt(12, 15))
        path.addLine(to: pt(10, 13))
        path.closeSubpath()
        return path
    }
}

/// Renders a navigation panel's icon, mapping the Capsules panel to the
/// `GaryxCapsuleGlyph` and every other panel to its SF Symbol. One shared
/// helper so the gem glyph appears consistently wherever panel icons render,
/// instead of scattering the special case across views.
struct GaryxPanelIconView: View {
    let systemName: String
    var size: CGFloat = 19
    var weight: Font.Weight = .regular

    var body: some View {
        if systemName == GaryxMobilePanel.capsules.iconName {
            GaryxCapsuleGlyph().frame(width: size * 1.16, height: size * 1.16)
        } else {
            Image(systemName: systemName)
                .font(GaryxFont.fixedSystem(size: size, weight: weight))
        }
    }
}
