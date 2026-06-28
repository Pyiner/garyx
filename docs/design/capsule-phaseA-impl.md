# Capsule Phase A Implementation Design — Gateway Backend

This document implements `docs/design/capsule.md` sections 1-6 and 9 only. No desktop or iOS code is in scope.

## Scope and non-goals

In scope:

- `garyx-models` path helper for `~/.garyx/capsules`.
- Gateway-owned `capsules` SQLite table, structs, and CRUD in `garyx_db`.
- Three MCP tools: `capsule_create`, `capsule_update`, `capsule_list`.
- Four protected HTTP endpoints: list/get/serve/delete.
- Single-file HTML storage with atomic temp-write-and-rename, 5 MiB cap, HTML self-contained checks, CSP meta injection on serve, and safe response headers.
- Tests for DB, MCP, HTTP, validation, and path/id edge cases.

Out of scope for Phase A:

- Desktop and iOS UI, IPC, mobile models, app-target builds.
- Manual HTTP create/update/import endpoints.
- Pagination, soft delete, optimistic concurrency, version history, thumbnails, public sharing, sidecar resources, `capsule_get` MCP tool.

## Existing patterns to follow

- `garyx-models/src/local_paths.rs`: mirror `default_skills_dir()`.
- `garyx-gateway/src/garyx_db/mod.rs`: mirror `WorkspaceRecord`/`WorkspaceDraft` and `upsert_workspace`, but with hard delete and `revision` bump semantics.
- `garyx-gateway/src/mcp/tools/schedule_followup.rs`: `run()` extracts `RunContext`, calls `run_inner`, records metrics, serializes JSON string.
- `garyx-gateway/src/workspaces.rs`: HTTP module structure and DB error mapping.
- `garyx-gateway/src/route_graph.rs`: protected route registration beside `/api/workspaces`.

## File diff plan

### `docs/design/capsule.md`

Copy the final design blueprint from the local handoff source into the repository.

### `docs/design/capsule-phaseA-impl.md`

This file. Keep it as the implementation-level design reviewed before coding.

### `garyx-models/src/local_paths.rs`

Add:

```rust
pub fn default_capsules_dir() -> PathBuf {
    gary_home_dir().join("capsules")
}
```

Add a focused `local_paths` test asserting it is `HOME/.garyx/capsules`.

### `garyx-gateway/src/garyx_db/mod.rs`

Add public data types near `WorkspaceRecord`:

```rust
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CapsuleRecord {
    pub id: String,
    pub title: String,
    pub description: String,
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub html_sha256: String,
    pub byte_size: i64,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsuleCreateDraft {
    pub id: String,
    pub title: String,
    pub description: String,
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub html_sha256: String,
    pub byte_size: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapsuleUpdateDraft {
    pub title: Option<String>,
    pub description: Option<String>,
    pub html_sha256: Option<String>,
    pub byte_size: Option<i64>,
}
```

Add schema to `initialize_connection()` in the same `execute_batch` as `workspaces`:

```sql
CREATE TABLE IF NOT EXISTS capsules (... ) STRICT;
CREATE INDEX IF NOT EXISTS idx_capsules_updated ON capsules(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_capsules_thread ON capsules(thread_id);
```

Add CRUD methods on `GaryxDbService`:

```rust
pub fn create_capsule(&self, draft: CapsuleCreateDraft) -> GaryxDbResult<CapsuleRecord>;
pub fn update_capsule(&self, id: &str, draft: CapsuleUpdateDraft) -> GaryxDbResult<Option<CapsuleRecord>>;
pub fn get_capsule(&self, id: &str) -> GaryxDbResult<Option<CapsuleRecord>>;
pub fn list_capsules(&self) -> GaryxDbResult<Vec<CapsuleRecord>>;
pub fn list_capsules_for_thread(&self, thread_id: &str) -> GaryxDbResult<Vec<CapsuleRecord>>;
pub fn delete_capsule(&self, id: &str) -> GaryxDbResult<bool>;
```

Implementation rules:

- Validate IDs with `Uuid::parse_str` and store lowercase UUID strings. Reject empty or malformed IDs with `GaryxDbError::BadRequest`.
- Normalize title/description to trimmed strings; allow empty strings.
- Normalize optional thread/run/agent/provider fields with existing `normalize_optional`.
- Validate `html_sha256` as exactly 64 ASCII hex characters.
- Validate `byte_size >= 0`.
- `create_capsule` inserts `revision = 1`, `created_at = updated_at = now_string()`.
- `update_capsule` first checks row existence. If missing, return `Ok(None)`. Otherwise perform `UPDATE`, preserving omitted fields via `COALESCE`, setting `updated_at`, and bumping `revision = revision + 1` exactly once.
- `list_capsules` orders `updated_at DESC, id ASC`; `list_capsules_for_thread` uses the same order with `WHERE thread_id = ?`.
- `delete_capsule` performs hard `DELETE` and returns whether a row was removed.

### `garyx-gateway/src/capsules.rs`

New HTTP module.

Constants and helpers:

```rust
pub(crate) const CAPSULE_MAX_HTML_BYTES: usize = 5 * 1024 * 1024;
pub(crate) const CAPSULE_MCP_BODY_LIMIT_BYTES: usize = CAPSULE_MAX_HTML_BYTES + 512 * 1024;
pub(crate) const CAPSULE_CSP: &str = "default-src ...";

pub(crate) fn parse_capsule_uuid(id: &str) -> Result<String, CapsuleError>;
pub(crate) fn capsule_file_path(id: &str) -> Result<PathBuf, CapsuleError>;
pub(crate) fn validate_capsule_html_bytes(bytes: &[u8]) -> Result<String, CapsuleError>;
pub(crate) async fn read_capsule_html_path(path: &str) -> Result<Vec<u8>, CapsuleError>;
pub(crate) async fn atomic_write_capsule_file(id: &str, html: &[u8]) -> Result<PathBuf, CapsuleError>;
pub(crate) fn inject_csp_meta(html: &str) -> String;
```

HTTP endpoints:

```rust
pub async fn list_capsules(State(state): State<Arc<AppState>>) -> impl IntoResponse;
pub async fn get_capsule(Path(id): Path<String>, State(state): State<Arc<AppState>>) -> impl IntoResponse;
pub async fn serve_capsule(Path(id): Path<String>, State(state): State<Arc<AppState>>) -> impl IntoResponse;
pub async fn delete_capsule(Path(id): Path<String>, State(state): State<Arc<AppState>>) -> impl IntoResponse;
```

Important behavior:

- All path IDs are normalized through `Uuid::parse_str` before DB or filesystem work.
- `GET /api/capsules` returns `{ "capsules": [...] }`.
- `GET /api/capsules/{id}` returns `{ "capsule": record }` or 404.
- `GET /api/capsules/{id}/serve` requires a DB row, reads `default_capsules_dir().join("<id>.html")`, 404s if missing, injects meta CSP into the returned string, and sets:
  - `Content-Type: text/html; charset=utf-8`
  - `X-Content-Type-Options: nosniff`
  - `Cache-Control: no-store`
  - `Content-Security-Policy: <same CSP>` as direct-load fallback
- `DELETE /api/capsules/{id}` hard-deletes the row and then best-effort removes the file. It returns `{ "deleted": true }` or 404.

Validation behavior:

- Input bytes must be non-empty UTF-8 and at most 5 MiB.
- Reject `file://` and other non-allowed schemes when they appear as resource attribute or CSS URL values.
- Reject relative sidecar references in `src=`, `href=`, `poster=`, `data=`, `action=`, `srcset=`, and CSS `url(...)`. Allow empty anchors, `#...`, `data:`, `blob:`, `https:`, and protocol-relative `//...`, matching the final blueprint.
- Reject explicit local filesystem absolute-looking references in resource attributes (`/...`, `./...`, `../...`) unless they are allowed URL schemes or fragments.

### `garyx-gateway/src/mcp/tools/capsule.rs`

New MCP tool implementation module.

Parameter structs live in `mcp.rs` so the `#[tool]` schema can derive there:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CapsuleCreateParams { title: String, description: Option<String>, html: Option<String>, html_path: Option<String> }

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CapsuleUpdateParams { capsule_id: String, title: Option<String>, description: Option<String>, html: Option<String>, html_path: Option<String> }
```

Tool functions in `capsule.rs`:

```rust
pub(crate) async fn create(server: &GaryMcpServer, ctx: RequestContext<RoleServer>, params: CapsuleCreateParams) -> Result<String, String>;
pub(crate) async fn update(server: &GaryMcpServer, ctx: RequestContext<RoleServer>, params: CapsuleUpdateParams) -> Result<String, String>;
pub(crate) async fn list(server: &GaryMcpServer, ctx: RequestContext<RoleServer>) -> Result<String, String>;
```

Shared helpers:

```rust
async fn create_inner(server: &GaryMcpServer, run_ctx: RunContext, params: CapsuleCreateParams) -> Result<Value, String>;
async fn update_inner(server: &GaryMcpServer, run_ctx: RunContext, params: CapsuleUpdateParams) -> Result<Value, String>;
async fn list_inner(server: &GaryMcpServer, run_ctx: RunContext) -> Result<Value, String>;
async fn capsule_thread_snapshot(server: &GaryMcpServer, thread_id: &str) -> Result<CapsuleThreadSnapshot, String>;
```

Important behavior:

- `capsule_create` requires a non-empty current `thread_id` from `RunContext`; it should not accept thread/agent/provider parameters from the tool call.
- It also records optional `run_id` from context.
- `agent_id` and `provider_type` are derived from `server.app_state.threads.thread_store.get(thread_id)`. If the thread row lacks them, store `NULL` rather than trusting params.
- `provider_type` normalization uses a thread row string if present, or a serialized provider enum string if that is how the row was stored.
- `html` and `html_path` must be exactly one for create; exactly zero or one for update. Update must include at least one of title, description, html, or html_path.
- For writes: validate/compute bytes and sha256, generate/normalize ID, `atomic_write_capsule_file`, then DB insert/update. If DB insert/update fails after file write, the orphan file is acceptable by design.
- Update looks up the DB row first. If missing, return a not-found error and do not write a file.
- `capsule_list` returns only `list_capsules_for_thread(thread_id)` and includes summary entries with `id`, `title`, `description`, `revision`, `byte_size`, `updated_at`, `open_url`, and `serve_path`.

### `garyx-gateway/src/mcp.rs`

- Add parameter structs above the tool impl.
- Add three `#[tool]` methods:

```rust
async fn capsule_create(&self, ctx: RequestContext<RoleServer>, Parameters(params): Parameters<CapsuleCreateParams>) -> Result<String, String>;
async fn capsule_update(&self, ctx: RequestContext<RoleServer>, Parameters(params): Parameters<CapsuleUpdateParams>) -> Result<String, String>;
async fn capsule_list(&self, ctx: RequestContext<RoleServer>) -> Result<String, String>;
```

- Update `get_info().instructions` to include `capsule_create`, `capsule_update`, and `capsule_list`.

### `garyx-gateway/src/mcp/tools/mod.rs`

Add:

```rust
pub(super) mod capsule;
```

### `garyx-gateway/src/route_graph.rs`

- Import `capsules` in the crate module list.
- Register protected routes near `/api/workspaces`:

```rust
.route("/api/capsules", axum::routing::get(capsules::list_capsules))
.route("/api/capsules/{id}", axum::routing::get(capsules::get_capsule).delete(capsules::delete_capsule))
.route("/api/capsules/{id}/serve", axum::routing::get(capsules::serve_capsule))
```

- Apply a route-local MCP request-body cap with `tower_http::limit::RequestBodyLimitLayer`, not `DefaultBodyLimit`. rmcp’s `StreamableHttpService` is mounted via `nest_service` and deserializes with `BodyExt::collect()`, so Axum extractor limits do not protect this path. Set a conservative limit of `CAPSULE_MAX_HTML_BYTES + 1024 * 1024` (about 6 MiB) on `/mcp` so inline `html` can reach the tool with JSON escaping headroom while obviously oversized JSON is rejected before full collection. Keep HTTP capsule routes GET/DELETE-only, so they need no body-limit changes.

### `garyx-gateway/src/lib.rs`

Add:

```rust
pub mod capsules;
```

### Dependency plan

- Add `sha2 = "0.10"` to `garyx-gateway` for SHA-256.
- Add `tower-http = { version = "0.6", features = ["limit"] }` to `garyx-gateway` for the required `/mcp` request-body cap. Use `tower_http::limit::RequestBodyLimitLayer`, not Axum `DefaultBodyLimit`, because rmcp is mounted as a nested Tower service and reads the body with `BodyExt::collect()` rather than Axum extractors.
- Prefer simple deterministic string scanning with existing `regex` over adding an HTML parser, because Phase A only needs static rejection of obvious local/relative sidecar references and `file://`.
- Do not rely on `tempfile` in production code; in `capsules.rs`, implement atomic writes with `tokio::fs::create_dir_all`, a same-directory unique temp path such as `.<id>.<Uuid::new_v4()>.tmp`, `tokio::fs::write`, then `tokio::fs::rename`. Cleanup the temp path on write/rename errors best-effort.

## Test plan

### `garyx-models`

- `cargo test -p garyx-models local_paths`
  - Add `default_capsules_dir_uses_garyx_home`.

### `garyx-gateway` DB unit tests

Add tests in `garyx_db/mod.rs` tests:

- `capsule_crud_create_update_get_list_delete_hard_deletes`
  - Create record, assert revision 1, metadata, `created_at`, `updated_at`, list content.
  - Update title/description/html metadata, assert revision 2 and `created_at` preserved.
  - Delete, assert `get_capsule` returns `None` and second delete returns false.
- `capsule_list_orders_by_updated_desc_then_id`
  - Create two records, update the first, assert updated record sorts first.
- `capsule_list_for_thread_filters_current_thread`
  - Create records across two threads; assert filter.
- `capsule_rejects_invalid_uuid_and_bad_hash`
  - Malformed ID and non-hex/non-64 hash return `BadRequest`.

### `garyx-gateway` capsule HTTP/module tests

Add tests in `capsules.rs` under `#[cfg(test)]`:

- `validate_capsule_html_rejects_oversize_and_local_references`
  - Oversize bytes, `file://`, `<img src="asset.png">`, CSS `url(./asset.png)` are rejected.
  - `data:`, `blob:`, `https:`, fragment anchors are allowed.
- `inject_csp_meta_inserts_into_head_or_prepends`
  - Existing `<head>` gets meta inside; no `<head>` prepends meta.
- `capsule_file_path_rejects_invalid_id_before_join`
  - Invalid id returns bad request.

HTTP route tests:

- Build a test `AppState` with gateway auth enabled and in-memory DB.
- Create a DB record and matching temp capsule file. Implement a test-only capsule-dir override in `capsules.rs` (for example a `OnceLock`/`Mutex<Option<PathBuf>>` or a helper guarded by `#[cfg(test)]`) so tests do not mutate process-global `HOME` in parallel Rust tests. Production code still uses `default_capsules_dir()`.
- `GET /api/capsules` without auth returns `401` or equivalent protected-route rejection; with auth returns the record.
- `GET /api/capsules/{id}` returns metadata; malformed UUID returns 400 before filesystem access.
- `GET /api/capsules/{id}/serve` returns 200, text/html, nosniff, no-store, HTTP CSP, and body contains the meta CSP plus original HTML.
- Missing file for an existing DB row returns 404.
- `DELETE /api/capsules/{id}` removes row and best-effort removes file; second delete returns 404.

### `garyx-gateway` MCP tests

Add tests in `mcp/tests.rs` or a capsule-specific test module:

- `test_mcp_tool_router_advertises_capsules`
  - `list_all()` includes `capsule_create`, `capsule_update`, `capsule_list`; `get_info().instructions` mentions them.
- `test_capsule_create_requires_thread_run_context_and_derives_agent_from_thread`
  - Seed `thread_store` with `agent_id = "agent-capsule"` and `provider_type = "codex_app_server"`.
  - Invoke `tools::capsule::create_inner`/public test-visible helper with `RunContext { thread_id, run_id }`.
  - Assert DB row has thread/run snapshot and derived agent/provider, returned JSON has `open_url` and `serve_path`, and file exists.
- `test_capsule_create_rejects_html_and_html_path_conflicts`
  - Both set and neither set should error.
- `test_capsule_update_rewrites_file_and_bumps_revision`
  - Create then update with new HTML/title; assert file body changed and revision incremented.
- `test_capsule_list_filters_to_current_thread`
  - Seed DB with two thread IDs; assert only current thread entries returned.
- `test_capsule_create_rejects_oversize_and_bad_html`
  - Cover 5 MiB + 1 and relative sidecar references through the MCP path.

### Final commands before code review

- `cargo fmt --check`
- `cargo test -p garyx-models local_paths`
- `cargo test -p garyx-gateway`
- `cargo build -p garyx-gateway`

## Review questions

1. Does this keep Capsule as gateway-owned application state and avoid router/projection coupling?
2. Are the write ordering, hard delete, UUID validation, thread-derived agent/provider snapshots, and MCP-only create/update boundaries faithful to `docs/design/capsule.md`?
3. Is the proposed static HTML validation strict enough for Phase A without overbuilding a sanitizer/parser?
4. Are the route/MCP registration points and body-limit plan compatible with current gateway patterns?

## Design review update 2026-06-28

After inspecting rmcp 0.17, the body-limit plan was corrected: rmcp calls `BodyExt::collect()` inside its nested Tower service, while Axum `DefaultBodyLimit` only affects Axum extractors. The implementation must therefore use `tower_http::limit::RequestBodyLimitLayer` on the `/mcp` service for a pre-collection cap. The test plan also now requires a test-only capsule directory override instead of mutating `HOME` for HTTP/file tests. A final dependency check showed `tempfile` is dev-only in `garyx-gateway`, so production atomic writes will use a same-directory UUID-named temp path plus `rename` rather than promoting `tempfile` to a runtime dependency.
