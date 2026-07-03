import type {
  DesktopDreamScan,
  DesktopDreamSpan,
  DesktopDreamTopic,
  DesktopDreamsPage,
  DesktopSettings,
  ListDreamsInput,
  ScanDreamsInput,
} from "@shared/contracts";
import { requestJson } from "./http.ts";

interface DreamSpanPayload {
  span_id?: string;
  spanId?: string;
  dream_id?: string;
  dreamId?: string;
  thread_id?: string;
  threadId?: string;
  workspace_dir?: string | null;
  workspacePath?: string | null;
  start_seq?: number;
  startSeq?: number;
  end_seq?: number;
  endSeq?: number;
  start_at?: string;
  startAt?: string;
  end_at?: string;
  endAt?: string;
  excerpt?: string | null;
  message_count?: number;
  messageCount?: number;
}

interface DreamTopicPayload {
  dream_id?: string;
  dreamId?: string;
  title?: string | null;
  summary?: string | null;
  first_message_at?: string;
  firstMessageAt?: string;
  last_message_at?: string;
  lastMessageAt?: string;
  updated_at?: string;
  updatedAt?: string;
  source?: string | null;
  confidence?: number | null;
  message_count?: number;
  messageCount?: number;
  span_count?: number;
  spanCount?: number;
  spans?: DreamSpanPayload[];
}

interface DreamScanPayload {
  run_id?: string;
  runId?: string;
  scanned_from?: string;
  scannedFrom?: string;
  scanned_to?: string;
  scannedTo?: string;
  created_at?: string;
  createdAt?: string;
  source?: string | null;
  status?: string | null;
  topics_count?: number;
  topicsCount?: number;
  spans_count?: number;
  spansCount?: number;
  error?: string | null;
}

interface DreamsPayload {
  dreams?: DreamTopicPayload[];
  dream?: DreamTopicPayload | null;
  count?: number;
  from?: string;
  to?: string;
  latest_scan?: DreamScanPayload | null;
  latestScan?: DreamScanPayload | null;
  scan?: DreamScanPayload | null;
}

function mapDreamScan(value?: DreamScanPayload | null): DesktopDreamScan | null {
  if (!value) {
    return null;
  }
  return {
    runId: value.run_id || value.runId || "",
    scannedFrom: value.scanned_from || value.scannedFrom || "",
    scannedTo: value.scanned_to || value.scannedTo || "",
    createdAt: value.created_at || value.createdAt || "",
    source: value.source?.trim() || "unknown",
    status: value.status?.trim() || "unknown",
    topicsCount: value.topics_count ?? value.topicsCount ?? 0,
    spansCount: value.spans_count ?? value.spansCount ?? 0,
    error: value.error || null,
  };
}

function mapDreamSpan(value: DreamSpanPayload): DesktopDreamSpan {
  return {
    spanId: value.span_id || value.spanId || "",
    dreamId: value.dream_id || value.dreamId || "",
    threadId: value.thread_id || value.threadId || "",
    workspacePath: value.workspace_dir || value.workspacePath || null,
    startSeq: value.start_seq ?? value.startSeq ?? 0,
    endSeq: value.end_seq ?? value.endSeq ?? 0,
    startAt: value.start_at || value.startAt || "",
    endAt: value.end_at || value.endAt || "",
    excerpt: value.excerpt?.trim() || "",
    messageCount: value.message_count ?? value.messageCount ?? 0,
  };
}

function mapDreamTopic(value: DreamTopicPayload): DesktopDreamTopic {
  const spans = Array.isArray(value.spans)
    ? value.spans.map(mapDreamSpan).filter((span) => span.threadId)
    : [];
  return {
    dreamId: value.dream_id || value.dreamId || "",
    title: value.title?.trim() || "Untitled Dream",
    summary: value.summary?.trim() || "",
    firstMessageAt: value.first_message_at || value.firstMessageAt || "",
    lastMessageAt: value.last_message_at || value.lastMessageAt || "",
    updatedAt: value.updated_at || value.updatedAt || "",
    source: value.source?.trim() || "unknown",
    confidence: value.confidence ?? 0,
    messageCount: value.message_count ?? value.messageCount ?? 0,
    spanCount: value.span_count ?? value.spanCount ?? spans.length,
    spans,
  };
}

function dreamListQuery(input: ListDreamsInput = {}): string {
  const query = new URLSearchParams();
  const from = input.from?.trim() || "";
  const to = input.to?.trim() || "";
  if (from) {
    query.set("from", from);
  } else {
    query.set("since_hours", String(Math.max(1, Math.min(24 * 31, input.sinceHours || 24))));
  }
  if (to) {
    query.set("to", to);
  }
  query.set("limit", String(Math.max(1, Math.min(500, input.limit || 80))));
  return query.toString();
}

function mapDreamsPage(payload: DreamsPayload): DesktopDreamsPage {
  const dreams = Array.isArray(payload.dreams)
    ? payload.dreams.map(mapDreamTopic).filter((dream) => dream.dreamId)
    : [];
  return {
    dreams,
    count: payload.count ?? dreams.length,
    from: payload.from || "",
    to: payload.to || "",
    latestScan: mapDreamScan(payload.latest_scan || payload.latestScan || null),
    scan: mapDreamScan(payload.scan || null),
  };
}

export async function listDreams(
  settings: DesktopSettings,
  input: ListDreamsInput = {},
): Promise<DesktopDreamsPage> {
  const payload = await requestJson<DreamsPayload>(
    settings,
    `/api/dreams?${dreamListQuery(input)}`,
    { signal: AbortSignal.timeout(8000) },
  );
  return mapDreamsPage(payload);
}

export async function scanDreams(
  settings: DesktopSettings,
  input: ScanDreamsInput = {},
): Promise<DesktopDreamsPage> {
  const body: Record<string, unknown> = {
    limit: Math.max(1, Math.min(2000, input.limit || 600)),
    mode: input.mode || "auto",
  };
  const from = input.from?.trim() || "";
  const to = input.to?.trim() || "";
  if (from) {
    body.from = from;
  } else {
    body.since_hours = Math.max(1, Math.min(24 * 31, input.sinceHours || 24));
  }
  if (to) {
    body.to = to;
  }
  const payload = await requestJson<DreamsPayload>(settings, "/api/dreams/scan", {
    method: "POST",
    signal: AbortSignal.timeout(180000),
    body: JSON.stringify(body),
  });
  return mapDreamsPage(payload);
}

export async function getDream(
  settings: DesktopSettings,
  dreamId: string,
): Promise<DesktopDreamTopic | null> {
  const payload = await requestJson<DreamsPayload>(
    settings,
    `/api/dreams/${encodeURIComponent(dreamId)}`,
    { signal: AbortSignal.timeout(8000) },
  );
  return payload.dream ? mapDreamTopic(payload.dream) : null;
}
