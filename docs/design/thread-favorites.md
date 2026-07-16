# Thread Favorites (线程收藏)

Status: draft v14 (addressing review round 13 — 2×P0 + 1×P1)
Date: 2026-07-16

## 0. 修订记录

### Round 13 findings → v14 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | **P0** 「listener bind 前」≠「无并发 writer」：同一 data dir 可再起一个 `gateway run`（端口可覆盖），B 进程的启动 purge / `clear_stale_active_runs` 会破坏 A 进程的真实 active run 与 favorites 围栏（这也是**现存 latent bug**：今天第二个进程就能把活进程的 run 状态改成 completed） | **R5（新根因）：per-data-dir OS 独占锁**——`GaryxDbService::open` 在任何 destructive initialization 之前对 `<data_dir>/garyx.lock` 取 flock 排他锁，进程全生命周期持有；取锁失败 → 启动中止并输出明确错误（data dir 已被另一 gateway 占用）。进程退出/被杀 flock 自动释放（restart 流程天然兼容）。加锁后单进程保证成立：启动 purge 的 bump-on-change 豁免与 `clear_stale_active_runs` 前移的论证恢复有效（理由改写为「独占锁 + bind 前」双前提）；并发双进程类反例整体消灭（§3.0） |
| 2 | **P0** `server_boot_id` 只是 feed 提示，未进收藏 revision/CAS 身份域：同 URL 可切数据域（`sessions.data_dir` 可配、recovery/备份恢复），旧 high-water 毒化新店接受、旧 expected_revision 可在错误数据域提交 | **收藏身份域三元化**：①持久 **`store_incarnation_id`**（uuid，建库时生成存 meta；legacy-import recovery / 备份恢复流程**必须重新生成**——一切可能使 revision 回退的管理操作换新身份）；②进程级 `server_boot_id`；③favorites_revision。**全部 favorites 页（GET/PUT/DELETE 的 200/409/404、snapshot）携带 `{store_incarnation_id, server_boot_id, revision}`**；**mutation 必带 `expected_store_incarnation`**，失配 → 409 tagged `wrong_incarnation` + 当前身份与页（旧 expected_revision 无法在错误数据域提交）。客户端：incarnation 失配 = **scope-clear 事件**（同 gateway 切换：epoch bump、废弃全部 flight/effect/raw/high-water/feed，重拉 snapshot；意图丢弃——管理性换域不保留跨域意图，与 gateway 切换语义一致并写明）；boot_id 失配仍 = feed 整替提示（§4.0a、§7.1） |
| 3 | P1 K 超限「被挤出行必可由 load-more 触达」证明不完整：链中上移行 X（X.seq > 首页头 H > 窗口尾 L）既不在新窗口也不满足 `< L` | 撤销「全部可由 load-more 触达」断言。补 **链尾 head 复核**：链/整替完成后重拉一次首页，头行 seq > 本链首页头 seq → 置 trailing-dirty → 立即补一轮 fill（至多 1 次，仍变则交下一周期）。正确性命题重述：链中上移行在「其不再于链中途活动的首个周期」被捕获（head 复核使常见单次上移当轮闭合）；持续精准规避属对抗时序，有界陈旧不丢失（下轮 old-head < X.seq 必捕获）。补「K 超限 + 链中上移」组合交错测试（§7.4） |

### 已确认（round 13）

普通删除同事务无条件 bump 封死删除前 CAS（单 writer mutex mod.rs:363）；
归档 bump-on-change 豁免成立（tombstone/记录/投影同事务 mod.rs:644，记录写
事务内查 tombstone mod.rs:3056）；`{endpoint op, gateway_auth}` 接受集成立
（middleware 短路位置 route_graph.rs:14 / gateway_auth.rs:155）；
activity_seq 初始化/一次性迁移/数值域/多 writer CAS 数学无新问题。

### 历史轮次（要点）

- **R12→v13**：普通删除无条件 bump（P0）；meta 初始化契约；行级 seq wire
  契约（CHECK < 2^53−1）；回填独立于 import generation + UNIQUE 索引；
  K 窗口语义；lifecycle ambiguous → forceReplacement；gateway_auth 接受集。
- **R11→v12**：activity_seq 取代 last_active_at 排序；R4 拆出；
  unresolvedFence 正交化；retryScheduled 退避门；tagged 生产者清单。
- **R10→v11**：Favorites 取消分页（snapshot 整列表 cap 500）；404 带页。
  **R9→v10**：七分支矩阵；presented 谓词；bot reader 内部 offset。
  **R8→v9**：根因三件套（传输契约/组合快照/keyset），删 marker/gate。
  **R7→v8**：awaitVerify。**R5→v6**：CAS 写围栏。**R4→v5**：三元组围栏。
  **R3→v4**：双身份。**R2→v3**：meta/同快照/清理点。**R1→v2**：守卫插入、
  判别联合、双端入口。

## 1. 需求

用户需求（产品裁决，不可改动）：

1. 线程支持「收藏」。
2. 筛选类别：**全部（All）/ Chats / 收藏（Favorites）**。
3. iOS：行长按收藏项；线程内右上角菜单（与置顶相邻）；过滤器加收藏类别。
4. Mac：触发点与置顶一致；筛选加收藏 tab。

**系统级连带改造（用户裁决授权，改根因并说清楚）**：
R1 传输契约（+auth middleware tagged）、R2 组合快照端点、R3 keyset 分页 +
activity_seq 排序、**R5 per-data-dir 独占锁 + store incarnation 身份**。
（R4 生命周期幂等化 = 独立后续任务，已立项。）

## 2. 目标 / 非目标

**目标**：收藏 + 双端入口 + 三分类；R1/R2/R3/R5 根治；客户端 = 意图状态机 +
presented 过滤 + range-fill。

**非目标**：收藏排序/重排；收藏分页（cap 500 + truncated）；首页独立收藏段；
All/Chats 行星标；SSE；bot 命令面收藏筛选；意图跨进程持久化；跨 store
incarnation 保留意图；`serverIdempotencyKey`；生命周期幂等化（独立任务，
本设计今日 parity + ambiguous 后 forceReplacement）。

## 3. 数据模型与进程模型（gateway）

### 3.0 per-data-dir 独占锁（R5，round13-1）

- `GaryxDbService::open` 最先动作：`<data_dir>/garyx.lock` flock 排他锁，
  **先于 schema init / legacy import / 启动 purge / clear_stale_active_runs
  等一切 destructive initialization**；进程全生命周期持有，退出自动释放。
- 取锁失败 → 启动中止，错误信息指明 data dir 被占用（含提示查看已运行
  gateway）。CLI 的第二个 `gateway run`（无论端口）被此挡住。
- 单进程保证成立后：启动 purge 与 `clear_stale_active_runs` 的「无并发
  HTTP/writer」前提 = 独占锁 + listener bind 前双重保障。
- 此锁同时修复现存 latent bug：第二进程启动即把活进程的 active run 改
  completed（mod.rs:1016 无条件 UPDATE）。

### 3.1 favorites（CONFIRMED + v13 P0 修订）

- `thread_favorites` + `thread_favorites_meta`（meta 初始化早于启动 purge；
  守卫式单事务插入；不用 FK）。
- revision bump：条件写接受即恒 bump（含 no-op）；**普通线程删除成功同事务
  无条件 bump**（CONFIRMED）；归档 bump-on-change（tombstone 豁免，
  CONFIRMED）；启动 purge bump-on-change（独占锁 + bind 前，v14 理由）。
- 清理点三处不变（mod.rs:669/2045/2785）。

### 3.2 activity_seq（v13 契约，CONFIRMED 部分不再重复）

- meta 初始化在 `initialize_connection` schema 段（先于 legacy import）；
  分配在投影写事务内；公开 `upsert_recent_thread` 显式事务。
- 回填 = 独立 durable marker 的真一次性迁移；meta 永不下降、recovery 不
  触碰；回填后建 `UNIQUE INDEX (activity_seq)`；`CHECK < 9007199254740991`；
  JSON number；desktop safe-integer 校验复用；iOS Int64。
- `clear_stale_active_runs`：listener bind 前 + 独占锁下执行，不分配新 seq
  （已结束 run 不跳顶）；**source-guard**：直接 UPDATE recent_threads 的
  call-site allow-list（今后新路径要么分配 seq 要么 bind 前运行）。

### 3.3 store 身份（v14 新增，round13-2）

```sql
-- 建库时一次生成（meta 表或专用单行表）：
store_incarnation_id TEXT NOT NULL   -- uuid
```

- **重新生成时机**：一切可能使 favorites revision 连续性失效的管理操作——
  legacy-import 手动 recovery（随 projection-state 行清除一并换新）、从
  备份恢复数据目录（写入 recovery 文档步骤）。程序内正常运行永不改变。
- `server_boot_id`：进程级 uuid（不落盘）。

## 4. Gateway API

### 4.0 tagged 错误 schema 与身份域

- tagged schema、生产者清单、`{endpoint op, gateway_auth}` 接受集：同 v13
  （CONFIRMED）。
- **（v14）favorites 身份域**：**全部 favorites 响应**（GET / PUT / DELETE
  的 200、409、404 页、snapshot）携带
  `{ store_incarnation_id, server_boot_id, revision }`。
- **mutation 必带 `expected_store_incarnation`**（与 `expected_revision`
  并列）；失配 → 409 tagged `code="wrong_incarnation"` + 当前完整身份与
  membership 页（同快照）。CAS 判定顺序：先 incarnation 后 revision。

### 4.1 收藏读写（CAS）——同 v13 + 身份字段

`PUT/DELETE …?expected_revision=N&expected_store_incarnation=<uuid>`；
200 接受即 bump 含 no-op；409（revision 或 incarnation）/404 tagged +
同快照页 + 身份；400 tagged。

### 4.2 `/api/recent-threads`（All/Chats）——同 v13

cursor-only；行级 `activity_seq`；响应带 `server_boot_id`（+
`store_incarnation_id`，v14 统一）；bot reader 内部 offset 保留。

### 4.3 组合快照端点——同 v13 + 身份字段

单读事务 `{ store_incarnation_id, server_boot_id, revision, thread_ids,
favorites, recent: { threads, total, truncated } }`；cap 500。

### 4.4 生命周期操作——同 v13

服务端不动；客户端 ambiguous → forceReplacement；承诺边界「快照前提交 →
行消失；之后 → 下周期收敛」。

## 5. 传输契约重构（R1）——同 v13

semantic mode 全 helper 必填（含 PATCH、web-api.ts）；四分类；definitive
按接受集判定；迁移清单以「一切直接 HTTP 调用」为界。

## 6. Desktop / iOS 落点——同 v13，增量

- 契约增量：favorites 页与 snapshot 的身份三元组；mutation 带
  `expected_store_incarnation`；per-feed 存 `(store_incarnation_id,
  server_boot_id)`。
- 其余同 v13（两处入口、第三 tab、行 accessory、range-fill、snapshot 整替、
  `+Gateway` 切换全清、xcodegen）。

## 7. 客户端收藏状态机规格

### 7.1 全局状态与身份围栏（v14 扩展）

- raw / 写派发前置 / flight 三元组 / transport / presented / unresolvedFence
  与等值退休门：同 v13（CONFIRMED）。
- **身份域（round13-2）**：客户端保存当前
  `(gatewayScope, store_incarnation_id)`；**incarnation 失配 = scope-clear
  事件**——原子执行：runtime epoch bump（废弃全部在途 flight 响应与退避
  effect）、清 raw/high-water/unresolvedFence/意图/feed，随后重拉 snapshot
  建立新身份基线。**意图不跨 incarnation 保留**（管理性换域，与 gateway
  切换清场语义一致）。`server_boot_id` 失配 = feed 整替提示（不清意图）。
- 旧 boot/旧 incarnation 的迟到响应：所有 favorites 页带身份三元组，接受
  前先验 incarnation（失配整体判弃或触发 scope-clear——若来自"更旧"身份则
  判弃）；flight 结算的 scope/epoch 围栏继续兜底。

### 7.2 意图 reducer——同 v13（CONFIRMED），增量一条

- `settle(conflict, code=wrong_incarnation)`：不入常规 conflict 分支——
  触发 §7.1 的 incarnation scope-clear（意图随场清除）。

### 7.3 presented 过滤——同 v13。

### 7.4 feed 协议（v14 增量）

- Favorites：snapshot 原子接受单元升级为
  `(gatewayScope, store_incarnation, epoch, ticket, revision)`；低
  revision（同 incarnation 内）整包丢弃 + trailing-dirty；incarnation 失配
  → scope-clear 后重拉。
- All/Chats：load-more seq keyset；refresh = range-fill 链（页尾 seq ≤
  旧头 seq 即止；has_more=false → 整替）；K=5 超限 = 链 K 页为新窗口。
- **链尾 head 复核（round13-3）**：链/整替提交后**重拉一次首页**：头行
  seq > 本链首页头行 seq → trailing-dirty → 立即补一轮 fill（至多一次，
  仍变化则交下一周期）。正确性命题（v14 重述）：链中上移行在其不再于链
  中途活动的首个周期被捕获；「全部被挤出行可由 load-more 触达」断言撤销
  ——被挤出且链中上移的行由 head 复核/下一周期捕获，有界陈旧、不丢失。
- 并发：每 feed 单飞行道（head 复核属链 ticket 的一部分）；整替 bump
  epoch；boot_id/incarnation 失配整替或清场。
- 幽灵尾行：整替触发点有界收敛（下拉、前台、K 超限、每 M=30 周期、
  boot_id 失配、lifecycle ambiguous）。

## 8. 测试计划

**Gateway**

- **独占锁（R5）**：双进程同 data dir——后者 open 即失败（bind 前）；锁
  释放后可启动；restart 流程正常；锁先于 purge/clear 的顺序断言。
- favorites：CAS/meta/守卫插入/同快照组页（404 带页）；孤儿写交错；
  round12-1 删除交错（无条件 bump）；**incarnation CAS**：错 incarnation
  的 mutation → wrong_incarnation 409 零副作用；recovery 换 incarnation
  后旧 expected_revision 不可提交。
- activity_seq：全套（同 v13）+ source-guard + bind 前顺序断言。
- keyset/snapshot/auth tagged：同 v13 + 响应身份三元组断言。

**传输契约（R1）**：同 v13。

**意图 reducer（双端）**

- 全矩阵 + 等值门（同 v13 回归）；**wrong_incarnation → scope-clear**
  （意图清除、epoch bump、旧退避 effect 失配丢弃）；旧身份迟到页判弃。

**feed（双端）**

- snapshot 原子接受（含 incarnation 维度）；range-fill 全场景 +
  **「K 超限 + 链中上移」组合交错**（head 复核补拉捕获；至多一次立即
  补拉；持续变化交下周期）；boot_id 失配整替；lifecycle ambiguous →
  forceReplacement 三路径；presented 全场景；幽灵行收敛。

**其余**：desktop/iOS UI 与既有回归；端到端 curl（含 wrong_incarnation
路径）+ 双端走查（`garyx-product-ui`）。

## 9. 实现切分（六步，同仓同发）

1. **gateway-lock**：per-data-dir flock（R5）+ 启动顺序（锁 → schema →
   import → purge/clear → bind）+ store_incarnation_id。
2. **gateway-favorites**：表/CAS（revision + incarnation 双围栏）/API
   （404 带页 + tagged + 身份三元组）+ snapshot + auth middleware tagged +
   delete 无条件 bump。
3. **gateway-recent**：activity_seq 全套 + seq keyset + 行级 seq + boot_id
   + source-guard。
4. **传输契约**：iOS + desktop（含 web-api.ts）。
5. **双端状态机与 feed**：reducer（+wrong_incarnation 清场）+ range-fill
   （K 窗口 + head 复核）+ snapshot 原子接受 + boot_id/incarnation 处理 +
   lifecycle forceReplacement（先测后 UI）。
6. **UI**：入口与 tab + xcodegen。

**独立后续任务（已立项）**：生命周期端点幂等化。
