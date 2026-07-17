# Lifecycle Write Safety (生命周期写安全加固)

Status: draft v7 (addressing review round 6 — #TASK-2370 FAIL: 1×P1 + 1×P2)
Date: 2026-07-17

## 0f. Round 6 findings → v7 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | P1 generation 跨 entry 删除/重建的 ABA：最后 lease 释放会删 entry（run_admission.rs:294/:327），重建后 per-entry 计数可回到旧数值，旧 decision token 错误命中 CAS、旧 Live 覆盖新 Archiving | **全局单调 epoch**（§B.2b 修订）：代际值改由 coordinator 进程级单调 nonce 分配（AtomicU64，进程生命周期内永不复用、跨 entry 驱逐恒单调）——entry 删除重建天然拿到全新 epoch，ABA 结构性不可达；**CAS 比较全元组 `(thread_id, unique_epoch, expected_state, owner_token)`**，不比裸数值；辅助规则：存在未结算 calibration/reservation/decision token 时 entry 不删除（防御性，双保险）。seam 测试按 reviewer 处方：旧 token 暂停 → 末 lease 释放删 entry → 重建+校准+新 reservation → 恢复旧 completion → 断言 CAS 失败、reservation 不被覆盖 |
| 2 | P2 「任何 mutex 不得跨异步 SQLite 读」过宽，与 EndpointBindingMutator 持锁跨权威读写（其正确性依赖）及 SQLite per-key 写锁跨 run_blocking 冲突 | 锁序收窄（§B.2b 修订）：**registry/coordinator mutex 不跨 durable await**；`EndpointBindingMutator` async lock **可以**覆盖其权威读写临界区（preflight_and_freeze 依赖），但进入 coordinator 前必须释放；SQLite per-key 写锁跨 run_blocking 维持现状 |

## 0e. Round 5 finding → v6 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | P1 read-through 校准/结算写回缺代际 fence：墓碑读必经异步 run_blocking、不能纳入 coordinator 同步 mutex 临界区——旧校准快照可在挂起期间覆盖他人新建的 Archiving/Deleting/lease（run 越过屏障 → 墓碑后活跃 run 复现）；DecisionRejected 的旧 durable 快照同理可覆盖新瞬态 | **代际 fence 协议**（§B.2b 新增）：①显式 `Uncalibrated` 状态 + per-thread `generation`（每次状态安装自增）；②校准两段式——锁内取 (G, Uncalibrated) → 释放锁做异步 durable 读 → 重取锁**仅当 entry 仍为同 G 且仍 Uncalibrated 才安装**，否则丢弃快照、以更新状态为准（重试/直接使用）；③三类 completion（Committed/DecisionRejected/TransientFailure）一律按 reservation/decision token **条件更新**（token 携带取得时的 generation），绝不覆盖更新的瞬态/终态；④锁序写死：registry mutex 与 coordinator mutex 互不嵌套、任何 mutex 不得跨 async SQLite 读持有、EndpointBindingMutator 锁与 coordinator 锁不同时持有（先 mutator 后 coordinator，释放后进入）；⑤确定性 seam：校准读完成→安装前暂停→archive/delete 建 reservation（G 自增）→恢复旧安装→断言被丢弃、run 不获准 |

## 0d. Round 4 findings → v5 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | P1 coordinator 缓存无归位边界：重启后空表默认 Live，prior 快照未经校准（Deleted→archive 拒绝后被恢复成 Live）；commit 后、内存结算前 panic → 重放走 completed 短路、缓存永不归位；run admission 先拿内存 lease 后读记录 → 重启后 Deleted 线程短暂 lease 可把矩阵该判 not_found 的固化成 conflict | **「墓碑唯一真源 + 内存 read-through 缓存」三条边界**（§B.2a 重写）：①**校准读**——coordinator 条目缺失/默认态时，一切消费点（reservation 捕获 prior、run admission、屏障裁决）先按持久墓碑校准再使用；②**DecisionRejected 恢复的是校准后的 durable 态**（decision tx 内重读墓碑），不是未校准内存 prior；**decision tx 内终态裁决优先于瞬态 lease 观察**（Deleted 线程即使短暂 lease 在途，矩阵仍判 rejected_not_found）；③**Committed 结算崩溃一致**——reservation 在事务 commit 后立刻置 committed 标记，guard Drop 见标记则应用 durable_terminal 而非恢复 prior；即便结算丢失，read-through 校准使下次访问自愈。测试补：重启后 archive(deleted)；commit 后结算前 panic → 下次访问已校准；inactive lease × 终态裁决交错 |
| 2 | P1 cell/freeze RAII 未覆盖不可取消后代：owner 外层 panic 释放 guard 后，coordinator owned child / spawn_blocking 闭包继续 drain/提交，与新 owner/新 bind 重叠 | **mutation supervisor 唯一存活域**（§B.0a 新增）：cell/freeze/reservation 与全部后代句柄由同一 supervisor（detached owned task）持有；结构约束 = 可失败裁决全部发生在「spawn 不可取消工作之前」或「join 之后」，spawn→join 之间无其它 await；**guard 释放前置条件 = 后代 quiesce 或 durable 台账行已存在**；防御性兜底 = guard Drop 不立即释放，而是移交 reaper task（持有后代 JoinHandle）待 quiesce 后再移除 cell/释放 freeze 并发布 transient-failure。测试补：coordinator handoff 后 abort、spawn_blocking 已调度后 abort × 并发 bind/重放 |
| 3 | P2 preflight_and_freeze 应在锁内先查已有 freeze 再分类，否则 freeze 期间 config 变 enabled 会把第二个 delete 持久化成 conflict 而非承诺的瞬态 in_progress | §C.0a 顺序写死：mutation_lock 内 **①查已有 freeze（命中 → 瞬态 operation_in_progress，直接返回）→ ②binding/config 分类 → ③登记 freeze** |
| 4 | P2 架构图仍把内存终态同步画在事务内、§A 残留「同步写两处」，与 §B.2a 矛盾 | 措辞清理：图中内存结算移到 commit 之后（标注非原子、read-through 自愈）；§A 改为「commit 后更新缓存，墓碑为唯一真源」 |

## 0c. Round 3 findings → v4 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | P1 preflight→freeze 非原子：并发 bind 在 mutator 锁内提交后 delete 才挂 freeze，drain 杀 run 后事务才发现 binding——破坏性拒绝复现；且 enabled 判定依赖独立 live config，freeze 冻不住 enable/disable | **原子 `preflight_and_freeze`**（§C.0a 重写）：进 `EndpointBindingMutator` 同一串行域——持 mutation_lock 内重读权威 binding、按**冻结时捕获的 config snapshot** 裁决、成功则**释放锁前**登记 owned freeze；已有 freeze → 瞬态 `operation_in_progress`。**config 线性化写死**：enabled-ness 在 freeze 时一次性裁决（快照即判据），最终事务 belt 用同一快照语义、不得用新 config 重分类；disabled binding 允许删除的现状回归保留。seam 测试：bind 卡提交前 × delete——两序皆断言 **drain 在拒绝路径上从未发生** |
| 2 | P1 terminal-aware 与 decision tx 缺 coordinator 状态结算协议：现有二值模型（Ok→无条件新终态 / Err→恢复 Live）映射不了矩阵（archive(deleted) 拒绝后须仍 Deleted；delete(archived) 瞬态失败须恢复 Archived） | **typed coordinator completion**（§B.2a 新增）：`Committed{durable_terminal}` / `DecisionRejected{keep_prior}` / `TransientFailure{restore_prior}`；`MutationReservation` 保存**进入前状态**（含先前终态，不再假设 Live）；**持久墓碑为真源**，内存终态仅在 SQLite commit 之后应用、失败恢复 prior。测试在 16 个 HTTP 断言外补 coordinator 态断言（Deleted→archive rejected 仍 Deleted；Archived→delete commit 失败恢复 Archived）+ 之后的 run admission 行为 |
| 3 | P2 owner cell 与 freeze 缺 all-exit Drop 契约：owner task panic 后 dead cell 永久占位；瞬态 IO 失败后 freeze 不释放 | **RAII 契约写死**（§B.0/§C.0a）：owner cell guard = conditional-remove on drop（panic/abort → 移除 cell + 向 joiners 发布 transient-failure 结局）；freeze guard = 一切退出路径释放（含 B.3 瞬态失败）。对齐 run_admission.rs:67/:401 的既有 Drop 模式；panic 与 IO seam 测试入计划 |

## 0. 威胁模型与范围边界

（同 v2，未质疑）单用户单机、单 SQLite 单 writer、data-dir flock ⇒ 单进程
保证。已上线设施（incarnation/boot_id/tagged/单 attempt 传输）复用不重建。
四缺陷 D1–D4；**永久非目标**（用户裁决）：WS/MCP 身份、全实体包络、CLI
身份、manifest、admission 前置。不改业务冲突语义；不加兼容层。

## 0b. Round 2 findings → v3 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | **P0** completed 点查与 insert-or-get 之间窗口：A 在 B 点查后提交并注销，B 成为新 owner 却被 coordinator 的 Archived→Unavailable 挡住，拿不到 A 的结局 | **owner 二次点查**：insert 成为 owner 后、进 coordinator 前**再查一次 completed**，命中 → 向自己的 cell 发布旧结局并注销（双检锁定式）。附加规定：registry mutex 仅用于原子 insert/get，释放后才进 coordinator（禁嵌套持锁）；**owner 链跑在 detached owned task**（HTTP handler 取消不取消操作，对齐 coordinator 现有保证 run_admission.rs:174）；补「B 读空→A 提交注销→B 注册」确定性 seam 测试 + owner HTTP future 被取消回归（§B.0） |
| 2 | P1 屏障/前置拒绝无持久台账落点：archive 被 active lease 拒绝、响应丢失、run 结束后重放同 id 二次成功 | **确定性业务结局先持久再发布**：coordinator/preflight 拒绝走独立 **decision transaction**（belt 重查 → 插 rejected 台账行 → commit → publish）；仅内部/瞬态失败（IO 等）不记账、按 ambiguous 由客户端重试（§B.3） |
| 3 | P1 in-flight join 未验指纹；`endpoint_keys` 是现有 archive 行为参数非「未来参数」；completed replay 需要成功载荷才能复现响应 | OperationCell 保存**规范化完整指纹**（kind、thread_id、endpoint_keys 等），insert-or-get 失配立即 `operation_id_conflict`；台账保存同一指纹 + **成功结果载荷**（含首次 `detached_endpoint_keys`），completed replay 复现原响应（§B.1） |
| 4 | P1 incarnation 校验顺序自相矛盾；台账未按 incarnation 分域（rotate 同库后旧 id 重放会命中旧台账而非 wrong_incarnation） | **身份最先**：expected vs current 校验先于 completed 点查/注册/coordinator 一切步骤；owner 最终事务 belt 再验。台账持久键改 **`(store_incarnation, operation_id)`**（与内存键一致），行内存 incarnation（§B.1、§E） |
| 5 | P1 终态矩阵与 coordinator 内存态冲突：非 Live 一律 Unavailable ⇒ 同进程内 archive(archived) 得 409、重启后才得 noop——结果随重启改变 | **terminal-aware 通路**：coordinator 对**持久终态**（墓碑 archived/deleted）不再回 Unavailable，放行至矩阵裁决（内存终态与持久终态一致化：迁移成功时同步写两处）；瞬态忙碌（Archiving/Deleting 进行中）仍 Unavailable→ambiguous 重试。8 格测试 × {同进程立即, 重启后} 双态断言（§B.2） |
| 6 | P1 屏障未按 kind 分型：统一「先 drain 后事务」会让带 enabled binding 的 delete 先杀 run 再被拒——拒绝却有破坏性副作用 | **kind 分型屏障序**（§C.0a）：archive = 非破坏性 lease check/reservation，active run → decision tx 持久 rejected（不 abort）；delete = **binding preflight 先于破坏性 drain**，并经序列化 EndpointBindingMutator 挂 **binding freeze**（删除窗口内新 bind 被拒，冻结在拒绝/完成时释放）→ drain → 最终事务 belt 重查——preflight 后新增 binding 的 TOCTOU 由冻结封死；身份错误最先返回 |
| 7 | P2 「现状语义保留」不实：route 现在无条件 drop 本地状态（测试 8455 断言），v2 的 RetryableFailure 保留 affinity 是行为变化需显式裁决 | **裁决：采纳 retain-until-cleared**（旧「provider 失败也无条件丢本地」正是 D3 类问题：丢了 affinity 重试就找不到 provider）。顺序 = provider clear 至 `Cleared\|AlreadyAbsent` → local drop → 标 done；RetryableFailure 保留 affinity 续重试。**显式行为变化**：8455 区域测试随迁改写（断言改为「provider 失败 → affinity 保留 + 作业 pending」）（§C.2） |
| 8 | P2 60s join 窗口超过 desktop 单请求 8s abort，tagged operation_in_progress 分支不可达 | join 等待窗口 = **6s**（压在 desktop 8s 传输预算内），超时返回 tagged `operation_in_progress`；生命周期请求的传输 timeout 与整体 retry budget 显式写入客户端契约；客户端测试按真实时限编写（§B.0、§2） |

### Round 2 已确认关闭

持久 endpoint detach 入事务 + volatile 条件缓存失效（ABA 关闭）；outbox
调度/退避/修剪闭合；墓碑覆盖生产写点；D4 boot recovery 方向；崩溃语义
「有台账返旧、无台账新执行」本身成立。

## 1. 架构总览（v3）

```
请求(op_id, expected_incarnation, 行为参数)
  ↓
⓪ 身份最先：expected ≠ current → 409 wrong_incarnation（不点查/不注册）
① completed 台账点查 (incarnation, op_id)：命中→验指纹→返旧结局/409 conflict
② 注册表 insert-or-get（mutex 仅原子操作）：
     已有 cell → 验指纹 → join（等 6s；超时 tagged operation_in_progress）
     新 owner → 移交 detached owned task：
③ 【owner】二次 completed 点查（双检）→ 命中即发布旧结局注销
④ 【owner】kind 分型屏障：
     archive：lease check（active run → decision tx 持久 rejected）
     delete：binding preflight + binding freeze → abort-and-drain → lease wait
⑤ 【owner】单 writer 生命周期事务：
     incarnation belt → 台账 belt → 业务前置终裁 →
     状态迁移 + 墓碑 → 持久 detach（点写）→
     台账行(指纹+结局+载荷) → volatile 作业入 outbox → commit
   【屏障拒绝路径】decision tx：belt → rejected 台账行 → commit
⑥ commit 后应用内存终态（非原子缓存更新，read-through 自愈）
⑦ 发布结局给 joiners → 注销 → （异步）outbox worker
```

### A. 删除墓碑统一化（同 v2，CONFIRMED）

delete 与 archive 共用墓碑（kind 记 archived/deleted）；既有事务内校验
自然覆盖；repository-contracts 条款推广；cutover 前历史裸删接受。
**v5 措辞更正**：墓碑为唯一真源；coordinator 内存终态在 commit 后更新（read-through 缓存，§B.2a）。

### B. 幂等操作台账

#### B.0 in-flight join/owner（v3 修订）

- 注册表 `Map<(incarnation, op_id), OperationCell>`；**mutex 仅覆盖原子
  insert/get**，释放后才做任何 IO/coordinator 调用。
- **cell 带规范化指纹**（round2-3）：insert-or-get 时指纹失配 → 立即
  `operation_id_conflict`（不 join 不注册）。
- **owner 链在 detached owned task 执行**（round2-1）：HTTP handler 只
  await cell 结局（有界 6s，round2-8）；handler 取消不影响操作推进
  （对齐 run_admission.rs:174 的 caller-cancellation 保证）。
- **cell RAII 契约（round3-3）**：owner 持 conditional-remove guard；
  正常完成路径先发布真实结局再解除 guard。dead cell 永久占位不可达。

#### B.0a mutation supervisor（round4-2 新增：guard 与不可取消后代同域）

- **唯一存活域**：owner 的 detached task 即 **mutation supervisor**——
  cell guard、freeze guard、coordinator reservation 与**全部后代句柄**
  （coordinator owned child 的 JoinHandle、`run_blocking` 的 handle）都
  由它持有。
- **结构约束**：可失败的裁决逻辑只允许出现在「spawn 不可取消工作之前」
  或「join 之后」；spawn→join 之间除 join 本身无其它 await——panic 无法
  发生在「后代在跑而 supervisor 已死」的窗口（正常路径）。
- **guard 释放前置条件**：后代全部 quiesce（join 返回），或 durable
  台账行已提交。二者皆无 → 不得释放 cell/freeze、不得允许替代 owner。
- **防御性兜底**：guard Drop（异常路径）不立即释放——把 cell/freeze 与
  后代句柄移交 **reaper task**，待后代 quiesce 后再移除 cell（发布
  transient-failure）并释放 freeze。「旧 child 继续 drain 而新 owner/新
  bind 已获准」的重叠不可达。
- **owner 双检**（round2-1 P0）：成为 owner 后第一步再查 completed 台账，
  命中 → 发布旧结局 → 注销（覆盖「点查空 → 他人提交注销 → 我注册」窗口）。
- join 等待 6s 超时 → tagged `operation_in_progress`（客户端按 ambiguous
  同 id 续发；后续到达 join 或命中台账）。
- 崩溃语义不变：注册表随进程消失；台账有行返旧、无行新执行。

#### B.1 台账（v3 修订）

```sql
CREATE TABLE IF NOT EXISTS lifecycle_operations (
  store_incarnation TEXT NOT NULL,
  operation_id      TEXT NOT NULL,
  kind              TEXT NOT NULL CHECK (kind IN ('archive','delete')),
  thread_id         TEXT NOT NULL,
  fingerprint       TEXT NOT NULL,   -- 规范化 JSON：kind/thread_id/endpoint_keys/…
  outcome           TEXT NOT NULL,
  result_payload    TEXT,            -- 成功载荷：如首次 detached_endpoint_keys
  detail            TEXT,
  completed_at      TEXT NOT NULL,
  PRIMARY KEY (store_incarnation, operation_id)
) STRICT;
```

- **持久键 = (store_incarnation, operation_id)**（round2-4，与内存键
  一致）；rotate 后旧 incarnation 的行不可命中（且身份最先校验已把这类
  请求挡在 409 wrong_incarnation）。
- 命中校验**完整指纹**；`result_payload` 使 completed replay 复现原响应
  （含 `detached_endpoint_keys`，round2-3）。

#### B.2 终态结果矩阵 + terminal-aware 通路（round2-5）

矩阵同 v2（8 格）。**通路修订**：coordinator 对持久终态（墓碑）不回
Unavailable——放行 owner 至矩阵裁决（decision/lifecycle tx 内以墓碑为准）；
仅瞬态忙碌（他人 Archiving/Deleting 进行中，不同 op_id）仍 Unavailable →
客户端按 ambiguous 重试。**8 格 × {同进程立即请求, 重启后请求} 共 16
断言**，结果恒同。TTL 修剪后同 id = 纯新请求恒走矩阵（同 v2）。

#### B.2a coordinator 状态结算协议（round3-2 新增，round4-1 重写）

**真源公理：持久墓碑是唯一真源；coordinator 内存态是 read-through 缓存。**

- **typed completion**：
  - `Committed { durable_terminal }`：SQLite commit **之后**应用内存终态；
  - `DecisionRejected`：恢复**校准后的 durable 态**（decision tx 内重读
    墓碑，不用未校准内存 prior——重启后 Deleted 线程 archive 被拒仍
    Deleted）；
  - `TransientFailure`：恢复校准 durable 态（delete(archived) 瞬态失败
    后仍 Archived）。
- **校准读（read-through）**：coordinator 条目缺失或处于未校准默认态时，
  一切消费点（reservation 捕获 prior、run admission、屏障裁决）先按持久
  墓碑校准再使用；`MutationReservation` 保存的 prior = 校准后快照。
- **终态裁决优先级**：decision tx 内以墓碑为准，**优先于瞬态 lease 观察**
  ——重启后 Deleted 线程即便存在短暂 inactive lease，archive 仍判
  `rejected_not_found`（矩阵），不因 lease 固化成 `rejected_conflict`。
- **Committed 结算崩溃一致**：reservation 在事务 commit 后立即置
  committed 标记；guard Drop 见标记 → 应用 durable_terminal（不恢复
  prior）。结算即便丢失（panic 时序极端），read-through 校准使下次访问
  自愈——缓存永不归位的路径不存在。

#### B.2b 校准与结算的代际 fence（round5-1 新增）

- **状态与代际（round6-1 修订）**：coordinator entry 增显式 `Uncalibrated`
  态；代际值 = **进程级全局单调 nonce**（AtomicU64 分配，每次状态安装取
  新值；进程生命周期内永不复用、跨 entry 删除/重建恒单调——重建 entry
  天然获得全新 epoch，ABA 结构性不可达）。存在未结算
  calibration/reservation/decision token 时 entry 不删除（双保险）。
- **两段式校准**（墓碑读必经异步 `run_blocking`，不得在 mutex 内进行）：
  1. 锁内读取 `(generation = G, Uncalibrated)`，释放锁；
  2. 异步 durable 读（墓碑/记录）；
  3. 重取锁：**entry 仍为 G 且仍 Uncalibrated 才安装校准结果**；否则丢弃
     本次快照，以更新状态为准（消费方重试或直接使用新状态）。
  旧快照永远无法覆盖挂起期间他人建立的 `Archiving/Deleting`/lease/终态。
- **条件结算（round6-1 修订）**：`Committed` / `DecisionRejected` /
  `TransientFailure` 携带 reservation/decision token；写回 CAS 比较
  **全元组 `(thread_id, unique_epoch, expected_state, owner_token)`**
  ——任何分量失配即放弃写回（durable 真源不受影响，read-through 校准
  自然收敛）。
- **锁序（round6-2 收窄）**：registry/coordinator mutex **不跨 durable
  await**；`EndpointBindingMutator` async lock **可以**覆盖其权威读写
  临界区（preflight_and_freeze 的原子性依赖之），但进入 coordinator 屏障
  前必须释放；SQLite per-key 写锁跨 run_blocking 维持现状。registry 与
  coordinator mutex 互不嵌套。
- **seam 测试（round7 PASS 附带 P2 修正：拆成两条不矛盾的时序）**：
  ①校准读完成、安装前暂停 → archive/delete 建 reservation（epoch 前进）
  → 恢复旧安装 → 断言旧快照被丢弃、run 不获准、reservation 屏障有效；
  ②正常路径：存在未结算 token 时末 lease 释放**不删除** entry（协议
  断言）；③**测试专用强制驱逐 seam**：强制驱逐并重建 entry + 新
  reservation → 恢复旧 completion → 断言全元组 CAS 失败、reservation
  不被覆盖（验证全局 epoch 在驱逐语义被破坏时仍兜底）。

#### B.3 decision transaction（round2-2）

- **一切确定性业务结局先持久再发布**：屏障/前置拒绝（archive 撞 active
  run、delete 撞 enabled binding、矩阵 rejected_*）走独立 decision tx：
  台账 belt 重查 → 插入 rejected 行（含指纹）→ commit → publish。
- 内部/瞬态失败（IO、provider 通信等）**不记账**，向 joiners 发布
  ambiguous-equivalent（客户端重试同 id，可能换 owner 重新执行）。

### C. 屏障 / 事务 / outbox 三层（v3 修订）

#### C.0a kind 分型屏障序（round2-6）

- **archive**（非破坏性）：⓪身份 → ①lease check/reservation——active
  run → **decision tx 持久 rejected_conflict**（不 abort、不杀 run，
  现状语义）；无 run → reservation 护住窗口 → 事务。
- **delete**（破坏性，顺序写死；round3-1 修订为原子操作）：⓪身份 →
  ①**`preflight_and_freeze`（EndpointBindingMutator 串行域内原子完成，
  round4-3 顺序写死）**：持 mutation_lock 内——**第一步查已有 freeze**
  （命中 → 瞬态 `operation_in_progress` 直接返回，不做任何分类）→
  重读权威 binding → 按**冻结时捕获的 config snapshot** 裁决 enabled-ness
  （发现 enabled binding → 返回拒绝，decision tx 持久 rejected_conflict，
  **未 drain 零破坏副作用**）→ 通过则**释放锁前**登记 owned freeze
  （冻结期内该线程新 bind 被拒 409）→
  ②token 失效 + abort-and-drain + lease wait → ③生命周期事务（belt 重查
  binding：freeze 保证无新增；**enabled 判定沿用冻结时快照，不用新
  config 重分类**——config 线性化点 = freeze）。
- **freeze RAII（round3-3）**：guard 一切退出路径释放（拒绝、完成、
  B.3 瞬态失败、panic）；内存标记随进程消失（单进程模型）。
- disabled binding 允许删除的现状语义与回归（routes/tests.rs:8061 区域）
  保留。
- preflight→事务的 TOCTOU 由「同锁域原子 preflight_and_freeze」封死：
  并发 bind 要么在锁内先提交（preflight 必见 → 拒绝于 drain 前）、要么
  排在 freeze 后（被拒）。

#### C.1 outbox 表（同 v2，CONFIRMED）

payload/attempt_count/next_attempt_at/调度规则/修剪不变。

#### C.2 worker 契约（round2-7 裁决落地）

- typed outcome `Cleared | AlreadyAbsent | RetryableFailure`。
- **runtime_teardown 顺序（行为变化，显式裁决）**：provider clear 至
  `Cleared|AlreadyAbsent` → **然后** local affinity/workspace drop →
  标 done。`RetryableFailure` → **保留 affinity**（否则重试无法路由到
  provider——旧「无条件丢本地」正是本设计要根治的半截清理）+ 退避重试。
- 既有回归（routes/tests.rs:8455 区域「provider 失败仍丢本地」）**改写**
  为「provider 失败 → affinity 保留 + 作业 pending + 告警」；新增
  「provider 成功后、local drop 前崩溃 → 重放 AlreadyAbsent → 补 local
  drop → done」。

### D. 读路径去写 + 孤儿归位（同 v2，方向 CONFIRMED）

### E. 身份先验（v3 修订，round2-4）

`expected_store_incarnation` 校验**先于一切**（completed 点查、注册、
coordinator）；owner 最终事务 belt 再验；失配 409 tagged 零副作用不记账。

## 2. 客户端契约（v3 修订）

- op_id 每操作一个、重试沿用；transport 单 attempt。
- **时限（round2-8）**：服务端 join 窗口 6s < desktop 单请求 8s；
  生命周期请求整体 retry budget 显式配置（默认 5 次退避重发，首发后
  1s/2s/4s/8s/8s）；`operation_in_progress` 与 transport ambiguous 同路
  处理（续发同 id）。
- 确定结局收尾三路径 + `operation_id_conflict` 终端报错（客户端 bug 级）。
- 状态机在 Core/ingress，复用 favorites 退避骨架；CLI 简单重发环。

## 3. 迁移与上线（同 v2 + provider typed 迁移 + coordinator 终态通路改造）

## 4. 测试计划（v3 增量）

**Gateway**

- **round2-1 P0 seam**：B 点查空 → A 提交+注销 → B 注册成 owner →
  owner 双检命中 → B 返回 A 的结局（确定性注入点）；owner HTTP future
  取消后操作仍完成、joiner/后续重发拿到结局。
- **round2-2**：archive 撞 active run → rejected 持久化（decision tx）
  → 响应丢失 → run 结束 → 重放同 id → 返回首次 rejected（不二次执行）；
  瞬态 IO 失败不记账 → 重放重新执行。
- **round2-3**：in-flight 指纹失配（并发 join 前）→ operation_id_conflict；
  completed replay 载荷含首次 detached_endpoint_keys（响应逐字段断言）。
- **round2-4**：rotate 后旧 expected 重放 → wrong_incarnation（先于台账
  命中）；台账按 (incarnation, op_id) 隔离。
- **round2-5**：矩阵 8 格 × {同进程, 重启后} 16 断言恒同；瞬态忙碌
  （并发不同 op_id）仍 Unavailable。
- **round2-6 + round3-1**：带 enabled binding 的运行中线程 delete →
  preflight 拒 → **run 未被杀**；freeze 期内新 bind 被拒、释放后恢复；
  **原子性 seam**：bind 卡在 mutator 提交前 × delete 进入
  preflight_and_freeze 的两种排序——两序均断言拒绝路径 drain 从未发生；
  config snapshot 线性化（freeze 后改 enable/disable 不改变本次裁决）。
- **round3-2**：Deleted→archive(不同 op_id) → rejected_not_found 且
  coordinator 仍 Deleted（后续 run admission 拒绝一致）；Archived→delete
  瞬态 commit 失败 → 恢复 Archived（非 Live）；Committed 仅在 commit 后
  应用内存终态。
- **round3-3**：owner task panic → cell 被 guard 移除 + joiners 收
  transient-failure → 重发可成为新 owner；freeze 在瞬态 IO 失败路径
  释放（seam 注入）。
- **round2-7**：provider RetryableFailure → affinity 保留 + pending；
  provider 成功后崩溃 → 重放 AlreadyAbsent → local drop → done；
  8455 区域旧断言改写。
- v2 既有计划全保留（join/崩溃/矩阵/墓碑/outbox 调度/D4）。

**双端**：重试环按真实时限（6s join / 8s transport / retry budget）；
`operation_in_progress` 续发路径；三路径收尾；conflict 终端报错。

**端到端**：curl 同 id 双发（drain 窗口内/decision 拒绝后/重启后）、
kill -9 于各阶段、双端走查。

## 5. 实现切分（四步，同 v2 + coordinator 终态通路 + freeze 机制入步骤 2）
