# Thread Favorites (线程收藏)

Status: draft v18 (addressing review round 17 — 1×P0 + 1×P1 + 1×P2)
Date: 2026-07-16

## 0. 修订记录

### Round 17 findings → v18 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | **P0** provenance 拒绝未进 reducer：raw 未就绪时 toggle 仍建 intent，bootstrap 后 rawAccepted 自动 drain，派发前拒绝无转移可走（丢弃卡死 inFlight / 映射 notSent 会重试），feed 刷新拿到 B/X provenance 后旧 retry 即可无确认地向 B 派发 | **采纳入场门方案 + 双保险**：①**admission gate 先于 toggle**——意图入场前置 = 当前 epoch known(uuid) ∧ raw 就绪（favorites 写）∧ 目标行 provenance == uuid；不满足 → 惰性 bootstrap（action token 护航）→ 复评；仍不满足 → **不创建 intent**，触发 feed 刷新 + 用户重确认。v16 的「raw 未就绪意图排队」机制**删除**（排队被 bootstrap-then-admit 取代）。②`latestDesired` 增**不可变 `authorizedIncarnation` 戳**；`dispatch` 与 `backoffFired` 守卫：戳 ≠ 当前 known → 新终端事件 **`actionCancelled`**（退休意图、清 effect，不重试不派发）——刷新永远不能改写旧 action 的授权。测试补「provenance 拒绝 → B 行刷新成功 → 推进全部 timer/effect → 第二次显式点击前 B 零 mutation」（§7.1、§7.2） |
| 2 | P1 provenance 只覆盖 recent/favorites/snapshot 行，实际入口（desktop DesktopState.threads 全量表、双端 `/api/threads/:id` 点查修补的 pinned 窗口外行、深链、workspace/bot 钻取、新建线程）拿到的是裸 summary，永远 unknown → 要么永拒、要么全局补标重引 A/X→B/X | **精确目标身份读取契约**：一切产出线程实体的 gateway 响应统一携带 `store_incarnation_id`——新增覆盖 `/api/threads`（desktop 全量 state slice）与 `/api/threads/:id`（双端点查）；客户端在 ingest 处把 summary 与 provenance **原子合并**（desktop state merge / iOS 点查回填处逐处标注）。枚举入口：pinned 窗口外、workspace/bot 钻取、深链、新建线程（创建响应即带身份）。裸 summary 无身份字段的旧解码路径在实现中同步更新（§4.2、§6） |
| 3 | P2 Gateway 测试条目残留「父退出（或 cap）后执行 destructive init」的 fail-open 表述 | 已更正为 fail-closed 唯一表述（§8） |

### Round 16 findings → v17 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | **P0** 惰性 bootstrap 把旧域用户操作重绑到新域：unknown 时点击 archive（目标 = 旧店缓存行 A/X），bootstrap 从新店 B 返回身份后原操作携带 B 的 expected incarnation 发往 B，误删 B/X；等待期间切网关同理（现有生命周期 Task 不随 gateway reset 取消） | 三重围栏：①**action token**——mutation 在点击时捕获 `(gatewayScope, runtimeEpoch, actionToken)`，bootstrap await 返回后、网络派发前**重校验全三元**，scope/epoch 任何变化 → 操作**永久取消**（新域身份不得唤醒旧操作）；②**目标实体身份出处（provenance）**——feed/snapshot 行携带其来源响应的 `storeIncarnation`；mutation 只允许目标 provenance == 当前 known(uuid) 的行；provenance unknown（如持久化 fallback 缓存行）或失配 → 不派发，触发 feed 刷新并要求用户在新行上重新确认；`unknown → known` bootstrap 只建立**读基线**，不授权任何来源身份未知的目标；③平台落点——desktop IPC 携带点击时 gateway scope、main 网络派发前与当前 settings 匹配；iOS 在 bootstrap await 后、调用 client 前重校验 generation。测试：`cached A/X → bootstrap B/X` 与 `click A → switch B → B identity arrives` 双端，断言 B 收到零 mutation（§7.1、§6） |
| 2 | P1 PPID 屏障 60s cap 到期仍 fail-open：父被 SIGSTOP/卡死时子进程照跑 destructive init，清掉父的真实 active rows | **fail closed**：cap 到期父进程仍存活 → 子进程在任何 destructive initialization 之前**释放锁、报错退出**，不得继续启动。测试分别断言「父退出 → 继续」与「cap 时父仍活 → 子退出且 DB 零修改」（§3.0） |
| （建议，采纳） | 裸 recent 响应只够建立 incarnation，不满足 favorites 写的 `expected_revision` 前置 | readiness 分级：**favorites 写** readiness = known incarnation **且已持有 raw revision**（须经 favorites GET / snapshot）；**archive/delete** readiness = known incarnation（任一含身份响应即可）（§7.1） |

### Round 15 findings → v16 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | P1 客户端缺 `store_incarnation_id` 初始化态与 mutation readiness 契约：身份未知时 nil≠uuid 会反复误触 scope-clear；archive/delete 入口不检查身份 readiness（desktop AppShell:3624/4726、iOS canArchiveThreadId 与深链 getThread 路径） | **身份状态显式化**：`incarnation = unknown \| known(uuid)`。①**bootstrap acceptance**：当前 epoch 的**首个**合法身份响应从 unknown 直接接受为 known（不是 mismatch，不触发 scope-clear）；仅 known(a)→known(b)（a≠b，当前 epoch）才 scope-clear。②**mutation readiness**：favorites 写与 archive/delete 派发前置要求「当前 epoch 已 known」——未知则先触发一次身份 bootstrap（拉 snapshot/recent，有界等待），bootstrap 失败按今日网络失败 UX 报错，不派发；入口不禁用（点击时惰性 bootstrap，避免 UI 状态泛滥）。③测试：cold start、feed/snapshot 失败后深链打开、身份响应晚于生命周期点击到达（双端）（§7.1、§6） |
| 2 | P1 flock 只封「双方均已升级」的 restart：首次从 pre-R5 升级时，旧进程无锁仍存活，新二进制拿到空闲锁立即 destructive init，可清掉旧进程的真实 active run | **cutover 屏障两件套**：①**父进程 handoff 屏障**——新进程启动取锁后、destructive init 之前，若 PPID 指向仍存活的同名 garyx 进程（unmanaged fallback 的 spawn 关系），**有界等待父进程退出**（cap 60s）再继续——封住 pre-R5 fallback 的重叠窗口；②**一次性升级策略写入发布文档**：托管路径（launchd/systemd）天然序列化（旧进程完全终止后才起新进程）无窗口；非托管手工启动升级须先停 pre-R5 进程（无法从新二进制侧探测非亲缘旧进程，边界如实声明）。测试：确定性「无锁旧父进程存活 → 新二进制启动 → destructive init 延迟到父退出/cap」用例（§3.0） |

### Round 15 已核验通过

RuntimeAssembler 单实例重构可实现；双 R5 锁下 bounded wait + CLOEXEC 封闭
unmanaged fallback；archive/delete 身份先验可置于 handler 最前（早于
endpoint detach，锁保证运行期 incarnation 不并发旋转）；三步响应裁决正确；
recovery 旋转可原子并入 `commit_legacy_import`（marker 保证 crash/retry
单次生效）。

### Round 14 findings → v15 处置

| # | Finding | 处置 |
|---|---|---|
| 1 | **P0** 生产启动开两次 DB（RuntimeAssembler 先开配置库，`with_persistent_local_stores()` 又开默认库再被覆盖）——非阻塞独占 flock 下第二次 open 必失败（实测 EWOULDBLOCK），gateway 自锁无法启动；自定义 data dir 下 builder 还会无意义地初始化甚至 purge 默认库（现存 latent bug） | **启动装配重构**：RuntimeAssembler 只创建**一个** `GaryxDbService` 实例，显式传给 builder；`with_persistent_local_stores` 拆掉隐式 DB open（skills/custom-agents 加载改用传入实例）。顺带修复「配置自定义 dir 仍破坏性初始化默认库」的现存 bug。测试覆盖真实 RuntimeAssembler 启动（default + custom data dir 两路径），不止双进程 open（§3.0） |
| 2 | P1 非托管 `/api/restart` subprocess fallback：旧进程持锁时 spawn 新进程 → 新进程抢锁失败退出 → 旧进程 spawn 成功后 exit(0) → 无人监听 | **锁获取改有界等待**：启动取锁 = flock 排他 + **有界阻塞等待**（默认 30s，env 可调），超时才中止——fallback 时序（spawn → 新进程等锁 → 旧进程 exit 释放 → 新进程继续）自然成立；误起的第二个 gateway 等满超时后清晰报错。**锁 FD 置 close-on-exec**（spawn 的子进程不继承锁）。测试覆盖 unmanaged subprocess fallback 端到端（§3.0） |
| 3 | **P0** incarnation 只围栏 favorites 端点：archive/delete 会删 favorite 行 + bump revision 但不带身份——旧 store 的延迟 archive 请求可落到新 store 永久归档错域同 ID 线程 | **incarnation 围栏扩展到一切触及 favorites 域的生命周期端点**：archive/delete HTTP 路由必带 `expected_store_incarnation`，**在任何副作用（含 endpoint detach，routes.rs:3144 早于归档事务）之前先验**，失配 → 409 wrong_incarnation 零副作用；desktop（threads.ts:1272）/iOS（client:446）调用改带身份（来源 = 任一 feed/snapshot 响应的身份字段，客户端启动即有）。内部同进程调用路径（bot 命令等）同店无跨域风险，不加参（注明）（§4.4） |
| 4 | P1 「迟到旧 incarnation 页判弃」缺可执行判定顺序（UUID 无新旧序；先按 mismatch 处理会被旧页再次清场并切回旧域） | **判定顺序写死三步**：①先验响应所属 `(gatewayScope, runtimeEpoch, ticket/requestToken)`；②旧 epoch → 直接丢弃（不看 incarnation）；③**仅当前 epoch 的响应**出现 incarnation mismatch 才是真实换域 → scope-clear。两种到达顺序（旧页先到/后到）都进测试（§7.1） |
| 5 | P1 recovery 轮换 incarnation 是政策描述，未绑定原子恢复入口；整目录备份恢复原样带回旧 UUID + 低 revision → 客户端永久拒收 / 旧 CAS 复配 | **绑定实现点**：①`commit_legacy_import(recovery=true)` 在其**既有原子提交事务内**（mod.rs:1070）旋转 incarnation，随提交 marker 幂等（crash/retry 不重复旋转）；②新增持锁管理命令 **`garyx gateway rotate-store-incarnation`**，整目录备份恢复的官方步骤（repository-contracts 恢复流程更新）在首次 serving 前执行它；③测试矩阵：recovery crash/retry 单次旋转、正常 reopen 不旋转、restore/clone 场景必旋转（§3.3） |

### 已确认（round 14）

favorites CAS 三元组、wrong_incarnation 零副作用、scope-clear 方向正确；
head 复核终止且封住单次链中上移（一轮补拉预算 + 下周期边界成立）；
R5 根因选择正确（问题只在启动/restart 调用面纳入，本轮已补）。

### 历史轮次（要点）

- **R13→v14**：per-data-dir flock（R5）；store_incarnation_id 进 CAS 身份域；
  链尾 head 复核。
- **R12→v13**：普通删除无条件 bump；meta 初始化契约；行级 seq wire 契约；
  回填独立 marker + UNIQUE；K 窗口语义；forceReplacement；gateway_auth
  接受集。
- **R11→v12**：activity_seq；R4 拆出独立任务；unresolvedFence；退避门。
- **R10→v11**：Favorites 取消分页；404 带页。**R9→v10**：七分支矩阵；
  presented 谓词。**R8→v9**：根因三件套，删 marker/gate。**R7→v8**：
  awaitVerify。**R5→v6**：CAS 写围栏。**R4→v5**：三元组围栏。**R3→v4**：
  双身份。**R2→v3**：meta/同快照/清理点。**R1→v2**：守卫插入、判别联合、
  双端入口。

## 1. 需求

用户需求（产品裁决，不可改动）：

1. 线程支持「收藏」。
2. 筛选类别：**全部（All）/ Chats / 收藏（Favorites）**。
3. iOS：行长按收藏项；线程内右上角菜单（与置顶相邻）；过滤器加收藏类别。
4. Mac：触发点与置顶一致；筛选加收藏 tab。

**系统级连带改造（用户裁决授权，改根因并说清楚）**：
R1 传输契约（+auth middleware tagged）、R2 组合快照端点、R3 keyset 分页 +
activity_seq、R5 data-dir 独占锁 + store incarnation 身份（含启动装配重构与
生命周期端点身份围栏）。（R4 生命周期幂等化 = 独立后续任务。）

## 2. 目标 / 非目标

**目标**：收藏 + 双端入口 + 三分类；R1/R2/R3/R5 根治；客户端 = 意图状态机 +
presented 过滤 + range-fill。

**非目标**：收藏排序/重排；收藏分页（cap 500 + truncated）；首页独立收藏段；
All/Chats 行星标；SSE；bot 命令面收藏筛选；意图跨进程/跨 incarnation 持久化；
`serverIdempotencyKey`；生命周期幂等化（独立任务；本设计仅加 incarnation
先验 + 今日 parity + ambiguous 后 forceReplacement）。

## 3. 数据模型与进程模型（gateway）

### 3.0 per-data-dir 独占锁与启动装配（R5，v15 修订）

- **单实例装配（round14-1）**：RuntimeAssembler 创建唯一 `GaryxDbService`
  并显式传入 builder；`with_persistent_local_stores` 不再隐式 open DB
  （skills/custom-agents 存储用传入实例）。修复现存 bug：自定义 data dir
  下默认库被无意义破坏性初始化。
- **锁语义（round14-2）**：`GaryxDbService::open` 最先对
  `<data_dir>/garyx.lock` 取 flock 排他，**有界阻塞等待**（默认 30s，
  `GARYX_DATA_LOCK_WAIT_SECS` 可调），超时中止并报「data dir 被另一
  gateway 占用」；锁先于 schema init / legacy import / purge /
  `clear_stale_active_runs`；进程全生命周期持有，**FD 置 CLOEXEC**
  （restart spawn 的子进程不继承）；退出自动释放。
- **restart 兼容**：托管（launchd/systemd）路径不变；非托管 subprocess
  fallback = spawn 新进程（等锁）→ 旧进程 exit 释放 → 新进程取锁继续。
- **pre-R5 升级 cutover 屏障（round15-2 + round16-2 fail-closed）**：新
  进程取锁后、destructive init 之前，若 PPID 指向仍存活的同名 garyx 进程
  （fallback spawn 亲缘），有界等待其退出（cap 60s）；**cap 到期父仍存活
  → 在任何 destructive initialization 之前释放锁、报错退出（fail
  closed），绝不继续启动**。托管路径天然序列化无窗口；非托管手工升级须先
  停 pre-R5 进程（发布文档写明；非亲缘旧进程无法从新二进制侧探测，边界
  如实声明）。
- 单进程保证下：启动 purge bump-on-change 豁免与 `clear_stale_active_runs`
  前移（bind 前）成立。

### 3.1 favorites（CONFIRMED）

`thread_favorites` + meta；条件写恒 bump（含 no-op）；**普通删除同事务
无条件 bump**；归档 bump-on-change（tombstone 豁免）；purge bump-on-change
（独占锁 + bind 前）；守卫式单事务插入；清理点三处。

### 3.2 activity_seq（CONFIRMED）

meta 初始化于 schema 段（先于 import）；写事务内分配；公开 upsert 显式
事务；回填独立 marker 一次性、meta 永不下降、UNIQUE 索引、CHECK < 2^53−1；
`clear_stale_active_runs` bind 前 + 不分配 seq；source-guard。

### 3.3 store 身份（v15 落点绑定）

- `store_incarnation_id`：建库生成，正常运行与 reopen 永不改变。
- **旋转实现点（round14-5）**：
  1. `commit_legacy_import(recovery=true)` 的原子提交事务内旋转（随提交
     marker 幂等，crash/retry 不重复）；
  2. 管理命令 `garyx gateway rotate-store-incarnation`（取 data-dir 锁后
     执行）；整目录备份恢复的官方恢复步骤更新进
     `docs/agents/repository-contracts.md`（实现 PR 同步改文档 +
     AGENTS/CLAUDE 镜像不涉及）。
- `server_boot_id`：进程级 uuid（不落盘）。

## 4. Gateway API

### 4.0 tagged 错误 schema 与身份域（CONFIRMED + v14）

tagged schema / 生产者清单 / `{endpoint op, gateway_auth}` 接受集；全部
favorites 响应携带 `{store_incarnation_id, server_boot_id, revision}`；
mutation 必带 `expected_store_incarnation`（先验 incarnation 后验 revision）。

### 4.1 收藏读写（CAS）——CONFIRMED

`PUT/DELETE …?expected_revision=N&expected_store_incarnation=<uuid>`；
200 恒 bump 含 no-op；409（revision/incarnation 分 code）/404 tagged +
同快照页 + 身份；400 tagged。

### 4.2 线程实体响应的身份契约（v18 扩展）

- `/api/recent-threads`（All/Chats）：cursor-only；行级 `activity_seq`；
  响应带身份二元组；bot reader 内部 offset。——CONFIRMED
- **（round17-2）一切产出线程实体的响应统一携带
  `store_incarnation_id`**：新增覆盖 `/api/threads`（desktop 全量 state
  slice）与 `/api/threads/{id}`（双端点查，含 pinned 窗口外修补、深链、
  workspace/bot 钻取路径）；线程创建响应同带身份。客户端 ingest 处把
  summary 与 provenance 原子合并（desktop `mergeRemoteDesktopState` /
  store.ts:1085 点查修补、iOS `+ThreadList.swift:50` /
  `+StateSync.swift:79` 点查回填、`getThread` 解码，逐处标注于实现清单）。

### 4.3 组合快照端点——CONFIRMED

单读事务 `{ store_incarnation_id, server_boot_id, revision, thread_ids,
favorites, recent: {threads, total, truncated} }`；cap 500。

### 4.4 生命周期操作（v15：incarnation 先验）

- 服务端幂等化仍属独立任务；本设计仅加**身份先验（round14-3）**：
  archive / delete HTTP 路由必带 `expected_store_incarnation`，**在一切
  副作用之前校验**（archive 现有的 endpoint detach（routes.rs:3144）位于
  归档事务前，校验必须再早于 detach），失配 → 409 tagged
  `wrong_incarnation` 零副作用。
- desktop / iOS 调用点改带身份（threads.ts:1272、GaryxGatewayClient:446）；
  身份来源 = 任一 feed/snapshot 响应（客户端启动即持有）。
- 内部同进程调用（bot 命令等非 HTTP 路径）同店无跨域风险，不加参（注明）。
- 客户端 ambiguous → forceReplacement；承诺边界不变。

### 4.5 已知边界

automation/hidden 不进投影；归档清 favorites 行 + bump-on-change；普通删除
无条件 bump。

## 5. 传输契约重构（R1）——CONFIRMED

semantic mode 全 helper 必填（含 PATCH、web-api.ts）；四分类；definitive
按接受集；迁移清单以「一切直接 HTTP 调用」为界。

## 6. Desktop / iOS 落点——同 v14，增量

- archive/delete 调用带 `expected_store_incarnation`；wrong_incarnation →
  按 §7.1 判定顺序处理。
- 其余同 v14（两处入口、第三 tab、行 accessory、range-fill + head 复核、
  snapshot 整替、per-feed 身份存储、`+Gateway` 切换全清、xcodegen）。

## 7. 客户端收藏状态机规格

### 7.1 全局状态与身份围栏（v15：判定顺序写死）

- raw / 写派发前置 / flight 三元组 / transport / presented /
  unresolvedFence 等值门：CONFIRMED。
- **身份状态（round15-1）**：`incarnation = unknown | known(uuid)`（per
  gatewayScope + runtimeEpoch）。**bootstrap acceptance**：当前 epoch 的
  首个合法身份响应 unknown → known 直接接受，不视为 mismatch、不清场。
- **响应判定三步（round14-4，一切 favorites/feed 响应与 flight 结算共用）**：
  1. 校验响应所属 `(gatewayScope, runtimeEpoch, ticket/requestToken)`；
  2. **旧 epoch → 直接丢弃**（不检查 incarnation，防旧页触发二次清场
     切回旧域）；
  3. 当前 epoch：身份 unknown → bootstrap acceptance；
     `known(a)` 且响应为 `b ≠ a` → 真实换域 → scope-clear（epoch bump、
     废弃 flight/effect、清 raw/水位/fence/意图/feed、重拉 snapshot 建
     新基线；意图不跨域保留）。
- **mutation readiness（round15-1 + round16 修订）**：
  - 分级前置：**favorites 写** = 当前 epoch known(uuid) **且已持有 raw
    revision**（须经 favorites GET / snapshot；裸 recent 不够）；
    **archive/delete** = known(uuid)（任一含身份响应即可）。
  - unknown → 点击时惰性 bootstrap（有界等待），失败按今日网络失败 UX
    报错、不派发；入口不常驻禁用。
  - **action token（round16-1）**：mutation 点击时捕获 `(gatewayScope,
    runtimeEpoch, actionToken)`；bootstrap await 返回后、网络派发前重校验
    全三元，scope/epoch 变化 → 操作永久取消（新域身份不得唤醒旧操作）。
  - **目标 provenance（round16-1）**：feed/snapshot 行携带来源响应的
    `storeIncarnation` 出处；mutation 仅允许目标 provenance == 当前
    known(uuid)；provenance unknown（持久化 fallback 缓存行）或失配 →
    不派发，触发 feed 刷新，用户在新行上重新确认。bootstrap 只建立读
    基线，不授权来源身份未知的目标。
  - 平台落点：desktop IPC 携带点击时 gateway scope，main 派发前与当前
    settings 匹配；iOS bootstrap await 后、调 client 前重校验 generation。
- `server_boot_id` 失配（同 incarnation）= feed 整替提示（不清意图）。

### 7.2 意图 reducer——CONFIRMED 基础上 v18 增量三条

- **admission gate（round17-1）**：`toggle` 入场前置校验（§7.1 mutation
  readiness + 目标 provenance == 当前 known）；不满足不创建 intent（先
  bootstrap 复评，仍不满足 → 刷新 + 重确认）。v16 的「raw 未就绪意图排队」
  删除——不存在先于身份授权的 intent。
- **`latestDesired.authorizedIncarnation`**：入场时盖不可变戳；`dispatch`
  与 `backoffFired` 守卫：戳 ≠ 当前 known(uuid) → **`actionCancelled`**
  （终端：退休意图、清 effect、不重试）。
- wrong_incarnation settle → 按 §7.1 三步判定（不变）。

### 7.3 presented 过滤——CONFIRMED。

### 7.4 feed 协议——同 v14（snapshot 原子接受单元含 incarnation 维度；
range-fill + K 窗口 + 链尾 head 复核；单飞行道；幽灵行整替触发点收敛）。

## 8. 测试计划

**Gateway**

- **启动装配（round14-1）**：真实 RuntimeAssembler 启动 default / custom
  data dir 两路径——单次 DB open、无默认库破坏性初始化、锁成功持有。
- **锁（round14-2）**：双进程后者有界等待→超时中止；**unmanaged subprocess
  fallback restart 端到端**（spawn→等锁→旧退→新继续 serving）；CLOEXEC
  断言；锁先于 purge/clear 顺序断言。
- **cutover 屏障（round15-2 + round16-2）**：无锁旧父进程存活 → 新二进制
  启动 → destructive init 延迟至父退出后执行；**cap 到期父仍活 → 子进程
  释放锁退出、DB 零修改**（fail-closed 两分支确定性用例）。
- favorites：CAS/守卫/同快照组页/孤儿写/删除交错/无条件 bump；
  **incarnation**：wrong_incarnation 409 零副作用（favorites 与
  archive/delete 路由都测，archive 的校验早于 endpoint detach）；
  recovery 旋转后旧 expected 不可提交。
- **incarnation 旋转（round14-5）**：recovery 原子提交内单次旋转
  （crash/retry 幂等）；正常 reopen 不旋转；rotate 命令持锁执行。
- activity_seq / keyset / snapshot / auth tagged：CONFIRMED 项回归。

**传输契约（R1）**：同 v14。

**意图 reducer（双端）**

- 全矩阵回归；**判定顺序（round14-4）**：旧 epoch 页先到/后到两序均直接
  丢弃不触发清场；当前 epoch 换域页 → 单次 scope-clear；
  wrong_incarnation settle 走三步判定。
- **身份初始化（round15-1）**：cold start unknown → 首个身份响应
  bootstrap acceptance（不清场）；feed/snapshot 失败后深链打开线程 →
  archive 点击触发惰性 bootstrap（成功派发 / 失败报错不派发）；身份响应
  晚于生命周期点击到达的两种顺序（双端）。
- **round16-1 双反例**：`cached A/X（provenance unknown/旧）→ bootstrap
  B/X` → 不派发、feed 刷新、要求重确认，B 收到零 mutation；`click A →
  switch gateway B → B identity arrives` → action token 的 scope/epoch
  失配 → 操作永久取消，B 收到零 mutation（双端）。
- **round17-1 延伸**：provenance 拒绝（未建 intent）→ B 行刷新成功 →
  推进全部 timer/effect → 第二次显式点击前 B 恒零 mutation；
  authorizedIncarnation 失配 → actionCancelled（不重试）；admission gate
  下不存在先于授权的排队意图。
- **round17-2 入口矩阵**：pinned 窗口外行（点查修补）、深链、workspace/
  bot 钻取、新建线程——各入口行均带 provenance，favorites/archive 入场
  校验可判定（不再有永拒或全局补标路径）。
- **readiness 分级**：裸 recent 后 favorites 写仍不 ready（缺 raw
  revision）；favorites GET/snapshot 后 ready。
- **cutover fail-closed（round16-2）**：父退出 → 继续启动；cap 时父仍活 →
  子进程释放锁退出、DB 零修改（两分支断言）。

**feed（双端）**：同 v14（snapshot 原子接受含 incarnation；range-fill 全
场景 + K 超限 + 链中上移 + head 复核；boot_id 整替；forceReplacement 三
路径；presented 全场景）。

**其余**：desktop/iOS UI 回归；端到端 curl（含 wrong_incarnation、
subprocess restart）+ 双端走查（`garyx-product-ui`）。

## 9. 实现切分（六步，同仓同发）

1. **gateway-lock**：启动装配重构（单 DB 实例）+ flock 有界等待/CLOEXEC +
   store_incarnation_id + rotate 命令 + recovery 原子旋转 +
   repository-contracts 恢复流程文档更新。
2. **gateway-favorites**：表/CAS（revision+incarnation 双围栏）/API +
   snapshot + auth middleware tagged + delete 无条件 bump +
   **archive/delete incarnation 先验**。
3. **gateway-recent**：activity_seq 全套 + seq keyset + 行级 seq + boot_id
   + source-guard。
4. **传输契约**：iOS + desktop（含 web-api.ts）。
5. **双端状态机与 feed**：reducer + 三步判定 + range-fill（K 窗口 + head
   复核）+ snapshot 原子接受 + lifecycle 身份携带与 forceReplacement
   （先测后 UI）。
6. **UI**：入口与 tab + xcodegen。

**独立后续任务（已立项）**：生命周期端点幂等化。
