import type {
  CommittedMessageEvent,
  DesktopChatStreamEvent,
  DesktopSettings,
  InterruptResult,
  MessageFileAttachment,
  MessageImageAttachment,
  OpenChatStreamResult,
  RenderState,
  SendMessageInput,
  SendStreamingInputResult,
  TranscriptMessage,
} from "@shared/contracts";
import {
  decideStreamSeq,
  isControlTranscriptMessage,
} from "../../shared/transcript-sync.ts";
import { applyGatewayAuthHeader, applyGatewayCustomHeaders, asBoolean, asFiniteNumber, asString, buildUrl, gatewayFetch, isLocalGatewayUrl, parseRecord, requestJson, tryParseJson } from "./http.ts";

const PROVIDER_ENV_METADATA_KEY = "provider_env";

const CODEX_API_KEY_ENV = "OPENAI_API_KEY";

type SerializedMessageAttachments = {
  attachments: Array<{
    kind: "image" | "file";
    path: string;
    name: string;
    media_type: string;
  }>;
  images: Array<{
    name: string;
    data: string;
    media_type: string;
  }>;
  files: Array<{
    name: string;
    data: string;
    media_type: string;
  }>;
};

function serializeMessageAttachments(
  images?: MessageImageAttachment[],
  files?: MessageFileAttachment[],
): SerializedMessageAttachments {
  const attachments: SerializedMessageAttachments["attachments"] = [];
  const fallbackImages: SerializedMessageAttachments["images"] = [];
  const fallbackFiles: SerializedMessageAttachments["files"] = [];

  for (const image of images || []) {
    const path = image?.path?.trim() || "";
    const mediaType = image?.mediaType?.trim() || "";
    if (path && mediaType) {
      attachments.push({
        kind: "image",
        path,
        name: image.name,
        media_type: mediaType,
      });
      continue;
    }
    const data = image?.data?.trim() || "";
    if (!data || !mediaType) {
      continue;
    }
    fallbackImages.push({
      name: image.name,
      data,
      media_type: mediaType,
    });
  }

  for (const file of files || []) {
    const path = file?.path?.trim() || "";
    if (path) {
      attachments.push({
        kind: "file",
        path,
        name: file.name,
        media_type: file?.mediaType?.trim() || "",
      });
      continue;
    }
    const data = file?.data?.trim() || "";
    if (!data) {
      continue;
    }
    fallbackFiles.push({
      name: file.name,
      data,
      media_type: file?.mediaType?.trim() || "",
    });
  }

  return {
    attachments,
    images: fallbackImages,
    files: fallbackFiles,
  };
}

function formatLocalChatTimestamp(date = new Date()): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const seconds = String(date.getSeconds()).padStart(2, "0");
  return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`;
}

function resolveInputThreadId(input: SendMessageInput): string {
  return input.threadId || input.sessionId || "";
}

export class ThreadStreamGapError extends Error {
  resumeAfterSeq: number;

  constructor(resumeAfterSeq: number) {
    super(`Thread stream seq gap after ${resumeAfterSeq}`);
    this.name = "ThreadStreamGapError";
    this.resumeAfterSeq = resumeAfterSeq;
  }
}

interface StreamThreadEventsOptions {
  afterSeq?: number;
  onConnected?: () => void;
  onCommittedSeq?: (seq: number) => void;
}

function textFromCommittedMessage(message: Record<string, unknown>): string {
  const explicitText = asString(message.text);
  if (explicitText) {
    return explicitText;
  }
  const content = message.content;
  return typeof content === "string" ? content.trim() : "";
}

function kindFromCommittedMessage(
  role: TranscriptMessage["role"],
  message: Record<string, unknown>,
): string | undefined {
  const explicitKind = asString(message.kind);
  if (explicitKind) {
    return explicitKind;
  }
  const internalKind = asString(message.internal_kind);
  if (internalKind === "control") {
    return "control";
  }
  if (role === "tool" || role === "tool_use" || role === "tool_result") {
    return "tool_trace";
  }
  if (role === "assistant") {
    return "assistant_reply";
  }
  if (role === "user") {
    return "user_input";
  }
  return undefined;
}

function roleFromCommittedMessage(role: unknown): TranscriptMessage["role"] {
  return role === "assistant" ||
    role === "user" ||
    role === "tool" ||
    role === "tool_use" ||
    role === "tool_result"
    ? role
    : "system";
}

function mapCommittedMessageEvent(
  payload: Record<string, unknown>,
  eventId?: number | null,
): DesktopChatStreamEvent | null {
  const seq = asFiniteNumber(payload.seq) ?? eventId ?? undefined;
  if (typeof seq !== "number" || seq < 1) {
    return null;
  }
  const threadId =
    asString(payload.threadId) || asString(payload.thread_id) || "";
  const runId = asString(payload.runId) || asString(payload.run_id) || "";
  const rawMessage = parseRecord(payload.message);
  if (!threadId || Object.keys(rawMessage).length === 0) {
    return null;
  }
  const role = roleFromCommittedMessage(rawMessage.role);
  const metadata = parseRecord(rawMessage.metadata);
  const contentRecord = parseRecord(rawMessage.content);
  const kind = kindFromCommittedMessage(role, rawMessage);
  const isControlRecord =
    kind === "control" || asString(rawMessage.internal_kind) === "control";
  const baseMessage: TranscriptMessage = {
    id: `${threadId}:${seq - 1}`,
    seq,
    role,
    text: isControlRecord ? "" : textFromCommittedMessage(rawMessage),
    content: isControlRecord ? rawMessage : rawMessage.content,
    input: rawMessage.input,
    result: rawMessage.result,
    timestamp:
      asString(rawMessage.timestamp) || asString(payload.timestamp) || null,
    toolUseId:
      asString(rawMessage.tool_use_id) ||
      asString(rawMessage.toolUseId) ||
      null,
    toolName:
      asString(rawMessage.tool_name) ||
      asString(rawMessage.toolName) ||
      asString(metadata.item_type) ||
      asString(metadata.itemType) ||
      asString(contentRecord.type) ||
      null,
    isError: asBoolean(rawMessage.is_error) ?? asBoolean(rawMessage.isError),
    metadata: Object.keys(metadata).length ? metadata : null,
    kind,
    internal: isControlRecord || Boolean(rawMessage.internal),
    internalKind:
      asString(rawMessage.internal_kind) ||
      asString(rawMessage.internalKind) ||
      (isControlRecord ? "control" : null),
    loopOrigin:
      asString(rawMessage.loop_origin) || asString(rawMessage.loopOrigin) || null,
  };
  const message = isControlTranscriptMessage(baseMessage)
    ? {
        ...baseMessage,
        role: "system" as const,
        text: "",
        content: rawMessage,
        kind: "control",
        internal: true,
        internalKind: "control",
      }
    : baseMessage;
  return {
    type: "committed_message",
    runId,
    threadId,
    sessionId: threadId,
    seq,
    message,
  };
}

// The wire `render_state` already matches the locked `RenderSnapshot` serde
// shape (snake_case + the documented renames), so this only validates the
// top-level envelope; the render-view-model mapping tolerates any structural
// surprises by skipping unresolvable refs.
function parseRenderState(value: unknown): RenderState | null {
  const record = parseRecord(value);
  if (Object.keys(record).length === 0) {
    return null;
  }
  if (typeof asFiniteNumber(record.based_on_seq) !== "number") {
    return null;
  }
  if (!Array.isArray(record.rows)) {
    return null;
  }
  return record as unknown as RenderState;
}

// Unwrap a `thread_render_frame` into one atomic desktop event: the contiguous
// committed events plus the full render snapshot. Gap detection runs per inner
// event (never on `based_on_seq` alone) so batched catch-up frames stay
// gapless instead of triggering an endless reconnect.
function mapThreadRenderFrameEvent(
  payload: Record<string, unknown>,
  connectionLastSeq: number,
): { event: DesktopChatStreamEvent; lastSeq: number } | null {
  const threadId =
    asString(payload.threadId) || asString(payload.thread_id) || "";
  if (!threadId) {
    return null;
  }
  const renderState = parseRenderState(payload.render_state ?? payload.renderState);
  if (!renderState) {
    return null;
  }
  const rawEvents = Array.isArray(payload.events) ? payload.events : [];
  // A frame marked replay:"windowed" is a server-degraded stale resume:
  // its records start at the window floor, deliberately NOT contiguous
  // with our cursor. The marker (never seq arithmetic) authorizes the
  // discontinuity; ordinary frames keep the per-event gap guard.
  const windowedReplay = asString(payload.replay) === "windowed";
  let lastSeq = connectionLastSeq;
  const events: CommittedMessageEvent[] = [];
  for (const raw of rawEvents) {
    const mapped = mapCommittedMessageEvent(parseRecord(raw));
    if (!mapped || mapped.type !== "committed_message") {
      continue;
    }
    if (!windowedReplay) {
      const decision = decideStreamSeq({
        incomingSeq: mapped.seq,
        connectionLastSeq: lastSeq,
      });
      if (decision.type === "gap_reconnect") {
        throw new ThreadStreamGapError(decision.resumeAfterSeq);
      }
      if (decision.type === "stale") {
        continue;
      }
    }
    events.push(mapped);
    lastSeq = Math.max(lastSeq, mapped.seq);
  }
  return {
    event: {
      type: "thread_render_frame",
      threadId,
      events,
      renderState,
      ...(windowedReplay ? { replay: "windowed" as const } : {}),
    },
    lastSeq,
  };
}

export async function streamThreadEvents(
  settings: DesktopSettings,
  threadId: string,
  onEvent: (event: DesktopChatStreamEvent) => void,
  signal?: AbortSignal,
  options?: StreamThreadEventsOptions,
): Promise<void> {
  const afterSeq = Math.max(0, Math.trunc(options?.afterSeq ?? 0));
  const headers = applyGatewayAuthHeader(
    applyGatewayCustomHeaders(
      new Headers({ Accept: "text/event-stream" }),
      settings.gatewayHeaders,
    ),
    settings.gatewayAuthToken,
  );
  headers.set("Last-Event-ID", String(afterSeq));
  const response = await gatewayFetch(
    buildUrl(
      settings,
      `/api/threads/${encodeURIComponent(threadId)}/stream?after_seq=${afterSeq}&windowed_resume=1`,
    ),
    {
      headers,
      signal,
    },
  );
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`);
  }
  if (!response.body) {
    throw new Error("Thread event stream returned no body");
  }
  options?.onConnected?.();

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let dataLines: string[] = [];
  let connectionLastSeq = afterSeq;

  const flushEvent = () => {
    if (dataLines.length === 0) {
      return;
    }
    const payloadText = dataLines.join("\n");
    dataLines = [];
    const payload = tryParseJson<Record<string, unknown>>(payloadText);
    if (!payload) {
      return;
    }
    if (asString(payload.type) !== "thread_render_frame") {
      return;
    }
    const frame = mapThreadRenderFrameEvent(payload, connectionLastSeq);
    if (!frame) {
      return;
    }
    onEvent(frame.event);
    connectionLastSeq = frame.lastSeq;
    options?.onCommittedSeq?.(connectionLastSeq);
  };
  const processLine = (line: string) => {
    if (line === "") {
      flushEvent();
      return;
    }
    if (line.startsWith(":")) {
      return;
    }
    if (line.startsWith("id:")) {
      // SSE id mirrors the frame's based_on_seq; the cursor is driven by the
      // committed events themselves, so the id line is informational only.
      return;
    }
    if (!line.startsWith("data:")) {
      return;
    }
    let value = line.slice(5);
    if (value.startsWith(" ")) {
      value = value.slice(1);
    }
    dataLines.push(value);
  };

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true });
      let newlineIndex = buffer.indexOf("\n");
      while (newlineIndex >= 0) {
        const rawLine = buffer.slice(0, newlineIndex).replace(/\r$/, "");
        buffer = buffer.slice(newlineIndex + 1);
        processLine(rawLine);
        newlineIndex = buffer.indexOf("\n");
      }
    }
    buffer += decoder.decode();
    if (buffer.length > 0) {
      processLine(buffer.replace(/\r$/, ""));
      buffer = "";
    }
    flushEvent();
  } finally {
    reader.releaseLock();
  }
}

function stripMatchingQuotes(value: string): string {
  if (value.length >= 2) {
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      return value.slice(1, -1);
    }
  }
  return value;
}

function parseProviderEnvBlock(raw: string): Record<string, string> {
  const env: Record<string, string> = {};

  for (const line of raw.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) {
      continue;
    }

    const normalized = trimmed.startsWith("export ")
      ? trimmed.slice("export ".length).trim()
      : trimmed;
    const separator = normalized.indexOf("=");
    if (separator <= 0) {
      continue;
    }

    const key = normalized.slice(0, separator).trim();
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
      continue;
    }

    const value = stripMatchingQuotes(normalized.slice(separator + 1).trim());
    env[key] = value;
  }

  return env;
}

function buildProviderMetadata(
  settings: DesktopSettings,
): Record<string, unknown> | undefined {
  if (!isLocalGatewayUrl(settings.gatewayUrl)) {
    return undefined;
  }

  const metadata: Record<string, unknown> = {};
  const providerEnv = parseProviderEnvBlock(settings.providerClaudeEnv);
  const oauthToken = asString(process.env.CLAUDE_CODE_OAUTH_TOKEN);
  if (
    oauthToken &&
    !Object.prototype.hasOwnProperty.call(providerEnv, "CLAUDE_CODE_OAUTH_TOKEN")
  ) {
    providerEnv.CLAUDE_CODE_OAUTH_TOKEN = oauthToken;
  }

  Object.assign(providerEnv, parseProviderEnvBlock(settings.providerGeminiEnv));
  providerEnv[CODEX_API_KEY_ENV] =
    settings.providerCodexAuthMode === "api_key"
      ? settings.providerCodexApiKey.trim()
      : "";

  if (Object.keys(providerEnv).length > 0) {
    metadata[PROVIDER_ENV_METADATA_KEY] = providerEnv;
  }

  return Object.keys(metadata).length > 0 ? metadata : undefined;
}

export async function openChatStream(
  settings: DesktopSettings,
  input: SendMessageInput,
  workspacePath?: string | null,
): Promise<{
  runId: string;
  threadId: string;
  sessionId?: string;
  response: string;
  status: OpenChatStreamResult["status"];
}> {
  const threadId = resolveInputThreadId(input);
  const providerMetadata = buildProviderMetadata(settings);
  const serializedAttachments = serializeMessageAttachments(
    input.images,
    input.files,
  );
  const payload = await requestJson<{
    status?: unknown;
    runId?: unknown;
    run_id?: unknown;
    threadId?: unknown;
    thread_id?: unknown;
  }>(settings, "/api/chat/start", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      message: input.message,
      attachments: serializedAttachments.attachments,
      images: serializedAttachments.images,
      files: serializedAttachments.files,
      threadId,
      accountId: settings.accountId,
      fromId: settings.fromId,
      waitForResponse: false,
      timeoutSeconds: settings.timeoutSeconds,
      workspacePath: workspacePath || undefined,
      metadata: {
        client_timestamp_local: formatLocalChatTimestamp(),
        client_intent_id: input.clientIntentId,
      },
      providerMetadata,
    }),
    signal: AbortSignal.timeout(8000),
  });
  const responseThreadId =
    asString(payload.threadId) || asString(payload.thread_id) || threadId;
  return {
    runId: asString(payload.runId) || asString(payload.run_id) || "",
    threadId: responseThreadId,
    sessionId: responseThreadId,
    response: "",
    status: asString(payload.status) === "accepted" ? "accepted" : "disconnected",
  };
}

export async function sendStreamingInput(
  settings: DesktopSettings,
  input: SendMessageInput,
): Promise<SendStreamingInputResult> {
  const threadId = resolveInputThreadId(input);
  const serializedAttachments = serializeMessageAttachments(
    input.images,
    input.files,
  );
  try {
    const payload = await requestJson<{
      status?: unknown;
      threadId?: unknown;
      thread_id?: unknown;
      sessionId?: unknown;
      clientIntentId?: unknown;
      client_intent_id?: unknown;
      pendingInputId?: unknown;
      pending_input_id?: unknown;
    }>(settings, "/api/chat/stream-input", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        threadId,
        clientIntentId: input.clientIntentId,
        message: input.message,
        attachments: serializedAttachments.attachments,
        images: serializedAttachments.images,
        files: serializedAttachments.files,
      }),
      signal: AbortSignal.timeout(8000),
    });
    const responseThreadId =
      asString(payload.threadId) || asString(payload.thread_id) || threadId;
    return {
      status: asString(payload.status) || "no_active_session",
      threadId: responseThreadId,
      sessionId: asString(payload.sessionId) || responseThreadId,
      clientIntentId:
        asString(payload.clientIntentId) ||
        asString(payload.client_intent_id) ||
        input.clientIntentId,
      pendingInputId:
        asString(payload.pendingInputId) || asString(payload.pending_input_id),
    };
  } catch {
    // Preserve the old local-only response when the gateway cannot be reached.
  }
  return {
    status: "no_active_session",
    threadId,
    sessionId: input.sessionId || threadId,
    clientIntentId: input.clientIntentId,
  };
}

export async function interruptThread(
  settings: DesktopSettings,
  threadId: string,
): Promise<InterruptResult> {
  try {
    const payload = await requestJson<{
      status?: unknown;
      threadId?: unknown;
      thread_id?: unknown;
      sessionId?: unknown;
      abortedRuns?: unknown;
      aborted_runs?: unknown;
    }>(settings, "/api/chat/interrupt", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ threadId }),
      signal: AbortSignal.timeout(8000),
    });
    const responseThreadId =
      asString(payload.threadId) || asString(payload.thread_id) || threadId;
    const abortedRunsPayload = Array.isArray(payload.abortedRuns)
      ? payload.abortedRuns
      : Array.isArray(payload.aborted_runs)
        ? payload.aborted_runs
        : [];
    return {
      status: asString(payload.status) || "not_found",
      threadId: responseThreadId,
      sessionId: asString(payload.sessionId) || responseThreadId,
      abortedRuns: abortedRunsPayload.map((entry) => String(entry)),
    };
  } catch {
    // Preserve local abort behavior when the gateway is unreachable.
  }
  return {
    status: "local_abort_only",
    threadId,
    sessionId: threadId,
    abortedRuns: [],
  };
}

export const interruptSession = interruptThread;
