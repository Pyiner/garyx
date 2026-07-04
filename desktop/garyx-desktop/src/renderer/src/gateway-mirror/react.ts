// Thin React bindings for the GatewayMirror (endgame architecture).
//
// The context carries only the stable mirror instance (identity never
// changes, so provider updates cost zero re-renders); volatile data enters
// React exclusively through useSyncExternalStore subscriptions.

import { createContext, useContext, useSyncExternalStore } from "react";

import type {
  CatalogSnapshot,
  GatewayMirror,
  GatewayRootSnapshot,
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
