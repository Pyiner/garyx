import type { DesktopAgentCatalog, DesktopCustomAgent } from "@shared/contracts";

export type AgentDefaultBadge =
  | "default"
  | "default-inactive"
  | "acting-default"
  | "default-auto"
  | null;

export function defaultBadgeForAgent(
  catalog: Pick<
    DesktopAgentCatalog,
    "defaultAgentId" | "effectiveDefaultAgentId"
  >,
  agent: Pick<DesktopCustomAgent, "agentId" | "enabled">,
): AgentDefaultBadge {
  const raw = catalog.defaultAgentId;
  const effective = catalog.effectiveDefaultAgentId;

  if (raw === agent.agentId) {
    return agent.enabled && effective === agent.agentId
      ? "default"
      : "default-inactive";
  }
  if (effective !== agent.agentId) {
    return null;
  }
  return raw === null ? "default-auto" : "acting-default";
}

export function canUseAgentForNewBinding(
  agent: Pick<DesktopCustomAgent, "enabled" | "standalone"> | null | undefined,
): boolean {
  return Boolean(agent?.enabled && agent.standalone);
}

export function isNewDraftBindingBlocked(
  hasNewThreadDraft: boolean,
  agent: Pick<DesktopCustomAgent, "enabled" | "standalone"> | null | undefined,
): boolean {
  return hasNewThreadDraft && !canUseAgentForNewBinding(agent);
}

export function suggestedAgentId(
  catalog: Pick<DesktopAgentCatalog, "effectiveDefaultAgentId">,
): string | null {
  return catalog.effectiveDefaultAgentId;
}

export function agentManagementActionState(
  catalog: Pick<DesktopAgentCatalog, "defaultAgentId">,
  agent: Pick<DesktopCustomAgent, "agentId" | "enabled" | "standalone">,
): { chatEnabled: boolean; setDefaultVisible: boolean } {
  return {
    chatEnabled: agent.enabled && agent.standalone,
    setDefaultVisible:
      agent.enabled
      && agent.standalone
      && catalog.defaultAgentId !== agent.agentId,
  };
}
