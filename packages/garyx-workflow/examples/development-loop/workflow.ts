import { agent, phase, schema, workflow, type WorkflowContext } from "@garyx/workflow";

const DEFAULT_CHILD_AGENT = process.env.GARYX_DEV_WORKFLOW_AGENT_ID || "claude";

type DevelopmentMode = "dry_run" | "plan_only" | "implement_review";
type TargetSurface = "mac_app" | "gateway" | "mobile" | "workflow" | "general";

type DevelopmentInput = {
  goal: string;
  mode: DevelopmentMode;
  targetSurface: TargetSurface;
  workspaceDir?: string;
  targetPaths: string[];
  constraints: string[];
  validationCommands: string[];
  childAgentId: string;
  plannerAgentId: string;
  implementerAgentId: string;
  reviewerAgentId: string;
};

type PlanStep = {
  title: string;
  details: string;
  targetPaths?: string[];
};

type PlanResult = {
  summary: string;
  approach: string;
  implementationSteps: PlanStep[];
  validationCommands: string[];
  filesLikelyTouched?: string[];
  risks: string[];
};

type ImplementationValidation = {
  command: string;
  status: "passed" | "failed" | "not_run";
  summary: string;
};

type ImplementationResult = {
  summary: string;
  changedFiles: string[];
  validation: ImplementationValidation[];
  readyForReview: boolean;
  blockers: string[];
};

type ReviewFinding = {
  severity: "blocker" | "major" | "minor";
  file?: string;
  line?: number;
  summary: string;
  recommendation: string;
};

type ReviewResult = {
  verdict: "approved" | "needs_changes";
  summary: string;
  findings: ReviewFinding[];
  requiredChanges: string[];
  validationGaps: string[];
};

const PlanSchema = schema<PlanResult>({
  type: "object",
  additionalProperties: false,
  required: ["summary", "approach", "implementationSteps", "validationCommands", "risks"],
  properties: {
    summary: { type: "string" },
    approach: { type: "string" },
    implementationSteps: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["title", "details"],
        properties: {
          title: { type: "string" },
          details: { type: "string" },
          targetPaths: { type: "array", items: { type: "string" } },
        },
      },
    },
    validationCommands: { type: "array", items: { type: "string" } },
    filesLikelyTouched: { type: "array", items: { type: "string" } },
    risks: { type: "array", items: { type: "string" } },
  },
});

const ImplementationSchema = schema<ImplementationResult>({
  type: "object",
  additionalProperties: false,
  required: ["summary", "changedFiles", "validation", "readyForReview", "blockers"],
  properties: {
    summary: { type: "string" },
    changedFiles: { type: "array", items: { type: "string" } },
    validation: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["command", "status", "summary"],
        properties: {
          command: { type: "string" },
          status: { type: "string", enum: ["passed", "failed", "not_run"] },
          summary: { type: "string" },
        },
      },
    },
    readyForReview: { type: "boolean" },
    blockers: { type: "array", items: { type: "string" } },
  },
});

const ReviewSchema = schema<ReviewResult>({
  type: "object",
  additionalProperties: false,
  required: ["verdict", "summary", "findings", "requiredChanges", "validationGaps"],
  properties: {
    verdict: { type: "string", enum: ["approved", "needs_changes"] },
    summary: { type: "string" },
    findings: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["severity", "summary", "recommendation"],
        properties: {
          severity: { type: "string", enum: ["blocker", "major", "minor"] },
          file: { type: "string" },
          line: { type: "integer" },
          summary: { type: "string" },
          recommendation: { type: "string" },
        },
      },
    },
    requiredChanges: { type: "array", items: { type: "string" } },
    validationGaps: { type: "array", items: { type: "string" } },
  },
});

await workflow({
  name: "Development Review Loop",
  description: "Plan, implement, and review a coding task with observable Garyx child agents.",
  output: developmentOutputText,
  phases: [
    {
      id: "dry-run",
      title: "Dry Run",
      detail: "Validate package wiring without launching child agents.",
    },
    {
      id: "plan",
      title: "Plan",
      detail: "Inspect the task and produce an implementation plan.",
    },
    {
      id: "implement",
      title: "Implement",
      detail: "Apply the plan in the workspace.",
    },
    {
      id: "review",
      title: "Review",
      detail: "Review the implementation and decide whether changes are required.",
    },
  ],
  async run(ctx) {
    const input = normalizeInput(ctx.input, ctx.workspaceDir);
    await ctx.log("development_loop.config", {
      mode: input.mode,
      targetSurface: input.targetSurface,
      workspaceDir: input.workspaceDir ?? null,
      targetPaths: input.targetPaths,
    });

    if (input.mode === "dry_run") {
      phase("Dry Run", "Validate package wiring without launching child agents");
      return dryRunResult(ctx, input);
    }

    phase("Plan", "Inspect the task and produce an implementation plan");
    const plan = await agent<PlanResult>(planPrompt(input), {
      label: "plan",
      agentId: input.plannerAgentId,
      phase: "Plan",
      workspaceDir: input.workspaceDir,
      schema: PlanSchema,
    });
    await ctx.log("development_loop.plan", {
      summary: plan.summary,
      steps: plan.implementationSteps.length,
      validationCommands: plan.validationCommands,
    });

    if (input.mode === "plan_only") {
      return {
        status: "planned",
        goal: input.goal,
        targetSurface: input.targetSurface,
        plan,
      };
    }

    phase("Implement", "Apply the plan in the workspace");
    const implementation = await agent<ImplementationResult>(implementationPrompt(input, plan), {
      label: "implement",
      agentId: input.implementerAgentId,
      phase: "Implement",
      workspaceDir: input.workspaceDir,
      schema: ImplementationSchema,
    });
    await ctx.log("development_loop.implementation", {
      changedFiles: implementation.changedFiles,
      readyForReview: implementation.readyForReview,
      blockers: implementation.blockers,
    });

    phase("Review", "Read-only review of the implementation");
    const review = await agent<ReviewResult>(reviewPrompt(input, plan, implementation), {
      label: "review",
      agentId: input.reviewerAgentId,
      phase: "Review",
      workspaceDir: input.workspaceDir,
      schema: ReviewSchema,
    });
    await ctx.log("development_loop.review", {
      verdict: review.verdict,
      findingCount: review.findings.length,
      requiredChanges: review.requiredChanges.length,
    });

    const approved = review.verdict === "approved" && !hasBlockingFindings(review);

    return {
      status: approved ? "approved" : "needs_changes",
      goal: input.goal,
      targetSurface: input.targetSurface,
      workspaceDir: input.workspaceDir,
      plan,
      implementation,
      review,
      approved,
    };
  },
});

function normalizeInput(raw: unknown, workspaceDir?: string): DevelopmentInput {
  const rawText = typeof raw === "string" ? raw.trim() : "";
  const input: Record<string, unknown> =
    raw && typeof raw === "object" && !Array.isArray(raw)
      ? (raw as Record<string, unknown>)
      : {};
  const goal = stringValue(input.goal) || rawText || stringValue(process.env.GARYX_WORKFLOW_ARGS);
  if (!goal) {
    throw new Error("development-loop workflow requires input.goal");
  }
  const childAgentId = stringValue(input.childAgentId) || DEFAULT_CHILD_AGENT;
  return {
    goal,
    mode: enumValue(input.mode, ["dry_run", "plan_only", "implement_review"], "implement_review"),
    targetSurface: enumValue(input.targetSurface, ["mac_app", "gateway", "mobile", "workflow", "general"], "general"),
    workspaceDir: stringValue(input.workspaceDir) || workspaceDir,
    targetPaths: stringList(input.targetPaths),
    constraints: stringList(input.constraints),
    validationCommands: stringList(input.validationCommands),
    childAgentId,
    plannerAgentId: stringValue(input.plannerAgentId) || childAgentId,
    implementerAgentId: stringValue(input.implementerAgentId) || childAgentId,
    reviewerAgentId: stringValue(input.reviewerAgentId) || childAgentId,
  };
}

function dryRunResult(ctx: WorkflowContext, input: DevelopmentInput) {
  const plan = {
    summary: "Dry run only: no child agents launched.",
    approach: "Validate file-backed workflow package loading, SDK context, input parsing, and workflow completion.",
    implementationSteps: [
      {
        title: "Load package",
        details: "Garyx loads garyx.workflow.json and runs workflow.ts as a Task-backed workflow.",
        targetPaths: ["packages/garyx-workflow/examples/development-loop"],
      },
      {
        title: "Expose context",
        details: "The SDK exposes ctx.input, workflow ids, workspace dir, and event logging.",
        targetPaths: [],
      },
    ],
    validationCommands: input.validationCommands,
    filesLikelyTouched: input.targetPaths,
    risks: ["Dry run does not validate provider child-agent behavior."],
  };
  return {
    status: "dry_run",
    goal: input.goal,
    targetSurface: input.targetSurface,
    workflowRunId: ctx.workflowRunId,
    workspaceDir: input.workspaceDir,
    plan,
  };
}

function planPrompt(input: DevelopmentInput) {
  return [
    "You are the planner in an observable Garyx development workflow.",
    "Read the local code around the requested change before proposing implementation steps.",
    "Return structured JSON only.",
    "",
    taskContext(input),
    "",
    surfaceGuidance(input.targetSurface),
    "",
    "Design a small, reviewable implementation plan. Include validation commands that are narrow but meaningful.",
    "Call out risks, likely files, and any assumptions that an implementer must verify before editing.",
  ].join("\n");
}

function implementationPrompt(input: DevelopmentInput, plan: PlanResult) {
  return [
    "You are the implementer in an observable Garyx development workflow.",
    "Implement the approved plan in the shared workspace. Keep changes tightly scoped and preserve unrelated user work.",
    "Run the validation commands that fit the touched area. Do not commit.",
    "Return structured JSON only.",
    "",
    taskContext(input),
    "",
    surfaceGuidance(input.targetSurface),
    "",
    "## Plan",
    JSON.stringify(plan, null, 2),
  ].join("\n");
}

function reviewPrompt(
  input: DevelopmentInput,
  plan: PlanResult,
  implementation: ImplementationResult,
) {
  return [
    "You are the reviewer for this workflow run. This is a read-only review.",
    "Prioritize correctness, regressions, missing validation, task-scope drift, and public-repo hygiene.",
    "Do not edit files. Inspect the actual diff and relevant code paths before judging.",
    "Return structured JSON only. Use verdict=approved only when there are no blocker or major findings.",
    "",
    taskContext(input),
    "",
    surfaceGuidance(input.targetSurface),
    "",
    "## Plan",
    JSON.stringify(plan, null, 2),
    "",
    "## Implementation report",
    JSON.stringify(implementation, null, 2),
  ].join("\n");
}

function taskContext(input: DevelopmentInput) {
  return [
    "## Task",
    input.goal,
    "",
    `Target surface: ${input.targetSurface}`,
    `Workspace: ${input.workspaceDir || "(use current workflow workspace)"}`,
    input.targetPaths.length ? `Target paths:\n${input.targetPaths.map((path) => `- ${path}`).join("\n")}` : "Target paths: not specified",
    input.constraints.length ? `Constraints:\n${input.constraints.map((item) => `- ${item}`).join("\n")}` : "Constraints: keep scope tight; preserve unrelated work",
    input.validationCommands.length
      ? `Requested validation:\n${input.validationCommands.map((command) => `- ${command}`).join("\n")}`
      : "Requested validation: choose the narrowest reliable validation for the touched area",
  ].join("\n");
}

function surfaceGuidance(targetSurface: TargetSurface) {
  if (targetSurface === "mac_app") {
    return [
      "## Mac app guidance",
      "- Treat the Mac app as the source of truth for workflow UI information architecture.",
      "- Inspect desktop/garyx-desktop and the gateway APIs before introducing UI state.",
      "- Do not touch iOS or mobile code unless the task explicitly requires it.",
      "- Prefer existing Electron, shadcn, and shared renderer patterns.",
      "- Validate with desktop-focused build or smoke commands when UI files change.",
    ].join("\n");
  }
  if (targetSurface === "workflow") {
    return [
      "## Workflow guidance",
      "- Workflow definitions are file-backed packages with garyx.workflow.json plus workflow.ts.",
      "- The database stores runs, events, and task linkage, not reusable definition source.",
      "- Validate with CLI definition install/get, task create, task get, and workflow events when possible.",
    ].join("\n");
  }
  if (targetSurface === "gateway") {
    return [
      "## Gateway guidance",
      "- Read gateway/router/bridge ownership boundaries before editing.",
      "- Gateway changes do not affect the running service until rebuilt and restarted.",
      "- Prefer focused Rust tests for the touched module.",
    ].join("\n");
  }
  if (targetSurface === "mobile") {
    return [
      "## Mobile guidance",
      "- Mobile may adapt layout but must not invent new top-level concepts.",
      "- Keep business-rule transformations in GaryxMobileCore with SwiftPM tests.",
    ].join("\n");
  }
  return "## General guidance\n- Read local code first, keep scope small, and run focused validation.";
}

function hasBlockingFindings(review: ReviewResult) {
  return review.findings.some((finding) => finding.severity === "blocker" || finding.severity === "major");
}

function developmentOutputText(result: unknown): string | undefined {
  const record = objectValue(result);
  if (!record) {
    return undefined;
  }
  const review = objectValue(record.review);
  const reviewSummary = stringValue(review?.summary);
  if (reviewSummary) {
    const verdict = stringValue(review?.verdict) || stringValue(record.status) || "completed";
    const requiredChanges = stringList(review?.requiredChanges);
    return [
      `### Review: ${verdict}`,
      reviewSummary,
      requiredChanges.length
        ? `Required changes:\n${requiredChanges.map((item) => `- ${item}`).join("\n")}`
        : "",
    ]
      .filter(Boolean)
      .join("\n\n");
  }
  const plan = objectValue(record.plan);
  const planSummary = stringValue(plan?.summary);
  if (planSummary) {
    return ["### Plan", planSummary].join("\n\n");
  }
  return stringValue(record.outputText) || stringValue(record.status);
}

function objectValue(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item) => typeof item === "string" && item.trim()).map((item) => item.trim()) : [];
}

function enumValue<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return typeof value === "string" && allowed.includes(value as T) ? (value as T) : fallback;
}
