import { net, session } from "electron";

import {
  setGatewayFetch,
  setGatewayStreamFetch,
} from "./gary-client/http.ts";

/**
 * In-memory session partition that carries only the per-thread SSE streams.
 * Isolating them from the default session keeps the streams' long-lived
 * connections out of the control plane's 6-per-host socket pool, so live
 * streams can never starve ordinary gateway requests into their AbortSignal
 * timeouts (#TASK-1840). No `persist:` prefix: nothing here needs disk state.
 */
export const GATEWAY_STREAM_SESSION_PARTITION = "gateway-sse";

/**
 * Route gateway HTTP through Chromium's network stack (Electron `net`) so
 * requests honor the OS system proxy. Node's global `fetch` (undici) ignores
 * the system proxy and resolves DNS locally, which makes a remote gateway
 * whose hostname resolves to a private/off-LAN address (split-horizon DNS
 * pointing at a home-LAN IP that is only reachable through a proxy/VPN
 * tunnel) unreachable directly. Localhost gateways still connect directly
 * (the system proxy bypasses loopback). Both sessions resolve the proxy the
 * same way; they differ only in socket pools.
 */
export function wireGatewayTransport(): void {
  setGatewayFetch((input, init) => net.fetch(input, init));
  setGatewayStreamFetch((input, init) =>
    session.fromPartition(GATEWAY_STREAM_SESSION_PARTITION).fetch(input, init),
  );
}
