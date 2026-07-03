import type { DesktopApiProviderType } from "./provider.ts";

export interface DesktopDreamSpan {
  spanId: string;
  dreamId: string;
  threadId: string;
  workspacePath?: string | null;
  startSeq: number;
  endSeq: number;
  startAt: string;
  endAt: string;
  excerpt: string;
  messageCount: number;
}

export interface DesktopDreamTopic {
  dreamId: string;
  title: string;
  summary: string;
  firstMessageAt: string;
  lastMessageAt: string;
  updatedAt: string;
  source: string;
  confidence: number;
  messageCount: number;
  spanCount: number;
  spans: DesktopDreamSpan[];
}

export interface DesktopDreamScan {
  runId: string;
  scannedFrom: string;
  scannedTo: string;
  createdAt: string;
  source: string;
  status: string;
  topicsCount: number;
  spansCount: number;
  error?: string | null;
}

export interface DesktopDreamsPage {
  dreams: DesktopDreamTopic[];
  count: number;
  from: string;
  to: string;
  latestScan?: DesktopDreamScan | null;
  scan?: DesktopDreamScan | null;
}

export interface DesktopCapsuleSummary {
  id: string;
  title: string;
  description: string;
  threadId?: string | null;
  runId?: string | null;
  agentId?: string | null;
  providerType?: DesktopApiProviderType | string | null;
  htmlSha256: string;
  byteSize: number;
  revision: number;
  createdAt: string;
  updatedAt: string;
}

export interface DesktopCapsulesPage {
  capsules: DesktopCapsuleSummary[];
}

// Result of fetching a Capsule's served HTML through the main process. A hard
// delete surfaces as a value (`deleted`) so chat cards / preview can render a
// disabled tombstone; transient/5xx/offline failures stay rejections so the
// renderer keeps them retryable and never mislabels them deleted.
export type DesktopCapsuleHtmlResult =
  | { status: "ok"; html: string }
  | { status: "deleted" };

// Result of rendering a Capsule into a cached thumbnail PNG (gallery 16:10 /
// chat card 16:9). Mirrors `DesktopCapsuleHtmlResult`: `deleted` is a value so
// cards flip to a tombstone, while transient render/network failures surface as
// `error` and stay retryable. `dataUrl` is a `data:image/png;base64,…` string
// ready to bind to `<img src=…>`.
export type DesktopCapsuleThumbnailResult =
  | { status: "ok"; dataUrl: string }
  | { status: "deleted" }
  | { status: "error"; message: string };

export interface DeleteCapsuleInput {
  capsuleId: string;
}

export interface ListDreamsInput {
  from?: string | null;
  to?: string | null;
  sinceHours?: number;
  limit?: number;
}

export interface ScanDreamsInput extends ListDreamsInput {
  mode?: "auto" | "claude" | "heuristic";
}
