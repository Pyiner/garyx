/**
 * Renderer-only thread ids shared by the app shell and GatewayMirror.
 * Draft state has no gateway history route, so it must never enter the
 * recoverable transcript-entry eviction pool.
 */
export const NEW_THREAD_DRAFT_THREAD_ID = "__garyx_new_thread_draft__";
