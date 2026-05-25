import Foundation
import SwiftUI
import UIKit

struct GaryxFormSheet<Content: View>: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    let canSave: Bool?
    let onCancel: (() -> Void)?
    let onSave: (() -> Void)?
    let onDone: (() -> Void)?
    let content: Content

    init(title: String, onDone: (() -> Void)? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.canSave = nil
        self.onCancel = nil
        self.onSave = nil
        self.onDone = onDone
        self.content = content()
    }

    init(
        title: String,
        canSave: Bool,
        onCancel: (() -> Void)? = nil,
        onSave: @escaping () -> Void,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.canSave = canSave
        self.onCancel = onCancel
        self.onSave = onSave
        self.onDone = nil
        self.content = content()
    }

    var body: some View {
        ZStack(alignment: .top) {
            GaryxFormPalette.pageBackground
                .ignoresSafeArea()

            ScrollView {
                content
                    .padding(.horizontal, 18)
                    .padding(.top, 92)
                    .padding(.bottom, 28)
                    .frame(maxWidth: 560, alignment: .leading)
                    .frame(maxWidth: .infinity)
            }

            ZStack {
                HStack {
                    Button(action: cancel) {
                        GaryxToolbarIcon(systemName: "xmark")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Cancel")

                    Spacer(minLength: 0)

                    if let onSave {
                        Button(action: onSave) {
                            GaryxToolbarIcon(systemName: "checkmark")
                                .opacity(canSave == false ? 0.42 : 1)
                        }
                        .buttonStyle(.plain)
                        .disabled(canSave == false)
                        .accessibilityLabel("Save")
                    }
                }

                Text(title)
                    .font(GaryxFont.title3(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 18)
            .padding(.top, 10)
        }
    }

    private func cancel() {
        if let onCancel {
            onCancel()
        } else if let onDone {
            onDone()
        } else {
            dismiss()
        }
    }
}

struct GaryxFormGroupedSection<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
                .padding(.horizontal, 14)

            VStack(alignment: .leading, spacing: 0) {
                content
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(GaryxFormPalette.cardBackground, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        }
    }
}

struct GaryxFormRow<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        HStack(spacing: 12) {
            Text(title)
                .font(GaryxFont.body())
                .foregroundStyle(.primary)
            Spacer(minLength: 8)
            content
                .font(GaryxFont.body())
                .foregroundStyle(.primary)
                .multilineTextAlignment(.trailing)
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 52)
    }
}

struct GaryxFormReadOnlyRow: View {
    let title: String
    let value: String

    var body: some View {
        GaryxFormRow(title: title) {
            Text(value)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}

struct GaryxFormMenuValueLabel: View {
    let value: String

    var body: some View {
        HStack(spacing: 6) {
            Text(value)
                .font(GaryxFont.body(weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Image(systemName: "chevron.up.chevron.down")
                .font(GaryxFont.system(size: 11, weight: .semibold))
                .foregroundStyle(.tertiary)
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

struct GaryxFormSelectionRow: View {
    let title: String
    let value: String
    let placeholder: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Text(title)
                    .font(GaryxFont.body())
                    .foregroundStyle(.primary)
                Spacer(minLength: 8)
                Text(displayValue)
                    .font(GaryxFont.body())
                    .foregroundStyle(isPlaceholder ? .secondary : .primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var displayValue: String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? placeholder : value
    }

    private var isPlaceholder: Bool {
        value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

struct GaryxFormErrorText: View {
    let text: String

    var body: some View {
        Text(text)
            .font(GaryxFont.caption(weight: .medium))
            .foregroundStyle(GaryxTheme.danger)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 14)
    }
}

enum GaryxFormPalette {
    static let pageBackground = Color(.systemGroupedBackground).opacity(0.72)
    static let cardBackground = Color(.systemBackground)
}

extension View {
    func garyxFormTextField(minHeight: CGFloat = 52, horizontalPadding: CGFloat = 16) -> some View {
        self
            .font(GaryxFont.body())
            .foregroundStyle(.primary)
            .padding(.horizontal, horizontalPadding)
            .frame(minHeight: minHeight, alignment: .leading)
    }

    func garyxFormTextArea(minHeight: CGFloat = 132) -> some View {
        self
            .font(GaryxFont.body())
            .foregroundStyle(.primary)
            .padding(16)
            .frame(minHeight: minHeight, alignment: .topLeading)
    }
}
