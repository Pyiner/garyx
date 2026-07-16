import type { DesktopSettings } from "@shared/contracts";

import { gatewayStreamFetch, requestJson, requestMutationJson } from "./http.ts";

declare const settings: DesktopSettings;

if (false) {
  // These negative assertions pin the compile-time contract: adding a default
  // semantics mode must fail type-checking before a call site can silently
  // inherit retry behavior.
  // @ts-expect-error request semantics are mandatory
  void requestJson(settings, "/api/status");
  // @ts-expect-error classified mutations require the explicit single-attempt mode
  void requestMutationJson(settings, "/api/thread-favorites/thread::test");
  // @ts-expect-error streaming reads require their explicit semantic mode too
  void gatewayStreamFetch("https://gateway.example.test/api/stream");
}
