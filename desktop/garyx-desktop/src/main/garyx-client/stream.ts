import type {
  CommittedMessageEvent,
  DesktopChatStreamEvent,
  DesktopSettings,
  InterruptResult,
  MessageFileAttachment,
  MessageImageAttachment,
  OpenChatStreamResult,
  RenderDelta,
  RenderRow,
  RenderState,
  SendMessageInput,
  SendStreamingInputResult,
  TranscriptMessage,
} from "@shared/contracts";
import {
  decideStreamSeq,
  isControlTranscriptMessage,
} from "../../shared/transcript-sync.ts";
import { applyGatewayAuthHeader, applyGatewayCustomHeaders, asBoolean, asFiniteNumber, asString, buildUrl, gatewayStreamFetch, parseRecord, requestJson, tryParseJson } from "./http.ts";

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

  constructor(resumeAfterSeq: number, reason?: string) {
    super(
      reason
        ? `Thread stream seq gap after ${resumeAfterSeq}: ${reason}`
        : `Thread stream seq gap after ${resumeAfterSeq}`,
    );
    this.name = "ThreadStreamGapError";
    this.resumeAfterSeq = resumeAfterSeq;
  }
}

interface StreamThreadEventsOptions {
  afterSeq?: number;
  /** Pin the server's windowed render derivation to this floor
   * (`render_floor` query param); 0/absent keeps today's behavior. */
  renderFloor?: number;
  onConnected?: () => void;
  onCommittedSeq?: (seq: number) => void;
  /** Fires whenever a frame carries `render_state.window.floor_seq > 0` —
   * the floor this connection is now rendering with. */
  onWindowFloor?: (floorSeq: number) => void;
  /** Test override for {@link STREAM_HEADER_TIMEOUT_MS}. */
  headerTimeoutMs?: number;
  /** Test override for {@link STREAM_IDLE_TIMEOUT_MS}. */
  idleTimeoutMs?: number;
}

/** Response headers must arrive within this window. A gateway that accepts
 * the TCP connection but never answers (control-plane saturation, a crash
 * mid-accept, a half-open tunnel) would otherwise pin one of Chromium's six
 * per-host connection slots indefinitely — enough stalled streams starve
 * every other gateway request into its own AbortSignal timeout (#TASK-1840). */
const STREAM_HEADER_TIMEOUT_MS = 20_000;

/** The gateway emits an SSE keep-alive ping every 30s (`routes.rs`). Two
 * missed pings plus margin means the connection is dead even though TCP
 * never noticed (typical for remote gateways behind VPN/proxy tunnels);
 * recycle it so the caller's reconnect loop resumes from its cursor. */
const STREAM_IDLE_TIMEOUT_MS = 90_000;

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

/** Per-connection delta reassembly base (#TASK-1956 batch 2): the last
 * full `render_state` this connection accepted, `rows_hash` chain token
 * included. The chain only needs to live within one connection — after a
 * reconnect the first frame is always a full replay/snapshot frame, which
 * reseeds unconditionally — so the holder is created per connection in
 * {@link forwardThreadStreamBody} and dies with it. */
interface RenderFrameReassembly {
  held: RenderState | null;
}

function renderRowId(row: unknown): string | undefined {
  return asString(parseRecord(row).id);
}

// Reassemble a full render snapshot from the held one plus a wire
// `render_delta` (#TASK-1956 batch 2). Mirrors `apply_render_delta` in
// garyx-models/src/transcript_render_state.rs minus the reassembled-rows
// hash tripwire: the server is the only hasher; the client stores
// `rows_hash` as an opaque token and validates the chain by equality.
// Every violation is a protocol breach — throw ThreadStreamGapError and
// ride the existing gap pipeline (hub stop → authoritative refetch); the
// reconnect's replay frame reseeds the chain.
function applyRenderDeltaFrame(
  held: RenderState | null,
  value: unknown,
  resumeAfterSeq: number,
): RenderState {
  const violation = (reason: string) =>
    new ThreadStreamGapError(resumeAfterSeq, `render delta ${reason}`);
  const delta = parseRecord(value) as Partial<RenderDelta> &
    Record<string, unknown>;
  const fromSeq = asFiniteNumber(delta.from_seq);
  const basedOnSeq = asFiniteNumber(delta.based_on_seq);
  const fromRowsHash = asString(delta.from_rows_hash);
  const rowsHash = asString(delta.rows_hash);
  if (
    typeof fromSeq !== "number" ||
    typeof basedOnSeq !== "number" ||
    !fromRowsHash ||
    !rowsHash ||
    !Array.isArray(delta.row_order) ||
    !Array.isArray(delta.upsert_rows)
  ) {
    throw violation("frame is malformed");
  }
  if (!held || fromSeq !== held.based_on_seq) {
    throw violation(
      `from_seq ${fromSeq} does not match held snapshot seq ${held ? held.based_on_seq : "(none)"}`,
    );
  }
  // Pure equality on the opaque chain token — the client never hashes. A
  // held snapshot without a token (a full frame that arrived without
  // rows_hash) can never anchor a delta.
  if (fromRowsHash !== held.rows_hash) {
    throw violation("from_rows_hash does not match held rows-hash token");
  }
  const upsertById = new Map<string, RenderRow>();
  const orderIds = new Set<string>();
  for (const rowId of delta.row_order) {
    if (!asString(rowId)) {
      throw violation("row_order carries a non-string id");
    }
    orderIds.add(rowId as string);
  }
  for (const row of delta.upsert_rows) {
    const rowId = renderRowId(row);
    if (!rowId) {
      throw violation("upsert row is missing its id");
    }
    if (upsertById.has(rowId)) {
      throw violation(`upsert row id ${rowId} appears more than once`);
    }
    // Every upsert must be referenced by row_order: a stray upsert is a
    // producer/consumer disagreement, not ignorable padding.
    if (!orderIds.has(rowId)) {
      throw violation(`upsert row id ${rowId} is absent from row_order`);
    }
    upsertById.set(rowId, row as RenderRow);
  }
  const prevById = new Map<string, RenderRow>();
  for (const row of held.rows) {
    const rowId = renderRowId(row);
    if (rowId) {
      prevById.set(rowId, row);
    }
  }
  // Rebuild rows in row_order: upsert body wins, else the held row by id.
  const rows: RenderRow[] = [];
  for (const rowId of delta.row_order as string[]) {
    const row = upsertById.get(rowId) ?? prevById.get(rowId);
    if (!row) {
      throw violation(
        `row id ${rowId} missing from upsert rows and held snapshot`,
      );
    }
    rows.push(row);
  }
  // Scalar fields are replaced wholesale; rateLimit/window mirror the full
  // frame's wire shape (absent when the server skipped them).
  return {
    based_on_seq: basedOnSeq,
    rows,
    tailActivity: delta.tailActivity as RenderState["tailActivity"],
    activeToolGroupId:
      (delta.activeToolGroupId as RenderState["activeToolGroupId"]) ?? null,
    progress_locus: delta.progress_locus as RenderState["progress_locus"],
    filtered_placeholders: Array.isArray(delta.filtered_placeholders)
      ? (delta.filtered_placeholders as RenderState["filtered_placeholders"])
      : [],
    ...(delta.rateLimit !== undefined
      ? { rateLimit: delta.rateLimit as RenderState["rateLimit"] }
      : {}),
    ...(delta.window !== undefined
      ? { window: delta.window as RenderState["window"] }
      : {}),
    rows_hash: rowsHash,
  };
}

// Unwrap a `thread_render_frame` into one atomic desktop event: the contiguous
// committed events plus the full render snapshot. Gap detection runs per inner
// event (never on `based_on_seq` alone) so batched catch-up frames stay
// gapless instead of triggering an endless reconnect.
//
// Delta frames (#TASK-1956 batch 2): a frame may carry `render_delta`
// instead of `render_state`; it is reassembled against `reassembly.held`
// BEFORE any event is emitted, so a chain violation discards the frame
// atomically. Either way the emitted desktop event always carries a full
// `renderState` — the renderer, mirror, and frontier never learn deltas
// exist. Every frame that carries a full `render_state` (replay,
// snapshot-only, first-live, same-seq reseed) unconditionally replaces the
// held snapshot + chain token; there is no other cache-invalidation event.
function mapThreadRenderFrameEvent(
  payload: Record<string, unknown>,
  connectionLastSeq: number,
  reassembly: RenderFrameReassembly,
): { event: DesktopChatStreamEvent; lastSeq: number } | null {
  const threadId =
    asString(payload.threadId) || asString(payload.thread_id) || "";
  if (!threadId) {
    return null;
  }
  let renderState = parseRenderState(payload.render_state ?? payload.renderState);
  if (!renderState) {
    const rawDelta = payload.render_delta ?? payload.renderDelta;
    if (rawDelta === undefined || rawDelta === null) {
      return null;
    }
    renderState = applyRenderDeltaFrame(
      reassembly.held,
      rawDelta,
      connectionLastSeq,
    );
  }
  reassembly.held = renderState;
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
  const renderFloor = Math.max(0, Math.trunc(options?.renderFloor ?? 0));
  const headerTimeoutMs = Math.max(
    1,
    Math.trunc(options?.headerTimeoutMs ?? STREAM_HEADER_TIMEOUT_MS),
  );
  const idleTimeoutMs = Math.max(
    1,
    Math.trunc(options?.idleTimeoutMs ?? STREAM_IDLE_TIMEOUT_MS),
  );
  const headers = applyGatewayAuthHeader(
    applyGatewayCustomHeaders(
      new Headers({ Accept: "text/event-stream" }),
      settings.gatewayHeaders,
    ),
    settings.gatewayAuthToken,
  );
  headers.set("Last-Event-ID", String(afterSeq));
  const renderFloorParam = renderFloor > 0 ? `&render_floor=${renderFloor}` : "";

  // Watchdog: aborts the underlying fetch when response headers or stream
  // bytes stop arriving, so a silently dead connection cannot hold one of
  // Chromium's per-host slots forever. External aborts propagate into it;
  // watchdog aborts surface as ordinary errors so the caller's reconnect
  // loop resumes from its committed cursor.
  const watchdog = new AbortController();
  let stalledPhase: "headers" | "stream" | null = null;
  let watchdogTimer: ReturnType<typeof setTimeout> | null = null;
  const armWatchdog = (phase: "headers" | "stream", timeoutMs: number) => {
    if (watchdogTimer) {
      clearTimeout(watchdogTimer);
    }
    watchdogTimer = setTimeout(() => {
      stalledPhase = phase;
      watchdog.abort();
    }, timeoutMs);
  };
  const disarmWatchdog = () => {
    if (watchdogTimer) {
      clearTimeout(watchdogTimer);
      watchdogTimer = null;
    }
  };
  const propagateExternalAbort = () => watchdog.abort();
  if (signal?.aborted) {
    watchdog.abort();
  } else {
    signal?.addEventListener("abort", propagateExternalAbort, { once: true });
  }
  const stalledError = () =>
    stalledPhase === "headers"
      ? new Error(
          `Thread event stream stalled: no response headers within ${headerTimeoutMs}ms`,
        )
      : new Error(
          `Thread event stream stalled: no bytes within ${idleTimeoutMs}ms (missed keep-alives)`,
        );

  try {
    armWatchdog("headers", headerTimeoutMs);
    let response: Response;
    try {
      // Streams ride their own socket pool (gatewayStreamFetch): a live SSE
      // connection holds its socket for as long as it runs, and six of them
      // on the shared pool starve every control request (#TASK-1840).
      response = await gatewayStreamFetch(
        buildUrl(
          settings,
          // render_mode=delta (#TASK-1956 batch 2): live frames carry
          // `render_delta` instead of a full `render_state`; the
          // reassembler below rebuilds full snapshots for downstream.
          `/api/threads/${encodeURIComponent(threadId)}/stream?after_seq=${afterSeq}&render_mode=delta${renderFloorParam}`,
        ),
        {
          headers,
          signal: watchdog.signal,
        },
      );
    } catch (error) {
      if (stalledPhase) {
        throw stalledError();
      }
      throw error;
    } finally {
      disarmWatchdog();
    }
    if (!response.ok) {
      throw new Error(`${response.status} ${response.statusText}`);
    }
    if (!response.body) {
      throw new Error("Thread event stream returned no body");
    }
    options?.onConnected?.();
    await forwardThreadStreamBody(response.body, afterSeq, onEvent, options, {
      armIdle: () => armWatchdog("stream", idleTimeoutMs),
      disarmIdle: disarmWatchdog,
      stalled: () => (stalledPhase ? stalledError() : null),
    });
  } finally {
    disarmWatchdog();
    // Release the underlying connection on EVERY exit path. A gap error (or
    // any parse throw) leaves the response body unconsumed; without this
    // abort the socket stays checked out of Chromium's per-host pool — one
    // zombie ESTABLISHED connection per gap (#TASK-1840). Aborting after a
    // normally-completed stream is a no-op.
    watchdog.abort();
    signal?.removeEventListener("abort", propagateExternalAbort);
  }
}

interface ThreadStreamWatchdogHooks {
  armIdle: () => void;
  disarmIdle: () => void;
  /** Non-null when the watchdog (not an external abort) killed the fetch. */
  stalled: () => Error | null;
}

async function forwardThreadStreamBody(
  body: ReadableStream<Uint8Array>,
  afterSeq: number,
  onEvent: (event: DesktopChatStreamEvent) => void,
  options: StreamThreadEventsOptions | undefined,
  watchdog: ThreadStreamWatchdogHooks,
): Promise<void> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let dataLines: string[] = [];
  let connectionLastSeq = afterSeq;
  // Delta chain state lives in this single-connection closure and dies
  // with it: a reconnect always starts on a full replay frame (reseed).
  const reassembly: RenderFrameReassembly = { held: null };

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
    const frame = mapThreadRenderFrameEvent(payload, connectionLastSeq, reassembly);
    if (!frame) {
      return;
    }
    onEvent(frame.event);
    connectionLastSeq = frame.lastSeq;
    options?.onCommittedSeq?.(connectionLastSeq);
    const windowFloor =
      frame.event.type === "thread_render_frame"
        ? (frame.event.renderState.window?.floor_seq ?? 0)
        : 0;
    if (windowFloor > 0) {
      options?.onWindowFloor?.(windowFloor);
    }
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
      watchdog.armIdle();
      let chunk: ReadableStreamReadResult<Uint8Array>;
      try {
        chunk = await reader.read();
      } catch (error) {
        const stalled = watchdog.stalled();
        if (stalled) {
          throw stalled;
        }
        throw error;
      } finally {
        watchdog.disarmIdle();
      }
      const { done, value } = chunk;
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
