# Capsule — 综合设计定稿

Capsule 是 agent 通过 MCP 工具创建 / 更新的**自包含单文件 HTML**（讲解 / 可视化 / 演示），
落盘到 `~/.garyx/capsules/`，元数据存入 gateway 数据库，并从 Mac / iOS app 的**顶层入口**浏览 + 运行。

本文是两版独立设计（#TASK-1400 claude / #TASK-1401 codex）对比后的**综合定稿**，是实现蓝本。
分歧点的取舍记录在每节末尾，实现以本文为准。

---

## 0. 架构定性（两版一致，直接采纳）

> Capsule 是 **gateway 自有的权威应用状态**，不是 router 投影。`garyx_db` 里的元数据行是真相源，
> 磁盘上的 HTML 文件是内容 blob。**不**走 projecting-store wrapper、**不**引入 router reader trait，
> 因为 Capsule 不是从 thread 记录派生出来的。

依据：`garyx_db`（`garyx-gateway/src/garyx_db/mod.rs`）已有两类表——(a) router thread 数据的 write-time
投影（`recent_threads` / `task_projection` / `thread_meta`）；(b) gateway 自有、无 router 拥有的目录表
（`workspaces` / `automation_thread_runs`）。Capsule 属于 (b)，
**照 `workspaces` 先例**：一张扁平权威表 + CRUD，保持 gateway→router 单向依赖。创建它的 thread / agent
仅作**快照列**记录（write-time 从 MCP 请求上下文取，非活引用）。

---

## 1. 命名与目录

产品名 **Capsule**，全链路统一：

| 面 | 标识 |
| --- | --- |
| DB 表 | `capsules` |
| 磁盘目录 | `~/.garyx/capsules/<id>.html` |
| MCP 工具 | `capsule_create` / `capsule_update` / `capsule_list` |
| HTTP 路由 | `/api/capsules`、`/api/capsules/{id}`、`/api/capsules/{id}/serve` |
| Desktop 视图 | `ContentView` 值 `'capsules'`，**顶层**左 rail 入口，label **Capsules** |
| iOS | `GaryxMobilePanel.capsules`，**顶层** drawer 入口，label **Capsules** |

- **单文件** `~/.garyx/capsules/<id>.html`（不是目录）——硬保"单 HTML 自包含可运行"；元数据全在 DB，无 sidecar。
- 路径助手：在 `garyx-models/src/local_paths.rs` 加 `default_capsules_dir() -> PathBuf { gary_home_dir().join("capsules") }`
  （照 `default_skills_dir()`）。
- **id = 小写 UUIDv7**（root `Cargo.toml` 已启用 `uuid` v7 feature），时间可排序、无 namespace 前缀。
  文件名 `<id>.html`。所有 serve / get / delete 路径**先**用 `Uuid::parse_str` 严格校验再碰文件系统 ⇒ 路径穿越结构上不可能。

---

## 2. 数据模型（综合：取 codex 的 description/html_sha256/revision，去掉其软删除/分页/乐观并发）

在 `garyx-gateway/src/garyx_db/mod.rs` 的 `initialize_connection()`（与 `workspaces` 同一 `execute_batch`）新增：

```sql
CREATE TABLE IF NOT EXISTS capsules (
    id            TEXT PRIMARY KEY,
    title         TEXT NOT NULL DEFAULT '',
    description   TEXT NOT NULL DEFAULT '',
    thread_id     TEXT,            -- 归属快照：创建它的 thread（plain id）
    run_id        TEXT,            -- 归属快照：创建它的 agent run
    agent_id      TEXT,            -- 从 thread 记录推导（不信任工具参数）
    provider_type TEXT,            -- 快照：claude / codex / gemini…
    html_sha256   TEXT NOT NULL,   -- HTML 原文 bytes 的 sha256 hex（缓存 key + 完整性）
    byte_size     INTEGER NOT NULL DEFAULT 0,
    revision      INTEGER NOT NULL DEFAULT 1,   -- 每次 update +1；客户端缓存失效用
    created_at    TEXT NOT NULL,   -- RFC3339 / now_string()
    updated_at    TEXT NOT NULL    -- 每次写 bump
) STRICT;

CREATE INDEX IF NOT EXISTS idx_capsules_updated ON capsules(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_capsules_thread  ON capsules(thread_id);
```

Rust（照 `WorkspaceRecord` / `upsert_workspace`，`garyx_db/mod.rs`）：
- `CapsuleRecord`（返回 API/MCP）、`CapsuleCreateDraft`、`CapsuleUpdateDraft`（`title/description/html_sha256/byte_size` 均 `Option`，只改给到的）。
- `GaryxDbService`：`create_capsule` / `update_capsule` / `get_capsule(id)` / `list_capsules()` /
  `list_capsules_for_thread(thread_id)` / `delete_capsule(id) -> bool`。
- `update_capsule`：`INSERT … ON CONFLICT(id) DO UPDATE SET …`，保 `created_at`、bump `updated_at`、`revision = revision + 1`。

**写顺序（write-time，MCP handler 自己做，无 projecting store 可挂）**：
1. 原子写文件：写临时文件到同目录再 `rename` 成 `<id>.html`（避免半截文件被 serve）。
2. `garyx_db.create_capsule/update_capsule`（`byte_size`/`html_sha256` 来自 HTML 原文）。
顺序 = 先文件后行：崩在中间留孤儿文件（无害、可 GC），不会留下 serve 即 404 的行。

**删除 = 硬删**：删 row + `fs::remove_file`（best-effort）。不做 `deleted_at` 软删（v1 YAGNI）。

> 取舍：codex 版有 `expected_revision` 乐观并发 / `deleted_at` 软删 / 分页 filter / `storage_key` 列——
> v1 单用户串行写都用不上，**砍掉**。`revision` 保留但只作"更新计数 + 客户端缓存失效 key"，不做冲突检测。

---

## 3. MCP 工具（采 claude：只 3 个；get/delete/browse 归 HTTP）

`agent` 只需「产出」+「召回自己刚做的」；浏览 / 打开 / 删除是**用户**动作，归 app + HTTP。工具少 ⇒ 模型 tool-selection 更准。

落点：声明在 `garyx-gateway/src/mcp.rs` 的 `#[tool_router] impl GaryMcpServer`（照 `status`/`schedule_followup`），
实现新建 `garyx-gateway/src/mcp/tools/capsule.rs`（照 `schedule_followup.rs`：`run()` → `RunContext::from_request_context`
→ `run_inner` → `record_tool_metric` → JSON string），并在 `mcp/tools/mod.rs` 挂模块。`#[tool]` 宏自动生成 dispatch，无 enum/match 要改。

- **`capsule_create`**：参数 `title: String`、`description: Option<String>`、`html: Option<String>`、`html_path: Option<String>`
  （`html` / `html_path` 恰好二选一）。生成 UUIDv7 → 写文件 + 行 → 返回 JSON。
- **`capsule_update`**：参数 `capsule_id: String`、`title? / description? / html? / html_path?`（至少一项）。
- **`capsule_list`**：无参，列**当前 thread**创建的 capsule（id/title/updated/serve_path），供后续 run 召回 id 再 update。

**HTML 传入**：
- inline `html`：常规路径（MCP 是 JSON-over-HTTP，`json_response: true`，几十～数百 KB 没问题；MCP body limit 在 `route_graph.rs` 相应放宽）。
- `html_path`：agent 常先把 HTML 写进工作区文件，给绝对路径避免大文档挤 tool 通道；gateway 照 `workspace_files.rs` 纪律
  `canonicalize` + 必须是常规文件后读取。
- **静态校验**（create/update 都做）：拒绝本地 `file://` 引用 / 相对 sidecar 资源引用（保"自包含"）；HTML 解析出的 bytes
  **5 MB 上限**，超限明确报错。allow inline / `data:` / `blob:` / `https:` CDN。
- **`agent_id` / `provider_type`** 一律从 `thread_store.get(thread_id)` 的 thread 记录推导，**不信任工具参数**。

返回 JSON（照 text 工具惯例返回字符串）：
```json
{ "tool": "capsule_create", "status": "ok", "capsule_id": "0190…",
  "title": "…", "revision": 1, "byte_size": 18342, "updated_at": "…",
  "open_url": "garyx://capsules/0190…", "serve_path": "/api/capsules/0190…/serve" }
```
更新 MCP `get_info` 的 instructions（`mcp.rs`）提及这 3 个新工具。

> v1 **不**给 agent `capsule_get(include_html)`。若实践中 agent 需"读回上一版再改"，后续再加；当前让它重新生成整份。

---

## 4. HTTP API

新模块 `garyx-gateway/src/capsules.rs`（兄弟于 `workspaces.rs`），`lib.rs` 加 `pub mod capsules;`，路由注册在
`route_graph.rs` 紧跟 `/api/workspaces` 块、**protected router 内**（套 gateway-auth）：

- `GET /api/capsules` → `{ "capsules": [ {id,title,description,thread_id,agent_id,provider_type,html_sha256,byte_size,revision,created_at,updated_at}, … ] }`，按 `updated_at DESC`。
- `GET /api/capsules/{id}` → 单条 metadata，或 404。
- `DELETE /api/capsules/{id}` → 硬删 row + 文件，`{ "deleted": true }` 或 404。
- `GET /api/capsules/{id}/serve` → **HTML**（唯一非 JSON 路由）。流程：`Uuid::parse_str` 校验 → DB 必须存在 →
  读 `default_capsules_dir().join("<uuid>.html")` → 返回。

**serve 返回的 HTML 注入 `<meta http-equiv="Content-Security-Policy" …>`**（磁盘原文不改），并带 HTTP 头
`Content-Type: text/html; charset=utf-8`、`X-Content-Type-Options: nosniff`、`Cache-Control: no-store`。
因为客户端是 fetch 出字符串后用 srcDoc / loadHTMLString 渲染（见 §5），HTTP 头 CSP 不会随之生效，**meta CSP 才是真正起作用的那道**，HTTP 头 CSP 作直连兜底。

CSP（permissive-but-contained，配"inline 或 CDN, 打开即跑"）：
```
default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' https: blob: data:;
style-src 'unsafe-inline' https:; img-src https: data: blob:; font-src https: data:;
connect-src https:; media-src https: data: blob:; frame-src https:; object-src 'none';
base-uri 'none'; form-action 'none'
```

---

## 5. 取 HTML 来运行（采 codex：统一 auth fetch → 沙箱，不直连 iframe src）

**两端都**：① 经已认证的 gateway client 带 Authorization `GET …/serve` 取出 HTML 字符串 → ② 塞进沙箱化容器渲染。
**不**用 `<iframe src={gatewayBase}/api/capsules/{id}/serve>` 直连。

> 取舍依据：iframe / WKWebView 的子资源请求**带不上** Authorization 头；且 desktop 可连**远程** gateway，
> 直连 serve URL 会 401，并有把 token 拼进 URL 泄漏的风险。统一 fetch 字符串对 local/remote、Mac/iOS 都成立、路径一致。
> claude 版"Mac 靠 loopback bypass auth 直接 iframe src"只在同机成立，弃用。

---

## 6. 安全（两版一致）

威胁模型：HTML 由 LLM 生成，**不可信**——不得读用户 gateway 会话、不得碰宿主 DOM、不得持久化 cookie/storage。
主边界 = **opaque-origin 沙箱**，CSP 作纵深防御：
- Desktop：`<iframe sandbox="allow-scripts" srcDoc={html}>`——**不给** `allow-same-origin`；无 Electron preload / Node integration；第一期不加 `allow-popups`。
- iOS：`WKWebView` + `WKWebViewConfiguration.websiteDataStore = .nonPersistent()`、允许 JS、**无** `addUserScript` / **无** `WKScriptMessageHandler`（无 JS↔native 桥）、`loadHTMLString(html, baseURL: nil)`（nil base ⇒ opaque origin）、navigation delegate 拦截顶层导航、外链走 Safari。
- 既是 opaque origin，即便 `connect-src https:` 也读不到 gateway 同源认证响应。
- 路径穿越：UUID 严格校验在前；文件名恒 `dir.join(uuid + ".html")`。

---

## 7. Desktop UI（Mac，IA 真相源，顶层入口）

左 rail **顶层**入口 `Capsules`（放 Automation 之后、Tasks 之前）。落点（`desktop/garyx-desktop/src/…`）：
- renderer：`app-shell/types.ts` 加 `'capsules'` 到 `ContentView`；`desktop-route.ts` 加 `capsules: 'capsules'`（`#/capsules` 深链）；
  `icons.tsx` 加 `CapsulesIcon`（lucide `Package`/`Box` 或自绘 capsule glyph，单色）；`components/AppLeftRail.tsx` 加
  `isCapsulesView`/`onOpenCapsules` + nav button；`AppShell.tsx` 加 view flag、saved-view、render branch `<CapsulesPanel/>`。
- 新 `components/CapsulesPanel.tsx`：左列表（title / description / 相对 updated / bytes / 创建 agent badge——经共享 identity presentation helper，不写本地 switch 表）+ 右 detail/runner（header + Refresh/Delete/Copy ID + iframe runner），空/加载/错误态照现有 panel。
- 取数走 **main 进程带 auth fetch + IPC**（renderer 不直接碰 gateway token）：`shared/contracts.ts` 加
  `DesktopCapsuleSummary` 等 + `GaryxDesktopApi.listCapsules/getCapsule/getCapsuleHtml/deleteCapsule`；`main/gary-client.ts`
  加对应方法（含取 `/serve` 文本的 `requestText`）；`main/index.ts` 加 `ipcMain.handle("garyx:list-capsules" / "garyx:get-capsule-html" / …)`；`preload/index.ts` expose。
- 运行：`<iframe sandbox="allow-scripts" srcDoc={htmlFromIpc} />`（见 §5/§6）。不用 Electron `webview` 标签、不复用 Browser `WebContentsView`。

---

## 8. iOS UI（顶层 drawer，老板拍板 2026-06-28）

drawer root **顶层**入口 `Capsules`（Automation 之后）。逻辑进 `GaryxMobileCore` + SwiftPM 测试；app target 只做 SwiftUI 组合。
- Core（`mobile/garyx-mobile/Sources/GaryxMobileCore/`）：
  - `GaryxMobileNavigationState.swift`：`GaryxMobilePanel` 加 `.capsules`（label `"Capsules"`、icon `"capsule.fill"`，缺则 `"shippingbox.fill"`）。
  - `GaryxMobileRouteLink.swift`：`GaryxMobileRoute` 加 `.panel(.capsules)` 与 `.capsule(String)`；parse `/capsules`、`/capsule?id=`；make `garyx://mobile/capsules`、`…/capsule?id=`。
  - `GaryxGatewayCapsuleModels.swift`（新）：`GaryxCapsuleSummary`（snake_case 容错 `init(from:)`，照 `GaryxGatewayTaskModels`）+ `GaryxCapsulesPage`。
  - `GaryxGatewayClient.swift`：`listCapsules()` / `deleteCapsule(id:)` / `capsuleHTML(id:)`（带 auth 取 `/serve` 文本）。
  - `GaryxMobileCatalogCache.swift`：metadata 走 **gateway-scoped stale-while-refresh**（照 tasks/skills）；HTML body 另用 **id + revision/html_sha256** 内存缓存避免重复 fetch（panel state machine：`beginHTMLLoad/applyHTML/applyHTMLFailure`，纯逻辑可测）。
- app target：`GaryxMobileSidebarViews.swift` 顶层 drawer 行 + `panelContent(for: .capsules)`；`GaryxMobileModel.swift`
  `@Published var capsules`（restore-from-cache + refresh）；新 `GaryxMobileCapsuleViews.swift`（feature-specific）：
  `GaryxCapsulesView`（native grouped `List`、`garyxPageBackground`、pull-to-refresh、行 ellipsis→Delete、badge 经共享 helper）、
  `GaryxCapsuleDetailView`、`GaryxCapsuleWebView`（`UIViewRepresentable`，§6 硬化配置）。
- **xcodegen**：新 Core/app 文件后必须 `xcodegen generate` 并提交 `GaryxMobile.xcodeproj/project.pbxproj`，否则 app 编不到（`swift test` 会假绿）。验证须 `xcodebuild`。

---

## 9. 分阶段实现（3 个 task；A 先合 main，B/C 并行）

每个 task 都走标准流程：**实现级设计 → 自开跨模型 review（claude 作者→codex 审 / codex 作者→claude 审）→ 实现 → 自开 code review 到 100% → 合 main → 正常结束 run（不自己切 in_review）**。worktree 隔离。

- **阶段 A（gateway backend，最核心，headless 可测）**：`default_capsules_dir`；`capsules` 表 + Rust 结构 + CRUD；
  `mcp/tools/capsule.rs` 三工具 + 注册 + `get_info` instructions；`capsules.rs` 四 HTTP endpoint + meta CSP 注入 + 安全头；
  原子写文件；html/html_path + 静态校验 + 5MB cap；UUID 校验。
  验证：`cargo test -p garyx-gateway`（DB CRUD / 非法 id 拒绝 / list 顺序 / 缺文件行为 / HTTP auth / serve 头与 meta CSP /
  MCP `list_tools` 含三工具 / create 用 thread-run 上下文 / agent_id 从 thread 推导 / 超大与坏 HTML 拒绝）+ `cargo test -p garyx-models local_paths`。
- **阶段 B（Desktop，依赖 A）**：§7 全部。验证：`npm run build:ui` + smoke；改了 renderer/preload/IPC 须 packaged-app 实测（`npm run dist:dir` 后开装好的 app + CDP attach）。
- **阶段 C（iOS，依赖 A）**：§8 全部 + SwiftPM 测试 + `xcodegen` + `xcodebuild`。验证：`swift test` + 模拟器。

gateway 改动须 build+install+restart 才生效（`scripts/build-local-cli.sh` / `install-local-cli.sh`，用 garyx 自己的 restart）。

---

## 10. Scope 边界（v1 不做）

跨设备 / 云同步 / 公开分享；版本历史（update 原地覆盖）；多文件 / 本地资源 bundle（只单文件，inline 或 CDN）；
服务端缩略图；应用内编辑 HTML；超出 gateway auth 的 ACL / 多用户；进 transcript / `render_state`（capsule 不进聊天记录）；
`POST /api/capsules` 手动导入（v1 创建仅 MCP）；软删除 / 分页 / 乐观并发 / 给 agent `capsule_get`。
