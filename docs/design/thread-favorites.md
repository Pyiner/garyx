# Thread Favorites (线程收藏)

Status: draft v13 (addressing review round 12 — 1×P0 + 7×P1)
Date: 2026-07-16

## 0. 修订记录

### Round 12 findings → v13 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | **P0** 普通 delete 在无 favorite 行时不 bump revision + 无 tombstone 可被旧 writer 复活 ⇒ 孤儿 PUT 在 404 退休后仍可用旧 expected_revision 落库，反转最后意图 | **普通线程删除成功无条件 bump `favorites_revision`**（同一删除事务内，无论是否删到 favorite 行）——删除后 revision 必前进，一切携带删除前 expected 的旧写被 CAS 拒绝；随后的复活写不影响（favorite 插入已被围栏）。归档维持 bump-on-change（tombstone 已防复活 + 记录消失使守卫插入 NotFound）；启动 purge 维持 bump-on-change（运行于 listener bind 前，无并发 HTTP 写在途）。两处豁免理由写入代码注释；加「delete→404 退休→复活→孤儿 PUT 被 409」确定性交错测试（§3.1） |
| 2 | P1 `recent_threads_meta` 无初始化契约；真实启动序 = schema init → legacy import 写投影 → cutover 最后跑；公开 `upsert_recent_thread`（mod.rs:1709）在普通连接上调 helper | `ensure_recent_threads_meta_row`（`INSERT (1,0) ON CONFLICT DO NOTHING`）进 `initialize_connection` 的 schema 段（open() 先于一切 import/upsert，legacy_boot_import 的 set 路径自然拿到已初始化 meta）；公开 `upsert_recent_thread` 改显式事务包裹；测试覆盖 fresh DB / legacy import / 重开幂等（§3.2） |
| 3 | P1 `clear_stale_active_runs` 直接 UPDATE 绕过 seq；且在 listener bind 后异步 warmup，跨重启旧 feed 与 warmup 窗口都会持有陈旧 run_state 的深处行 | 双修：①**warmup 提前到 listener bind 之前**（单条 SQL，代价可忽略——窗口本身消除）；②**服务端 boot incarnation**：recent-threads 与 snapshot 响应携带 `server_boot_id`（每 gateway 进程一个 uuid），客户端 per-feed 存储，任何响应失配 → 强制整替——跨重启的一切投影级修正（含该 UPDATE）由整替收敛，不需要给修正行分配新 seq（否则已结束的 run 会错误跳到列表顶部）（§3.2、§4.2、§7.4） |
| 4 | P1 range-fill 锚点无 wire contract（cursor opaque、行无 seq；desktop i64→JS number 安全性） | **行级 `activity_seq` 进响应契约**：`RecentThreadRecord` / desktop `thread.ts` summary / iOS summary 增字段；客户端用行 seq 计算锚点（页尾 vs 存储的旧头）。**数值域裁决**：DB 加 `CHECK (activity_seq < 9007199254740991)`（2^53−1；每天百万次活动也够数千年），JSON 以 number 发出，desktop 复用既有 safe-integer 校验（http.ts:262），iOS Int64；契约测试钉界（§4.2） |
| 5 | P1 回填若随 import generation 重跑会重置单调序号，旧 head/cursor 全错 | 回填 = **独立于 import generation 的真一次性迁移**（自有 durable marker，永不随 recovery generation 重跑）；**meta 计数器单调不变量：永不下降、recovery 不触碰**——恢复导入的行走正常 allocator（seq 继续上行）；回填完成后建 **`UNIQUE INDEX (activity_seq)`** 把全局唯一性落成 DB 约束；测试含 generation-2 recovery + 旧 cursor/head 仍有效（§3.2） |
| 6 | P1 K=5 超限整替与「≤1 周期」证明不兼容（X 被挤出窗口后 range-fill 永不主动捕获） | 定义 K 超限整替 = **以链已取的 K 页为新窗口**（cursor = 第 K 页尾，has_more=true）：被挤出的行落入窗口下方，**由 load-more 正常触达**（其 seq < 第 K 页尾）。正确性命题限定：「≤1 周期捕获」适用于单周期变更量 ≤ K×N；超过时退化为标准窗口分页语义（无丢失、可分页触达）。测试按此唯一预期编写（§7.4） |
| 7 | P1 lifecycle ambiguous 后的普通 refresh 兑现不了「已提交→行消失」（range-fill 不删幽灵尾行；现有客户端 error 路径还会回滚本地删除） | §4.4 改为显式 **forceReplacement**（整替，非 range-fill；All/Chats 整替 + favorites snapshot）；承诺降级并写明：提交发生在整替快照前 → 行消失；之后 → 下一周期整替收敛。现有 error 回滚行为保留（回滚后由整替裁决）（§4.4、§7.4、测试） |
| 8 | P1 `gateway_auth` operation 与「双匹配」无可执行判定规则（严格匹配→401 永 ambiguous；任意接受→失去防伪目的） | 判定规则：每个请求的 definitive 接受集 = **`{本端点 operation} ∪ {"gateway_auth"}`**，且 `gateway_auth` 仅接受 `code ∈ {unauthorized, forbidden}` → 终端拒绝分支。消费者契约测试：favorites mutation 收到 gateway_auth 401 → terminal reject（不循环）（§4.0） |

### 已确认（round 12）

activity_seq 核心数学（单 writer 事务内分配严格递增；回填保序；keyset 谓词）；
unresolvedFence 封住 v11 全反例，纯 reducer 矩阵无新时序；snapshot
原子接受单元成立；投影/JOIN 合规；Core/App 分层合规。

### 历史轮次（要点）

- **R11→v12**：activity_seq 取代 last_active_at 排序；R4 拆出独立任务
  （今日 parity 人工重试）；unresolvedFence 正交化；retryScheduled 不被
  rawAccepted 绕过；tagged 生产者清单。
- **R10→v11**：Favorites 取消分页（snapshot 整列表，cap 500）；range-fill；
  404 带页；tagged union。**R9→v10**：七分支矩阵；presented 谓词；bot
  reader 内部 offset。**R8→v9**：根因三件套，删 marker/gate。**R7→v8**：
  awaitVerify。**R5→v6**：CAS 写围栏（CONFIRMED）。**R4→v5**：三元组围栏。
  **R3→v4**：双身份。**R2→v3**：meta/同快照/清理点（CONFIRMED）。
  **R1→v2**：守卫插入、判别联合、双端入口。

## 1. 需求

用户需求（产品裁决，不可改动）：

1. 线程支持「收藏」。
2. 筛选类别：**全部（All）/ Chats / 收藏（Favorites）**。
3. iOS：行长按收藏项；线程内右上角菜单（与置顶相邻）；过滤器加收藏类别。
4. Mac：触发点与置顶一致；筛选加收藏 tab。

**系统级连带改造（用户裁决授权，改根因并说清楚）**：R1 传输契约（+auth
middleware tagged）、R2 组合快照端点、R3 keyset 分页 + activity_seq 排序 +
boot incarnation。（R4 生命周期幂等化 = 独立后续任务。）

## 2. 目标 / 非目标

**目标**：收藏 + 双端入口 + 三分类；R1–R3 根治；客户端 = 意图状态机 +
presented 过滤 + range-fill。

**非目标**：收藏排序/重排；收藏分页（cap 500 + truncated）；首页独立收藏段；
All/Chats 行星标；SSE；bot 命令面收藏筛选（bot reader 内部 offset）；意图
跨进程持久化；`serverIdempotencyKey`；生命周期幂等化（独立任务；本设计
今日 parity + ambiguous 后 forceReplacement）。

## 3. 数据模型（gateway SQLite）

### 3.1 favorites（round 6 起 CONFIRMED + v13 P0 修订）

- `thread_favorites` + `thread_favorites_meta`（同 v12；meta 初始化早于启动
  purge；守卫式单事务插入；不用 FK）。
- revision bump 规则（v13 修订）：
  - 条件写（PUT/DELETE）接受即恒 bump（含 no-op）；
  - **普通线程删除成功：同事务无条件 bump**（round12-1——围栏一切删除前
    的旧写；复活写不再能配上旧 expected_revision）；
  - 归档：bump-on-change（tombstone 防复活 + 记录消失使守卫插入 NotFound，
    注释写明豁免理由）；
  - 启动 purge：bump-on-change（listener bind 前运行，无并发 HTTP 写，
    注释写明）。
- 清理点三处不变（mod.rs:669/2045/2785）。

### 3.2 activity_seq（v13 契约补全）

```sql
ALTER TABLE recent_threads ADD COLUMN activity_seq INTEGER NOT NULL DEFAULT 0
  CHECK (activity_seq >= 0 AND activity_seq < 9007199254740991);
CREATE TABLE IF NOT EXISTS recent_threads_meta (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  activity_seq INTEGER NOT NULL CHECK (activity_seq >= 0)
) STRICT;
-- 回填完成后：
CREATE UNIQUE INDEX idx_recent_threads_activity_seq
  ON recent_threads (activity_seq DESC);
-- task / non-task partial 索引同步改 (activity_seq DESC)
```

- **meta 初始化（round12-2）**：`ensure_recent_threads_meta_row` 在
  `initialize_connection` schema 段执行（open() 先于 legacy import 与一切
  upsert）；分配在投影写事务内 `++meta`；公开 `upsert_recent_thread`
  （mod.rs:1709）改显式事务。
- **回填迁移（round12-5）**：独立于 import generation 的**真一次性**迁移
  （自有 durable marker）；按 `(last_active_at ASC, thread_id DESC)` 保序
  回填；meta 置为最大值；**meta 单调不变量：永不下降，recovery 不触碰**
  （恢复导入行走正常 allocator）；回填后建 UNIQUE 索引。
- **数值域（round12-4）**：CHECK < 2^53−1；JSON number 发出；desktop 复用
  safe-integer 校验（http.ts:262）；iOS Int64。
- **投影级直改路径（round12-3）**：`clear_stale_active_runs`（mod.rs:1011）
  **提前到 listener bind 之前**执行（消除 warmup 窗口），不分配新 seq
  （已结束 run 不应跳顶）；跨重启陈旧由 boot incarnation 收敛（§4.2）。
  规约：**今后任何直接 UPDATE recent_threads 的路径要么分配新 seq、要么
  仅允许在 listener bind 前运行**，写入代码注释与 source-guard 测试
  （扫描 call-site allow-list，先例 = agent-enable 的 source-guard 模式）。

## 4. Gateway API

### 4.0 tagged 错误 schema（v13：判定规则闭合）

- 生产者（本设计内）：auth middleware（401/403，operation="gateway_auth"）+
  favorites 全端点 + recent-threads 参数错误。遗留端点不迁移（untagged 恒
  ambiguous，仅影响 favorites reducer；遗留调用方不依赖 definitive 分类）。
- **消费者判定（round12-8）**：请求的 definitive 接受集 =
  `{本端点 operation} ∪ {"gateway_auth"}`；`gateway_auth` 仅当
  `code ∈ {unauthorized, forbidden}` 时接受 → 映射终端拒绝。其余
  kind/operation/code 组合一律 ambiguous。契约测试：favorites mutation 收
  gateway_auth 401 → terminal reject。

### 4.1 收藏读写（CAS）——CONFIRMED（同 v12）

GET / PUT / DELETE + `expected_revision` 必填；200 接受即 bump 含 no-op；
409/404 tagged + 同快照 membership 页；400 tagged。

### 4.2 `/api/recent-threads`（All/Chats；v13 契约补全）

- cursor-only；`tasks` 不变；`limit` 1..cap；cursor opaque
  `{v, filter, activity_seq}`；跨滤镜/解码失败 400 tagged。
- keyset：`activity_seq < :seq ORDER BY activity_seq DESC LIMIT N+1`；
  `next_cursor` 从末条实际返回行签发；`total` 同快照仅展示。
- **行级 `activity_seq` 进响应**（round12-4）：每行携带 seq（number），
  三层模型（`RecentThreadRecord`/desktop summary/iOS summary）加字段。
- **`server_boot_id`**（round12-3）：进程级 uuid，随 recent-threads 与
  snapshot 响应返回；客户端 per-feed 存储，失配 → 强制整替。
- bot reader 内部 offset 保留（internal-only，行为=今日）。

### 4.3 组合快照端点（R2）——CONFIRMED（同 v12 + boot_id）

单读事务 `{ revision, thread_ids, favorites, recent: { threads(含
activity_seq), total, truncated }, server_boot_id }`；cap 500。

### 4.4 生命周期操作（v13：forceReplacement）

服务端行为与错误契约不动（幂等化=独立任务）。客户端编排：ambiguous 结局在
既有报错/回滚之后**触发 forceReplacement**（All/Chats 整替 + favorites
snapshot，非 range-fill——range-fill 不删幽灵尾行，round12-7）。承诺边界：
提交发生在整替快照前 → 行消失；之后 → 下一周期整替收敛。

### 4.5 已知边界

automation/hidden 不进投影；归档清 favorites 行 + bump-on-change；普通删除
无条件 bump（§3.1）。

## 5. 传输契约重构（R1）——同 v12

semantic mode 全 helper 必填（含 PATCH、web-api.ts）；mutation 四分类；
definitive 仅认 tagged 且按 §4.0 接受集判定；迁移清单以「一切直接 HTTP
调用」为界。

## 6. Desktop / iOS 落点（同 v12，增量）

- 契约增量：summary 带 `activity_seq`（safe-integer 校验）与响应级
  `server_boot_id`；feed 状态存 per-feed boot_id。
- desktop：main 纯 raw；判别联合；favorites 三端点 + snapshot + seq-cursor；
  renderer favorites-ingress、两处入口（header 菜单紧邻 Pin +
  Favorites tab 行内 `[Unfavorite, Archive]` accessory）、第三 tab、
  All/Chats range-fill、i18n。
- iOS：Core `GaryxFavoritesState` + transport 语义 + Feeds/Pager seq-cursor
  与 range-fill + `.favorites` 全链 + snapshot 客户端；App 三入口、
  `+ThreadList` 穷尽 switch、`+Gateway` 切换全清、lifecycle ambiguous →
  forceReplacement；xcodegen + pbxproj。

## 7. 客户端收藏状态机规格（同 v12，唯 feed 协议增量）

### 7.1–7.3（同 v12，CONFIRMED）

raw/围栏/写派发前置/presented；**unresolvedFence 正交状态 + 等值退休门**；
意图 reducer 全分支（ok/conflict/notFound/definitiveRejected×2/ambiguous/
notSent/rawAccepted/backoffFired/scopeCleared）；Favorites feed presented
后置过滤。

### 7.4 feed 协议（v13 增量）

**Favorites**：snapshot 原子接受单元 `(scope, epoch, ticket, revision)`
（低 revision 整包丢弃 + trailing-dirty 补拉）；**boot_id 失配 → 立即重拉
snapshot**（favorites 无 cursor，重拉即收敛）。

**All/Chats**：

- load-more：seq keyset。
- refresh = range-fill 链（页尾行 seq ≤ 旧头行 seq 即止；行级 seq 来自
  §4.2 响应契约）；`has_more=false` → 整替。
- **K=5 超限（round12-6 语义钉死）**：新窗口 = **链已取的 K 页**（cursor =
  第 K 页尾行 seq，has_more=true），旧列表丢弃。被挤出窗口的行位于窗口
  下方，**load-more 正常触达**。正确性命题：单周期变更 ≤ K×N 时「≤1 周期
  捕获」；超过时退化为标准窗口分页（无丢失、可触达）。
- **boot_id 失配 / lifecycle ambiguous → forceReplacement**（= K 超限同款
  整替：取首 K' 页为新窗口或首页起重建，实现取一致路径）。
- 并发：每 feed 单飞行道；链单 ticket 全成才原子提交，任一页失败整链
  丢弃；整替 bump epoch。
- 幽灵尾行：整替触发点（下拉、前台、K 超限、每 M=30 周期、boot_id 失配、
  lifecycle ambiguous）有界收敛。

## 8. 测试计划

**Gateway**

- favorites：CAS/meta/守卫插入/同快照组页（404 带页）；孤儿写交错；
  **round12-1 P0 交错**：thread delete（无 favorite 行）→ 无条件 bump →
  复活写回 → 孤儿 PUT(旧 E) 409 零副作用；归档/purge 豁免路径注释与行为
  测试。
- activity_seq：meta 初始化（fresh/legacy import/重开）；事务内分配严格
  递增（并发序列断言）；`upsert_recent_thread` 显式事务；回填保序 + 独立
  marker 一次性 + **generation-2 recovery 不重置 meta、旧 head/cursor 仍
  有效**；UNIQUE 索引生效；CHECK 上界；RMW 时序下排序仍单调；
  **source-guard：直接 UPDATE recent_threads 的 call-site allow-list**；
  `clear_stale_active_runs` 在 bind 前执行的启动顺序断言。
- keyset：删除后不跳行；N+1 边界；参数校验 tagged 400；boot_id 出现在
  两端点响应。
- snapshot：同快照；cap/truncated；空收藏。
- auth middleware tagged：401/403 kind/operation/code；遗留端点现状回归。

**传输契约（R1）**

- attempt 计数；分类矩阵（接受集判定：本端点 op / gateway_auth 401 →
  terminal、错 operation → ambiguous、「JSON 非端点响应」→ ambiguous、
  PATCH、web-api.ts、notSent）；无默认语义编译断言。

**意图 reducer（双端）**

- 全分支矩阵 + 等值退休门；R11-7/R11-8/R10/R9/R7/R5 反例回归；
  **R12-1 客户端侧**：404 退休后 fence 场景不再可达（服务端 bump 围栏），
  仍保留「404 页 revision > fence」断言；backoff 四元失配；活性；切网关
  全清。

**feed（双端）**

- snapshot 原子接受（低 revision 整包丢弃/trailing-dirty/boot_id 重拉）。
- range-fill：多页新增收敛；链中移动 ≤1 周期（变更 ≤ K×N 域内）；
  **K 超限 = K 页新窗口 + 被挤出行 load-more 可触达**（round12-6 唯一
  预期）；has_more=false 整替；链失败整链丢弃；互斥 + 两种完成顺序。
- **boot_id 失配 → 整替**（重启后陈旧 run_state 深行收敛）。
- **lifecycle ambiguous → forceReplacement 三路径**（快照前提交 → 行
  消失；快照后提交 → 下周期收敛；未提交 → 列表不变）。
- presented 过滤全场景；幽灵行在整替触发点消失。

**其余**

- desktop/iOS UI 与既有回归（同 v12）；端到端 curl + 双端走查（
  `garyx-product-ui`）。

## 9. 实现切分（五步，同仓同发）

1. **gateway-favorites**：表/CAS/API + **delete 无条件 bump** + snapshot +
   auth middleware tagged。
2. **gateway-recent**：activity_seq（meta 初始化/分配/一次性回填/UNIQUE/
   CHECK）+ seq keyset + 行级 seq 契约 + `server_boot_id` + warmup 提前 +
   source-guard。
3. **传输契约**：iOS + desktop（含 web-api.ts）。
4. **双端状态机与 feed**：reducer（unresolvedFence/等值门）+ range-fill
   （K 窗口语义）+ snapshot 原子接受 + boot_id 整替 + lifecycle
   forceReplacement（先测后 UI）。
5. **UI**：入口与 tab + xcodegen。

**独立后续任务（已立项）**：生命周期端点幂等化。
