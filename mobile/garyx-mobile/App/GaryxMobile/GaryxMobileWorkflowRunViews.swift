import SwiftUI

struct GaryxWorkflowRunView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxPanelScaffold(
            title: title,
            subtitle: "",
            onRefresh: { await model.refreshSelectedWorkflowRun() },
            showsRefreshButton: true,
            contentHorizontalPadding: 16
        ) {
            content
        } actions: {
            EmptyView()
        }
        .task {
            await model.refreshSelectedWorkflowRun()
            model.startWorkflowRunPollingIfNeeded()
        }
    }

    @ViewBuilder
    private var content: some View {
        switch model.workflowRunPanelState.mode {
        case .idle:
            GaryxEmptyPanelView(icon: "point.3.connected.trianglepath.dotted", title: "No workflow run selected.", text: "")
        case .resolving(let threadId):
            GaryxWorkflowRunLoadingView(title: "Opening thread", detail: threadId)
        case .run:
            switch model.workflowRunPanelState.phase {
            case .idle, .loading:
                GaryxWorkflowRunLoadingView(title: "Loading workflow run", detail: model.workflowRunPanelState.activeWorkflowRunId ?? "")
            case .failed(let message):
                if let presentation = model.workflowRunPanelState.presentation {
                    GaryxWorkflowRunContent(presentation: presentation)
                } else {
                    GaryxEmptyPanelView(icon: "exclamationmark.triangle", title: "Unable to load workflow run.", text: message)
                }
            case .loaded:
                if let presentation = model.workflowRunPanelState.presentation {
                    GaryxWorkflowRunContent(presentation: presentation)
                } else {
                    GaryxWorkflowRunLoadingView(title: "Loading workflow run", detail: model.workflowRunPanelState.activeWorkflowRunId ?? "")
                }
            }
        }
    }

    private var title: String {
        model.workflowRunPanelState.presentation?.title
            ?? model.selectedWorkflowRunThread?.title
            ?? "Workflow Run"
    }
}

private struct GaryxWorkflowRunLoadingView: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(alignment: .center, spacing: 10) {
            ProgressView()
            Text(title)
                .font(GaryxFont.subheadline(weight: .semibold))
                .foregroundStyle(.primary)
            if !detail.isEmpty {
                Text(detail)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 48)
    }
}

private struct GaryxWorkflowRunContent: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let presentation: GaryxWorkflowPresentation

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            GaryxWorkflowRunHeader(presentation: presentation)
            GaryxWorkflowRunProgress(presentation: presentation)
            GaryxWorkflowPhaseList(phases: presentation.phases)
            GaryxWorkflowChildrenSection(children: presentation.childCards)
            GaryxWorkflowOutcomeSection(presentation: presentation)
        }
        .animation(.easeInOut(duration: 0.18), value: presentation.snapshotVersion)
    }
}

private struct GaryxWorkflowRunHeader: View {
    let presentation: GaryxWorkflowPresentation

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "point.3.connected.trianglepath.dotted")
                    .font(GaryxFont.system(size: 20, weight: .semibold))
                    .foregroundStyle(GaryxTheme.accent)
                    .frame(width: 42, height: 42)
                    .background(GaryxTheme.accent.opacity(0.10), in: Circle())

                VStack(alignment: .leading, spacing: 5) {
                    Text(presentation.title)
                        .font(GaryxFont.title3(weight: .semibold))
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let description = presentation.description?.trimmingCharacters(in: .whitespacesAndNewlines),
                       !description.isEmpty {
                        Text(description)
                            .font(GaryxFont.footnote())
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Text(presentation.workflowRunId)
                        .font(GaryxFont.system(size: 11))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 0)

                GaryxStatusPill(
                    text: presentation.stale == true ? "Converging" : GaryxWorkflowStatusPresentation.label(for: presentation.status),
                    tone: presentation.stale == true ? .warning : presentation.status.workflowStatusTone
                )
            }

            if let activePhase = presentation.activePhase {
                HStack(spacing: 8) {
                    Image(systemName: "arrow.trianglehead.2.clockwise")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(GaryxTheme.accent)
                    Text(activePhase.title)
                        .font(GaryxFont.footnote(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    if let detail = activePhase.detail, !detail.isEmpty {
                        Text(detail)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(GaryxTheme.accent.opacity(0.07), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            }
        }
        .padding(14)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }
}

private struct GaryxWorkflowRunProgress: View {
    let presentation: GaryxWorkflowPresentation

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Text("\(presentation.counts.completed) of \(presentation.counts.total) children")
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                Spacer(minLength: 8)
                Text("\(presentation.counts.completedPhases) of \(presentation.counts.totalPhases) phases")
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            GeometryReader { proxy in
                ZStack(alignment: .leading) {
                    Capsule()
                        .fill(Color.primary.opacity(0.08))
                    Capsule()
                        .fill(progressColor)
                        .frame(width: max(6, proxy.size.width * presentation.progressFraction))
                }
            }
            .frame(height: 8)

            HStack(spacing: 8) {
                GaryxWorkflowMetricPill(label: "Running", value: presentation.counts.runningChildren)
                GaryxWorkflowMetricPill(label: "Queued", value: presentation.counts.queuedChildren)
                GaryxWorkflowMetricPill(label: "Failed", value: presentation.counts.failedChildren)
                Spacer(minLength: 0)
            }
        }
        .padding(12)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }

    private var progressColor: Color {
        presentation.counts.failedChildren > 0 || presentation.stale == true ? GaryxTheme.warning : GaryxTheme.accent
    }
}

private struct GaryxWorkflowMetricPill: View {
    let label: String
    let value: Int

    var body: some View {
        Text("\(label) \(value)")
            .font(GaryxFont.system(size: 11, weight: .semibold))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .fixedSize(horizontal: true, vertical: false)
            .padding(.horizontal, 7)
            .padding(.vertical, 4)
            .background(Color.primary.opacity(0.055), in: Capsule())
    }
}

private struct GaryxWorkflowPhaseList: View {
    let phases: [GaryxWorkflowPhase]

    var body: some View {
        GaryxSectionBlock(title: "Phases") {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(phases, id: \.phaseId) { phase in
                    GaryxWorkflowPhaseRow(phase: phase)
                }
            }
        }
    }
}

private struct GaryxWorkflowPhaseRow: View {
    let phase: GaryxWorkflowPhase

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .center, spacing: 10) {
                Image(systemName: phase.status.workflowStatusSymbol)
                    .font(GaryxFont.system(size: 13, weight: .semibold))
                    .foregroundStyle(phase.status.workflowStatusColor)
                    .frame(width: 26, height: 26)
                    .background(phase.status.workflowStatusColor.opacity(0.10), in: Circle())
                VStack(alignment: .leading, spacing: 2) {
                    Text(phase.title)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    if let detail = phase.detail, !detail.isEmpty {
                        Text(detail)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                }
                Spacer(minLength: 0)
                GaryxStatusPill(
                    text: GaryxWorkflowStatusPresentation.label(for: phase.status),
                    tone: phase.status.workflowStatusTone
                )
            }

            if !phase.children.isEmpty {
                VStack(spacing: 6) {
                    ForEach(phase.children) { child in
                        GaryxWorkflowCompactChildRow(child: child)
                    }
                }
                .padding(.leading, 36)
            }
        }
        .padding(12)
        .background(phase.active ? GaryxTheme.accent.opacity(0.055) : GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(phase.active ? GaryxTheme.accent.opacity(0.25) : GaryxTheme.hairline, lineWidth: 1)
        }
    }
}

private struct GaryxWorkflowCompactChildRow: View {
    let child: GaryxWorkflowChildCard

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(child.status.workflowStatusColor)
                .frame(width: 7, height: 7)
            Text(child.label)
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
            Spacer(minLength: 0)
            Text(GaryxWorkflowStatusPresentation.label(for: child.status))
                .font(GaryxFont.system(size: 10, weight: .semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}

private struct GaryxWorkflowChildrenSection: View {
    let children: [GaryxWorkflowChildCard]

    var body: some View {
        GaryxSectionBlock(title: "Child Runs") {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(children) { child in
                    GaryxWorkflowChildCardView(child: child)
                }
            }
        }
    }
}

private struct GaryxWorkflowChildCardView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let child: GaryxWorkflowChildCard

    var body: some View {
        Button {
            Task { await model.openThread(id: child.threadId, source: .current) }
        } label: {
            HStack(alignment: .top, spacing: 11) {
                GaryxAgentAvatarView(
                    agentId: target?.id ?? child.agentId ?? "",
                    avatarDataUrl: target?.avatarDataUrl ?? "",
                    label: target?.title ?? child.agentId ?? child.label,
                    providerType: target?.providerType ?? "",
                    builtIn: target?.builtIn ?? false,
                    diameter: 34
                )

                VStack(alignment: .leading, spacing: 6) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(child.label)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(2)
                        Spacer(minLength: 0)
                        GaryxStatusPill(
                            text: GaryxWorkflowStatusPresentation.label(for: child.status),
                            tone: child.status.workflowStatusTone
                        )
                    }
                    Text(child.phaseTitle)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    if let preview = child.resultPreview?.trimmingCharacters(in: .whitespacesAndNewlines), !preview.isEmpty {
                        Text(preview)
                            .font(GaryxFont.footnote())
                            .foregroundStyle(.secondary)
                            .lineLimit(3)
                    } else if let error = child.error?.trimmingCharacters(in: .whitespacesAndNewlines), !error.isEmpty {
                        Text(error)
                            .font(GaryxFont.footnote())
                            .foregroundStyle(GaryxTheme.danger)
                            .lineLimit(3)
                    }
                    HStack(spacing: 8) {
                        Text("\(child.tokens) tokens")
                        Text("\(child.toolCalls) tools")
                        Spacer(minLength: 0)
                    }
                    .font(GaryxFont.system(size: 11, weight: .medium))
                    .foregroundStyle(.tertiary)
                }
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Open \(child.label)")
    }

    private var target: GaryxMobileAgentTarget? {
        guard let agentId = child.agentId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !agentId.isEmpty else {
            return nil
        }
        return model.agentTargets.first { $0.id == agentId }
    }
}

private struct GaryxWorkflowOutcomeSection: View {
    let presentation: GaryxWorkflowPresentation

    var body: some View {
        GaryxSectionBlock(title: "Outcome") {
            VStack(alignment: .leading, spacing: 10) {
                if let output = presentation.outputText?.trimmingCharacters(in: .whitespacesAndNewlines), !output.isEmpty {
                    Text(output)
                        .font(GaryxFont.footnote())
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .overlay {
                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                .stroke(GaryxTheme.hairline, lineWidth: 1)
                        }
                } else if let error = presentation.error?.trimmingCharacters(in: .whitespacesAndNewlines), !error.isEmpty {
                    GaryxNotice(title: "Workflow failed", text: error)
                } else {
                    GaryxNotice(title: outcomeTitle, text: outcomeText)
                }
            }
        }
    }

    private var outcomeTitle: String {
        switch presentation.outcome.kind {
        case "finalText":
            return "Final output"
        case "structuredOnly":
            return "Structured result"
        case "completedNoOutput":
            return "Completed"
        case "failed":
            return "Failed"
        case "cancelled":
            return "Cancelled"
        default:
            return "In progress"
        }
    }

    private var outcomeText: String {
        if presentation.outcome.hasResult {
            return "Structured result is available."
        }
        if presentation.stale == true {
            return "Child runs are still converging."
        }
        if presentation.terminalComplete {
            return "Workflow run is complete."
        }
        return "Waiting for the workflow run to finish."
    }
}

private extension String {
    var workflowStatusTone: GaryxStatusPill.Tone {
        switch GaryxWorkflowStatusPresentation.normalized(self) {
        case "succeeded":
            return .good
        case "failed", "cancelled":
            return .danger
        case "running", "in_progress":
            return .warning
        default:
            return .muted
        }
    }

    var workflowStatusColor: Color {
        switch workflowStatusTone {
        case .good:
            return GaryxTheme.accent
        case .warning:
            return GaryxTheme.warning
        case .danger:
            return GaryxTheme.danger
        case .muted:
            return .secondary
        }
    }

    var workflowStatusSymbol: String {
        switch GaryxWorkflowStatusPresentation.normalized(self) {
        case "succeeded":
            return "checkmark"
        case "failed":
            return "xmark"
        case "cancelled":
            return "minus"
        case "running", "in_progress":
            return "arrow.trianglehead.2.clockwise"
        case "queued":
            return "clock"
        case "skipped":
            return "forward"
        default:
            return "circle"
        }
    }
}
