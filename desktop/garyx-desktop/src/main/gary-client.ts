// Barrel: re-exports the garyx-client gateway client, split by domain.
// Importers depend on `./gary-client`; keep this surface stable. The barrel
// keeps its legacy `gary-client` filename only because in-flight WIP holds
// index.ts/thread-stream-hub.ts; rename it to `garyx-client.ts` (and update
// importers) once that lands.

export * from "./garyx-client/threads.ts";
export * from "./garyx-client/workspaces.ts";
export * from "./garyx-client/tasks.ts";
export * from "./garyx-client/automations.ts";
export * from "./garyx-client/capsules.ts";
export * from "./garyx-client/agents.ts";
export * from "./garyx-client/catalog.ts";
export * from "./garyx-client/provider.ts";
export * from "./garyx-client/channels.ts";
export * from "./garyx-client/bots.ts";
export * from "./garyx-client/gateway.ts";

export {
  GatewayRequestError,
  requestJson,
  requestText,
  setGatewayFetch,
  setGatewayStreamFetch,
} from "./garyx-client/http.ts";
export type { GatewayFetch } from "./garyx-client/http.ts";
export {
  ThreadStreamGapError,
  streamThreadEvents,
  openChatStream,
  sendStreamingInput,
  interruptThread,
  interruptSession,
} from "./garyx-client/stream.ts";
