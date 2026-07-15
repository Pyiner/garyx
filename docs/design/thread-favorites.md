# Thread Favorites (线程收藏)

Status: draft v4 (addressing review round 3 — #TASK-2324 FAIL, 4 blocking + 1 factual)
Date: 2026-07-16

## 0. 修订记录

### Round 3 findings → v4 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | single-flight 把「当前 flight」与「最新 desired」塞进同一个 `{desired, token}`，新意图覆盖 token 导致旧 flight 响应被误弃、终态漂移 | 状态机拆为**两个独立身份**：`inFlight {requestToken, target, flightGeneration}` 与 `latestDesired {generation, desired}`；新意图**永不**取消当前 flight 的结算资格；响应处理顺序写死为「结算 flight → revision 门控接受 raw → 对比 latestDesired → drain 续发」；失败只按 generation 判定退休哪个意图（§7.2 完整规格） |
| 2 | Desktop raw 所有权自相矛盾（main 合并 pending 优先 vs renderer 把 main 快照当 raw） | 裁决唯一模型：**main 对 favorites 只发布纯 raw**（仅来自服务端页：周期 fetch + 写响应），不做任何 pending 投影（与 pin 的 main 侧 controller 模式**有意分道**，理由见 §5.1）；全部 intent/single-flight/drain 状态唯一地活在 renderer ingress（§5.1、§5.2） |
| 3 | replacement-refresh 保缓存 + DELETE 确认即清 suppression → 行从保留缓存复活；refresh 失败则持续显示 | suppression 升级为分级 tombstone：`hidden-pending`（乐观期）→ DELETE 200 后转 `hidden-tombstone`（确认删除），**只有「DELETE settle 之后签发的 replacement ticket」成功时才与整页替换原子退休**；replacement 失败则 tombstone 存续、行保持隐藏（§7.3） |
| 4 | replacement 期间 load-more 未隔离：旧 offset 页可追加到新 head，跳窗/复活旧行 | replacement 开始 = 同时废弃 refresh/load-more 两个 track + 清 active flags + 关闭分页 gate（cursor 冻结）；settle 前拒绝新 load-more；成功 = 原子换页 + cursor 重置到新 head + 开 gate；失败 = 保留显示缓存 + 以旧 cursor 开 gate（stale-but-consistent，tombstone 继续过滤）；两种完成顺序都进测试（§7.4） |
| 5 | `ensure_existing_thread_id` 并不解析别名（只 trim + `is_thread_key` 校验 + 存在 point-check） | 措辞更正：route 层前置为「规范 thread key 校验 + 存在检查的友好 404 短路」，最终裁决仍在 db 事务（§4.1） |

### Round 2 → v3 处置（round 3 已逐项裁决）

round2-1 meta 初始化 **CONFIRMED**；round2-2 同快照组页 **CONFIRMED**；
round2-3/4 细化为本轮 findings 1–4；round2-5 NotFound 枚举 **CONFIRMED**；
round2-6 行 accessory 契约 **CONFIRMED**；round2-7 FK 理由/清理点行号 **CONFIRMED**
（`thread_pins` 生产删除点：显式 unpin mod.rs:581、归档 mod.rs:669、通用删除
mod.rs:2045、启动 purge mod.rs:2785；favorites 的显式删除点即 DELETE 端点本身，
生命周期清理点三处无遗漏）。

### Round 1 → v2 处置（round 2 已核验）

幽灵行守卫插入封死；load-more 复活被 suppression 挡住（本轮继续加固）；
All feed 穷尽 switch、gateway 切换清理、判别联合、Mac 行内取消均正确。

评审已确认的全局裁决：独立 `thread_favorites` 表 + SQL JOIN 合规；
`favorites=only` 与显式 `tasks` 互斥、Favorites 含 task 线程语义正确；
Core/App 分层与双端入口正确。

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

- 每线程一个可切换的收藏标记，gateway 持久化，双端入口与置顶入口同位。
- `/api/recent-threads` 支持服务端收藏过滤（与 `tasks` 过滤同层、同分页语义）。
- 双端筛选器各加 Favorites 类别；Favorites feed 用 replacement-refresh 语义（§7.4）。
- 跨端收敛：客户端统一收藏意图状态机（§7.2–7.4）+ revision 单调接受。

**非目标**

- 收藏排序 / 拖拽重排（不引入 sort_order、reorder CAS、reorder outbox；
  revision 仍需要，用于快照单调接受）。
- 首页独立「收藏段」；All/Chats 行星标徽标（Favorites tab 行内取消按钮除外）。
- SSE 推送收藏变更（沿用刷新收敛）。
- bot 命令面不加收藏筛选。
- 收藏意图跨进程/重启持久化（pin 的 reorder outbox 为排序耐久性而设；收藏意图是
  瞬态 UI 状态，进程退出丢意图、回落 raw，可接受）。

## 3. 数据模型（gateway SQLite）——round 2/3 已 CONFIRMED

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

- **真源**：`thread_favorites` 表。不写 `thread_records` body、不在
  `recent_threads` 加列。
- **meta 初始化**：建表后立即 `ensure_thread_favorites_meta_row`
  （`INSERT (1,0) ON CONFLICT(id) DO NOTHING`，镜像 mod.rs:3529 的
  `ensure_thread_pins_meta_row`），**早于 retired-workflow 启动清理**
  （pin 先例：mod.rs:2669 调用点在 purge 前）。重开幂等，不重置 revision。
- `favorites_revision` 全局单调；任何集合变更（插入/删除，含三清理点）同事务
  bump（镜像 mod.rs:2924，仅实际变更时 bump）。不用于写侧 CAS。
- **守卫式单事务插入**（round 1 反例已封死；单 writer mutex mod.rs:363 +
  SQLite writer 串行化）：

```sql
INSERT INTO thread_favorites (thread_id, favorited_at)
SELECT ?1, ?2
WHERE EXISTS (SELECT 1 FROM thread_records WHERE key = ?1)
ON CONFLICT(thread_id) DO NOTHING;
```

  守卫 0 行且记录不存在 → `FavoriteThreadResult::NotFound`。
- **不用 FK 级联**：级联删除不经 bump 路径，`favorites_revision` 不递增，破坏
  单调收敛契约（显式删除 + 同事务 bump 无论如何必须存在）；且与 `thread_pins`
  无 FK 模式一致。（`foreign_keys=ON` 本身已开启，mod.rs:2403——不是理由。）
- **生命周期清理点，精确三处**（同事务删行 + bump）：
  归档事务（mod.rs:669 附近）、通用线程删除（mod.rs:2045 附近）、
  retired-workflow 启动清理（删除语句 mod.rs:2785，入口 mod.rs:2698）。
  实现 PR 描述附三处 diff 对照表。
- 契约合规：条件查询全 SQL（JOIN），无 `list_keys`/记录体扫描。

## 4. Gateway API——round 2/3 已 CONFIRMED（含措辞更正）

### 4.1 收藏读写

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids, favorites: [{thread_id, favorited_at}], revision }`，按 `favorited_at DESC, thread_id ASC` |
| `PUT /api/thread-favorites/{key}` | 200 `{ favorited: true, thread_ids, favorites, revision }`；线程不存在 404 `{ favorited: false, error }`；幂等 |
| `DELETE /api/thread-favorites/{key}` | 200 `{ favorited: false, removed, thread_id, thread_ids, favorites, revision }`；幂等 |

- **同快照组页不变量**：GET 在单一读事务内读 `{行集, revision}`（对齐 pin
  mod.rs:538）；PUT/DELETE 在写事务内变更 + bump 后**同一事务**读回整页。
  配确定性交错测试（对齐 mod.rs:5462 模式）。
- **NotFound**：db 层 `FavoriteThreadResult::{Updated(ThreadFavoritesPage) |
  NotFound}`（形态对齐 `ReorderThreadPinsResult`）；route 显式映射 404。
- **route 前置（round3-5 措辞更正）**：`ensure_existing_thread_id` 做的是
  **规范 thread key 校验（trim + `is_thread_key`）+ 存在 point-check 的友好
  404 短路**（routes.rs:1104），不是别名解析；最终存在性裁决在 db 事务内的
  守卫插入。
- 路由注册 `route_graph.rs`，handler 在 `routes.rs` 紧邻 thread-pins。

### 4.2 `/api/recent-threads` favorites 过滤

- 新可选参数 `favorites`，唯一合法值 `"only"`，其它值 400；与显式 `tasks`
  同传 400（`favorites cannot be combined with tasks`）。
- 映射：All → `tasks=include`；Chats → `tasks=exclude`；Favorites →
  `favorites=only`。既有 `tasks=only` 行为与回归（routes/tests.rs:3896）不动。
- **Favorites 含 task 线程**（收藏语义优先于类型）。
- SQL：`recent_threads r JOIN thread_favorites f ON f.thread_id = r.thread_id
  ORDER BY r.last_active_at DESC, r.thread_id ASC LIMIT ? OFFSET ?`；count 同
  过滤域；page+count 一次读快照（对齐既有两条测试模式）。
- 返回体不变；不加 per-row favorited 字段。

### 4.3 已知边界

- automation generated / hidden 线程不进投影 → 收藏了也不出现在 Favorites tab
  （本就不出现在首页，一致，接受）。
- 收藏的线程被归档：归档事务清行 + bump，与 pin 一致。

## 5. Desktop（Electron）

### 5.1 契约与主进程——**raw 唯一所有权（对 round3-2）**

裁决：**favorites 的乐观状态机只有一个 owner = renderer ingress**。main 进程
对 favorites 不做任何 pending 投影，这与 pin 的 main 侧
`PinnedOrderController`（store.ts:765 发布 `presentedOrder`）**有意分道**。
理由：pin 的 main 侧 controller 存在是为了 reorder outbox 的持久化与重试；
收藏无 reorder、意图瞬态（§2 非目标），双层投影只会制造 round3-2 的 raw 污染。

- main 发布的 `favoritedThreadIds` + `favoritesRevision` **定义为纯 raw**：
  仅来自服务端页——周期 `fetchThreadFavorites`（`mergeRemoteDesktopState`
  并行拉取）与写响应回传页；main 侧接受规则同样是 revision 单调。
  **main 永不把 pending 混入该字段**（契约测试钉死：写请求 in-flight 期间
  main 发布值不变）。
- **按 `entitiesGatewayUrl` 归一化 key 存取**（对齐 store.ts:498），切网关不串。
- `src/shared/contracts/thread.ts`：判别联合
  `RecentThreadListFilter = "all" | "chats" | "favorites"`；wire 映射收敛在
  main 层纯函数（`all→tasks=include`、`chats→tasks=exclude`、
  `favorites→favorites=only`），类型排除 `tasks`+`favorites` 并存。
  新增 `DesktopThreadFavoritesPage`（`thread_ids` + `revision`）。
- `src/main/garyx-client/threads.ts`：`fetchThreadFavorites()`、
  `setRemoteThreadFavorited(threadId, favorited)`；`fetchRecentThreads` 按联合
  拼参；`validateListRecentThreadsInput` 校验。
- IPC：`setThreadFavorited(threadId, favorited)` = **纯转发**（HTTP 调用 →
  返回 `{page}` 或错误；main 仅用返回页按 revision 单调更新 raw 缓存）。

### 5.2 Renderer

- **favorites-ingress（唯一乐观层）**：实现 §7.2 的每线程意图状态机 +
  §7.3/7.4 的 feed 一致性。呈现值 = main raw ⊕ ingress intents；菜单勾选态、
  行内按钮态、行 suppression 全读 ingress。
- **入口 1**：`ConversationHeaderTitle.tsx` dropdown，紧邻 "Pin/Unpin
  conversation" 增加 "Favorite conversation" / "Unfavorite conversation"，
  图标 **`lucide-react` 的 `Star` / `StarOff`**（不自造 SVG）。不设快捷键。
- **入口 2**：Favorites tab 行内取消收藏。落点：**扩展共享 `ThreadRailRow`
  行 action 契约为可组合 accessory 列表**（`ThreadConversationSidebar.tsx` 现
  declared Archive-only 结构升级），Favorites tab 行传 `[Unfavorite, Archive]`
  （悬浮显示，StarOff + 既有 Archive），其它 tab 不变。不局部叠按钮、不 fork
  行组件。
- **筛选 tab**：`app-shell/recent-thread-feeds.ts` 扩展
  `RecentThreadFilter = "all" | "nonTask" | "favorites"`（内部枚举沿现状，wire
  走 §5.1 联合），第三条独立 feed，label `"Favorites"`；`useRecentThreadFeeds`、
  `RecentConversationSidebar`（第三 tab + tabRefs + 方向键）、
  `recent-conversation-sidebar-model`（空态 "No favorite threads"）。
- **Favorites feed**：replacement-refresh + tombstone 语义按 §7.3/7.4 实现
  （desktop 侧同样受 round3-4 约束：`recent-thread-feeds.ts:232` 现无
  `isRefreshingHead` 阻断 load-more，replacement 必须显式关 gate）。
- i18n：增 "Favorites"、"Favorite conversation"、"Unfavorite conversation"、
  空态文案。

## 6. iOS

### 6.1 GaryxMobileCore

- **`GaryxFavoritesState`**：实现 §7.2 状态机（纯逻辑、SwiftPM 可测）。机制
  对齐 `GaryxPinnedOrderState` 的两阶段（flight 结算与 raw-page 接受分离，
  :227/:290），但意图模型按 §7.2 的 flight/desired 双身份。gateway 切换全清。
- `GaryxRecentThreadFilter`：新 case `.favorites`；`displayName` /
  `activeStatusLabel = "Favorites"`；`homeMenuOptions` 加入（过滤器菜单自动
  带出）；wire 映射改结构化 query 描述（`tasks=…` 与 `favorites=only` 二选一），
  纯函数 + 穷尽测试。
- `GaryxRecentThreadFeeds`：第三条 feed，replacement-refresh + tombstone 按
  §7.3/7.4。**不改 `GaryxHomeThreadListPager` 的既有语义**（refresh/load-more
  可并发是 pager 契约，PagerTests:240 钉死；隔离在 Favorites feed 层做——
  replacement 开始即废弃两 track + 关 gate，见 §7.4）。不复用现有 `reset()`
  （清 IDs + `isPrimed=false` 会配合 10s 静默刷新周期性 skeleton）。
- `GaryxRecentThreadFilterStorage`：持久化新 case。
- `GaryxGatewayClient`：`listThreadFavorites()`、`setThreadFavorited(id:_:)`；
  `listRecentThreads` 支持 favorites 参数。
- `GaryxGatewayThreadModels`：`GaryxThreadFavoritesPage`（兼容解码 + revision）。
- `HomeProjectionReducer`/`Actor`：`favoritesChanged(favoritedThreadIds:revision:)`；
  `GaryxHomeThreadListPresentation` 传收藏集合供长按菜单判态。

### 6.2 App 层

- **入口 1（长按）**：`GaryxMobileSidebarViews.swift` `.garyxThreadActionMenu`
  紧邻 Pin 项加 "Favorite thread"/"Unfavorite thread"（`star`/`star.slash`）。
- **入口 2（线程内右上角）**：`GaryxMobileConversationViews.swift` title 菜单
  （:942 附近）紧邻 Pin 项加同项。
- **入口 3（过滤器）**：`GaryxRecentThreadFilterMenu` 自动出现 Favorites。
- `GaryxMobileModel+ThreadPersistence.swift`：`isThreadFavorited`/
  `toggleFavoriteThread` = IO 编排薄层，裁决全委托 `GaryxFavoritesState`。
- `GaryxMobileModel+ThreadList.swift`：`refreshThreads` 并行组增拉
  `listThreadFavorites`；**All feed 辅助刷新改穷尽 switch**（`.all` 只刷 All；
  其它一切 case 刷自身 + All），扩展 RefreshCommitTests（:523 附近）。
- **Gateway 切换清理**：`GaryxMobileModel+Gateway.swift`（:55 附近）同步清
  `GaryxFavoritesState`（raw + intents + revision 水位）与 Favorites feed。
- 新 Core 文件跑 `xcodegen generate` 并提交 pbxproj，验证走 `xcodebuild`。

## 7. 客户端收藏状态机规格（双端共同契约）

本节是 round3-1/2/3/4 的完整答案，iOS `GaryxFavoritesState` 与 desktop
`favorites-ingress` 都按此实现（各自平台惯用形态，语义逐条一致，测试逐条对照）。

### 7.1 全局状态

- `raw`: 最近一次**按 revision 单调接受**的服务端整页 membership
  （`Set<threadId>` + `revision`）。来源：GET 周期拉取、写响应回传页。
  接受条件 `page.revision >= highestObservedRevision`。
- per-thread：`inFlight?` 与 `latestDesired?`（见 7.2）。
- 呈现值 `presented(id) = latestDesired[id]?.desired ?? raw.contains(id)`。

### 7.2 每线程意图状态机（对 round3-1）

**两个独立身份，互不覆盖：**

```
inFlight      = { requestToken, target: bool, flightGeneration }   // 当前唯一在途请求
latestDesired = { generation, desired: bool }                      // 最新用户意图
```

- **toggle(id, desired)**：`generation += 1`；`latestDesired = {generation,
  desired}`；若无 `inFlight` → dispatch。**若有 inFlight：只更新
  latestDesired，绝不动 inFlight**——新意图不取消、不覆盖、不使当前 flight 的
  响应失去结算资格。
- **dispatch(id)**：`inFlight = { 新 requestToken, target: latestDesired.desired,
  flightGeneration: latestDesired.generation }`；发 PUT/DELETE。
  per-thread single-flight：同 ID 任意时刻至多一个 inFlight。
- **响应处理顺序（写死，四步）**：
  1. **结算 flight**：按 `requestToken` 匹配 `inFlight`（token 只用于识别本
     flight，如 gateway 切换清场后的孤儿响应；匹配即结算，与 latestDesired
     新旧无关）；
  2. **接受 raw**：成功响应携带的整页按 revision 单调接受，更新 `raw`；
  3. **对比意图**：
     - 成功：若 `latestDesired.generation <= flightGeneration` → 意图已被本
       flight 完整服务，退休 `latestDesired`；若 `generation > flightGeneration`
       且 `latestDesired.desired != raw.contains(id)` → 进入第 4 步；若
       `generation > flightGeneration` 但 desired 与 raw 一致 → 直接退休（无需
       续发）。
     - 失败：若 `latestDesired.generation == flightGeneration` → 退休该意图
       （呈现回落 raw）；若 `generation > flightGeneration` → **不退休**（flight
       期间产生的更新意图有权获得自己的请求）→ 第 4 步。
  4. **drain**：清 `inFlight` 后若存在未退休的 `latestDesired` 且与 raw 不一致
     → dispatch 续发。
- 该规格下 round3-1 反例不可达：PUT(token=1, gen=1) 在途、Unfavorite 置
  `latestDesired={gen=2, false}`；PUT 返回**照常结算**（raw 接受为 {A}）；
  gen 2 > 1 且 desired=false != raw(A)=true → 续发 DELETE；终态 false =
  最后意图。✅
- round2-3 原反例（DELETE 先落库、旧 PUT 后落）依旧被 single-flight 排除。

### 7.3 Favorites feed 的 unfavorite tombstone（对 round3-3）

feed 行隐藏集合由意图状态机投影，分两级：

- **hidden-pending**：`latestDesired.desired == false` 且未确认（乐观期）。
  DELETE 失败且意图退休 → 解除隐藏（行重现，正确）。
- **hidden-tombstone**：DELETE 成功结算后转入。**退休条件唯一**：一个
  **在 DELETE settle 之后签发的 replacement ticket** 成功完成，tombstone 与
  整页替换**原子**退休（新页本就不含该行）。replacement 失败 → tombstone
  存续，行继续隐藏（即使显示缓存被保留）。
- **重新收藏打断**：tombstone 期间用户再 Favorite → 新 `latestDesired
  {desired: true}` 使 `presented(id)=true`，行解除隐藏；PUT 成功后照常走
  replacement。
- 隐藏是**后置过滤**：对已加载缓存行与任何 in-flight 响应行一律生效——
  这同时继续挡住 round1-3 的 load-more 复活时序。

### 7.4 replacement-refresh 与 load-more 隔离（对 round3-4）

Favorites feed 专用刷新协议（All/Chats feed 保持现有 pager 语义完全不动）：

- **开始 replacement**（触发源：写确认、周期刷新、下拉、切入 tab）：
  1. 签发新 replacement ticket（单调递增）；
  2. **同时废弃两个 track**：epoch bump 使 in-flight refresh 与 load-more 响应
     全部失效（含 desktop `recent-thread-feeds.ts:232` 无 `isRefreshingHead`
     阻断的问题——favorites feed 显式管理）；
  3. 清 active flags，**关闭分页 gate**（cursor 冻结，settle 前拒绝新
     load-more）；
  4. **保留 display-only IDs**（可见缓存不清，无 skeleton；tombstone 后置过滤
     持续生效）。
- **replacement 成功**：整页**原子替换**（丢弃旧尾部——跨端取消/尾部变化由此
  完全收敛）；cursor 重置到新 head 窗口；开 gate；按 §7.3 退休满足条件的
  tombstone。
- **replacement 失败**：保留显示缓存；以**旧 cursor** 重开 gate（stale-but-
  consistent：缓存与 cursor 同代，tombstone 继续过滤）；tombstone 不退休。
- 两种完成顺序都无害：旧 load-more 响应晚于 replacement 到达 → epoch 不匹配被
  弃；replacement 晚于旧 load-more → 旧响应已按旧 epoch 判弃或先行提交后被整页
  替换覆盖。**不存在「旧 offset 页追加到新 head」路径**。
- 收藏集合小（个位~数十），整页替换成本可忽略；10s 静默刷新期间用户可见的只有
  「行集原子变化」，无清空中间态。

## 8. 测试计划

**Gateway（`cargo test -p garyx-gateway --lib`）**

- meta：建库即有 `(1,0)`；重开幂等不重置 revision；初始化早于启动 purge。
- db：favorite/unfavorite 幂等；revision 仅实际变更 bump；三清理点同事务清行 +
  bump；守卫插入交错测试（归档先提交 → 插入空、无幽灵行）；**同快照组页交错
  测试**（并发写下 GET 不产生 `{旧集合, 新 revision}`，对齐 mod.rs:5462）；
  `favorites=only` 过滤域内 total/has_more；page+count 一次读快照。
- routes：三端点契约（NotFound→404 / 幂等 / 整页+revision）；`favorites` 非法
  值 400；`favorites=only`+显式 `tasks` 400；`tasks=only` 回归保持。

**双端状态机（desktop `npm run test:unit` / iOS SwiftPM，逐条对照 §7）**

- **同 ID 覆盖 + 旧 flight 成功**（round3-1 主反例）：PUT 在途翻转为 false →
  PUT 响应照常结算、raw 接受 → 续发 DELETE → 终态 false。
- **同 ID 覆盖 + 旧 flight 失败**：flight 期间的新意图不被失败连坐清除，获得
  自己的请求。
- 同 ID 意图与 flight 同代成功/失败的退休路径；drain 后 desired==raw 不空发。
- 不同 ID 部分失败：A 失败只回落 A，B 不受影响，无幽灵乐观态。
- 远端更新反向态：更高 revision 快照推翻本地已确认态；乱序旧快照被单调规则弃。
- **desktop raw 纯度契约**（round3-2）：写请求 in-flight 期间 main 发布的
  `favoritedThreadIds` 不变（无 pending 混入）；renderer 呈现 = raw ⊕ intents。
- gateway 切换全清（raw、intents、revision 水位、feed）。

**Favorites feed（双端）**

- **DELETE 成功 → replacement 失败 → 行仍隐藏**（round3-3 主反例）；后续
  replacement 成功后 tombstone 原子退休。
- DELETE 失败 → 行重现；tombstone 期间重新收藏 → 行立即重现、PUT 后收敛。
- **replacement vs load-more 两种完成顺序**（round3-4）：replacement 先成功、
  旧 load-more 响应后到被弃；旧 load-more 先提交、replacement 后整页覆盖。
  replacement 在途时新 load-more 被拒；失败后旧 cursor 恢复分页。
- replacement 期间可见缓存保留（10s 静默刷新无 skeleton）；失败保缓存。
- 乐观取消期间 in-flight load-more 返回该行不显示（后置过滤）。

**其余（沿用 v3）**

- desktop：三 tab 渲染、方向键、空态、Favorites 行 accessory =
  Unfavorite+Archive 并存（其它 tab 仅 Archive）；判别联合 wire 映射穷尽；
  store 按 gateway key 隔离。
- iOS：FilterStorage 往返；GatewayClient 三端点 + favorites query；
  Reducer/Actor/Presentation favoritesChanged 与长按菜单态；
  RefreshCommitTests `.favorites` 时 All feed 仍刷新。
- 端到端：本地 gateway curl 三端点 + `favorites=only`；双端 UI 按
  `garyx-product-ui` 走查两处入口 + 筛选切换 + 行内取消。

## 9. 实现切分建议

单 PR 可完成，按依赖顺序提交：① gateway（表 + meta + revision + API + 过滤 +
三清理点）→ ② 双端共享状态机规格落地（iOS Core `GaryxFavoritesState` /
desktop `favorites-ingress`，先测后 UI）→ ③ desktop renderer / iOS App UI +
xcodegen。每步各自带测试。
