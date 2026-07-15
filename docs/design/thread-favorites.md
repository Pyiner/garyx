# Thread Favorites (线程收藏)

Status: draft v2 (addressing review round 1 — #TASK-2324 FAIL, 9 findings)
Date: 2026-07-16

## 0. v2 修订记录（对 round 1 findings 的处置）

| # | Finding | 处置 |
|---|---|---|
| 1 | PUT 存在检查与插入分两步，与归档/删除竞争产生幽灵收藏 | 存活检查与 INSERT 收进**同一事务**（存在守卫式插入），加确定性交错测试（§3、§4.1） |
| 2 | 无 revision 的整页快照会倒退，覆盖乐观写 | 增加 `thread_favorites_meta.favorites_revision` 单调版本号；客户端按「revision 单调接受 + pending intent overlay」收敛；iOS 权威状态入 Core（§3、§7） |
| 3 | 「移除+标脏」挡不住行复活，head refresh 保留尾部不收敛 | Favorites feed 改为 **reset-on-refresh** 语义 + 本地 **pending suppression（tombstone）** 后置过滤；写完成后重置并重新 prime feed epoch；补齐全部时序测试（§5.2、§6.1） |
| 4 | Desktop 乐观状态放 main store 层，renderer 看不到即时反馈 | 定义 **renderer 侧 membership ingress**（简化版 pinned-order-ingress：先本地提交 → IPC → 失败回滚 + 过期响应防护）（§5.2） |
| 5 | iOS 选 Favorites 时 canonical All feed 停止维护 | 辅助 All 刷新条件改为**对 filter 枚举穷尽 switch**（所有非 `.all` filter 都刷新 All feed），扩展 refresh-commit 测试（§6.2） |
| 6 | Gateway 切换未清收藏状态，泄漏上一网关数据 | iOS gateway 切换清 `favoritedThreadIds`/pending intents/favorites pager（与 pin 清理同处）；desktop 状态按 `entitiesGatewayUrl` 归一化 key（§5.1、§6.2） |
| 7 | Mac 第二触发点未对齐 pin（Pinned 块有行内 unpin） | 采纳对称方案：**Favorites tab 行内提供取消收藏按钮**（悬浮 star-off，对齐 PinnedThreadsSidebar 行内 unpin 形态）（§5.2） |
| 8 | pin 清理点清单不准，漏 retired-workflow 启动 purge | 清理点显式钉三处：归档事务（mod.rs:668）、通用线程删除（mod.rs:2044）、retired-workflow 启动清理（mod.rs:2776，入口 2698），全部同步删 favorites（§3） |
| 9 | Desktop 契约留白，平行维度会造出 `tasks+favorites` 非法组合 | 客户端契约改**判别联合** `all \| chats \| favorites`，每值恰映射一种 wire 形态，结构性排除非法组合；保留服务端 `tasks=only` 行为与回归测试（§5.1） |

评审已确认的裁决（v2 保持不变）：独立 `thread_favorites` 表 + SQL JOIN 过滤不违反
repository contract；`favorites=only` 与显式 `tasks` 互斥、Favorites 包含 task 线程
语义正确；Core/App 归属方向正确（v2 进一步把 membership 权威状态放入 Core）。

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
  （Favorites feed 的刷新语义见 §5.2/§6.1，与 All/Chats 不同）。
- 跨端收敛模型：发起端乐观更新 + revision 单调接受；其它端靠列表刷新重拉。

**非目标**

- 收藏排序 / 拖拽重排（pin 专属，收藏无此需求 → 不引入 sort_order、reorder CAS、
  reorder outbox；注意：**revision 仍然需要**，用于快照单调接受，见 §7）。
- 首页独立「收藏段」（pin 有 Pinned 独立段；收藏只是筛选类别，不改首页结构）。
- All/Chats 列表行上的星标徽标（未提需求，不做；Favorites tab 行内取消按钮除外，见 §5.2）。
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
  favorites_revision INTEGER NOT NULL
) STRICT;
```

- **真源**：`thread_favorites` 表本身。收藏是独立的用户意图事实（与 pin 同类），
  **不写进 `thread_records` body**，也**不在 `recent_threads` 加列**。
- `favorites_revision`：全局单调递增，任何 favorites 集合变更（插入/删除，含清理点
  删除）在同一事务内 bump（复用 `bump_thread_pins_revision_if_changed_tx` 的形态，
  仅在实际有行变更时 bump）。所有读/写响应携带 revision。它**不用于写侧 CAS**
  （membership 幂等切换无冲突面），只用于客户端快照单调接受（§7）。
- 命名对齐既有先例：`capsules.favorited_at`。
- **写入原子性（对 finding 1）**：收藏插入使用存在守卫式单事务写。route 层仍用
  `ensure_existing_thread_id` 做别名解析与快速 404，但**最终裁决在 db 事务内**：

```sql
INSERT INTO thread_favorites (thread_id, favorited_at)
SELECT ?1, ?2
WHERE EXISTS (SELECT 1 FROM thread_records WHERE key = ?1)
ON CONFLICT(thread_id) DO NOTHING;
```

  插入行数为 0 且线程记录不存在 → 返回 NotFound（route 层映射 404）。这样
  「检查通过 → 归档事务删线程与关联行 → 收藏事务插入」的交错不再可能产生
  幽灵行：归档若先提交，守卫子查询看不到记录，插入为空。**不采用外键级联**：
  仓库现有表均无 FK（`thread_pins` 同为手工清理），全局开 `PRAGMA foreign_keys`
  影响面不可控；以同事务守卫 + 显式清理点 + 交错测试兜底。
- **生命周期清理点（对 finding 8）——精确三处**，每处在同一事务内加
  `DELETE FROM thread_favorites WHERE thread_id = ?`（并 bump revision）：
  1. 线程**归档事务**（`garyx_db/mod.rs:668` 附近，现删 `thread_pins` 处）；
  2. **通用线程删除**（`mod.rs:2044` 附近）；
  3. **retired-workflow 启动清理**（`mod.rs:2776`，入口 `mod.rs:2698`）。
  实现 PR 描述附三处 diff 对照表；tombstone 写入属于归档事务，不存在第四处。
- 契约合规（`docs/agents/repository-contracts.md`）：所有条件查询走 SQL
  （JOIN `thread_favorites`），不引入 `list_keys` / 记录体扫描。先例：
  `task_forest` 的 Pinned scope 直接查 `thread_pins`。（评审已确认合规。）

不选的替代方案及理由：

- **`recent_threads` 加 `favorited` 列**：该表由 thread record 写路径在同事务内派生，
  收藏事实不在 record body 里，加列意味着 upsert 时还要回读 favorites 表回填，
  写路径复杂化且引入两处真相；JOIN 一张小表即可，无性能问题。
- **写进 record body（`thread_kind` 式派生）**：一次星标切换将触发整条 record
  写 + 5 投影派生，代价与语义都不对；pin 的先例已确立独立表模式。
- **外键 ON DELETE CASCADE**：见上，全局 FK 开关影响面不可控，与现有表模式不一致。

## 4. Gateway API

### 4.1 收藏读写（镜像 `/api/thread-pins`，去掉 reorder）

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids: [...], favorites: [{ thread_id, favorited_at }], revision: n }`，按 `favorited_at DESC, thread_id ASC` |
| `PUT /api/thread-favorites/{key}` | 收藏。200 `{ favorited: true, thread_ids, favorites, revision }`；`{key}` 经 `ensure_existing_thread_id` 解析别名；db 层存在守卫式单事务插入（§3），线程不存在 404 `{ favorited: false, error }`；重复收藏幂等 |
| `DELETE /api/thread-favorites/{key}` | 取消收藏。200 `{ favorited: false, removed: bool, thread_id, thread_ids, favorites, revision }`；幂等 |

- 路由注册在 `route_graph.rs`，handler 在 `routes.rs`，紧邻 thread-pins 一组。
- 每个响应都带整页 membership + revision，发起端据此收敛本地状态（§7）。

### 4.2 `/api/recent-threads` 新增 favorites 过滤

- 新增可选 query 参数 `favorites`，唯一合法值 `"only"`；其它值 400
  （沿用 `tasks must be one of: ...` 的显式 400 约定）。
- **参数互斥（评审确认语义正确）**：`favorites=only` 与**显式传入的** `tasks` 参数
  同传 400（`favorites cannot be combined with tasks`）。三个 tab 的映射：
  - All → `tasks=include`（现状）
  - Chats → `tasks=exclude`（现状）
  - Favorites → `favorites=only`（**不带** tasks 参数）
- 服务端既有 `tasks=only` 行为与回归测试（`routes/tests.rs:3896` 附近）**保持不动**。
- **收藏 tab 包含 task 线程**：收藏语义优先于线程类型。
- SQL（`garyx_db/mod.rs` 的 `RecentThreadTaskFilter` 处扩展为统一 recent-thread
  filter 枚举，新增 `Favorites` 变体）：

```sql
SELECT r.* FROM recent_threads r
JOIN thread_favorites f ON f.thread_id = r.thread_id
ORDER BY r.last_active_at DESC, r.thread_id ASC
LIMIT ? OFFSET ?
```

  count 在同一过滤域内计算（保持 `total` / `has_more` 语义，与现有
  `recent_threads_filtered_page_filters_before_pagination` 测试对齐）。
- 返回体结构不变（`RecentThreadRecord` 序列化 + `thread_runtime`），
  **不加 per-row favorited 字段**：Favorites feed 里所有行天然是收藏的；
  菜单里的收藏态由 membership 集合提供。

### 4.3 已知边界

- automation generated / hidden 线程不进 `recent_threads` 投影 → 即使被收藏也不会
  出现在 Favorites tab。这些线程本就不出现在首页任何列表，行为一致，接受。
- 收藏一个后来被归档的线程：归档事务清 favorites 行 + bump revision，
  Favorites tab 不再出现，与 pin 行为一致。

## 5. Desktop（Electron）

### 5.1 契约与主进程

- **契约（对 finding 9）**：`src/shared/contracts/thread.ts` 用**判别联合**表达
  列表过滤：`RecentThreadListFilter = "all" | "chats" | "favorites"`，
  `ListRecentThreadsInput` 携带该联合；wire 映射收敛在 main 层一个纯函数：
  `all → tasks=include`、`chats → tasks=exclude`、`favorites → favorites=only`。
  类型上不存在 `tasks` 与 `favorites` 并存的表达，非法组合被结构性排除。
  新增 `DesktopThreadFavoritesPage`（`thread_ids` + `revision`）。
- `src/main/garyx-client/threads.ts`：`fetchThreadFavorites()`、
  `setRemoteThreadFavorited(threadId, favorited)`（PUT/DELETE）；
  `fetchRecentThreads` 按判别联合拼参数；`validateListRecentThreadsInput`
  同步校验联合值。
- `src/main/store.ts`：desktop-state 增 `favoritedThreadIds` + `favoritesRevision`，
  **按 `entitiesGatewayUrl` 归一化 key 存取**（对 finding 6，形态对齐 store.ts:498
  的 pin 处理）；`mergeRemoteDesktopState` 刷新时并行 `fetchThreadFavorites`，
  合并规则 = revision 单调接受 + 本地 pending intent 优先（§7）。
- `src/main/index.ts` + `src/preload/index.ts`：IPC `setThreadFavorited`（返回
  最终页 + revision）、membership 随 desktop-state 下发。

### 5.2 Renderer

- **乐观层（对 finding 4）**：新增轻量 **favorites membership ingress**
  （`favorites-ingress.ts`，简化自 `pinned-order-ingress.ts`：仅 membership，
  无排序）。交互路径 = renderer 先 `commitLocalMembership`（星标态即时翻转）
  → IPC → 成功用响应页按 revision 收敛 / 失败回滚；带 request stamp 丢弃过期
  响应。AppShell 持有 `favoritedThreadIds` 的 renderer 权威视图（本地 overlay
  ∪ 主进程快照），菜单勾选态、行内按钮态都读它。
- **入口 1（与 pin 同菜单）**：`ConversationHeaderTitle.tsx` 的 dropdown 菜单，
  紧邻 "Pin/Unpin conversation" 增加 "Favorite conversation" / "Unfavorite
  conversation"（Star / StarOff 图标，`icons.tsx` 新增，风格对齐
  PinIcon/PinOffIcon）。不设快捷键。
- **入口 2（对 finding 7，对齐 pin 的第二触发点）**：**Favorites tab 的行内
  取消收藏按钮**——悬浮显示 StarOff，形态与 `PinnedThreadsSidebar.tsx:145` 的
  行内 unpin 完全对称。仅 Favorites tab 的行有此按钮；All/Chats 行不加。
- **筛选 tab**：`app-shell/recent-thread-feeds.ts` 扩展为
  `RecentThreadFilter = "all" | "nonTask" | "favorites"`（renderer 内部枚举名
  沿用现状，wire 映射走 §5.1 联合），第三条独立 feed，label `"Favorites"`；
  `useRecentThreadFeeds.ts`、`RecentConversationSidebar.tsx`（第三 tab +
  tabRefs + 方向键）、`recent-conversation-sidebar-model.ts`
  （空态 "No favorite threads"）。
- **Favorites feed 一致性（对 finding 3）**：
  1. **reset-on-refresh**：Favorites feed 的 refresh 一律整体重置（epoch bump、
     丢弃已加载尾部、从 head 重拉），不做 head-merge 保尾。收藏集合小
     （个位~数十），整拉成本可忽略，换取跨端取消收藏/尾部变化的完全收敛。
  2. **pending suppression**：乐观取消收藏 → thread_id 进 suppression 集合，
     作为 feed 行的**后置过滤**（不管行来自已加载页还是 in-flight 响应，一律
     隐藏）；写确认后触发 feed 重置并清除该 suppression；写失败回滚并清除。
     这样「DELETE 未提交时 load-more 返回该行」也不会复活——行进来即被
     suppression 滤掉。重新收藏 = 从 suppression 移除 + feed 重置。
- i18n：`renderer/src/i18n/index.tsx` 增 "Favorites"、"Favorite conversation"、
  "Unfavorite conversation"、空态文案。

## 6. iOS

### 6.1 GaryxMobileCore（业务逻辑全部下沉 Core，SwiftPM 可测）

- **membership 权威状态入 Core（对 finding 2）**：新增
  `GaryxFavoritesState`（纯逻辑，形态取 `GaryxPinnedOrderState` 的 membership
  子集：pending intents、`highestObservedRevision`、快照单调接受、失败回滚），
  App 层只做 IO 编排。
- `GaryxRecentThreadFilter`：新 case `.favorites`；`displayName = "Favorites"`；
  `activeStatusLabel = "Favorites"`；`homeMenuOptions` 加入（过滤器菜单遍历该
  数组自动带出，无需改 `GaryxRecentThreadFilterMenu.swift`）；wire 映射改为
  返回结构化 query 描述（能表达 `tasks=…` 或 `favorites=only` 二选一），
  保持纯函数 + 穷尽测试。
- `GaryxRecentThreadFeeds`：第三条 feed。**Favorites feed 的刷新/变更语义
  （对 finding 3）与 §5.2 相同**：reset-on-refresh + pending suppression 后置
  过滤 + 写确认后重置 re-prime。注意现有 `GaryxHomeThreadListPager` 的语义是
  「mutation 后发出的请求仍可提交」（PagerTests:552 明确保留），**不改动该
  pager 语义**——suppression 在 feed 层做后置过滤，复活行进来也显示不出去。
- `GaryxRecentThreadFilterStorage`：持久化新 case（UserDefaults 往返）。
- `GaryxGatewayClient`：`listThreadFavorites()`、`setThreadFavorited(id:_:)`
  （PUT/DELETE）；`listRecentThreads` 支持 favorites 过滤参数。
- `GaryxGatewayThreadModels`：`GaryxThreadFavoritesPage`（兼容解码
  `thread_ids` / `favorites[].thread_id` + `revision`）。
- `HomeProjectionReducer` / `HomeProjectionActor`：
  `favoritesChanged(favoritedThreadIds:revision:)` action；
  `GaryxHomeThreadListPresentation` 把 `favoritedThreadIds` 传入行呈现，
  供长按菜单判定收藏态。

### 6.2 App 层

- **入口 1（长按）**：`GaryxMobileSidebarViews.swift` 的 `.garyxThreadActionMenu`
  行菜单，紧邻 "Pin thread"/"Unpin thread" 增加 "Favorite thread" /
  "Unfavorite thread"（`star` / `star.slash`）→
  `model.toggleFavoriteThread(row.id)`。
- **入口 2（线程内右上角）**：`GaryxMobileConversationViews.swift` 的 title 菜单
  （line 942 附近），紧邻 Pin 项增加同样的收藏项。
- **入口 3（过滤器）**：右上角 `GaryxRecentThreadFilterMenu` 自动出现
  Favorites 项。
- `GaryxMobileModel+ThreadPersistence.swift`：`isThreadFavorited` /
  `toggleFavoriteThread` —— IO 编排薄层，状态裁决全部委托 Core 的
  `GaryxFavoritesState`。
- `GaryxMobileModel+ThreadList.swift`：`refreshThreads` 的 `async let` 并行组
  增拉 `listThreadFavorites`，与 pins 同批落投影。
  **辅助 All feed 刷新（对 finding 5）**：现有「仅 `.nonTask` 时额外刷新 All」
  的条件改为**对 `GaryxRecentThreadFilter` 穷尽 switch**：`.all` → 只刷 All；
  其它一切 case（`.nonTask`、`.favorites` 及未来新增）→ 刷新自身 feed +
  辅助刷新 canonical All feed（widget/自动化/其他投影依赖它）。穷尽 switch
  保证未来加 case 编译期强制表态。扩展
  `GaryxHomeThreadListRefreshCommitTests`（:523 附近）覆盖 `.favorites`。
- **Gateway 切换清理（对 finding 6）**：`GaryxMobileModel+Gateway.swift`（:55
  附近清 pin 处）同步清空 `favoritedThreadIds`、`GaryxFavoritesState` pending
  intents、Favorites pager/feed 状态。
- **新增 Core 文件必须跑 `xcodegen generate` 并提交 pbxproj**（swift test 假绿
  教训），验证走 `xcodebuild`。

## 7. 同步模型

- **服务端**：`favorites_revision` 全局单调，任何集合变更（含清理点）bump；
  所有响应携带 `{ thread_ids, favorites, revision }` 整页。
- **发起端**：本地 pending intent 即时生效（乐观）→ API 调用 → 成功后按
  「revision ≥ 本地最高已见 revision 才接受快照」收敛，pending 中的 intent
  覆盖快照对应项（overlay）；失败回滚 intent。
- **其它端**：desktop 靠 `mergeRemoteDesktopState` 周期刷新、iOS 靠
  `refreshThreads`（10s 轮询 / 下拉 / 动作触发）重拉 `/api/thread-favorites`，
  同样按 revision 单调接受——旧响应网络乱序后到（finding 2 的
  「B 先到 A 后到回退 {A}」时序）被单调规则丢弃。
- 无写侧 CAS：membership 幂等切换，后写覆盖先写即正确语义；revision 只做
  快照排序，不做条件写。

## 8. 测试计划

**Gateway（`cargo test -p garyx-gateway --lib`）**

- db：favorite/unfavorite 幂等；revision 仅在实际变更时 bump；
  **归档/删除/retired-workflow purge 三清理点**各自同事务清行 + bump revision；
  **确定性交错测试（finding 1）**：模拟「存在检查后、插入前归档提交」顺序，
  断言收藏插入为空、GET 无幽灵行；
  `favorites=only` 分页在过滤域内计 `total`/`has_more`；JOIN 查询走一次读快照
  （对齐 `..uses_one_read_snapshot` 模式）。
- routes：三端点契约（404 / 幂等 / 整页 + revision）；`favorites` 非法值 400；
  `favorites=only` + 显式 `tasks` 同传 400；既有 `tasks=only` 回归保持。

**Desktop（`npm run test:unit`）**

- `recent-thread-feeds.test.mjs`：第三 feed；**Favorites feed reset-on-refresh
  （尾部被丢弃重拉）**；suppression 后置过滤（乐观取消期间 load-more 返回该行
  不显示；确认后重置；失败回滚重现）。
- favorites-ingress 测试：本地即时翻转、失败回滚、过期响应（stale stamp）丢弃、
  revision 单调接受（乱序旧快照不回退）。
- sidebar 测试：三 tab 渲染、方向键循环、空态文案、Favorites 行内取消按钮。
- `gary-client.test.mjs`：判别联合 → wire 参数映射穷尽；非法值拒绝。
- store：按 gateway key 归一化的状态隔离（切网关不串数据）。

**iOS（SwiftPM `swift test` + `xcodebuild` 编译验证）**

- `GaryxFavoritesState`：pending intent overlay、revision 单调接受（乱序回退
  时序）、失败回滚、gateway 切换清空。
- Feeds：**finding 3 全时序**——乐观取消期间 in-flight load-more 返回该行不复活；
  多页尾部跨端取消后 refresh 完全收敛；重新收藏恢复；连续多次取消。
- `GaryxRecentThreadFilterStorageTests`：新 case 往返；
  `GaryxGatewayClientTests`：favorites query 与三端点；
  Reducer/Actor/Presentation：favoritesChanged 与长按菜单收藏态；
  `GaryxHomeThreadListRefreshCommitTests`：`.favorites` 选中时 All feed 仍刷新。

**端到端**

- 本地起 gateway，curl 三个 favorites 端点 + `recent-threads?favorites=only`
  验证真实返回（含 revision 单调）；双端 UI 按 `garyx-product-ui` skill 走查
  两处入口 + 筛选切换 + Favorites tab 行内取消。

## 9. 实现切分建议

单 PR 可完成，但按依赖顺序提交：① gateway（表 + revision + API + 过滤 + 三清理点）
→ ② desktop（契约判别联合/IPC/store → ingress → renderer）→ ③ iOS（Core：
FavoritesState + feeds → App → xcodegen）。每步各自带测试。
