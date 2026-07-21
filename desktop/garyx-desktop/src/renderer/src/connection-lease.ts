import {
  currentPinnedOrderDomainGeneration,
  isCurrentPinnedOrderDomainGeneration,
} from "./pinned-order-ingress.ts";

/**
 * A connection lease for long-running renderer continuations (polling
 * timers, retry chains) that must die with the gateway connection they
 * were scheduled on.
 *
 * It binds BOTH connection identities, because they invalidate at
 * different moments and each covers the other's blind window:
 *
 * - The ingress DOMAIN GENERATION advances when a gateway switch is
 *   REQUESTED (and on rollback) — before the new state ever commits. It
 *   fences the window where transport already answers for the new
 *   gateway while the mirror still holds the old universe (a new-gateway
 *   answer applied there would persist into the OLD cache partition).
 * - The mirror CONNECTION EPOCH advances when the switch COMMITS. It
 *   fences everything scheduled before the commit, including same-URL
 *   A -> B -> A returns the generation alone would re-match... (it does
 *   not: the generation also advances per switch — the epoch covers
 *   consumers created before the ingress was installed and keeps the
 *   lease valid only while BOTH owners agree).
 *
 * isCurrent() is true only while neither identity has moved.
 */
export interface ConnectionLease {
  isCurrent(): boolean;
}

export function openConnectionLease(mirror: {
  readonly currentConnectionEpoch: number;
  isCurrentConnectionEpoch(epoch: number): boolean;
}): ConnectionLease {
  const epoch = mirror.currentConnectionEpoch;
  const generation = currentPinnedOrderDomainGeneration();
  return {
    isCurrent: () =>
      mirror.isCurrentConnectionEpoch(epoch) &&
      isCurrentPinnedOrderDomainGeneration(generation),
  };
}
