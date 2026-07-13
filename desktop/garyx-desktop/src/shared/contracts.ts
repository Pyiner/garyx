// Barrel: re-exports the shared desktop contracts, split by domain.
// Importers depend on `./contracts` / `@shared/contracts`; keep this
// surface stable.

export * from "./contracts/settings.ts";
export * from "./contracts/provider.ts";
export * from "./contracts/workspace.ts";
export * from "./contracts/transcript.ts";
export * from "./contracts/channel.ts";
export * from "./contracts/automation.ts";
export * from "./contracts/task.ts";
export * from "./contracts/capsule.ts";
export * from "./contracts/agent.ts";
export * from "./contracts/catalog.ts";
export * from "./contracts/bot.ts";
export * from "./contracts/thread.ts";
export * from "./contracts/state.ts";
export * from "./contracts/browser-terminal.ts";
export * from "./contracts/update.ts";
export * from "./contracts/window-layout.ts";
export * from "./contracts/desktop-api.ts";
