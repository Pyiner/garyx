/**
 * React hook: fetch the gateway's schema-driven channel-plugin
 * catalog (`GET /api/channels/plugins`) and keep it in component
 * state, with manual refresh.
 *
 * IMPORTANT design invariant: the Mac App never reads local files.
 * Icons arrive as base64 `data:` URLs baked into the catalog
 * payload by the gateway — a remote gateway works identically.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import type { ChannelPluginCatalogEntry } from "@shared/contracts";

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

export function useChannelPluginCatalog(): ChannelPluginCatalogState {
  const [entries, setEntries] = useState<ChannelPluginCatalogEntry[] | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  // Guard against state updates after unmount (React strict mode /
  // rapid navigation would otherwise surface a console warning).
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const api = window.garyxDesktop;
      if (!api?.fetchChannelPlugins) {
        // Electron preload didn't expose the method — likely running
        // in the web preview bundle. Degrade quietly rather than
        // throwing; callers can check `entries === null` to know.
        if (mountedRef.current) {
          setEntries([]);
          setError(null);
        }
        return;
      }
      const next = await api.fetchChannelPlugins();
      if (mountedRef.current) {
        setEntries(next);
        setError(null);
      }
    } catch (err) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) {
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    void refresh();
    return () => {
      mountedRef.current = false;
    };
  }, [refresh]);

  return { entries, error, loading, refresh };
}
