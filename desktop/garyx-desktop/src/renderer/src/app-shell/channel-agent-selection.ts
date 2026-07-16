import type { AgentTargetOption } from "./agent-options";

/** Radix Select needs a non-empty UI value; this token never crosses IPC. */
export const FOLLOW_GLOBAL_AGENT_SELECT_VALUE = "__garyx_follow_global__";

export function channelAgentSelectValue(agentId: string | null): string {
  return agentId || FOLLOW_GLOBAL_AGENT_SELECT_VALUE;
}

export function channelAgentIdFromSelectValue(value: string): string | null {
  return value === FOLLOW_GLOBAL_AGENT_SELECT_VALUE ? null : value;
}

export function suggestedChannelAgentId(
  targets: readonly AgentTargetOption[],
  effectiveDefaultAgentId: string | null,
): string | null {
  return targets.find((target) => target.value === effectiveDefaultAgentId)?.value
    || null;
}

export function explicitChannelAgentUnavailable(
  targets: readonly AgentTargetOption[],
  agentId: string | null,
): boolean {
  return Boolean(
    agentId && !targets.some((target) => target.value === agentId),
  );
}
