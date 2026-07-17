# Lifecycle Write Safety (生命周期写安全加固)

Status: draft v1 (pending adversarial design review)
Date: 2026-07-17
Origin: 从 thread favorites 设计（docs/design/thread-favorites.md，#TASK-2324
round 5–20）拆出的现存系统缺陷群，用户裁决单独立项、系统性根治不打补丁。

## 0. 威胁模型与范围边界（先读这节再评审）

**部署形态**：单用户单机 gateway，单 SQLite store（单 writer mutex），
data-dir 独占 flock 已上线（favorites 项目 R5）。「同 URL 换库」为重启级
管理事件；store_incarnation_id / server_boot_id / tagged 错误 schema /
传输层写单 attempt（iOS/desktop）均已上线，本设计**复用不重建**。

**本设计要根治的四类现存缺陷**（今天可复现，与收藏功能无关）：

| # | 缺陷 | 今日后果 |
|---|---|---|
| D1 | archive/delete 请求 ambiguous（超时/响应丢失）后结局不可确认：服务端 blocking 任务可在客户端放弃后才提交 | 用户看到报错但操作实际成功（或反之）；重试也无法判定 |
| D2 | 普通 delete 裸删行、无墓碑：删除前已读旧记录的 writer 可原样写回（`ensure_thread_entry` 类时序） | 已删线程幽灵复活（记录+投影+收藏位） |
| D3 | delete 的 DB 删除与 runtime/transcript/log 清理是无保护两阶段（routes.rs:1136 区域）：中间崩溃后重试见 missing 即判完成 | 残留 runtime 状态/孤儿文件永不清理 |
| D4 | `GET /api/threads/history` 读中执行 `set_logged` 持久修复（api.rs:1055 区域），违反 repository-contracts「读路由不做修复」；崩溃孤儿 queued-input 无权威清理路径（正常路径只在 run final flush，persistence_worker.rs:509；崩溃后新 run 的 merge 不清 inactive-but-queued 孤儿，persistence.rs:578） | 契约违反 + 不浏览 history 就永不清理的孤儿数据 |

**永久非目标（用户威胁模型裁决，不因评审 finding 回摆）**：
WS 握手身份、MCP path context 身份、全实体身份包络/provenance、CLI 身份
迁移、跨语言豁免 manifest、admission gate 类客户端前置。理由：收藏域已由
CAS 双围栏承重；其余写面的跨库窗口在单机部署下频率与后果均可接受，且
线程/实体 ID 为 uuid 时无关库不碰撞。评审请勿以这些面提阻塞 finding。

**其它非目标**：不改 archive/delete 的业务语义（active run/binding 冲突
409 照旧）；不加兼容层（同仓同发）；不做跨进程操作台账同步。

## 1. 架构总览：三承重件 + 两小件

统一心法（来自 favorites CAS 的推广）：**把「裸写」升级为「有身份、可
幂等重放、结局留档、副作用入队」的状态迁移**。

```
客户端                     gateway 写事务（单 writer）            异步
────────                  ─────────────────────────────         ──────
archive/delete            ① 验 expected_store_incarnation
 + operation_id      →    ② 查 lifecycle_operations[op_id]
 (ambiguous 则同 id         命中 → 返回已记录结局（零副作用）
  退避重发直到确定)         ③ 验业务前置（active run/binding…）
                          ④ 状态迁移 + 写墓碑（delete 也留）
                          ⑤ 记台账行（op_id → 结局）
                          ⑥ 排清理作业进 cleanup_outbox
                          ——以上同一事务——            →  outbox worker
                                                          幂等执行清理；
                                                          启动时重跑 pending
```

### A. 删除墓碑统一化（根治 D2）

- 现状：归档已有墓碑且记录写路径**事务内**查墓碑拒复活（mod.rs:644 区域
  写入、:3056 区域校验）；delete 走 mod.rs:2034 区域裸删。
- 方案：delete 与 archive 共用同一墓碑机制——线程终态迁移统一为
  `active → archived | deleted`，两者都在同一事务写墓碑；既有的写路径
  墓碑校验自然覆盖 delete（**复用已证明机制，不新造**）。
- 墓碑记录 `kind`（archived/deleted）供诊断；对写路径拒绝语义二者等同。
- 与 repository-contracts 一致性：现有条款「Archived-thread tombstones
  always win」推广为「terminal-state tombstones always win」，实现 PR
  同步更新契约文档（AGENTS/CLAUDE 不涉及）。
- 边界：cutover 之前已删除的线程无墓碑（历史裸删不可追溯）——接受并注明；
  cutover 起新删除全部留墓碑。墓碑不参与任何列表投影（现状同）。

### B. 幂等操作台账（根治 D1）

```sql
CREATE TABLE IF NOT EXISTS lifecycle_operations (
  operation_id TEXT PRIMARY KEY,          -- 客户端生成 uuid
  kind         TEXT NOT NULL CHECK (kind IN ('archive','delete')),
  thread_id    TEXT NOT NULL,
  outcome      TEXT NOT NULL,             -- applied_changed | applied_noop
                                          -- | rejected_conflict | rejected_not_found
  detail       TEXT,                      -- tagged 错误负载（rejected 时）
  completed_at TEXT NOT NULL
) STRICT;
```

- **写协议**：archive/delete 请求必带 `operation_id`（uuid）+
  `expected_store_incarnation`（复用 favorites 身份设施；middleware 级或
  handler 首行校验，失配 409 tagged 零副作用、**不记台账**）。
- **同事务序**：查台账（命中即返回记录的结局）→ 业务前置校验 → 状态迁移
  + 墓碑 → 记台账 → 排 outbox。全部在单 writer 事务内。
- **幂等语义**：同 operation_id 重放恒返回首次记录的结局——「迟到的首次
  attempt 在重试之后才取得 writer」的时序（favorites round-5 类反例）
  无害：先提交者记账，后到者查账返回同一结局、零二次副作用。
- **对已删目标的新操作**（不同 op_id）：`rejected_not_found`（带墓碑
  kind 供 UI 区分「已归档」/「已删除」）。
- **台账修剪**：启动恢复流程删除 `completed_at` 早于 TTL（默认 7 天）的
  行（一次 SQL，挂现有 startup 恢复族）。TTL 内客户端重试窗口远小于 7
  天（分钟级退避即弃）。修剪后同 id 重放会当新操作执行——archive/delete
  对同线程幂等（墓碑在，`applied_noop`），无害。
- **endpoint detach 时序修正**：archive 现状在 DB 事务**前** detach
  endpoint（routes.rs:3144 区域）——改为 detach 进 outbox（事务后异步、
  幂等），消除「前置检查通过后停顿、他人状态已变、仍提交」的窗口的
  detach 半截面。业务前置校验仍在事务内做最终裁决。

### C. 清理 outbox（根治 D3）

```sql
CREATE TABLE IF NOT EXISTS cleanup_outbox (
  job_id     INTEGER PRIMARY KEY AUTOINCREMENT,
  thread_id  TEXT NOT NULL,
  step       TEXT NOT NULL,   -- runtime_teardown | endpoint_detach
                              -- | transcript_remove | thread_log_remove | …
  status     TEXT NOT NULL DEFAULT 'pending'
             CHECK (status IN ('pending','done')),
  created_at TEXT NOT NULL,
  settled_at TEXT
) STRICT;
CREATE INDEX IF NOT EXISTS idx_cleanup_outbox_pending
  ON cleanup_outbox(status) WHERE status = 'pending';
```

- 生命周期事务在**同一事务**里为该迁移排全部清理步骤；HTTP 响应在事务
  提交后即可返回（清理异步）。
- **worker**：进程内单 worker 顺序执行 pending 作业；每步**幂等**（删
  不存在的文件 = 成功；拆不存在的 runtime = 成功）；成功标 done（点写）。
  失败重试带退避；持续失败留 pending + 告警日志。
- **启动恢复**：boot 时重跑全部 pending（挂在现有 startup 恢复族，
  `clear_stale_active_runs` 同级；独占锁保证无并发进程）。崩溃于任意点
  → 事务已提交则作业必在、必被重跑；事务未提交则整体未发生。D3 消除。
- 作业执行顺序按 job_id（同线程步骤有序）；跨线程无序无碍（各自幂等）。
- 显式对照：现有 delete 的同步清理链（routes.rs:1136 区域）整体迁入
  outbox steps；archive 的 detach 迁入（见 B）。

### D. 读路径去写 + 孤儿归位（根治 D4，两小件之一）

- `GET /api/threads/history` 的 `set_logged` 持久修复**删除**（读路由
  归纯）。
- 正常路径保持：run final flush 时 queued→abandoned（persistence_worker
  现状）。
- **崩溃孤儿的权威清理** = 启动恢复一次 SQL 过：将 inactive-but-queued
  的孤儿 input 记录判定并落 abandoned（判据与现 GET 内过滤逻辑等价，
  搬移非重写；boot 时 bridge run index 为空 = 全部 queued 皆孤儿，判据
  比运行时更简单）。挂 startup 恢复族。
- 既有回归测试（api/tests.rs:1778 区域「持久修复」断言）改写为「boot
  恢复后读到干净结果 + 读路由零写」两条。

### E. 生命周期身份先验（两小件之二）

archive/delete 带 `expected_store_incarnation`（同 favorites 参数形态），
handler 副作用前校验。**仅此一项身份工作**——WS/MCP/包络等永久非目标。

## 2. 客户端契约（desktop / iOS）

- 生成 `operation_id`（每次用户操作一个，重试沿用同 id）。
- transport 沿已上线的 `mutationSingleAttempt`；结局分类沿 tagged 规则。
- **重试环**：ambiguous → 有界退避重发同 `operation_id`（首发 1–2s，
  之后对齐周期刷新节拍，上限 N=5 次）→ 确定结局（200 outcome / 409
  tagged）后按结局收尾：applied_* → 本地删行 + 触发 feed 整替（复用
  favorites 的 forceReplacement 路径）；rejected_* → 恢复行 + 报错。
  重试耗尽仍 ambiguous → 报错 + 触发 feed 整替（由服务端状态收敛显示）。
- 重试状态机放 Core/ingress（iOS `GaryxMobileCore`、desktop renderer
  编排层——复用 favorites 已落地的退避 effect 骨架），App/main 只做
  IO 宿主。scope/epoch 清场即弃（沿 favorites 语义）。
- CLI：生成 op_id 一次、失败重发同 id（简单循环即可，无状态机）。

## 3. 迁移与上线

- 建表两张 + 墓碑 kind 扩展：schema init 幂等（IF NOT EXISTS +
  ensure 列模式）。无历史回填（台账/outbox 从零开始；delete 墓碑自
  cutover 起）。
- archive/delete 路由签名变更（必带 operation_id + incarnation）——同仓
  同发，双端与 CLI 调用点同 PR 迁移，无兼容层。
- 一次性 cutover marker 不需要（无数据迁移，纯增量机制）。

## 4. 测试计划

**Gateway（确定性交错为主，对齐 favorites 项目测试风格）**

- 台账幂等：同 op_id 双发（顺序/交错/迟到首发后至）恒同结局零二次
  副作用；不同 op_id 对已删目标 → rejected_not_found(kind)；incarnation
  失配 → 409 零副作用不记账。
- 墓碑：delete 后旧 writer 回写被事务内拒绝（`ensure_thread_entry` 时序
  复现）；archive/delete 墓碑语义等同；cutover 前历史删除无墓碑的边界
  行为。
- outbox：事务提交 + worker 执行前崩溃（模拟）→ boot 重跑 pending 收敛；
  每步幂等（重复执行安全）；持续失败留 pending；同线程步骤有序。
- D4：读路由零写断言（history GET 前后 DB 字节级不变）；boot 孤儿清理
  一次过后干净；既有 1778 区域回归改写。
- detach 迁 outbox 后：archive 事务前无任何副作用（校验早于一切）。
- 台账 TTL 修剪 + 修剪后重放 no-op。

**双端**

- 重试环：ambiguous → 同 id 重发 → 确定结局三路径收尾；重试耗尽路径；
  scope 清场弃环；`attemptCount == 1` 每次 dispatch。
- UI：applied 后行消失（整替）；rejected_conflict 恢复行 + 报错文案；
  rejected_not_found(deleted/archived) 文案区分。

**端到端**

- 隔离 gateway：curl 同 id 双发、kill -9 于事务后清理前再起、验证 boot
  收敛；双端真实操作走查。

## 5. 实现切分（四步提交）

1. **gateway-core**：墓碑统一 + lifecycle_operations + cleanup_outbox +
   worker + boot 恢复（含 D4 孤儿一次过）+ 全部服务端测试。
2. **gateway-routes**：archive/delete 路由改造（op_id + incarnation +
   同事务序 + detach/清理迁 outbox）+ history GET 去写。
3. **双端编排**：重试环（Core/ingress）+ UI 收尾 + CLI 简单重发。
4. **契约文档**：repository-contracts 墓碑条款推广 + 恢复流程补 outbox
   说明。
