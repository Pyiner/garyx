# Thread Favorites (线程收藏)

Status: draft v12 (addressing review round 11 — 10 findings)
Date: 2026-07-16

## 0. 修订记录

### v12 两个结构性决定

1. **All/Chats 排序改数据库分配的全局活动序号 `activity_seq`**（round11-1/2/3
   根因）：round 11 证明 `last_active_at` 在现码里不单调（RMW 竞态可写回旧
   时间戳，navigation.rs:151 + sqlite_thread_store.rs:166 的锁只包 set 不包
   read-modify-write），且时间戳并列在页边界必漏行、多页链跨快照无围栏。
   v12：`recent_threads` 新增 `activity_seq INTEGER NOT NULL`，**在投影写
   事务内从 meta 计数器单调分配**（单 writer 串行化 ⇒ 构造性单调、全局唯一、
   无并列）；排序、keyset、range-fill 锚点全部改用 seq；`last_active_at`
   仅作展示字段。三条 finding 的反例（时钟倒退 / 并列键边界 / 链间移动）
   全部结构性消灭或收敛为「≤1 个刷新周期的有界陈旧」（§4.2、§7.4）。
2. **R4（生命周期端点幂等化）从本设计移除，拆为独立后续任务**（round11-5/
   6/10）：round 11 证明 ensure-absent 远不够——409 不能结算整个操作（检查
   与提交间有异步窗口）、delete 无 tombstone 可被旧 writer 复活、两阶段清理
   崩溃后 already-gone 误判完成。这是独立于收藏的系统工程（operation token +
   结果日志 + tombstone + 清理 outbox），塞进本设计只会持续扩张边界。v12
   回到 round 9 评审明示可接受的「**明确的人工重试政策**」= 今日 parity：
   ambiguous archive/delete → 报错给用户 + 触发 feed refresh，不做新承诺。
   **收藏正确性不依赖生命周期结局**：分页安全靠 seq-keyset（删除不移位）、
   收藏行清理靠服务端事务（清 favorites 行 + bump revision → 下次 snapshot
   消失）、幽灵行靠整替触发点有界收敛。幂等化已立独立后续任务另行设计评审。

### Round 11 findings → v12 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | P1 「last_active_at 只增不减」与现码不符（RMW 竞态倒退） | `activity_seq` 事务内分配（决定 1）：排序键单调由 DB 构造保证，与时间戳载荷无关 |
| 2 | P1 逐线程单调推不出全局界；并列键页边界漏行；DESC/ASC 混合比较器未定义 | seq 全局唯一严格递增：无并列、无混合比较器；range-fill 锚点 = 旧头 seq，「页尾 seq ≤ 旧头 seq 即止」精确；「刷新前新增行必有 seq > 旧头 seq」由全局分配直接成立 |
| 3 | P1 多页链非同一快照，页间移动漏行 | seq 下形式化边界：链中/链后上移的行获得新的全局最大 seq，本轮链最多错过其**一个刷新周期**（下轮 range-fill 的旧头 seq 必小于其新 seq → 必被捕获）；期间该行要么仍在旧尾显示（位置陈旧）、要么属未加载区（今日 parity）。「永久漏行」类消灭，边界写入测试（§7.4） |
| 4 | P1 snapshot 缺 membership+rows 原子接受门控（旧 snapshot 晚到 vs 更新的写响应 raw） | snapshot 定义为**原子接受单元** `(scope, epoch, ticket, revision)`：`revision < highestObserved` → **整包丢弃**（membership 与 rows 一起弃）+ 置 trailing-dirty；接受则 raw 与 rows 一次事务性替换。飞行期间新触发 → trailing-dirty，settle 后补拉一次（§7.4） |
| 5 | P1 R4 的 409 只结算当前 attempt，不能结算用户操作 | R4 移除（决定 2）：今日 parity 人工重试；幂等化独立任务 |
| 6 | P1 delete 无 tombstone 可复活；already-gone 清理完成条件未定义 | 同上：R4 移除，本设计不改 delete 语义 |
| 7 | P1 toggle 覆盖 phase 会丢未消歧 orphan fence（gen2 notSent 后被 raw@E 等值退休，orphan 后提交反向） | **`unresolvedFence` 提为正交 per-thread 状态**（独立于 generation/phase）：ambiguous settle 置 `min(现值, flight.E)`；清除条件 = 观察到 `revision > fence` 或本端后续 CAS 写被接受（其返回 revision 必 > fence）。**一切等值退休捷径（raw == desired → 退休）都要求 `raw.revision > unresolvedFence`**，否则保持等待（§7.2） |
| 8 | P1 rawAccepted 的立即 drain 绕过 429/503 退避 | `retryScheduled`（fence 有无皆然）在 raw ≠ desired 时**保持原 effectToken 等 timer**，仅 raw == desired（且过 fence 门）才退休；只有真 `active` 才即时 drain（§7.2） |
| 9 | P1 tagged schema 只迁移消费者未迁移生产者；auth 401 无 kind/operation → 永久 ambiguous 循环 | **生产者迁移清单**：①gateway auth middleware（gateway_auth.rs:143）改 tagged（operation="gateway_auth"，code=unauthorized/forbidden）——favorites reducer 的终端 401/403 分支由此成立；②favorites 端点全 tagged（新建即 tagged）；③**遗留端点不在本设计迁移**——策略明示：untagged 错误在新传输契约下恒 ambiguous，仅影响使用新 reducer 的调用方（= favorites），遗留 mutation 调用方保持既有粗粒度处理、不依赖 definitive 分类（§4.0、§5） |
| 10 | P1/P2 R4 重试无 scope/epoch 身份、未入 Core、漏 automation 冲突 | R4 移除后无重试机；今日 archive 编排（runtimeGeneration 校验等）原样保留 |

### 已确认（round 11）

独立 SQL favorites 投影/JOIN 合规；Favorites 取消分页 + cap 500/truncated 为
明确产品边界；CAS 恒 bump、409/404 同快照页、snapshot 单读事务方向正确。

### 历史轮次（要点）

- **R10→v11**：Favorites 取消分页；range-fill 初版（v12 改 seq）；reducer
  类型补齐；404 带页；tagged union。
- **R9→v10**：七分支结算矩阵；presented 谓词；bot reader 内部 offset；
  LIMIT N+1。**R8→v9**：根因三件套 R1/R2/R3，删除 marker/gate 体系。
- **R7→v8**：awaitVerify 验证循环。**R5→v6**：CAS 写围栏（CONFIRMED）。
  **R4→v5**：三元组围栏。**R3→v4**：双身份、main 纯 raw。**R2→v3**：meta
  singleton、同快照组页、清理点三处（CONFIRMED）。**R1→v2**：守卫插入、
  判别联合、双端入口。

## 1. 需求

用户需求（产品裁决，不可改动的部分）：

1. 线程支持「收藏」（favorite）。
2. 最近线程列表的筛选类别变为三个：**全部（All）/ Chats / 收藏（Favorites）**。
3. iOS：首页线程行**长按**出收藏项；线程内**右上角菜单**（与置顶相邻）；
   右上角过滤器增加「收藏」类别。
4. Mac：收藏触发点与置顶一致；筛选处新增「收藏」tab。

**系统级连带改造（用户 scope 裁决授权，改根因并说清楚）**：
R1 传输契约（+auth middleware tagged）、R2 组合快照端点、
R3 All/Chats keyset 分页 + **activity_seq 排序**（v12）。
（原 R4 生命周期幂等化已拆出为独立后续任务。）

## 2. 目标 / 非目标

**目标**：收藏标记 + 双端入口 + 三分类筛选；R1–R3 系统级根治；客户端只保留
意图状态机（§7.2）+ presented 后置过滤（§7.3）+ All/Chats range-fill（§7.4）。

**非目标**：收藏排序/重排；收藏分页（cap 500 + truncated）；首页独立收藏段；
All/Chats 行星标；SSE；bot 命令面收藏筛选（bot reader 保留内部 offset）；
收藏意图跨进程持久化；`serverIdempotencyKey`（预留位）；**生命周期端点
幂等化**（独立后续任务；本设计对 archive/delete 保持今日行为与今日错误
处理，仅在 ambiguous 报错后追加一次 feed refresh 触发）。

## 3. 数据模型（gateway SQLite）

### 3.1 favorites（round 6 起 CONFIRMED）

（同 v11：`thread_favorites` + `thread_favorites_meta`；meta 初始化早于启动
purge；条件写接受即恒 bump 含 no-op、清理点删除变更时 bump；守卫式单事务
插入；不用 FK；清理点三处 mod.rs:669/2045/2785。）

### 3.2 activity_seq（v12 新增，round11-1/2/3 根因）

```sql
ALTER TABLE recent_threads ADD COLUMN activity_seq INTEGER NOT NULL DEFAULT 0;
CREATE TABLE IF NOT EXISTS recent_threads_meta (
  id           INTEGER PRIMARY KEY CHECK (id = 1),
  activity_seq INTEGER NOT NULL CHECK (activity_seq >= 0)
) STRICT;
CREATE INDEX … ON recent_threads (activity_seq DESC);        -- 全量
-- task / non-task partial 索引同步改为 (activity_seq DESC)
```

- **分配**：`upsert_recent_thread_tx` 在**同一投影写事务**内 `activity_seq =
  ++meta.activity_seq`（单 writer mutex + writer 串行化 ⇒ 严格递增、全局
  唯一）。每次行 upsert 都分配新 seq（行有任何投影变化即前移，与今日
  「活动即上浮」语义一致）。
- **一次性迁移**：现有行按 `(last_active_at ASC, thread_id DESC)` 顺序回填
  seq（保持现有展示顺序），meta 计数器置为最大值；启动 cutover 模式对齐
  `recent_task_thread_kind_v1` 先例。
- **排序与分页键 = `activity_seq DESC`**；`last_active_at` 降级为纯展示
  字段（客户端行内时间显示不变）。时间戳载荷的任何倒退（round11-1 的 RMW
  竞态）不再影响排序正确性；该 RMW 竞态本身留存为展示层小误差（今日已
  存在，非本设计扩大面）。

## 4. Gateway API

### 4.0 错误响应 tagged schema（round10-8 + round11-9 生产者清单）

`{ "kind": "garyx_api_error", "operation": "…", "code": "…", …payload }`

- **生产者迁移（本设计内）**：①**gateway auth middleware**
  （gateway_auth.rs:143）——401/403 改 tagged（operation="gateway_auth"）；
  ②favorites 全部端点（新建即 tagged）；③`/api/recent-threads` 参数错误
  （改造中顺带）。
- **遗留端点策略（明示）**：其余 route 的普通 `{ok,error}` 错误**不在本设计
  迁移**；新传输契约下 untagged 错误恒 ambiguous——只影响使用新 reducer 的
  调用方（favorites）；遗留 mutation 调用方不使用 definitive 分类，保持
  既有处理，无行为回退。favorites reducer 需要的 definitive 生产者
  （401/403 = middleware，400/404/409 = favorites 端点）在本设计内全部
  tagged 化，无 ambiguous 循环（round11-9 反例消灭）。
- 客户端判定：kind + operation 双匹配才 definitive；否则 ambiguous。

### 4.1 收藏读写（CAS；404/409 带同快照页）——CONFIRMED，未改动

（同 v11：GET / PUT / DELETE + `expected_revision` 必填；200 接受即 bump 含
no-op；409/404 tagged + 同快照 membership 页；400 tagged；
`FavoriteThreadResult::{Updated|Conflict|NotFound}(page)`。）

### 4.2 `/api/recent-threads`（仅 All/Chats；v12 seq keyset）

- cursor-only；`tasks` 参数不变；`favorites` 参数不存在（v11 删除）。
- `limit` 1..cap；cursor = opaque base64url `{ v, filter, activity_seq }`，
  绑定滤镜；解码失败/跨滤镜 400（tagged）。
- keyset 谓词：`WHERE <filter> AND activity_seq < :seq ORDER BY activity_seq
  DESC LIMIT N+1`；`next_cursor` 从末条实际返回行的 seq 签发；
  `has_more = 取到 N+1`；`total` 同快照 count 仅展示。
- bot `RecentThreadPageReader` 保留内部 offset（internal-only，行为=今日）。

### 4.3 组合快照端点（R2；非分页）——CONFIRMED

`GET /api/thread-favorites/snapshot`：单读事务返回
`{ revision, thread_ids, favorites, recent: { threads, total, truncated } }`；
`recent.threads` = 全部收藏线程 recent 行按 `activity_seq DESC`，cap 500。

### 4.4 生命周期操作（v12：今日 parity，无新语义）

archive/delete 的服务端行为与错误契约**完全不动**。客户端编排唯一追加：
ambiguous 结局在既有报错之后**触发一次 feed refresh/snapshot**（若操作实际
已提交，行随之消失；未提交则列表不变）。幂等化见独立后续任务。

### 4.5 已知边界

automation/hidden 不进投影；归档清 favorites 行 + bump（行随下次 snapshot
消失）。

## 5. 传输契约重构（R1）

（同 v11 + round11-9 修订）semantic mode 全 helper 必填（含 PATCH 与
web-api.ts requestJson）；mutation 结果四分类；**definitive 仅认 tagged
双匹配**——本设计内 tagged 生产者 = auth middleware + favorites 端点；
untagged 错误恒 ambiguous 且明示只影响 favorites reducer（遗留调用方不依赖
分类）。迁移清单以「一切直接 HTTP 调用」为界，实现 PR 附 grep 全量清单。

## 6. Desktop / iOS 落点

### 6.1 Desktop

- main 发布纯 raw（revision 单调、写 in-flight 不变、按 `entitiesGatewayUrl`
  归一化）；判别联合 `"all" | "chats" | "favorites"`（favorites 走
  snapshot）；`garyx-client`：favorites 三端点 + snapshot + seq-cursor 版
  `fetchRecentThreads`；IPC 带 scope stamp。
- renderer：`favorites-ingress`（§7）；入口 1 = `ConversationHeaderTitle.tsx`
  紧邻 Pin（lucide `Star`/`StarOff`）；入口 2 = Favorites tab 行内取消
  （`ThreadRailRow` accessory 扩展，`[Unfavorite, Archive]`）；第三 tab +
  方向键 + 空态；All/Chats feed seq-cursor + range-fill；Favorites =
  snapshot 整替（§7.4 原子接受单元）；i18n 四条。

### 6.2 iOS

- Core：`GaryxFavoritesState`（§7 reducer + 退避 effect）；transport 按 §5；
  Feeds/Pager：All/Chats seq-cursor + range-fill，Favorites snapshot 整替；
  `.favorites` case 全链；snapshot 客户端。
- App：入口 1 长按（紧邻 Pin，`star`/`star.slash`）、入口 2 线程内 title
  菜单（:942 附近）、入口 3 过滤器自动带出；`+ThreadPersistence` IO 薄层；
  `+ThreadList` refresh 增拉 favorites + All feed 辅助刷新穷尽 switch
  （RefreshCommitTests :523）；`+Gateway`（:55）切换全清；archive/delete
  编排 = 今日行为 + ambiguous 报错后触发 refresh（§4.4）。
- 新 Core 文件 `xcodegen generate` + 提交 pbxproj；验证 `xcodebuild`。

## 7. 客户端收藏状态机规格（双端共同契约）

### 7.1 全局状态与身份围栏

- `raw`（revision 单调 + scope 围栏；来源 GET/写响应/409 页/404 页/
  snapshot）；写派发前置；flight 三元组，token allocator 永不重置；
  transport = §5；`presented(id) = latestDesired[id]?.desired ??
  raw.contains(id)`。
- **`unresolvedFence[id]: Option<revision>`（round11-7 新增，正交状态）**：
  该线程存在「未消歧的可能已提交写」时的围栏水位。置位：ambiguous settle →
  `min(现值, flight.expectedRevision)`。清除：观察到 accepted revision >
  fence，或本端该线程的后续 CAS 写被接受（返回 revision 必 > fence）。
  **不随 toggle / phase 覆盖而清除**（独立于 generation 与 phase 生存）。

### 7.2 意图 reducer（v12 修订）

```
inFlight      = { requestToken, target, flightGeneration, expectedRevision,
                  origin: ordinary | verify }
latestDesired = { generation, desired,
                  phase: active
                       | retryScheduled(effectToken, cause)
                       | awaitVerify(effectToken) }
unresolvedFence[id]（§7.1，正交）
```

（v11 的 phase 内嵌 fence 移除——fence 统一由正交的 `unresolvedFence`
承担，v11 round10-4 的「probe 保留 fence」由此自然成立且不再依赖 origin
传递。）

**等值退休门（贯穿所有分支）**：任何「raw == desired → 退休」的判定，
**要求 `raw.revision > unresolvedFence[id]`（或 fence 为空）**；未过门 →
保持当前相位继续等待（round11-7 反例封闭：GET false@E 不能在 fence=E 未过
时退休意图）。

**事件与转移：**

- `toggle(desired)`：generation += 1；latestDesired = {gen, desired, active}
  （旧 effectToken 自然失配；**unresolvedFence 不动**）；无 inFlight 且 raw
  就绪 → dispatch。
- `dispatch`：inFlight = {新 token, desired, gen, E = raw.revision, origin}；
  单飞行。
- `settle(ok, page)`：页入 raw（其 revision > 派发时 E ≥ fence ⇒ fence
  清除）。gen ≤ flightGen → 退休；否则 desired ≠ raw → active + drain；
  相等 → 退休（fence 已清，门自动通过）。
- `settle(conflict, page)`：409 页入 raw；desired ≠ raw → active + drain
  （新 E）；相等 → **过等值门才退休**，未过门 → awaitVerify(新
  effectToken) + 退避 effect。
- `settle(notFound, page)`：404 页入 raw → 线程已不存在，退休（presented
  false，无复活）。
- `settle(definitiveRejected, retryable=false)`（tagged 400/401/403）：只
  结算 flightGeneration——同代退休 + surface error；更新代保留 active +
  drain。
- `settle(definitiveRejected, retryable=true)`（tagged 429/unavailable）：
  → retryScheduled(新 effectToken, .rejected)。
- `settle(ambiguous)`：**置 unresolvedFence** → awaitVerify(新 effectToken)
  + 退避 effect。
- `settle(notSent)`：→ retryScheduled(新 effectToken, .notSent)（fence 如有
  则已在正交状态，不丢失——round10-4/round11-7 一并成立）。
- `rawAccepted(page)`：更新 raw；`revision > fence` → 清 fence。对无
  inFlight 的意图：
  - **active**：desired == raw 且过等值门 → 退休；≠ → **立即 drain**；
  - **awaitVerify**：fence 已清 → desired == raw → 退休、≠ → active +
    drain；fence 未清 → 保持等 timer；
  - **retryScheduled（round11-8）**：desired == raw 且过等值门 → 退休；
    ≠ → **保持原 effectToken 等 timer**（不 drain，不绕过退避）。
- `backoffFired(stamp)`：四元 (scope, epoch, generation, effectToken) 全
  匹配 → 重派（同 desired，E = 当前 raw.revision，origin=verify 当来自
  awaitVerify）。
- `gatewayScopeCleared`：全清（含 unresolvedFence）+ epoch bump。

**round11-7 反例走查**：raw=false@E；PUT(g1,true) ambiguous → fence=E，
awaitVerify；toggle g2=false → active（fence 仍 =E）；DELETE notSent →
retryScheduled；GET false@E → 等值门要求 revision>E，未过 → **不退休**，
保持等 timer；timer → 重派 DELETE(E)——CAS：DELETE 接受则 revision>E
（fence 清、orphan 被围栏）；orphan 先提交则 DELETE 409 拿新页重派。终态
false = 最后意图。✅

### 7.3 Favorites feed 呈现过滤（不变）

行仅当 `presented(id) == true` 才渲染（缓存行与 in-flight 响应行一律后置
过滤）。

### 7.4 feed 协议

**Favorites（round11-4 原子接受单元）**：snapshot 包 = `(scope, epoch,
ticket, revision)` 门控的整体——`revision < highestObserved` 或 scope/
epoch/ticket 失配 → **membership 与 rows 整包丢弃** + 置 trailing-dirty；
接受 → raw 与 display rows 一次性原子替换。触发（写确认、周期 10s、下拉、
切 tab）在 in-flight 期间 → 置 trailing-dirty，settle 后补拉一次。失败保
缓存。无 load-more。

**All/Chats（seq range-fill）**：

- load-more：seq keyset（删除不移位、不跳行）。
- refresh = range-fill 链：从首页起链式拉取，**页尾 seq ≤ 旧头 seq 即止**
  （seq 全局唯一严格递增 ⇒ 无并列边界）；`has_more=false` 先到 → 整替。
  终止后原子提交：新区间行 + 旧列表 dedup 拼接。
- **正确性（v12 重述）**：自上次加载以来每个新增/变更行都获得 > 旧头 seq
  的新 seq ⇒ 必落在拉取区间。链进行中再变更的行获得 > 本链首页头 seq 的
  seq ⇒ 本链可能错过，但**下轮 range-fill 必捕获**（其 seq > 下轮的旧头
  seq）——「≤1 刷新周期有界陈旧」，写入测试断言；期间该行或以旧位置显示
  （已加载）或暂不可见（未加载，今日 parity）。
- 并发：每 feed 单飞行道（链与 load-more 互斥）；链单 ticket 全成才原子
  提交，任一页失败整链丢弃；K=5 超限 → 整替 bump epoch。
- 幽灵尾行（远端删除/滤镜退出）：整替触发点有界收敛（下拉、前台、K 超限、
  每 M=30 周期）。

## 8. 测试计划

**Gateway**

- favorites CAS/meta/清理点/守卫插入/同快照组页（含 404 带页）；孤儿写
  交错。
- **activity_seq**：投影写事务内分配严格递增（并发写序列断言）；一次性
  回填迁移保序 + cutover 幂等；**round11-1 RMW 时序下排序仍单调**（旧
  时间戳写回不影响 seq 序）；索引迁移 EXPLAIN 契约。
- seq keyset：删除后不跳行；LIMIT N+1 边界；limit/cursor 校验 400
  tagged；page+count 一次读快照。
- snapshot：同快照（commit-between）；cap 500/truncated；空收藏。
- **auth middleware tagged**：401/403 kind/operation 断言；favorites 端点
  非 200 全 tagged；遗留端点 untagged 现状回归（不迁移）。

**传输契约（R1）**

- attempt 计数；分类矩阵（tagged 双匹配、untagged 401 → ambiguous 在
  middleware 迁移后不再出现于 favorites 路径、「JSON 但非端点响应」→
  ambiguous、PATCH、web-api.ts、notSent）；无默认语义编译断言。

**意图 reducer（双端）**

- 全分支 × generation × 相位矩阵 + **等值退休门**：R11-7 走查（fence 存续
  跨 toggle/notSent，GET@E 不退休，timer 重派收敛）；R11-8（retryScheduled
  在 raw≠desired 时不被 rawAccepted drain，等 timer）；R10 全反例回归
  （probe fence 经正交状态自然保留）；R5 双序 / R7 丢失 409 / R9 搁浅；
  终端拒绝不吞新代；404 带页无复活；backoff 四元失配；活性；切网关全清
  （含 fence）。

**feed（双端）**

- **snapshot 原子接受单元（R11-4）**：旧 snapshot 晚到（revision 低）→
  整包丢弃 + trailing-dirty 补拉；写响应先接受 raw 后旧 snapshot 到 →
  不撕裂（B 不消失）；in-flight 触发合并 → settle 后补拉。
- range-fill：多页新增收敛；**链中移动行 ≤1 周期捕获**（R11-3 边界断言）；
  has_more=false 整替；K 超限；链失败整链丢弃；互斥两方向 + 两种完成顺序；
  幽灵行在整替触发点消失。
- presented 过滤全场景；Favorites 整替无 skeleton/失败保缓存。
- **archive/delete 今日 parity**：ambiguous 报错后触发 refresh 的三路径
  （已提交 → 行消失；未提交 → 列表不变；refresh 失败 → 下轮周期收敛）。

**其余**

- desktop：三 tab、方向键、空态、行 accessory；判别联合映射；store 隔离。
  iOS：FilterStorage；Reducer/Actor/Presentation；RefreshCommitTests。
  端到端：curl 三端点 + snapshot + seq 翻页 + range-fill；双端 UI 按
  `garyx-product-ui` 走查两处入口 + 筛选切换 + 行内取消。

## 9. 实现切分（五步提交，同仓同发无跨版本兼容）

1. **gateway-favorites**：表/CAS/API（404 带页 + tagged）+ snapshot +
   auth middleware tagged + 服务端测试。
2. **gateway-recent**：activity_seq（列 + meta + 分配 + 回填迁移 + 索引）+
   seq keyset cursor + bot reader 内部 offset 保留。
3. **传输契约**：iOS + desktop（含 web-api.ts）语义重构。
4. **双端状态机与 feed**：`GaryxFavoritesState` / `favorites-ingress`
   （unresolvedFence + 等值门）+ All/Chats range-fill + Favorites snapshot
   原子接受（先测后 UI）。
5. **UI**：desktop renderer / iOS App 入口与 tab + xcodegen。

**独立后续任务（已立项）**：生命周期端点幂等化（operation token / delete
tombstone / 清理 outbox / Core 重试状态机）——另行设计与评审。
