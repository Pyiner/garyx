import type { DesktopApiProviderType, DesktopCustomAgent, DesktopTeam } from "@shared/contracts";

export type AgentOptionKind = "builtin" | "agent" | "team";

export type AgentPickerOption = {
  id: string;
  label: string;
  kind: AgentOptionKind;
  avatarDataUrl?: string | null;
  detail?: string;
  providerType?: DesktopApiProviderType;
};

export type ComposerAgentOption = AgentPickerOption;

export type AgentTargetOption = AgentPickerOption & {
  value: string;
};

type AgentLabelStyle = "display" | "target";

type TeamLabelStyle = "display" | "target";

const byDisplayName = <T extends { displayName: string }>(a: T, b: T) =>
  a.displayName.localeCompare(b.displayName);

function sortAgents(agents: DesktopCustomAgent[]): DesktopCustomAgent[] {
  return [...agents]
    .sort((left, right) => {
      if (left.builtIn !== right.builtIn) {
        return left.builtIn ? -1 : 1;
      }
      return left.displayName.localeCompare(right.displayName)
        || left.agentId.localeCompare(right.agentId);
    });
}

function sortStandaloneAgents(agents: DesktopCustomAgent[]): DesktopCustomAgent[] {
  return sortAgents(agents.filter((agent) => agent.standalone));
}

function displayAgentName(agent: DesktopCustomAgent): string {
  return agent.displayName.trim() || agent.agentId;
}

function displayTeamName(team: DesktopTeam): string {
  return team.displayName.trim() || team.teamId;
}

export function formatAgentOptionLabel(
  agent: DesktopCustomAgent,
  style: AgentLabelStyle = "target",
): string {
  const displayName = displayAgentName(agent);
  return style === "display" || displayName.trim() === agent.agentId.trim()
    ? displayName
    : `${displayName} (${agent.agentId})`;
}

export function formatTeamOptionLabel(
  team: DesktopTeam,
  style: TeamLabelStyle = "target",
): string {
  const displayName = displayTeamName(team);
  if (style === "display") {
    return displayName;
  }
  return displayName.trim() === team.teamId.trim()
    ? `${displayName} (team)`
    : `${displayName} (${team.teamId}, team)`;
}

function toAgentPickerOption(
  agent: DesktopCustomAgent,
  labelStyle: AgentLabelStyle,
): AgentPickerOption {
  return {
    id: agent.agentId,
    label: formatAgentOptionLabel(agent, labelStyle),
    kind: agent.builtIn ? "builtin" : "agent",
    avatarDataUrl: agent.avatarDataUrl,
    providerType: agent.providerType,
  };
}

function toTeamPickerOption(
  team: DesktopTeam,
  labelStyle: TeamLabelStyle,
  detail?: string,
): AgentPickerOption {
  return {
    id: team.teamId,
    label: formatTeamOptionLabel(team, labelStyle),
    kind: "team",
    avatarDataUrl: team.avatarDataUrl,
    detail,
  };
}

export function buildAgentPickerOptions(
  agents: DesktopCustomAgent[],
  options: {
    excludeAgentIds?: ReadonlySet<string>;
    labelStyle?: AgentLabelStyle;
    standaloneOnly?: boolean;
  } = {},
): AgentPickerOption[] {
  const { excludeAgentIds, labelStyle = "display", standaloneOnly = false } = options;
  const sortedAgents = standaloneOnly ? sortStandaloneAgents(agents) : sortAgents(agents);
  return sortedAgents
    .filter((agent) => !excludeAgentIds?.has(agent.agentId))
    .map((agent) => toAgentPickerOption(agent, labelStyle));
}

export function buildStandaloneAgentOptions(
  agents: DesktopCustomAgent[],
  options: {
    excludeAgentIds?: ReadonlySet<string>;
    labelStyle?: AgentLabelStyle;
  } = {},
): AgentPickerOption[] {
  return buildAgentPickerOptions(agents, { ...options, standaloneOnly: true });
}

export function buildTeamOptions(
  teams: DesktopTeam[],
  options: {
    detail?: string | ((team: DesktopTeam) => string | undefined);
    labelStyle?: TeamLabelStyle;
  } = {},
): AgentPickerOption[] {
  const { detail, labelStyle = "display" } = options;
  return [...teams]
    .sort(byDisplayName)
    .map((team) => toTeamPickerOption(
      team,
      labelStyle,
      typeof detail === "function" ? detail(team) : detail,
    ));
}

export function buildAgentAndTeamOptions(
  agents: DesktopCustomAgent[],
  teams: DesktopTeam[],
  options: {
    agentLabelStyle?: AgentLabelStyle;
    teamDetail?: string;
    teamLabelStyle?: TeamLabelStyle;
    teamsFirst?: boolean;
  } = {},
): AgentPickerOption[] {
  const agentOptions = buildStandaloneAgentOptions(agents, {
    labelStyle: options.agentLabelStyle ?? "display",
  });
  const teamOptions = buildTeamOptions(teams, {
    detail: options.teamDetail,
    labelStyle: options.teamLabelStyle ?? "display",
  });
  return options.teamsFirst
    ? [...teamOptions, ...agentOptions]
    : [...agentOptions, ...teamOptions];
}

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

  const builtInAgents = buildStandaloneAgentOptions(
    agents.filter((agent) => agent.builtIn),
    { labelStyle: "display" },
  );
  const customAgents = buildStandaloneAgentOptions(
    agents.filter((agent) => !agent.builtIn),
    { excludeAgentIds: teamLeaderIds, labelStyle: "display" },
  );
  const teamOptions = buildTeamOptions(teams, {
    detail: (team) => {
      const leaderLabel = agentNameById.get(team.leaderAgentId) || team.leaderAgentId;
      return `Lead: ${leaderLabel}`;
    },
    labelStyle: "display",
  });

  return [...builtInAgents, ...customAgents, ...teamOptions];
}

export function groupAgentOptions(options: ComposerAgentOption[]) {
  return {
    builtin: options.filter((o) => o.kind === "builtin"),
    agent: options.filter((o) => o.kind === "agent"),
    team: options.filter((o) => o.kind === "team"),
  };
}

export function buildAgentTargetOptions(
  agents: DesktopCustomAgent[],
  teams: DesktopTeam[],
  options: { teamsFirst?: boolean } = {},
): AgentTargetOption[] {
  const toTarget = (option: AgentPickerOption): AgentTargetOption => ({
    ...option,
    value: option.id,
  });
  const allOptions = buildAgentAndTeamOptions(agents, teams, {
    agentLabelStyle: "target",
    teamDetail: "Team",
    teamLabelStyle: "target",
    teamsFirst: options.teamsFirst,
  });
  return allOptions.map(toTarget);
}
