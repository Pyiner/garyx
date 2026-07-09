import type {
  DeleteCapsuleInput,
  DesktopCapsuleHtmlResult,
  DesktopCapsuleSummary,
  DesktopCapsulesPage,
  DesktopSettings,
} from "@shared/contracts";
import { GatewayRequestError, asFiniteNumber, asString, requestJson, requestText } from "./http.ts";

interface CapsulePayload {
  id?: string;
  title?: string | null;
  description?: string | null;
  thread_id?: string | null;
  threadId?: string | null;
  run_id?: string | null;
  runId?: string | null;
  agent_id?: string | null;
  agentId?: string | null;
  provider_type?: string | null;
  providerType?: string | null;
  html_sha256?: string | null;
  htmlSha256?: string | null;
  byte_size?: number | null;
  byteSize?: number | null;
  revision?: number | null;
  created_at?: string | null;
  createdAt?: string | null;
  updated_at?: string | null;
  updatedAt?: string | null;
}

interface CapsulesPayload {
  capsules?: CapsulePayload[];
  capsule?: CapsulePayload | null;
}

function normalizeCapsuleProviderType(value: unknown): DesktopCapsuleSummary["providerType"] {
  return asString(value) || null;
}

function mapCapsuleSummary(value: CapsulePayload): DesktopCapsuleSummary | null {
  const id = asString(value.id);
  if (!id) {
    return null;
  }
  const byteSize =
    asFiniteNumber(value.byte_size) ?? asFiniteNumber(value.byteSize) ?? 0;
  const revision = asFiniteNumber(value.revision) ?? 1;
  return {
    id,
    title: asString(value.title) || "Untitled Capsule",
    description: asString(value.description) || "",
    threadId: asString(value.thread_id) || asString(value.threadId) || null,
    runId: asString(value.run_id) || asString(value.runId) || null,
    agentId: asString(value.agent_id) || asString(value.agentId) || null,
    providerType: normalizeCapsuleProviderType(value.provider_type ?? value.providerType),
    htmlSha256: asString(value.html_sha256) || asString(value.htmlSha256) || "",
    byteSize: Math.max(0, Math.trunc(byteSize)),
    revision: Math.max(1, Math.trunc(revision)),
    createdAt:
      asString(value.created_at) ||
      asString(value.createdAt) ||
      new Date(0).toISOString(),
    updatedAt:
      asString(value.updated_at) ||
      asString(value.updatedAt) ||
      new Date(0).toISOString(),
  };
}

function mapCapsulesPage(payload: CapsulesPayload): DesktopCapsulesPage {
  const capsules = Array.isArray(payload.capsules)
    ? payload.capsules
        .map(mapCapsuleSummary)
        .filter((capsule): capsule is DesktopCapsuleSummary => Boolean(capsule))
    : [];
  return { capsules };
}

export async function listCapsules(
  settings: DesktopSettings,
): Promise<DesktopCapsulesPage> {
  const payload = await requestJson<CapsulesPayload>(settings, "/api/capsules", {
    signal: AbortSignal.timeout(8000),
  });
  return mapCapsulesPage(payload);
}

export async function getCapsule(
  settings: DesktopSettings,
  capsuleId: string,
): Promise<DesktopCapsuleSummary | null> {
  const id = capsuleId?.trim() || "";
  if (!id) {
    throw new Error("capsuleId is required");
  }
  try {
    const payload = await requestJson<CapsulesPayload>(
      settings,
      `/api/capsules/${encodeURIComponent(id)}`,
      { signal: AbortSignal.timeout(8000) },
    );
    return payload.capsule ? mapCapsuleSummary(payload.capsule) : null;
  } catch (error) {
    if (error instanceof GatewayRequestError && error.status === 404) {
      return null;
    }
    throw error;
  }
}

export async function getCapsuleHtml(
  settings: DesktopSettings,
  capsuleId: string,
): Promise<DesktopCapsuleHtmlResult> {
  const id = capsuleId?.trim() || "";
  if (!id) {
    throw new Error("capsuleId is required");
  }
  try {
    const html = await requestText(
      settings,
      `/api/capsules/${encodeURIComponent(id)}/serve`,
      { signal: AbortSignal.timeout(15000) },
    );
    return { status: "ok", html };
  } catch (error) {
    // A hard delete returns 404 from `/serve`: surface it as a value so callers
    // render a "Capsule deleted" tombstone. Transient/5xx/offline failures stay
    // rejections so the renderer keeps them retryable and never mislabels them.
    if (error instanceof GatewayRequestError && error.status === 404) {
      return { status: "deleted" };
    }
    throw error;
  }
}

export async function deleteCapsule(
  settings: DesktopSettings,
  input: DeleteCapsuleInput,
): Promise<void> {
  const id = input.capsuleId?.trim() || "";
  if (!id) {
    throw new Error("capsuleId is required");
  }
  await requestJson<unknown>(settings, `/api/capsules/${encodeURIComponent(id)}`, {
    method: "DELETE",
    signal: AbortSignal.timeout(8000),
  });
}
