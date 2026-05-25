import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxDreamsView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxPanelScaffold(
            title: "Dreams",
            subtitle: subtitle,
            onRefresh: { await model.refreshDreams() }
        ) {
            VStack(alignment: .leading, spacing: 14) {
                GaryxSectionBlock(title: "Settings") {
                    GaryxCompactListGroup {
                        GaryxDreamsAutoScanRow()
                    }
                }

                if model.dreams.isEmpty, model.isRemoteStatePending || model.isScanningDreams {
                    GaryxLoadingPanelView(title: "Loading dreams...")
                } else if model.dreams.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "moon.stars",
                        title: "No dreams yet.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Last 24 Hours") {
                        GaryxCompactListGroup {
                            GaryxDreamTopicList(dreams: model.dreams)
                        }
                    }
                }
            }
        } actions: {
            Button {
                Task { await model.scanDreams() }
            } label: {
                GaryxToolbarIcon(systemName: model.isScanningDreams ? "hourglass" : "sparkles")
            }
            .buttonStyle(.plain)
            .disabled(model.isScanningDreams)
            .accessibilityLabel("Scan dreams")
        }
    }

    private var subtitle: String {
        if let scan = model.latestDreamScan {
            let status = scan.status.trimmingCharacters(in: .whitespacesAndNewlines)
            let updated = garyxFormattedTaskTimestamp(scan.createdAt)
            let statusText = status.isEmpty ? "scan" : status
            return updated.isEmpty
                ? "\(model.dreams.count) topics / \(statusText)"
                : "\(model.dreams.count) topics / \(statusText) \(updated)"
        }
        return "\(model.dreams.count) topics"
    }
}

struct GaryxDreamsAutoScanRow: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "clock.arrow.2.circlepath")
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 24, height: 24)

            VStack(alignment: .leading, spacing: 3) {
                Text("Dreams")
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                Text("Shows Dreams in the app and runs periodic scans when recent user messages exist.")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }

            Spacer(minLength: 0)

            Toggle(
                "Dreams",
                isOn: Binding(
                    get: { model.dreamsAutoScanEnabled },
                    set: { nextValue in
                        Task { await model.setDreamsAutoScanEnabled(nextValue) }
                    }
                )
            )
            .labelsHidden()
            .toggleStyle(.switch)
            .disabled(model.isSavingDreamsSettings)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }
}

struct GaryxDreamTopicList: View {
    let dreams: [GaryxDreamTopic]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(dreams.enumerated()), id: \.element.id) { index, dream in
                GaryxDreamTopicRow(dream: dream)
                if index < dreams.count - 1 {
                    GaryxCompactRowDivider()
                }
            }
        }
    }
}

struct GaryxDreamTopicRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let dream: GaryxDreamTopic

    var body: some View {
        Button {
            if let firstSpan = dream.spans.first {
                Task { await model.openDreamSpan(firstSpan) }
            }
        } label: {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(dream.title)
                        .font(GaryxFont.body(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)

                    Spacer(minLength: 8)

                    Text("\(dream.messageCount)")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .frame(height: 24)
                        .background(Color(.tertiarySystemFill), in: Capsule())
                }

                if !dream.summary.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(dream.summary)
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                }

                VStack(alignment: .leading, spacing: 6) {
                    ForEach(dream.spans.prefix(3)) { span in
                        GaryxDreamSpanRow(span: span)
                    }
                }

                HStack(spacing: 8) {
                    Text(dream.sourceDisplayLabel)
                    Spacer(minLength: 8)
                    Text(dream.formattedLastMessageAt)
                }
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(dream.spans.isEmpty)
    }
}

struct GaryxDreamSpanRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let span: GaryxDreamSpan

    var body: some View {
        Button {
            Task { await model.openDreamSpan(span) }
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "arrow.turn.down.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .frame(width: 14)

                VStack(alignment: .leading, spacing: 2) {
                    Text(span.excerpt.isEmpty ? span.threadId : span.excerpt)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                    Text(span.threadDisplayLabel)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 6)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}


private extension GaryxDreamTopic {
    var sourceDisplayLabel: String {
        let normalized = source.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return "unknown" }
        return normalized.replacingOccurrences(of: "_", with: " ")
    }

    var formattedLastMessageAt: String {
        garyxFormattedTaskTimestamp(lastMessageAt)
    }
}

private extension GaryxDreamSpan {
    var threadDisplayLabel: String {
        let seqLabel = startSeq == endSeq ? "#\(startSeq)" : "#\(startSeq)-#\(endSeq)"
        let workspace = workspacePath?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .garyxLastPathComponent ?? ""
        if workspace.isEmpty {
            return "\(threadId) \(seqLabel)"
        }
        return "\(workspace) / \(seqLabel)"
    }
}
