import type { DesktopCustomAgent, DesktopTeam } from "@shared/contracts";

export type AgentOptionKind = "builtin" | "agent" | "team";

export type ComposerAgentOption = {
  id: string;
  label: string;
  kind: AgentOptionKind;
  detail?: string;
};

export type AgentTargetOption = {
  value: string;
  label: string;
};

const PROVIDER_LABELS: Record<string, string> = {
  codex_app_server: "Codex",
  gemini_cli: "Gemini",
};

const byDisplayName = <T extends { displayName: string }>(a: T, b: T) =>
  a.displayName.localeCompare(b.displayName);

/**
 * Build the flat agent-option list shown in new-thread pickers: built-in
 * agents, custom solo agents (excluding team leaders), then teams.
 */
export function buildAgentOptions(
  agents: DesktopCustomAgent[],
  teams: DesktopTeam[],
): ComposerAgentOption[] {
  const teamLeaderIds = new Set(teams.map((team) => team.leaderAgentId));
  const agentNameById = new Map(
    agents.map((agent) => [agent.agentId, agent.displayName] as const),
  );

  const options: ComposerAgentOption[] = [];

  for (const agent of agents
    .filter((a) => a.builtIn && a.standalone)
    .sort(byDisplayName)) {
    options.push({
      id: agent.agentId,
      label: agent.displayName,
      kind: "builtin",
    });
  }

  for (const agent of agents
    .filter(
      (a) => !a.builtIn && a.standalone && !teamLeaderIds.has(a.agentId),
    )
    .sort(byDisplayName)) {
    options.push({
      id: agent.agentId,
      label: agent.displayName,
      kind: "agent",
      detail: PROVIDER_LABELS[agent.providerType],
    });
  }

  for (const team of [...teams].sort(byDisplayName)) {
    const leaderLabel =
      agentNameById.get(team.leaderAgentId) || team.leaderAgentId;
    options.push({
      id: team.teamId,
      label: team.displayName,
      kind: "team",
      detail: `Lead: ${leaderLabel}`,
    });
  }

  return options;
}

export function groupAgentOptions(options: ComposerAgentOption[]) {
  return {
    builtin: options.filter((o) => o.kind === "builtin"),
    agent: options.filter((o) => o.kind === "agent"),
    team: options.filter((o) => o.kind === "team"),
  };
}

function formatAgentTargetLabel(agent: DesktopCustomAgent): string {
  const core = agent.displayName.trim() === agent.agentId.trim()
    ? agent.displayName
    : `${agent.displayName} (${agent.agentId})`;
  return `${core} · ${PROVIDER_LABELS[agent.providerType] || "Claude"}`;
}

function formatTeamTargetLabel(team: DesktopTeam): string {
  return team.displayName.trim() === team.teamId.trim()
    ? `${team.displayName} (team)`
    : `${team.displayName} (${team.teamId}, team)`;
}

export function buildAgentTargetOptions(
  agents: DesktopCustomAgent[],
  teams: DesktopTeam[],
): AgentTargetOption[] {
  const agentOptions = [...agents]
    .filter((agent) => agent.standalone)
    .sort((left, right) => {
      if (left.builtIn !== right.builtIn) {
        return left.builtIn ? -1 : 1;
      }
      return left.displayName.localeCompare(right.displayName)
        || left.agentId.localeCompare(right.agentId);
    })
    .map((agent) => ({
      value: agent.agentId,
      label: formatAgentTargetLabel(agent),
    }));

  const teamOptions = [...teams]
    .sort((left, right) => {
      return left.displayName.localeCompare(right.displayName)
        || left.teamId.localeCompare(right.teamId);
    })
    .map((team) => ({
      value: team.teamId,
      label: formatTeamTargetLabel(team),
    }));

  return [...agentOptions, ...teamOptions];
}
