/**
 * Diagnostic panel listing every channel the gateway knows about,
 * rendered purely from the schema-driven
 * `GET /api/channels/plugins` catalog.
 *
 * This component is the visible proof that the catalog endpoint +
 * IPC wire work — the later settings-panel rewrite (Step 3C)
 * replaces the hardcoded per-channel iteration with the same shape.
 * Keeping a lightweight panel around even after that migration is
 * useful: it's the fastest way to see at a glance whether a
 * subprocess plugin is `running` vs `error` with its last
 * error message visible.
 */
import { useMemo, type ReactElement } from "react";

import type { ChannelPluginCatalogEntry } from "@shared/contracts";

import { useChannelPluginCatalog } from "./useChannelPluginCatalog";

export function ChannelPluginCatalogPanel(): ReactElement {
  const { entries, error, loading, refresh } = useChannelPluginCatalog();

  const rows = useMemo(() => entries ?? [], [entries]);

  return (
    <div className="channel-plugin-catalog">
      <div className="channel-plugin-catalog-header">
        <h3 className="channel-plugin-catalog-title">Channel plugins</h3>
        <button
          type="button"
          className="channel-plugin-catalog-refresh"
          disabled={loading}
          onClick={() => void refresh()}
        >
          {loading ? "Refreshing…" : "Refresh"}
        </button>
      </div>
      {error ? (
        <div className="channel-plugin-catalog-error">
          Couldn't fetch the plugin list: {error}
        </div>
      ) : null}
      {entries === null && !error ? (
        <div className="channel-plugin-catalog-empty">Loading…</div>
      ) : rows.length === 0 ? (
        <div className="channel-plugin-catalog-empty">
          No channels detected. Install one with{" "}
          <code>garyx plugins install &lt;path&gt;</code>.
        </div>
      ) : (
        <ul className="channel-plugin-catalog-list">
          {rows.map((entry) => (
            <ChannelPluginRow key={entry.id} entry={entry} />
          ))}
        </ul>
      )}
    </div>
  );
}

interface ChannelPluginRowProps {
  entry: ChannelPluginCatalogEntry;
}

function ChannelPluginRow({ entry }: ChannelPluginRowProps): ReactElement {
  return (
    <li className="channel-plugin-catalog-row" data-state={entry.state}>
      <div className="channel-plugin-catalog-row-logo">
        {entry.icon_data_url ? (
          <img
            src={entry.icon_data_url}
            alt={`${entry.display_name} icon`}
            className="channel-plugin-catalog-icon"
            width={28}
            height={28}
          />
        ) : (
          <div className="channel-plugin-catalog-icon-fallback" aria-hidden>
            {initial(entry.display_name || entry.id)}
          </div>
        )}
      </div>
      <div className="channel-plugin-catalog-row-body">
        <div className="channel-plugin-catalog-row-title">
          {entry.display_name || entry.id}
          <span className="channel-plugin-catalog-row-version">
            v{entry.version}
          </span>
        </div>
        <div className="channel-plugin-catalog-row-meta">
          <span data-tone={stateTone(entry.state)}>{entry.state}</span>
          <span>·</span>
          <span>
            {entry.accounts.length}{" "}
            {entry.accounts.length === 1 ? "account" : "accounts"}
          </span>
        </div>
        {entry.last_error ? (
          <div className="channel-plugin-catalog-row-error">
            Last error: {entry.last_error}
          </div>
        ) : null}
      </div>
    </li>
  );
}

function stateTone(state: string): "success" | "warn" | "danger" | "neutral" {
  switch (state) {
    case "running":
    case "ready":
      return "success";
    case "initializing":
    case "loaded":
      return "neutral";
    case "error":
      return "danger";
    case "stopped":
      return "warn";
    default:
      return "neutral";
  }
}

function initial(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return "?";
  return trimmed.charAt(0).toUpperCase();
}
