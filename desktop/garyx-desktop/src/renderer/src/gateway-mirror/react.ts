// Thin React bindings for the GatewayMirror (endgame architecture).
//
// The context carries only the stable mirror instance (identity never
// changes, so provider updates cost zero re-renders); volatile data enters
// React exclusively through useSyncExternalStore subscriptions.

import {
  createContext,
  useCallback,
  useContext,
  useSyncExternalStore,
} from "react";

import type {
  CatalogSnapshot,
  GatewayMirror,
  GatewayRootSnapshot,
  ThreadMirrorSnapshot,
} from "./mirror.ts";

export const GatewayMirrorContext = createContext<GatewayMirror | null>(null);

export function useGatewayMirror(): GatewayMirror {
  const mirror = useContext(GatewayMirrorContext);
  if (!mirror) {
    throw new Error("GatewayMirrorContext is not provided");
  }
  return mirror;
}

export function useGatewayRoot(): GatewayRootSnapshot {
  const mirror = useGatewayMirror();
  return useSyncExternalStore(
    (onChange) => mirror.subscribeRoot(onChange),
    () => mirror.getRootSnapshot(),
    () => mirror.getRootSnapshot(),
  );
}

export function useCatalog(): CatalogSnapshot {
  const mirror = useGatewayMirror();
  return useSyncExternalStore(
    (onChange) => mirror.subscribeCatalog(onChange),
    () => mirror.getCatalogSnapshot(),
    () => mirror.getCatalogSnapshot(),
  );
}

/**
 * Read a single thread's mirror snapshot via useSyncExternalStore.
 * Returns null only when threadId is null. A non-null missing id receives the
 * mirror's stable empty snapshot for that id until its first commit.
 */
export function useThreadMirror(
  threadId: string | null,
): ThreadMirrorSnapshot | null {
  const mirror = useGatewayMirror();
  return useGatewayThreadMirror(mirror, threadId);
}

/**
 * Explicit-instance variant for owners that render the context provider
 * themselves (notably AppShell). Both bindings share the same per-thread
 * subscription and stable-snapshot contract.
 */
export function useGatewayThreadMirror(
  mirror: GatewayMirror,
  threadId: string | null,
): ThreadMirrorSnapshot | null {
  const subscribe = useCallback(
    (onChange: () => void) => {
      if (!threadId) {
        return () => {};
      }
      return mirror.subscribeThread(threadId, onChange);
    },
    [mirror, threadId],
  );
  const getSnapshot = useCallback(() => {
    if (!threadId) {
      return null;
    }
    return mirror.getThreadSnapshot(threadId);
  }, [mirror, threadId]);
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
