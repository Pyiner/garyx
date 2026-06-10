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
    let text: String

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.isEditable = false
        view.isSelectable = true
        view.alwaysBounceVertical = true
        view.dataDetectorTypes = [.link]
        view.backgroundColor = .clear
        view.textContainerInset = UIEdgeInsets(top: 14, left: 14, bottom: 28, right: 14)
        view.adjustsFontForContentSizeCategory = true
        view.attributedText = Self.attributedText(for: text)
        return view
    }

    func updateUIView(_ uiView: UITextView, context: Context) {
        let next = Self.attributedText(for: text)
        if uiView.attributedText != next {
            uiView.attributedText = next
        }
    }

    private static func attributedText(for text: String) -> NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = 5
        paragraph.paragraphSpacing = 6
        return NSAttributedString(
            string: text,
            attributes: [
                .font: UIFont.systemFont(ofSize: 17),
                .foregroundColor: UIColor.label,
                .paragraphStyle: paragraph,
            ]
        )
    }
}
