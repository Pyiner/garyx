export type CustomAgentDeleteConfirmation = {
  agentId: string;
  displayName: string;
};

export type CustomAgentDeleteCandidate = {
  agentId: string;
  displayName?: string | null;
  builtIn?: boolean | null;
};

type RunCustomAgentDeleteConfirmationInput = {
  confirmation: CustomAgentDeleteConfirmation | null;
  deleteCustomAgent: (input: { agentId: string }) => Promise<void>;
  closeConfirmation: () => void;
  closeAgentDialog: () => void;
  loadData: () => Promise<void>;
  refreshAgentTargets?: () => Promise<void> | void;
};

export function customAgentDeleteConfirmationFor(
  agent: CustomAgentDeleteCandidate,
): CustomAgentDeleteConfirmation | null {
  if (agent.builtIn) {
    return null;
  }
  const agentId = agent.agentId.trim();
  if (!agentId) {
    return null;
  }
  return {
    agentId,
    displayName: agent.displayName?.trim() || agentId,
  };
}

export async function runCustomAgentDeleteConfirmation({
  confirmation,
  deleteCustomAgent,
  closeConfirmation,
  closeAgentDialog,
  loadData,
  refreshAgentTargets,
}: RunCustomAgentDeleteConfirmationInput): Promise<'cancelled' | 'deleted'> {
  if (!confirmation) {
    return 'cancelled';
  }
  await deleteCustomAgent({ agentId: confirmation.agentId });
  closeConfirmation();
  closeAgentDialog();
  await loadData();
  await refreshAgentTargets?.();
  return 'deleted';
}
