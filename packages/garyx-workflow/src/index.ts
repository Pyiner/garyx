import { AsyncLocalStorage } from "node:async_hooks";
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

export interface JsonSchema<T = unknown> {
  readonly type?: string;
  readonly properties?: Record<string, JsonSchema>;
  readonly items?: JsonSchema;
  readonly required?: string[];
  readonly additionalProperties?: boolean | JsonSchema;
  readonly enum?: readonly JsonValue[];
  readonly __type?: T;
  readonly [key: string]: unknown;
}

export interface WorkflowSchema<T> {
  readonly json: JsonSchema<T>;
}

export type SchemaInput<T = unknown> = WorkflowSchema<T> | JsonSchema<T>;

type MutableJsonSchema = { -readonly [K in keyof JsonSchema]: JsonSchema[K] };

export type WorkflowPhaseInput =
  | string
  | {
      readonly id?: string;
      readonly title: string;
      readonly detail?: string;
    };

export interface WorkflowPhaseDefinition {
  readonly id?: string;
  readonly title: string;
  readonly detail?: string;
  readonly index: number;
}

export type WorkflowOutputSelector<T> = (
  result: T,
  ctx: WorkflowContext,
) => string | null | undefined;

export interface WorkflowRunOptions<T = unknown> {
  readonly name?: string;
  readonly description?: string;
  readonly phases?: readonly WorkflowPhaseInput[];
  readonly output?: WorkflowOutputSelector<T>;
  readonly parentRunId?: string;
  readonly workspaceDir?: string;
  readonly input?: JsonValue;
  readonly workflowDefinitionId?: string;
  readonly workflowDefinitionVersion?: number;
  readonly workflowDefinitionSnapshot?: JsonValue;
  readonly gatewayUrl?: string;
  readonly gatewayToken?: string;
  readonly signal?: AbortSignal;
}

export interface AgentOptions<T = string> {
  readonly label?: string;
  readonly agentId?: string;
  readonly workspaceDir?: string;
  readonly schema?: WorkflowSchema<T> | JsonSchema<T>;
  readonly optional?: boolean;
  readonly phase?: string;
  readonly phaseTitle?: string;
  readonly phaseIndex?: number;
  readonly binding?: string;
}

export interface WorkflowProgram<T> {
  readonly name?: string;
  readonly description?: string;
  readonly phases?: readonly WorkflowPhaseInput[];
  readonly output?: WorkflowOutputSelector<T>;
  run(ctx: WorkflowContext): Promise<T> | T;
}

export interface WorkflowRunResult<T> {
  readonly workflowRunId: string;
  /** @deprecated Use workflowRunId. */
  readonly workflowId: string;
  readonly result: T;
  readonly outputText?: string;
}

export interface WorkflowContext {
  readonly workflowRunId: string;
  /** @deprecated Use workflowRunId. */
  readonly workflowId: string;
  readonly workspaceDir?: string;
  readonly input: JsonValue;
  readonly phases: readonly WorkflowPhaseDefinition[];
  readonly client: GaryxWorkflowClient;
  readonly signal?: AbortSignal;
  log(messageOrEventType: string, payload?: JsonValue): Promise<void>;
  agent<T = string>(prompt: string, options?: AgentOptions<T>): Promise<T>;
  phase(title: string, detail?: string): void;
  pipeline<T>(items: Iterable<T> | Promise<Iterable<T>>, ...stages: PipelineStage[]): Promise<unknown[]>;
  parallel<T>(
    tasks: Array<(() => Promise<T> | T) | Promise<T> | T>,
    options?: { concurrency?: number },
  ): Promise<T[]>;
}

export type PipelineStage = (input: any, index: number) => unknown | Promise<unknown>;

export type PhaseAgentOptions<T = string> = Omit<
  AgentOptions<T>,
  "label" | "phase" | "phaseTitle" | "phaseIndex"
>;

export interface WorkflowPhaseHandle {
  readonly name: string;
  start(detail?: string): WorkflowPhaseHandle;
  agent<T = string>(label: string, prompt: string, options?: PhaseAgentOptions<T>): Promise<T | null>;
  parallel<T>(tasks: Array<() => Promise<T> | T>, options?: { concurrency?: number }): Promise<T[]>;
  pipeline<T>(
    items: Iterable<T> | Promise<Iterable<T>>,
    ...stages: PipelineStage[]
  ): Promise<unknown[]>;
}

export interface WorkflowFlow {
  readonly ctx: WorkflowContext;
  readonly input: JsonValue;
  log(messageOrEventType: string, payload?: JsonValue): Promise<void>;
  logLine(message: string, payload?: JsonObject): Promise<void>;
  phase(name: string): WorkflowPhaseHandle;
}

export interface WorkflowDefinition<T> {
  readonly name?: string;
  readonly description?: string;
  readonly phases?: readonly WorkflowPhaseInput[];
  readonly agent?: string;
  readonly output?: WorkflowOutputSelector<T>;
  run(flow: WorkflowFlow): Promise<T> | T;
}

interface WorkflowStore {
  ctx: WorkflowContext;
  nextOrderIndex: number;
  nextPhaseIndex: number;
  phasePlan: WorkflowPhaseDefinition[];
  currentPhaseTitle?: string;
  currentPhaseIndex?: number;
  phaseIndexes: Map<string, number>;
  phaseDisplayTitles: Map<string, string>;
}

const activeWorkflow = new AsyncLocalStorage<WorkflowStore>();

export class GaryxWorkflowError extends Error {
  constructor(
    message: string,
    readonly status?: number,
    readonly payload?: unknown,
  ) {
    super(message);
    this.name = "GaryxWorkflowError";
  }
}

export class GaryxWorkflowClient {
  readonly gatewayUrl: string;
  readonly gatewayToken?: string;

  constructor(options: { gatewayUrl?: string; gatewayToken?: string } = {}) {
    this.gatewayUrl = normalizeGatewayUrl(
      options.gatewayUrl ?? env("GARYX_GATEWAY_URL") ?? "http://127.0.0.1:31337",
    );
    this.gatewayToken = options.gatewayToken ?? env("GARYX_GATEWAY_TOKEN");
  }

  async startWorkflow(options: Omit<WorkflowRunOptions, "output"> & { name?: string; description?: string }) {
    const workflowThreadId = env("GARYX_WORKFLOW_THREAD_ID") ?? env("GARYX_WORKFLOW_RUN_ID");
    const taskId = env("GARYX_TASK_ID");
    const taskThreadId = env("GARYX_TASK_THREAD_ID");
    const taskRef = taskId && taskThreadId ? { taskId, taskThreadId } : {};
    return this.request("/api/workflows/sdk", {
      method: "POST",
      signal: options.signal,
      body: {
        workflowRunId: workflowThreadId,
        workflowId: workflowThreadId,
        ...taskRef,
        workflowDefinitionId: options.workflowDefinitionId ?? env("GARYX_WORKFLOW_DEFINITION_ID"),
        workflowDefinitionVersion:
          options.workflowDefinitionVersion ?? numberEnv("GARYX_WORKFLOW_DEFINITION_VERSION"),
        workflowDefinitionSnapshot:
          options.workflowDefinitionSnapshot ?? jsonEnv("GARYX_WORKFLOW_DEFINITION_SNAPSHOT"),
        input: options.input ?? jsonEnv("GARYX_WORKFLOW_INPUT_JSON") ?? null,
        parentThreadId: env("GARYX_PARENT_THREAD_ID"),
        parentRunId: options.parentRunId ?? env("GARYX_PARENT_RUN_ID"),
        name: options.name,
        description: options.description,
        phases: normalizePhaseDefinitions(options.phases),
        workspaceDir: options.workspaceDir ?? env("GARYX_WORKSPACE_DIR"),
        createdBy: "typescript-sdk",
      },
    });
  }

  async log(workflowRunId: string, eventType: string, payload: JsonValue, signal?: AbortSignal) {
    await this.request(`/api/workflows/${encodeURIComponent(workflowRunId)}/events`, {
      method: "POST",
      signal,
      body: { eventType, payload },
    });
  }

  async runAgent<T>(
    workflowRunId: string,
    input: {
      prompt: string;
      label?: string;
      agentId?: string;
      workspaceDir?: string;
      schema?: WorkflowSchema<T> | JsonSchema<T>;
      optional?: boolean;
      phaseTitle?: string;
      phaseIndex?: number;
      binding?: string;
      orderIndex: number;
      signal?: AbortSignal;
    },
  ): Promise<T> {
    const schema = unwrapSchema(input.schema);
    const payload = await this.request(
      `/api/workflows/${encodeURIComponent(workflowRunId)}/agents`,
      {
        method: "POST",
        signal: input.signal,
        body: {
          prompt: input.prompt,
          label: input.label,
          agentId: input.agentId,
          workspaceDir: input.workspaceDir,
          schema,
          optional: input.optional,
          phaseTitle: input.phaseTitle,
          phaseIndex: input.phaseIndex,
          binding: input.binding,
          orderIndex: input.orderIndex,
        },
      },
    );
    if (payload.failed && !input.optional) {
      throw new GaryxWorkflowError(payload.error ?? "Garyx workflow child failed", undefined, payload);
    }
    return (payload.failed ? null : payload.result) as T;
  }

  async finishWorkflow(
    workflowRunId: string,
    input: { status?: "succeeded" | "failed" | "cancelled"; result?: JsonValue; outputText?: string; error?: string },
    signal?: AbortSignal,
  ) {
    return this.request(`/api/workflows/${encodeURIComponent(workflowRunId)}/finish`, {
      method: "POST",
      signal,
      body: input,
    });
  }

  private async request(
    path: string,
    options: { method: "GET" | "POST"; body?: unknown; signal?: AbortSignal },
  ): Promise<any> {
    const headers: Record<string, string> = { accept: "application/json" };
    let body: string | undefined;
    if (options.body !== undefined) {
      headers["content-type"] = "application/json";
      body = JSON.stringify(options.body);
    }
    if (this.gatewayToken) {
      headers.authorization = `Bearer ${this.gatewayToken}`;
    }
    const response = await requestText(`${this.gatewayUrl}${path}`, {
      method: options.method,
      headers,
      body,
      signal: options.signal,
    });
    const text = response.text;
    const payload = text ? safeJson(text) : undefined;
    if (response.status < 200 || response.status >= 300) {
      throw new GaryxWorkflowError(
        payload?.message ?? `Garyx gateway request failed with ${response.status}`,
        response.status,
        payload,
      );
    }
    return payload;
  }
}

export function schema<T>(json: JsonSchema<T>): WorkflowSchema<T> {
  return { json };
}

export const s = {
  string(extra: Record<string, JsonValue | undefined> = {}): JsonSchema<string> {
    return stripUndefined({ type: "string", ...extra });
  },
  number(extra: Record<string, JsonValue | undefined> = {}): JsonSchema<number> {
    return stripUndefined({ type: "number", ...extra });
  },
  integer(extra: Record<string, JsonValue | undefined> = {}): JsonSchema<number> {
    return stripUndefined({ type: "integer", ...extra });
  },
  boolean(): JsonSchema<boolean> {
    return { type: "boolean" };
  },
  enum<const T extends readonly JsonPrimitive[]>(values: T): JsonSchema<T[number]> {
    return { type: typeof values[0] === "number" ? "number" : typeof values[0], enum: values };
  },
  array<T>(items: SchemaInput<T>, extra: Record<string, JsonValue | undefined> = {}): JsonSchema<T[]> {
    return stripUndefined({ type: "array", items: schemaJson(items), ...extra });
  },
  object<T = JsonObject>(
    properties: Record<string, SchemaInput<any>>,
    required = Object.keys(properties),
    extra: Record<string, JsonValue | undefined> = {},
  ): WorkflowSchema<T> {
    return schema<T>(
      stripUndefined({
        type: "object",
        additionalProperties: false,
        required,
        properties: Object.fromEntries(
          Object.entries(properties).map(([key, value]) => [key, schemaJson(value)]),
        ),
        ...extra,
      }) as JsonSchema<T>,
    );
  },
};

export async function defineWorkflow<T>(
  definition: WorkflowDefinition<T>,
  options: WorkflowRunOptions<T> = {},
): Promise<WorkflowRunResult<T>> {
  return workflow(
    {
      name: definition.name,
      description: definition.description,
      phases: definition.phases,
      run(ctx) {
        return definition.run(createWorkflowFlow(ctx, definition.agent));
      },
      output: definition.output,
    },
    options,
  );
}

export async function workflow<T>(
  definition: WorkflowProgram<T>,
  options: WorkflowRunOptions<T> = {},
): Promise<WorkflowRunResult<T>> {
  const { output: optionOutput, ...runOptions } = options;
  const client = new GaryxWorkflowClient(runOptions);
  const workflowInput = runOptions.input ?? jsonEnv("GARYX_WORKFLOW_INPUT_JSON") ?? null;
  const phasePlan = normalizePhaseDefinitions(runOptions.phases ?? definition.phases);
  const startPayload = await client.startWorkflow({
    ...runOptions,
    input: workflowInput,
    name: runOptions.name ?? definition.name,
    description: runOptions.description ?? definition.description,
    phases: phasePlan,
  });
  const workflowRunId = String(startPayload.workflow.workflowRunId ?? startPayload.workflow.workflowId);
  const workflowId = workflowRunId;
  const envSnapshot = snapshotEnv(workflowEnvNames);
  setEnv("GARYX_WORKFLOW_ID", workflowId);
  setEnv("GARYX_WORKFLOW_RUN_ID", workflowRunId);
  setEnv("GARYX_WORKFLOW_THREAD_ID", workflowRunId);
  setOptionalEnv(
    "GARYX_WORKSPACE_DIR",
    startPayload.workflow.workspaceDir ? String(startPayload.workflow.workspaceDir) : undefined,
  );

  const store: WorkflowStore = {
    nextOrderIndex: 0,
    nextPhaseIndex: phasePlan.length,
    phasePlan,
    phaseIndexes: phaseIndexMap(phasePlan),
    phaseDisplayTitles: phaseDisplayTitleMap(phasePlan),
    ctx: undefined as unknown as WorkflowContext,
  };
  const ctx: WorkflowContext = {
    workflowId,
    workflowRunId,
    workspaceDir: startPayload.workflow.workspaceDir ?? runOptions.workspaceDir,
    input: startPayload.workflow.input ?? workflowInput,
    phases: phasePlan,
    client,
    signal: runOptions.signal,
    log: (messageOrEventType, payload) => log(messageOrEventType, payload),
    agent: (prompt, agentOptions) => agent(prompt, agentOptions),
    phase: (title, detail) => phase(title, detail),
    pipeline: (items, ...stages) => pipeline(items, ...stages),
    parallel: (tasks, parallelOptions) => parallel(tasks, parallelOptions),
  };
  store.ctx = ctx;

  try {
    const result = await activeWorkflow.run(store, () => definition.run(ctx));
    const resultJson = toJsonValue(result);
    const outputText = outputTextFromResult(
      result,
      resultJson,
      ctx,
      optionOutput ?? definition.output,
    );
    await client.finishWorkflow(
      workflowRunId,
      { result: resultJson, status: "succeeded", outputText },
      runOptions.signal,
    );
    return { workflowId, workflowRunId, result, outputText };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    await client.finishWorkflow(workflowRunId, { status: "failed", error: message }, runOptions.signal).catch(() => {});
    throw error;
  } finally {
    restoreEnv(envSnapshot);
  }
}

function createWorkflowFlow(ctx: WorkflowContext, defaultAgentId?: string): WorkflowFlow {
  return {
    ctx,
    input: ctx.input,
    log: (messageOrEventType, payload) => ctx.log(messageOrEventType, payload),
    logLine: (message, payload) => ctx.log(message, payload),
    phase(name) {
      return createWorkflowPhaseHandle(ctx, name, defaultAgentId);
    },
  };
}

function createWorkflowPhaseHandle(
  ctx: WorkflowContext,
  name: string,
  defaultAgentId?: string,
): WorkflowPhaseHandle {
  return {
    name,
    start(detail) {
      ctx.phase(name, detail);
      return this;
    },
    agent<T = string>(label: string, prompt: string, options: PhaseAgentOptions<T> = {}) {
      return ctx.agent<T>(prompt, {
        ...options,
        label,
        phase: name,
        agentId: options.agentId ?? defaultAgentId,
        workspaceDir: options.workspaceDir ?? ctx.workspaceDir,
      }) as Promise<T | null>;
    },
    parallel<T>(tasks: Array<() => Promise<T> | T>, options?: { concurrency?: number }) {
      return ctx.parallel(tasks, options);
    },
    pipeline<T>(items: Iterable<T> | Promise<Iterable<T>>, ...stages: PipelineStage[]) {
      return ctx.pipeline(items, ...stages);
    },
  };
}

export async function log(messageOrEventType: string, payload?: JsonValue): Promise<void> {
  const store = requireActiveWorkflow();
  const eventType = messageOrEventType.includes(".") ? messageOrEventType : "workflow.log";
  const body = eventType === "workflow.log" ? { message: messageOrEventType, ...(asObject(payload) ?? {}) } : (payload ?? {});
  await store.ctx.client.log(store.ctx.workflowRunId, eventType, toJsonValue(body), store.ctx.signal);
}

export function phase(title: string, detail?: string): void {
  const store = requireActiveWorkflow();
  const resolved = phaseForTitle(store, title);
  const phaseIndex = resolved.index;
  const phaseTitle = resolved.title;
  store.currentPhaseTitle = phaseTitle;
  store.currentPhaseIndex = phaseIndex;
  void store.ctx.client.log(
    store.ctx.workflowRunId,
    "workflow.phase_started",
    toJsonValue({ title: phaseTitle, detail, phaseIndex }),
    store.ctx.signal,
  ).catch(() => {});
}

export async function agent<T = string>(prompt: string, options: AgentOptions<T> = {}): Promise<T> {
  const store = requireActiveWorkflow();
  const orderIndex = store.nextOrderIndex++;
  const rawPhaseTitle = options.phaseTitle ?? options.phase ?? store.currentPhaseTitle;
  const resolvedPhase = rawPhaseTitle ? phaseForTitle(store, rawPhaseTitle) : undefined;
  const phaseTitle = resolvedPhase?.title ?? store.currentPhaseTitle;
  const phaseIndex =
    options.phaseIndex ?? resolvedPhase?.index ?? store.currentPhaseIndex;
  return store.ctx.client.runAgent<T>(store.ctx.workflowRunId, {
    ...options,
    phaseTitle,
    phaseIndex,
    prompt,
    workspaceDir: options.workspaceDir ?? store.ctx.workspaceDir,
    orderIndex,
    signal: store.ctx.signal,
  });
}

export async function parallel<T>(
  tasks: Array<(() => Promise<T> | T) | Promise<T> | T>,
  options: { concurrency?: number } = {},
): Promise<T[]> {
  const concurrency = Math.max(1, Math.floor((options.concurrency ?? tasks.length) || 1));
  const results = new Array<T>(tasks.length);
  let cursor = 0;
  async function worker() {
    for (;;) {
      const index = cursor++;
      if (index >= tasks.length) {
        return;
      }
      const task = tasks[index];
      results[index] = await (typeof task === "function" ? (task as () => Promise<T> | T)() : task);
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, tasks.length) }, worker));
  return results;
}

export async function pipeline<T>(
  items: Iterable<T> | Promise<Iterable<T>>,
  ...stages: PipelineStage[]
): Promise<unknown[]> {
  const resolvedItems = Array.from(await items);
  async function runStages(input: unknown, index: number): Promise<unknown> {
    let value = input;
    for (const stage of stages) {
      value = await stage(value, index);
    }
    return value;
  }
  return Promise.all(resolvedItems.map((item, index) => runStages(item, index)));
}

function requireActiveWorkflow(): WorkflowStore {
  const store = activeWorkflow.getStore();
  if (!store) {
    throw new GaryxWorkflowError("Garyx workflow SDK call must run inside workflow({ run })");
  }
  return store;
}

function unwrapSchema<T>(value?: WorkflowSchema<T> | JsonSchema<T>): JsonSchema<T> | undefined {
  if (!value) {
    return undefined;
  }
  let raw: JsonSchema<T>;
  if ("json" in value && typeof value.json === "object" && value.json !== null) {
    raw = value.json as JsonSchema<T>;
  } else {
    raw = value as JsonSchema<T>;
  }
  return normalizeSchema(raw) as JsonSchema<T>;
}

function schemaJson<T>(value: SchemaInput<T>): JsonSchema<T> {
  return ("json" in value ? (value as WorkflowSchema<T>).json : value) as JsonSchema<T>;
}

function normalizeSchema(value: JsonSchema): JsonSchema {
  const normalized: MutableJsonSchema = { ...value };
  if (!normalized.type && Array.isArray(normalized.enum)) {
    const inferred = inferEnumType(normalized.enum);
    if (inferred) {
      normalized.type = inferred;
    }
  }
  if (normalized.properties && typeof normalized.properties === "object") {
    normalized.properties = Object.fromEntries(
      Object.entries(normalized.properties).map(([key, schemaValue]) => [key, normalizeSchema(schemaValue)]),
    );
  }
  if (normalized.items && typeof normalized.items === "object" && !Array.isArray(normalized.items)) {
    normalized.items = normalizeSchema(normalized.items);
  }
  if (
    normalized.additionalProperties &&
    typeof normalized.additionalProperties === "object" &&
    !Array.isArray(normalized.additionalProperties)
  ) {
    normalized.additionalProperties = normalizeSchema(normalized.additionalProperties as JsonSchema);
  }
  return normalized;
}

function normalizePhaseDefinitions(
  phases?: readonly WorkflowPhaseInput[],
): WorkflowPhaseDefinition[] {
  if (!phases?.length) {
    return [];
  }
  const normalized: WorkflowPhaseDefinition[] = [];
  const seen = new Set<string>();
  for (const phase of phases) {
    const rawTitle = typeof phase === "string" ? phase : phase.title;
    const title = rawTitle.trim();
    if (!title || seen.has(title)) {
      continue;
    }
    seen.add(title);
    const id = typeof phase === "string" ? undefined : phase.id?.trim();
    const detail = typeof phase === "string" ? undefined : phase.detail?.trim();
    normalized.push({
      ...(id ? { id } : {}),
      title,
      ...(detail ? { detail } : {}),
      index: normalized.length,
    });
  }
  return normalized;
}

function phaseIndexMap(phases: readonly WorkflowPhaseDefinition[]): Map<string, number> {
  const map = new Map<string, number>();
  for (const phase of phases) {
    map.set(phase.title, phase.index);
    if (phase.id) {
      map.set(phase.id, phase.index);
    }
  }
  return map;
}

function phaseDisplayTitleMap(phases: readonly WorkflowPhaseDefinition[]): Map<string, string> {
  const map = new Map<string, string>();
  for (const phase of phases) {
    map.set(phase.title, phase.title);
    if (phase.id) {
      map.set(phase.id, phase.title);
    }
  }
  return map;
}

function inferEnumType(values: readonly JsonValue[]): string | undefined {
  if (values.length === 0) {
    return undefined;
  }
  const primitiveTypes = new Set(
    values.map((value) => {
      if (value === null || typeof value === "object") {
        return undefined;
      }
      return typeof value;
    }),
  );
  if (primitiveTypes.size !== 1 || primitiveTypes.has(undefined)) {
    return undefined;
  }
  const only = [...primitiveTypes][0];
  return only === "number" ? "number" : only;
}

function phaseForTitle(store: WorkflowStore, title: string): { title: string; index: number } {
  const normalized = title.trim();
  if (!normalized) {
    throw new GaryxWorkflowError("phase title is required");
  }
  let phaseIndex = store.phaseIndexes.get(normalized);
  if (phaseIndex === undefined) {
    phaseIndex = store.nextPhaseIndex++;
    store.phaseIndexes.set(normalized, phaseIndex);
    store.phaseDisplayTitles.set(normalized, normalized);
  }
  return {
    title: store.phaseDisplayTitles.get(normalized) ?? normalized,
    index: phaseIndex,
  };
}

function normalizeGatewayUrl(value: string): string {
  return value.replace(/\/+$/, "");
}

function safeJson(text: string): any {
  try {
    return JSON.parse(text);
  } catch {
    return { text };
  }
}

function toJsonValue(value: unknown): JsonValue {
  if (value === undefined) {
    return null;
  }
  return JSON.parse(JSON.stringify(value)) as JsonValue;
}

function outputTextFromResult<T>(
  result: T,
  value: JsonValue,
  ctx: WorkflowContext,
  selector?: WorkflowOutputSelector<T>,
): string | undefined {
  if (selector) {
    return normalizeOutputText(selector(result, ctx));
  }
  if (typeof value === "string") {
    return normalizeOutputText(value);
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  for (const key of ["outputText", "output", "markdown"]) {
    const candidate = value[key];
    const normalized = normalizeOutputText(candidate);
    if (normalized) {
      return normalized;
    }
  }
  return undefined;
}

function normalizeOutputText(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function asObject(value: JsonValue | undefined): JsonObject | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value;
}

function stripUndefined<T extends Record<string, unknown>>(value: T): T {
  return Object.fromEntries(Object.entries(value).filter(([, entry]) => entry !== undefined)) as T;
}

function env(name: string): string | undefined {
  return typeof process === "undefined" ? undefined : process.env[name];
}

function numberEnv(name: string): number | undefined {
  const raw = env(name);
  if (!raw) {
    return undefined;
  }
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function jsonEnv(name: string): JsonValue | undefined {
  const raw = env(name);
  if (!raw) {
    return undefined;
  }
  try {
    return JSON.parse(raw) as JsonValue;
  } catch {
    return undefined;
  }
}

function setEnv(name: string, value: string): void {
  if (typeof process !== "undefined") {
    process.env[name] = value;
  }
}

function setOptionalEnv(name: string, value: string | undefined): void {
  if (typeof process === "undefined") {
    return;
  }
  if (value === undefined) {
    delete process.env[name];
  } else {
    process.env[name] = value;
  }
}

const workflowEnvNames = [
  "GARYX_WORKFLOW_ID",
  "GARYX_WORKFLOW_RUN_ID",
  "GARYX_WORKFLOW_THREAD_ID",
  "GARYX_WORKSPACE_DIR",
] as const;

type EnvSnapshot = Map<(typeof workflowEnvNames)[number], string | undefined>;

function snapshotEnv(names: readonly (typeof workflowEnvNames)[number][]): EnvSnapshot {
  const snapshot: EnvSnapshot = new Map();
  if (typeof process === "undefined") {
    return snapshot;
  }
  for (const name of names) {
    snapshot.set(name, process.env[name]);
  }
  return snapshot;
}

function restoreEnv(snapshot: EnvSnapshot): void {
  if (typeof process === "undefined") {
    return;
  }
  for (const [name, value] of snapshot.entries()) {
    if (value === undefined) {
      delete process.env[name];
    } else {
      process.env[name] = value;
    }
  }
}

function requestText(
  urlString: string,
  options: {
    method: "GET" | "POST";
    headers: Record<string, string>;
    body?: string;
    signal?: AbortSignal;
  },
): Promise<{ status: number; text: string }> {
  return new Promise((resolve, reject) => {
    const url = new URL(urlString);
    const transport = url.protocol === "https:" ? httpsRequest : httpRequest;
    const headers = { ...options.headers };
    if (options.body !== undefined) {
      headers["content-length"] = String(Buffer.byteLength(options.body));
    }

    let settled = false;
    const finish = (fn: () => void) => {
      if (settled) {
        return;
      }
      settled = true;
      if (options.signal) {
        options.signal.removeEventListener("abort", onAbort);
      }
      fn();
    };
    const onAbort = () => {
      req.destroy(new Error("Garyx gateway request aborted"));
    };

    const req = transport(
      url,
      {
        method: options.method,
        headers,
      },
      (res) => {
        res.setEncoding("utf8");
        let text = "";
        res.on("data", (chunk) => {
          text += chunk;
        });
        res.on("end", () => {
          finish(() => resolve({ status: res.statusCode ?? 0, text }));
        });
      },
    );

    req.on("error", (error) => {
      finish(() => reject(error));
    });
    if (options.signal) {
      if (options.signal.aborted) {
        onAbort();
      } else {
        options.signal.addEventListener("abort", onAbort, { once: true });
      }
    }
    if (options.body !== undefined) {
      req.write(options.body);
    }
    req.end();
  });
}
