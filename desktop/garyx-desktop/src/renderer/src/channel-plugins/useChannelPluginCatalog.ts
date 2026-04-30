/**
 * React hook: fetch the gateway's schema-driven channel-plugin
 * catalog (`GET /api/channels/plugins`) and keep it in component
 * state, with manual refresh.
 *
 * IMPORTANT design invariant: the Mac App never reads local files.
 * Icons arrive as base64 `data:` URLs baked into the catalog
 * payload by the gateway — a remote gateway works identically.
 */
import { useCallback, useEffect, useState } from "react";

import type { ChannelPluginCatalogEntry } from "@shared/contracts";
import { measureUiAction } from "../perf-metrics";

export interface ChannelPluginCatalogState {
  /** `null` until the first successful fetch; the empty array is a
   * valid "no channels installed" state. */
  entries: ChannelPluginCatalogEntry[] | null;
  /** Error message from the most recent fetch, or `null` when fresh. */
  error: string | null;
  /** True while a fetch is in flight (initial load OR refresh). */
  loading: boolean;
  /** Trigger a re-fetch. Cheap; backed by the IPC round-trip to the
   * gateway. Callers should use this after mutations that could
   * change the catalog (account add/remove, plugin install). */
  refresh: () => Promise<void>;
}

type CatalogSnapshot = Omit<ChannelPluginCatalogState, "refresh">;

const catalogListeners = new Set<() => void>();
let cachedEntries: ChannelPluginCatalogEntry[] | null = null;
let cachedError: string | null = null;
let cachedLoading = false;
let catalogRequest: Promise<void> | null = null;

function currentSnapshot(): CatalogSnapshot {
  return {
    entries: cachedEntries,
    error: cachedError,
    loading: cachedLoading,
  };
}

function emitCatalogChange() {
  for (const listener of catalogListeners) {
    listener();
  }
}

async function loadChannelPluginCatalog(force = false): Promise<void> {
  if (!force && cachedEntries !== null) {
    return;
  }
  if (catalogRequest) {
    return catalogRequest;
  }

  cachedLoading = true;
  emitCatalogChange();

  catalogRequest = measureUiAction("channel_plugin_catalog.fetch", async () => {
    try {
      const api = window.garyxDesktop;
      if (!api?.fetchChannelPlugins) {
        cachedEntries = [];
        cachedError = null;
        return;
      }
      cachedEntries = await api.fetchChannelPlugins();
      cachedError = null;
    } catch (err) {
      cachedError = err instanceof Error ? err.message : String(err);
    } finally {
      cachedLoading = false;
      catalogRequest = null;
      emitCatalogChange();
    }
  });

  return catalogRequest;
}

export function useChannelPluginCatalog(): ChannelPluginCatalogState {
  const [snapshot, setSnapshot] = useState<CatalogSnapshot>(
    currentSnapshot,
  );

  const refresh = useCallback(async () => {
    await loadChannelPluginCatalog(true);
  }, []);

  useEffect(() => {
    const listener = () => {
      setSnapshot(currentSnapshot());
    };
    catalogListeners.add(listener);
    void loadChannelPluginCatalog(false);
    return () => {
      catalogListeners.delete(listener);
    };
  }, []);

  return { ...snapshot, refresh };
}
