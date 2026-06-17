import type {
  DesktopApiProviderType,
  DesktopCustomAgent,
  DesktopProviderIconDescriptor,
  DesktopTeam,
  DesktopThreadSummary,
} from "@shared/contracts";

export type ThreadAvatarIdentity = {
  agentId: string | null;
  avatarDataUrl: string | null;
  kind: "builtin" | "agent" | "team";
  label: string;
  providerIcon?: DesktopProviderIconDescriptor | null;
  providerType?: DesktopApiProviderType | null;
};

export type ThreadAvatarCatalog = {
  agentById: ReadonlyMap<string, DesktopCustomAgent>;
  teamById: ReadonlyMap<string, DesktopTeam>;
};

function trimmed(value?: string | null): string {
  return value?.trim() || "";
}

function nullableTrimmed(value?: string | null): string | null {
  const next = trimmed(value);
  return next || null;
}

export function buildThreadAvatarCatalog(
  agents: readonly DesktopCustomAgent[],
  teams: readonly DesktopTeam[],
): ThreadAvatarCatalog {
  return {
    agentById: new Map(agents.map((agent) => [agent.agentId, agent] as const)),
    teamById: new Map(teams.map((team) => [team.teamId, team] as const)),
  };
}

export function resolveThreadAvatarIdentity(
  thread: DesktopThreadSummary,
  catalog: ThreadAvatarCatalog,
): ThreadAvatarIdentity {
  const explicitTeamId =
    trimmed(thread.team?.team_id) ||
    trimmed(thread.teamId);
  const threadAgentId = trimmed(thread.agentId);
  const teamId = explicitTeamId ||
    (threadAgentId && catalog.teamById.has(threadAgentId) ? threadAgentId : "");

  if (teamId) {
    const team = catalog.teamById.get(teamId);
    return {
      agentId: team?.teamId || teamId,
      avatarDataUrl: nullableTrimmed(team?.avatarDataUrl),
      kind: "team",
      label:
        trimmed(team?.displayName) ||
        trimmed(thread.team?.display_name) ||
        trimmed(thread.teamName) ||
        teamId,
      providerIcon: null,
      providerType: null,
    };
  }

  if (threadAgentId) {
    const agent = catalog.agentById.get(threadAgentId);
    return {
      agentId: agent?.agentId || threadAgentId,
      avatarDataUrl: nullableTrimmed(agent?.avatarDataUrl),
      kind: agent?.builtIn ? "builtin" : "agent",
      label: trimmed(agent?.displayName) || threadAgentId,
      providerIcon: agent?.providerIcon || null,
      providerType: agent?.providerType || null,
    };
  }

  return {
    agentId: null,
    avatarDataUrl: null,
    kind: "agent",
    label: trimmed(thread.title) || "Thread",
    providerIcon: null,
    providerType: null,
  };
}
