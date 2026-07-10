import type {
  DesktopApiProviderType,
  DesktopCustomAgent,
  DesktopProviderIconDescriptor,
  DesktopTaskSummary,
  DesktopThreadSummary,
} from "@shared/contracts";

export type ThreadAvatarIdentity = {
  agentId: string | null;
  avatarDataUrl: string | null;
  kind: "builtin" | "agent";
  label: string;
  providerIcon?: DesktopProviderIconDescriptor | null;
  providerType?: DesktopApiProviderType | null;
};

export type ThreadAvatarCatalog = {
  agentById: ReadonlyMap<string, DesktopCustomAgent>;
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
): ThreadAvatarCatalog {
  return {
    agentById: new Map(agents.map((agent) => [agent.agentId, agent] as const)),
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
  thread: Pick<
    DesktopThreadSummary,
    "title" | "agentId"
  >,
  catalog: ThreadAvatarCatalog,
): ThreadAvatarIdentity {
  const threadAgentId = trimmed(thread.agentId);
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
  if (executor?.type === "agent") {
    return agentAvatarIdentity(executor.agentId, catalog);
  }

  if (task.assignee?.kind === "agent") {
    return agentAvatarIdentity(task.assignee.agentId, catalog);
  }
  if (task.assignee?.kind === "human") {
    return fallbackAvatarIdentity(`@${task.assignee.userId}`);
  }

  const runtimeAgentId = trimmed(task.runtimeAgentId);
  if (runtimeAgentId) {
    return agentAvatarIdentity(runtimeAgentId, catalog);
  }

  return fallbackAvatarIdentity("unassigned");
}
