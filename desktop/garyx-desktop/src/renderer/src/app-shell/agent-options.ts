import type {
  DesktopApiProviderType,
  DesktopCustomAgent,
  DesktopProviderIconDescriptor,
} from "@shared/contracts";

export type AgentOptionKind = "builtin" | "agent";

export type AgentPickerOption = {
  id: string;
  label: string;
  kind: AgentOptionKind;
  avatarDataUrl?: string | null;
  detail?: string;
  providerIcon?: DesktopProviderIconDescriptor | null;
  providerType?: DesktopApiProviderType;
};

export type ComposerAgentOption = AgentPickerOption;

export type AgentTargetOption = AgentPickerOption & {
  value: string;
};

type AgentLabelStyle = "display" | "target";

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

export function formatAgentOptionLabel(
  agent: DesktopCustomAgent,
  style: AgentLabelStyle = "target",
): string {
  const displayName = displayAgentName(agent);
  return style === "display" || displayName.trim() === agent.agentId.trim()
    ? displayName
    : `${displayName} (${agent.agentId})`;
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
    providerIcon: agent.providerIcon,
    providerType: agent.providerType,
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
    .filter((agent) => agent.enabled)
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

/**
 * Build the flat agent-option list shown in new-thread pickers: built-in
 * agents followed by custom standalone agents.
 */
export function buildAgentOptions(
  agents: DesktopCustomAgent[],
): ComposerAgentOption[] {
  const builtInAgents = buildStandaloneAgentOptions(
    agents.filter((agent) => agent.builtIn),
    { labelStyle: "display" },
  );
  const customAgents = buildStandaloneAgentOptions(
    agents.filter((agent) => !agent.builtIn),
    { labelStyle: "display" },
  );

  return [...builtInAgents, ...customAgents];
}

export function groupAgentOptions(options: ComposerAgentOption[]) {
  return {
    builtin: options.filter((o) => o.kind === "builtin"),
    agent: options.filter((o) => o.kind === "agent"),
  };
}

export function buildAgentTargetOptions(
  agents: DesktopCustomAgent[],
): AgentTargetOption[] {
  return buildStandaloneAgentOptions(agents, { labelStyle: "target" }).map((option) => ({
    ...option,
    value: option.id,
  }));
}
