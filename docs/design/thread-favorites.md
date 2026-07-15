# Thread Favorites (线程收藏)

Status: draft v3 (addressing review round 2 — #TASK-2324 FAIL, 7 findings)
Date: 2026-07-16

## 0. 修订记录

### Round 2 findings → v3 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | `thread_favorites_meta` 未初始化 singleton 行，GET/bump 会炸 | 显式 `ensure_thread_favorites_meta_row`（INSERT (1,0) ON CONFLICT DO NOTHING），**早于启动 purge** 执行；`CHECK (favorites_revision >= 0)`；重开幂等测试（§3） |
| 2 | membership 与 revision 必须同事务快照组页，否则 `{旧集合, 新 revision}` 响应会被单调规则错误接受 | 明确不变量：**GET 与全部写响应的 `{thread_ids, favorites, revision}` 在单一事务内组装**（对齐 pin 的单 WAL 读事务 mod.rs:538）；加确定性交错测试（对齐 mod.rs:5462 模式）（§4.1、§8） |
| 3 | revision 只排序服务端提交，封不死请求乱序（Favorite→Unfavorite，DELETE 先落库、旧 PUT 后落，终态违背最后意图） | 客户端契约升级：**per-thread single-flight + latest-intent-wins 合并**（同线程同时至多一个 in-flight membership 写；期间新意图只覆盖待发意图，前一请求完成后才发下一个）；保留 accepted raw snapshot，intent 退休后重投影（对齐 GaryxPinnedOrderState:227/:290）。Desktop ingress 弃用整集 overlay/整集回滚，改 **per-ID intent token**（§5.2、§6.1、§7） |
| 4 | reset-on-refresh 按现有 Core `reset()` 实现会每 10s 清空 UI 显示 skeleton | 改为 **replacement-refresh**：bump epoch 废弃旧请求但保留可见缓存；成功时**原子整体替换**（含丢尾部）；失败保留缓存（§5.2、§6.1） |
| 5 | db 最终 NotFound 的类型与 404 映射未定义 | db 层返回专用结果枚举 `FavoriteThreadResult::{Updated(page) \| NotFound}`，route 显式映射 404；不依赖 route 前置检查（§4.1） |
| 6 | Desktop 行内取消收藏未覆盖共享行组件双 action 结构；图标不应自造 SVG | 显式扩展共享 `ThreadRailRow` 行 action 契约（可组合 accessory：Favorites tab 行 = Unfavorite + Archive 并存）；图标用 `lucide-react` 的 Star/StarOff（§5.2） |
| 7 | 不用 FK 的理由与现码不符（`foreign_keys=ON` 早已开启）；purge 行号应为 2785 | 修正理由：FK 级联删除**不会 bump `favorites_revision`**，破坏 revision 单调收敛契约，且与 `thread_pins` 无 FK 的既有模式不一致——显式删除 + 同事务 bump 本来就必须存在，级联反而制造第二条无版本删除路径；行号更正（§3） |

### Round 1 findings → v2 处置（已被 round 2 核验）

1（幽灵行 TOCTOU）守卫式单事务插入 + 单 writer mutex（mod.rs:363）串行化，**已封死**；2（快照倒退）revision 方向正确，round 2 细化为本轮 findings 1–3；3（load-more 复活）epoch reset + 后置 suppression **已挡住**，衍生本轮 finding 4；4（renderer 乐观层）已补，多 intent 回滚细化为本轮 finding 3；5/6/8/9（All feed 维护、gateway 切换清理、三清理点、判别联合）**处置正确**；7（Mac 第二触发点行内取消）产品裁决正确，落点细化为本轮 finding 6。

评审已确认的裁决（保持不变）：独立 `thread_favorites` 表 + SQL JOIN 过滤合规；
`favorites=only` 与显式 `tasks` 互斥、Favorites 包含 task 线程语义正确；
Core/App 归属方向正确。

## 1. 需求

用户需求（产品裁决，不可改动的部分）：

1. 线程支持「收藏」（favorite）。
2. 最近线程列表的筛选类别变为三个：**全部（All）/ Chats / 收藏（Favorites）**。
3. iOS：
   - 首页线程行**长按 context menu** 出现收藏项；
   - 进入线程后**右上角菜单**里也有收藏项，与置顶（Pin）位置相邻；
   - 首页**右上角过滤器**增加「收藏」类别，点击查看收藏线程。
4. Mac app：收藏的触发点与置顶的触发点一致（同菜单同位置）；筛选处新增一个「收藏」tab。

## 2. 目标 / 非目标

**目标**

- 每线程一个可切换的收藏标记，gateway 持久化，双端（desktop + iOS）入口与置顶入口同位。
- `/api/recent-threads` 支持服务端收藏过滤（与现有 `tasks` 过滤同层、同分页语义）。
- 双端筛选器各加一个 Favorites 类别，复用现有 per-filter feed 状态机骨架
  （Favorites feed 的刷新语义见 §5.2/§6.1：replacement-refresh，与 All/Chats 不同）。
- 跨端收敛模型：per-thread single-flight 乐观写 + revision 单调接受；
  其它端靠列表刷新重拉。

**非目标**

- 收藏排序 / 拖拽重排（pin 专属 → 不引入 sort_order、reorder CAS、reorder outbox；
  注意 **revision 仍然需要**，用于快照单调接受，见 §7）。
- 首页独立「收藏段」（收藏只是筛选类别，不改首页结构）。
- All/Chats 列表行上的星标徽标（Favorites tab 行内取消按钮除外，见 §5.2）。
- SSE 推送收藏变更（pin 也没有；沿用刷新收敛）。
- bot 命令面（`/threads` 等）不加收藏筛选。

## 3. 数据模型（gateway SQLite）

```sql
CREATE TABLE IF NOT EXISTS thread_favorites (
  thread_id    TEXT PRIMARY KEY,
  favorited_at TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS thread_favorites_meta (
  id                 INTEGER PRIMARY KEY CHECK (id = 1),
  favorites_revision INTEGER NOT NULL CHECK (favorites_revision >= 0)
) STRICT;
```

- **真源**：`thread_favorites` 表本身。收藏是独立的用户意图事实（与 pin 同类），
  **不写进 `thread_records` body**，也**不在 `recent_threads` 加列**。
- **meta 行初始化（对 round2-1）**：`initialize_connection` 建表后立即
  `ensure_thread_favorites_meta_row`：`INSERT INTO thread_favorites_meta
  (id, favorites_revision) VALUES (1, 0) ON CONFLICT(id) DO NOTHING`
  （镜像 `ensure_thread_pins_meta_row`，mod.rs:3529），**顺序必须早于
  retired-workflow 启动清理**（pin 的先例：mod.rs:2669 的调用点在 purge 之前）。
  重开数据库幂等（不重置 revision）。
- `favorites_revision`：全局单调递增，任何 favorites 集合变更（插入/删除，含三个
  清理点）在同一事务内 bump（复用 `bump_thread_pins_revision_if_changed_tx` 形态，
  mod.rs:2924：仅在实际有行变更时 bump）。它**不用于写侧 CAS**，只用于客户端快照
  单调接受（§7）。
- 命名对齐既有先例：`capsules.favorited_at`。
- **写入原子性（round1-1，已核验封死）**：收藏插入使用存在守卫式单事务写。
  route 层仍用 `ensure_existing_thread_id` 做别名解析，但**最终裁决在 db 事务内**：

```sql
INSERT INTO thread_favorites (thread_id, favorited_at)
SELECT ?1, ?2
WHERE EXISTS (SELECT 1 FROM thread_records WHERE key = ?1)
ON CONFLICT(thread_id) DO NOTHING;
```

  守卫插入 0 行且线程记录不存在 → db 层返回 `FavoriteThreadResult::NotFound`
  （§4.1）。单 writer mutex（mod.rs:363）+ SQLite writer 串行化保证：归档先提交则
  守卫看不到记录、插入为空；插入先提交则归档事务的清理点删掉它——两个方向都无幽灵行。
- **为什么不用 FK 级联（对 round2-7，理由修正）**：`foreign_keys=ON` 在连接初始化
  时已开启（mod.rs:2403），技术上可用；不用的真实理由是：
  1. 级联删除**不经过 bump 路径**，`favorites_revision` 不递增，客户端单调收敛
     契约被破坏（删除后旧快照反而更"新"）；显式删除 + 同事务 bump 无论如何必须
     存在，级联只会制造第二条无版本删除路径；
  2. 与 `thread_pins` 的既有模式（无 FK、清理点显式删除）保持一致，避免同类事实
     两套生命周期机制。
- **生命周期清理点（round1-8，行号按现码更正）**——精确三处，每处同一事务内
  `DELETE FROM thread_favorites WHERE thread_id = ?` 并 bump revision：
  1. 线程**归档事务**（mod.rs:668 附近，现删 `thread_pins` 处；tombstone 写入
     属于本事务，不构成第四处）；
  2. **通用线程删除**（mod.rs:2044 附近）；
  3. **retired-workflow 启动清理**（删除语句 mod.rs:2785，入口 mod.rs:2698）。
  实现 PR 描述附三处 diff 对照表。
- 契约合规：所有条件查询走 SQL（JOIN `thread_favorites`），无 `list_keys` /
  记录体扫描（评审已确认合规；先例 = `task_forest` Pinned scope 查 `thread_pins`）。

不选的替代方案：`recent_threads` 加列（写路径复杂化、两处真相）；写进 record body
（一次星标触发整条 record 写 + 5 投影派生）；FK 级联（见上）。

## 4. Gateway API

### 4.1 收藏读写（镜像 `/api/thread-pins`，去掉 reorder）

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids: [...], favorites: [{ thread_id, favorited_at }], revision: n }`，按 `favorited_at DESC, thread_id ASC` |
| `PUT /api/thread-favorites/{key}` | 收藏。200 `{ favorited: true, thread_ids, favorites, revision }`；线程不存在 404 `{ favorited: false, error }`；重复收藏幂等 |
| `DELETE /api/thread-favorites/{key}` | 取消收藏。200 `{ favorited: false, removed: bool, thread_id, thread_ids, favorites, revision }`；幂等 |

- **同快照组页不变量（对 round2-2）**：GET 在**单一读事务**内读出
  `{favorites 行集, revision}`（对齐 pin 的 `list_pinned_threads` 单 WAL 读事务，
  mod.rs:538）；PUT/DELETE 在**写事务内**完成变更 + bump 后、同一事务内读回整页与
  revision 再返回。任何路径都不允许「集合一个事务、revision 另一个事务」。
  配确定性交错测试（对齐 mod.rs:5462 的 pin 交错测试模式）。
- **NotFound 类型（对 round2-5）**：db 层收藏写返回专用结果枚举
  `FavoriteThreadResult::{Updated(ThreadFavoritesPage) | NotFound}`（形态对齐
  `ReorderThreadPinsResult`）；route 层把 `NotFound` 显式映射 404
  `{ favorited: false, error }`。不依赖 `ensure_existing_thread_id` 前置检查
  做最终裁决（它只负责别名解析与友好 404 短路）。
- 路由注册在 `route_graph.rs`，handler 在 `routes.rs`，紧邻 thread-pins 一组。

### 4.2 `/api/recent-threads` 新增 favorites 过滤

- 新增可选 query 参数 `favorites`，唯一合法值 `"only"`；其它值 400
  （沿用 `tasks must be one of: ...` 的显式 400 约定）。
- **参数互斥（评审确认语义正确）**：`favorites=only` 与**显式传入的** `tasks`
  同传 400（`favorites cannot be combined with tasks`）。三个 tab 映射：
  All → `tasks=include`；Chats → `tasks=exclude`；Favorites → `favorites=only`
  （不带 tasks）。
- 服务端既有 `tasks=only` 行为与回归测试（routes/tests.rs:3896 附近）保持不动。
- **收藏 tab 包含 task 线程**：收藏语义优先于线程类型。
- SQL（`RecentThreadTaskFilter` 处扩展统一 filter 枚举，新增 `Favorites` 变体）：

```sql
SELECT r.* FROM recent_threads r
JOIN thread_favorites f ON f.thread_id = r.thread_id
ORDER BY r.last_active_at DESC, r.thread_id ASC
LIMIT ? OFFSET ?
```

  count 在同一过滤域内计算（对齐
  `recent_threads_filtered_page_filters_before_pagination`）；page+count 走一次
  读快照（对齐 `..uses_one_read_snapshot`）。
- 返回体结构不变；**不加 per-row favorited 字段**。

### 4.3 已知边界

- automation generated / hidden 线程不进 `recent_threads` 投影 → 收藏了也不出现在
  Favorites tab（这些线程本就不出现在首页，行为一致，接受）。
- 收藏的线程被归档：归档事务清 favorites 行 + bump revision，与 pin 行为一致。

## 5. Desktop（Electron）

### 5.1 契约与主进程

- **契约（round1-9）**：`src/shared/contracts/thread.ts` 用**判别联合**
  `RecentThreadListFilter = "all" | "chats" | "favorites"` 表达列表过滤；wire 映射
  收敛在 main 层一个纯函数（`all → tasks=include`、`chats → tasks=exclude`、
  `favorites → favorites=only`），类型上排除 `tasks`+`favorites` 并存。
  新增 `DesktopThreadFavoritesPage`（`thread_ids` + `revision`）。
- `src/main/garyx-client/threads.ts`：`fetchThreadFavorites()`、
  `setRemoteThreadFavorited(threadId, favorited)`（PUT/DELETE）；
  `fetchRecentThreads` 按判别联合拼参数；`validateListRecentThreadsInput` 校验联合值。
- `src/main/store.ts`：desktop-state 增 `favoritedThreadIds` + `favoritesRevision`，
  **按 `entitiesGatewayUrl` 归一化 key 存取**（round1-6，对齐 store.ts:498）；
  `mergeRemoteDesktopState` 刷新时并行 `fetchThreadFavorites`，合并 = revision
  单调接受 + pending intent 优先（§7）。
- `src/main/index.ts` + `src/preload/index.ts`：IPC `setThreadFavorited`（返回
  终页 + revision）、membership 随 desktop-state 下发。

### 5.2 Renderer

- **乐观层（round1-4 + round2-3）**：新增 `favorites-ingress.ts`。**不是**
  `pinned-order-ingress` 的整集 overlay 简化版——那套整集 rollback 在「A、B 两个
  乐观写都失败且 A 先失败」时会留下幽灵乐观态（epoch 跳过 A 的回滚、B 回滚到含 A
  的旧集合）。favorites ingress 的结构：
  - **per-ID intent**：`Map<threadId, {desired: boolean, token}>`，翻转即建/覆盖
    本 ID 的 intent；
  - **per-thread single-flight**：同一 threadId 至多一个 in-flight 请求；in-flight
    期间用户再翻转只更新该 ID 的 desired（latest-intent-wins），当前请求完成后若
    desired 与已确认态不一致则续发下一个请求；
  - **raw snapshot 保留**：主进程快照原样保存，呈现值 = raw snapshot ⊕ 所有
    pending intents（逐 ID overlay）；某 ID intent 失败只清该 ID 的 intent
    （呈现自动回落 raw），不做整集回滚；
  - **revision 单调**：接受主进程快照的条件 = `revision >= 已见最高`；
  - 过期响应用 per-ID token 判弃。
  菜单勾选态、行内按钮态都读 ingress 的呈现值。
- **入口 1（与 pin 同菜单）**：`ConversationHeaderTitle.tsx` dropdown，紧邻
  "Pin/Unpin conversation" 增加 "Favorite conversation" / "Unfavorite
  conversation"。**图标用 `lucide-react` 的 `Star` / `StarOff`**（round2-6，
  不自造 SVG；现有 PinIcon 自绘是因为需要 1:1 对齐 Codex app 的历史原因，
  收藏无此约束）。不设快捷键。
- **入口 2（round1-7 + round2-6）**：Favorites tab 行内取消收藏按钮。落点：
  **扩展共享 `ThreadRailRow` 的行 action 契约**（`ThreadConversationSidebar.tsx`
  中声明 Archive 为唯一行 action 的结构升级为可组合 accessory 列表），Favorites
  tab 的行传 `[Unfavorite, Archive]` 两个 action（悬浮显示，StarOff + 既有
  Archive），其它 tab 行为不变（仍只有 Archive）。不在局部叠按钮、不 fork 行组件。
- **筛选 tab**：`app-shell/recent-thread-feeds.ts` 扩展
  `RecentThreadFilter = "all" | "nonTask" | "favorites"`（renderer 内部枚举沿用
  现状命名，wire 走 §5.1 联合），第三条独立 feed，label `"Favorites"`；
  `useRecentThreadFeeds.ts`、`RecentConversationSidebar.tsx`（第三 tab + tabRefs +
  方向键）、`recent-conversation-sidebar-model.ts`（空态 "No favorite threads"）。
- **Favorites feed 一致性（round1-3 + round2-4）**：
  1. **replacement-refresh**：refresh 时 bump epoch 废弃 in-flight 旧请求，
     **保留当前可见行**；成功后用新结果**原子整体替换**（含丢弃已加载尾部——
     跨端取消收藏/尾部变化由此完全收敛）；失败保留原缓存。不出现清空-再加载的
     skeleton 闪烁。收藏集合小（个位~数十），整拉成本可忽略。
  2. **pending suppression**：乐观取消收藏 → threadId 进 suppression 集合，作为
     feed 行的**后置过滤**（已加载页与 in-flight 响应一律隐藏）；写确认后触发
     replacement-refresh 并清 suppression；失败回滚并清除。重新收藏 = 移除
     suppression + replacement-refresh。
- i18n：增 "Favorites"、"Favorite conversation"、"Unfavorite conversation"、
  空态文案。

## 6. iOS

### 6.1 GaryxMobileCore（业务逻辑全部下沉 Core，SwiftPM 可测）

- **`GaryxFavoritesState`（round1-2 + round2-3）**：纯逻辑权威状态，形态取
  `GaryxPinnedOrderState` 的 membership 子集并保留其关键机制：
  - **per-thread single-flight**：同线程禁止第二个并发 membership 请求
    （对齐 GaryxPinnedOrderState:227 的既有裁决）；in-flight 期间的新意图
    latest-intent-wins 合并为待发 desired，前一请求完成后按需续发；
  - **raw snapshot 保留 + 重投影**：accepted raw membership 原样保存，呈现值 =
    raw ⊕ pending intents；intent 退休（成功/失败/被覆盖）后重投影
    （对齐 GaryxPinnedOrderState:290）；
  - **revision 单调接受**：低于已见最高 revision 的快照丢弃；
  - gateway 切换全清（§6.2）。
- `GaryxRecentThreadFilter`：新 case `.favorites`；`displayName` /
  `activeStatusLabel = "Favorites"`；`homeMenuOptions` 加入（过滤器菜单遍历自动
  带出）；wire 映射改为结构化 query 描述（`tasks=…` 与 `favorites=only` 二选一），
  纯函数 + 穷尽测试。
- `GaryxRecentThreadFeeds`：第三条 feed。**replacement-refresh（对 round2-4）**：
  不复用现有 `reset()`（它清 IDs、`isPrimed=false`，配合 10s 静默刷新会周期性
  skeleton）；新增「epoch bump + 保留可见缓存 + 成功原子替换（丢尾部）+ 失败保留」
  的刷新路径，保持现有 pager「refresh 不截断已加载内容」契约对 All/Chats 不变。
  **不改 `GaryxHomeThreadListPager` 的 mutation/请求语义**（PagerTests:552 保留的
  「mutation 后发出的请求仍可提交」不动）——suppression 在 feed 层后置过滤。
- `GaryxRecentThreadFilterStorage`：持久化新 case（UserDefaults 往返）。
- `GaryxGatewayClient`：`listThreadFavorites()`、`setThreadFavorited(id:_:)`；
  `listRecentThreads` 支持 favorites 参数。
- `GaryxGatewayThreadModels`：`GaryxThreadFavoritesPage`（兼容解码
  `thread_ids` / `favorites[].thread_id` + `revision`）。
- `HomeProjectionReducer` / `HomeProjectionActor`：
  `favoritesChanged(favoritedThreadIds:revision:)`；
  `GaryxHomeThreadListPresentation` 把收藏集合传入行呈现，供长按菜单判定收藏态。

### 6.2 App 层

- **入口 1（长按）**：`GaryxMobileSidebarViews.swift` 的 `.garyxThreadActionMenu`
  行菜单，紧邻 Pin 项增加 "Favorite thread" / "Unfavorite thread"
  （`star` / `star.slash`）→ `model.toggleFavoriteThread(row.id)`。
- **入口 2（线程内右上角）**：`GaryxMobileConversationViews.swift` title 菜单
  （:942 附近），紧邻 Pin 项增加同样收藏项。
- **入口 3（过滤器）**：`GaryxRecentThreadFilterMenu` 自动出现 Favorites 项。
- `GaryxMobileModel+ThreadPersistence.swift`：`isThreadFavorited` /
  `toggleFavoriteThread` —— IO 编排薄层，状态裁决全部委托 `GaryxFavoritesState`
  （含 single-flight 排队、续发）。
- `GaryxMobileModel+ThreadList.swift`：`refreshThreads` 并行组增拉
  `listThreadFavorites`，与 pins 同批落投影。
  **辅助 All feed（round1-5）**：「仅 `.nonTask` 时额外刷新 All」改为对
  `GaryxRecentThreadFilter` **穷尽 switch**：`.all` → 只刷 All；其它一切 case →
  刷自身 feed + 辅助刷新 canonical All feed（widget/自动化/投影依赖）。未来加 case
  编译期强制表态。扩展 `GaryxHomeThreadListRefreshCommitTests`（:523 附近）。
- **Gateway 切换清理（round1-6）**：`GaryxMobileModel+Gateway.swift`（:55 附近清
  pin 处）同步清空 `GaryxFavoritesState`（raw + intents + revision 水位）与
  Favorites feed 状态。
- **新增 Core 文件必须跑 `xcodegen generate` 并提交 pbxproj**，验证走 `xcodebuild`。

## 7. 同步模型

- **服务端**：`favorites_revision` 全局单调，任何集合变更（含三清理点）同事务
  bump；**所有响应的页与 revision 同事务组装**（§4.1 不变量）。
- **发起端（客户端写契约，对 round2-3）**：
  - per-thread **single-flight**：同 threadId 至多一个 in-flight membership 写，
    杜绝「旧 PUT 晚于新 DELETE 落库」的请求乱序——同 ID 第二个意图只覆盖待发
    desired，前一请求 settle 后按需续发；
  - pending intent 即时生效（乐观），呈现值 = raw snapshot ⊕ pending intents；
  - 失败只退休该 ID intent（无整集回滚）；
  - 快照接受规则 = `revision >= 已见最高`。
- **其它端**：desktop 靠 `mergeRemoteDesktopState`、iOS 靠 `refreshThreads`
  （10s 轮询 / 下拉 / 动作）重拉 `/api/thread-favorites`，同按 revision 单调接受，
  乱序旧响应被丢弃。
- 无写侧 CAS：membership 幂等切换 + 客户端 single-flight 已保证最后意图胜出。

## 8. 测试计划

**Gateway（`cargo test -p garyx-gateway --lib`）**

- meta 初始化：建库即有 `(1,0)` 行；**重开数据库幂等不重置 revision**；初始化
  顺序早于启动 purge（purge 触发 bump 不炸）。
- db：favorite/unfavorite 幂等；revision 仅实际变更时 bump；三清理点各自同事务
  清行 + bump；**守卫插入交错测试**（round1-1：存在检查后归档提交，断言插入空、
  无幽灵行）；**同快照组页交错测试**（round2-2：并发写下 GET 不产生
  `{旧集合, 新 revision}`，对齐 mod.rs:5462 模式）；`favorites=only` 分页过滤域
  内计 total/has_more；page+count 一次读快照。
- routes：三端点契约（`FavoriteThreadResult::NotFound` → 404 / 幂等 / 整页 +
  revision）；`favorites` 非法值 400；`favorites=only`+显式 `tasks` 400；
  既有 `tasks=only` 回归保持。

**Desktop（`npm run test:unit`）**

- favorites-ingress：per-ID intent 覆盖与退休；**同 ID 反向切换**（Favorite→
  Unfavorite in-flight 合并、settle 后续发）；**不同 ID 部分失败**（A 失败只回落
  A，B 不受影响，无幽灵乐观态）；**远端较新反向状态**（revision 更高的快照推翻
  本地已确认态）；过期响应 token 判弃；revision 单调。
- `recent-thread-feeds.test.mjs`：第三 feed；**replacement-refresh**（刷新期间
  保留可见行、成功原子替换丢尾部、失败保留缓存、无清空中间态）；suppression
  后置过滤（乐观取消期间 load-more 返回该行不显示；确认后替换；失败回滚重现）。
- sidebar：三 tab 渲染、方向键、空态、**Favorites 行 accessory = Unfavorite +
  Archive 并存**（其它 tab 仍仅 Archive）。
- `gary-client.test.mjs`：判别联合 → wire 映射穷尽；非法值拒绝。
- store：按 gateway key 归一化的状态隔离。

**iOS（SwiftPM `swift test` + `xcodebuild` 编译验证）**

- `GaryxFavoritesState`：single-flight（同线程并发第二请求被拒/合并）；
  latest-intent-wins 续发；raw snapshot 重投影；revision 单调（乱序回退时序）；
  失败仅退休单 ID；gateway 切换全清。
- Feeds：**replacement-refresh 不清可见缓存**（10s 静默刷新无 skeleton；失败保
  缓存）；乐观取消期间 in-flight load-more 不复活行；多页尾部跨端取消后 refresh
  完全收敛；重新收藏恢复；连续多次取消。
- Storage 往返；GatewayClient 三端点 + favorites query；
  Reducer/Actor/Presentation：favoritesChanged 与长按菜单收藏态；
  RefreshCommitTests：`.favorites` 选中时 All feed 仍刷新。

**端到端**

- 本地起 gateway：curl 三端点 + `recent-threads?favorites=only`（含 revision
  单调与同快照组页目测）；双端 UI 按 `garyx-product-ui` 走查两处入口 + 筛选切换 +
  Favorites tab 行内取消。

## 9. 实现切分建议

单 PR 可完成，按依赖顺序提交：① gateway（表 + meta 初始化 + revision + API +
过滤 + 三清理点）→ ② desktop（契约判别联合/IPC/store → per-ID ingress →
renderer/行 action 契约扩展）→ ③ iOS（Core：FavoritesState + feeds → App →
xcodegen）。每步各自带测试。
