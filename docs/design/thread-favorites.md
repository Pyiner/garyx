# Thread Favorites (线程收藏)

Status: draft v8 (addressing review round 7 — #TASK-2324 FAIL, 2×P0 + 3×P1)
Date: 2026-07-16

## 0. 修订记录

### Round 7 findings → v8 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | **P0** 纠偏 DELETE 的 409 响应也丢失（模糊）→ 同代退休规则吞掉最后意图，replacement 又按 R1>R0 清 marker，终态反转；且若模糊 attempt 从未送达、无其它写，revision 恒 =E → CAS marker 永不可清，gate 永闭（活性死锁） | **模糊失败永不静默退休意图**：意图转入 **`awaitVerify(fence = flight.expectedRevision)`** 态，呈现保持 desired；解决路径二选一：(a) 观察到任一 accepted raw 的 `revision > fence` → 对比 raw 与 desired，相等退休、不等立即重派；(b) **带退避的主动重派**（重试同一幂等 CAS 写、同 expectedRevision）——这既是消歧探针又是活性泵：200 则应用并 bump（fence 过）、409 则拿新页对比重派、再模糊则退避续试。CAS 恒 bump 保证探针与 orphan 至多一个提交、另一个 409，全部收敛到最后意图（§7.2） |
| 2 | **P0** 「并行配对拉取」非配对快照：recent GET 的 WAL 快照可早于 favorites GET（独立 reader pool 各持快照，mod.rs:363/968/6252），R_obs > E 清 marker 时换入的 recent 页可能是旧快照 → 已取消行复活/新收藏缺失/跨端漂移跳行 | **配对改串行**：favorites GET **完成后**才派 recent GET——单库单 writer 下，后建立的 recent 快照 ⊇ favorites 快照，被清 marker 的效果必含于换入页。iOS `refreshThreads` 的 `async let` 并行模式对 Favorites replacement 路径改为顺序两段；desktop 同。补 commit-between-two-reads 测试（§7.4） |
| 3 | **P1** lifecycle retry 的 token 语义自相矛盾（§6.1 新 token vs §7.4 同 opToken；新 token 旧条目无退休路径，复用 token 又允许后续 retry 的「确定性未应用」撤销先前 ambiguous 记录） | 定义 **operation-group**：每个用户发起的 Archive/Delete = 一个稳定 `opGroupToken`，组内每次显式重试 = 独立 request token（各自单 transport attempt）。marker 条目按 group 记账：**任一 attempt 达 success/already-gone → settle 整组**（删除域幂等，早先 straggler 提交是域上 no-op）；**组内只要出现过 ambiguous attempt，后续 attempt 的确定性未应用不得撤销该组条目**；撤销仅当组内全部 attempt 都可证明未发出（§7.4） |
| 4 | **P1** 派发前关 gate 不废弃已签发的 load-more ticket：pager completion 只查 epoch/localMutation（Pager:271，PagerTests:568 证明须 noteLocalMutation 才弃在途），旧 offset 页可在 marker 建立后、replacement 开始前落地 | **入标记 = 关 gate + 立即废弃在途分页**（Favorites feed epoch/local-mutation bump 在 marker 建立时执行，不等 replacement 开始）；补「ticket 先签发 → marker 后建立 → ticket 最后完成 → 结果被弃、cursor 不动」测试（§7.4） |
| 5 | **P1** 现有 transport classifier 把 `NSURLErrorNetworkConnectionLost` 归为「未到服务端」（:205，测试 :2146 钉其自动重试语义）；若 lifecycle 编排沿用，会把「已提交、回程丢失」映射成「未发出」而撤 marker | **副作用写专用保守分类器**：凡 transport 调用**已发起**之后的一切错误（network lost / 超时 / 截断 / 2xx 解码失败）一律 ambiguous；仅**可证明在发起前失败**（URL 构造、无网关配置等，session task 未创建）才算 not-sent、才允许撤 marker。现有 classifier 不得用于撤销判定；补 NetworkConnectionLost → ambiguous → marker 保留测试（§7.2、§7.4） |

round 7 已确认：单 attempt / 派发前 marker / success·already-gone 屏障方向正确；
`R_obs > E` 作为「CAS orphan 命运已封」判据本身成立。

### 历史轮次（要点）

- **R6→v7**：单 transport attempt；marker 派发前建立；marker 结算语义
  （CAS 按 R_obs > E、lifecycle 按 settled）；配对 replacement。
- **R5→v6**：服务端 CAS 写围栏（`expected_revision` 必填、接受即恒 bump 含
  no-op、409 + 同快照整页）；漂移标记扩容至模糊失败与 Archive/Delete。
- **R4→v5**：mustWrite 幂等消歧；`(gatewayScope, runtimeEpoch, requestToken)`
  围栏；未收敛变更时 gate 关闭。
- **R3→v4**：flight/desired 双身份；main 只发布纯 raw；分级 tombstone；
  replacement 废弃两 track + 关 gate。
- **R2→v3**：meta singleton（早于 purge）；同快照组页；`FavoriteThreadResult`
  枚举 + 404；行 accessory 契约；清理点三处（mod.rs:669/2045/2785）。
- **R1→v2**：守卫式单事务插入；suppression；All feed 穷尽 switch；gateway
  切换清理；判别联合；Mac 行内取消。

评审已确认的全局裁决：独立 `thread_favorites` 表 + SQL JOIN 合规；
`favorites=only` 与显式 `tasks` 互斥、Favorites 含 task 线程语义正确；
Core/App 分层与双端入口正确；revision+单 writer 构成正确 CAS 总序。

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
- 双端筛选器各加 Favorites 类别；Favorites feed 用串行配对 replacement（§7.4）。
- 跨端收敛：客户端统一收藏意图状态机（§7）+ revision 单调接受 + 写侧 CAS 围栏。

**非目标**

- 收藏排序 / 拖拽重排（不引入 sort_order、reorder outbox）。
- 首页独立「收藏段」；All/Chats 行星标徽标（Favorites tab 行内取消按钮除外）。
- SSE 推送收藏变更（沿用刷新收敛）。
- bot 命令面不加收藏筛选。
- 收藏意图/marker 跨进程持久化（瞬态 UI 状态；进程退出回落 raw，feed 从头
  prime，无旧 cursor 可漂移）。

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

- **真源**：`thread_favorites` 表。不写 `thread_records` body、不在
  `recent_threads` 加列。
- **meta 初始化**：`ensure_thread_favorites_meta_row`（`INSERT (1,0) ON
  CONFLICT DO NOTHING`，镜像 mod.rs:3529），早于 retired-workflow 启动清理
  （先例 mod.rs:2669）。重开幂等。
- **revision**：全局单调，兼任快照排序与写侧 CAS 围栏（先例 mod.rs:590）。
  **条件写被接受即恒 bump（含 no-op）**；清理点删除维持变更时 bump
  （镜像 mod.rs:2924）。
- **守卫式单事务插入**（单 writer mutex mod.rs:363）：CAS 校验、存在守卫、
  写入、bump、回读整页同一写事务：

```sql
INSERT INTO thread_favorites (thread_id, favorited_at)
SELECT ?1, ?2
WHERE EXISTS (SELECT 1 FROM thread_records WHERE key = ?1)
ON CONFLICT(thread_id) DO NOTHING;
```

- **不用 FK 级联**（级联不经 bump 破坏 revision 契约；与 thread_pins 模式一致）。
- **清理点三处**（同事务删行 + 变更时 bump）：归档（mod.rs:669 附近）、通用
  删除（mod.rs:2045 附近）、启动 purge（mod.rs:2785，入口 :2698）。
- 契约合规：条件查询全 SQL（JOIN），无 `list_keys`/记录体扫描。

## 4. Gateway API——round 6 起 CONFIRMED，未改动

### 4.1 收藏读写（写侧 CAS）

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids, favorites, revision }`，按 `favorited_at DESC, thread_id ASC` |
| `PUT /api/thread-favorites/{key}?expected_revision=N` | 200 `{ favorited: true, thread_ids, favorites, revision }`（接受即 bump 含 no-op）；失配 **409** `{ conflict: true, thread_ids, favorites, revision }`（同快照整页）；404 / 400 |
| `DELETE /api/thread-favorites/{key}?expected_revision=N` | 200 `{ favorited: false, removed, thread_id, thread_ids, favorites, revision }`（接受即 bump 含 no-op）；409 / 404 / 400 同上 |

- `expected_revision` 必填；同快照组页不变量（200 与 409 页均同事务回读，
  对齐 mod.rs:538/598；交错测试对齐 mod.rs:5462）。
- db 层 `FavoriteThreadResult::{Updated(page) | Conflict(page) | NotFound}`；
  route 映射 200/409/404。`ensure_existing_thread_id` = 规范 key 校验 + 存在
  point-check 友好 404 短路（routes.rs:1104）。
- 路由注册 `route_graph.rs`，handler 紧邻 thread-pins。

### 4.2 `/api/recent-threads` favorites 过滤（round 2 起 CONFIRMED）

- 新可选参数 `favorites`，唯一合法值 `"only"`，其它 400；与显式 `tasks` 同传
  400。映射：All → `tasks=include`；Chats → `tasks=exclude`；Favorites →
  `favorites=only`。`tasks=only` 回归（routes/tests.rs:3896）不动。
- **Favorites 含 task 线程**。
- SQL：`recent_threads r JOIN thread_favorites f ON f.thread_id = r.thread_id
  ORDER BY r.last_active_at DESC, r.thread_id ASC LIMIT ? OFFSET ?`；count 同
  过滤域；page+count 一次读快照。
- 返回体不变；不加 per-row favorited 字段。

### 4.3 已知边界

- automation generated / hidden 线程不进投影 → 不出现在 Favorites tab。
- 收藏的线程被归档：归档事务清行 + bump，与 pin 一致。

## 5. Desktop（Electron）

### 5.1 契约与主进程

裁决：favorites 乐观状态机唯一 owner = renderer ingress；main 不做 pending
投影（与 pin 的 main controller 有意分道：那是 reorder outbox 持久化需求）。

- main 发布 `favoritedThreadIds` + `favoritesRevision` = **纯 raw**（仅服务端
  页），revision 单调接受；写 in-flight 期间发布值不变（契约测试钉死）。按
  `entitiesGatewayUrl` 归一化 key（对齐 store.ts:498）。
- **transport 单 attempt**：`setRemoteThreadFavorited` 禁止任何自动重试；
  契约测试计数。**副作用写分类器（对 round7-5）**：transport 调用已发起后的
  一切错误 → ambiguous；仅可证明未发起 → not-sent。
- IPC：`setThreadFavorited(threadId, favorited, expectedRevision)` →
  `{status: ok|conflict|notFound, page, scopeStamp}`；`fetchThreadFavorites`
  带 scope stamp。
- 契约：判别联合 `RecentThreadListFilter = "all" | "chats" | "favorites"`；
  wire 映射 main 层纯函数；类型排除并存。新增 `DesktopThreadFavoritesPage`。
- `garyx-client/threads.ts`：`fetchThreadFavorites()`、
  `setRemoteThreadFavorited(...)`（三分结果，对齐 `reorderRemoteThreadPins`）。

### 5.2 Renderer

- **favorites-ingress**：实现 §7。呈现值 = main raw ⊕ ingress intents。
- **入口 1**：`ConversationHeaderTitle.tsx` dropdown 紧邻 Pin 项，
  "Favorite conversation"/"Unfavorite conversation"（lucide `Star`/`StarOff`）。
- **入口 2**：Favorites tab 行内取消收藏；共享 `ThreadRailRow` 行 action 契约
  扩展为可组合 accessory，Favorites 行 `[Unfavorite, Archive]`，其它 tab 不变。
- **marker 接入**：CAS 写在 ingress dispatch 前建条目；Archive/Delete 在行
  action/菜单派发前判 `wasInFavoritesDomain` 建 operation-group 条目；三类
  结局从编排层回喂（§7.4）。**入标记即废弃在途分页 + 关 gate**（round7-4）。
- **筛选 tab**：`recent-thread-feeds.ts` 扩展
  `RecentThreadFilter = "all" | "nonTask" | "favorites"`，第三条独立 feed，
  label "Favorites"；`useRecentThreadFeeds`、`RecentConversationSidebar`
  （第三 tab + 方向键）、sidebar-model（空态 "No favorite threads"）。
- **Favorites feed**：串行配对 replacement + tombstone + gate 按 §7.3/7.4。
- i18n：四条新文案。

## 6. iOS

### 6.1 GaryxMobileCore

- **`GaryxFavoritesState`**：实现 §7（纯逻辑、SwiftPM 可测）。围栏对齐
  `GaryxPinnedOrderState`（stamp :13；两阶段 :227/:290；conflict 对齐 reorder
  409）。gateway 切换全清 + epoch bump。
- **transport 单 attempt**：`setThreadFavorited(...)` **`maxAttempts: 1`**
  （先例 reorder :360）；Favorites 域 Archive/Delete 同理，显式重试由编排层
  以同 opGroup 新 attempt 发起。**副作用写分类器（round7-5）**：不复用现有
  establishment classifier（:205 把 NetworkConnectionLost 归「未到服务端」，
  :2146 钉其重试语义——对副作用写是错误语义）；专用规则 = 发起后一切错误
  ambiguous，仅证明未发起才 not-sent。
- `GaryxRecentThreadFilter`：`.favorites` case；`displayName`/
  `activeStatusLabel = "Favorites"`；`homeMenuOptions`；结构化 query 描述
  （二选一），穷尽测试。
- `GaryxRecentThreadFeeds`：第三条 feed；串行配对 replacement/tombstone/gate
  按 §7.3/7.4；**marker 建立时对 Favorites feed 执行 local-mutation/epoch
  bump 废弃在途 load-more**（round7-4；对齐 Pager:271 completion 语义与
  PagerTests:568 的 noteLocalMutation 先例）。不改 Pager 既有语义；不复用
  现有 `reset()`。
- `GaryxRecentThreadFilterStorage`：持久化新 case。
- `GaryxGatewayClient`：`listThreadFavorites()`、`setThreadFavorited(...)` 三分
  结果；`listRecentThreads` favorites 参数。
- `GaryxGatewayThreadModels`：`GaryxThreadFavoritesPage`（兼容解码 + revision）。
- `HomeProjectionReducer`/`Actor`：`favoritesChanged(...)`；Presentation 传
  收藏集合供长按菜单判态。

### 6.2 App 层

- **入口 1（长按）**：`GaryxMobileSidebarViews.swift` `.garyxThreadActionMenu`
  紧邻 Pin 项，"Favorite thread"/"Unfavorite thread"（`star`/`star.slash`）。
- **入口 2（线程内右上角）**：`GaryxMobileConversationViews.swift` title 菜单
  （:942 附近）紧邻 Pin 项。
- **入口 3（过滤器）**：`GaryxRecentThreadFilterMenu` 自动出现 Favorites。
- `+ThreadPersistence.swift`：IO 编排薄层，裁决全委托 `GaryxFavoritesState`；
  runtime UUID 围栏对齐 pin（:92）。
- **Archive/Delete 编排接入**：请求派发前判 `wasInFavoritesDomain` → Core 建
  operation-group marker + 废弃在途分页 + 关 gate；do/catch 三类结局
  （成功/确定失败/模糊，按 §6.1 保守分类器）回喂 Core；模糊后编排层带退避
  显式重试（同 opGroup 新 attempt）至确定性结局。不挂成功专用删行路径
  （Bots.swift:270/:286 成功分支、:300 catch、ThreadLifecycle.swift:323）。
- `+ThreadList.swift`：`refreshThreads` 增拉 favorites；**Favorites
  replacement 路径为串行两段**（favorites GET → recent GET，§7.4），不沿用
  `async let` 并行；All feed 辅助刷新穷尽 switch（`.all` 只刷 All；其它 case
  刷自身 + All），扩展 RefreshCommitTests（:523）。
- **Gateway 切换清理**：`+Gateway.swift`（:55）清 `GaryxFavoritesState`
  全部状态（raw、intents、awaitVerify、marker、gate、epoch、revision 水位）
  与 Favorites feed。
- 新 Core 文件跑 `xcodegen generate` 并提交 pbxproj，验证走 `xcodebuild`。

## 7. 客户端收藏状态机规格（双端共同契约）

### 7.1 全局状态与身份围栏

- `raw`：最近一次按 revision 单调接受的服务端整页（`Set<threadId>` +
  `revision`）。来源：GET、写响应页、409 页。接受条件
  `page.revision >= highestObservedRevision` 且 scope 围栏匹配。
- **写派发前置**：首次成功 GET 前不派发写；意图排队，raw 就绪后 drain。
- **flight 身份 = `(gatewayScope, runtimeEpoch, requestToken)`**；token
  allocator 进程内单调、永不随清场重置。
- **transport 契约**：每次 dispatch 恰一次网络 attempt；重试都是状态机新
  dispatch（新 token、当前 raw.revision、新 marker 条目）。
- **副作用写结果分类（对 round7-5）**：`ok(page) / conflict(page) / notFound /
  ambiguous / notSent`。ambiguous = transport 发起后的一切错误（network
  lost、超时、截断、2xx 解码失败）；notSent = 可证明未发起（session task 未
  创建）。**现有 establishment classifier 不得用于此判定**。
- per-thread：`inFlight?` 与 `latestDesired?`（§7.2）。
- 呈现值 `presented(id) = latestDesired[id]?.desired ?? raw.contains(id)`。
- GET raw 接受永不退休意图。

### 7.2 每线程意图状态机（v8：awaitVerify 取代「同代模糊退休」）

```
inFlight      = { requestToken, target, flightGeneration, expectedRevision }
latestDesired = { generation, desired, phase: active | awaitVerify(fence) }
```

- **toggle(id, desired)**：`generation += 1`；`latestDesired = {generation,
  desired, phase: active}`（覆盖任何 awaitVerify 前态）；无 inFlight 且 raw
  就绪 → dispatch；有 inFlight 只更新 latestDesired。
- **dispatch(id)**：建 CAS marker 条目（E = raw.revision）→ inFlight →
  发条件写（单 attempt）。per-thread single-flight。
- **结算处理（四步）**：
  1. 结算 flight（三元组匹配；不匹配整体判弃）。
  2. 接受页（200/409 页按 revision 单调入 raw）。
  3. 意图裁决：
     - **ok（200）**：`generation <= flightGeneration` → 退休；`generation >`
       且 `desired != raw.contains(id)` → 保留 active 进 4；相等 → 退休。
     - **conflict（409，单 attempt 恒真未应用）**：撤销本条 CAS marker；按
       409 页 raw 裁决：不一致 → 保留 active 进 4（新 revision 重派）；
       一致 → 退休。
     - **notFound**：撤销本条 marker；退休（线程已不存在）。
     - **ambiguous（对 round7-1）**：CAS marker 保留；**意图永不静默退休**——
       无论 generation 新旧，存活意图转
       `phase = awaitVerify(fence = flight.expectedRevision)`，呈现保持
       desired，进入验证循环（下述）。
  4. drain：清 inFlight 后若存在 active 意图且（`desired !=
     raw.contains(id)` 或来自 conflict/mustWrite 路径）→ dispatch。
- **awaitVerify 验证循环（消歧 + 活性泵，对 round7-1 两个问题）**：
  - **被动路径**：观察到任一 accepted raw 满足 `revision > fence` →
    `raw.contains(id) == desired` → 退休；不等 → 转 active 立即重派
    （新 E = 当前 raw.revision）。
  - **主动路径**：带退避（首次 1–2s，此后对齐 10s 刷新节拍）**重派同一幂等
    CAS 写**（同 desired、E = 当前 raw.revision，新 token 新 marker 条目）。
    结局：200 → 应用且 bump（fence 已过，退休）；409 → 新页对比后退休或
    重派；再 ambiguous → 继续 awaitVerify 退避。CAS 恒 bump 保证探针与任何
    orphan 至多一个提交、其余 409——不存在双重应用。
  - **活性**：即使 orphan 从未送达、无其它写（revision 恒 =E），主动重派
    最终产生一次接受写将 revision 推过 E → 意图与 CAS marker（`R_obs > E`）
    双双可结；持续网络故障时意图/marker 存续、gate 关闭——诚实降级。
- **round7-1 反例走查**：PUT(g1,true,R0) 模糊；g2=false 纠偏
  DELETE(g2,false,R0)；orphan PUT 先提交 true@R1；DELETE 得 409 但响应丢失
  （ambiguous）→ **g2 不退休**，转 awaitVerify(fence=R0)。replacement 可按
  R1>R0 清两条 CAS marker（分页域已一致：换入页含该行）——但意图仍活。
  下一次 accepted raw（R1>R0=fence）：raw=true != desired=false → 转 active
  重派 DELETE(E=R1) → 200 → false@R2。终态 = 最后意图。✅
  若 DELETE 从未送达且无 orphan：主动重派最终 200 → false，revision 推过
  fence，全部收敛。✅
- 既往反例（R5 双序、R4 mustWrite、R3 双身份、R2 single-flight、R6 重试伪装
  409）全部保持封闭：单 attempt 下 409 恒真；awaitVerify 语义强于原 mustWrite
  （mustWrite 并入：conflict/ambiguous 后的重派一律不比较过时 raw）。

### 7.3 Favorites feed 的 unfavorite tombstone

- **hidden-pending**：`latestDesired.desired == false` 且意图存活（含
  awaitVerify）→ 行隐藏。确定性失败退休（409 收敛为不需删 / 404）→ 解除。
- **hidden-tombstone**：DELETE 成功结算后转入。唯一退休条件：settle 后签发的
  成功 replacement，与整页替换原子退休。失败 → 存续。
- **重新收藏打断**：tombstone/awaitVerify 期间再 Favorite → 新意图覆盖，
  行重现；PUT 成功后照常 replacement。
- 隐藏是后置过滤：对缓存行与任何 in-flight 响应行一律生效。

### 7.4 marker、串行配对 replacement 与分页 gate

**marker（`unreconciledDomainMutations`）：**

```
CAS 条目 = { threadId, expectedRevision: E }
  建立：dispatch 前（含 awaitVerify 主动重派的每次 dispatch）
  撤销：本 flight 确定性未应用（单 attempt 409 / 404 / notSent）
  可清除：成功 replacement 的配对观察 R_obs > E（恒 bump ⇒ orphan 命运已封）

lifecycle 条目 = { opGroupToken, threadId, settled: bool }（对 round7-3）
  operation-group：一次用户 Archive/Delete = 一个稳定 opGroupToken；组内每次
    显式重试 = 独立 request token（各自单 attempt）
  建立：组内首次 attempt 派发前（先判 wasInFavoritesDomain）
  settle 整组：组内任一 attempt 达 success 或 already-gone（删除域幂等：一旦
    确认线程已归档/删除，早先 straggler 的延迟提交是域上 no-op）
  撤销：仅当组内全部 attempt 均为 notSent（出现过任何 ambiguous 后，后续
    attempt 的确定性未应用不得撤销该组）
  unsettled：编排层带退避显式重试（同组新 attempt）至确定性结局；耗尽仍模糊
    → 条目存续（gate 关，缓存可见，仅禁 load-more）
  可清除：settled 之后签发的成功 replacement
```

- **入标记三连（对 round7-4）**：建条目 + **立即废弃在途分页**（Favorites
  feed epoch/local-mutation bump——对齐 PagerTests:568 的 noteLocalMutation
  先例，使已签发 ticket 的 completion 被弃、cursor 不动）+ 关 gate。不等
  replacement 开始。
- **gate 闭合条件：marker 非空 ⟺ gate 关闭**。
- **串行配对 replacement（对 round7-2）**：一次 replacement =
  **先 `GET /api/thread-favorites`（得配对观察 R_obs），成功后再
  `GET /api/recent-threads?favorites=only`**；两段都成功才算成功。时序论证：
  单库单 writer，recent 读事务在 favorites 响应**之后**建立快照 ⇒ recent
  快照 ⊇ favorites 快照 ⇒ 按 `R_obs > E` 清除的条目，其效果必含于换入页
  （favorites GET 前提交的写对后建立的 recent 快照可见；mod.rs:6252 的快照
  语义测试佐证）。并行 `async let` 模式对此路径禁用。
- **成功**：整页原子替换、cursor 重置新 head、逐条清满足条件的 marker
  （CAS 按 `R_obs > E`；lifecycle 按 settled-before-dispatch）、退休满足条件
  的 tombstone；仍有条目 → gate 保持关闭并驱动下一轮周期 replacement。
- **失败**（任一段失败）：不换页、不清 marker、tombstone 不动；marker 非空 →
  gate 关；为空 → 允许旧 cursor 重开（无本端漂移源；他端漂移与 All/Chats
  既有容忍度一致）。
- 两种完成顺序无害（epoch 判弃 / 整页覆盖）。
- 收藏集合小，串行两段成本可忽略；用户可见的只有行集原子变化。

**round6-3/round7 反例走查**：六个 Archive 超时（blocking 写等 writer）——
六个 lifecycle 条目派发前已建、在途 load-more 已废弃、gate 关。replacement
越过 pending writer 成功 → unsettled 条目不可清，gate 不开。延迟提交后编排层
重试拿 already-gone → settle 整组 → 下次成功串行配对 replacement（recent 快照
晚于 favorites 观察，含删除效果）清条目 → gate 开、cursor 已重置、无漏行。✅

## 8. 测试计划

**Gateway（`cargo test -p garyx-gateway --lib`）**

- meta：建库 `(1,0)`；重开幂等；初始化早于 purge。
- db（CAS）：接受写恒 bump（含 no-op）；失配 Conflict 同快照零副作用；孤儿写
  交错（旧 E 在后继接受写后提交 → 拒）；守卫插入交错；三清理点；同快照组页
  交错（mod.rs:5462 模式）；**commit-between-two-reads**（快照 A 建立 → 提交
  写 → 快照 B 建立：B 见效果、A 不见——支撑 §7.4 串行时序论证，对齐
  mod.rs:6252 模式）；`favorites=only` 过滤域分页；page+count 一次读快照。
- routes：200/409/404/400（409 带整页）；`favorites` 非法值 400；与显式
  `tasks` 同传 400；`tasks=only` 回归。

**双端状态机（desktop `npm run test:unit` / iOS SwiftPM，逐条对照 §7）**

- transport 单 attempt 计数契约；**副作用写分类器（round7-5）**：
  NetworkConnectionLost/超时/截断/2xx 解码失败 → ambiguous、marker 保留；
  仅 notSent 撤 marker。
- **awaitVerify（round7-1）**：
  (a) 纠偏 DELETE 的 409 丢失 → 意图不退休 → fence 过后 raw≠desired → 重派
  → 终态 = 最后意图；
  (b) 模糊 attempt 从未送达、无其它写 → 主动重派最终应用并推过 fence →
  意图与 CAS marker 双结、gate 重开（活性）；
  (c) fence 过后 raw==desired（orphan 实际已应用同向）→ 静默退休不空发；
  (d) awaitVerify 期间用户再 toggle → 新意图覆盖、循环终止于新意图。
- 同 ID 覆盖 + 旧 flight ok/ambiguous/409 全矩阵：终态恒 = 最后意图；R5 双序
  走查；不同 ID 隔离；网关切换围栏（token 不复用、旧页判弃）；desktop raw
  纯度；gateway 切换全清（含 awaitVerify/marker/gate）。

**marker/gate/replacement（双端）**

- **入标记三连（round7-4）**：ticket 先签发 → marker 建立（废弃在途）→
  ticket 完成 → 结果被弃、cursor 不动、gate 关。
- **lifecycle operation-group（round7-3）**：组内 attempt1 ambiguous →
  attempt2 notSent **不撤组**；attempt3 already-gone → settle 整组 → 其后
  成功 replacement 清；全 notSent 组 → 撤销。
- **串行配对（round7-2）**：favorites GET 与 recent GET 之间提交跨端写 →
  换入页含该写（不复活/不缺失）；任一段失败 → 整体失败不清 marker；
  「replacement 成功在先、延迟 mutation 提交在后」CAS 与 lifecycle 两类
  （R_obs == E 不清 / unsettled 不清 → 后续轮次收敛）。
- offset 漂移三源（确认取消 / 模糊取消 / Archive 含模糊）各 >overlap +
  replacement 全失败 → gate 关 load-more 拒；满足清除条件的成功 replacement
  后收敛重开。

**Favorites feed（双端）**

- DELETE 成功 → replacement 失败 → 行仍隐藏；成功后 tombstone 原子退休。
- 确定性失败 → 行重现；tombstone/awaitVerify 期间重新收藏 → 行重现。
- replacement vs load-more 两种完成顺序；在途拒新 load-more；replacement 期间
  缓存保留（10s 刷新无 skeleton）；乐观取消期间 in-flight load-more 不复活行。

**其余**

- desktop：三 tab、方向键、空态、Favorites 行 accessory；判别联合映射穷尽；
  store 按 gateway key 隔离。
- iOS：FilterStorage 往返；GatewayClient 三分结果 + 单 attempt；
  Reducer/Actor/Presentation；RefreshCommitTests；archive 编排三类结局回喂。
- 端到端：本地 gateway curl 三端点（含 409）+ `favorites=only`；双端 UI 按
  `garyx-product-ui` 走查两处入口 + 筛选切换 + 行内取消。

## 9. 实现切分建议

单 PR 可完成，按依赖顺序提交：① gateway（表 + meta + CAS + API + 过滤 + 三
清理点）→ ② 双端共享状态机（iOS Core `GaryxFavoritesState` / desktop
`favorites-ingress`，先测后 UI；含 awaitVerify、marker/gate、串行配对
replacement）→ ③ desktop renderer / iOS App UI + xcodegen（含 Archive/Delete
编排层接入）。每步各自带测试。
