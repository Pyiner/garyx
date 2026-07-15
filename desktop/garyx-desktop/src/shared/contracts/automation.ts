export type DesktopAutomationSchedule =
  | {
      kind: "daily";
      time: string;
      weekdays: string[];
      timezone: string;
    }
  | {
      kind: "interval";
      hours: number;
    }
  | {
      kind: "once";
      at: string;
    };

export type DesktopAutomationStatus = "success" | "failed" | "skipped";
export type DesktopAutomationAgentResolution =
  | "resolved"
  | "follow_thread"
  | "target_missing";
export type DesktopAutomationValidationState = "valid" | "invalid";

export interface DesktopAutomationSummary {
  id: string;
  label: string;
  prompt: string;
  agentId: string | null;
  agentResolution: DesktopAutomationAgentResolution;
  effectiveAgentId: string | null;
  enabled: boolean;
  workspacePath: string;
  // Existing thread this automation pushes scheduled prompts into, when set.
  targetThreadId: string;
  // Latest execution thread for this automation. Empty until it has run at least once.
  threadId: string;
  nextRun: string;
  lastRunAt?: string | null;
  lastStatus: DesktopAutomationStatus;
  unreadHintTimestamp?: string | null;
  schedule: DesktopAutomationSchedule;
  validationState: DesktopAutomationValidationState;
  validationError?: string | null;
}

export interface DesktopAutomationActivityEntry {
  runId: string;
  status: DesktopAutomationStatus;
  startedAt: string;
  finishedAt?: string | null;
  durationMs?: number | null;
  excerpt?: string | null;
  threadId: string;
}

export interface DesktopAutomationActivityFeed {
  automationId: string;
  // Latest execution thread represented by this feed page. Empty if there is no activity yet.
  threadId: string;
  count: number;
  items: DesktopAutomationActivityEntry[];
}

export interface CreateAutomationInput {
  label: string;
  prompt: string;
  /// Omitted for thread-bound automations: the thread's own agent handles
  /// each run, so an automation-level agent choice does not apply.
  agentId?: string;
  workspacePath?: string;
  targetThreadId?: string | null;
  schedule: DesktopAutomationSchedule;
}

export interface UpdateAutomationInput {
  automationId: string;
  label?: string;
  prompt?: string;
  agentId?: string;
  workspacePath?: string;
  targetThreadId?: string | null;
  schedule?: DesktopAutomationSchedule;
  enabled?: boolean;
}

export interface DeleteAutomationInput {
  automationId: string;
}

export interface RunAutomationNowInput {
  automationId: string;
}

export interface SelectAutomationInput {
  automationId: string | null;
}

export interface MarkAutomationSeenInput {
  automationId: string;
  seenAt: string | null;
}
