import type { AutomationDraft } from "./types";

export function initialAutomationAgentId(input: {
  targetMode: AutomationDraft["targetMode"];
  configuredAgentId?: string | null;
  targetEffectiveAgentId?: string | null;
  effectiveDefaultAgentId: string | null;
}): string {
  return input.targetMode === "existing_thread"
    ? input.targetEffectiveAgentId?.trim() || ""
    : input.configuredAgentId?.trim() || input.effectiveDefaultAgentId || "";
}

export function generatedAutomationAgentError(
  mode: "create" | "edit",
  draft: Pick<
    AutomationDraft,
    "agentId" | "agentChanged" | "initialTargetMode" | "targetMode"
  >,
  availableAgentIds: ReadonlySet<string>,
): string | null {
  if (draft.targetMode === "existing_thread") {
    return null;
  }
  const agentId = draft.agentId.trim();
  const mayPreserveUnavailable =
    mode === "edit"
    && draft.initialTargetMode === "new_thread"
    && !draft.agentChanged;
  return agentId && (availableAgentIds.has(agentId) || mayPreserveUnavailable)
    ? null
    : "Choose an agent for this automation.";
}

export function automationAgentIdForMutation(
  mode: "create" | "edit",
  draft: Pick<AutomationDraft, "agentId" | "agentChanged" | "targetMode">,
): string | undefined {
  if (draft.targetMode === "existing_thread") {
    return undefined;
  }
  if (mode === "edit" && !draft.agentChanged) {
    return undefined;
  }
  return draft.agentId.trim() || undefined;
}
