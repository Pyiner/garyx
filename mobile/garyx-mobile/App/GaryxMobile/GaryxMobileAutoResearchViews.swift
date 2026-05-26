import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxAutoResearchView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateRun = false
    @State private var detailRun: GaryxAutoResearchRun?

    var body: some View {
        GaryxPanelScaffold(
            title: "Auto Research",
            subtitle: "\(model.runningResearchCount) active / \(model.autoResearchRuns.count) total",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                if model.autoResearchRuns.isEmpty, model.isRemoteStatePending {
                    GaryxLoadingPanelView(title: "Loading Auto Research runs...")
                } else if model.autoResearchRuns.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "atom",
                        title: "No Auto Research runs",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Auto Research") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.autoResearchRuns.enumerated()), id: \.element.id) { index, run in
                                GaryxAutoResearchRunCard(run: run) {
                                    detailRun = run
                                    Task { await model.loadAutoResearchDetail(run) }
                                }
                                if index < model.autoResearchRuns.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Auto Research Run") {
                showsCreateRun = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateRun) {
            GaryxCreateAutoResearchCard()
        }
        .sheet(item: $detailRun) { run in
            GaryxAutoResearchDetailSheet(run: run)
        }
    }
}

struct GaryxCreateAutoResearchCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxFormSheet(
            title: "Create Auto Research Run",
            canSave: canStart,
            onSave: { Task { await createRun() } }
        ) {
            VStack(alignment: .leading, spacing: 22) {
                GaryxFormGroupedSection(title: "Goal") {
                    TextField("Goal", text: $model.draftAutoResearchGoal, axis: .vertical)
                        .lineLimit(2...5)
                        .garyxFormTextArea()
                }

                GaryxFormGroupedSection(title: "Workspace") {
                    GaryxWorkspacePathSelectionRow(
                        title: "Workspace",
                        path: workspaceSelection,
                        workspacePaths: workspacePaths,
                        placeholder: "Choose workspace",
                        allowsEmpty: false
                    )
                }

                GaryxFormGroupedSection(title: "Limits") {
                    TextField("Iterations", text: $model.draftAutoResearchIterations)
                        .keyboardType(.numberPad)
                        .garyxFormTextField()
                    Divider().padding(.leading, 16)
                    TextField("Budget min", text: $model.draftAutoResearchTimeBudgetMinutes)
                        .keyboardType(.numberPad)
                        .garyxFormTextField()
                }
            }
        }
        .onAppear(perform: ensureWorkspaceSelection)
    }

    private var workspacePaths: [String] {
        model.userWorkspacePaths
    }

    private var workspaceSelection: Binding<String> {
        Binding {
            effectiveWorkspacePath
        } set: { value in
            model.selectedWorkspacePath = value
        }
    }

    private var effectiveWorkspacePath: String {
        let selected = model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty {
            return selected
        }
        return workspacePaths.first ?? ""
    }

    private var canStart: Bool {
        !model.draftAutoResearchGoal.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !effectiveWorkspacePath.isEmpty
            && positiveInteger(model.draftAutoResearchIterations) != nil
            && positiveAutoResearchBudgetMinutes(model.draftAutoResearchTimeBudgetMinutes) != nil
    }

    private func ensureWorkspaceSelection() {
        guard model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        let nextSelection = effectiveWorkspacePath
        if model.selectedWorkspacePath != nextSelection {
            model.selectedWorkspacePath = nextSelection
        }
    }

    private func createRun() async {
        guard canStart else { return }
        if await model.createAutoResearchRunFromDraft() {
            dismiss()
        }
    }

    private func positiveInteger(_ value: String) -> Int? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = Int(trimmed), parsed > 0 else { return nil }
        return parsed
    }

    private func positiveAutoResearchBudgetMinutes(_ value: String) -> Int? {
        guard let parsed = positiveInteger(value), parsed <= Int.max / 60 else { return nil }
        return parsed
    }
}

struct GaryxAutoResearchRunCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let run: GaryxAutoResearchRun
    let onOpenDetail: () -> Void
    @State private var showsDeleteConfirmation = false

    var body: some View {
        GaryxRowActionMenu(actions: researchSwipeActions) {
            Button(action: onOpenDetail) {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(alignment: .center, spacing: 10) {
                        Image(systemName: "atom")
                            .font(GaryxFont.system(size: 15, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: 24, height: 24)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(run.goal.isEmpty ? run.runId : run.goal)
                                .font(GaryxFont.body(weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(2)
                            Text(run.workspaceDir?.garyxLastPathComponent ?? run.runId)
                                .font(GaryxFont.caption(weight: .medium))
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        GaryxStatusPill(text: garyxAutoResearchStateLabel(run.state), tone: researchTone)
                    }
                    Text("\(run.iterationsUsed) of \(run.maxIterations) iterations")
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 11)
            }
            .buttonStyle(.plain)
            .accessibilityHint("Open Auto Research details")
        }
        .confirmationDialog("Delete Auto Research run?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteAutoResearchRun(run) }
            }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("This removes the run, iterations, and candidates.")
        }
    }

    private var researchSwipeActions: [GaryxRowAction] {
        var actions: [GaryxRowAction] = []
        if !garyxAutoResearchIsTerminal(run.state) {
            actions.append(
                GaryxRowAction(title: "Stop", systemImage: "stop.fill", tone: .warning) {
                    Task { await model.stopAutoResearchRun(run) }
                }
            )
        }
        actions.append(
            GaryxRowAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        )
        return actions
    }

    private var researchTone: GaryxStatusPill.Tone {
        garyxAutoResearchTone(run)
    }
}

struct GaryxAutoResearchDetailSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let run: GaryxAutoResearchRun
    @State private var feedbackCandidate: GaryxResearchCandidate?
    @State private var feedbackDraft = ""

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    summaryBlock
                    iterationBlock
                    if orphanCandidates.count > 0 {
                        candidateBlock
                    }
                }
                .padding(12)
                .frame(maxWidth: 620, alignment: .leading)
                .frame(maxWidth: .infinity)
            }
            .background(GaryxTheme.background)
            .refreshable {
                await model.loadAutoResearchDetail(runId: run.runId)
            }
            .navigationTitle("Auto Research")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Done") {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    if let activeThreadId {
                        Button {
                            openThread(activeThreadId)
                        } label: {
                            Label("Open", systemImage: "arrow.up.right")
                        }
                    }
                }
            }
        }
        .task {
            await model.loadAutoResearchDetail(runId: run.runId)
        }
        .sheet(item: $feedbackCandidate, onDismiss: {
            feedbackDraft = ""
        }) { candidate in
            GaryxAutoResearchFeedbackSheet(candidate: candidate, feedback: $feedbackDraft) { feedback in
                let current = currentRun
                feedbackCandidate = nil
                feedbackDraft = ""
                Task {
                    await model.sendAutoResearchFeedback(
                        run: current,
                        candidate: candidate,
                        feedback: feedback
                    )
                }
            }
        }
    }

    private var currentRun: GaryxAutoResearchRun {
        model.autoResearchDetailsByRunId[run.runId]?.run
            ?? model.autoResearchRuns.first { $0.runId == run.runId }
            ?? run
    }

    private var detail: GaryxAutoResearchDetail? {
        model.autoResearchDetailsByRunId[run.runId]
    }

    private var candidatesPage: GaryxAutoResearchCandidatesPage? {
        model.researchCandidatesByRunId[run.runId]
    }

    private var candidates: [GaryxResearchCandidate] {
        candidatesPage?.candidates ?? []
    }

    private var candidatesByIteration: [Int: GaryxResearchCandidate] {
        var result: [Int: GaryxResearchCandidate] = [:]
        for candidate in candidates {
            result[candidate.iteration] = candidate
        }
        return result
    }

    private var displayIterations: [GaryxAutoResearchIteration] {
        var items = model.autoResearchIterationsByRunId[run.runId] ?? []
        if let latest = detail?.latestIteration,
           !items.contains(where: { $0.iterationIndex == latest.iterationIndex }) {
            items.append(latest)
        }
        return items.sorted { $0.iterationIndex < $1.iterationIndex }
    }

    private var orphanCandidates: [GaryxResearchCandidate] {
        let iterationIds = Set(displayIterations.map(\.iterationIndex))
        return candidates
            .filter { !iterationIds.contains($0.iteration) }
            .sorted { $0.iteration > $1.iteration }
    }

    private var summaryBlock: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "atom")
                    .font(GaryxFont.system(size: 16, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)
                    .background(Color(.secondarySystemGroupedBackground), in: Circle())
                VStack(alignment: .leading, spacing: 5) {
                    Text(currentRun.goal.isEmpty ? currentRun.runId : currentRun.goal)
                        .font(GaryxFont.body(weight: .semibold))
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(summarySubtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Spacer(minLength: 0)
                GaryxStatusPill(text: garyxAutoResearchStateLabel(currentRun.state), tone: garyxAutoResearchTone(currentRun))
            }
            if let terminalReason {
                Text(terminalReason)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            HStack(spacing: 8) {
                GaryxAutoResearchMetricPill(
                    title: "Iterations",
                    value: "\(currentRun.iterationsUsed) of \(currentRun.maxIterations)"
                )
                if let selectedCandidate {
                    GaryxAutoResearchMetricPill(
                        title: "Winner",
                        value: candidateMetricValue(selectedCandidate)
                    )
                } else if let bestCandidate {
                    GaryxAutoResearchMetricPill(
                        title: "Best",
                        value: candidateMetricValue(bestCandidate)
                    )
                }
                Spacer(minLength: 0)
            }
            if let activeThreadId {
                Button {
                    openThread(activeThreadId)
                } label: {
                    Label("Open Active Thread", systemImage: "arrow.up.right")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(GaryxSecondaryButtonStyle())
            }
        }
        .padding(12)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }

    private var iterationBlock: some View {
        GaryxSectionBlock(title: "Iterations") {
            if displayIterations.isEmpty {
                Text("No iteration records yet.")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 4)
            } else {
                GaryxCompactListGroup {
                    ForEach(Array(displayIterations.enumerated()), id: \.element.id) { index, iteration in
                        let candidate = candidatesByIteration[iteration.iterationIndex]
                        GaryxResearchIterationRow(
                            iteration: iteration,
                            candidate: candidate,
                            isBest: candidate?.candidateId == candidatesPage?.bestCandidateId,
                            isSelected: candidate?.candidateId == currentRun.selectedCandidate,
                            isRunTerminal: garyxAutoResearchIsTerminal(currentRun.state),
                            onSelect: { candidate in
                                Task { await model.selectAutoResearchCandidate(run: currentRun, candidate: candidate) }
                            },
                            onReverify: { candidate in
                                Task { await model.reverifyAutoResearchCandidate(run: currentRun, candidate: candidate) }
                            },
                            onFeedback: openFeedback,
                            onOpenThread: openThread
                        )
                        if index < displayIterations.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                }
            }
        }
    }

    private var candidateBlock: some View {
        GaryxSectionBlock(title: "Candidates") {
            GaryxCompactListGroup {
                ForEach(Array(orphanCandidates.enumerated()), id: \.element.id) { index, candidate in
                    GaryxResearchCandidateRow(
                        candidate: candidate,
                        isBest: candidate.candidateId == candidatesPage?.bestCandidateId,
                        isSelected: candidate.candidateId == currentRun.selectedCandidate,
                        isRunTerminal: garyxAutoResearchIsTerminal(currentRun.state),
                        onSelect: {
                            Task { await model.selectAutoResearchCandidate(run: currentRun, candidate: candidate) }
                        },
                        onReverify: {
                            Task { await model.reverifyAutoResearchCandidate(run: currentRun, candidate: candidate) }
                        },
                        onFeedback: {
                            openFeedback(candidate)
                        }
                    )
                    if index < orphanCandidates.count - 1 {
                        GaryxCompactRowDivider()
                    }
                }
            }
        }
    }

    private var summarySubtitle: String {
        let workspace = currentRun.workspaceDir?.garyxLastPathComponent ?? "No workspace"
        let updated = garyxFormattedTaskTimestamp(currentRun.updatedAt)
        return updated.isEmpty ? workspace : "\(workspace) · updated \(updated)"
    }

    private var activeThreadId: String? {
        let value = detail?.activeThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    private var terminalReason: String? {
        let value = currentRun.terminalReason?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : garyxAutoResearchReasonLabel(value)
    }

    private var selectedCandidate: GaryxResearchCandidate? {
        let selectedId = currentRun.selectedCandidate?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !selectedId.isEmpty else { return nil }
        return candidates.first { $0.candidateId == selectedId }
    }

    private var bestCandidate: GaryxResearchCandidate? {
        let bestId = candidatesPage?.bestCandidateId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !bestId.isEmpty else { return nil }
        return candidates.first { $0.candidateId == bestId }
    }

    private func candidateMetricValue(_ candidate: GaryxResearchCandidate) -> String {
        if let score = candidate.verdict?.score {
            return String(format: "%.1f/10", score)
        }
        return "Candidate \(candidate.iteration)"
    }

    private func openFeedback(_ candidate: GaryxResearchCandidate) {
        feedbackCandidate = candidate
        feedbackDraft = ""
    }

    private func openThread(_ threadId: String?) {
        let threadId = threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !threadId.isEmpty else { return }
        dismiss()
        Task { await model.openThread(id: threadId) }
    }
}

struct GaryxAutoResearchMetricPill: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
            Text(value)
                .font(GaryxFont.footnote(weight: .semibold))
                .foregroundStyle(.primary)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

struct GaryxAutoResearchFeedbackSheet: View {
    @Environment(\.dismiss) private var dismiss
    let candidate: GaryxResearchCandidate
    @Binding var feedback: String
    let onSend: (String) -> Void

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 12) {
                TextEditor(text: $feedback)
                    .font(GaryxFont.body())
                    .scrollContentBackground(.hidden)
                    .padding(8)
                    .frame(minHeight: 160)
                    .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .stroke(GaryxTheme.hairline, lineWidth: 1)
                    }
                Text("\(feedback.trimmingCharacters(in: .whitespacesAndNewlines).count) characters")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
            .padding(16)
            .background(GaryxTheme.background)
            .navigationTitle("Feedback on Candidate \(candidate.iteration)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Cancel") {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Send") {
                        let value = feedback.trimmingCharacters(in: .whitespacesAndNewlines)
                        onSend(value)
                        dismiss()
                    }
                    .disabled(feedback.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}

struct GaryxResearchIterationRow: View {
    let iteration: GaryxAutoResearchIteration
    let candidate: GaryxResearchCandidate?
    let isBest: Bool
    let isSelected: Bool
    let isRunTerminal: Bool
    let onSelect: (GaryxResearchCandidate) -> Void
    let onReverify: (GaryxResearchCandidate) -> Void
    let onFeedback: (GaryxResearchCandidate) -> Void
    let onOpenThread: (String?) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 8) {
                Text("Iteration \(iteration.iterationIndex)")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                GaryxStatusPill(
                    text: garyxAutoResearchStateLabel(iteration.state.isEmpty ? "pending" : iteration.state),
                    tone: garyxAutoResearchTone(iteration.state)
                )
                if isSelected {
                    GaryxStatusPill(text: "Winner", tone: .good)
                } else if isBest {
                    GaryxStatusPill(text: "Current best", tone: .good)
                }
                Spacer(minLength: 0)
            }
            if let candidate {
                GaryxResearchCandidateContent(candidate: candidate)
                GaryxResearchCandidateActions(
                    candidate: candidate,
                    isSelected: isSelected,
                    isRunTerminal: isRunTerminal,
                    onSelect: { onSelect(candidate) },
                    onReverify: { onReverify(candidate) },
                    onFeedback: { onFeedback(candidate) }
                )
            } else {
                Text(iteration.state.lowercased() == "completed" ? "No candidate recorded for this iteration." : "This iteration is still running.")
                    .font(GaryxFont.footnote())
                    .foregroundStyle(.secondary)
            }
            if hasThreadLinks {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) {
                        threadLinkControls
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        threadLinkControls
                    }
                }
            }
        }
        .padding(10)
    }

    private var workThreadId: String? {
        let value = iteration.workThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    private var verifyThreadId: String? {
        let value = iteration.verifyThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return value.isEmpty ? nil : value
    }

    private var hasThreadLinks: Bool {
        workThreadId != nil || verifyThreadId != nil
    }

    @ViewBuilder
    private var threadLinkControls: some View {
        if let workThreadId {
            Button {
                onOpenThread(workThreadId)
            } label: {
                Label("Work", systemImage: "doc.text")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
        if let verifyThreadId {
            Button {
                onOpenThread(verifyThreadId)
            } label: {
                Label("Verify", systemImage: "checkmark.seal")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
    }
}

struct GaryxResearchCandidateRow: View {
    let candidate: GaryxResearchCandidate
    let isBest: Bool
    let isSelected: Bool
    let isRunTerminal: Bool
    let onSelect: () -> Void
    let onReverify: () -> Void
    let onFeedback: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack {
                Text("Candidate \(candidate.iteration)")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                if isSelected {
                    GaryxStatusPill(text: "Winner", tone: .good)
                } else if isBest {
                    GaryxStatusPill(text: "Current best", tone: .good)
                }
                Spacer(minLength: 0)
            }
            GaryxResearchCandidateContent(candidate: candidate)
            GaryxResearchCandidateActions(
                candidate: candidate,
                isSelected: isSelected,
                isRunTerminal: isRunTerminal,
                onSelect: onSelect,
                onReverify: onReverify,
                onFeedback: onFeedback
            )
        }
        .padding(10)
    }
}

struct GaryxResearchCandidateContent: View {
    let candidate: GaryxResearchCandidate

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(candidate.output.isEmpty ? "No candidate output yet." : candidate.output)
                .font(GaryxFont.footnote())
                .foregroundStyle(.secondary)
                .lineLimit(8)
            if let verdict = candidate.verdict {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Score \(String(format: "%.1f", verdict.score))/10")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.primary)
                    if !verdict.feedback.isEmpty {
                        Text(verdict.feedback)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(3)
                    }
                }
            }
        }
    }
}

struct GaryxResearchCandidateActions: View {
    let candidate: GaryxResearchCandidate
    let isSelected: Bool
    let isRunTerminal: Bool
    let onSelect: () -> Void
    let onReverify: () -> Void
    let onFeedback: () -> Void

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                controls
            }
            VStack(alignment: .leading, spacing: 8) {
                controls
            }
        }
    }

    @ViewBuilder
    private var controls: some View {
        if isSelected {
            GaryxStatusPill(text: "Selected Winner", tone: .good)
                .fixedSize(horizontal: true, vertical: false)
        } else {
            Button {
                onSelect()
            } label: {
                Label("Select", systemImage: "checkmark")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
        if !isRunTerminal {
            Button {
                onReverify()
            } label: {
                Label("Reverify", systemImage: "arrow.clockwise")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
            Button {
                onFeedback()
            } label: {
                Label("Feedback", systemImage: "text.bubble")
            }
            .buttonStyle(GaryxSecondaryButtonStyle())
            .fixedSize(horizontal: true, vertical: false)
        }
    }
}

func garyxAutoResearchIsTerminal(_ state: String) -> Bool {
    switch state.lowercased() {
    case "user_stopped", "budget_exhausted", "blocked":
        true
    default:
        false
    }
}

func garyxAutoResearchStateLabel(_ state: String) -> String {
    switch state.lowercased() {
    case "queued":
        "Queued"
    case "researching":
        "Researching"
    case "judging":
        "Judging"
    case "budget_exhausted":
        "Budget exhausted"
    case "blocked":
        "Blocked"
    case "user_stopped":
        "Stopped"
    case "completed":
        "Completed"
    case "pending":
        "Pending"
    default:
        state
            .split(separator: "_")
            .map { word in
                word.prefix(1).uppercased() + String(word.dropFirst())
            }
            .joined(separator: " ")
    }
}

func garyxAutoResearchReasonLabel(_ reason: String) -> String {
    let normalized = reason.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !normalized.isEmpty else { return "" }
    switch normalized.lowercased() {
    case "user_requested", "user_stopped":
        return "Stopped by user"
    case "time_budget_exhausted":
        return "Time budget exhausted"
    case "budget_exhausted":
        return "Budget exhausted"
    case "blocked":
        return "Blocked"
    default:
        return normalized
            .split(separator: "_")
            .map { word in
                word.prefix(1).uppercased() + String(word.dropFirst())
            }
            .joined(separator: " ")
    }
}

func garyxAutoResearchTone(_ run: GaryxAutoResearchRun) -> GaryxStatusPill.Tone {
    let selected = run.selectedCandidate?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    if !selected.isEmpty {
        return .good
    }
    return garyxAutoResearchTone(run.state)
}

func garyxAutoResearchTone(_ state: String) -> GaryxStatusPill.Tone {
    switch state.lowercased() {
    case "completed":
        .good
    case "blocked":
        .danger
    case "user_stopped", "budget_exhausted":
        .muted
    default:
        .warning
    }
}
