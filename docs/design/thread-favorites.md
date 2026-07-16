# Thread Favorites (线程收藏)

Status: draft v7 (addressing review round 6 — #TASK-2324 FAIL, 3 findings)
Date: 2026-07-16

## 0. 修订记录

### Round 6 findings → v7 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | iOS transport 对普通 PUT/DELETE 默认 `idempotent: true` 最多三次自动重试（:950/:1025）：首 attempt 已提交、响应丢失、重试同 expected_revision 得 409/404——「本 flight 已应用」被伪装成确定性未应用，marker 漏记 | **CAS favorites 写与需副作用判定的生命周期写一律单 transport attempt**（iOS `maxAttempts: 1`，先例 = pins reorder :360；desktop main 侧同样禁止自动重试，契约测试计数 attempt）。重试所有权归状态机（conflict 重派 / mustWrite / drain）。单 attempt 下 409/404 才是真确定性未应用（§5.1、§6.1、§7.2） |
| 2 | 生命周期 marker 在结果处理时才建，晚于服务端可能提交时刻；iOS 指定接入点（Bots.swift:270 本地删行）是成功专用路径，模糊失败走 catch 根本不经过 | **marker 一律派发前建立**：派发前捕获 `wasInFavoritesDomain`、创建 operation-token marker、**立即关 gate**；确定性未应用（单 attempt 409/404/未发出）才撤销。接入点改为 **App/renderer 编排层**（请求派发处），成功/失败/模糊三类结局都从编排层喂给 Core 状态机，不挂在成功专用删行函数上（§5.2、§6.2、§7.4） |
| 3 | replacement 是独立 WAL read connection 上的只读快照，会越过仍在等 writer 的 pending blocking 写；「成功 replacement 清 marker」会在延迟提交前清掉标记、重开 gate → 永久漏行 | marker 引入**结算语义**，replacement 只能清「服务端命运已封」的条目：**CAS 条目**带 `expectedRevision E`，仅当成功 replacement 配对观察到 favorites revision `R_obs > E` 才清（恒 bump 保证：orphan 要么已提交且效果含于 R_obs 页、要么被永久围栏）；**生命周期条目**须先达确定性结局（成功/already-gone，幂等重试到确定）转 settled，其后签发的成功 replacement 才清；unsettled 条目永不被 replacement 清除。replacement 协议升级为**配对拉取**（favorites GET 提供 R_obs + recent-threads?favorites=only，两者同成功才算成功）（§7.4） |

round 6 已确认：全局 revision + 单 writer 构成正确 CAS 总序；no-op 恒 bump 封死
v5 两个提交序；200/409 页同事务回读同快照成立（pins 先例 mod.rs:598）。

### 历史轮次（要点）

- **Round 5 → v6**：服务端 CAS 写围栏（`expected_revision` 必填、接受即恒 bump
  含 no-op、409 + 同快照整页）；漂移标记扩容至模糊失败与 Archive/Delete。
- **Round 4 → v5**：mustWrite 幂等消歧；`(gatewayScope, runtimeEpoch,
  requestToken)` 围栏；未收敛变更时 gate 关闭。
- **Round 3 → v4**：flight/desired 双身份分离；main 只发布纯 raw；分级
  tombstone 原子退休；replacement 废弃两 track + 关 gate。
- **Round 2 → v3**：meta singleton 初始化（早于 purge）；同快照组页不变量；
  `FavoriteThreadResult` 枚举 + 404；共享行 accessory 契约；清理点三处
  （归档 mod.rs:669、通用删除 mod.rs:2045、启动 purge mod.rs:2785）。
- **Round 1 → v2**：守卫式单事务插入封幽灵行；suppression 挡 load-more 复活；
  All feed 穷尽 switch；gateway 切换清理；判别联合；Mac 行内取消。

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
- 双端筛选器各加 Favorites 类别；Favorites feed 用配对 replacement-refresh（§7.4）。
- 跨端收敛：客户端统一收藏意图状态机（§7）+ revision 单调接受 + 写侧 CAS 围栏。

**非目标**

- 收藏排序 / 拖拽重排（不引入 sort_order、reorder outbox）。
- 首页独立「收藏段」；All/Chats 行星标徽标（Favorites tab 行内取消按钮除外）。
- SSE 推送收藏变更（沿用刷新收敛）。
- bot 命令面不加收藏筛选。
- 收藏意图跨进程/重启持久化（瞬态 UI 状态，进程退出回落 raw；marker/gate 亦然
  ——重启后 feed 从头 prime，无旧 cursor 可漂移）。

## 3. 数据模型（gateway SQLite）——round 6 已确认，未改动

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
  （`INSERT (1,0) ON CONFLICT(id) DO NOTHING`，镜像 mod.rs:3529），早于
  retired-workflow 启动清理（pin 先例 mod.rs:2669）。重开幂等，不重置 revision。
- **revision 语义**：全局单调，兼任快照排序与写侧 CAS 围栏（先例 pins reorder
  mod.rs:590）。**条件写（PUT/DELETE）被接受即恒 bump**——含 membership 无变化
  的 no-op；生命周期清理点删除（非 CAS 写）维持仅实际变更时 bump（镜像
  mod.rs:2924）。
- **守卫式单事务插入**（单 writer mutex mod.rs:363 + writer 串行化）：CAS 校验、
  存在守卫、写入、bump、回读整页在同一写事务内：

```sql
INSERT INTO thread_favorites (thread_id, favorited_at)
SELECT ?1, ?2
WHERE EXISTS (SELECT 1 FROM thread_records WHERE key = ?1)
ON CONFLICT(thread_id) DO NOTHING;
```

  守卫 0 行且记录不存在 → `NotFound`。
- **不用 FK 级联**：级联删除不经 bump 路径破坏 revision 契约；与 `thread_pins`
  无 FK 模式一致。
- **生命周期清理点，精确三处**（同事务删行 + 变更时 bump）：归档事务
  （mod.rs:669 附近）、通用线程删除（mod.rs:2045 附近）、retired-workflow
  启动清理（删除语句 mod.rs:2785，入口 mod.rs:2698）。实现 PR 附对照表。
- 契约合规：条件查询全 SQL（JOIN），无 `list_keys`/记录体扫描。

## 4. Gateway API——round 6 已确认，未改动

### 4.1 收藏读写（写侧 CAS）

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids, favorites: [{thread_id, favorited_at}], revision }`，按 `favorited_at DESC, thread_id ASC` |
| `PUT /api/thread-favorites/{key}?expected_revision=N` | 接受：200 `{ favorited: true, thread_ids, favorites, revision }`（接受即 bump，含重复 PUT no-op）；失配：**409** `{ conflict: true, thread_ids, favorites, revision }`（当前整页，同快照）；线程不存在：404；参数缺失/非法：400 |
| `DELETE /api/thread-favorites/{key}?expected_revision=N` | 接受：200 `{ favorited: false, removed, thread_id, thread_ids, favorites, revision }`（接受即 bump，含 no-op）；失配 409 / 404 / 400 同上 |

- **`expected_revision` 必填**；围栏语义 = 同客户端 single-flight 序列的提交
  顺序保证；跨端并发写 last-writer-wins + 409 重派收敛。
- **同快照组页不变量**：GET 单一读事务；200 与 409 响应页均在同一事务内回读
  （对齐 mod.rs:538 / mod.rs:598；交错测试对齐 mod.rs:5462）。
- **NotFound**：db 层 `FavoriteThreadResult::{Updated(page) | Conflict(page) |
  NotFound}`；route 映射 200/409/404。route 前置 `ensure_existing_thread_id` =
  规范 key 校验 + 存在 point-check 的友好 404 短路（routes.rs:1104）。
- 路由注册 `route_graph.rs`，handler 在 `routes.rs` 紧邻 thread-pins。

### 4.2 `/api/recent-threads` favorites 过滤（round 2 起 CONFIRMED）

- 新可选参数 `favorites`，唯一合法值 `"only"`，其它值 400；与显式 `tasks`
  同传 400。映射：All → `tasks=include`；Chats → `tasks=exclude`；Favorites →
  `favorites=only`。既有 `tasks=only` 行为与回归（routes/tests.rs:3896）不动。
- **Favorites 含 task 线程**。
- SQL：`recent_threads r JOIN thread_favorites f ON f.thread_id = r.thread_id
  ORDER BY r.last_active_at DESC, r.thread_id ASC LIMIT ? OFFSET ?`；count 同
  过滤域；page+count 一次读快照。
- 返回体不变；不加 per-row favorited 字段。

### 4.3 已知边界

- automation generated / hidden 线程不进投影 → 收藏了也不出现在 Favorites tab。
- 收藏的线程被归档：归档事务清行 + bump，与 pin 一致。

## 5. Desktop（Electron）

### 5.1 契约与主进程

裁决：favorites 乐观状态机唯一 owner = renderer ingress；main 对 favorites 不做
任何 pending 投影（与 pin 的 main 侧 controller 有意分道：那是 reorder outbox
持久化需求，收藏没有）。

- main 发布 `favoritedThreadIds` + `favoritesRevision` = **纯 raw**（仅来自
  服务端页：周期 fetch + 写响应/409 响应页），revision 单调接受；写 in-flight
  期间 main 发布值不变（契约测试钉死）。按 `entitiesGatewayUrl` 归一化 key
  存取（对齐 store.ts:498）。
- **transport 单 attempt（对 round6-1）**：`setRemoteThreadFavorited` 的 HTTP
  调用**禁止任何自动重试**（单 attempt；重试所有权归 renderer 状态机）。凡
  desktop 侧 fetch 封装带重试语义的，此调用显式豁免；契约测试用计数 mock 钉死
  「一次 settle 至多一次网络 attempt」。
- IPC：`setThreadFavorited(threadId, favorited, expectedRevision)` 纯转发 →
  `{status: ok|conflict|notFound, page, scopeStamp}`；`fetchThreadFavorites`
  同样带 scope stamp。
- `src/shared/contracts/thread.ts`：判别联合
  `RecentThreadListFilter = "all" | "chats" | "favorites"`；wire 映射 main 层
  纯函数；类型排除 `tasks`+`favorites` 并存。新增 `DesktopThreadFavoritesPage`。
- `src/main/garyx-client/threads.ts`：`fetchThreadFavorites()`、
  `setRemoteThreadFavorited(...)`（ok/conflict/notFound 三分，对齐
  `reorderRemoteThreadPins` 形态）；`fetchRecentThreads` 按联合拼参。

### 5.2 Renderer

- **favorites-ingress（唯一乐观层）**：实现 §7 状态机。呈现值 = main raw ⊕
  ingress intents。
- **入口 1**：`ConversationHeaderTitle.tsx` dropdown 紧邻 Pin 项增加
  "Favorite conversation"/"Unfavorite conversation"（`lucide-react`
  `Star`/`StarOff`）。不设快捷键。
- **入口 2**：Favorites tab 行内取消收藏。扩展共享 `ThreadRailRow` 行 action
  契约为可组合 accessory 列表，Favorites tab 行传 `[Unfavorite, Archive]`，
  其它 tab 不变。
- **marker 接入（对 round6-2）**：CAS 写与生命周期写的 marker 都在**编排层
  派发前**建立（§7.4）：
  - favorite/unfavorite：ingress dispatch 处（建 CAS 条目携带
    expectedRevision）；
  - Archive/Delete：行 action / 菜单的请求派发处，派发前捕获
    `wasInFavoritesDomain`（presented-favorited 或在 favorites feed 缓存中）
    → 建 lifecycle 条目 + 关 gate；成功/失败/模糊三类结局都从编排层回喂
    （不挂在成功专用删行路径上）。
- **筛选 tab**：`recent-thread-feeds.ts` 扩展
  `RecentThreadFilter = "all" | "nonTask" | "favorites"`，第三条独立 feed，
  label `"Favorites"`；`useRecentThreadFeeds`、`RecentConversationSidebar`
  （第三 tab + 方向键）、sidebar-model（空态 "No favorite threads"）。
- **Favorites feed**：配对 replacement + tombstone + gate 按 §7.3/7.4
  （desktop `recent-thread-feeds.ts:232` 现无 `isRefreshingHead` 阻断，
  favorites feed 显式管理）。
- i18n：增 "Favorites"、"Favorite conversation"、"Unfavorite conversation"、
  空态文案。

## 6. iOS

### 6.1 GaryxMobileCore

- **`GaryxFavoritesState`**：实现 §7 状态机（纯逻辑、SwiftPM 可测）。围栏与
  两阶段机制对齐 `GaryxPinnedOrderState`（stamp 含 gatewayIdentity+epoch :13；
  结算与 raw 接受分离 :227/:290；conflict 对齐 reorder 409 路径）。gateway
  切换全清 + epoch bump。
- **transport 单 attempt（对 round6-1）**：`setThreadFavorited(id:favorited:
  expectedRevision:)` 必须 **`maxAttempts: 1`**（先例 = pins reorder :360；
  默认 `idempotent: true` 的三次自动重试 :950/:1025 对 CAS 写是错误语义——
  首 attempt 已提交、重试得 409/404 会伪装成确定性未应用）。契约测试用计数
  transport mock 钉死。**从 Favorites 域发起的 Archive/Delete 同理单 attempt**
  （通用线程 DELETE 首次已删、重试得 404 的同类误判），显式重试由编排层带新
  operation-token 发起。
- `GaryxRecentThreadFilter`：新 case `.favorites`；`displayName`/
  `activeStatusLabel = "Favorites"`；`homeMenuOptions` 加入；wire 映射结构化
  query 描述（二选一），纯函数 + 穷尽测试。
- `GaryxRecentThreadFeeds`：第三条 feed，配对 replacement/tombstone/gate 按
  §7.3/7.4。不改 `GaryxHomeThreadListPager` 既有语义（并发契约
  PagerTests:240 不动；隔离在 Favorites feed 层）。不复用现有 `reset()`。
- `GaryxRecentThreadFilterStorage`：持久化新 case。
- `GaryxGatewayClient`：`listThreadFavorites()`、
  `setThreadFavorited(...)` → `ok(page)/conflict(page)/notFound` 三分。
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
  runtime UUID 围栏对齐 pin（:92）。
- **Archive/Delete marker 接入（对 round6-2，改编排层）**：现有 archive
  编排（Bots.swift 的请求发起处）与通用删除（ThreadLifecycle.swift:323 的
  发起处）在**派发前**判定 `wasInFavoritesDomain` 并调 Core 建 lifecycle
  marker + 关 gate；`do/catch` 的成功、确定性失败、模糊失败三类结局都回喂
  Core（catch 分支 :300 一类不再只取消 transition）。**不挂在成功专用本地
  删行函数上**（:270/:286 只在成功分支执行，覆盖不了模糊）。
- `GaryxMobileModel+ThreadList.swift`：`refreshThreads` 并行组增拉
  `listThreadFavorites`（正好构成 §7.4 的配对观察）；**All feed 辅助刷新穷尽
  switch**（`.all` 只刷 All；其它 case 刷自身 + All），扩展
  RefreshCommitTests（:523 附近）。
- **Gateway 切换清理**：`GaryxMobileModel+Gateway.swift`（:55 附近）清
  `GaryxFavoritesState`（raw、intents、revision 水位、epoch、marker、gate）
  与 Favorites feed。
- 新 Core 文件跑 `xcodegen generate` 并提交 pbxproj，验证走 `xcodebuild`。

## 7. 客户端收藏状态机规格（双端共同契约）

### 7.1 全局状态与身份围栏

- `raw`：最近一次按 revision 单调接受的服务端整页（`Set<threadId>` +
  `revision`）。来源：GET、写响应页、409 页。接受条件
  `page.revision >= highestObservedRevision` 且 scope 围栏匹配。
- **写派发前置**：首次成功 GET 之前不派发写（`expected_revision` 必填）；
  期间 toggle 意图排队（presented 即时生效），raw 就绪后 drain。
- **flight 身份 = `(gatewayScope, runtimeEpoch, requestToken)`**；token
  allocator 进程内单调、永不随清场重置；结算与页接受要求三元组全匹配。
- **transport 契约（对 round6-1）**：本状态机的每次 dispatch 恰对应**一次**
  网络 attempt；transport 层不得自动重试。一切重试都是状态机新 dispatch
  （新 token、新 expectedRevision、新 marker 条目）。
- per-thread：`inFlight?` 与 `latestDesired?`（§7.2）。
- 呈现值 `presented(id) = latestDesired[id]?.desired ?? raw.contains(id)`。
- GET raw 接受永不退休意图；意图只经 flight 结算退休。

### 7.2 每线程意图状态机

```
inFlight      = { requestToken, target, flightGeneration, expectedRevision }
latestDesired = { generation, desired, mustWrite }
```

- **toggle(id, desired)**：`generation += 1`；`latestDesired = {generation,
  desired, mustWrite: false}`；无 inFlight 且 raw 就绪 → dispatch。有 inFlight：
  只更新 latestDesired，绝不动 inFlight。
- **dispatch(id)**：建 CAS marker 条目（§7.4，携带 `expectedRevision =
  raw.revision`）→ `inFlight = {新 token, target, flightGeneration,
  expectedRevision}` → 发条件写（单 attempt）。per-thread single-flight。
- **结算四分 + 四步处理**：
  1. **结算 flight**：三元组匹配即结算；不匹配整体判弃。
  2. **接受页**：200 页与 409 页按 revision 单调接受入 raw。
  3. **意图裁决**：
     - **成功（200）**：`generation <= flightGeneration` → 退休；
       `generation >` 且 `desired != raw.contains(id)` → 保留进 4；相等 →
       退休。CAS marker 条目保留（待 §7.4 围栏规则清除）。
     - **conflict（409，单 attempt 下确定性未应用，零副作用）**：撤销本次
       CAS marker 条目；按 409 页更新 raw 后同规则裁决意图（不一致 → 保留
       进 4 用新 revision 重派；一致 → 退休）。
     - **模糊失败（超时/截断/2xx 解码失败——服务端可能已提交，且其
       blocking task 可能仍在等 writer）**：CAS marker 条目**保留**（其
       expectedRevision 即围栏水位）；`generation == flightGeneration` →
       退休意图，呈现回落 raw，终态由周期 GET 对齐；`generation >` → 置
       `mustWrite`，保留进 4。
     - **notFound（404，单 attempt 下确定性未应用）**：撤销本次 CAS marker
       条目；退休意图（线程已不存在，行随 GET/replacement 消失）。
  4. **drain**：清 inFlight 后若存在未退休 latestDesired：`mustWrite` → 无条件
     dispatch（新条目、当前 raw.revision）；否则按 `desired !=
     raw.contains(id)`。
- **既往反例封闭性**：round5-1 双序（no-op 恒 bump 拒孤儿 / 409 拿新页重派）、
  round4-1（mustWrite）、round3-1（双身份）、round2-3（single-flight）全部
  保持；round6-1（重试伪装 409）被单 attempt 契约排除——409 恒真。

### 7.3 Favorites feed 的 unfavorite tombstone

- **hidden-pending**：`latestDesired.desired == false` 未确认。确定性失败
  （409 收敛为不需删 / 404）且意图退休 → 解除隐藏。
- **hidden-tombstone**：DELETE 成功结算后转入。唯一退休条件：DELETE settle 后
  签发的成功 replacement，与整页替换原子退休。失败 → 存续，行隐藏。
- **模糊失败的取消**：意图退休后行不再由 intent 隐藏（呈现回落 raw），但 CAS
  marker 条目仍在、gate 已关；若服务端实际已删，周期 GET 新 raw 移除该行。
  短暂回显可接受。
- **重新收藏打断**：tombstone 期间再 Favorite → 行重现；PUT 成功后照常
  replacement。
- 隐藏是后置过滤：对缓存行与任何 in-flight 响应行一律生效。

### 7.4 marker、配对 replacement 与分页 gate（对 round6-2/3 重构）

**marker（`unreconciledDomainMutations`）——条目分两类，各带结算语义：**

```
CAS 条目       = { threadId, expectedRevision: E }
  建立：favorite/unfavorite dispatch 前（§7.2）
  撤销：本 flight 确定性未应用（单 attempt 409 / 404）
  可清除条件：某次成功 replacement 的配对观察 R_obs > E
    （恒 bump 保证：orphan 写要么已提交且效果含于 revision ≥ E+1 的页，
     要么因围栏永不能再提交——两种命运都已封）

lifecycle 条目 = { opToken, threadId, settled: bool }
  建立：Archive/Delete 派发前（先判 wasInFavoritesDomain）
  撤销：确定性未应用（单 attempt 下的确定性错误 / 请求未发出）
  settled 置位：达到确定性结局——成功，或 already-gone（404 于**编排层显式
    重试**的后续 attempt；每次显式重试是新 attempt 但同一 opToken）。
    Archive/Delete 对 favorites 域是幂等效果：一旦确认线程已归档/已删，
    早先超时 attempt 的延迟提交是域上 no-op，不再构成漂移源
  可清除条件：settled 之后签发的成功 replacement
  unsettled 期间：编排层以有界退避显式重试至确定性结局；重试耗尽仍模糊 →
    条目存续（gate 持续关闭，缓存仍可见，只禁 load-more）
```

- **gate 闭合条件：marker 非空 ⟺ 分页 gate 关闭**。条目建立于派发前 →
  in-flight 窗口内 gate 已关（round6-2 的窗口消除）。
- **配对 replacement**（对 round6-3）：一次 replacement = **并行拉
  `GET /api/thread-favorites`（提供配对观察 `R_obs`）+
  `GET /api/recent-threads?favorites=only`**，两者都成功才算成功（iOS
  `refreshThreads` 本就并行拉两者；desktop favorites feed 刷新照此配对）。
  成功时：整页原子替换、cursor 重置新 head、按上述**可清除条件**逐条清
  marker（CAS 条目按 `R_obs > E`；lifecycle 条目按 settled-before-dispatch）、
  退休满足条件的 tombstone；仍有条目 → gate 保持关闭并驱动下一轮周期
  replacement。**replacement 只读快照可越过 pending writer（mod.rs:363/978），
  因此它永不清除 unsettled/未过围栏的条目**——round6-3 的时序下六个 CAS/
  lifecycle 条目保留，gate 不开，延迟提交后的下一轮配对观察才满足清除条件。
- **开始 replacement**：签发单调 ticket；epoch bump 废弃 in-flight refresh/
  load-more 两 track；清 active flags；关 gate；保留 display-only IDs
  （无 skeleton，tombstone 后置过滤持续生效）。
- **失败**：保留显示缓存；tombstone 与 marker 不动；marker 非空 → gate 关；
  marker 为空 → 允许旧 cursor 重开（无本端漂移源；他端漂移与 All/Chats feed
  既有容忍度一致）。
- 两种完成顺序无害（epoch 判弃 / 整页覆盖）；无「旧 offset 页追加新 head」。
- 收藏集合小，配对整拉成本可忽略；用户可见的只有行集原子变化。

**round6-3 反例走查**：六个 Favorites 域内 Archive 超时（blocking 写等
writer）——六个 lifecycle 条目已于派发前建立、均 unsettled、gate 关。其后
replacement 越过 pending writer 读到旧集合并成功 → **unsettled 条目不可清**，
gate 保持关闭。延迟提交发生后，编排层显式重试拿到 already-gone → settled →
下一次成功配对 replacement（其页已含删除效果）清条目 → gate 开，cursor 已
重置，无漏行。✅

## 8. 测试计划

**Gateway（`cargo test -p garyx-gateway --lib`）**

- meta：建库即有 `(1,0)`；重开幂等；初始化早于启动 purge。
- db（CAS）：接受的条件写恒 bump（含 no-op DELETE / 重复 PUT）；失配 →
  Conflict + 当前页同快照、零副作用；孤儿写交错（旧 expected_revision 在
  后继接受写之后提交 → 拒）；守卫插入交错（归档先提交 → NotFound）；三清理点
  同事务清行 + 变更时 bump；同快照组页交错（对齐 mod.rs:5462）；
  `favorites=only` 过滤域分页；page+count 一次读快照。
- routes：200/409/404/400 契约（409 带整页）；`favorites` 非法值 400；
  `favorites=only`+显式 `tasks` 400；`tasks=only` 回归。

**双端状态机（desktop `npm run test:unit` / iOS SwiftPM，逐条对照 §7）**

- **transport 单 attempt 契约（round6-1）**：计数 mock 钉死一次 settle 至多
  一次网络 attempt（iOS `maxAttempts: 1` 断言 + desktop fetch 无重试断言）；
  「首 attempt 提交、响应丢失」在单 attempt 下即模糊失败 → marker 保留（不
  存在次 attempt 409/404 伪装路径）。
- 同 ID 覆盖 + 旧 flight 成功/模糊失败/409：终态恒 = 最后意图；409 新 revision
  重派；mustWrite 无条件派发。round5-1 双序走查。
- 同代模糊失败：意图退休 + CAS 条目保留 + gate 关；周期 GET 对齐终态。
- 不同 ID 部分失败隔离；网关切换围栏；远端更高 revision 推翻本地态；desktop
  raw 纯度；gateway 切换全清（含 marker/gate）。

**marker/gate/replacement（双端，对 round6-2/3）**

- **派发前建条目**：CAS 写与 Archive/Delete 在请求发出前 marker 已非空、gate
  已关（in-flight 窗口无 load-more）。
- **round6-3 主反例**：lifecycle 条目 unsettled + replacement 成功 → 条目不清、
  gate 不开；显式重试 already-gone → settled → 下次成功 replacement 清 →
  gate 开、无漏行。**「replacement 成功在先、延迟 mutation 提交在后」两类都
  测（CAS 与 lifecycle），不只测 replacement 全失败**。
- **CAS 围栏清除规则**：`R_obs == E` 的成功 replacement 不清条目（orphan 仍可
  提交）；`R_obs > E` 才清。orphan 后提交 → 下轮配对观察 R_obs > E → 清。
- 确定性未应用（409/404）→ 条目即时撤销；gate 随 marker 清空重开。
- offset 漂移三源（round4-3/round5-2/round5-3 保留）：>overlap 连续确认取消 /
  模糊失败取消 / Archive（含模糊），replacement 全失败 → gate 关、load-more
  拒；一次满足清除条件的成功 replacement 后收敛重开。
- 配对 replacement：favorites GET 与 recent-threads 任一失败 → replacement
  失败（不换页、不清 marker）。

**Favorites feed（双端）**

- DELETE 成功 → replacement 失败 → 行仍隐藏；成功后 tombstone 原子退休。
- 确定性失败 → 行重现；tombstone 期间重新收藏 → 行重现。
- replacement vs load-more 两种完成顺序；在途拒新 load-more。
- replacement 期间缓存保留（10s 刷新无 skeleton）；乐观取消期间 in-flight
  load-more 不复活行。

**其余**

- desktop：三 tab、方向键、空态、Favorites 行 accessory = Unfavorite+Archive；
  判别联合映射穷尽；store 按 gateway key 隔离。
- iOS：FilterStorage 往返；GatewayClient 三分结果 + 单 attempt；
  Reducer/Actor/Presentation；RefreshCommitTests `.favorites` 时 All feed 刷新；
  archive 编排层三类结局回喂（成功/确定失败/模糊）。
- 端到端：本地 gateway curl 三端点（含 409 路径）+ `favorites=only`；双端 UI
  按 `garyx-product-ui` 走查两处入口 + 筛选切换 + 行内取消。

## 9. 实现切分建议

单 PR 可完成，按依赖顺序提交：① gateway（表 + meta + CAS revision + API +
过滤 + 三清理点）→ ② 双端共享状态机（iOS Core `GaryxFavoritesState` /
desktop `favorites-ingress`，先测后 UI；含 marker/gate/配对 replacement）→
③ desktop renderer / iOS App UI + xcodegen（含 Archive/Delete 编排层接入）。
每步各自带测试。
