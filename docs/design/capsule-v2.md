# Capsule v2 体验优化设计

> 设计阶段产物。本文只描述落点与契约，不含生产代码实现、不开实现子任务。
>
> v2 全程复用 v1：`capsules` 表、`~/.garyx/capsules/<uuid>.html`、`/api/capsules/{id}/serve`、MCP `capsule_create`/`capsule_update`/`capsule_list`、fetch-then-sandbox 渲染、opaque-origin sandbox + meta CSP。**不引服务端缩略图**，不把不可信 HTML 变成宿主 DOM。

---

## 0. 本次读码结论（带 file:line）

v1 真相源（不重新解释）：

- 存储：`capsules` 表 — `garyx-gateway/src/garyx_db/mod.rs:3273`（`STRICT`）。列：`id`(UUIDv7)、`title`、`description`、`thread_id?`、`run_id?`、`agent_id?`、`provider_type?`、`html_sha256`、`byte_size`、`revision`、`created_at`、`updated_at`。HTML 落盘 `default_capsules_dir()/{uuid}.html`（`garyx-models/src/local_paths.rs`），表里只存 `html_sha256`+`byte_size`。记录结构 `CapsuleRecord`（12 字段，snake_case 序列化）— `garyx_db/mod.rs:359`。
- Gateway HTTP：`garyx-gateway/src/route_graph.rs:208-216` — `GET /api/capsules`（`capsules.rs:324`，返回 `{capsules:[...]}`）、`GET /api/capsules/{id}`、`DELETE /api/capsules/{id}`、`GET /api/capsules/{id}/serve`（`capsules.rs:350`，严格 UUID 校验、读 `<id>.html`、注入 `<meta http-equiv="Content-Security-Policy">`、返回 `text/html`+`nosniff`+`no-store`+CSP header）。CSP 常量 `CAPSULE_CSP` — `capsules.rs:23`。
- MCP 写入：`capsule_create`/`capsule_update`/`capsule_list` — `garyx-gateway/src/mcp.rs:360`，参数 `mcp.rs:111-143`，实现 `garyx-gateway/src/mcp/tools/capsule.rs`。`thread_id`/`run_id` 只从 `RunContext` 取（`mcp.rs:185-226`），不信任工具参数。返回 JSON 含 `id`/`capsule_id`/`title`/`revision`/`html_sha256`/`serve_path`/`open_url: "garyx://capsules/{id}"`（`capsule.rs:275-301`）。`update_capsule` 时 `revision += 1`（`garyx_db/mod.rs:745`），`id` 永不变，纯缓存失效键、非 OCC。
- Desktop v1：`desktop/garyx-desktop/src/renderer/src/app-shell/components/CapsulesPanel.tsx` 是 master-detail：左 `.capsules-list`（`:331-359`），右 `.capsules-detail`（`:361-434`，title/description/revision/byteSize/timestamp/id + Copy ID/Refresh/Delete + `<iframe sandbox="allow-scripts" srcDoc={selectedHtml}>` `:418`）。HTML 走主进程鉴权 IPC：`main/gary-client.ts:4857 listCapsules`/`:4889 getCapsuleHtml`/`:4902 deleteCapsule`、`shared/contracts.ts`、`preload/index.ts`。
- iOS v1：Core `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxGatewayCapsuleModels.swift` 有 `GaryxCapsuleSummary`(`:20`)/`GaryxCapsuleHTMLCacheKey`(`:102`)/`GaryxCapsuleHTMLLoadState`(`:118`)；`GaryxGatewayClient.swift` 有 `listCapsules()`/`deleteCapsule(id:)`/`capsuleHTML(id:)`；App `App/GaryxMobile/GaryxMobileCapsuleViews.swift` 是 grouped `List`+`GaryxCapsuleRow`(`:134`)+`fullScreenCover` detail(`:237`)，detail 显示 title/description/metadata 再嵌 `GaryxCapsuleWebView`(`:378`，`websiteDataStore=.nonPersistent()` `:388`、JS allowed、无 native bridge、`loadHTMLString(html, baseURL:nil)` `:405`、navigation delegate 拦外链 `:411-437`）。
- render_state 契约（`CLAUDE.md`「Transcript Rendering」+ `docs/agents/repository-contracts.md`，**必须严守**）：committed/control ledger 是聊天结构唯一来源；`garyx-models/src/transcript_render_state.rs` 拥有 reducer；gateway per-thread SSE 发 `thread_render_frame { events, render_state }`（`garyx-gateway/src/routes.rs:2202`）；desktop/iOS 只 dumb-render，不本地重算 user turn / tool grouping / tail thinking / final-answer placement。
- 当前 reducer：top-level `RenderRow` 只有 `user_turn`（`transcript_render_state.rs:72`，**非 `#[non_exhaustive]`**）；`RenderActivityRow` 只有 `assistant_reply`/`step`（`:594` `build_activity_rows` 恒返回**单个** activity row）；final answer 经 `take_final_assistant`（`:662`）提到 `RenderStepRow.final_message`，渲染在 step 末。主循环对 `control` 记录 `is_control_message → continue`（`:222-224`）。snapshot 派生：`garyx-router/src/thread_history.rs:1206 render_snapshot_at_seq` / `:1220 render_snapshot_in_window`（按 `seq<=based_on_seq` 过滤 committed records → 调 reducer）。控制记录形态 `message.control={kind,...}`（`transcript_run_state.rs:107`）。
- **关键写侧先例（必须按这个，不按 `range_rewrite`）**：普通 run 内控制记录走 bridge 的 run 长 `transcript_controls` 累加器，而不是 router 侧带外 append。`RunControlRecord`（`garyx-bridge/src/multi_provider/persistence.rs:275`）由 `RunControlRecord::new` 生成真实控制 envelope：`role:"system"`、顶层 `kind:"control"`、`internal:true`、`internal_kind:"control"`、`control:{kind,...}`（`:297-303`）。`run_management.rs` 维护 `transcript_controls`（`:391`），`control_record_for_stream_event`（`:242-293`）/`RunControlRecord::new` 把 `assistant_boundary`/`user_ack`/`done` 等控制记录压入该累加器（`:476-497` 等）。`build_run_record_drafts(content_messages, transcript_controls)`（`persistence.rs:816-855`）按 `after_content_count` 把控制记录交织进 authoritative drafts；terminal reconcile 再用同一函数重建并传给 `reconcile_run_records_tail`（`:1228-1231`）。所以 `capsule_attached` 必须做成同类 `RunControlRecord`，才能在 streaming append 与 terminal reconcile 双路径都存活。`append_range_rewrite_marker` / `build_range_rewrite_record` 是 reconcile 差异审计 marker（`thread_history.rs:2444-2489`），不是普通业务 marker 先例；外部 append 且不在 `transcript_controls` 的 marker 会在 terminal reconcile 中被抹掉。
- 依赖方向硬约束：`garyx-models` / `garyx-router` **不得**依赖 `garyx-gateway::garyx_db`。下文主方案天然满足（marker 是 ledger 内普通 committed 记录，reducer 无需任何 capsule DB 输入）。

---

## 1. 目标体验

### 1.1 Capsules 列表 → 卡片画廊

- 顶层入口不变；进入后默认看到卡片网格（不再左列表+右 detail）。
- 每卡：上半 = live HTML 预览（desktop sandbox iframe / iOS WKWebView，先带 auth fetch `/serve` 字符串再塞沙箱）；下半 = 名字 + 1 行副信息（更新时间 / creator，优先短）。点卡 → app 内 preview。
- 不引服务端缩略图；不持久化截图；不复用 Browser/WebContentsView。

### 1.2 预览页去「套娃」

- 打开后网页尽量全屏专注；收起 title/description/revision/timestamp/id 大块 metadata 与一排按钮。
- 必要操作只留极简 chrome：返回、删除、复制链接、刷新；默认不挡网页。

### 1.3 聊天 final message 后自动补 Capsule 卡片

- 某 agent run 本轮 create/update 过 Capsule，则该 run 所属 user turn 的 final answer 之后，由 **server render_state** 自动追加 Capsule 卡片（同款预览+名字）。
- Desktop/iOS 只渲染这个 row；**不得**扫描 tool result、不得本地查 `capsules` 决定插卡、不得重排 final answer。
- 点击聊天卡片进 app 内 preview（desktop capsule route；iOS `GaryxMobileRoute.capsule(id)`）。

---

## 2. 非目标与保留边界

- 不做服务端缩略图、静态图缓存、公开分享、云同步、多文件 bundle、版本历史页、内嵌编辑器。
- 不给 untrusted HTML 宿主权限：仍 sandbox iframe / non-persistent WKWebView；不加 `allow-same-origin`；无 WK script message handler。
- 不改 v1 MCP 工具数量；不新增 `capsule_get`。
- 不让客户端从 raw messages / tool rows / capsules list 本地推导卡片行结构。

---

## 3. Desktop 画廊与预览

### 3.1 文件落点

- `.../app-shell/components/CapsulesPanel.tsx`：master-detail → gallery + preview 两态；可拆同目录新文件 `CapsuleCard.tsx`、`CapsuleLivePreviewFrame.tsx`、`CapsulePreviewPage.tsx`。
- `.../renderer/src/styles.css`：`.capsules-list`/`.capsules-detail` → `.capsules-gallery-grid`、`.capsule-gallery-card`、`.capsule-card-preview-shell`、`.capsule-preview-page`、`.capsule-preview-toolbar`。
- `.../shared/contracts.ts`：复用 `DesktopCapsuleSummary`；新增 render card 类型见 §5。
- `main/gary-client.ts`/`main/index.ts`/`preload/index.ts`：复用 `listCapsules/getCapsule/getCapsuleHtml/deleteCapsule`。
- `.../app-shell/desktop-route.ts`：新增 capsule preview route `#/capsules/<capsuleId>`；`#/capsules` 仍表示 gallery。
- `main/deep-link.ts`/`shared/contracts.ts`/`AppShell.tsx`：新增 canonical deep link `garyx://capsules/<capsuleId>` → `DesktopDeepLinkEvent { type:"open-capsule", capsuleId }`。

### 3.2 Gallery layout

```text
Capsules header (compact)                         Refresh
┌──────────────────────────────────────────────┐
│  card     card     card     card              │
│  preview  preview  preview  preview           │
│  title    title    title    title             │
│  meta     meta     meta     meta              │
└──────────────────────────────────────────────┘
```

- 仍在 ContentView `'capsules'` 下；CSS grid `repeat(auto-fill, minmax(220px, 1fr))`，大屏卡宽上限 ~320px。
- 卡片按钮整体可点；iframe `pointer-events:none` 防 untrusted HTML 吃点击。
- 下半：`title` 单行 ellipsis（空标题 `Untitled Capsule`）；副信息 `updated relative · creator`，revision 进 tooltip/次级。
- Empty/loading/error 沿用 `tasks-state`/`tasks-empty-state` 语义，去掉左栏视觉。

### 3.3 Desktop live preview component（三处复用基石）

新增 `CapsuleLivePreviewFrame`，props `{ capsule: DesktopCapsuleSummary; mode:"card"|"preview"; active:boolean }`：

- HTML cache key 统一为 `id:revision`。gallery summary 仍可携带 `htmlSha256` 用于调试/强校验，但聊天卡 wire 只带 `capsule_id+revision`，三处共用缓存不能分裂成 `id:revision:sha` 与 `id:revision` 两套。取 HTML 仍走 `window.garyxDesktop.getCapsuleHtml(id)`（不让 renderer 直连 gateway）；缓存上提到 AppShell 级 `htmlByKey`，画廊/预览/聊天卡共用。
- Card 模式：wrapper 固定 aspect（16:10/4:3），iframe 虚拟视口 `1024x640` + `transform:scale(...)` + `transform-origin:top left` + wrapper `overflow:hidden`；iframe `sandbox="allow-scripts"` `srcDoc={html}` `pointer-events:none` `tabIndex=-1`，只展示不交互。
- Preview 模式：iframe `width/height:100%`，允许网页内交互，sandbox 仍仅 `allow-scripts`。

### 3.4 Desktop preview 去 chrome

```text
┌─────────────────────────────────────────────────────┐
│  ←  Title (compact)                            ⋯    │ thin translucent overlay
├─────────────────────────────────────────────────────┤
│                  live capsule iframe                │
└─────────────────────────────────────────────────────┘
```

- `capsule-preview-page` 占满内容区；不显示 description/revision/byteSize/timestamp/id 大块详情。
- 顶部细条 ~36-44px、半透明/blur；首版不必做复杂自动隐藏。左 Back（`#/capsules/<id>`→`#/capsules`）；右：Refresh（强制重 fetch，cache key 不变也刷 iframe）、Copy link（复制 `garyx://capsules/<id>`，复制裸 UUID 进 overflow 次级）、Delete（overflow destructive，确认后删并回 gallery）。
- 聊天卡片打开的 preview，Back 首版统一回 gallery（不维护跨-view return stack）；可在 `CapsulesPanel` 本地存 `returnView` 增强，但不得把 transcript 结构与 route 耦合。

### 3.5 Desktop routing

```ts
export type DesktopRoute = | ... | { kind:'view'; view:... } | { kind:'capsule'; capsuleId:string };
```

- `#/capsules` → `{kind:'view',view:'capsules'}`；`#/capsules/<uuid>` → `{kind:'capsule',capsuleId}`；`contentViewForDesktopRoute({kind:'capsule'})` → `'capsules'`；`buildDesktopRouteHash` → `#/capsules/<encoded>`。
- `AppShell` lift `capsuleRouteSelection`，传 `CapsulesPanel`：`selectedCapsuleIdFromRoute?`、`onOpenCapsulePreview(id)`、`onCloseCapsulePreview()`。
- Deep link `garyx://capsules/<id>` → `{type:'open-capsule',capsuleId}` → renderer `applyDesktopRoute({kind:'capsule',capsuleId})`。

---

## 4. iOS 画廊与预览

### 4.1 文件落点

Core：

- `GaryxGatewayCapsuleModels.swift`：复用 summary/cacheKey/loadState；新增 `GaryxCapsulePreviewLoadPlanner` / `GaryxCapsulePreviewSlotState`（管理可见 id、最大并发、cache invalidation，SwiftPM 测试覆盖）。若实现要持久化 gallery summary，注意当前 `GaryxCapsuleSummary` 是 `Decodable` only；要么继续用既有 `GaryxCachedCapsule` DTO，要么显式补 `Encodable/Codable`，不要把 `GaryxCapsuleSummary` 直接塞进需要编码的 snapshot/cache。
- `GaryxGatewayClient.swift`：复用 `capsuleHTML(id:)`。
- `GaryxMobileRouteLink.swift`：复用 `.panel(.capsules)`（`garyx://mobile/capsules`）/ `.capsule(String)`（`garyx://mobile/capsule?id=<id>`，构建 `:46`、解析 `:129`）。
- `GaryxMobileRenderState.swift`/`GaryxMobileRenderRows.swift`：聊天卡片映射见 §5。

App target：

- `GaryxMobileCapsuleViews.swift`：`GaryxCapsulesView` grouped `List` → gallery grid；新增/重写 `GaryxCapsuleGalleryCard`、`GaryxCapsulePreviewWebView`、`GaryxCapsuleFocusedPreviewView`；`GaryxCapsuleWebView` 保留安全配置，扩展 `mode:.thumbnail/.focused`。
- `GaryxMobileModel+Capsules.swift`：preview cache / visible slots 加载；刷新后 prune cache。聊天卡入口不要直接调用 private `openCapsuleRoute`；应走 public `openMobileRoute(.capsule(id), source:.conversation)`，再由内部 route 复用 `openCapsuleRoute`/`openCapsule(_:)`。
- 新 Core/app 文件必须 `xcodegen generate` + 提交 `GaryxMobile.xcodeproj/project.pbxproj`（`project.yml` 按路径 glob，不必改）；验证不能只 `swift test`。

### 4.2 iOS Gallery layout

- `GaryxCapsulesView` 保持 `.garyxPageBackground()` + adaptive top bar；`List{Section}` → `ScrollView + LazyVGrid`：iPhone portrait 2 列；landscape/iPad `adaptive(minimum:170,maximum:260)`。
- Card：上半 `GaryxCapsulePreviewWebView` clipped rounded rect；下半 title + small metadata；`contextMenu`/row action 保留 Delete。
- `onAppear/onDisappear` 上报可见 id 给 Core planner；只有 planner 批准的 visible ids 才挂 WKWebView。

### 4.3 iOS WKWebView thumbnail strategy（多实例内存重，比 desktop 更严）

- `maxActivePreviews`：iPhone 2 / iPad 4（首版固定 2，后续按 size class 提升）。`GaryxCapsulePreviewLoadPlanner` 输入 visible ids + capsules → 输出 active ids（按出现顺序，≤N）。
- HTML cache key 与 focused/chat 一致为 `GaryxCapsuleHTMLCacheKey(id,revision)`；`htmlSha256` 可作为 summary 的校验/调试字段，但不参与共享预览缓存主键，避免聊天卡（无 sha）与 gallery/focused 双重 fetch。
- Thumbnail WKWebView：`isUserInteractionEnabled=false`、`scrollView.isScrollEnabled=false`/`bounces=false`、`.nonPersistent()`、`loadHTMLString(html,baseURL:nil)`；虚拟画布 `760x480` + `.scaleEffect(cardWidth/760, anchor:.topLeading)` + 外层 `.frame().clipped()`；无 native bridge，不把 token 放 URL。
- 不后台加载所有 HTML；不创建离屏 WKWebView 生成图片（若实测必须降级，用**客户端** `WKWebView.takeSnapshot` 抓首帧位图后释放 webview —— 仍是客户端快照，不违「不引服务端缩略图」）。

### 4.4 iOS preview 去 chrome

- `GaryxCapsuleDetailView`（`:237`）改 focused preview：保留 `fullScreenCover`（从顶层进入），但内容像网页全屏而非表单 detail；移除顶部 title/description/metadata stack。
- `GaryxCapsuleWebView` 填满，safe-area padding 避开 overlay；顶部极简 overlay：左 close/back（`xmark`/`chevron.down`），中可选短 title（单行小字透明底），右 refresh + overflow（Copy link `garyx://mobile/capsule?id=<id>` / Copy ID / Delete）。Delete 后 dismiss 并刷新 list/cache。
- `garyx://mobile/capsule?id=<id>` route：`openCapsuleRoute` 打开 panel、刷新 capsules、`await openCapsule(capsule)`；`GaryxCapsulesView` 监听 selected id 弹 focused preview。

---

## 5. 聊天内自动补 Capsule 卡片（render_state 设计）

v2 最高风险部分。核心原则：**server 派生，client dumb-render；row 位置由 `garyx-models` reducer 决定；desktop/iOS 不得从 tool rows / capsules list 本地推导插卡。**

### 5.0 两个关键决策（评审重点）

**决策一：capsule 怎么进 render 层？— 选「自包含 committed marker」。**

capsule 在独立 `capsules` 表、不在 transcript ledger 里；reducer 只吃 `seq<=based_on_seq` 的 committed 记录（`thread_history.rs:1211-1217`）。要让 reducer 派生卡片，capsule 必须以某种 committed 记录进入输入。三方案：

| 方案 | 做法 | 合契约 | 取舍 |
|---|---|---|---|
| A. side-input 实时 join（**否决**） | gateway snapshot 时查 `capsules` 表，把 `&[CapsuleRecord]` 旁路传 reducer，按 `run_id` 关联 turn | **违反 write-then-derive** | capsule 无 seq，晚于 `based_on_seq` 创建者会污染旧 snapshot / floor window，replay 不确定 |
| **B. 自包含 committed marker（主方案，推荐）** | capsule_create/update 成功时由 bridge 在该 run 的 `transcript_controls` 中追加一条 `capsule_attached` `RunControlRecord`（自带 `capsule_id/revision/title/run_id`）；reducer 识别 committed control 记录派生卡片 | **完全合契约** | 需一次写入且必须进入 run authoritative drafts（见 §5.2）；reducer 保持 **payload 无关**；gateway/router snapshot 路径**不改**；天然满足「不依赖 garyx_db」 |
| C. reducer 解析已有 tool_result（**备选/降级**） | 不新增写入；reducer 在配对 tool_use/tool_result 时识别 `capsule_*` 工具、解析 result JSON 取 metadata；并由 gateway hydrate 纯 side-input | 合契约（tool_result 本是 committed） | reducer 解析 **provider 相关** tool 载荷（Codex result 自带 `mcp:garyx:*`，Claude result 匿名、必须按 `tool_use_id` 反查 tool_use 的 `mcp__garyx__*`），且 router snapshot API 需要接收纯 context，详见 §5.9 |

**选 B 的决定性理由：把 provider 相关的「tool 结果形态解析」从 reducer（read 路径、每次 snapshot、在纯 `garyx-models` 里）移到 bridge（write 路径、一次、provider 编排层本就该知道这些）。** `garyx-bridge` 按 `CLAUDE.md` 定义即「provider orchestration」，tool_result 形态知识属于它；`garyx-models` reducer 应保持对 provider 无知。B 同时让 reducer/gateway/router 的 snapshot 派生**一行不改**，并自动满足「不依赖 garyx_db」。C 作为「若 bridge 写入 marker 不可行」的完整降级方案保留（§5.9）。

**决策二：新 row 用「字段」还是「enum 变体」？— 选「在 `RenderUserTurnRow` 上加字段」。**

- ✗ 新增顶层 `RenderRow::CapsuleCard` 或 `RenderActivityRow::CapsuleCards` 变体：iOS `GaryxRenderRow`/`GaryxRenderActivityRow` 解码器对未知 `kind` **直接 throw 丢帧**（`GaryxMobileRenderState.swift:171-197`/`:249-281`，`init(from:)` 无 default 分支）。iOS TestFlight 独立发版（`CLAUDE.md` Release 边界）→ **旧 app + 新 gateway** 会丢整帧、聊天崩。
- ✓ 在 `RenderUserTurnRow` 加可选字段 `capsule_cards: Vec<RenderCapsuleCard>`：旧解码器对未知键**静默忽略**（Swift `Codable` / TS 结构化类型）→ 优雅降级（旧端不显示卡片，聊天照常）。一个 turn 的 activity 恒为单元素（`build_activity_rows`），final answer 是该 turn 最后可见内容，卡片渲染在 activity 之后即「final 之后」，无需 interleave，字段足够。
- 防御纵深（强烈建议）：同时给三端的 row/activity enum 解码器加 **tolerant default 分支**（未知 kind → 跳过该 row，不 throw），为未来任何 render_state 演进兜底。这是当前 iOS 解码器的潜在脆弱点，值得在 T1 一并修。

### 5.1 Wire model（主方案）

`garyx-models/src/transcript_render_state.rs` 新增：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderCapsuleCard {
    pub id: String,          // 稳定 row id = "capsule_card:<capsule_id>"
    pub capsule_id: String,
    pub title: String,
    pub revision: i64,       // iframe/webview 缓存键
    pub action: RenderCapsuleAction,   // created | updated（本轮该 capsule 的动作）
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderCapsuleAction { Created, Updated }

// RenderUserTurnRow 增加：
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub capsule_cards: Vec<RenderCapsuleCard>,
```

title/revision/action 全部来自 marker，自包含 → reducer 不查 DB、不解析 tool 载荷。`description/byte_size/html_sha256` 不进 row（预览只需 `capsule_id`+`revision`，serve 永远给最新内容；副信息客户端按需另取）。`deleted` 不进 wire：客户端按 serve 404 判定并渲染 disabled（§6）。

TS mirror（`desktop/garyx-desktop/src/shared/contracts.ts:1390-1488` 区）：

```ts
export interface RenderCapsuleCard { id:string; capsule_id:string; title:string; revision:number; action:'created'|'updated' }
export interface RenderUserTurnRow { kind:'user_turn'; id:string; user:RenderMessageRef|null; activity:RenderActivityRow[]; started_at:string|null; finished_at:string|null; capsule_cards?:RenderCapsuleCard[] }
```

Swift mirror（`GaryxMobileRenderState.swift:199-247`，**不动** `GaryxRenderRow.Kind`/`GaryxRenderActivityRow.Kind`）：

```swift
public struct GaryxRenderCapsuleCard: Codable, Equatable, Identifiable, Sendable { public var id:String; public var capsuleId:String; public var title:String; public var revision:Int; public var action:GaryxRenderCapsuleAction }
// GaryxRenderUserTurnRow 增 capsuleCards: [GaryxRenderCapsuleCard]，decodeIfPresent → 缺省 []；该 snapshot 会被 transcript cache 编码，手写 Codable 的 CodingKeys/init/encode 都要同步更新。
```

### 5.2 写侧：`capsule_attached` committed marker（必须进 `RunControlRecord` / `transcript_controls`）

marker 记录最终落到 transcript ledger 后的形态（由 `RunControlRecord::new` 统一生成；**不要**手写 `role:"control"`）：

```json
{ "seq": N, "thread_id":"...", "run_id":"run::...", "timestamp":"...",
  "message": {
    "role":"system", "kind":"control", "internal":true, "internal_kind":"control",
    "control": {
      "kind":"capsule_attached", "thread_id":"...", "run_id":"run::...", "at":"...",
      "capsule_id":"0190...", "revision":2, "action":"updated", "title":"..."
    }
  }
}
```

写入路径（**唯一主路径**）：

1. **bridge 在成功 capsule tool_result 被纳入 run 内容后，向 run 长 `transcript_controls` 追加 `RunControlRecord::new("capsule_attached", ...)`。** 位置用 `after_content_count = 当前 run content message 数`（即成功 tool_result 内容消息之后、final assistant 之前），这样 `build_run_record_drafts` 在 streaming flush 与 terminal reconcile 中都会把 marker 交织到同一位置。provider 相关的工具名/结果嵌套解析落在 bridge（它本就 provider-aware），不污染 reducer。落点：`garyx-bridge/src/multi_provider/run_management.rs` 的 provider event → `transcript_controls` 管理，以及 `garyx-bridge/src/multi_provider/persistence.rs` 的 `RunControlRecord`/`build_run_record_drafts` 路径；**不新增 router `build_capsule_attached_record`，不复用 `append_range_rewrite_marker`。**
   - **reconcile 存活性是写侧硬要求。** terminal finish 会用 `build_run_record_drafts(run_messages, &transcript_controls)` 重新生成 authoritative tail，再调 `reconcile_run_records_tail`；任何 bridge 累加器之外带外 append 的 marker 都不在 authoritative drafts 中，可能被尾部 reconcile 重写/抹掉。`range_rewrite` 是 reconcile 自己追加的审计 marker，不能作为业务 marker 写入先例。
   - **不得只读 result-side `tool_name`。** Codex completed `mcpToolCall` result 会带 `tool_name="mcp:garyx:capsule_create|update"` 与 `content.type/server/tool`；Claude Code 的 tool_result 明确 `tool_name=None`（`claude_provider.rs:691-699`），只有同 `tool_use_id` 的前序 tool_use 带 `tool_name="mcp__garyx__capsule_create|update"`。所以 bridge marker extractor 必须维护本 run/turn 的 `tool_use_id -> tool_name` 关联，或用 result payload 自识别（见下一条）作为兜底。
   - payload extractor 解析 MCP 返回的 JSON 字符串：Garyx capsule 工具返回的是 stringified JSON，内含 `tool/status/capsule_id/id/title/revision/html_sha256/open_url`（`mcp/tools/capsule.rs:275-301`）。必须覆盖 Claude wrapper `content.result`/`content.text` 与 Codex `content.result`（预期 `content.result.content[].text`，但要用真实 capture 固化）等常见嵌套；解析后得到 `{action,capsule_id,title,revision}` 再写 `RunControlRecord` payload。
   - **实现前置验证**：先用真实 Codex 与 Claude 各跑一次 `capsule_create`，保存脱敏 fixture（合成 thread/run/id/path，无真实个人数据）来 pin completed MCP result nesting；没有这两个 capture，不允许把 marker extractor 判绿。
2. **禁止的写法：MCP handler 或 router 直接 append marker 到 transcript。** handler 数据最权威但不拥有 bridge run 的 `transcript_controls`，带外 append 不是 seq 竞争问题，而是 terminal reconcile 去同步问题；除非未来把 handler 结果作为显式命令回传给 bridge 累加器，否则不作为 v2 主/备选方案。
- 时机：工具调用在 final message 之前（先调工具再写最终答复），marker committed seq 通常落在 tool group 与 final 之间 → reducer **延后渲染**到 final 之后（§5.3，镜像 `take_final_assistant`）。
- run/turn 关联：marker 落在该 run authoritative record 序列内、对应 user-turn 的 block 区间内，reducer 按**序列位置**自然归属（不靠 reducer 读 DB `run_id` 匹配）；`run_id` 仅留作可观测/校验。

### 5.3 Reducer 派生（镜像 `take_final_assistant` 的延后提取）

`reduce_transcript_render_state_with_run_state`（`:198`）主循环里 control 记录现被 `is_control_message → continue` 跳过（`:222-224`）。改为在跳过前先识别 `capsule_attached`：

1. 新增 reducer 内部 side record `CapsuleMark { capsule_id, revision, title, action, seq }`（可以实现成 `RenderBlock::CapsuleMark` 后立即剥离，或更推荐单独 `capsule_marks: Vec<CapsuleMark>`；不要让它成为 tail/status 输入）。主循环遇 `is_control_message(record.message)` 且 `message.control.kind=="capsule_attached"`：先 `current_tool_group.flush_into(blocks)`，把 marker 追加到 side list（带 seq），`continue`；其它 control 仍跳过。
2. **top-level tail/status 保护**：当前 reducer 在 `build_rows` 之前就会调用 `apply_tool_group_statuses(&mut blocks, run_state)` 与 `derive_tail_activity(blocks.last(), run_state)`（约 `:261/:264`），所以不能只在 `build_rows/flush_turn` 里剥离 marker。实现必须在这些调用之前构造 `visible_blocks`（只含 message/tool blocks）和 `capsule_marks`（side list），然后只对 `visible_blocks` 调 `apply_tool_group_statuses`、`build_rows`、`derive_tail_activity`。否则 marker 位于 run tail 时会把 tail activity 误判成 Thinking 或破坏 active tool group。
3. `build_rows_with_capsule_marks(visible_blocks, capsule_marks, run_state)` 按 seq 区间把 marker 归属到 turn：某 turn 的范围是该 user block seq（或 orphan turn 起点）到下一 user block seq 之前；marker seq 落入哪个范围就挂到哪个 `RenderUserTurnRow`。这样仍是序列位置归属，不靠 DB `run_id`。
4. 提取出的 CapsuleMark **按 `capsule_id` 去重保留该轮最大 revision**（同轮先 create rev1 再 update rev2 → 只留一张 rev2、action=updated）→ `Vec<RenderCapsuleCard>`，顺序按首次出现 seq。
5. **busy 门控**：若该 turn 是 trailing turn 且 `run_state.busy`（与 `defer_final` `:616` 同类条件），先不把 `capsule_cards` 挂到 `RenderUserTurnRow`；等 terminal/idle 后下一帧再出现。这样不会出现「工具刚成功、final 还没落 ledger，卡片先出现」。
6. **跨轮 freshness（必答：update 后卡片随 revision 更新）**：serve 永远给最新内容，故卡片 `revision` 应是「与最新内容一致的缓存键」。做法：reducer 入口对输入记录做一遍 `latest_by_capsule: HashMap<capsule_id,(rev,title)>` 预扫描（全是 ledger 内 marker，纯函数、不查 DB），卡片 `revision/title` 取**该范围内全局最新**。后续轮 update → 新 marker（新 committed 记录）→ 新 frame 重算 → 历史轮卡片缓存键一并升到新 revision → 三端按 `capsule_id:revision` 重挂 iframe/webview 刷新到新内容，全端一致。

`CapsuleMark` 不进 `build_activity_rows`、不进 tool group、不进 `visible_message_ids`（它不是消息）；tail/status 相关函数只能看到 `visible_blocks`。

### 5.4 与 final answer / tool row 的相对位置

```text
user bubble
TurnSummary / tool rows / assistant steps (collapsible)
final assistant answer (RenderStepRow.final_message)
capsule cards (RenderUserTurnRow.capsule_cards)   ← 渲染在整轮 activity 之后
```

- run 仍 busy（`defer_final` 的 trailing turn，`:616`）时**绝不**提前显示卡片；等 final/terminal committed 后下一帧再出现 → 不会「卡片先于 final」。
- 异常 run 无 final assistant 但已 terminal：卡片仍挂在该 turn 末（最后 completed step 之后）；客户端不补偿「after final」。
- `AssistantReply` 单消息 turn（无工具）通常无 marker，不受影响。

### 5.5 Gateway / snapshot 路径（主方案：**不改**）

marker 是普通 committed 记录，`render_snapshot_at_seq`/`render_snapshot_in_window`（`thread_history.rs:1206/1220`）按 `seq<=based_on_seq` 过滤后自动喂进 reducer；`thread_render_frame` 下发（`routes.rs:2166-2214`）不改。`based_on_seq` 仍等于当前 snapshot/frame 的 cursor/seq；如果本帧包含新 marker，它自然会前进到包含该 marker 的 seq，**不做任何 backfill 或旧 seq 污染**。无 gateway DB hydrate、无新派生路径、无依赖方向问题。

floor window（`render_snapshot_in_window` `:1220`）：`run_state` 由全 prefix 算、rows 由 window 内记录算（`:1232-1243`）。某 `capsule_attached` 在 `render_floor` 之下（已滚出）→ 其 CapsuleMark 不在 window，对应那轮也滚出，卡片随该轮消失（正确）。§5.3 第 4 步的「全局最新 revision」：

- 全量 snapshot（`render_snapshot_at_seq`，无 floor，**实时常见**）：reducer 见全部记录，预扫描天然全局，freshness 完美。
- window 模式：退化为 window 内最新（below-floor 的 update 看不到），可接受（freshness 主要对近尾轮重要）。要 window 也完美：把 `latest_by_capsule` 像 `run_state` 一样由全 prefix 算好旁路传入 reducer（仍是 ledger 派生、不违契约；§11 R5）。
- **events 摄入（提醒，与 §5.0 决策二的 render_state 解码器是两回事）**：`capsule_attached` 是新上线的 control kind，会随 per-thread SSE `thread_render_frame.events` 下发以同步 cache/cursor。确认 desktop/iOS 的 `events` 摄入对未知 control kind **惰性处理**（仅推进 seq/cursor，不渲染、不报错）——几乎肯定 OK（events 按 seq 通用处理），但作为新 control kind 值得一条断言。

### 5.6 Desktop dumb-render

Files：`shared/contracts.ts`、`renderer/src/render-view-model.ts`、`app-shell/components/ThreadPage.tsx`、新增 `CapsuleChatCard.tsx`。

- `render-view-model.ts:267 buildThreadViewRows`：把 `row.capsule_cards` 透传进 `UserTurnRow` 视图模型（新增 `capsuleCards`）；orphan turn（无 user，走 `rows.push(...activityRows)` `:285`）也把卡片接到 activity 之后。**不**在此做任何分组/配对（守文件头 `:11-20` 「纯结构翻译」契约）；capsule cards 不计入 visible message id。
- `ThreadPage.tsx`：`user_turn` 渲染分支（`:929` 起，userBlock + activityRows 之后）追加 `row.capsuleCards.map(c => <CapsuleChatCard .../>)`；卡片内嵌 `<CapsuleLivePreviewFrame mode="card"/>` + 标题；点击 `onOpenCapsule(c.capsule_id)` → `navigate({kind:'capsule',capsuleId})`。serve 404 → disabled「Capsule deleted」，不加载 HTML。
- 聊天卡性能保守：`IntersectionObserver` 可视区才加载，同时 active ≤ 2；folded/side history 先显 static shell，可见再加载。

### 5.7 iOS dumb-render

Files：`GaryxMobileRenderState.swift`、`GaryxMobileRenderRows.swift`、`App/.../GaryxMobileTurnViews.swift`、`GaryxMobileCapsuleViews.swift`。

- `GaryxMobileRenderRows.swift`：`GaryxMobileTurnRow`（`:33`）加 `capsuleCards: [GaryxRenderCapsuleCard]`（纯透传）。
- mapper `GaryxRenderUserTurnRow.mobileRow`（`:680`）把 `capsuleCards` 映射进 `GaryxMobileTurnRow`（哑映射，不 grouping —— 守 mobile-ui「mapper 哑映射」）；`messageRefs`/`hasUnresolvedVisibleRefs` 忽略 capsule cards（否则初始 history 误判永远 unresolved）。
- view `GaryxMobileTurnRowsView`（`GaryxMobileTurnViews.swift:12`）在该行 activityRows 之后渲染 `row.capsuleCards` → 新 `GaryxMobileCapsuleChatCardsView`（横向 scroll 或单列 compact，上半 `GaryxCapsulePreviewWebView` 下半 title）；点击 `Task { await model.openMobileRoute(.capsule(card.capsuleId), source:.conversation) }`（只走 app route/model，不在 view 查 `/api/capsules` 决定是否存在；`openCapsuleRoute` 当前是 private）。**source 为 conversation 时不得切到 Capsules panel 再返回 overview**：应在当前会话上方 present focused preview，关闭后回到原会话；从 gallery/deep link 进入才使用 panel/gallery 语境。serve 404 → disabled，不 `capsuleHTML(id:)`。
- 纯映射/视图模型在 `GaryxMobileCore` + SwiftPM 测试；卡片 SwiftUI 视图在 app target。

### 5.8 Tests（headless 优先，守 CLAUDE.md「Prioritize headless tests」）

Rust（先写 reducer tests，`test-fixtures/render-layer/render-state-cases.json` + 单测）：

1. `capsule_card_after_final_for_create`：fixture `run_start → user → capsule tool_use/result → capsule_attached(marker) → assistant final → run_complete`，断言 `capsule_cards` 排在 final 之后。
2. `capsule_card_waits_until_not_busy`：busy trailing run 不出卡；terminal/idle 后出。
3. `same_run_create_then_update_dedupes_to_latest_revision`。
4. `multiple_capsules_order_by_first_mark_seq`。
5. `later_run_update_bumps_revision_on_all_cards`（freshness）。
6. `marker_below_render_floor_omits_card`。
7. `non_capsule_control_does_not_emit_card` / `marker_seq_advances_frame_cursor_without_backfilling_old_snapshots`。
8. `capsule_mark_does_not_break_tail_activity_or_tool_group_status`：marker 位于 tool result 后、final 前或 run tail 时，tail/status 仍来自剥离后的 message/tool block。
9. bridge/marker extractor focused tests（不在 reducer 内猜 provider）：`codex_completed_mcp_capsule_result_emits_marker`（result 自带 `tool_name="mcp:garyx:*"`）、`claude_anonymous_tool_result_correlates_tool_use_id_to_emit_marker`（tool_result `tool_name=None`，靠同 id tool_use 的 `mcp__garyx__*`）、`capsule_result_payload_self_identifies_when_tool_name_missing`（JSON 含 `tool/open_url/capsule_id`）。这些 fixture 必须来自真实 completed MCP result capture 的脱敏样本。
10. 写侧/reconcile tests：`capsule_attached_run_control_has_control_envelope`（顶层 `kind/internal_kind=control`）、`capsule_attached_survives_terminal_reconcile`（streaming append 后 terminal reconcile 不丢 marker、不额外 range_rewrite）、`direct_external_marker_append_is_not_supported`（证明带外 append 不作为主路径）。

- iOS：`GaryxMobileRenderStateMapperTests` 解码+映射 `capsule_cards`、断言无 unresolved refs、旧帧无该字段 → `[]` 不崩；`GaryxCapsulePreviewLoadPlannerTests` 覆盖 max active / cache invalidation。
- Desktop：`render-view-model` 测试加 `capsule_cards` 透传与顺序、不计 visible id。

### 5.9 备选/降级方案 C（reducer 解析 tool_result + gateway hydrate）

仅当主方案的 bridge/handler 写入 marker 被证明不可行时启用。要点（保留以便快速切换）：

- reducer 从 committed tool trace 抽「capsule mutation marker」：仅 `role=tool_result`/`tool_use_result=true` 且非 error；工具识别不能只看 result-side `tool_name`。Codex result 可直接识别 `mcp:garyx:capsule_create|update` / `content.type=mcpToolCall`；Claude result 匿名，必须用 `tool_use_id` 反查同组 tool_use 的 `mcp__garyx__capsule_create|update`，或从 result payload 的 `tool/open_url/capsule_id` 自识别。result JSON 解析同样必须覆盖真实 Codex/Claude completed result nesting；`run_id` 取 outer `ThreadTranscriptRecord.run_id`；`source_seq` 取该 tool result seq。
- 为避免 `garyx-models` 依赖 `garyx_db`：新增纯输入 `TranscriptRenderContext{ run_state, capsules:&[TranscriptRenderCapsuleRecord] }`（pure struct）+ `reduce_transcript_render_state_with_context(...)`；保留旧 API（capsules empty）。
- 不在 gateway 复制 `render_snapshot_at_seq`/`render_snapshot_in_window` 的 based_on_seq/window 逻辑。若降级 C 启用，应在 `garyx-router::ThreadTranscriptStore` 增加 `render_snapshot_at_seq_with_context` / `render_snapshot_in_window_with_context`（或参数化现有函数）并复用同一 prefix/window 实现；gateway 只负责 hydrate `state.ops.garyx_db.list_capsules_for_thread(thread_id)` → pure `TranscriptRenderCapsuleRecord` 后传入 router。若必须 gateway 读 records，也只能用 public `records(thread_id)`，不能调用 private `read_records`。
- hydrate 规则：committed tool marker 证明「存在与位置」，DB 行 hydrate「最新 metadata/revision」；DB 缺失 → `deleted:true` tombstone。
- 代价：reducer 携带 provider 相关解析；snapshot 派生新增 context 参数但不复制 window 算法。**故仅作降级。**

---

## 6. Update / delete 语义

### Update 后卡片是否随 revision 更新 —— 是

- Row identity `capsule_card:<capsule_id>`，**不带 revision**，避免 update 后 React/SwiftUI 卸载重建整卡。
- Freshness：§5.3 第 4 步全局最新 revision（主方案，纯 ledger 派生）；预览 cache key `capsule_id:revision`，revision 变 → gallery/preview/chat card 自然 refetch HTML。
- 后续 run 更新同一 capsule：旧 run 卡片显示最新 revision；更新 run 的 final 后也出现一张本轮卡（本轮有 marker）。这表示 Capsule 是「活文档」而非历史快照；版本历史 out of scope。

### Delete 后聊天卡片

- `DELETE /api/capsules/{id}` 硬删 DB/file，历史 `capsule_attached` marker 仍在 ledger。
- 下次 render snapshot：卡片仍派生（marker 在），但 `/serve` **仅 HTTP 404** → 客户端渲染 disabled「Capsule deleted」，不加载 HTML、不刷屏报错（`deleted` 不进 wire，由客户端按 serve 404 判定）。**瞬态/5xx/离线 → 保持 retryable-loading，绝不误标 deleted。**
- 即时性：DELETE handler 可向受影响 thread 发布 snapshot-only `thread_render_frame`（events `[]`，same committed tail seq）让当前打开的聊天即时变 disabled；首版不做也可（重连/重开后更新），但需在风险写明。

---

## 7. 性能设计

### Desktop

- Gallery：`IntersectionObserver` 控制 card mount，只 fetch/mount 可视区 + 小 overscan；同时 loading HTML ≤ 4、mounted iframe ≤ 6（首版常量），超出显 skeleton。
- Card iframe `pointer-events:none`/`tabIndex=-1`/scaled virtual viewport；HTML cache `Map<id:revision, html>`（与画廊/聊天卡同 2 段键，m6），刷新强制 bypass、删除 prune。
- 隐藏即卸载：离开 `contentView==='capsules'`、preview 关闭、card 离屏 → iframe 卸载，避免脚本继续跑。
- Chat card：更保守，同时 active ≤ 2，长 thread 只可视区加载。

### iOS

- WKWebView 并发更低：iPhone 2 / iPad 4；`LazyVGrid` + `onAppear/onDisappear` 只表达可见性，真正准入在 Core planner（可测）。
- 每 view `.nonPersistent()`，离屏即释放；thumbnail `isUserInteractionEnabled=false`+`scrollView.isScrollEnabled=false`，focused 才允许交互。
- `capsuleHTMLCache` 按 `id:revision` cacheKey prune，首版保留最近 ~16 个 HTML 串，防 5MiB×N。

---

## 8. 安全设计（保持 v1 边界）

- Desktop card/focused：`iframe sandbox="allow-scripts" srcDoc={html}`，不加 `allow-same-origin`，不用 Electron `webview`，不注入 preload，renderer 不直接带 token fetch gateway。
- iOS：`.nonPersistent()`、JS allowed、无 `WKScriptMessageHandler`、`loadHTMLString(html,baseURL:nil)`；thumbnail 禁交互；focused 允许网页内交互但外链走 `UIApplication.shared.open`、未知 scheme cancel。
- `/serve` 的 meta CSP + HTTP CSP 照旧（`capsules.rs:23,381`）；srcDoc/loadHTMLString 不继承 HTTP header，故 meta CSP 仍关键。
- 多预览各自 opaque origin、互不可达、无 same-origin、无持久存储。卡片 title/metadata 是 native text（来自 DB/marker/JSON），不作为 HTML 注入。Copy link 用 app deep link，不把 token 放 URL。
- 不引服务端缩略图 → 不新增「服务端渲染不可信 HTML」攻击面；iOS `takeSnapshot` 是本地位图，仍经沙箱 WKWebView 渲染。

---

## 9. iOS 端到端截图验证方案

Gary 修好 CoreSimulator 后，用 `garyx-app-screenshots` skill 对真实 app + 本地 gateway 数据截图。数据准备：

1. 本地 gateway 跑一次真实 agent run，调 `capsule_create` 生成自包含 HTML，final answer 后结束。记 `thread_id`、`capsule_id`。
2. 同 thread 再跑 `capsule_update`，验 revision 更新与本轮 update card。
3. 不用 mock 截图（除非 simulator/gateway 阻塞）；若用 DEBUG fixture，证据须标明 mock。

构建（实现后）：

```bash
cd mobile/garyx-mobile
xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build CODE_SIGNING_ALLOWED=NO
```

三张验收图：

```bash
# S1 卡片画廊
node ~/.garyx/skills/garyx-app-screenshots/scripts/capture-garyx-app-screenshot.mjs ios \
  --device "Garyx Mobile UI QA" --install-latest-build \
  --open-url 'garyx://mobile/capsules' --wait 1800 \
  --output /tmp/garyx-screenshots/capsule-v2-ios-gallery.png
# S2 去套娃预览
node ~/.garyx/skills/garyx-app-screenshots/scripts/capture-garyx-app-screenshot.mjs ios \
  --device "Garyx Mobile UI QA" \
  --open-url 'garyx://mobile/capsule?id=<capsule_id>' --wait 2200 \
  --output /tmp/garyx-screenshots/capsule-v2-ios-preview.png
# S3 聊天 final 后卡片
node ~/.garyx/skills/garyx-app-screenshots/scripts/capture-garyx-app-screenshot.mjs ios \
  --device "Garyx Mobile UI QA" \
  --open-url 'garyx://mobile/thread?threadId=<thread_id>' --wait 2200 \
  --output /tmp/garyx-screenshots/capsule-v2-ios-chat-card.png
```

验收图须证明：S1 是 card grid，至少一张卡上半 live 预览、下半 title；S2 网页占主视觉、无 v1 大块 metadata chrome；S3 聊天 final 之后出现 Capsule card，点击进 app 内 preview（可追加一张 after-tap 截图）。可选 S4：update 前后对比证明卡片随 revision 刷新。Mac 是 IA 真相源，iOS 跟随；desktop 同期落地可并附对照截图。

---

## 10. 分阶段实现建议（只建议，不在本任务建实现子任务）

依赖：T1 →（T2 ∥ T3）→ T4。T2/T3 的「画廊+去套娃」不依赖 T1，可更早起；只有「聊天卡片」依赖 T1 的 contracts 字段。

- **T1 render 契约 + 派生（最高风险，先做，阻塞聊天卡片）**
  - bridge 写侧：把 `capsule_attached` 做成 `RunControlRecord` 压入 run 长 `transcript_controls`（不是 router `build_capsule_attached_record`，不是 MCP handler 带外 append），provider-aware capsule-result extractor 覆盖 Claude tool_use_id correlation + Codex result direct + payload self-identification，并用真实 completed MCP result 脱敏 fixture 固化 nesting；`garyx-models` 加 `RenderCapsuleCard`/`RenderUserTurnRow.capsule_cards`/`RenderBlock::CapsuleMark` + reducer 提取/去重/全局最新 revision/busy 门控/tail 剥离；三端 row/activity 解码器加 tolerant default 分支；`render-state-cases.json` 夹具 + reducer 单测 + 写侧 terminal reconcile 存活测试。
  - 验收：`cargo test -p garyx-models transcript_render_state --all-targets` 全绿；bridge/persistence focused tests 证明 `capsule_attached` control envelope 正确且 terminal reconcile 后仍在；frame cursor/based_on_seq 语义不回填旧 snapshot，cold replay/floor 用例通过；证明非 capsule 记录不插卡、busy 时不提前出卡、final 后顺序。**不碰 UI。**
- **T2 Desktop UI（依赖 T1 contracts 字段）**
  - 抽 `CapsuleLivePreviewFrame`；`CapsulesPanel` 改画廊（§3）；preview 去套娃 + `{kind:'capsule'}` route + deep link（§3.4/3.5）；`render-view-model.ts`/`ThreadPage.tsx` 渲染 `capsule_cards` + 点击路由（§5.6）；contracts.ts 镜像字段；性能（IntersectionObserver / 并发上限 / HTML 缓存上提）。
  - 验收：`npm run build:ui` + focused route/view-model 测试；`npm run dist:dir` 装包 + CDP attach 截画廊/预览/聊天卡。
- **T3 iOS UI（依赖 T1，可与 T2 并行）**
  - Core models/mapper/planner SwiftPM 测试；`GaryxMobileCapsuleViews.swift` 画廊 + 去套娃 + WebView mode 化（§4）；`GaryxMobileRenderState` 镜像字段（`decodeIfPresent`）+ Core mapper 透传 + `GaryxMobileTurnViews.swift` 渲染卡片 + `.capsule(id)` 路由（§5.7）；性能（planner / 并发上限 / 客户端 snapshot 降级）；`xcodegen generate` + 提交 pbxproj。
  - 验收：`swift test`；`xcodebuild ... build CODE_SIGNING_ALLOWED=NO`；§9 三张 iOS 截图。
- **T4 e2e 收尾与加固**
  - 真实 run create/update capsule；desktop 证明截图/录屏（可选）；iOS 三张强制；window 完美 freshness 旁路（§5.5/R5）；delete tombstone 即时性（§6/R7）；review 安全/性能回归（live preview 数量、iframe/WK 清理、route/back 行为）。

---

## 11. 主要风险与取舍

| # | 风险/取舍 | 防线/缓解 |
|---|---|---|
| R1 | **marker 写侧被 terminal reconcile 抹掉** | 只走 bridge `RunControlRecord` → run 长 `transcript_controls` → `build_run_record_drafts` 的 authoritative draft 路径；禁止 MCP handler/router 带外 append。写侧测试必须覆盖 streaming append 后 terminal reconcile 仍保留 marker 且不触发多余 range_rewrite。 |
| R2 | **新 row：字段 vs enum 变体** | 选「`RenderUserTurnRow` 加可选字段」：iOS 对未知 `kind` throw 丢帧、TestFlight 独立发版会让旧 app+新 gateway 崩；字段旧端静默忽略、优雅降级。并给三端解码器加 tolerant default 分支兜底未来演进。 |
| R3 | **render_state 契约腐蚀** | clients 绝不扫 tool result / 查 capsules 本地插卡；存在与位置只由 server render_state 决定；先写 headless reducer/mapper 测试再动 UI（守 C2/C3）。 |
| R4 | **provider 相关解析位置** | 主方案把 tool 结果解析放 bridge（write 一次，provider-aware 层），reducer 纯净、无 provider 耦合、无 garyx_db 依赖。实现必须覆盖 Claude tool_result 匿名/按 tool_use_id 关联、Codex result direct、payload 自识别；仅降级方案 C 才在 reducer 解析 + router context hydrate。 |
| R5 | **floor window 下 freshness 退化** | window 全局最新退化为 window 内最新（§5.5），可接受；要完美则 `latest_by_capsule` 由全 prefix 算好旁路传 reducer（镜像 run_state，仍 ledger 派生）。 |
| R5a | **真实 MCP result nesting 猜错** | 实现前必须捕获并脱敏 Codex+Claude completed `capsule_create` transcript；bridge extractor tests 用这些 fixture，而不是手写想象结构。 |
| R6 | **多 live preview 性能** | IntersectionObserver / LazyVGrid 可见性 + active 上限 + 离屏卸载 + cache 上限 + 冻结交互；iOS 必要时客户端 `takeSnapshot` 降级（§7）。 |
| R7 | **DELETE 后当前线程即时性** | 至少下次 snapshot 显 tombstone；更好是 DELETE handler 发 snapshot-only frame（events `[]`，same tail seq）。 |
| R8 | **不可信 HTML 安全** | card 与 focused preview 同安全策略；不引服务端缩略图不等于放松 sandbox（§8）。 |
| R9 | **Mac/iOS IA 一致** | 卡片/预览/聊天卡的标签、副信息字段、图标语义以 Mac 为准，iOS 仅适配布局（`CLAUDE.md` UI Direction / mobile-ui）。 |
| R10 | **iOS route-driven fullScreenCover 时序 / 返回语义** | 当前 `detailCapsule` 是 `GaryxCapsulesView` 本地 state；从 conversation 打开的 capsule preview 必须 present-over-conversation 并 dismiss 回会话，不可切到 Capsules overview。若时序不稳，把 selected preview 提升到 `GaryxMobileModel`（app target state），Core 仍只放 pure state/route/cache 规则。 |

---

## 12. 一句话总结

复用 v1 全部存储/serve/sandbox/CSP；最关键的需求三通过「capsule_create/update 成功后由 **bridge**（先用真实 Codex/Claude completed result 脱敏 fixture 固化解析；Claude 需 tool_use_id 关联，Codex 可 result direct）把自包含 `capsule_attached` 写成 run 内 `RunControlRecord` 并压入 `transcript_controls`，由 `build_run_record_drafts` 在 streaming append 与 terminal reconcile 双路径确定性提交 → `garyx-models` reducer 在 `RenderUserTurnRow.capsule_cards`（加字段非加变体）按序列位置派生、busy 时不提前、final 之后渲染、全局最新 revision → 三端哑渲染读字段摆卡、点卡进 app 内 preview」，**全程在契约内**（committed ledger 唯一来源、server 派生、write-then-derive、三端不重算、reducer 不依赖 garyx_db）。provider 相关解析留在 bridge、reducer 保持纯净；降级方案 C 若启用也必须通过 router context 复用现有 based_on_seq/window 算法，不在 gateway 复制。
