import { useCallback, useEffect, useRef, useState } from "react";

import type { DesktopThreadFavoritesPage } from "@shared/contracts";

import {
  completeFavoritesSnapshot,
  createFavoritesIngressState,
  failFavoritesSnapshot,
  favoriteIsPresented,
  fireFavoriteBackoff,
  observeStoreIdentity,
  presentedFavoriteRows,
  replaceFavoritesGatewayScope,
  requestFavoritesSnapshot,
  settleFavoriteMutation,
  toggleFavoriteIntent,
  type FavoriteMutationSettlement,
  type FavoritesIngressEffect,
  type FavoritesIngressState,
  type FavoritesTransition,
  type StoreIdentityDecision,
  type StoreResponseStamp,
} from "./favorites-ingress";

type ThreadFavoritesControllerOptions = {
  enabled: boolean;
  gatewayScope: string;
  onError: (message: string) => void;
};

export type ThreadFavoritesController = {
  state: FavoritesIngressState;
  favoriteThreads: ReturnType<typeof presentedFavoriteRows>;
  isFavorite: (threadId: string) => boolean;
  setFavorite: (threadId: string, desired: boolean) => void;
  toggleFavorite: (threadId: string) => void;
  refreshSnapshot: () => void;
  observeStoreResponse: (
    stamp: StoreResponseStamp,
    storeIncarnationId: string,
  ) => StoreIdentityDecision;
};

export function useThreadFavorites({
  enabled,
  gatewayScope,
  onError,
}: ThreadFavoritesControllerOptions): ThreadFavoritesController {
  const [state, setState] = useState(() =>
    createFavoritesIngressState(gatewayScope),
  );
  const stateRef = useRef(state);
  const executeEffectRef = useRef<
    ((effect: FavoritesIngressEffect) => void) | null
  >(null);

  const apply = useCallback((transition: FavoritesTransition) => {
    stateRef.current = transition.state;
    setState(transition.state);
    for (const effect of transition.effects) {
      executeEffectRef.current?.(effect);
    }
    return transition.state;
  }, []);

  const executeEffect = useCallback(
    (effect: FavoritesIngressEffect) => {
      if (effect.kind === "backoff") {
        window.setTimeout(() => {
          apply(fireFavoriteBackoff(stateRef.current, effect.stamp));
        }, effect.delayMs);
        return;
      }
      if (effect.kind === "surfaceError") {
        onError(effect.message);
        return;
      }
      if (effect.kind === "snapshot") {
        void window.garyxDesktop
          .getThreadFavoritesSnapshot({
            gatewayScope: effect.ticket.gatewayScope,
          })
          .then((snapshot) => {
            apply(
              completeFavoritesSnapshot(
                stateRef.current,
                effect.ticket,
                snapshot,
              ),
            );
          })
          .catch(() => {
            apply(failFavoritesSnapshot(stateRef.current, effect.ticket));
          });
        return;
      }

      void window.garyxDesktop
        .setThreadFavorite({
          gatewayScope: effect.ticket.gatewayScope,
          threadId: effect.ticket.threadId,
          favorited: effect.ticket.target,
          expectedRevision: effect.ticket.expectedRevision,
          expectedStoreIncarnation:
            effect.ticket.expectedStoreIncarnation,
        })
        .then((result) => {
          let settlement: FavoriteMutationSettlement;
          switch (result.kind) {
            case "ok":
              settlement = { kind: "ok", page: result.value };
              break;
            case "definitiveEndpointResponse":
              settlement = {
                kind: "definitiveEndpointResponse",
                status: result.status,
                code: result.error.code,
                message: result.error.message,
                page: result.value as DesktopThreadFavoritesPage | null,
              };
              break;
            case "ambiguous":
              settlement = { kind: "ambiguous", message: result.message };
              break;
            case "notSent":
              settlement = { kind: "notSent", message: result.message };
              break;
          }
          apply(
            settleFavoriteMutation(
              stateRef.current,
              effect.ticket,
              settlement,
            ),
          );
        })
        .catch((error: unknown) => {
          apply(
            settleFavoriteMutation(stateRef.current, effect.ticket, {
              kind: "ambiguous",
              message:
                error instanceof Error
                  ? error.message
                  : "Favorite update result was unavailable",
            }),
          );
        });
    },
    [apply, onError],
  );
  executeEffectRef.current = executeEffect;

  useEffect(() => {
    apply(
      replaceFavoritesGatewayScope(
        stateRef.current,
        gatewayScope,
        enabled,
      ),
    );
  }, [apply, enabled, gatewayScope]);

  useEffect(() => {
    if (!enabled || !gatewayScope) {
      return;
    }
    const interval = window.setInterval(() => {
      apply(requestFavoritesSnapshot(stateRef.current));
    }, 10_000);
    return () => window.clearInterval(interval);
  }, [apply, enabled, gatewayScope]);

  const setFavorite = useCallback(
    (threadId: string, desired: boolean) => {
      apply(toggleFavoriteIntent(stateRef.current, threadId, desired));
    },
    [apply],
  );

  const toggleFavorite = useCallback(
    (threadId: string) => {
      setFavorite(
        threadId,
        !favoriteIsPresented(stateRef.current, threadId),
      );
    },
    [setFavorite],
  );

  const refreshSnapshot = useCallback(() => {
    apply(requestFavoritesSnapshot(stateRef.current));
  }, [apply]);

  const observeStoreResponse = useCallback(
    (stamp: StoreResponseStamp, storeIncarnationId: string) => {
      const transition = observeStoreIdentity(
        stateRef.current,
        stamp,
        storeIncarnationId,
      );
      apply(transition);
      return transition.decision;
    },
    [apply],
  );

  // Scope masking is synchronous: an old gateway's membership/rows never get
  // one render frame while the effect-owned clear is waiting to run.
  const visibleState =
    state.gatewayScope === gatewayScope
      ? state
      : replaceFavoritesGatewayScope(state, gatewayScope, false).state;
  if (stateRef.current.gatewayScope !== gatewayScope) {
    stateRef.current = visibleState;
  }

  return {
    state: visibleState,
    favoriteThreads: presentedFavoriteRows(visibleState),
    isFavorite: (threadId) => favoriteIsPresented(visibleState, threadId),
    setFavorite,
    toggleFavorite,
    refreshSnapshot,
    observeStoreResponse,
  };
}
