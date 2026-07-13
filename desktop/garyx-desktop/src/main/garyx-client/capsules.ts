import type {
  DeleteCapsuleInput,
  DesktopCapsuleHtmlResult,
  DesktopCapsuleSummary,
  DesktopCapsulesPage,
  DesktopSettings,
} from "@shared/contracts";
import {
  GatewayContractError,
  GatewayRequestError,
  requestJson,
  requestText,
  requireContractArray,
  requireContractField,
  requireContractNonEmptyString,
  requireContractNonNegativeInteger,
  requireContractRecord,
  requireContractString,
} from "./http.ts";

interface CapsulePayload {
  id?: string;
  title?: string | null;
  description?: string | null;
  thread_id?: string | null;
  run_id?: string | null;
  agent_id?: string | null;
  provider_type?: string | null;
  html_sha256?: string | null;
  byte_size?: number | null;
  revision?: number | null;
  created_at?: string | null;
  updated_at?: string | null;
}

interface CapsulesPayload {
  capsules?: CapsulePayload[];
  capsule?: CapsulePayload | null;
}

function requiredNullableString(
  record: Record<string, unknown>,
  field: string,
  path: string,
): string | null {
  const value = requireContractField(record, field, path);
  return value === null
    ? null
    : requireContractString(value, `${path}.${field}`);
}

function mapCapsuleSummary(value: unknown, path: string): DesktopCapsuleSummary {
  const record = requireContractRecord(value, path);
  const revision = requireContractNonNegativeInteger(
    requireContractField(record, "revision", path),
    `${path}.revision`,
  );
  if (revision < 1) {
    throw new GatewayContractError(`${path}.revision`, "must be at least 1");
  }
  return {
    id: requireContractNonEmptyString(
      requireContractField(record, "id", path),
      `${path}.id`,
    ),
    title: requireContractString(
      requireContractField(record, "title", path),
      `${path}.title`,
    ),
    description: requireContractString(
      requireContractField(record, "description", path),
      `${path}.description`,
    ),
    threadId: requiredNullableString(record, "thread_id", path),
    runId: requiredNullableString(record, "run_id", path),
    agentId: requiredNullableString(record, "agent_id", path),
    providerType: requiredNullableString(record, "provider_type", path),
    htmlSha256: requireContractNonEmptyString(
      requireContractField(record, "html_sha256", path),
      `${path}.html_sha256`,
    ),
    byteSize: requireContractNonNegativeInteger(
      requireContractField(record, "byte_size", path),
      `${path}.byte_size`,
    ),
    revision,
    createdAt: requireContractNonEmptyString(
      requireContractField(record, "created_at", path),
      `${path}.created_at`,
    ),
    updatedAt: requireContractNonEmptyString(
      requireContractField(record, "updated_at", path),
      `${path}.updated_at`,
    ),
  };
}

function mapCapsulesPage(payload: unknown): DesktopCapsulesPage {
  const record = requireContractRecord(payload, "capsule list");
  return {
    capsules: requireContractArray(
      requireContractField(record, "capsules", "capsule list"),
      "capsule list.capsules",
    ).map((capsule, index) =>
      mapCapsuleSummary(capsule, `capsule list.capsules[${index}]`),
    ),
  };
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
    const record = requireContractRecord(payload, "get capsule response");
    return mapCapsuleSummary(
      requireContractField(record, "capsule", "get capsule response"),
      "get capsule response.capsule",
    );
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
