# Thread Favorites (线程收藏)

Status: draft v9 (addressing review round 8 — root-cause adoption + reducer unification)
Date: 2026-07-16

## 0. 修订记录

### v9 架构转折（用户 scope 裁决：现有系统设计问题直接改根因）

Round 1–8 的客户端 marker/gate/operation-group 体系是对 **offset 分页漂移体质**
的层层防御，round 8 证明它在进程退出场景下无法闭合（finding 4）。v9 按新 scope
采纳 reviewer 根因裁决表的全部三项系统级修改，**删除整个 marker/gate 体系**：

| 根因修改 | 取代的绕行 | 连带消灭的 findings |
|---|---|---|
| **R1 传输契约重构**：删除 `idempotent: Bool` 隐式语义，显式 `readRetryable / mutationSingleAttempt`；mutation 的 session task 创建后一切 transport error 恒 ambiguous | Favorites 专用 maxAttempts:1 + 专用 classifier | round6-1、round7-5 类（全局根治） |
| **R2 单事务组合快照端点**：`GET /api/thread-favorites/snapshot` 一个读事务返回 favorites 页 + revision + Favorites recent 页 | 客户端串行双 GET 编排 | round7-2 类（同快照由服务端构造保证） |
| **R3 keyset cursor 分页**：`/api/recent-threads` 全滤镜改 opaque keyset cursor（`last_active_at, thread_id`）；favorites cursor 内嵌 `favorites_revision`，失配由**服务端返回全新首页 + reset 标记** | unreconciledDomainMutations marker、分页 gate、lifecycle operation-group、入标记三连、settlement 语义 | round4-3、round5-2/3、round6-2/3、round7-3/4、**round8-1/2/3/4 全部**（删除行不再移位后续页；域变更由服务端 cursor 校验强制 reset；无客户端记账 → 无进程退出丢失问题） |

### Round 8 findings → v9 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | P0 CAS notSent 分支未定义 + marker 无 attempt 身份，探针 notSent 会误删旧 ambiguous marker | marker 体系整体删除（R3）。意图 reducer 补 **notSent 分支**：意图保持（active），按带 `(scope, epoch, generation, fence)` stamp 的退避 effect 重新调度（§7.2） |
| 2 | P1 notFound 只撤当前 marker，留下 revision 恒 =E 的永不可清旧 marker，gate 永闭 | marker 体系删除后不存在；notFound → 退休该线程意图（线程已消失，行随 snapshot 消失）（§7.2） |
| 3 | P1 lifecycle operation-group 缺「确定性业务冲突（archive/delete 409）」出口，marker 永留 | operation-group 整体删除（R3）：lifecycle 操作不再需要 favorites 侧记账——归档/删除在服务端清 favorites 行 + bump revision，favorites cursor 失配 → 服务端强制 reset |
| 4 | P1 进程退出丢 marker + gateway blocking task 存活 → 新进程旧 offset cursor 漂移跳行 | keyset cursor 根治：删除行不移位后续页；favorites cursor 内嵌 revision，任何域变更（含旧进程遗留的延迟提交）→ 服务端判 stale 强制 reset。无客户端持久化需求 |
| 5 | P2 awaitVerify 三处内部矛盾（GET 永不退休 vs 被动退休；「至多一个提交」只对同 E 成立；退避 timer 身份归属） | §7.2 统一为**单一 reducer 总定义**：意图退休的唯一入口 = reducer 的 settle/verify 转移（「GET 永不退休」改述为「raw 接受本身不是退休入口，verify 转移才是」）；「至多一个提交」限定**同 expectedRevision**；退避调度 = **Core 产生的带 stamp effect**，App/renderer 只做定时器宿主，到期把 effect 原样喂回 Core 校验 stamp |

### 历史轮次（要点）

- **R7→v8**：awaitVerify 意图验证循环（模糊失败永不静默退休 + 主动重派活性泵）
  ——**保留**，是 v9 意图机的核心；串行配对（被 R2 取代）；operation-group
  （被 R3 取代）。
- **R6→v7**：单 attempt（上移为 R1 全局契约）；派发前 marker（删除）；
  结算语义（删除）。
- **R5→v6**：**服务端 CAS 写围栏**（`expected_revision` 必填、接受即恒 bump 含
  no-op、409 同快照整页）——**保留，已 CONFIRMED**。
- **R4→v5**：三元组围栏 `(gatewayScope, runtimeEpoch, requestToken)`——保留。
- **R3→v4**：flight/desired 双身份；main 只发布纯 raw；tombstone——保留。
- **R2→v3**：meta singleton；同快照组页；`FavoriteThreadResult` 枚举；行
  accessory 契约；清理点三处（mod.rs:669/2045/2785）——保留，已 CONFIRMED。
- **R1→v2**：守卫式单事务插入；All feed 穷尽 switch；gateway 切换清理；
  判别联合；Mac 行内取消——保留。

评审已确认的全局裁决：独立 `thread_favorites` 表 + SQL JOIN 合规；参数互斥与
Favorites 含 task 线程语义正确；Core/App 分层与双端入口正确；revision+单
writer 构成正确 CAS 总序；串行双 GET 的 happens-before 论证成立（v9 改由
服务端单事务承担，不再需要）。

## 1. 需求

用户需求（产品裁决，不可改动的部分）：

1. 线程支持「收藏」（favorite）。
2. 最近线程列表的筛选类别变为三个：**全部（All）/ Chats / 收藏（Favorites）**。
3. iOS：
   - 首页线程行**长按 context menu** 出现收藏项；
   - 进入线程后**右上角菜单**里也有收藏项，与置顶（Pin）位置相邻；
   - 首页**右上角过滤器**增加「收藏」类别，点击查看收藏线程。
4. Mac app：收藏的触发点与置顶的触发点一致（同菜单同位置）；筛选处新增一个「收藏」tab。

**系统级连带改造（用户 scope 裁决授权）**：R1 传输契约、R2 组合快照端点、
R3 recent-threads keyset 分页（同时根治 All/Chats 现存的 offset 跳行缺陷——
现有测试 PagerTests:456 已证明 6 removals + overlap 5 必跳行）。

## 2. 目标 / 非目标

**目标**

- 收藏标记 + 双端入口 + 三分类筛选（原始需求）。
- R1/R2/R3 三项系统级根治（见 §0）。
- 客户端只保留**意图状态机**（§7.2）与 **tombstone 呈现过滤**（§7.3）；
  分页一致性全部由服务端保证。

**非目标**

- 收藏排序 / 拖拽重排；首页独立「收藏段」；All/Chats 行星标徽标；SSE 推送；
  bot 命令面收藏筛选。
- 收藏意图跨进程持久化（意图瞬态；分页一致性已不依赖客户端状态）。
- `serverIdempotencyKey`（reviewer 提及的可选项）：本次不建——CAS
  expected_revision 已覆盖收藏写的消歧需求；作为传输契约的未来扩展位预留。

## 3. 数据模型（gateway SQLite）——round 6 起 CONFIRMED，未改动

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

- 真源 `thread_favorites`；不写 record body、不在 `recent_threads` 加列。
- meta 初始化 `ensure_thread_favorites_meta_row`（镜像 mod.rs:3529），早于
  retired-workflow 启动清理（先例 mod.rs:2669）；重开幂等。
- revision 全局单调，兼任快照排序与写侧 CAS；**条件写接受即恒 bump（含
  no-op）**；清理点删除维持变更时 bump（镜像 mod.rs:2924）。
- 守卫式单事务插入（单 writer mutex mod.rs:363）：CAS 校验、存在守卫、写入、
  bump、回读整页同一写事务。
- 不用 FK 级联（级联不经 bump 破坏 revision 契约）。
- 清理点三处：归档（mod.rs:669 附近）、通用删除（mod.rs:2045 附近）、启动
  purge（mod.rs:2785，入口 :2698）。
- 契约合规：条件查询全 SQL（JOIN），无 `list_keys`/记录体扫描。

## 4. Gateway API

### 4.1 收藏读写（写侧 CAS）——round 6 起 CONFIRMED，未改动

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids, favorites, revision }` |
| `PUT /api/thread-favorites/{key}?expected_revision=N` | 200 整页（接受即 bump 含 no-op）；失配 409 同快照整页；404 / 400 |
| `DELETE /api/thread-favorites/{key}?expected_revision=N` | 同上（`removed` 字段） |

- 同快照组页不变量（200/409 页同事务回读，对齐 mod.rs:538/598）；
  `FavoriteThreadResult::{Updated | Conflict | NotFound}` → 200/409/404；
  `ensure_existing_thread_id` = 规范 key 校验 + 存在 point-check 友好 404 短路。

### 4.2 `/api/recent-threads` keyset cursor 分页（R3，全滤镜）

**offset 分页整体退役**（同仓同发无兼容包袱，先例 = 仓库「不考虑旧网关不做
兼容设计」裁决）：

- 参数：`limit` + 可选 `cursor`（不带 = 首页）。`tasks`/`favorites` 滤镜参数
  与互斥规则不变（All → `tasks=include`；Chats → `tasks=exclude`；Favorites →
  `favorites=only`）。
- **cursor = opaque token**（base64url(JSON)），内容：
  `{ v: 1, filter, last_active_at, thread_id, favorites_revision? }`。
  `filter` 绑定签发滤镜（跨滤镜使用 → 400）；`favorites_revision` 仅
  `favorites=only` cursor 携带。
- keyset 谓词（排序键与现有索引 `(last_active_at DESC, thread_id ASC)` 一致）：
  `WHERE <filter> AND (last_active_at < :t OR (last_active_at = :t AND
  thread_id > :id))`。
- 响应：`{ threads, next_cursor: string|null, total, has_more }`；`total` 仍为
  滤镜域全量 count（page+count 一次读快照，沿用既有模式）。
- **漂移根治论证**：删除/归档行不再移位后续页——keyset 从「最后一行的键」
  继续，跳行类缺陷（PagerTests:456 证明的 6 removals + overlap 5 漏行）对
  All/Chats/Favorites 一并消失。行因新活动被 bump 到 head → 后续页不重复
  出现，由 head refresh + 现有按 id dedup 收敛（与今日行为一致，非回归）。
- **favorites cursor 的 revision 校验**：load-more 时服务端在同一读事务内比对
  cursor.favorites_revision 与当前 `favorites_revision`；**失配 → 200
  `{ reset: true, threads: <全新首页>, next_cursor, revision, total,
  has_more }`**（服务端直接回全新首页，客户端原子替换整个 feed——省一次
  RTT，客户端无需自行判断域漂移）。匹配 → 正常下一页。
- **内部消费者**：bot `/threads`/`/bindthread` 的 `RecentThreadPageReader`
  （garyx-router/src/recent_threads.rs:35）翻页语义改为携带 opaque cursor 的
  prev/next token（reader trait 的 page token 由 cursor 序列化承担）；gateway
  内部 db 函数只保留 keyset 形态。

### 4.3 组合快照端点（R2）

`GET /api/thread-favorites/snapshot?limit=N`

- **单一读事务**内组装并返回：

```json
{
  "revision": 7,
  "thread_ids": ["…"],
  "favorites": [{ "thread_id": "…", "favorited_at": "…" }],
  "recent": { "threads": […], "next_cursor": "…", "total": 3, "has_more": false }
}
```

- `recent` = `favorites=only` 滤镜的首页（含按同快照 revision 签发的
  cursor）。**同快照由构造保证**：membership、revision、recent 页出自同一
  事务，round7-2 的双快照错位类缺陷结构性不存在。
- 这是 Favorites feed 的 replacement 原语（§7.4）；membership-only 的轮询
  仍走轻量 `GET /api/thread-favorites`。

### 4.4 已知边界

- automation generated / hidden 线程不进投影 → 不出现在 Favorites tab。
- 收藏的线程被归档：归档事务清行 + bump；favorites cursor 随之 stale →
  服务端强制 reset，无客户端记账。

## 5. 传输契约重构（R1，iOS + desktop 共同）

- **删除 `idempotent: Bool` 隐式重试语义**（iOS `GaryxGatewayClient` :950 的
  默认三次重试 + :205 establishment classifier 的组合对副作用写是错误语义，
  :2146 钉住的自动重试行为即将废止）。替代为显式请求语义：
  - `readRetryable`：无副作用读（GET），可自动重试（沿用现有 establishment
    classifier 与次数）；
  - `mutationSingleAttempt`：一切副作用写（PUT/POST/DELETE），**恰一次
    attempt**；结果分类 `ok(body) / definitiveHttpError(status, body) /
    ambiguous / notSent`——**session task 创建后的一切 transport error
    （NetworkConnectionLost、超时、截断、2xx 解码失败）恒 ambiguous**；
    `notSent` 仅当可证明 task 未创建（URL 构造失败、无网关配置等）。
- **迁移面**：iOS `GaryxGatewayClient` 全部 mutation 调用点显式标注语义
  （行为对齐现状的机械迁移；favorites 写、pins reorder（本就 maxAttempts:1）、
  Favorites 域 Archive/Delete 使用 `mutationSingleAttempt` 语义结果）；
  desktop main `garyx-client` 同构（fetch 封装禁默认重试，结果四分类进 IPC
  契约）。既有调用点的行为变更逐个列入实现 PR 描述；重试语义收紧引起的 UX
  变化（mutation 不再自动二次尝试）按「重试所有权归调用方状态机」原则接受。
- 契约测试：attempt 计数 mock；NetworkConnectionLost → ambiguous；
  分类穷尽 switch。

## 6. Desktop（Electron）

### 6.1 契约与主进程

- main 发布 `favoritedThreadIds` + `favoritesRevision` = **纯 raw**（服务端页：
  周期 fetch + 写/409 响应页 + snapshot），revision 单调接受；写 in-flight
  期间发布值不变（契约测试）。按 `entitiesGatewayUrl` 归一化 key
  （对齐 store.ts:498）。
- 判别联合 `RecentThreadListFilter = "all" | "chats" | "favorites"`；wire 映射
  main 层纯函数；类型排除 `tasks`+`favorites` 并存。
- `garyx-client/threads.ts`：`fetchThreadFavorites()`、
  `fetchThreadFavoritesSnapshot(limit)`、
  `setRemoteThreadFavorited(threadId, favorited, expectedRevision)`（四分类
  结果）、`fetchRecentThreads(filter, limit, cursor)`（keyset + reset 处理）。
- IPC 全部带 scope stamp；`setThreadFavorited` 纯转发。

### 6.2 Renderer

- **favorites-ingress**：实现 §7 意图机。呈现值 = main raw ⊕ intents。
- **入口 1**：`ConversationHeaderTitle.tsx` dropdown 紧邻 Pin 项，
  "Favorite conversation"/"Unfavorite conversation"（lucide `Star`/`StarOff`），
  不设快捷键。
- **入口 2**：Favorites tab 行内取消收藏；共享 `ThreadRailRow` 行 action 契约
  扩展为可组合 accessory，Favorites 行 `[Unfavorite, Archive]`，其它 tab 不变。
- **筛选 tab**：`recent-thread-feeds.ts` 扩展
  `RecentThreadFilter = "all" | "nonTask" | "favorites"`，三条独立 feed 全部
  改 keyset cursor 分页（`nextCursor` 取代 offset/overlap 字段）；
  `RecentConversationSidebar` 第三 tab + 方向键；sidebar-model 空态
  "No favorite threads"。
- **Favorites feed 协议**（§7.4）：refresh = snapshot 端点原子替换；
  load-more = cursor；`reset: true` 响应 → 原子替换整 feed。tombstone 后置
  过滤。无 marker/gate。
- i18n 四条新文案。

## 7. 客户端收藏状态机规格（双端共同契约，v9 精简版）

### 7.1 全局状态与身份围栏

- `raw`：最近一次按 revision 单调接受的整页（GET / 写响应 / 409 页 /
  snapshot）。接受条件 `page.revision >= highestObservedRevision` 且 scope
  围栏匹配。
- 写派发前置：首次成功获取 raw 前不派发写；意图排队，raw 就绪后 drain。
- flight 身份 = `(gatewayScope, runtimeEpoch, requestToken)`；token allocator
  进程内单调、永不随清场重置。
- transport 契约 = §5（每 dispatch 恰一次 attempt；四分类）。
- 呈现值 `presented(id) = latestDesired[id]?.desired ?? raw.contains(id)`。

### 7.2 意图 reducer（单一总定义；退休唯一入口 = 本节转移）

```
inFlight      = { requestToken, target, flightGeneration, expectedRevision }
latestDesired = { generation, desired, phase: active | awaitVerify(fence) }
```

**事件与转移（穷尽）：**

- `toggle(desired)`：generation += 1；latestDesired = {generation, desired,
  active}（覆盖任何前态）；无 inFlight 且 raw 就绪 → dispatch。
- `dispatch`：inFlight = {新 token, latestDesired.desired,
  latestDesired.generation, E = raw.revision}；发 CAS 写
  （mutationSingleAttempt）。per-thread single-flight。
- `settle(ok, page)`：接受页入 raw。`generation <= flightGeneration` → 退休；
  否则 `desired != raw.contains(id)` → active，drain 重派；相等 → 退休。
- `settle(conflict, page)`（单 attempt 恒真未应用）：接受 409 页入 raw；
  `desired != raw.contains(id)` → active，drain 重派（新 E）；相等 → 退休。
- `settle(notFound)`：退休（线程已不存在；行随 snapshot 消失）。
- `settle(ambiguous)`：意图（无论 generation 新旧）→
  `awaitVerify(fence = flight.expectedRevision)`，呈现保持 desired，
  发出**退避 effect**（下述）。
- `settle(notSent)`（round8-1 补分支）：确定性未发出，服务端无任何副作用；
  意图保持 active，发出退避 effect（重派沿用当前 raw.revision）。
- `rawAccepted(page)`（GET/snapshot/他 flight 的页）：更新 raw fallback。
  **本事件不是退休入口**，但触发 awaitVerify 意图的 **verify 转移**：
  `page.revision > fence` 时——`raw.contains(id) == desired` → 退休；
  不等 → active，drain 重派（新 E）。（v8「GET 永不退休」与被动路径的表述
  矛盾在此统一：退休发生在 verify 转移里，rawAccepted 只是它的触发器之一。）
- `backoffFired(stamp)`：退避 effect 到期回调。stamp =
  `(gatewayScope, runtimeEpoch, generation, fence)`，与当前状态**全匹配**才
  生效（用户已再 toggle → generation 不匹配 → 丢弃；切网关 → scope/epoch 不
  匹配 → 丢弃；fence 已过 → verify 转移已处理 → 丢弃）：awaitVerify → 重派
  同 desired 探针（E = 当前 raw.revision，新 token）；active（notSent 后）→
  重派。**退避 effect 由 Core 产生并校验，App/renderer 只做定时器宿主**
  （round8-5 归属裁决）。
- `gatewayScopeCleared`：全清（raw、intents、水位、epoch bump）。

**性质（测试断言）：**

- 消歧探针与任何同 `expectedRevision` 的 orphan 至多一个被接受（CAS）；
  探针链的不同 E 提交都是「重申当前 desired」的有意提交，无双重应用语义
  （round8-5 措辞修正：唯一性限定同 E）。
- 活性：orphan 从未送达且无其它写时，探针最终接受并推 revision 过 fence；
  持续网络故障时意图存续、呈现保持 desired——诚实降级。
- 终态恒 = 最后用户意图（R5 双序、R7 丢失 409、R8 探针 notSent 全走查）。

### 7.3 Favorites feed 的 tombstone（呈现层，唯一保留的 feed 侧状态）

- **hidden-pending**：`latestDesired.desired == false` 且意图存活（含
  awaitVerify）→ 行隐藏（对缓存行与 in-flight 响应行一律后置过滤）。
- **hidden-tombstone**：DELETE 成功结算后转入；退休条件 = settle 后签发的
  成功 replacement（snapshot 原子替换，新页天然不含该行）。
- 确定性失败退休（conflict 收敛为不需删 / notFound）→ 解除隐藏；
  重新收藏 → 新意图覆盖，行重现。

### 7.4 Favorites feed 协议（v9 精简：服务端一致性，无 marker/gate）

- **refresh / replacement**：调 snapshot 端点（§4.3）→ 单快照页**原子替换**
  整个 feed（display IDs 在响应到达前保留——无 skeleton；失败保留缓存）。
  触发源：写确认、周期 10s、下拉、切入 tab。
- **load-more**：`cursor` 翻页。`reset: true` 响应 → 原子替换整 feed（服务端
  已判定域漂移并回全新首页）。keyset 保证删除不跳行；revision 校验保证域
  变更必 reset——**进程重启/切网关回切/他端写/延迟提交全部被服务端裁决
  覆盖**（round8-4 类时序：新进程 prime 后旧 blocking task 提交 → revision
  bump → 下次 load-more 即 reset）。
- **并发纪律**：replacement 开始时 epoch bump 废弃在途 load-more（响应判弃）；
  replacement in-flight 期间新 load-more 延后到 settle。无分页 gate 概念。
- All/Chats feed：改 keyset cursor（R3 迁移），其余语义（refresh 合并、
  dedup）保持现状。

## 8. iOS 落点

- **`GaryxFavoritesState`**（Core，新文件）：§7 reducer + 退避 effect 产生/
  校验。SwiftPM 全覆盖测试。
- **transport**：`GaryxGatewayClient` 按 §5 重构请求语义（全 mutation 调用点
  显式标注）；`setThreadFavorited(id:favorited:expectedRevision:)` 四分类；
  `listThreadFavorites()`、`fetchThreadFavoritesSnapshot(limit:)`；
  `listRecentThreads(filter:limit:cursor:)`（keyset + reset）。
- **`GaryxRecentThreadFeeds` / `GaryxHomeThreadListPager`**：分页状态从
  offset/overlap 迁移到 `nextCursor`（All/Chats/Favorites 三 feed 一致）；
  Favorites feed 按 §7.4 协议（snapshot 替换 + reset 处理）。pager 的
  epoch/mutation-sequence 骨架保留。
- `GaryxRecentThreadFilter`：`.favorites` case；`displayName`/
  `activeStatusLabel = "Favorites"`；`homeMenuOptions`；穷尽测试。
  `GaryxRecentThreadFilterStorage` 持久化新 case。
- `HomeProjectionReducer`/`Actor`：`favoritesChanged(...)`；Presentation 传
  收藏集合供长按菜单判态。
- **App 层**：入口 1 长按菜单（`GaryxMobileSidebarViews.swift`
  `.garyxThreadActionMenu` 紧邻 Pin）；入口 2 线程内 title 菜单
  （`GaryxMobileConversationViews.swift` :942 附近紧邻 Pin）；入口 3 过滤器
  自动带出。`+ThreadPersistence.swift` IO 薄层（runtime UUID 围栏对齐 pin
  :92）；`+ThreadList.swift` refresh 增拉 favorites、All feed 辅助刷新穷尽
  switch（扩展 RefreshCommitTests :523）；`+Gateway.swift`（:55）切换全清。
  Archive/Delete 无需 favorites 侧特殊接入（R3 已根治）。
- 新 Core 文件跑 `xcodegen generate` 并提交 pbxproj；验证走 `xcodebuild`。

## 9. 测试计划

**Gateway**

- CAS/meta/清理点/守卫插入/同快照组页：沿用 v8 计划（全部 CONFIRMED 项）。
- **keyset（R3）**：删除 N>overlap 行后 load-more 不跳行（对照今日 offset
  会跳的既有行为）；cursor 跨滤镜使用 400；非法 cursor 400；`total`/
  `has_more` 域语义；page+count 一次读快照；bump 行为（行升 head 后不在
  后续页重复）。
- **favorites cursor reset**：revision 失配 → 200 reset + 全新首页（同一读
  事务）；匹配 → 正常翻页。「snapshot/首页签发 → 域写提交 → load-more」
  必 reset（含延迟提交时序）。
- **snapshot（R2）**：membership、revision、recent 页同事务同快照
  （commit-between 测试：两请求间提交写，snapshot 内部无错位）。
- routes：favorites 写 200/409/404/400；滤镜互斥；`tasks=only` 回归。

**传输契约（R1，双端）**

- attempt 计数（mutation 恰一次）；NetworkConnectionLost/超时/截断/2xx 解码
  失败 → ambiguous；notSent 仅 task 未创建路径；分类穷尽；既有 mutation
  调用点语义标注核对表。

**意图 reducer（双端，逐条对照 §7.2）**

- settle 六分支矩阵（ok/conflict/notFound/ambiguous/notSent × generation
  新旧）；R5 双序、R7 丢失 409、R8 探针 notSent 走查；verify 转移
  （fence 过后等→退休、不等→重派）；backoff stamp 四元不匹配丢弃
  （toggle 反向后旧 timer 不得发旧 desired）；探针同 E 唯一提交；活性
  （探针最终推过 fence）；不同 ID 隔离；desktop raw 纯度；切网关全清。

**Feed（双端）**

- snapshot 原子替换（期间缓存保留无 skeleton；失败保缓存）；reset 响应原子
  替换；replacement 废弃在途 load-more（两种完成顺序）；tombstone
  （DELETE 成功 → replacement 前行隐藏；确定性失败行重现；重新收藏行重现；
  in-flight load-more 不复活隐藏行）；**进程重启模拟**（新 feed prime 后域
  写提交 → 下次 load-more reset，不跳行）。
- All/Chats keyset 迁移回归：现有 feed 测试改写后全绿；跳行缺陷的回归测试
  （旧 offset 行为的对照用例转为 keyset 断言）。

**其余**

- desktop：三 tab、方向键、空态、行 accessory；判别联合映射；store 按
  gateway key 隔离。iOS：FilterStorage 往返；Reducer/Actor/Presentation；
  RefreshCommitTests。端到端：curl 三端点 + snapshot + cursor 翻页 + reset
  路径；双端 UI 按 `garyx-product-ui` 走查两处入口 + 筛选切换 + 行内取消。

## 10. 实现切分（影响面按 R1/R2/R3 扩大，分四步提交）

1. **gateway**：favorites 表/CAS/API + keyset cursor 迁移（含 bot reader）+
   snapshot 端点 + 全部服务端测试。
2. **传输契约**：iOS `GaryxGatewayClient` + desktop `garyx-client` 请求语义
   重构（机械迁移 + 契约测试）。
3. **双端状态机与 feed**：`GaryxFavoritesState` / `favorites-ingress` +
   三 feed cursor 迁移 + Favorites feed 协议（先测后 UI）。
4. **UI**：desktop renderer / iOS App 入口与 tab + xcodegen。

每步独立可验证；步骤 1 的 keyset 迁移与步骤 3 的客户端迁移之间存在部署耦合
（同仓同发，无跨版本兼容需求）。
