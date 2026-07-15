# Thread Favorites (线程收藏)

Status: draft v1 (pending adversarial design review)
Date: 2026-07-16

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
- 双端筛选器各加一个 Favorites 类别，复用现有 per-filter feed 状态机。
- 跨端收敛模型与 pin 一致：发起端乐观更新，其它端靠列表刷新重拉。

**非目标**

- 收藏排序 / 拖拽重排（pin 专属，收藏无此需求 → 不引入 sort_order、revision、CAS、outbox）。
- 首页独立「收藏段」（pin 有 Pinned 独立段；收藏只是筛选类别，不改首页结构）。
- 列表行上的星标徽标（未提需求，不做）。
- SSE 推送收藏变更（pin 也没有；沿用刷新收敛）。
- bot 命令面（`/threads` 等）不加收藏筛选。

## 3. 数据模型（gateway SQLite）

新表，镜像 `thread_pins` 模式但更简：

```sql
CREATE TABLE IF NOT EXISTS thread_favorites (
  thread_id    TEXT PRIMARY KEY,
  favorited_at TEXT NOT NULL
) STRICT;
```

- **真源**：`thread_favorites` 表本身。收藏是独立的用户意图事实（与 pin 同类），
  **不写进 `thread_records` body**，也**不在 `recent_threads` 加列**。
- 无 `sort_order`（无重排需求）、无 revision 元表（membership 是幂等 PUT/DELETE，
  无并发重排冲突面，不需要 CAS）。
- 命名对齐既有先例：`capsules.favorited_at`。
- **生命周期与 `thread_pins` 逐点对齐**：线程归档 / 删除时，在**同一事务**内删除对应
  `thread_favorites` 行。实现时 grep `thread_pins` 的全部清理点（归档、删除、
  tombstone），每处同步加 favorites 清理；一次性对照表进入实现 PR 描述。
- 契约合规（`docs/agents/repository-contracts.md`）：所有条件查询走 SQL
  （JOIN `thread_favorites`），不引入 `list_keys` / 记录体扫描。先例：
  `task_forest` 的 Pinned scope 直接查 `thread_pins`。

不选的替代方案及理由：

- **`recent_threads` 加 `favorited` 列**：该表由 thread record 写路径在同事务内派生，
  收藏事实不在 record body 里，加列意味着 upsert 时还要回读 favorites 表回填，
  写路径复杂化且引入两处真相；JOIN 一张小表即可，无性能问题（收藏数量级为个位到
  数十）。
- **写进 record body（`thread_kind` 式派生）**：一次星标切换将触发整条 record
  写 + 5 投影派生，代价与语义都不对；pin 的先例已确立独立表模式。

## 4. Gateway API

### 4.1 收藏读写（镜像 `/api/thread-pins`，去掉 reorder）

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids: [...], favorites: [{ thread_id, favorited_at }] }`，按 `favorited_at DESC, thread_id ASC` |
| `PUT /api/thread-favorites/{key}` | 收藏。200 `{ favorited: true, thread_ids, favorites }`；`{key}` 经 `ensure_existing_thread_id` 解析，不存在 404 `{ favorited: false, error }`；重复收藏幂等（`ON CONFLICT DO NOTHING`） |
| `DELETE /api/thread-favorites/{key}` | 取消收藏。200 `{ favorited: false, removed: bool, thread_id, thread_ids, favorites }`；幂等 |

- 路由注册在 `route_graph.rs`，handler 在 `routes.rs`，紧邻 thread-pins 一组。
- 每个响应都带整页 membership（同 pin 的响应形态），发起端用它直接收敛本地状态。

### 4.2 `/api/recent-threads` 新增 favorites 过滤

- 新增可选 query 参数 `favorites`，唯一合法值 `"only"`；其它值 400
  （沿用 `tasks must be one of: ...` 的显式 400 约定）。
- **参数正交性裁决**：`favorites=only` 与**显式传入的** `tasks` 参数互斥，同传 400
  （`favorites cannot be combined with tasks`）。三个 tab 的映射为：
  - All → `tasks=include`（现状）
  - Chats → `tasks=exclude`（现状）
  - Favorites → `favorites=only`（**不带** tasks 参数）
- **收藏 tab 包含 task 线程**：收藏语义优先于线程类型；用户明确收藏过的东西不应被
  类型过滤吃掉。
- SQL（`garyx_db/mod.rs` 的 `RecentThreadTaskFilter` 处扩展为一个统一的
  recent-thread filter 枚举，新增 `Favorites` 变体）：

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
  All/Chats feed 的行不需要展示星标；菜单里的收藏态由 membership 集合提供。

### 4.3 已知边界

- automation generated / hidden 线程不进 `recent_threads` 投影 → 即使被收藏也不会
  出现在 Favorites tab。这些线程本就不出现在首页任何列表，行为一致，接受。
- 收藏一个后来被归档的线程：归档事务清 favorites 行，Favorites tab 不再出现，
  与 pin 行为一致。

## 5. Desktop（Electron）

### 5.1 契约与主进程

- `src/shared/contracts/thread.ts`：`RecentThreadTaskFilter` 联合类型扩展（或并列新增
  favorites 维度类型，以实现阶段贴合现有代码形状为准）；新增
  `DesktopThreadFavoritesPage` 契约。
- `src/main/garyx-client/threads.ts`：`fetchThreadFavorites()`、
  `setRemoteThreadFavorited(threadId, favorited)`（PUT/DELETE）；
  `fetchRecentThreads` 支持 favorites 参数；`validateListRecentThreadsInput`
  同步校验。
- `src/main/store.ts`：desktop-state 增 `favoritedThreadIds`；
  `mergeRemoteDesktopState` 刷新时并行 `fetchThreadFavorites`；
  `setDesktopThreadFavorited` = 乐观更新 → 远端调用 → 失败回滚。
  **不需要** pin 的 `PinnedOrderController` / outbox / ingress 三件套
  （那是重排 CAS 的复杂度，收藏没有）。
- `src/main/index.ts` + `src/preload/index.ts`：IPC `setThreadFavorited`、
  membership 随 desktop-state 下发。

### 5.2 Renderer

- **入口 1（与 pin 同菜单）**：`ConversationHeaderTitle.tsx` 的 dropdown 菜单，
  紧邻 "Pin/Unpin conversation" 增加 "Favorite conversation" / "Unfavorite
  conversation"（Star / StarOff 图标，`icons.tsx` 按需新增，风格对齐既有
  PinIcon/PinOffIcon）。不设快捷键（pin 的 ⌥⌘P 是历史行为，收藏暂不占快捷键）。
- **入口 2 说明**：desktop 的 pin 第二触发点是左栏 Pinned 块的行内 unpin 按钮，
  那是 pin 专属 UI（收藏无独立段）；desktop 线程列表行当前没有 context menu，
  不为收藏新造一个。desktop 的收藏入口 = 会话头部菜单一处 + 新 tab 的查看面。
- **筛选 tab**：`app-shell/recent-thread-feeds.ts` 扩展
  `RecentThreadFilter = "all" | "nonTask" | "favorites"`，第三条独立 feed，
  label `"Favorites"`，query 映射 favorites=only；`useRecentThreadFeeds.ts`、
  `RecentConversationSidebar.tsx`（第三个 tab + tabRefs + 方向键）、
  `recent-conversation-sidebar-model.ts`（空态 "No favorite threads"）。
- **feed 一致性**：在 Favorites tab 内取消收藏 → 该行立即从 favorites feed 移除
  （复用 `removeThreadFromRecentFeeds` 式 mutation）；在其它 tab 收藏/取消不影响
  该 tab 的行集。收藏状态变更后将 favorites feed 标脏（下次切入时 refresh），
  避免陈旧。
- i18n：`renderer/src/i18n/index.tsx` 增 "Favorites"、"Favorite conversation"、
  "Unfavorite conversation"、空态文案。

## 6. iOS

### 6.1 GaryxMobileCore（业务逻辑全部下沉 Core，SwiftPM 可测）

- `GaryxRecentThreadFilter`：新 case `.favorites`；`displayName = "Favorites"`；
  `activeStatusLabel = "Favorites"`（头部显示 "Recent · Favorites"）；
  `homeMenuOptions` 加入（菜单 UI 遍历该数组自动带出，无需改
  `GaryxRecentThreadFilterMenu.swift`）；query 映射从单一 `tasksQueryValue`
  演进为能表达 favorites=only 的形态（保持纯函数 + 测试）。
- `GaryxRecentThreadFeeds`：第三条 feed（复用 `GaryxHomeThreadListPager`），
  所有 switch 穷尽新 case。
- `GaryxRecentThreadFilterStorage`：持久化新 case（UserDefaults 往返）。
- `GaryxGatewayClient`：`listThreadFavorites()`、`setThreadFavorited(id:_:)`
  （PUT/DELETE）；`listRecentThreads` 支持 favorites 过滤参数。
- `GaryxGatewayThreadModels`：`GaryxThreadFavoritesPage`（兼容解码
  `thread_ids` / `favorites[].thread_id`，形态对齐 `GaryxThreadPinsPage` 但无
  revision）。
- `HomeProjectionReducer` / `HomeProjectionActor`：`favoritesChanged(favoritedThreadIds:)`
  action；`GaryxHomeThreadListPresentation` 把 `favoritedThreadIds` 传入行呈现，
  供长按菜单判定收藏态。
- **feed mutation 对称性**：Favorites tab 内取消收藏 → 行立即移除，且 loadMore /
  dedup-append 不得复活已移除行（对齐 `GaryxHomeThreadListPager` 的既有教训与
  测试模式）。

### 6.2 App 层

- **入口 1（长按）**：`GaryxMobileSidebarViews.swift` 的 `.garyxThreadActionMenu`
  行菜单，紧邻 "Pin thread"/"Unpin thread" 增加 "Favorite thread" /
  "Unfavorite thread"（`star` / `star.slash`）→
  `model.toggleFavoriteThread(row.id)`。
- **入口 2（线程内右上角）**：`GaryxMobileConversationViews.swift` 的 title 菜单，
  紧邻 Pin 项增加同样的收藏项。
- **入口 3（过滤器）**：右上角 `GaryxRecentThreadFilterMenu` 因遍历
  `homeMenuOptions` 自动出现 Favorites 项。
- `GaryxMobileModel+ThreadPersistence.swift`：`isThreadFavorited` /
  `toggleFavoriteThread`（乐观更新 + 失败回滚，形态仿 pin 的
  `beginThreadPinRequest`/`finishThreadPinRequest` 但无 pinned-order 状态机）。
- `GaryxMobileModel+ThreadList.swift`：`refreshThreads` 的 `async let` 并行组
  增拉 `listThreadFavorites`，与 pins 同批落到投影。
- **新增 Core 文件必须跑 `xcodegen generate` 并提交 pbxproj**（swift test 假绿
  教训），验证走 `xcodebuild`。

## 7. 同步模型

与 pin 完全一致，无新机制：

- 发起端：乐观本地更新 → API 调用 → 用响应整页 membership 收敛 → 失败回滚。
- 其它端：desktop 靠 `mergeRemoteDesktopState` 周期刷新、iOS 靠
  `refreshThreads`（10s 轮询 / 下拉 / 动作触发）重拉 `/api/thread-favorites`。
- 无 revision CAS：membership 幂等切换，后写覆盖先写即为正确语义。

## 8. 测试计划

**Gateway（`cargo test -p garyx-gateway --lib`）**

- db：favorite/unfavorite 幂等；归档/删除同事务清 favorites 行；
  `favorites=only` 分页在过滤域内计 `total`/`has_more`（对齐既有
  `recent_threads_filtered_page_filters_before_pagination` 模式）；
  JOIN 查询走一次读快照（对齐 `..uses_one_read_snapshot` 模式）。
- routes：三端点契约（404 / 幂等 / 整页响应）；`favorites` 非法值 400；
  `favorites=only` + 显式 `tasks` 同传 400。

**Desktop（`npm run test:unit`）**

- `recent-thread-feeds.test.mjs`：第三 feed 独立分页/切换/mutation；
- sidebar 测试：三 tab 渲染、方向键循环、空态文案；
- `gary-client.test.mjs`：favorites 参数拼接与校验；
- store：favorited 乐观切换 + 失败回滚 + 刷新合并。

**iOS（SwiftPM `swift test` + `xcodebuild` 编译验证）**

- `GaryxRecentThreadFeedsTests`：三 feed；`GaryxRecentThreadFilterStorageTests`：
  新 case 往返；`GaryxGatewayClientTests`：favorites query 与三端点；
  Reducer/Actor/Presentation：favoritesChanged 与长按菜单收藏态。

**端到端**

- 本地起 gateway，curl 三个 favorites 端点 + `recent-threads?favorites=only`
  验证真实返回；双端 UI 按 `garyx-product-ui` skill 走查两处入口 + 筛选切换。

## 9. 实现切分建议

单 PR 可完成，但按依赖顺序提交：① gateway（表 + API + 过滤）→ ② desktop
（契约/IPC/store → renderer）→ ③ iOS（Core → App → xcodegen）。每步各自带测试。
