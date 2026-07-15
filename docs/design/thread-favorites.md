# Thread Favorites (线程收藏)

Status: draft v6 (addressing review round 5 — #TASK-2324 FAIL, 3 findings)
Date: 2026-07-16

## 0. 修订记录

### Round 5 findings → v6 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | 客户端超时的旧写，其服务端 blocking task 仍存活，可在后继写**之后**取得 writer mutex 落库（提交顺序 ≠ dispatch 顺序）；三元组围栏只能弃响应、撤不了 DB 副作用，latest-intent-wins 被反转 | **服务端写围栏：PUT/DELETE 必带 `expected_revision`（CAS）**，不匹配 → 409 + 当前整页（同快照）；**被接受的条件写恒 bump revision（即使 membership 无变化的 no-op）**——孤儿旧写携带过时 expected_revision 必被拒。先例：pins reorder 的 `expected_revision`/409（§3、§4.1、§7.2）。v5 的「revision 不用于写侧 CAS」裁决**撤销** |
| 2 | 「服务端已提交、客户端模糊失败」的写（2xx 截断/解码失败，transport 不重试）不产生 tombstone 也不进漂移标记，gate 照开 → 六次这种取消后 load-more 旧 offset 永久跳行（现有 PagerTests:456 已证明 6 removals + overlap 5 跳行） | 漂移标记升级为 **`unreconciledDomainMutations`**，收录一切「已确认 **或 possiblyCommitted**」的域变更：写成功、**模糊失败（除 409/404 等确定性未应用外一律视为 possiblyCommitted）**；入标记即**立刻关分页 gate**，只有其后签发的成功 replacement 才清除（§7.4） |
| 3 | Archive/Delete 生命周期写会改变 Favorites 过滤域（归档事务连带清 favorites 行），但未接入标记/gate；连续归档 >overlap 行 + replacement 全失败 → 同样永久跳行 | 本端发起的 **Archive/Delete（确认或模糊）凡涉及 Favorites 域内线程（presented-favorited 或在 feed 缓存中）→ 同样计入 `unreconciledDomainMutations` + 关 gate**；接入点 = 双端生命周期操作的本地行移除处（iOS Bots.swift:270 一类路径、desktop 行 Archive action）；补 >overlap 归档/删除测试（§5.2、§6.2、§7.4） |

round 5 已确认：round4-2 三元组响应围栏成立；round4-3 对「确认成功的取消」成立
（本轮扩标记覆盖面）；round4-1 文档反例已修，本轮封其「超时旧写后落库」变体。

### 历史轮次（要点）

- **Round 4 → v5**：mustWrite 幂等消歧（失败路径禁用过时 raw 消意图）；
  `(gatewayScope, runtimeEpoch, requestToken)` 围栏；未收敛变更时 gate 关闭。
- **Round 3 → v4**：flight/desired 双身份分离；main 只发布纯 raw；分级
  tombstone 原子退休；replacement 废弃两 track + 关 gate。
- **Round 2 → v3**：meta singleton 初始化（早于 purge）；同快照组页不变量；
  `FavoriteThreadResult::{Updated|NotFound}` + 404；共享行 accessory 契约；
  清理点三处（归档 mod.rs:669、通用删除 mod.rs:2045、启动 purge mod.rs:2785）。
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
- 双端筛选器各加 Favorites 类别；Favorites feed 用 replacement-refresh 语义（§7.4）。
- 跨端收敛：客户端统一收藏意图状态机（§7）+ revision 单调接受 + **写侧 CAS 围栏**。

**非目标**

- 收藏排序 / 拖拽重排（不引入 sort_order、reorder outbox）。
- 首页独立「收藏段」；All/Chats 行星标徽标（Favorites tab 行内取消按钮除外）。
- SSE 推送收藏变更（沿用刷新收敛）。
- bot 命令面不加收藏筛选。
- 收藏意图跨进程/重启持久化（瞬态 UI 状态，进程退出回落 raw）。

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

- **真源**：`thread_favorites` 表。不写 `thread_records` body、不在
  `recent_threads` 加列。
- **meta 初始化**：建表后立即 `ensure_thread_favorites_meta_row`
  （`INSERT (1,0) ON CONFLICT(id) DO NOTHING`，镜像 mod.rs:3529），**早于
  retired-workflow 启动清理**（pin 先例 mod.rs:2669）。重开幂等，不重置 revision。
- **revision 语义（v6 修订，对 round5-1）**：
  - `favorites_revision` 全局单调，**兼任快照排序与写侧 CAS 围栏**（v5 的
    「不用于写侧 CAS」撤销；先例：pins reorder 的 `expected_revision` CAS，
    mod.rs:590）。
  - **条件写（PUT/DELETE）被接受即恒 bump**——包括 membership 无变化的 no-op
    （no-op DELETE/重复 PUT）。这是围栏成立的必要条件：后继 no-op 写 bump 后，
    孤儿旧写的过时 `expected_revision` 必然失配被拒。
  - **生命周期清理点删除**（非 CAS 写）维持「仅实际变更时 bump」
    （镜像 mod.rs:2924）。
- **守卫式单事务插入**（单 writer mutex mod.rs:363 + writer 串行化；幽灵行
  两方向封死）：CAS 校验、存在守卫、写入、bump、回读整页在**同一写事务**内：

```sql
-- 事务内：①校验 expected_revision == favorites_revision，否则 Conflict(当前页)
-- ②存在守卫插入 ③bump ④同事务回读整页
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

## 4. Gateway API

### 4.1 收藏读写（写侧 CAS，v6 修订）

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids, favorites: [{thread_id, favorited_at}], revision }`，按 `favorited_at DESC, thread_id ASC` |
| `PUT /api/thread-favorites/{key}?expected_revision=N` | 接受：200 `{ favorited: true, thread_ids, favorites, revision }`（**接受即 bump，含重复 PUT no-op**）；revision 失配：**409** `{ conflict: true, thread_ids, favorites, revision }`（当前整页，同快照）；线程不存在：404 `{ favorited: false, error }`；`expected_revision` 缺失/非法：400 |
| `DELETE /api/thread-favorites/{key}?expected_revision=N` | 接受：200 `{ favorited: false, removed, thread_id, thread_ids, favorites, revision }`（**接受即 bump，含 no-op**）；失配 409 / 404 / 400 同上 |

- **`expected_revision` 必填**：客户端在首次成功 GET 拿到 raw 之前不派发写
  （意图排队，§7.1）。围栏语义：它保证**同一客户端 single-flight 序列的提交
  顺序**——被超时遗弃的旧写携带旧 revision，在任何后继接受写（恒 bump）之后
  必被 409 拒绝、零副作用。跨端并发写不需要围栏语义（last-writer-wins 可接受），
  409 后按 §7.2 重派收敛；持续外部写导致的重试理论上可无限（对抗性场景），
  用户手速节奏下实际有界，接受。
- **同快照组页不变量**：GET 单一读事务读 `{行集, revision}`（对齐 mod.rs:538）；
  PUT/DELETE 的 200 与 **409 响应页**都在同一写/读事务内回读。配交错测试
  （对齐 mod.rs:5462 模式）。
- **NotFound**：db 层 `FavoriteThreadResult::{Updated(page) | Conflict(page) |
  NotFound}`；route 映射 200/409/404。route 前置 `ensure_existing_thread_id` =
  规范 key 校验 + 存在 point-check 的友好 404 短路（routes.rs:1104），非别名
  解析；最终裁决在 db 事务。
- 路由注册 `route_graph.rs`，handler 在 `routes.rs` 紧邻 thread-pins。

### 4.2 `/api/recent-threads` favorites 过滤（round 2 起 CONFIRMED，未改动）

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

### 5.1 契约与主进程——raw 唯一所有权

裁决：**favorites 乐观状态机唯一 owner = renderer ingress**；main 对 favorites
不做任何 pending 投影（与 pin 的 main 侧 controller 有意分道：那是 reorder
outbox 持久化需求，收藏没有）。

- main 发布 `favoritedThreadIds` + `favoritesRevision` **= 纯 raw**（仅来自
  服务端页：周期 fetch + 写响应/409 响应页），revision 单调接受；写 in-flight
  期间 main 发布值不变（契约测试钉死）。
- **按 `entitiesGatewayUrl` 归一化 key 存取**（对齐 store.ts:498）。
- **IPC 带 scope stamp + expected_revision**：`setThreadFavorited(threadId,
  favorited, expectedRevision)` 纯转发（HTTP → `{status: ok|conflict|notFound,
  page, scopeStamp}`）；renderer 按 §7.1 围栏校验。
- `src/shared/contracts/thread.ts`：判别联合
  `RecentThreadListFilter = "all" | "chats" | "favorites"`；wire 映射 main 层
  纯函数；类型排除 `tasks`+`favorites` 并存。新增 `DesktopThreadFavoritesPage`。
- `src/main/garyx-client/threads.ts`：`fetchThreadFavorites()`、
  `setRemoteThreadFavorited(threadId, favorited, expectedRevision)`（区分
  ok/conflict/notFound，对齐 `reorderRemoteThreadPins` 的 accepted/conflict
  形态）；`fetchRecentThreads` 按联合拼参。

### 5.2 Renderer

- **favorites-ingress（唯一乐观层）**：实现 §7 状态机。呈现值 = main raw ⊕
  ingress intents。
- **入口 1**：`ConversationHeaderTitle.tsx` dropdown 紧邻 Pin 项增加
  "Favorite conversation"/"Unfavorite conversation"（`lucide-react`
  `Star`/`StarOff`）。不设快捷键。
- **入口 2**：Favorites tab 行内取消收藏。**扩展共享 `ThreadRailRow` 行 action
  契约为可组合 accessory 列表**，Favorites tab 行传 `[Unfavorite, Archive]`，
  其它 tab 不变。
- **Archive 接入标记（对 round5-3）**：Favorites tab（及任何面）上对
  **Favorites 域内线程**（presented-favorited 或在 favorites feed 缓存中）发起
  的 Archive/Delete，结果为确认或模糊（possiblyCommitted）时 → 计入
  `unreconciledDomainMutations` + 关分页 gate（§7.4）。接入点 = 行 Archive
  action 的结果处理处（本地行移除的同一位置）。
- **筛选 tab**：`recent-thread-feeds.ts` 扩展
  `RecentThreadFilter = "all" | "nonTask" | "favorites"`，第三条独立 feed，
  label `"Favorites"`；`useRecentThreadFeeds`、`RecentConversationSidebar`
  （第三 tab + 方向键）、sidebar-model（空态 "No favorite threads"）。
- **Favorites feed**：replacement-refresh + tombstone + gate 按 §7.3/7.4
  （desktop `recent-thread-feeds.ts:232` 现无 `isRefreshingHead` 阻断，
  favorites feed 显式管理）。
- i18n：增 "Favorites"、"Favorite conversation"、"Unfavorite conversation"、
  空态文案。

## 6. iOS

### 6.1 GaryxMobileCore

- **`GaryxFavoritesState`**：实现 §7 状态机（纯逻辑、SwiftPM 可测）。围栏与
  两阶段机制对齐 `GaryxPinnedOrderState`（stamp 含 gatewayIdentity+epoch :13；
  结算与 raw 接受分离 :227/:290；conflict 处理对齐其 reorder 409 路径）。
  gateway 切换全清 + epoch bump。
- `GaryxRecentThreadFilter`：新 case `.favorites`；`displayName`/
  `activeStatusLabel = "Favorites"`；`homeMenuOptions` 加入；wire 映射结构化
  query 描述（二选一），纯函数 + 穷尽测试。
- `GaryxRecentThreadFeeds`：第三条 feed，replacement/tombstone/gate 按
  §7.3/7.4。**不改 `GaryxHomeThreadListPager` 既有语义**（并发契约
  PagerTests:240 不动；隔离在 Favorites feed 层）。不复用现有 `reset()`。
- `GaryxRecentThreadFilterStorage`：持久化新 case。
- `GaryxGatewayClient`：`listThreadFavorites()`、
  `setThreadFavorited(id:favorited:expectedRevision:)` → 结果三分
  `ok(page)/conflict(page)/notFound`（对齐 `reorderThreadPins` 的 409 →
  `.conflict` 模式）。**模糊失败面**：超时/截断（:1025/:1061）、2xx 后解码失败
  （:987/:1000，transport 不重试）→ §7.2 possiblyCommitted 处理。
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
- **Archive/Delete 接入标记（对 round5-3）**：现有归档成功路径「本地删行 +
  刷新」（Bots.swift:270；feed 本地删除只移 ID 不修 cursor，
  GaryxRecentThreadFeeds.swift:174）处，凡目标线程在 Favorites 域内（确认或
  模糊结果）→ 计入 `unreconciledDomainMutations` + 关 gate。通用删除路径同理。
- `GaryxMobileModel+ThreadList.swift`：`refreshThreads` 并行组增拉
  `listThreadFavorites`；**All feed 辅助刷新穷尽 switch**（`.all` 只刷 All；
  其它 case 刷自身 + All），扩展 RefreshCommitTests（:523 附近）。
- **Gateway 切换清理**：`GaryxMobileModel+Gateway.swift`（:55 附近）清
  `GaryxFavoritesState`（raw、intents、revision 水位、epoch、标记）与
  Favorites feed。
- 新 Core 文件跑 `xcodegen generate` 并提交 pbxproj，验证走 `xcodebuild`。

## 7. 客户端收藏状态机规格（双端共同契约）

### 7.1 全局状态与身份围栏

- `raw`: 最近一次按 revision 单调接受的服务端整页（`Set<threadId>` +
  `revision`）。来源：GET、写响应页、**409 conflict 响应页**。接受条件
  `page.revision >= highestObservedRevision` 且 scope 围栏匹配。
- **写派发前置**：首次成功 GET 之前不派发写（`expected_revision` 必填）；
  期间 toggle 意图排队（presented 即时生效），raw 就绪后 drain。
- **flight 身份 = `(gatewayScope, runtimeEpoch, requestToken)`**：`runtimeEpoch`
  随 gateway 切换/清场单调 bump；`requestToken` 进程内单调、**allocator 永不
  随清场重置**。结算与 raw 接受要求三元组全匹配；孤儿响应（含页）整体判弃。
- per-thread：`inFlight?` 与 `latestDesired?`（§7.2）。
- 呈现值 `presented(id) = latestDesired[id]?.desired ?? raw.contains(id)`。
- **GET raw 接受永不退休意图**；意图只经 flight 结算退休。

### 7.2 每线程意图状态机

```
inFlight      = { requestToken, target, flightGeneration, expectedRevision }
latestDesired = { generation, desired, mustWrite }
```

- **toggle(id, desired)**：`generation += 1`；`latestDesired = {generation,
  desired, mustWrite: false}`；无 inFlight 且 raw 就绪 → dispatch。有 inFlight：
  只更新 latestDesired，绝不动 inFlight。
- **dispatch(id)**：`inFlight = {新 token, target: latestDesired.desired,
  flightGeneration: latestDesired.generation, expectedRevision:
  raw.revision}`；发条件写。per-thread single-flight。
- **结算三分 + 四步处理**：
  1. **结算 flight**：三元组匹配即结算；不匹配整体判弃。
  2. **接受页**：成功 200 页与 conflict 409 页都按 revision 单调接受入 raw。
  3. **意图裁决**：
     - **成功（200）**（raw 新鲜无歧义）：`generation <= flightGeneration` →
       退休；`generation >` 且 `desired != raw.contains(id)` → 保留进 4；
       `generation >` 且相等 → 退休。**计入 `unreconciledDomainMutations`
       （确认写）**。
     - **conflict（409，确定性未应用，零副作用）**：不计入标记；
       `generation <= flightGeneration` 且 `desired != raw.contains(id)`
       （409 页）→ 保留进 4（用新 revision 重派）；相等 → 退休；
       `generation >` → 按新 raw 同规则裁决。
     - **模糊失败（超时/截断/2xx 解码失败——服务端可能已提交）**：
       **无论哪代，先计入 `unreconciledDomainMutations`（possiblyCommitted）+
       关分页 gate（§7.4）**；`generation == flightGeneration` → 退休意图，
       呈现回落 raw，实际终态由周期 GET 对齐；`generation >` → 置
       `mustWrite`，保留进 4。
     - **notFound（404，确定性未应用）**：线程已不存在；退休意图，等 GET/
       replacement 收敛（该行同时会从 recent_threads 消失）。
  4. **drain**：清 inFlight 后若存在未退休 latestDesired：`mustWrite` → 无条件
     dispatch（携带当前 raw.revision）；否则按 `desired != raw.contains(id)`。
- **round5-1 反例走查**：raw=false@R0；PUT(g1,true,expected=R0) 超时（服务端
  blocking task 存活未提交）；g2=false 置 mustWrite → DELETE(expected=R0)。
  - 序 A：DELETE 先取得 writer → no-op 但**接受即 bump** R0→R1，g2 成功退休；
    孤儿 PUT 后取得 writer → expected R0 ≠ R1 → **409 拒绝零副作用**。终态
    false = 最后意图。✅
  - 序 B：孤儿 PUT 先提交 R0→R1（true）；DELETE(expected=R0) → 409 + 页
    {true, R1} → raw 更新，desired false ≠ raw true → 重派
    DELETE(expected=R1) → 接受 R1→R2，终态 false。✅
  （v5 在序 A 下 DELETE no-op 不 bump、孤儿 PUT expected 匹配落库 → true，
  被 REFUTED；恒 bump + CAS 封死。）
- round4-1/round3-1/round2-3 反例依旧封闭（mustWrite、双身份、single-flight）。

### 7.3 Favorites feed 的 unfavorite tombstone

- **hidden-pending**：`latestDesired.desired == false` 未确认。确定性失败
  （409 后收敛为不需删 / 404）且意图退休 → 解除隐藏。
- **hidden-tombstone**：DELETE 成功结算后转入。唯一退休条件：DELETE settle 后
  签发的 replacement ticket 成功，与整页替换原子退休。失败 → 存续，行隐藏。
- **模糊失败的取消**：意图退休后行**不再由 intent 隐藏**（呈现回落 raw），但
  已计入标记、gate 已关；若服务端实际已删，周期 GET 的新 raw（revision 更高）
  会移除该行，随后成功 replacement 清标记。短暂回显可接受（呈现层无歧义信息）。
- **重新收藏打断**：tombstone 期间再 Favorite → 行重现；PUT 成功后照常
  replacement。
- 隐藏是**后置过滤**：对缓存行与任何 in-flight 响应行一律生效。

### 7.4 replacement-refresh 与分页 gate

Favorites feed 专用协议（All/Chats feed 现有 pager 语义完全不动）：

- **`unreconciledDomainMutations`（对 round5-2/3 扩容）**：记录自上次成功
  replacement 以来，本端发起的、**已确认或 possiblyCommitted** 的一切
  Favorites 域变更：
  (a) 确认成功的 favorite/unfavorite（200）；
  (b) **模糊失败的 favorite/unfavorite**（超时/截断/2xx 解码失败）；
  (c) **本端发起的 Archive/Delete**（确认或模糊），目标线程在 Favorites 域内
  （presented-favorited 或在 feed 缓存中）。
  确定性未应用（409/404/连接前失败可判定者）不计入。
  **入标记即立刻关分页 gate**（不等 replacement 失败）。
- **开始 replacement**（写确认、周期刷新、下拉、切入 tab）：签发单调 ticket；
  epoch bump 废弃 in-flight refresh/load-more 两 track；清 active flags；关
  gate；保留 display-only IDs（无 skeleton，tombstone 后置过滤持续生效）。
- **成功**：整页原子替换；cursor 重置新 head；退休满足条件的 tombstone；
  **清除本次 replacement 派发之前记录的标记项**（派发后新增的保留并继续关
  gate、驱动下一轮）；标记空 → 开 gate。
- **失败**：保留显示缓存；tombstone 不退休；标记**非空 → gate 保持关闭**
  （周期 10s 刷新持续重试）；标记为空 → 允许旧 cursor 重开（无本端漂移源；
  他端漂移与 All/Chats feed 既有容忍度一致）。
- 两种完成顺序无害（epoch 判弃 / 整页覆盖）；无「旧 offset 页追加新 head」路径。
- 收藏集合小，整页替换成本可忽略；用户可见的只有行集原子变化。

## 8. 测试计划

**Gateway（`cargo test -p garyx-gateway --lib`）**

- meta：建库即有 `(1,0)`；重开幂等；初始化早于启动 purge。
- db（CAS，对 round5-1）：**接受的条件写恒 bump（含 no-op DELETE / 重复
  PUT）**；expected_revision 失配 → Conflict + 当前页同快照、零副作用；
  **孤儿写交错测试：旧 expected_revision 写在后继接受写之后提交 → 被拒**；
  守卫插入交错（归档先提交 → NotFound）；三清理点同事务清行 + 变更时 bump；
  同快照组页交错（对齐 mod.rs:5462）；`favorites=only` 过滤域分页；
  page+count 一次读快照。
- routes：200/409/404/400 契约（409 带整页）；`favorites` 非法值 400；
  `favorites=only`+显式 `tasks` 400；`tasks=only` 回归。

**双端状态机（desktop `npm run test:unit` / iOS SwiftPM，逐条对照 §7）**

- 同 ID 覆盖 + 旧 flight 成功/模糊失败/409：终态恒 = 最后意图；409 用新
  revision 重派；mustWrite 无条件派发。
- **round5-1 双序走查**（no-op bump 拒孤儿 / 409 后重派）。
- 同代模糊失败：意图退休 + **标记计入 + gate 立即关**；周期 GET 对齐终态。
- 不同 ID 部分失败隔离；网关切换围栏（旧孤儿响应不结算新 flight、token
  allocator 不重置）；远端更高 revision 推翻本地态；desktop raw 纯度；
  gateway 切换全清。

**Favorites feed（双端）**

- DELETE 成功 → replacement 失败 → 行仍隐藏；成功后 tombstone 原子退休。
- 确定性失败 → 行重现；tombstone 期间重新收藏 → 行重现。
- replacement vs load-more 两种完成顺序；在途拒新 load-more。
- **offset 漂移三源测试（round4-3 + round5-2/3）**：
  (a) >overlap 连续**确认**取消 + replacement 全失败 → gate 关、load-more 拒；
  (b) >overlap 连续**模糊失败**取消（服务端实际已提交）→ 标记计入、gate 关；
  (c) >overlap 连续 **Archive**（含模糊）Favorites 域内线程 + replacement 全
  失败 → gate 关；
  各自在一次成功 replacement 后收敛、gate 重开。
- 标记为空时 replacement 失败 → 旧 cursor 重开可分页。
- replacement 期间缓存保留（10s 刷新无 skeleton）；乐观取消期间 in-flight
  load-more 不复活行。

**其余**

- desktop：三 tab、方向键、空态、Favorites 行 accessory = Unfavorite+Archive；
  判别联合映射穷尽；store 按 gateway key 隔离。
- iOS：FilterStorage 往返；GatewayClient 三端点（ok/conflict/notFound 三分）；
  Reducer/Actor/Presentation；RefreshCommitTests `.favorites` 时 All feed 刷新。
- 端到端：本地 gateway curl 三端点（含 409 路径）+ `favorites=only`；双端 UI
  按 `garyx-product-ui` 走查两处入口 + 筛选切换 + 行内取消。

## 9. 实现切分建议

单 PR 可完成，按依赖顺序提交：① gateway（表 + meta + CAS revision + API +
过滤 + 三清理点）→ ② 双端共享状态机（iOS Core `GaryxFavoritesState` /
desktop `favorites-ingress`，先测后 UI）→ ③ desktop renderer / iOS App UI +
xcodegen（含 Archive/Delete 标记接入）。每步各自带测试。
