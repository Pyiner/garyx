import type { DesktopGatewayMutationResult } from "@shared/contracts";

export const LIFECYCLE_JOIN_WINDOW_MS = 6_000;
export const LIFECYCLE_TRANSPORT_TIMEOUT_MS = 8_000;
export const LIFECYCLE_RETRY_DELAYS_MS = [1_000, 2_000, 4_000, 8_000, 8_000] as const;

export interface LifecycleIngressIdentity {
  gatewayScope: string;
  runtimeEpoch: number;
}

export interface LifecycleIngressRequest extends LifecycleIngressIdentity {
  operationId: string;
  expectedStoreIncarnation: string;
  threadId: string;
}

export interface LifecycleIngressAttempt extends LifecycleIngressRequest {
  /** One-based transport attempt number. Every dispatch remains single-shot. */
  attemptNumber: number;
}

export type LifecycleIngressCompletion<T> =
  | { kind: "applied"; value: T; attempts: number }
  | {
      kind: "rejected";
      code: string;
      message: string;
      attempts: number;
    }
  | {
      kind: "operationIdConflict";
      message: string;
      attempts: number;
    }
  | { kind: "exhausted"; message: string; attempts: number }
  | { kind: "cancelled"; attempts: number };

export interface LifecycleUiSettlement {
  rollbackOptimistic: boolean;
  requireFullReplacement: boolean;
  errorMessage: string | null;
  operationIdConflict: boolean;
}

export type LifecycleAttemptDecision<T> =
  | { kind: "applied"; value: T }
  | { kind: "rejected"; code: string; message: string }
  | { kind: "operationIdConflict"; message: string }
  | { kind: "retry"; message: string };

export interface LifecycleIngressRuntime {
  isCurrent: (identity: LifecycleIngressIdentity) => boolean;
  sleep?: (delayMs: number) => Promise<void>;
}

/**
 * Resolves the store identity only when every loaded feed agrees. Missing
 * feeds are tolerated; a torn cross-feed identity must be refreshed before a
 * lifecycle operation is dispatched.
 */
export function resolveLifecycleStoreIncarnation(
  candidates: ReadonlyArray<string | null | undefined>,
): string | null {
  const identities = new Set(
    candidates
      .map((candidate) => candidate?.trim() ?? "")
      .filter((candidate) => candidate.length > 0),
  );
  return identities.size === 1 ? (identities.values().next().value ?? null) : null;
}

/**
 * Converts the terminal ingress result into the three UI cleanup paths:
 * applied commits and replaces, rejected restores, and exhausted/unknown
 * restores then replaces. Conflict is the explicit client-bug terminal case.
 */
export function lifecycleUiSettlement<T>(
  completion: LifecycleIngressCompletion<T>,
): LifecycleUiSettlement {
  switch (completion.kind) {
    case "applied":
      return {
        rollbackOptimistic: false,
        requireFullReplacement: true,
        errorMessage: null,
        operationIdConflict: false,
      };
    case "rejected":
      return {
        rollbackOptimistic: true,
        requireFullReplacement: completion.code === "wrong_incarnation",
        errorMessage: completion.message,
        operationIdConflict: false,
      };
    case "operationIdConflict":
      return {
        rollbackOptimistic: true,
        requireFullReplacement: true,
        errorMessage: completion.message,
        operationIdConflict: true,
      };
    case "exhausted":
      return {
        rollbackOptimistic: true,
        requireFullReplacement: true,
        errorMessage: completion.message,
        operationIdConflict: false,
      };
    case "cancelled":
      return {
        rollbackOptimistic: true,
        requireFullReplacement: false,
        errorMessage: null,
        operationIdConflict: false,
      };
  }
}

/**
 * Classifies one already-single-shot transport result. Only results whose
 * durable outcome is still unknown enter the retry lane. In particular,
 * `operation_in_progress` is deliberately not treated as a 409 rejection.
 */
export function classifyLifecycleAttempt<T>(
  result: DesktopGatewayMutationResult<T>,
): LifecycleAttemptDecision<T> {
  switch (result.kind) {
    case "ok":
      return { kind: "applied", value: result.value };
    case "ambiguous":
    case "notSent":
      return { kind: "retry", message: result.message };
    case "definitiveEndpointResponse": {
      const message = result.error.message || result.error.code;
      if (
        result.error.code === "operation_in_progress" ||
        result.error.code === "unavailable"
      ) {
        return { kind: "retry", message };
      }
      if (result.error.code === "operation_id_conflict") {
        return { kind: "operationIdConflict", message };
      }
      return {
        kind: "rejected",
        code: result.error.code,
        message,
      };
    }
  }
}

/**
 * Renderer-owned lifecycle state machine. The operation identity is captured
 * once and passed unchanged to every dispatch; Main remains an IO-only,
 * single-attempt host. A gateway-scope/runtime-epoch change retires the loop.
 */
export async function runLifecycleMutation<T>(
  request: LifecycleIngressRequest,
  dispatch: (
    attempt: LifecycleIngressAttempt,
  ) => Promise<DesktopGatewayMutationResult<T>>,
  runtime: LifecycleIngressRuntime,
): Promise<LifecycleIngressCompletion<T>> {
  const sleep = runtime.sleep ?? sleepFor;
  let latestMessage = "Thread lifecycle result was unavailable.";

  for (let retryIndex = 0; ; retryIndex += 1) {
    const attempts = retryIndex + 1;
    if (!runtime.isCurrent(request)) {
      return { kind: "cancelled", attempts: retryIndex };
    }

    let result: DesktopGatewayMutationResult<T>;
    try {
      result = await dispatch({ ...request, attemptNumber: attempts });
    } catch (error) {
      result = {
        kind: "ambiguous",
        message:
          error instanceof Error
            ? error.message
            : "Thread lifecycle result was unavailable.",
      };
    }

    if (!runtime.isCurrent(request)) {
      return { kind: "cancelled", attempts };
    }

    const decision = classifyLifecycleAttempt(result);
    switch (decision.kind) {
      case "applied":
        return { kind: "applied", value: decision.value, attempts };
      case "rejected":
        return { ...decision, attempts };
      case "operationIdConflict":
        return { ...decision, attempts };
      case "retry":
        latestMessage = decision.message || latestMessage;
        break;
    }

    const delayMs = LIFECYCLE_RETRY_DELAYS_MS[retryIndex];
    if (delayMs === undefined) {
      return { kind: "exhausted", message: latestMessage, attempts };
    }
    await sleep(delayMs);
  }
}

function sleepFor(delayMs: number): Promise<void> {
  return new Promise((resolve) => globalThis.setTimeout(resolve, delayMs));
}
