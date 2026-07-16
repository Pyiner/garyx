# Thread Favorites (线程收藏)

Status: draft v10 (addressing review round 9 — 6×P1 + 3×P2)
Date: 2026-07-16

## 0. 修订记录

### Round 9 findings → v10 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | P1 All/Chats 保留式 head merge + keyset：>1 页新增/前移行落在「新首页尾端与旧 cursor 之间」永久漏行（keyset 只根治删除移位，不管插入） | **refresh 改 gap-fill 链式拉取**：head 刷新从首页开始 cursor 链式续拉，直到页内出现已加载集合中的 thread_id（重叠命中）或 has_more=false；上限 K=5 页，超限 → 整 feed replacement + cursor 重置。保留尾部的既有 UX 不变（重叠命中即停）。All/Chats/Favorites 统一（Favorites 本就 snapshot 整替）（§7.4） |
| 2 | P1 reducer 非穷尽：R1 会产生任意 HTTP error（400/401/403/408/429/5xx、代理响应），五分支没有归宿；泛化 5xx 不能视为确定未应用 | 结算矩阵补全为**七分支**：`ok / conflict / notFound / definitiveRejected(retryable) / ambiguous / notSent`。**只有成功解码的本端点结构化响应才可判定确定性**：解码 409→conflict、解码 404→notFound、解码 400/401/403→definitiveRejected(非重试：退休+报错)、解码 429/503（结构化）→definitiveRejected(可重试：保持意图+退避)；**一切未解码/意外状态（裸 5xx、代理垃圾、截断）→ ambiguous**（§7.2） |
| 3 | P1 notSent 保持 active 但 timer stamp 带 fence，active 无 fence 可匹配 → 意图永久搁浅；probe notSent 丢原 ambiguity fence | 意图相位扩为 **`active \| retryScheduled(effectToken, cause, verificationFence?) \| awaitVerify(fence)`**：notSent（含 probe notSent）→ retryScheduled，probe 场景携带原 fence；timer stamp 匹配键改为 **effectToken**（每 effect 唯一，无表示问题）；**rawAccepted 对一切无 inFlight 的意图触发 drain/verify 检查**（active 搁浅路径消除）（§7.2） |
| 4 | P1 409/404 收敛退休意图 + 解除隐藏，但 feed 缓存在 snapshot 前仍含该行 → 权威已 false 的行复活 | §7.3 整体塌缩为**单一权威后置过滤：Favorites feed 行仅当 `presented(id) == true` 才渲染**（presented = intents ⊕ raw）。DELETE 成功、409/404 收敛、他端删除全部自然隐藏（raw 已不含）；hidden-pending/tombstone 两态机制删除（§7.3） |
| 5 | P1 bot `RecentThreadPageReader` 的 `/threads N`/next/prev 依赖任意 offset 两次求页，forward-only cursor 无法承载 | 裁决：**HTTP API cursor-only；bot reader 保留内部 offset 形态**——`list_recent_threads_page(filter, limit, offset)` 降为 internal-only（仅 `SqlRecentThreadPageReader` 调用，行为与今日完全一致，漂移容忍度=今日）。bot 命令 UX 不动。备选（cursor 栈 + 顺序 walk）记录在案不采纳——bot 面低风险，不值当重造 UX（§4.2） |
| 6 | P1 删除 operation-group 后 Archive/Delete 的 ambiguity 无 owner：现有 catch 只报错，用户操作可能已提交/未提交无人裁决 | 定义**编排层 reconcile 政策**：ambiguous archive/delete → 立即**点查线程存在性**（thread meta point GET，readRetryable）——已消失 → 按成功收尾（本地删行 + 触发 feed refresh）；仍存在 → 按失败报错（**人工重试政策**，与今日 UX 一致）；点查也失败 → 报错 + 触发 refresh（分页安全由 keyset/reset 兜底，不需要 gate）（§6.2、§8） |
| 7 | P2 R1 漏 PATCH 与 garyx-client 目录外调用点 | R1 明确：semantic mode 为**全部请求 helper 的必填参数**（GET/PUT/POST/DELETE/**PATCH**，无默认值，编译期强制）；迁移清单含 iOS PATCH helper（:938/:1054）与 desktop `garyx-client` 外调用点（如 agent-avatar.ts:71），实现 PR 附 grep 全量清单（§5） |
| 8 | P2 keyset 的 has_more/next_cursor 推导与 limit=0 未定义 | 定义：**LIMIT N+1 探测**——取 N+1 行返回前 N，`has_more = 取到第 N+1 行`，`next_cursor` 从最后一条实际返回行签发（`!has_more` → null）；`total` 保留为同快照独立 count（仅展示用，不参与 has_more）。**`limit` 必须 ≥1 且 ≤ 上限（沿现有 cap），0/负值/超限 → 400**（§4.2） |
| 9 | P2 「与现有复合索引一致」事实错误：All 用的普通索引只有 `last_active_at DESC` 单列 | 事实更正 + R3 动作：task/non-task **partial** 索引已含两列（mod.rs:2451 附近）；**普通索引 `idx_recent_threads_last_active` 升级为复合 `(last_active_at DESC, thread_id ASC)`**；favorites JOIN 路径按需补部分索引；加 EXPLAIN QUERY PLAN 契约测试（对齐既有 `..uses_partial_order_indexes` 模式）（§4.2） |

### 已确认（round 9）

Round 8 跨进程延迟提交时序已封闭（清理 bump revision + cursor 同事务校验必
reset；All/Chats 纯删除不再漏行）；R2 组合快照为正确根因方案（现有
`read_conn().transaction()` 可承载）；JOIN 投影合规；Core/App 分层、入口、
accessory 无新问题；R1 全局显式传输语义方向正确。

### 历史轮次（要点）

- **R8→v9**：采纳根因三件套（R1 传输契约 / R2 组合快照端点 / R3 keyset
  cursor），删除 marker/gate/operation-group 体系；reducer 统一总定义。
- **R7→v8**：awaitVerify 意图验证循环（保留，v9/v10 意图机核心）。
- **R5→v6**：服务端 CAS 写围栏（`expected_revision` 必填、接受即恒 bump 含
  no-op、409 同快照整页）——CONFIRMED。
- **R4→v5**：三元组围栏。**R3→v4**：flight/desired 双身份；main 纯 raw。
- **R2→v3**：meta singleton；同快照组页；`FavoriteThreadResult`；行 accessory；
  清理点三处（mod.rs:669/2045/2785）——CONFIRMED。
- **R1→v2**：守卫式单事务插入；All feed 穷尽 switch；gateway 切换清理；
  判别联合；Mac 行内取消。

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
R3 recent-threads keyset 分页（同时根治 All/Chats 现存 offset 跳行缺陷）。

## 2. 目标 / 非目标

**目标**：收藏标记 + 双端入口 + 三分类筛选；R1/R2/R3 系统级根治；客户端只保留
意图状态机（§7.2）与 presented 后置过滤（§7.3）。

**非目标**：收藏排序/重排；首页独立收藏段；All/Chats 行星标；SSE 推送；bot
命令面收藏筛选；收藏意图跨进程持久化；`serverIdempotencyKey`（预留扩展位）；
bot 分页 UX 改造（§4.2 裁决保留内部 offset）。

## 3. 数据模型（gateway SQLite）——round 6 起 CONFIRMED

（同 v9：`thread_favorites` + `thread_favorites_meta`；meta 初始化早于启动
purge；条件写接受即恒 bump 含 no-op、清理点删除变更时 bump；守卫式单事务
插入；不用 FK；清理点三处 mod.rs:669/2045/2785；条件查询全 SQL。）

**R3 索引动作（round9-9 更正）**：现状 = task/non-task partial 索引含
`(last_active_at DESC, thread_id ASC)` 两列，普通 `idx_recent_threads_
last_active` 仅单列。改造：普通索引升级为复合 `(last_active_at DESC,
thread_id ASC)`（drop + create，幂等迁移）；favorites JOIN 扫描按 EXPLAIN
结果决定是否补 `thread_favorites(thread_id)` 之外的辅助索引（PK 已覆盖）。
加 EXPLAIN QUERY PLAN 契约测试（对齐 `..uses_partial_order_indexes` 模式）。

## 4. Gateway API

### 4.1 收藏读写（写侧 CAS）——round 6 起 CONFIRMED，未改动

（同 v9：GET / PUT / DELETE + `expected_revision` 必填 + 409 同快照整页 +
`FavoriteThreadResult::{Updated|Conflict|NotFound}` → 200/409/404。）

### 4.2 `/api/recent-threads` keyset cursor（R3，v10 契约补全）

- **HTTP API cursor-only**；`limit` + 可选 `cursor`；滤镜参数与互斥规则不变。
- **`limit` 校验（round9-8）**：必须 `1 ≤ limit ≤ cap`（cap 沿现有上限），
  0/负值/超限 → 400。
- cursor = opaque base64url(JSON)：`{ v:1, filter, last_active_at, thread_id,
  favorites_revision? }`；跨滤镜使用 → 400；解码失败 → 400。
- keyset 谓词：`WHERE <filter> AND (last_active_at < :t OR (last_active_at
  = :t AND thread_id > :id))`，排序 `(last_active_at DESC, thread_id ASC)`。
- **翻页语义（round9-8）**：同快照内 `LIMIT N+1` 探测——返回前 N 行；
  `has_more = 第 N+1 行存在`；`next_cursor` 从最后一条**实际返回行**签发，
  `!has_more` → null。`total` = 同快照滤镜域 count（展示用，不参与 has_more
  推导）。
- **favorites cursor 的 revision 校验**：load-more 同读事务比对；失配 →
  200 `{ reset: true, threads: <全新首页>, next_cursor, revision, total,
  has_more }`。
- **内部消费者裁决（round9-5）**：bot `RecentThreadPageReader`（`/threads N`
  / next / prev 依赖任意页随机访问，recent_threads.rs:47/69、
  local_commands.rs:57）**保留内部 offset 形态**——
  `list_recent_threads_page(filter, limit, offset)` 降级为 internal-only
  （仅 `SqlRecentThreadPageReader` 使用；行为与漂移容忍度 = 今日现状，
  非本设计正确性包络的一部分）。HTTP 面不再暴露 offset。

### 4.3 组合快照端点（R2）——round 9 CONFIRMED

`GET /api/thread-favorites/snapshot?limit=N`：单读事务返回
`{ revision, thread_ids, favorites, recent: { threads, next_cursor, total,
has_more } }`；`recent` = favorites 滤镜首页（cursor 按同快照 revision 签发；
翻页语义同 §4.2）。Favorites replacement 原语。

### 4.4 已知边界

（同 v9：automation/hidden 不进投影；归档清行 + bump → cursor stale →
服务端强制 reset。）

## 5. 传输契约重构（R1，v10 补全）

- 删除 `idempotent: Bool` 隐式语义；**semantic mode 为全部请求 helper 的必填
  参数（无默认值，编译期强制）**，覆盖 GET/PUT/POST/DELETE/**PATCH**
  （round9-7；iOS PATCH helper :938 及其经由 :1054 的共享重试逻辑一并迁移）：
  - `readRetryable`：无副作用读，可自动重试（沿用现有 establishment
    classifier 与次数）；
  - `mutationSingleAttempt`：一切副作用写，恰一次 attempt。
- **mutation 结果分类（round9-2 收紧）**：
  - `ok(decodedBody)`；
  - `definitiveEndpointResponse(decoded)`：**仅当成功解码出本端点结构化响应
    体**——由调用方按端点契约映射（favorites：409→conflict、404→notFound、
    400/401/403→终端拒绝、429/结构化 503→可重试拒绝）；
  - `ambiguous`：session task 创建后的一切其它结果——网络错误、超时、截断、
    2xx 解码失败、**未解码的任意 HTTP 状态（裸 5xx、代理响应、解码失败的
    409/404）**；
  - `notSent`：可证明 task 未创建。
- **迁移面**：iOS `GaryxGatewayClient` 全部调用点 + desktop `garyx-client` +
  **`garyx-client` 目录外的 main 进程调用点**（如 agent-avatar.ts:71）；
  实现 PR 附 grep 全量清单（`fetch(`/`request(`/PATCH 穷举），逐点标注语义
  与行为变化。
- 契约测试：attempt 计数；分类矩阵（含 PATCH、裸 5xx→ambiguous、解码
  409→definitive）；无默认语义的编译期断言。

## 6. Desktop / iOS 落点

### 6.1 Desktop

- main 发布纯 raw（`favoritedThreadIds` + `favoritesRevision`，revision 单调
  接受，写 in-flight 期间不变，按 `entitiesGatewayUrl` 归一化）；判别联合
  `RecentThreadListFilter`；`garyx-client`：favorites 三端点 + snapshot +
  cursor 版 `fetchRecentThreads`；IPC 带 scope stamp。
- renderer：`favorites-ingress`（§7 意图机）；入口 1 =
  `ConversationHeaderTitle.tsx` 紧邻 Pin（lucide `Star`/`StarOff`）；入口 2 =
  Favorites tab 行内取消（共享 `ThreadRailRow` accessory 契约扩展，
  `[Unfavorite, Archive]`）；三 feed cursor 化 + gap-fill refresh（§7.4）；
  第三 tab + 方向键 + 空态；i18n 四条。

### 6.2 iOS

- Core：`GaryxFavoritesState`（§7 reducer + 退避 effect）；transport 按 §5；
  `GaryxRecentThreadFeeds`/`GaryxHomeThreadListPager` 分页迁移 cursor +
  gap-fill；`.favorites` case 全链（Filter/Storage/Reducer/Actor/
  Presentation）；snapshot 端点客户端。
- App：入口 1 长按（`GaryxMobileSidebarViews` 紧邻 Pin）、入口 2 线程内
  title 菜单（`GaryxMobileConversationViews` :942 附近）、入口 3 过滤器自动
  带出；`+ThreadPersistence` IO 薄层；`+ThreadList` refresh 增拉 favorites +
  All feed 辅助刷新穷尽 switch（RefreshCommitTests :523）；`+Gateway`（:55）
  切换全清；**Archive/Delete ambiguity reconcile（round9-6）**：现有 catch
  分支（Bots.swift:270 一类、ThreadLifecycle.swift:318）升级为——ambiguous →
  点查线程存在性（readRetryable）→ 已消失按成功收尾（本地删行 + refresh）/
  仍存在按失败报错（人工重试政策）/ 点查失败报错 + refresh。分页安全由
  keyset/reset 兜底，无 gate。
- 新 Core 文件 `xcodegen generate` + 提交 pbxproj；验证 `xcodebuild`。

## 7. 客户端收藏状态机规格（双端共同契约）

### 7.1 全局状态与身份围栏（同 v9）

raw（revision 单调 + scope 围栏）；写派发前置（首次 raw 就绪前意图排队）；
flight 三元组 `(gatewayScope, runtimeEpoch, requestToken)`，token allocator
永不重置；transport = §5；`presented(id) = latestDesired[id]?.desired ??
raw.contains(id)`。

### 7.2 意图 reducer（v10：七分支 + 三相位）

```
inFlight      = { requestToken, target, flightGeneration, expectedRevision }
latestDesired = { generation, desired,
                  phase: active
                       | retryScheduled(effectToken, cause, verificationFence?)
                       | awaitVerify(fence) }
```

**事件与转移（穷尽）：**

- `toggle(desired)`：generation += 1；latestDesired = {generation, desired,
  active}（覆盖任何前态与已排程 effect——旧 effectToken 自然失配）；无
  inFlight 且 raw 就绪 → dispatch。
- `dispatch`：inFlight = {新 token, desired, generation, E = raw.revision}；
  发 CAS 写（mutationSingleAttempt）。per-thread single-flight。
- `settle(ok, page)`：页入 raw。generation ≤ flightGeneration → 退休；否则
  desired ≠ raw → active + drain；相等 → 退休。
- `settle(conflict, page)`（解码 409，恒真未应用）：409 页入 raw；desired ≠
  raw → active + drain（新 E）；相等 → 退休。
- `settle(notFound)`（解码 404）：退休（线程已不存在）。
- `settle(definitiveRejected, retryable=false)`（解码 400/401/403）：退休 +
  surface error（用户可见失败，呈现回落 raw）。
- `settle(definitiveRejected, retryable=true)`（解码 429/结构化 503）：
  → retryScheduled(新 effectToken, .rejected, fence: nil)。
- `settle(ambiguous)`：→ awaitVerify(fence = flight.expectedRevision) +
  退避 effect(effectToken)。
- `settle(notSent)`：确定未发出。普通 dispatch → retryScheduled(新
  effectToken, .notSent, fence: nil)；**awaitVerify 探针的 notSent →
  retryScheduled(新 effectToken, .probeNotSent, verificationFence = 原
  fence)**（round9-3：原 orphan 的 ambiguity fence 保留）。
- `rawAccepted(page)`：更新 raw。**对一切无 inFlight 的意图触发检查**
  （round9-3 搁浅消除）：
  - awaitVerify(fence) / retryScheduled(_, _, fence≠nil)：`page.revision >
    fence` → verify 转移（raw == desired → 退休；≠ → active + drain）；
  - active / retryScheduled(fence=nil)：`raw == desired` → 退休（意图已被
    他端满足）；≠ → 保持（等 effect 或 drain）。
- `backoffFired(stamp)`：stamp = `(gatewayScope, runtimeEpoch, generation,
  effectToken)`，**四元全匹配**当前状态才生效（round9-3：匹配键为
  effectToken，无 fence 表示问题；toggle/切网关/已 verify 均自然失配丢弃）：
  retryScheduled/awaitVerify → 重派（同 desired，E = 当前 raw.revision，
  新 token）。
- `gatewayScopeCleared`：全清 + epoch bump。
- 退避 effect 由 **Core 产生并校验**，App/renderer 仅做定时器宿主。

**性质（测试断言）**：同 E 探针/orphan 至多一个被接受（CAS）；活性（探针
最终推 revision 过 fence；持续故障诚实降级）；终态恒 = 最后用户意图；
七分支 × generation 新旧 × 相位矩阵穷尽。

### 7.3 Favorites feed 呈现过滤（round9-4：单一权威谓词）

**Favorites feed 行仅当 `presented(id) == true` 才渲染**（对缓存行与
in-flight 响应行一律后置过滤）。推论：

- 乐观取消（意图 desired=false 存活，含 awaitVerify/retryScheduled）→ 隐藏；
- DELETE 成功 / 409/404 收敛 / 他端删除（raw 已不含）→ 隐藏（**缓存行不
  复活**——round9-4 反例封闭）；
- 确定性失败退休且 raw 仍含 → 行重现（正确）；重新收藏 → presented=true
  行重现。

v9 的 hidden-pending/hidden-tombstone 两态机制删除（被该谓词完全蕴含）；
replacement 只负责把缓存行集换新，不再承担 tombstone 退休语义。

### 7.4 feed 分页协议（v10：gap-fill 补全）

- **Favorites refresh/replacement**：snapshot 端点 → 原子替换整 feed（期间
  保留 display IDs，无 skeleton；失败保缓存）。load-more：cursor；
  `reset: true` → 原子替换。replacement 开始时 epoch bump 废弃在途
  load-more；in-flight 期间新 load-more 延后。
- **All/Chats refresh（round9-1）——gap-fill 链式拉取**：从首页开始按
  next_cursor 连续拉取，**每页与已加载集合求交，命中已知 thread_id 即停**
  （重叠达成，merge 保尾）；`has_more=false` 先到 → 整域已尽收，直接替换；
  **上限 K=5 页，超限 → 整 feed replacement + cursor 重置**（新增行过多，
  保尾无意义）。合并去重沿现有 dedup。该协议消除「新首页尾端与旧 cursor
  之间」的永久盲区。
- **load-more（全 feed）**：keyset cursor —— 删除不移位；插入/前移行由下次
  gap-fill refresh 收敛；Favorites 域变更由服务端 reset 裁决。
- 进程重启/切网关回切：feed 从头 prime；旧进程延迟提交 → revision bump →
  favorites 首个 load-more 即 reset（round 8/9 已确认封闭）。

## 8. 测试计划

**Gateway**

- CAS/meta/清理点/守卫插入/同快照组页：沿用（CONFIRMED 项）。
- keyset：删除 N 行后 load-more 不跳行；**LIMIT N+1 语义**（恰满页 has_more
  边界、末行签发 cursor、`!has_more` cursor=null）；`limit` 0/负/超限 400；
  cursor 跨滤镜/解码失败 400；bump 行为；page+count 一次读快照；
  **EXPLAIN QUERY PLAN 契约**（复合索引覆盖 All/滤镜扫描，round9-9）。
- favorites cursor reset（失配 → 同事务全新首页）；snapshot 同快照
  （commit-between 测试）。
- routes：favorites 写 200/409/404/400；滤镜互斥；bot reader 的内部 offset
  路径回归（行为不变）。

**传输契约（R1）**

- attempt 计数；分类矩阵：解码 409/404/400/401/403/429 → definitive 各映射、
  裸 5xx/代理响应/解码失败 409 → ambiguous、task 未创建 → notSent；PATCH
  helper 迁移；无默认语义编译期断言；调用点清单核对（含 garyx-client 外）。

**意图 reducer（双端，逐条对照 §7.2）**

- 七分支 × generation × 相位矩阵；R5 双序 / R7 丢失 409 / R8 探针 notSent /
  **R9 搁浅反例**（notSent → retryScheduled；raw R6 到达 → 检查触发；
  effectToken 失配丢弃；probe notSent 保留 fence）走查；verify 转移；
  backoffFired 四元失配（toggle 反向后旧 timer 不发旧 desired）；活性；
  不同 ID 隔离；desktop raw 纯度；切网关全清。

**feed（双端）**

- **gap-fill（round9-1）**：>1 页新增行 → 链式拉取至重叠命中、尾部保留、
  无盲区；K 超限 → 整替 + cursor 重置；has_more=false 提前终止。
- **presented 过滤（round9-4）**：409/404 收敛后缓存行不复活；他端删除后
  snapshot 前行已隐藏；确定性失败行重现；重新收藏行重现；in-flight
  load-more 响应行同样过滤。
- snapshot 原子替换（无 skeleton、失败保缓存）；reset 响应原子替换；
  replacement 废弃在途 load-more（两种完成顺序）；进程重启模拟（prime 后
  延迟提交 → 首个 load-more reset）。
- All/Chats cursor 迁移回归全绿。

**编排（iOS/desktop）**

- **Archive/Delete ambiguity reconcile（round9-6）**：ambiguous → 点查——
  线程已消失按成功收尾 / 仍存在报错 / 点查失败报错 + refresh；三路径测试。

**其余**

- desktop：三 tab、方向键、空态、行 accessory；判别联合映射；store 按
  gateway key 隔离。iOS：FilterStorage 往返；Reducer/Actor/Presentation；
  RefreshCommitTests。端到端：curl 三端点 + snapshot + cursor 翻页 + reset +
  gap-fill 场景；双端 UI 按 `garyx-product-ui` 走查两处入口 + 筛选切换 +
  行内取消。

## 9. 实现切分（四步提交，同仓同发无跨版本兼容）

1. **gateway**：favorites 表/CAS/API + keyset cursor（HTTP 面）+ 复合索引 +
   snapshot 端点 + bot reader 内部 offset 保留 + 全部服务端测试。
2. **传输契约**：iOS + desktop 请求语义重构（必填参数 + 分类矩阵 + 全调用点
   清单迁移）。
3. **双端状态机与 feed**：`GaryxFavoritesState` / `favorites-ingress`（七分支
   reducer）+ 三 feed cursor/gap-fill 迁移 + Favorites feed 协议 +
   Archive/Delete reconcile（先测后 UI）。
4. **UI**：desktop renderer / iOS App 入口与 tab + xcodegen。
