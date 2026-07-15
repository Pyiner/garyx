# Thread Favorites (线程收藏)

Status: draft v5 (addressing review round 4 — #TASK-2324 FAIL, 3 findings)
Date: 2026-07-16

## 0. 修订记录

### Round 4 findings → v5 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | 旧 flight **模糊失败**（超时/截断/2xx 解码失败，服务端可能已应用）后，更新代意图被 `desired != raw` 的 drain 守卫用**可能过时的 raw** 消掉，永远得不到自己的请求 | 失败结算分流：存在更新代意图时置 **`mustWrite`**，drain **无条件**为其发一次幂等写（PUT/DELETE 均幂等，安全消歧）；`desired != raw` 守卫只允许出现在**成功结算**路径（那里的 raw 来自本 flight 响应，新鲜、无歧义）。同代模糊失败 → 退休意图、呈现回落 raw，由周期 GET（revision 单调）最终对齐服务端实际终态。GET raw 接受**永不退休意图**（§7.2） |
| 2 | flight 身份只有 `requestToken`，gateway 切换清场后 token 可复用，旧网关孤儿响应可错误结算新 flight、其页被误接受为 raw | flight 身份升级为复合围栏 **`(gatewayScope, runtimeEpoch, requestToken)`**，结算要求三元组全匹配；切网关 bump `runtimeEpoch` + 全清，孤儿响应连同其页整体判弃（对齐 pin 现码：iOS stamp 含 gatewayIdentity+epoch GaryxPinnedOrderState.swift:13、App runtime UUID 围栏 ThreadPersistence.swift:92、desktop gateway stamp pinned-order-state.ts:199）；desktop IPC 响应携带派发时的 scope stamp，回程校验（§7.1、§5.1）；补「旧/新网关同 token」交错测试 |
| 3 | replacement 失败后以旧 cursor 重开 load-more：连续确认取消超过 overlap(5) 时 offset 漂移永久跳行，tombstone 只防复活不修 offset | 规则改为：**自上次成功 replacement 以来存在已确认 membership 变更（tombstone/新增）时，replacement 失败后分页 gate 保持关闭**，直到一次 replacement 成功（周期刷新会持续重试；缓存行仍可见）；无已确认变更时才允许旧 cursor 重开（此时无漂移源）。不做 cursor delta 精确修正（过度工程）。补「>overlap 连续取消 + replacement 全失败 + load-more 被拒」测试（§7.4） |

### 历史轮次

- **Round 3 → v4**：flight/desired 双身份分离（round4 确认修掉「旧 PUT 响应因
  token 被覆盖无法结算」主反例）；main raw / renderer intent 单一裁决成立；
  成功 replacement 原子换页 + 旧 track epoch 作废方向成立。
- **Round 2 → v3**（round 3 裁决 CONFIRMED）：meta singleton 初始化（早于
  purge、非负 CHECK、重开幂等）；同快照组页不变量；`FavoriteThreadResult::
  {Updated|NotFound}` + 404 映射；共享行 accessory 契约；FK 理由修正与清理点
  行号（归档 mod.rs:669、通用删除 mod.rs:2045、启动 purge mod.rs:2785）。
- **Round 1 → v2**：守卫式单事务插入封死幽灵行（单 writer mutex mod.rs:363）；
  suppression 挡住 load-more 复活；All feed 穷尽 switch；gateway 切换清理；
  判别联合；Mac 行内取消产品裁决。

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
- 跨端收敛：客户端统一收藏意图状态机（§7）+ revision 单调接受。

**非目标**

- 收藏排序 / 拖拽重排（不引入 sort_order、reorder CAS、reorder outbox；
  revision 仍需要，用于快照单调接受）。
- 首页独立「收藏段」；All/Chats 行星标徽标（Favorites tab 行内取消按钮除外）。
- SSE 推送收藏变更（沿用刷新收敛）。
- bot 命令面不加收藏筛选。
- 收藏意图跨进程/重启持久化（意图是瞬态 UI 状态，进程退出回落 raw，可接受）。

## 3. 数据模型（gateway SQLite）——round 2/3/4 已 CONFIRMED，未改动

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
  （`INSERT (1,0) ON CONFLICT(id) DO NOTHING`，镜像 mod.rs:3529），**早于
  retired-workflow 启动清理**（pin 先例 mod.rs:2669）。重开幂等，不重置 revision。
- `favorites_revision` 全局单调；任何集合变更（含三清理点）同事务 bump
  （镜像 mod.rs:2924，仅实际变更时 bump）。不用于写侧 CAS。
- **守卫式单事务插入**（单 writer mutex mod.rs:363 + SQLite writer 串行化，
  幽灵行时序两个方向都封死）：

```sql
INSERT INTO thread_favorites (thread_id, favorited_at)
SELECT ?1, ?2
WHERE EXISTS (SELECT 1 FROM thread_records WHERE key = ?1)
ON CONFLICT(thread_id) DO NOTHING;
```

  守卫 0 行且记录不存在 → `FavoriteThreadResult::NotFound`。
- **不用 FK 级联**：级联删除不经 bump 路径破坏 revision 单调契约（显式删除 +
  同事务 bump 必须存在）；与 `thread_pins` 无 FK 模式一致。
- **生命周期清理点，精确三处**（同事务删行 + bump）：归档事务（mod.rs:669
  附近）、通用线程删除（mod.rs:2045 附近）、retired-workflow 启动清理
  （删除语句 mod.rs:2785，入口 mod.rs:2698）。实现 PR 附三处 diff 对照表。
- 契约合规：条件查询全 SQL（JOIN），无 `list_keys`/记录体扫描。

## 4. Gateway API——round 2/3/4 已 CONFIRMED，未改动

### 4.1 收藏读写

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids, favorites: [{thread_id, favorited_at}], revision }`，按 `favorited_at DESC, thread_id ASC` |
| `PUT /api/thread-favorites/{key}` | 200 `{ favorited: true, thread_ids, favorites, revision }`；线程不存在 404 `{ favorited: false, error }`；幂等 |
| `DELETE /api/thread-favorites/{key}` | 200 `{ favorited: false, removed, thread_id, thread_ids, favorites, revision }`；幂等 |

- **同快照组页不变量**：GET 单一读事务读 `{行集, revision}`（对齐 mod.rs:538）；
  PUT/DELETE 写事务内变更 + bump 后**同一事务**读回整页。配交错测试
  （对齐 mod.rs:5462 模式）。
- **NotFound**：db 层 `FavoriteThreadResult::{Updated(ThreadFavoritesPage) |
  NotFound}`；route 显式映射 404。route 前置 `ensure_existing_thread_id` =
  规范 thread key 校验（trim + `is_thread_key`）+ 存在 point-check 的友好 404
  短路（routes.rs:1104），非别名解析；最终裁决在 db 事务。
- 路由注册 `route_graph.rs`，handler 在 `routes.rs` 紧邻 thread-pins。

### 4.2 `/api/recent-threads` favorites 过滤

- 新可选参数 `favorites`，唯一合法值 `"only"`，其它值 400；与显式 `tasks`
  同传 400。映射：All → `tasks=include`；Chats → `tasks=exclude`；Favorites →
  `favorites=only`。既有 `tasks=only` 行为与回归（routes/tests.rs:3896）不动。
- **Favorites 含 task 线程**。
- SQL：`recent_threads r JOIN thread_favorites f ON f.thread_id = r.thread_id
  ORDER BY r.last_active_at DESC, r.thread_id ASC LIMIT ? OFFSET ?`；count 同
  过滤域；page+count 一次读快照。
- 返回体不变；不加 per-row favorited 字段。

### 4.3 已知边界

- automation generated / hidden 线程不进投影 → 收藏了也不出现在 Favorites tab
  （本就不出现在首页，一致，接受）。
- 收藏的线程被归档：归档事务清行 + bump，与 pin 一致。

## 5. Desktop（Electron）

### 5.1 契约与主进程——raw 唯一所有权

裁决：**favorites 乐观状态机唯一 owner = renderer ingress**；main 对 favorites
不做任何 pending 投影（与 pin 的 main 侧 `PinnedOrderController` 有意分道：
pin 的 main controller 为 reorder outbox 持久化而设，收藏无此需求，双层投影
只会制造 raw 污染）。

- main 发布的 `favoritedThreadIds` + `favoritesRevision` **定义为纯 raw**：仅
  来自服务端页（周期 `fetchThreadFavorites` + 写响应回传页），main 侧接受同为
  revision 单调。**写请求 in-flight 期间 main 发布值不变**（契约测试钉死）。
- **按 `entitiesGatewayUrl` 归一化 key 存取**（对齐 store.ts:498）。
- **IPC 响应带 scope stamp（对 round4-2）**：`setThreadFavorited` 与
  `fetchThreadFavorites` 的请求/响应携带派发时的 gateway scope stamp
  （对齐 desktop pin 的 gateway stamp 模式，pinned-order-state.ts:199）；
  renderer 结算按 §7.1 复合围栏校验，scope 不匹配的迟到响应连同其页判弃。
- `src/shared/contracts/thread.ts`：判别联合
  `RecentThreadListFilter = "all" | "chats" | "favorites"`；wire 映射收敛在
  main 层纯函数，类型排除 `tasks`+`favorites` 并存。新增
  `DesktopThreadFavoritesPage`（`thread_ids` + `revision`）。
- `src/main/garyx-client/threads.ts`：`fetchThreadFavorites()`、
  `setRemoteThreadFavorited(threadId, favorited)`；`fetchRecentThreads` 按联合
  拼参；`validateListRecentThreadsInput` 校验。
- IPC `setThreadFavorited` = 纯转发（HTTP → `{page, scopeStamp}` 或错误；main
  仅按 revision 单调更新 raw 缓存）。

### 5.2 Renderer

- **favorites-ingress（唯一乐观层）**：实现 §7 状态机。呈现值 = main raw ⊕
  ingress intents；菜单勾选态、行内按钮态、行 suppression 全读 ingress。
- **入口 1**：`ConversationHeaderTitle.tsx` dropdown 紧邻 "Pin/Unpin
  conversation" 增加 "Favorite conversation" / "Unfavorite conversation"，
  图标 `lucide-react` `Star`/`StarOff`。不设快捷键。
- **入口 2**：Favorites tab 行内取消收藏。**扩展共享 `ThreadRailRow` 行 action
  契约为可组合 accessory 列表**（`ThreadConversationSidebar.tsx` Archive-only
  结构升级），Favorites tab 行传 `[Unfavorite, Archive]`，其它 tab 不变。
- **筛选 tab**：`recent-thread-feeds.ts` 扩展
  `RecentThreadFilter = "all" | "nonTask" | "favorites"`（wire 走 §5.1 联合），
  第三条独立 feed，label `"Favorites"`；`useRecentThreadFeeds`、
  `RecentConversationSidebar`（第三 tab + 方向键）、sidebar-model（空态
  "No favorite threads"）。
- **Favorites feed**：replacement-refresh + tombstone + 分页 gate 按 §7.3/7.4
  （desktop `recent-thread-feeds.ts:232` 现无 `isRefreshingHead` 阻断，
  favorites feed 显式管理）。
- i18n：增 "Favorites"、"Favorite conversation"、"Unfavorite conversation"、
  空态文案。

## 6. iOS

### 6.1 GaryxMobileCore

- **`GaryxFavoritesState`**：实现 §7 状态机（纯逻辑、SwiftPM 可测）。围栏与
  两阶段机制对齐 `GaryxPinnedOrderState`（stamp 含 gatewayIdentity+epoch :13；
  flight 结算与 raw 接受分离 :227/:290）。gateway 切换全清 + epoch bump。
- `GaryxRecentThreadFilter`：新 case `.favorites`；`displayName`/
  `activeStatusLabel = "Favorites"`；`homeMenuOptions` 加入；wire 映射改结构化
  query 描述（`tasks=…` 与 `favorites=only` 二选一），纯函数 + 穷尽测试。
- `GaryxRecentThreadFeeds`：第三条 feed，replacement-refresh/tombstone/gate 按
  §7.3/7.4。**不改 `GaryxHomeThreadListPager` 既有语义**（refresh/load-more
  并发契约 PagerTests:240 不动；隔离在 Favorites feed 层）。不复用现有
  `reset()`（清 IDs + `isPrimed=false` 会周期性 skeleton）。
- `GaryxRecentThreadFilterStorage`：持久化新 case。
- `GaryxGatewayClient`：`listThreadFavorites()`、`setThreadFavorited(id:_:)`；
  `listRecentThreads` 支持 favorites 参数。**注意模糊失败面**：超时/截断
  （:1025/:1061）与 2xx 解码失败（:997）都会以 error 抛出——§7.2 的 mustWrite
  消歧正是为此设计。
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
  `toggleFavoriteThread` = IO 编排薄层，裁决全委托 `GaryxFavoritesState`；
  runtime UUID 围栏对齐现有 pin 模式（:92）。
- `GaryxMobileModel+ThreadList.swift`：`refreshThreads` 并行组增拉
  `listThreadFavorites`；**All feed 辅助刷新改穷尽 switch**（`.all` 只刷 All；
  其它一切 case 刷自身 + All），扩展 RefreshCommitTests（:523 附近）。
- **Gateway 切换清理**：`GaryxMobileModel+Gateway.swift`（:55 附近）清
  `GaryxFavoritesState`（raw、intents、revision 水位、epoch bump）与
  Favorites feed。
- 新 Core 文件跑 `xcodegen generate` 并提交 pbxproj，验证走 `xcodebuild`。

## 7. 客户端收藏状态机规格（双端共同契约）

iOS `GaryxFavoritesState` 与 desktop `favorites-ingress` 按此实现（平台惯用
形态，语义逐条一致，测试逐条对照）。

### 7.1 全局状态与身份围栏

- `raw`: 最近一次按 revision 单调接受的服务端整页 membership
  （`Set<threadId>` + `revision`）。来源：GET 周期拉取、写响应回传页。
  接受条件 `page.revision >= highestObservedRevision` **且 scope 围栏匹配**。
- **flight 身份 = `(gatewayScope, runtimeEpoch, requestToken)` 三元组
  （对 round4-2）**：`gatewayScope` = 归一化网关标识；`runtimeEpoch` 在每次
  gateway 切换 / 状态清场时单调 bump；`requestToken` 进程内单调递增、
  **allocator 永不随清场重置**。结算与 raw 接受都要求三元组全匹配——旧网关的
  迟到孤儿响应（含其页）整体判弃。对齐 pin 现码的复合 stamp 模式。
- per-thread：`inFlight?` 与 `latestDesired?`（§7.2）。
- 呈现值 `presented(id) = latestDesired[id]?.desired ?? raw.contains(id)`。
- **GET raw 接受永不退休意图**：意图只经 flight 结算退休（§7.2）；raw 更新只
  改变 fallback 层。

### 7.2 每线程意图状态机（对 round3-1 + round4-1）

```
inFlight      = { requestToken, target: bool, flightGeneration }   // 当前唯一在途请求（身份含 §7.1 围栏）
latestDesired = { generation, desired: bool, mustWrite: bool }     // 最新用户意图
```

- **toggle(id, desired)**：`generation += 1`；`latestDesired = {generation,
  desired, mustWrite: false}`；无 `inFlight` → 立即 dispatch。有 inFlight：
  只更新 latestDesired，**绝不动 inFlight**、不使其失去结算资格。
- **dispatch(id)**：`inFlight = {新 token, target: latestDesired.desired,
  flightGeneration: latestDesired.generation}`；发幂等 PUT/DELETE。
  per-thread single-flight：同 ID 至多一个 inFlight。
- **响应处理四步（写死）**：
  1. **结算 flight**：按 §7.1 三元组匹配；匹配即结算，与 latestDesired 新旧
     无关；围栏不匹配 → 整体判弃（含页）。
  2. **接受 raw**：成功响应的整页按 revision 单调接受。
  3. **意图裁决**：
     - **成功结算**（raw 来自本 flight 响应，新鲜无歧义）：
       `latestDesired.generation <= flightGeneration` → 已被完整服务，退休；
       `generation > flightGeneration` 且 `desired != raw.contains(id)` →
       保留，进第 4 步；`generation > flightGeneration` 且 desired == raw →
       退休（**仅成功路径允许用 raw 消意图**）。
     - **失败结算**（含模糊失败：超时/截断/2xx 解码失败——服务端可能已应用，
       本地 raw 不可信）：
       `generation == flightGeneration` → 退休该意图，呈现回落 raw；服务端
       实际终态由周期 GET（revision 单调）最终对齐——若写实际已落库，revision
       已 bump，下次接受的 raw 会带出真实状态。
       `generation > flightGeneration` → **置 `mustWrite = true`，保留**，进
       第 4 步。
  4. **drain**：清 `inFlight` 后若存在未退休 `latestDesired`：
     `mustWrite == true` → **无条件 dispatch**（幂等写是确定性消歧，不比较
     raw）；`mustWrite == false` → 按 `desired != raw.contains(id)` 决定
     dispatch 或退休。
- **round4-1 反例走查**：raw=false，PUT(gen1,true) 在途；用户改 gen2=false；
  PUT 实际落库（server=true）但响应超时 → 失败结算，gen2 > gen1 → mustWrite
  → drain **无条件**发 DELETE → server=false = 最后意图。✅（旧规格在此因
  `desired(false)==raw(false)` 而不发，被 REFUTED。）
- round3-1 反例（token 覆盖致响应无法结算）依旧不可达：双身份分离。
- round2-3 反例（DELETE 先落库、旧 PUT 后落）依旧被 single-flight 排除。

### 7.3 Favorites feed 的 unfavorite tombstone（round3-3 处置，round4 未驳回）

- **hidden-pending**：`latestDesired.desired == false` 未确认（乐观期）。
  DELETE 失败且意图退休 → 解除隐藏。
- **hidden-tombstone**：DELETE 成功结算后转入。**唯一退休条件**：一个在
  DELETE settle 之后签发的 replacement ticket 成功完成，与整页替换**原子**
  退休。replacement 失败 → tombstone 存续，行保持隐藏。
- **重新收藏打断**：tombstone 期间再 Favorite → `presented(id)=true` 行重现；
  PUT 成功后照常 replacement。
- 隐藏是**后置过滤**：对缓存行与任何 in-flight 响应行一律生效。

### 7.4 replacement-refresh 与分页 gate（对 round3-4 + round4-3）

Favorites feed 专用协议（All/Chats feed 现有 pager 语义完全不动）：

- **开始 replacement**（写确认、周期刷新、下拉、切入 tab）：签发单调
  replacement ticket；epoch bump 废弃 in-flight refresh/load-more 两 track；
  清 active flags；**关闭分页 gate**；保留 display-only IDs（无 skeleton，
  tombstone 后置过滤持续生效）。
- **成功**：整页原子替换（丢旧尾部，跨端变化完全收敛）；cursor 重置到新 head；
  开 gate；退休满足条件的 tombstone；清「未收敛变更」标记（下条）。
- **失败（对 round4-3）**：保留显示缓存；tombstone 不退休；gate 是否重开取决于
  **未收敛变更标记**：
  - 维护 `confirmedMutationsSinceLastSuccessfulReplacement`（自上次成功
    replacement 以来本端已确认的 membership 变更：确认的 unfavorite tombstone
    或确认的新收藏）；
  - 标记**非空** → **gate 保持关闭**直到某次 replacement 成功（周期 10s 刷新
    持续重试；用户仍可见缓存行，只是不能 load-more）——杜绝「旧 cursor +
    服务端已删 N>overlap 行」的 offset 漂移跳行；
  - 标记**为空** → 允许以旧 cursor 重开 gate（无本端漂移源；他端变更引起的
    漂移与现有 All/Chats feed 的既有容忍度一致，由下次成功 replacement 收敛）。
- 两种完成顺序无害：旧 load-more 响应晚到 → epoch 判弃；replacement 晚于旧
  load-more → 旧响应已按旧 epoch 判弃或被整页替换覆盖。无「旧 offset 页追加到
  新 head」路径。
- 收藏集合小（个位~数十），整页替换成本可忽略；10s 静默刷新期间用户可见的只有
  行集原子变化，无清空中间态。

## 8. 测试计划

**Gateway（`cargo test -p garyx-gateway --lib`）**

- meta：建库即有 `(1,0)`；重开幂等不重置 revision；初始化早于启动 purge。
- db：favorite/unfavorite 幂等；revision 仅实际变更 bump；三清理点同事务清行 +
  bump；守卫插入交错测试（归档先提交 → 插入空）；同快照组页交错测试（并发写下
  GET 不产生 `{旧集合, 新 revision}`，对齐 mod.rs:5462）；`favorites=only`
  过滤域内 total/has_more；page+count 一次读快照。
- routes：三端点契约（NotFound→404 / 幂等 / 整页+revision）；`favorites` 非法
  值 400；`favorites=only`+显式 `tasks` 400；`tasks=only` 回归保持。

**双端状态机（desktop `npm run test:unit` / iOS SwiftPM，逐条对照 §7）**

- 同 ID 覆盖 + 旧 flight 成功：响应照常结算、raw 接受、续发反向写、终态 =
  最后意图。
- **同 ID 覆盖 + 旧 flight 模糊失败（round4-1 主反例）**：gen2 置 mustWrite，
  drain 无条件发幂等写（即使 `desired == raw`），终态 = 最后意图。
- 同代成功/失败退休路径；成功路径 `desired == raw` 退休不空发；mustWrite 只在
  失败+更新代路径置位。
- 同代模糊失败：意图退休、呈现回落 raw、后续周期 GET（更高 revision）对齐
  服务端实际终态。
- 不同 ID 部分失败：A 失败只回落 A，B 不受影响。
- **网关切换围栏（round4-2）**：旧网关 token=N 在途 → 切换清场 + epoch bump →
  新网关同值 token 的 flight 不被旧孤儿响应结算，旧页不被接受为 raw；token
  allocator 不随清场重置的不变量。
- 远端更新反向态：更高 revision 快照推翻本地已确认态；乱序旧快照弃。
- desktop raw 纯度契约：写 in-flight 期间 main 发布值不变。
- gateway 切换全清（raw、intents、revision 水位、feed、epoch bump）。

**Favorites feed（双端）**

- DELETE 成功 → replacement 失败 → 行仍隐藏；后续成功后 tombstone 原子退休。
- DELETE 失败 → 行重现；tombstone 期间重新收藏 → 行立即重现。
- replacement vs load-more 两种完成顺序；replacement 在途时新 load-more 被拒。
- **offset 漂移防护（round4-3 主反例）**：连续 >overlap(5) 次确认取消 + 每次
  replacement 均失败 → gate 保持关闭、load-more 被拒（不跳行）；一次
  replacement 成功后 gate 重开、行集收敛。
- 无未收敛变更时 replacement 失败 → 旧 cursor 重开 gate 可继续分页。
- replacement 期间可见缓存保留（10s 静默刷新无 skeleton）；失败保缓存。
- 乐观取消期间 in-flight load-more 返回该行不显示。

**其余**

- desktop：三 tab 渲染、方向键、空态、Favorites 行 accessory =
  Unfavorite+Archive 并存；判别联合 wire 映射穷尽；store 按 gateway key 隔离。
- iOS：FilterStorage 往返；GatewayClient 三端点 + favorites query；
  Reducer/Actor/Presentation；RefreshCommitTests `.favorites` 时 All feed 仍
  刷新。
- 端到端：本地 gateway curl 三端点 + `favorites=only`；双端 UI 按
  `garyx-product-ui` 走查两处入口 + 筛选切换 + 行内取消。

## 9. 实现切分建议

单 PR 可完成，按依赖顺序提交：① gateway（表 + meta + revision + API + 过滤 +
三清理点）→ ② 双端共享状态机（iOS Core `GaryxFavoritesState` / desktop
`favorites-ingress`，先测后 UI）→ ③ desktop renderer / iOS App UI + xcodegen。
每步各自带测试。
