# Thread Favorites (线程收藏)

Status: draft v11 (addressing review round 10 — 7×P1 + 2×P2)
Date: 2026-07-16

## 0. 修订记录

### v11 两个结构性减法

1. **Favorites feed 取消分页**：收藏是用户人工精选集（现实量级个位~两位数），
   给它做通用 cursor 分页是 round 8–10 反复漏水的复杂度来源。v11：snapshot
   端点一次返回全部收藏行（cap 500 + `truncated` 标记），feed 协议 = 整列表
   原子替换，**favorites cursor / reset 协议 / 排序域围栏 / favorites 分页
   并发全部消失**；`/api/recent-threads` 的 `favorites=only` 滤镜参数一并
   删除（无消费者）。round10-1 与 round10-6 的 reset 分支随之无对象。
2. **All/Chats gap-fill 锚点改单调排序键 range-fill**：round10-2 证明「任意
   已知 ID 交集」是不稳定锚点。v11 利用排序键单调性（`last_active_at` 只增
   不减 → 行只上移不下移）：refresh 链式拉取**恰好填满 [新头 → 旧头排序键]
   区间**（页尾键 ≤ 旧头键即止），区间外的旧尾仍有效（除幽灵行，见下）。
   移动行必在区间内出现 → dedup 从旧尾移除；盲区结构性不存在。

### Round 10 findings → v11 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | P1 favorites cursor 只围栏 membership，不围栏排序域（活动重投影 last_active_at 不 bump favorites_revision） | Favorites 取消分页（减法 1）：无 cursor 即无围栏问题；每次 snapshot 整列表替换，排序天然最新 |
| 2 | P1 gap-fill「任意已知 ID 交集即停」两个永久错误（moved-known 假锚点；远端删除幽灵尾行） | 锚点改**旧头排序键 range-fill**（减法 2）：键单调 → 所有变动行都上移进区间，假锚点不存在。幽灵尾行（远端删除/滤镜退出）：range-fill 不解决，**由全量替换触发点收敛**——下拉刷新、app 前台/页面出现、K 超限、每 M=30 个周期（约 5 分钟）强制整替。此为对今日现状的严格改进（今日 offset feed 同样幽灵且无强制整替）；声明为有界陈旧而非永久（§7.4） |
| 3 | P1 gap-fill 多请求链无原子结算与并发围栏（K 整替 vs 旧 load-more 竞争 cursor） | **每 feed 单飞行道**：refresh 链与 load-more 互斥（链在途拒/延后 load-more，反之亦然）；链 = 单 ticket，**全链成功才一次性原子提交**（拼接 + dedup + cursor 归属），任一页失败整链丢弃（旧状态原样）；K 超限整替 bump epoch 废弃一切在途；两种完成顺序进测试（§7.4） |
| 4 | P1 reducer 类型不足：awaitVerify 无 effectToken；inFlight 无 origin，普通派发与 probe 的 notSent 不可区分（probe 必须保留 orphan fence）；active+mismatch 的「等 effect」无 effect 可等 | 类型补齐：`inFlight{..., origin: ordinary \| verify(fence)}`；`awaitVerify(fence, effectToken)`；probe notSent 从 `inFlight.origin` 取回 fence 进 `retryScheduled(…, verificationFence)`；`rawAccepted` 的 active+mismatch **立即产生 drain effect**（无 inFlight 即派发）（§7.2） |
| 5 | P1 终端拒绝（400/401/403）无条件退休整个 latestDesired，吞掉更新代意图 | 终端拒绝只结算 `flightGeneration`：`latestDesired.generation > flightGeneration` → 保留 + drain 独立派发（其自身失败独立结算）；仅同代退休 + surface error（§7.2） |
| 6 | P1 presented 依赖 raw 与 feed 同权威：404 不带页 → 旧 raw 仍含则行复活；favorites reset 首页可能含旧 raw 没有的行被过滤 | favorites **404 改为携带同快照 membership 页**（`{favorited:false, error, thread_ids, favorites, revision}`，与 409 同构）→ notFound settle 接受页入 raw；reset 路径随减法 1 消失（snapshot 本身就是 membership+rows 同快照，presented 一致性由构造保证）（§4.1、§7.2） |
| 7 | P1 point GET 走独立 WAL reader 越过 pending writer，不是提交消歧屏障 | 根因修改：**生命周期端点改幂等 ensure-absent**——archive/delete 对已归档/已不存在的目标返回 200 `{ok:true, changed:false}`（而非 404 错误）；ambiguity 所有者 = 编排层状态机**显式重派同一操作**（每次 mutationSingleAttempt，退避）直至确定性结局：200（changed 或 already-gone）= 成功、结构化 409（active run/binding）= 确定失败。point-GET 屏障方案废弃（§4.4、§6.2） |
| 8 | P2 「本端点结构化响应」无可判别 wire schema，代理 JSON 可误配宽松 error DTO | 非 200 响应定义**严格 tagged union**：`{ kind: "garyx_api_error", operation: "<端点操作名>", code: "conflict\|not_found\|invalid_request\|…", …payload }`；客户端 definitive 判定要求 kind+operation 双匹配，缺失/未知一律 ambiguous；补「JSON 但非端点响应」测试（§4.1、§5） |
| 9 | P2 R1 迁移面漏 desktop web surface 的 `requestJson`（web-api.ts:97/:295 直接 fetch） | R1 迁移清单纳入 `web-api.ts` helper（semantic mode 必填同规则）；实现 PR 的 grep 清单以「一切直接 fetch/URLSession 调用」为界，不以目录为界（§5） |

### 已确认（round 10）

独立 favorites SQL 投影/JOIN 合规；CAS 恒 bump、409 同快照页、组合 snapshot
方向正确；LIMIT N+1、keyset 谓词、复合索引修正合理。

### 历史轮次（要点）

- **R9→v10**：gap-fill 初版（v11 改 range-fill）；七分支结算矩阵；presented
  谓词统一；bot reader 保留内部 offset；LIMIT N+1；索引事实修正。
- **R8→v9**：根因三件套（R1 传输契约 / R2 组合快照 / R3 keyset），删除
  marker/gate 体系。
- **R7→v8**：awaitVerify 意图验证循环。**R5→v6**：服务端 CAS 写围栏
  （CONFIRMED）。**R4→v5**：三元组围栏。**R3→v4**：flight/desired 双身份、
  main 纯 raw。**R2→v3**：meta singleton、同快照组页、清理点三处
  （CONFIRMED）。**R1→v2**：守卫插入、判别联合、双端入口。

## 1. 需求

用户需求（产品裁决，不可改动的部分）：

1. 线程支持「收藏」（favorite）。
2. 最近线程列表的筛选类别变为三个：**全部（All）/ Chats / 收藏（Favorites）**。
3. iOS：首页线程行**长按**出收藏项；线程内**右上角菜单**（与置顶相邻）；
   右上角过滤器增加「收藏」类别。
4. Mac：收藏触发点与置顶一致；筛选处新增「收藏」tab。

**系统级连带改造（用户 scope 裁决授权）**：R1 传输契约、R2 组合快照端点、
R3 All/Chats keyset 分页、**R4（v11 新增）生命周期端点幂等化**。

## 2. 目标 / 非目标

**目标**：收藏标记 + 双端入口 + 三分类筛选；R1–R4 系统级根治；客户端只保留
意图状态机（§7.2）+ presented 后置过滤（§7.3）+ All/Chats range-fill（§7.4）。

**非目标**：收藏排序/重排；**收藏分页**（cap 500 + truncated 标记，超限
展示不完整属可接受产品边界）；首页独立收藏段；All/Chats 行星标；SSE；bot
命令面收藏筛选（bot reader 保留内部 offset，UX 不动）；收藏意图跨进程持久化；
`serverIdempotencyKey`（预留位）。

## 3. 数据模型（gateway SQLite）——round 6 起 CONFIRMED

（同 v10：`thread_favorites` + `thread_favorites_meta`；meta 初始化早于启动
purge；条件写接受即恒 bump 含 no-op、清理点删除变更时 bump；守卫式单事务
插入；不用 FK；清理点三处 mod.rs:669/2045/2785；条件查询全 SQL。）

索引动作：普通 `idx_recent_threads_last_active` 升级为复合
`(last_active_at DESC, thread_id ASC)`（task/non-task partial 已含两列）；
EXPLAIN QUERY PLAN 契约测试。

## 4. Gateway API

### 4.0 错误响应 tagged schema（round10-8，新增，favorites/生命周期端点适用）

一切非 200 结构化响应：

```json
{ "kind": "garyx_api_error", "operation": "thread_favorites_put",
  "code": "conflict", …操作相关负载… }
```

`operation` 每端点唯一；`code` 枚举（`conflict` / `not_found` /
`invalid_request` / `unauthorized` / `forbidden` / `rate_limited` /
`unavailable` / …）。客户端 definitive 判定要求 **kind + operation 双匹配**；
解码失败、kind 缺失、operation 不符 → 一律 ambiguous。

### 4.1 收藏读写（写侧 CAS；v11：404 带页）

| 方法 & 路径 | 行为 |
|---|---|
| `GET /api/thread-favorites` | 200 `{ thread_ids, favorites, revision }` |
| `PUT /api/thread-favorites/{key}?expected_revision=N` | 200 整页（接受即 bump 含 no-op）；失配 409 tagged + **同快照整页**；**404 tagged + 同快照整页**（round10-6：`{kind, operation, code:"not_found", thread_ids, favorites, revision}`）；400 tagged |
| `DELETE /api/thread-favorites/{key}?expected_revision=N` | 同上（`removed` 字段） |

同快照组页不变量扩展到 404（同一读/写事务内回读 membership 页）。
`FavoriteThreadResult::{Updated(page) | Conflict(page) | NotFound(page)}`。

### 4.2 `/api/recent-threads` keyset cursor（仅 All/Chats）

- **`favorites` 滤镜参数删除**（减法 1，无消费者）；`tasks` 参数与语义不变。
- cursor-only（HTTP 面）；`limit` 1..cap，0/负/超限 400；cursor opaque
  base64url，绑定滤镜，解码失败 400。
- keyset 谓词 + `(last_active_at DESC, thread_id ASC)`；LIMIT N+1 探测；
  `next_cursor` 从末条实际返回行签发；`total` 同快照 count 仅展示。
- bot `RecentThreadPageReader` 保留内部 offset（internal-only，行为=今日）。

### 4.3 组合快照端点（R2；v11：非分页整列表）

`GET /api/thread-favorites/snapshot`

单读事务返回：

```json
{ "revision": 7, "thread_ids": ["…"], "favorites": [{…}],
  "recent": { "threads": […], "total": 3, "truncated": false } }
```

- `recent.threads` = 全部收藏线程的 recent 行（JOIN，`last_active_at DESC,
  thread_id ASC`），**cap 500**，超限 `truncated: true`（展示前 500，产品
  可接受边界）。无 cursor。
- membership、revision、rows 同快照由构造保证 → presented 一致性无错位
  （round10-6 第二支消灭）。

### 4.4 生命周期端点幂等化（R4，round10-7 根因）

- **archive / delete 改 ensure-absent 语义**：目标已归档/已不存在 → 200
  `{ ok: true, changed: false }`（不再 404 错误）；实际执行 → 200
  `{ ok: true, changed: true }`；业务冲突（active run / active binding，
  routes.rs:3137/:3247）→ 结构化 409 tagged（确定性失败）。
- ambiguity 所有者 = 客户端编排状态机：ambiguous → **退避显式重派同一操作**
  （每次 mutationSingleAttempt）直至确定性结局——200（changed 任意值）=
  成功收尾（本地删行 + snapshot/refresh）；结构化 409 = 确定失败报错。
  重派对 ensure-absent 幂等安全；早先超时 attempt 的延迟提交与重派结果
  语义一致。point-GET 屏障方案废弃（reader 越过 pending writer，
  mod.rs:2051，不能证明未来不提交）。
- 影响面：gateway 两个 route 的响应语义 + 双端 archive/delete 编排
  （Bots.swift:270 catch 路径、ThreadLifecycle.swift:318、desktop 对应）。
  既有「404 视为失败」的调用方随迁移清单更新。

### 4.5 已知边界

automation/hidden 不进投影；归档清 favorites 行 + bump（收藏行随下次
snapshot 消失）。

## 5. 传输契约重构（R1）

- semantic mode **全请求 helper 必填**（GET/PUT/POST/DELETE/PATCH，无默认，
  编译期强制）：`readRetryable` / `mutationSingleAttempt`。
- mutation 结果：`ok(decoded)` / `definitiveEndpointResponse(decoded tagged)`
  （kind+operation 双匹配才算，round10-8）/ `ambiguous`（task 创建后其它
  一切，含裸 5xx、代理 JSON、解码失败）/ `notSent`（task 未创建）。
- **迁移面以「一切直接 HTTP 调用」为界**（round10-9）：iOS
  `GaryxGatewayClient` 全调用点（含 PATCH :938/:1054）；desktop
  `garyx-client` + main 内其它 fetch（agent-avatar.ts:71）+ **web surface
  `web-api.ts` requestJson（:97，settings PUT :295）**。实现 PR 附 grep
  全量清单（fetch/URLSession/requestJson 穷举）。
- 契约测试：attempt 计数；分类矩阵（解码 tagged 各 code、裸 5xx→ambiguous、
  「JSON 但非端点响应」→ambiguous、PATCH、notSent）；无默认语义编译断言。

## 6. Desktop / iOS 落点

### 6.1 Desktop

- main 发布纯 raw（revision 单调、写 in-flight 不变、按 `entitiesGatewayUrl`
  归一化）；判别联合 `RecentThreadListFilter = "all" | "chats" | "favorites"`
  （favorites 走 snapshot，不走 recent-threads）；`garyx-client`：favorites
  三端点 + snapshot + cursor 版 `fetchRecentThreads`；IPC 带 scope stamp。
- renderer：`favorites-ingress`（§7）；入口 1 = `ConversationHeaderTitle.tsx`
  紧邻 Pin（lucide `Star`/`StarOff`，无快捷键）；入口 2 = Favorites tab 行内
  取消（`ThreadRailRow` accessory 契约扩展，`[Unfavorite, Archive]`）；
  第三 tab + 方向键 + 空态 "No favorite threads"；All/Chats feed cursor 化 +
  range-fill（§7.4）；Favorites feed = snapshot 整替；i18n 四条。

### 6.2 iOS

- Core：`GaryxFavoritesState`（§7 reducer + 退避 effect，SwiftPM 全测）；
  transport 按 §5；`GaryxRecentThreadFeeds`/Pager：All/Chats cursor +
  range-fill 迁移，Favorites = snapshot 整替（无分页态）；`.favorites` case
  全链（Filter/Storage/Reducer/Actor/Presentation）；snapshot 客户端。
- App：入口 1 长按（紧邻 Pin，`star`/`star.slash`）、入口 2 线程内 title
  菜单（:942 附近）、入口 3 过滤器自动带出；`+ThreadPersistence` IO 薄层
  （runtime UUID 围栏对齐 pin :92）；`+ThreadList` refresh 增拉 favorites +
  All feed 辅助刷新穷尽 switch（RefreshCommitTests :523）；`+Gateway`（:55）
  切换全清；**Archive/Delete 编排改 R4 重派协议**（§4.4；catch 分支升级为
  ambiguous → 退避重派至确定性结局）。
- 新 Core 文件 `xcodegen generate` + 提交 pbxproj；验证 `xcodebuild`。

## 7. 客户端收藏状态机规格（双端共同契约）

### 7.1 全局状态与身份围栏（同 v10）

raw（revision 单调 + scope 围栏；来源 GET/写响应/409 页/404 页/snapshot）；
写派发前置；flight 三元组，token allocator 永不重置；transport = §5；
`presented(id) = latestDesired[id]?.desired ?? raw.contains(id)`。

### 7.2 意图 reducer（v11 类型补齐）

```
inFlight      = { requestToken, target, flightGeneration, expectedRevision,
                  origin: ordinary | verify(fence) }
latestDesired = { generation, desired,
                  phase: active
                       | retryScheduled(effectToken, cause, verificationFence?)
                       | awaitVerify(fence, effectToken) }
```

**事件与转移（穷尽）：**

- `toggle(desired)`：generation += 1；latestDesired = {generation, desired,
  active}（旧 effectToken 自然失配）；无 inFlight 且 raw 就绪 → dispatch。
- `dispatch`：inFlight = {新 token, desired, generation, E = raw.revision,
  origin: ordinary}；probe 重派用 origin: verify(原 fence)。单飞行。
- `settle(ok, page)`：页入 raw。gen ≤ flightGen → 退休；否则 desired ≠ raw →
  active + **drain effect**；相等 → 退休。
- `settle(conflict, page)`：409 页入 raw；desired ≠ raw → active + drain
  （新 E）；相等 → 退休。
- `settle(notFound, page)`（v11：404 带页）：**页入 raw**（raw 已不含该
  线程）→ 退休。presented 立即 false，无复活（round10-6 第一支消灭）。
- `settle(definitiveRejected, retryable=false)`（解码 400/401/403）：
  **只结算 flightGeneration**（round10-5）：gen == flightGen → 退休 +
  surface error；gen > flightGen → 保留 active + drain（新代意图独立派发、
  独立结算）。
- `settle(definitiveRejected, retryable=true)`（解码 429/unavailable）：
  → retryScheduled(新 effectToken, .rejected, fence: origin 为 verify 时
  保留原 fence)。
- `settle(ambiguous)`：→ awaitVerify(fence = flight.expectedRevision
  （origin 为 verify 时取原 fence，取更小者以保守），新 effectToken) +
  退避 effect。
- `settle(notSent)`：origin ordinary → retryScheduled(新 effectToken,
  .notSent, fence: nil)；**origin verify(fence) → retryScheduled(新
  effectToken, .probeNotSent, verificationFence = fence)**（round10-4：
  probe 的 orphan fence 从 inFlight.origin 取回，不丢失）。
- `rawAccepted(page)`：更新 raw。对一切无 inFlight 的意图：
  - awaitVerify(fence, _) / retryScheduled(_, _, fence≠nil)：page.revision >
    fence → verify（raw == desired → 退休；≠ → active + **立即 drain
    effect**）；
  - active / retryScheduled(fence=nil)：raw == desired → 退休；≠ →
    **立即产生 drain effect**（无 inFlight 即派发；round10-4「无 effect 可
    等」消除）。
- `backoffFired(stamp)`：stamp = (scope, epoch, generation, effectToken)
  四元全匹配（awaitVerify/retryScheduled 均持有 effectToken，可匹配）→
  重派（同 desired，E = 当前 raw.revision，origin 按相位）。
- `gatewayScopeCleared`：全清 + epoch bump。
- 退避 effect 由 Core 产生并校验；宿主只做定时器。

**性质**：同 E 至多一个接受（CAS）；活性（probe 推 revision 过 fence）；
终态恒 = 最后用户意图；七分支 × generation × 相位 × origin 矩阵穷尽。

### 7.3 Favorites feed 呈现过滤（round9-4，v11 不变）

**行仅当 `presented(id) == true` 才渲染**（缓存行与 in-flight 响应行一律
后置过滤）。snapshot 的 membership 与 rows 同快照 → 无 presented 错位。

### 7.4 feed 协议

**Favorites**：snapshot 整列表原子替换（触发：写确认、周期 10s、下拉、切入
tab；期间保留 display IDs 无 skeleton；失败保缓存）；单飞行（in-flight 期间
新触发合并）。无 load-more、无 cursor、无 reset。

**All/Chats（R3 + round10-2/3 重构）**：

- **load-more**：keyset cursor（删除不移位、不跳行）。
- **refresh = range-fill 链**（锚点 = 旧头排序键 `(last_active_at,
  thread_id)`，键单调只增）：从首页起按 next_cursor 链式拉取，**页尾键 ≤
  旧头键即止**；`has_more=false` 先到 → 整域尽收直接整替。终止时原子提交：
  新区间行 + 旧列表 dedup 拼接（移动行必在新区间 → 从旧尾去重）。
  **正确性**：键只增 ⇒ 自上次加载以来每个变动/新增行的键 > 旧头键 ⇒ 全部
  落在拉取区间内；区间外旧尾除删除外未变。
- **并发（round10-3）**：每 feed 单飞行道——refresh 链与 load-more 互斥
  （一方在途，另一方拒/延后）；链 = 单 ticket，全链成功才一次原子提交，
  任一页失败**整链丢弃**（旧状态与 cursor 原样）；**K=5 页超限 → 整替**
  （bump epoch 废弃一切在途，cursor 重置）；整替与旧 load-more 两种完成
  顺序：epoch 判弃 / 原子提交互斥。
- **幽灵尾行收敛（round10-2 第二支）**：远端删除/滤镜退出的行留在旧尾 =
  今日现状 parity；**全量替换触发点**保证有界陈旧：用户下拉刷新、app 前台
  /页面出现、K 超限、每 M=30 个周期刷新（约 5 分钟）强制整替。相对今日
  （offset、无强制整替、删除还会跳行）为严格改进。

进程重启/切网关：feed 从头 prime；favorites 无 cursor 无遗留漂移面。

## 8. 测试计划

**Gateway**

- CAS/meta/清理点/守卫插入/同快照组页（含 **404 带页同快照**）；孤儿写交错；
  `favorites=only` 参数已删除（回归：传入 → 400 未知参数或忽略，按现有参数
  校验约定定一致行为）。
- keyset（All/Chats）：删除后不跳行；LIMIT N+1 边界；limit 校验；cursor
  绑定滤镜/解码失败 400；EXPLAIN 契约（复合索引）；page+count 一次读快照。
- snapshot：membership+revision+rows 同快照（commit-between 测试）；cap 500 +
  truncated；空收藏。
- **生命周期幂等化（R4）**：archive/delete 对已归档/已删目标 200
  `{changed:false}`；重复执行幂等；业务冲突 409 tagged；「旧 attempt 延迟
  提交 + 重派」交错下终态一致。
- tagged error schema：各端点非 200 响应 kind/operation/code 断言。

**传输契约（R1）**

- attempt 计数；分类矩阵（tagged 双匹配、「JSON 但非端点响应」→ambiguous、
  裸 5xx、PATCH、notSent）；web-api.ts helper 迁移；无默认语义编译断言；
  全调用点清单核对。

**意图 reducer（双端）**

- 七分支 × generation × 相位 × origin 矩阵；R5 双序 / R7 丢失 409 / R9 搁浅
  / **R10 全反例**：probe notSent 保留 fence（origin 区分两个同形历史）、
  终端拒绝不吞新代意图（gen1 400 后 gen3 仍派发）、404 带页后 presented
  false 不复活、active+mismatch 立即 drain；backoffFired 四元失配；活性；
  不同 ID 隔离；desktop raw 纯度；切网关全清。

**feed（双端）**

- **range-fill**：多页新增全收敛；**moved-known 场景**（旧行升头不再假锚，
  新增行全部进区间）；has_more=false 提前整替；K 超限整替；链任一页失败
  整链丢弃旧态原样；refresh 链与 load-more 互斥两方向 + 两种完成顺序；
  幽灵尾行在整替触发点消失（下拉/前台/周期 M）。
- Favorites snapshot 整替：原子替换、无 skeleton、失败保缓存、单飞行合并；
  presented 过滤（409/404 收敛不复活、他端删除 snapshot 前已隐藏、确定性
  失败重现、重新收藏重现、in-flight 响应行同过滤）。
- **Archive/Delete 重派协议（R4）**：ambiguous → 重派至 200/409 确定性
  结局三路径；重派期间行呈现策略（沿现有 transition 语义）。

**其余**

- desktop：三 tab、方向键、空态、行 accessory；判别联合映射；store 按
  gateway key 隔离。iOS：FilterStorage 往返；Reducer/Actor/Presentation；
  RefreshCommitTests。端到端：curl 三端点 + snapshot + cursor 翻页 +
  range-fill 场景 + 生命周期幂等；双端 UI 按 `garyx-product-ui` 走查两处
  入口 + 筛选切换 + 行内取消。

## 9. 实现切分（五步提交，同仓同发无跨版本兼容）

1. **gateway-favorites**：表/CAS/API（404 带页 + tagged errors）+ snapshot
   （非分页）+ 服务端测试。
2. **gateway-recent**：All/Chats keyset cursor + 复合索引 + bot reader 内部
   offset 保留 + R4 生命周期幂等化。
3. **传输契约**：iOS + desktop（含 web-api.ts）语义重构。
4. **双端状态机与 feed**：`GaryxFavoritesState` / `favorites-ingress` +
   All/Chats range-fill + Favorites snapshot 整替 + Archive/Delete 重派编排
   （先测后 UI）。
5. **UI**：desktop renderer / iOS App 入口与 tab + xcodegen。
