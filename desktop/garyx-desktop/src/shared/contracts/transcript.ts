export type TranscriptRole =
  | "assistant"
  | "system"
  | "user"
  | "tool"
  | "tool_use"
  | "tool_result";

export interface TranscriptMessage {
  id: string;
  // Raw transcript record seq (1-based). Stamped at the wire boundary — live
  // committed from the SSE envelope, history from `index + 1`. render_state row
  // refs carry this seq, so the renderer resolves bodies by seq rather than by
  // the message id, which is rewritten to a stable id across optimistic
  // reconciliation and so cannot be trusted to encode the seq.
  seq?: number;
  role: TranscriptRole;
  text: string;
  content?: unknown;
  input?: unknown;
  result?: unknown;
  toolUseId?: string | null;
  toolName?: string | null;
  toolUseResult?: boolean | null;
  toolRelated?: boolean | null;
  isError?: boolean;
  metadata?: Record<string, unknown> | null;
  timestamp?: string | null;
  pending?: boolean;
  error?: boolean;
  kind?: string;
  internal?: boolean;
  internalKind?: string | null;
  loopOrigin?: string | null;
}

export interface MessageImageAttachment {
  id: string;
  name: string;
  mediaType: string;
  path?: string;
  data?: string;
}

export interface MessageFileAttachment {
  id: string;
  name: string;
  mediaType: string;
  path?: string;
  data?: string;
}

export type ChatAttachmentKind = "image" | "file";

export interface UploadChatAttachmentBlob {
  kind: ChatAttachmentKind;
  name: string;
  mediaType?: string | null;
  dataBase64: string;
}

export interface UploadedChatAttachment {
  kind: ChatAttachmentKind;
  path: string;
  name: string;
  mediaType: string;
}

export interface UploadChatAttachmentsInput {
  files: UploadChatAttachmentBlob[];
}

export interface UploadChatAttachmentsResult {
  files: UploadedChatAttachment[];
}

// Wire mirror of `garyx-models` `RenderSnapshot` (transcript_render_state.rs).
// Platform-neutral semantic structure only: message bodies are referenced by
// `seq` and resolved against the local committed cache on the desktop side.
export type RenderTailActivity =
  | "none"
  | "thinking"
  | "assistant_streaming"
  | "tool_active";

export type RenderProgressLocus = "none" | "tail" | "tool_group";

export interface RenderMessageRef {
  id: string;
  seq: number;
  role: string;
}

export type RenderToolEntryStatus = "running" | "completed" | "failed";

export type RenderToolKind =
  | "command"
  | "file_read"
  | "file_write"
  | "file_edit"
  | "search"
  | "web"
  | "agent"
  | "task"
  | "image"
  | "system"
  | "generic";

export type RenderToolFieldRoot = "content" | "input" | "result" | "text";

export type RenderToolFieldFormat =
  | "text"
  | "code"
  | "path"
  | "json"
  | "diff"
  | "image";

export type RenderToolFieldLabel =
  | "call"
  | "command"
  | "file"
  | "query"
  | "url"
  | "prompt"
  | "parameters"
  | "content"
  | "output"
  | "result"
  | "response"
  | "diff"
  | "image"
  | "error";

export type RenderToolVisibility = "normal" | "nested" | "quiet" | "hidden";

export interface RenderToolFieldSelector {
  root: RenderToolFieldRoot;
  path?: string[];
  format: RenderToolFieldFormat;
  label: RenderToolFieldLabel;
}

export interface RenderToolFieldProjection {
  tool_name?: string;
  kind: RenderToolKind;
  visibility: RenderToolVisibility;
  summary?: RenderToolFieldSelector;
  call?: RenderToolFieldSelector;
  result?: RenderToolFieldSelector;
  status?: string;
  exit_code?: number;
  duration_ms?: number;
}

export interface RenderToolEntry {
  id: string;
  tool_use_id: string | null;
  status: RenderToolEntryStatus;
  tool_use: RenderMessageRef | null;
  tool_result: RenderMessageRef | null;
  projection?: RenderToolFieldProjection;
}

export type RenderToolGroupStatus = "active" | "completed";

export interface RenderToolGroup {
  kind: "tool_group";
  id: string;
  status: RenderToolGroupStatus;
  entries: RenderToolEntry[];
  started_at: string | null;
  finished_at: string | null;
}

export interface RenderAssistantStep {
  kind: "assistant_message";
  id: string;
  message: RenderMessageRef;
  streaming: boolean;
}

export type RenderStepItem = RenderAssistantStep | RenderToolGroup;

export interface RenderStepRow {
  kind: "step";
  id: string;
  steps: RenderStepItem[];
  final_message: RenderMessageRef | null;
  running: boolean;
  started_at: string | null;
  finished_at: string | null;
}

export interface RenderAssistantReplyRow {
  kind: "assistant_reply";
  id: string;
  message: RenderMessageRef;
  streaming: boolean;
}

export type RenderActivityRow = RenderAssistantReplyRow | RenderStepRow;

export type RenderCapsuleAction = "created" | "updated";

export interface RenderCapsuleCard {
  id: string;
  capsule_id: string;
  title: string;
  revision: number;
  action: RenderCapsuleAction;
}

export interface RenderUserTurnRow {
  kind: "user_turn";
  id: string;
  user: RenderMessageRef | null;
  activity: RenderActivityRow[];
  started_at: string | null;
  finished_at: string | null;
  capsule_cards?: RenderCapsuleCard[];
}

export type RenderRow = RenderUserTurnRow;

export type RenderPlaceholderFilterReason = "empty_streaming_assistant";

export interface RenderFilteredPlaceholder {
  message: RenderMessageRef;
  reason: RenderPlaceholderFilterReason;
}

export interface RenderRateLimit {
  provider?: string | null;
  resetAt?: string | null;
  window?: string | null;
  message?: string | null;
  willAutoResend: boolean;
}

export interface RenderStateWindow {
  floor_seq: number;
  has_more_above: boolean;
}

export interface RenderState {
  based_on_seq: number;
  rows: RenderRow[];
  tailActivity: RenderTailActivity;
  activeToolGroupId: string | null;
  progress_locus: RenderProgressLocus;
  filtered_placeholders: RenderFilteredPlaceholder[];
  rateLimit?: RenderRateLimit | null;
  window?: RenderStateWindow | null;
  /**
   * Opaque rows-hash chain token (#TASK-1956 knife 1). The server is the
   * only hasher; the client compares it by equality against
   * `RenderDelta.from_rows_hash` to validate the delta chain. Present on
   * every frame of a `render_mode=delta` connection; absent otherwise.
   */
  rows_hash?: string;
}

/**
 * Wire mirror of `garyx-models` `RenderDelta` (transcript_render_state.rs,
 * #TASK-1956 knife 1): the live-frame payload of a `render_mode=delta`
 * connection. Scalar fields are always sent whole; rows travel as the full
 * id order plus full bodies for new/changed rows only. Reassembly and
 * validation live in the main-process transport layer (garyx-client
 * stream); the renderer only ever sees full `RenderState` snapshots.
 */
export interface RenderDelta {
  /** The client must hold the snapshot at this seq... */
  from_seq: number;
  /** ...with exactly this rows content (chain token, drift tripwire). */
  from_rows_hash: string;
  based_on_seq: number;
  /** Chain token AFTER applying this delta; stored on accept. */
  rows_hash: string;
  /** Full row id sequence: re-order is unambiguous. */
  row_order: string[];
  /** Full bodies for new/changed rows only. */
  upsert_rows: RenderRow[];
  tailActivity: RenderTailActivity;
  activeToolGroupId: string | null;
  progress_locus: RenderProgressLocus;
  rateLimit?: RenderRateLimit | null;
  window?: RenderStateWindow | null;
  filtered_placeholders: RenderFilteredPlaceholder[];
}

export interface CommittedMessageEvent {
  type: "committed_message";
  /** Local main→renderer correlation only; absent on gateway wire payloads
   * and on committed events nested inside a thread-render frame. */
  requestId?: string;
  runId: string;
  threadId: string;
  sessionId?: string;
  seq: number;
  message: TranscriptMessage;
}

export type DesktopChatStreamEvent =
  | CommittedMessageEvent
  | {
      // One atomic per-thread render frame: the contiguous committed `events`
      // plus the full `renderState` snapshot derived at `based_on_seq`.
      type: "thread_render_frame";
      /** Logical renderer request that owns this locally forwarded frame. */
      requestId?: string;
      threadId: string;
      events: CommittedMessageEvent[];
      renderState: RenderState;
      /**
       * "windowed": the gateway degraded a stale opted-in resume to the
       * initial window; cached committed records below
       * renderState.window.floor_seq are no longer contiguous with this
       * connection and must be dropped, not appended to.
       */
      replay?: "windowed";
    }
  | {
      type: "error";
      /** Logical renderer request that owns this locally forwarded error. */
      requestId?: string;
      runId: string;
      threadId: string;
      sessionId?: string;
      error: string;
      terminal?: boolean;
    };

export type DesktopChatStreamListener = (event: DesktopChatStreamEvent) => void;
