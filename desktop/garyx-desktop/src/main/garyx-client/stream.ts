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
import {
  GatewayContractError,
  applyGatewayAuthHeader,
  applyGatewayCustomHeaders,
  asBoolean,
  asFiniteNumber,
  asString,
  buildUrl,
  gatewayStreamFetch,
  hasContractField,
  parseRecord,
  requestJson,
  requireContractArray,
  requireContractBoolean,
  requireContractField,
  requireContractNonEmptyString,
  requireContractNonNegativeInteger,
  requireContractRecord,
  requireContractString,
  tryParseJson,
} from "./http.ts";

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
  /** Fires for every accepted full frame, including floor 0, so callers can
   * clear a formerly-windowed connection state. */
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
  value: unknown,
  path: string,
): CommittedMessageEvent {
  const payload = requireContractRecord(value, path);
  const eventType = requireContractString(
    requireContractField(payload, "type", path),
    `${path}.type`,
  );
  if (eventType !== "committed_message") {
    throw new GatewayContractError(
      `${path}.type`,
      'must be "committed_message"',
    );
  }
  const seq = requireContractNonNegativeInteger(
    requireContractField(payload, "seq", path),
    `${path}.seq`,
  );
  if (seq < 1) {
    throw new GatewayContractError(`${path}.seq`, "must be at least 1");
  }
  const threadId = requireContractNonEmptyString(
    requireContractField(payload, "thread_id", path),
    `${path}.thread_id`,
  );
  const rawRunId = requireContractField(payload, "run_id", path);
  const runId =
    rawRunId === null
      ? ""
      : requireContractString(rawRunId, `${path}.run_id`);
  const rawMessage = requireContractRecord(
    requireContractField(payload, "message", path),
    `${path}.message`,
  );
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
    timestamp: asString(rawMessage.timestamp) || null,
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
// shape (snake_case + the documented explicit renames). Validate its complete
// required top-level contract without deriving or rewriting any render rows.
function parseRenderState(value: unknown, path: string): RenderState {
  const record = requireContractRecord(value, path);
  requireContractNonNegativeInteger(
    requireContractField(record, "based_on_seq", path),
    `${path}.based_on_seq`,
  );
  requireContractArray(
    requireContractField(record, "rows", path),
    `${path}.rows`,
  );
  const tailActivity = requireContractString(
    requireContractField(record, "tailActivity", path),
    `${path}.tailActivity`,
  );
  if (
    tailActivity !== "none" &&
    tailActivity !== "thinking" &&
    tailActivity !== "assistant_streaming" &&
    tailActivity !== "tool_active"
  ) {
    throw new GatewayContractError(`${path}.tailActivity`, "has an unknown value");
  }
  const activeToolGroupId = requireContractField(
    record,
    "activeToolGroupId",
    path,
  );
  if (activeToolGroupId !== null) {
    requireContractString(activeToolGroupId, `${path}.activeToolGroupId`);
  }
  const progressLocus = requireContractString(
    requireContractField(record, "progress_locus", path),
    `${path}.progress_locus`,
  );
  if (
    progressLocus !== "none" &&
    progressLocus !== "tail" &&
    progressLocus !== "tool_group"
  ) {
    throw new GatewayContractError(`${path}.progress_locus`, "has an unknown value");
  }
  requireContractArray(
    requireContractField(record, "filtered_placeholders", path),
    `${path}.filtered_placeholders`,
  );
  if (hasContractField(record, "rateLimit")) {
    requireContractRecord(record.rateLimit, `${path}.rateLimit`);
  }
  if (hasContractField(record, "window")) {
    const window = requireContractRecord(record.window, `${path}.window`);
    requireContractNonNegativeInteger(
      requireContractField(window, "floor_seq", `${path}.window`),
      `${path}.window.floor_seq`,
    );
    requireContractBoolean(
      requireContractField(window, "has_more_above", `${path}.window`),
      `${path}.window.has_more_above`,
    );
  }
  if (hasContractField(record, "rows_hash")) {
    requireContractNonEmptyString(record.rows_hash, `${path}.rows_hash`);
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
    !Number.isSafeInteger(fromSeq) ||
    fromSeq < 0 ||
    typeof basedOnSeq !== "number" ||
    !Number.isSafeInteger(basedOnSeq) ||
    basedOnSeq < 0 ||
    !fromRowsHash ||
    !rowsHash ||
    !Array.isArray(delta.row_order) ||
    !Array.isArray(delta.upsert_rows) ||
    !Array.isArray(delta.filtered_placeholders) ||
    typeof delta.tailActivity !== "string" ||
    !(delta.activeToolGroupId === null ||
      typeof delta.activeToolGroupId === "string") ||
    typeof delta.progress_locus !== "string"
  ) {
    throw violation("frame is malformed");
  }
  if (
    !["none", "thinking", "assistant_streaming", "tool_active"].includes(
      delta.tailActivity,
    ) ||
    !["none", "tail", "tool_group"].includes(delta.progress_locus)
  ) {
    throw violation("frame carries an unknown render enum");
  }
  if (delta.rateLimit !== undefined && !Object.keys(parseRecord(delta.rateLimit)).length) {
    throw violation("rateLimit must be an object when present");
  }
  if (delta.window !== undefined) {
    const window = parseRecord(delta.window);
    if (
      !Number.isSafeInteger(window.floor_seq) ||
      (window.floor_seq as number) < 0 ||
      typeof window.has_more_above !== "boolean"
    ) {
      throw violation("window is malformed");
    }
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
    filtered_placeholders:
      delta.filtered_placeholders as RenderState["filtered_placeholders"],
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
): { event: DesktopChatStreamEvent; lastSeq: number } {
  const context = "thread render frame";
  const threadId = requireContractNonEmptyString(
    requireContractField(payload, "thread_id", context),
    `${context}.thread_id`,
  );
  const hasRenderState = hasContractField(payload, "render_state");
  const hasRenderDelta = hasContractField(payload, "render_delta");
  if (hasRenderState === hasRenderDelta) {
    throw new GatewayContractError(
      context,
      "must carry exactly one of render_state or render_delta",
    );
  }
  let renderState: RenderState;
  if (hasRenderState) {
    renderState = parseRenderState(payload.render_state, `${context}.render_state`);
  } else {
    renderState = applyRenderDeltaFrame(
      reassembly.held,
      payload.render_delta,
      connectionLastSeq,
    );
  }
  reassembly.held = renderState;
  const rawEvents = requireContractArray(
    requireContractField(payload, "events", context),
    `${context}.events`,
  );
  // A frame marked replay:"windowed" is a server-degraded stale resume:
  // its records start at the window floor, deliberately NOT contiguous
  // with our cursor. The marker (never seq arithmetic) authorizes the
  // discontinuity; ordinary frames keep the per-event gap guard.
  let windowedReplay = false;
  if (hasContractField(payload, "replay")) {
    const replay = requireContractString(payload.replay, `${context}.replay`);
    if (replay !== "windowed") {
      throw new GatewayContractError(
        `${context}.replay`,
        'must be "windowed" when present',
      );
    }
    windowedReplay = true;
  }
  let lastSeq = connectionLastSeq;
  const events: CommittedMessageEvent[] = [];
  for (const [index, raw] of rawEvents.entries()) {
    const mapped = mapCommittedMessageEvent(
      raw,
      `${context}.events[${index}]`,
    );
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
        "readRetryable",
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
      throw new ThreadStreamGapError(
        connectionLastSeq,
        "thread render frame is not valid JSON",
      );
    }
    let frame: ReturnType<typeof mapThreadRenderFrameEvent>;
    try {
      const frameType = requireContractString(
        requireContractField(payload, "type", "thread render frame"),
        "thread render frame.type",
      );
      if (frameType !== "thread_render_frame") {
        throw new GatewayContractError(
          "thread render frame.type",
          'must be "thread_render_frame"',
        );
      }
      frame = mapThreadRenderFrameEvent(
        payload,
        connectionLastSeq,
        reassembly,
      );
    } catch (error) {
      if (error instanceof GatewayContractError) {
        throw new ThreadStreamGapError(connectionLastSeq, error.message);
      }
      throw error;
    }
    onEvent(frame.event);
    connectionLastSeq = frame.lastSeq;
    options?.onCommittedSeq?.(connectionLastSeq);
    const windowFloor =
      frame.event.type === "thread_render_frame"
        ? (frame.event.renderState.window?.floor_seq ?? 0)
        : 0;
    options?.onWindowFloor?.(windowFloor);
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
  response: string;
  status: OpenChatStreamResult["status"];
}> {
  const body = buildChatStartRequestBody(settings, input, workspacePath);
  const payloadValue = await requestJson<unknown>(
    settings,
    "/api/chat/start",
    "mutationSingleAttempt",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
      signal: AbortSignal.timeout(8000),
    },
  );
  const payload = requireContractRecord(payloadValue, "chat start response");
  const status = requireContractString(
    requireContractField(payload, "status", "chat start response"),
    "chat start response.status",
  );
  if (status !== "accepted") {
    throw new GatewayContractError(
      "chat start response.status",
      'must be "accepted"',
    );
  }
  return {
    runId: requireContractNonEmptyString(
      requireContractField(payload, "runId", "chat start response"),
      "chat start response.runId",
    ),
    threadId: requireContractNonEmptyString(
      requireContractField(payload, "threadId", "chat start response"),
      "chat start response.threadId",
    ),
    response: "",
    status,
  };
}

export function buildChatStartRequestBody(
  settings: DesktopSettings,
  input: SendMessageInput,
  workspacePath?: string | null,
): string {
  const threadId = resolveInputThreadId(input);
  const serializedAttachments = serializeMessageAttachments(
    input.images,
    input.files,
  );
  return JSON.stringify({
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
      client_timestamp_local: input.clientTimestampLocal,
      client_intent_id: input.clientIntentId,
    },
  });
}

function optionalChatResponseString(
  payload: Record<string, unknown>,
  field: string,
  context: string,
): string | undefined {
  if (!hasContractField(payload, field)) {
    return undefined;
  }
  return requireContractString(payload[field], `${context}.${field}`);
}

export async function sendStreamingInput(
  settings: DesktopSettings,
  input: SendMessageInput,
): Promise<SendStreamingInputResult> {
  const threadId = resolveInputThreadId(input);
  try {
    const payloadValue = await requestJson<unknown>(
      settings,
      "/api/chat/stream-input",
      "mutationSingleAttempt",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: buildStreamInputRequestBody(input),
        signal: AbortSignal.timeout(8000),
      },
    );
    const payload = requireContractRecord(
      payloadValue,
      "chat stream-input response",
    );
    const status = requireContractString(
      requireContractField(payload, "status", "chat stream-input response"),
      "chat stream-input response.status",
    );
    if (status !== "queued" && status !== "no_active_session") {
      throw new GatewayContractError(
        "chat stream-input response.status",
        'must be "queued" or "no_active_session"',
      );
    }
    if (hasContractField(payload, "threadStatus")) {
      requireContractString(
        payload.threadStatus,
        "chat stream-input response.threadStatus",
      );
    }
    return {
      status,
      threadId: requireContractNonEmptyString(
        requireContractField(
          payload,
          "threadId",
          "chat stream-input response",
        ),
        "chat stream-input response.threadId",
      ),
      clientIntentId: optionalChatResponseString(
        payload,
        "clientIntentId",
        "chat stream-input response",
      ),
      pendingInputId: optionalChatResponseString(
        payload,
        "pendingInputId",
        "chat stream-input response",
      ),
    };
  } catch (error) {
    if (error instanceof GatewayContractError) {
      throw error;
    }
    // Preserve the old local-only response when the gateway cannot be reached.
  }
  return {
    status: "no_active_session",
    threadId,
    clientIntentId: input.clientIntentId,
  };
}

export function buildStreamInputRequestBody(input: SendMessageInput): string {
  const serializedAttachments = serializeMessageAttachments(
    input.images,
    input.files,
  );
  return JSON.stringify({
    threadId: resolveInputThreadId(input),
    clientIntentId: input.clientIntentId,
    message: input.message,
    attachments: serializedAttachments.attachments,
    images: serializedAttachments.images,
    files: serializedAttachments.files,
    metadata: {
      client_timestamp_local: input.clientTimestampLocal,
    },
  });
}

export async function interruptThread(
  settings: DesktopSettings,
  threadId: string,
): Promise<InterruptResult> {
  try {
    const payloadValue = await requestJson<unknown>(
      settings,
      "/api/chat/interrupt",
      "mutationSingleAttempt",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ threadId }),
        signal: AbortSignal.timeout(8000),
      },
    );
    const payload = requireContractRecord(payloadValue, "chat interrupt response");
    const status = requireContractString(
      requireContractField(payload, "status", "chat interrupt response"),
      "chat interrupt response.status",
    );
    if (status !== "interrupted" && status !== "not_found") {
      throw new GatewayContractError(
        "chat interrupt response.status",
        'must be "interrupted" or "not_found"',
      );
    }
    const abortedRunsPayload = requireContractArray(
      requireContractField(
        payload,
        "abortedRuns",
        "chat interrupt response",
      ),
      "chat interrupt response.abortedRuns",
    );
    return {
      status,
      threadId: requireContractNonEmptyString(
        requireContractField(payload, "threadId", "chat interrupt response"),
        "chat interrupt response.threadId",
      ),
      abortedRuns: abortedRunsPayload.map((entry, index) =>
        requireContractNonEmptyString(
          entry,
          `chat interrupt response.abortedRuns[${index}]`,
        ),
      ),
    };
  } catch (error) {
    if (error instanceof GatewayContractError) {
      throw error;
    }
    // Preserve local abort behavior when the gateway is unreachable.
  }
  return {
    status: "local_abort_only",
    threadId,
    abortedRuns: [],
  };
}

export const interruptSession = interruptThread;
