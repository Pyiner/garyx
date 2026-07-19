import SwiftUI
import UIKit

/// Free-form text selection for a transcript message. SwiftUI's
/// `textSelection` cannot select a range inside transcript bubbles (and
/// fights the bubble long-press menu), so the menu's "Select Text" opens
/// this sheet with a real `UITextView`: drag handles, partial ranges, and
/// the system Copy/Look Up/Share menu all work.
struct GaryxMessageTextSelectionSheet: View {
    let text: String

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            GaryxSelectableTextView(text: text)
                .ignoresSafeArea(edges: .bottom)
                .navigationTitle("Select Text")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Done") {
                            dismiss()
                        }
                        .font(GaryxFont.subheadline(weight: .semibold))
                    }
                }
        }
    }
}

private struct GaryxSelectableTextView: UIViewRepresentable {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let text: String

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.isEditable = false
        view.isSelectable = true
        view.alwaysBounceVertical = true
        view.dataDetectorTypes = [.link]
        view.backgroundColor = .clear
        view.adjustsFontForContentSizeCategory = true
        configure(view)
        return view
    }

    func updateUIView(_ uiView: UITextView, context: Context) {
        configure(uiView)
    }

    private func configure(_ view: UITextView) {
        // Reading the environment makes SwiftUI update this representable when
        // Dynamic Type changes while the selection sheet is open.
        _ = dynamicTypeSize
        let font = GaryxFont.uiFont(.body, compatibleWith: view.traitCollection)
        let next = Self.attributedText(for: text, font: font)
        if view.attributedText != next {
            view.attributedText = next
        }
        let inset = UIFontMetrics(forTextStyle: .body).scaledValue(
            for: 14,
            compatibleWith: view.traitCollection
        )
        view.textContainerInset = UIEdgeInsets(
            top: inset,
            left: inset,
            bottom: inset * 2,
            right: inset
        )
    }

    private static func attributedText(for text: String, font: UIFont) -> NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = font.lineHeight * (5.0 / 22.0)
        paragraph.paragraphSpacing = font.lineHeight * (6.0 / 22.0)
        return NSAttributedString(
            string: text,
            attributes: [
                .font: font,
                .foregroundColor: UIColor.label,
                .paragraphStyle: paragraph,
            ]
        )
    }
}
