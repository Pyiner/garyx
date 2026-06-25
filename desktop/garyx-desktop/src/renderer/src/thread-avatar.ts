import type {
  DesktopApiProviderType,
  DesktopCustomAgent,
  DesktopProviderIconDescriptor,
  DesktopTaskSummary,
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

function teamAvatarIdentity(
  teamId: string,
  catalog: ThreadAvatarCatalog,
  fallbackLabel?: string | null,
): ThreadAvatarIdentity {
  const team = catalog.teamById.get(teamId);
  return {
    agentId: team?.teamId || teamId,
    avatarDataUrl: nullableTrimmed(team?.avatarDataUrl),
    kind: "team",
    label: trimmed(team?.displayName) || trimmed(fallbackLabel) || teamId,
    providerIcon: null,
    providerType: null,
  };
}

function agentAvatarIdentity(
  agentId: string,
  catalog: ThreadAvatarCatalog,
  fallbackLabel?: string | null,
): ThreadAvatarIdentity {
  const agent = catalog.agentById.get(agentId);
  return {
    agentId: agent?.agentId || agentId,
    avatarDataUrl: nullableTrimmed(agent?.avatarDataUrl),
    kind: agent?.builtIn ? "builtin" : "agent",
    label: trimmed(agent?.displayName) || trimmed(fallbackLabel) || agentId,
    providerIcon: agent?.providerIcon || null,
    providerType: agent?.providerType || null,
  };
}

function agentOrTeamAvatarIdentity(
  agentId: string,
  catalog: ThreadAvatarCatalog,
): ThreadAvatarIdentity {
  return catalog.teamById.has(agentId)
    ? teamAvatarIdentity(agentId, catalog)
    : agentAvatarIdentity(agentId, catalog);
}

function fallbackAvatarIdentity(label: string): ThreadAvatarIdentity {
  return {
    agentId: null,
    avatarDataUrl: null,
    kind: "agent",
    label,
    providerIcon: null,
    providerType: null,
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
    return teamAvatarIdentity(
      teamId,
      catalog,
      trimmed(thread.team?.display_name) || trimmed(thread.teamName),
    );
  }

  if (threadAgentId) {
    return agentAvatarIdentity(threadAgentId, catalog);
  }

  return fallbackAvatarIdentity(trimmed(thread.title) || "Thread");
}

export function resolveTaskAvatarIdentity(
  task: Pick<DesktopTaskSummary, "assignee" | "executor" | "runtimeAgentId">,
  catalog: ThreadAvatarCatalog,
): ThreadAvatarIdentity {
  const executor = task.executor;
  if (executor?.type === "team") {
    return teamAvatarIdentity(executor.teamId, catalog);
  }
  if (executor?.type === "agent") {
    return agentAvatarIdentity(executor.agentId, catalog);
  }

  if (task.assignee?.kind === "agent") {
    return agentOrTeamAvatarIdentity(task.assignee.agentId, catalog);
  }
  if (task.assignee?.kind === "human") {
    return fallbackAvatarIdentity(`@${task.assignee.userId}`);
  }

  const runtimeAgentId = trimmed(task.runtimeAgentId);
  if (runtimeAgentId) {
    return agentOrTeamAvatarIdentity(runtimeAgentId, catalog);
  }

  return fallbackAvatarIdentity("unassigned");
}
