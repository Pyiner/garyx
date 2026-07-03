// Barrel: re-exports the gary-client gateway client, split by domain.
// Importers depend on `./gary-client`; keep this surface stable.

export * from "./gary-client/threads.ts";
export * from "./gary-client/workspaces.ts";
export * from "./gary-client/tasks.ts";
export * from "./gary-client/automations.ts";
export * from "./gary-client/workflows.ts";
export * from "./gary-client/capsules.ts";
export * from "./gary-client/dreams.ts";
export * from "./gary-client/agents.ts";
export * from "./gary-client/catalog.ts";
export * from "./gary-client/provider.ts";
export * from "./gary-client/channels.ts";
export * from "./gary-client/bots.ts";
export * from "./gary-client/gateway.ts";

export {
  GatewayRequestError,
  requestJson,
  requestText,
  setGatewayFetch,
} from "./gary-client/http.ts";
export type { GatewayFetch } from "./gary-client/http.ts";
export {
  ThreadStreamGapError,
  streamThreadEvents,
  openChatStream,
  sendStreamingInput,
  interruptThread,
  interruptSession,
} from "./gary-client/stream.ts";
