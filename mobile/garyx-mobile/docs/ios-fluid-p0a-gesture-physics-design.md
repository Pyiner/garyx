# P0-A 设计 v41：交互式返回 + 手势物理统一

母计划：`mobile/garyx-mobile/docs/ios-fluid-interface-optimization-plan.md`。依据 Apple *Designing Fluid Interfaces*。**仅 iOS 26**。锚点与核对以 `origin/main` 为准（现含 A1/A2：`03d999a38`）。

**版本史**：v1-v11 见历轮。r7-r10 四轮评审将战线推入消息投递可靠性子系统（durable outbox / gateway 幂等与原子 create+dispatch / 附件 scope 绑定与 TTL / 跨端 canonical delivery state）——这些是**既有系统债务**（今日已存在），非本批引入，且正确解法跨 gateway+双端。→ **v12（范围裁决）**：**P0-A 收敛回导航/手势本体**；composer/发送/附件采用**行为等价迁移契约**（§1.5，不改这些代码路径、暴露面不扩大、已知问题登记+非回归断言）；投递可靠性全部内容（含 r7-r10 已定稿的 ScopeBoundOperationContext、operation 状态机、manifest、at-most-once 协议、PayloadConflictSet 等设计资产）**整体移交独立的「P0-G 消息投递可靠性批」**（已登记入母计划，gateway+双端一体设计，前序定稿内容随迁复用）。导航侧历轮 CONFIRMED 契约全部保留不动。v12 FAIL（r11：范围裁决被接受）→ v13：激活缝/可观测等价/trace 非回归/全表内联。v13 FAIL（r12：V12-4 闭环）→ v14：闭 r12 五条——激活缝**逐相位状态机**（live→closing→transferred，settle source 保持 live，仅 commit 触发 close，closing capability 独立存续至 ack）；legacy in-flight completion **显式豁免**并单独计数；**RouteOccurrenceID 与 composer key 分离**（同 conversation 可多 occurrence 入栈，保回发起页语义，仅栈顶 occurrence 绑活 adapter）；双 oracle 非回归（业务调用=基线等价、新导航生命周期=规范期望 trace，随机字段注入/归一）；A4b 措辞与版本标记修正。v14 FAIL（r13：双 oracle 闭环）→ v15：闭 r13 五条——key 级草稿状态与每次 activation 的 `ComposerInputSession` 分离 + 同 key handoff 线性化（close 发出刻**原子预留下一 generation/sequence 区间**）；能力矩阵 source 拆两列 + activation 的 outcome×visibility 结果表 + commit 权威步骤 `beginClosingAndCaptureFinalText`；occurrence 规则统一到全部测试与切分文字；legacy effect ticket 线性化"已准入"判定；母计划版本同步。v15 FAIL（r14：occurrence/能力列/母计划闭环）→ v16：闭 r14 五条——key 级状态仅含现有 per-key 文本（附件保持全局与基线清空语义，守住 P0-G 边界）；`InputSessionEpoch` 与 `PayloadGeneration` 分离 + beginClosing 原子快照 reducer（迟到 close 只终结旧 epoch 记录绝不写当前草稿）；activation 结果表加 route 类型与 key 相等条件；result-bearing lease 增 dismissal×result join 态；版本史残留修正。v16 FAIL（r15：F1/F3/F4/F5 闭环，仅剩输入双代际未统一）→ v17：§1.4 唯一权威协议/五元组/reducer/两序。v17 FAIL（r16：术语统一与母计划闭环）→ v18：闭 r16 四条——alias 生命周期与文本空值解耦（显式回收条件决定表）；adapter 关闭协议 MainActor 临界区四步（先 `unmarkText()` 终结 IME 组合再固化 finalText）；reducer 补全五元组分支（SessionID×epoch 绑定/重复 seq/future gen/closing 异 gen/finalText 不可变）；A4b 术语残留清理。v18 FAIL（r17：r16 四条全闭）→ v19：闭 r17 三条——关闭协议泛化 `live → finalizingInput → closing`（producer registry：marked text 同步终结 + 听写异步 terminal，producer 集合空才固化 finalText；path 写入不被 finalizing 延迟，同 key adapter 授予 = terminal ∧ finalize 完成）；升格后 source draft 进入 `retiring` 自然产生 identity-discard（alias 有界）；`ComposerInputSession` 永久绑定 GatewayScope（alias 表按 scope 分区，reducer 先验 scope lifecycle）。v19 FAIL（r18：registry/retiring 方向确认）→ v20：闭 r18 四条——输入终结统一为**事务正交状态机**（release=冻结+清焦+提交 path；producer terminal=固化 N/建 N+1/发 close；ack=retired），全文旧"commit 同刻 close"条款清除；焦点拆解与 `InputFinalizationLease` 分离（resign 立即、drain 脱离 first responder、五类事件确定性 cancel、finalizer 入预算）；**撤回 suspended 草稿保留**（gateway switch 按基线清空 composer，scope 绑定只做拒串写）；alias 不变量冻结（drain 后 =0 + bytes 上限）。→ v21（老板指令转向 2026-07-18）：「架构以最漂亮为准，历史行为不对直接改，不做行为等价绕行」——**废除 v12 起的行为等价桥接层**（legacy 豁免/effect ticket/trace 冻结双 oracle 全删，它们是 r11-r18 findings 的接缝之源）；**r7-r11 已定稿的干净客户端架构拉回本批**：ComposerPayloadStore（EntryID 主键/统一载荷/废全局数组）、ScopeBoundOperationContext（上传目的地冻结）、OperationCapability 状态机、process-death manifest、payloadPreparing 发送锁、PayloadConflictSet、durable at-most-once 发送协议（三合一事务/ambiguous 用户出口/kill-relaunch 矩阵）、scope 化 composer（suspended 保留、切回恢复——r18-F3 按新指令反转）；**P0-G 收缩为纯 gateway 侧**（幂等账本/原子 create+dispatch/附件 TTL）；跨端 DurableDeliveryState 进 canonical spec（iOS 先行，Mac 对齐列跟进任务）。导航本体不变。v21 FAIL（r19：转向被接受、导航零回退）→ v22：闭 r19 九条——SendBarrier × Finalization 乘积表（唯一线性化 + 缓冲输入归宿 + 失败原子恢复）；create-response 丢失如实建模为 ambiguous（撤"无幽灵 thread"保证，唯一索引列 P0-G 前置）；事务 terminal 为 producer 必达 cancel 事件（活性有界）；§1.1 附件旧条款删除（统一 EntryID/scope 规则）；revoke × delivery record 逐状态表；ConflictSet 改 durable candidate list（配额计量永不静默丢）；manifest 逐状态恢复表（attempted 不自动续跑）；A3 定义单一 ComposerDurabilityStore + A4d 拆两层；beginClosing 残留清理。v22 FAIL（r20：F4/F7/F9 闭环）→ v23：闭 r20 七条——seal 降为 provisional、**durableCommitted 为唯一线性化点**（失败=provisional 撤销合并而非回滚）；`SendCommitBarrier`（每 EntryID 短生命）与多 `DeliveryRecord` 分离（支持 busy 续发/Queue-Steer）；producer terminal 双边界 oracle 统一（六类残留修正）；`DeliveryEvidenceIngress` 独立入口（只读 correlation 不入 domain）；create 唯一索引正式登记 P0-G；candidate membership 元数据成本固定预留 + 拒绝时 pin source 禁 alias 退休；durability 层归属唯一化（A4d-1 先于 A4b）。v23 FAIL（r21：F3/F4/F7 闭环）→ v24：闭 r21 五条——`SendReservationID` + PayloadGeneration 单调永不复用（撤销消耗代际，合并落新代）；final snapshot 延迟物化至 barrier 结算（成功/撤销都先持久化 follow-up snapshot 再 idle/inputReady），文本/附件/capability 逐格转移表；DeliveryRecord durable 配额与背压（seal 前预检、超限留草稿可见反馈、非 terminal 永不静默淘汰）；ConflictSet 准入 fail-closed（准入不可 durable 则 domain promotion 不提交，source 保持权威）；母计划 P0-G 第四项落笔；六类 cancel 逐项列名。v24 FAIL（r22：F2-F5 全闭）→ v25：闭 r22 三条——事件身份升**六元组**（+ReservationID?）+ 唯一乘积 reducer `ReservationPhase × ProducerFinalizationPhase`（双边 terminal 才物化 final，§1.4a 与 awaitingBarrier 矛盾消除）；generation 分配器 = durable hi-lo 水位（批量预提升，重启续用永不复用，单 seal 无同步 fsync）；capability 分支修正（committed 后 DeliveryRecord payload 不可变，post-seal operation 恒属 follow-up composer），ReservationID 入 context/manifest/恢复表；§7 更新为新协议全分支。v25 FAIL（r23：方向确认，传播差三处实文）→ v26：乘积 reducer **12 格真内联**（committed×finalizing 写 G+1、revoked×finalizing 写 G+2、仅 finalSequence 固化后拒绝）；独立 `OperationCapabilityKey` schema 实文入 context/manifest/恢复表（不复用输入六元组）；provisional reservation ledger 初版。v26 FAIL（r24：F2 CapabilityKey 闭环）→ v27：producer terminal 在 reservation sealed 时**只记录 `producerDrained`**（不物化/不建 N+1/不 close），双边 terminal 后单一 durable 事务完成「选代→物化→建 N+1→exactly-once close」（§1.4a/§2.2a/§4 全同步）；**ledger schema 实表 + operation×attempted×scope 交叉恢复矩阵内联**，合成终局五步定义为单一事务；版本史双"本版"清理。v27 FAIL（r25：提前固化反例修复、schema/矩阵落地）→ v28：ledger admission 扩为「ledger → 一切 durable descendant（producerDrained/manifest）→ 网络」且 producerDrained 与 ledger upsert 原子提交（无 operation 的纯输入路径也有 ledger，孤儿态消除）；kill 语义拆分（drained commit 前 U 可丢、commit 后必恢复 G+2）；operation terminal 拆四态 `completed/failedRetryable/failedTerminal/cancelled` 逐状态×scope 恢复行 + 双重 relaunch 测试；settle 兜底措辞改 producerDrained。v28 FAIL（r26：r25 三条全闭、CRITICAL 清零）→ v29：`failedRetryable` 纳入 `.payloadPreparing` 发送锁（先显式重附/移除才能发；重附=原记录原子 supersede + 新 operationID；beginSend 不推进代际）；`failedTerminal` 的 suspended/revoked 行改 scope 隔离语义（suspended 反馈留 origin 分区、切回+interaction owner 后显示一次；revoked 清理归档零用户 UI）。v29 FAIL（r27：CRITICAL 保持清零）→ v30：operation 状态机增 `superseded`——重附 = `failedRetryable(O1) → supersededBy(O2)` 原子转移（吊销 O1 capability、stagedAssetID+配额预留+manifest ownership 转 O2、superseded 不清文件），failpoint 断言恒一个文件 owner；一切非 revoked 反馈持久化为 (scope, EntryID, operationID) 一次性记录、仅匹配 EntryID 的 host 获 interaction owner 后呈现并消费；版本史仅留本版标记。v30 FAIL（r28）→ v31：换新文件重附改 `pendingReplacement`+单一原子 swap 事务；`ComposerPayloadEntry` 子项身份显式化（terminal 只删子项，EntryID 空且无引用才回收）；durable feedback 状态机入 §1.5 正文（持久化 inline chip + 用户动作 durable ack）。v31 FAIL（r29）→ v32：cancelled 矩阵行改仅删子项（整 Entry 清理只由显式 reset/scope revoke 定义）；durable `ReplacementRecord` 入 manifest 家族（启动/revoke 对未提交 swap 统一 abort+释放配额+删文件，已提交恢复 O2，测试三断言 owner/磁盘/配额）；feedback ack 并入动作事务。v32 FAIL（r30）→ v33：ReplacementRecord schema 补 scope+新旧完整 CapabilityKey+可选 reservation（nil-reservation 常规重附走 scope 分区事务，仅 non-nil 要求 ledger 先行；O2 terminal 后 record GC）；failedTerminal 转移+feedback 插入+Entry 引用+清理登记=同一 durable 事务（物理删文件提交后幂等执行）；identityDiscard/routeInvalidated 原子归档 feedback。v33 FAIL（r31）→ v34：Entry/destination 增 durable identity lifecycle 终态（失效事务同时吊销相关 capability；completion 事务三重 CAS operation×scope×identity，失效后只归档+清文件不建 feedback，零复活）；promotion **恒保持 EntryID**（"或迁移"分叉删除，targetMapping 只改 alias/generation/branch 映射）；ReplacementRecord 回收表六行定死。v34 FAIL（r32：promotion 稳定 EntryID CONFIRMED）→ v35：identity 事件拆四类（AliasSourceRetired/RouteOccurrenceSuperseded/DestinationDiscarded/PayloadEntryDiscarded），capability 绑稳定 payload lifecycle token（promotion 保持 active 只迁 alias，仅领域删除/显式销毁可吊销）；identity-discard override 表覆盖全 operation state × ReplacementRecord phase（failedRetryable 原子取消、pendingReplacement abort、资源归零）；failedTerminal 重附原子事务。v35 FAIL（r33：r32 三方向 CONFIRMED）→ v36：新增 `PayloadGenerationReset`（只清当前代，保 Entry token/历史 DeliveryRecord/ambiguous 出口——普通清空不再误升终局销毁）；`PayloadLifecycleToken` 升为**一切 durable payload mutation 的统一准入门**（presentation result/manifest admission/pendingReplacement/swap/fresh operation/operation transition 全 CAS token）；lineage tombstone 回收条件改 feedback 非 terminal（ack 或 archived 同事务释放）；事件模型传播。v36 FAIL（r34：admission CAS 与 lineage 回收 CONFIRMED）→ v37：`PayloadGenerationReset` 准入条件 = `barrier idle ∧ producer live`（sealed/finalizing 拒绝，UI 该窗口禁用清空——毫秒级瞬态；无 Reset×Reservation 组合态）；`ComposerInputSession` 捕获 PayloadLifecycleToken/revision，**§1.4 全部 durable reducer 事件 + beginSend + reset 同事务 CAS token**（discard 赢即终结 finalizer 拒绝后续输入）；五事件命名与矩阵全文传播。v37 FAIL（r35：reset gate 与输入 CAS CONFIRMED）→ v38：token 生命周期改三态 `active → discarding → discarded`——discarding 拒普通输入/admission、但**专用 settlement capability** 原子收敛在途 barrier/finalizer（producer、reservation ledger、DeliveryRecord/evidence、资源清理），全部收敛后才 terminal；`Reservation × Producer × 两类 discard` 决策表内联；`context replacement` 从代际边界删除（key/activation 切换走 session handoff）；drain/双边事务入 discard 矩阵。v38 FAIL（r36：三态收敛主体确认）→ v39：`DiscardFinalizationTombstone`（只含 token/revision/session/epoch/finalSequence/disposition，无文本附件路径；settlement 事务内直接 terminal 不等 adapter ack，closePendingAck 原子转入、迟到 ack 幂等拒绝）；discarding 表 producer 维拆三列（live-finalizing/closePendingAck/retired）；committed DeliveryRecord 按 delivery phase 三分（notDispatched=cancelledByDiscard 清 envelope/attempted-ambiguous=evidence/acknowledged=terminal evidence，竞争由 record CAS 决胜）；discarding 全部持久化边界入真实 kill harness。v39 FAIL（r37：r36 三条闭环）→ v40：DeliveryRecord 从 discard 表**完全正交拆出**（settlement 对 Entry 每个 record 按 phase 独立 CAS，与当前 barrier 相位无关）；4×3 表明确为**逐 session reducer**（tombstone 键 =(token, discardRevision, session, epoch)，discarded 前断言 descendant 集合空）；tombstone 有界回收契约。v40 FAIL（r38：仅剩一条）→ **v41（本版）**：session settlement membership 改按稳定 `PayloadLifecycleToken`/EntryID 枚举（ComposerKey/alias 只作事件路由）——分量 3 遍历该 token 捕获的全部 (sessionID, epoch)，跨 promotion/alias 不变；补 D:S₁ pendingAck→promote→T:S₂ live→discard 全序测试。

**已定案项**（不再复述）：A0 结论与系统曲线；D2 物理层（**已合 main**：`03d999a38`，A1/A2 验收完成）；home 呈现节点；UIKit 容器 VC + wrapper 变换契约；五维 owner 相位矩阵（§2.2）；touch-down 快照 edge 判定与仲裁表；drilldown predecessor；按钮同时序；部署目标 26（已合 main）。

## 0. 目标

跟手、速度交接、动量投影、rubber-band、可中断（cancel-settle 可 regrab；commit 不可逆）。

## 1. 路由模型

### 1.1 canonical path 与身份

```
rootNavigationPath: [RouteEntry]   // RouteEntry { id: RouteInstanceID, destination }
RouteDestination = .conversation(threadID) | .conversationDraft(draftID)
                 | .panel(p) | .settingsDetail(tab) | .workspaceDrilldown(d)
```

- path 唯一路由真相；selection 为栈顶派生投影；实时数据走 destination 索引 live store；context 只带不可变小 scope。
- **双 revision**：`stackRevision`（拓扑）/ 每 entry `payloadRevision`（entry 内容原地变更）。用途限定：**拓扑 CAS 与 relative intent 依赖校验**（§2.1）；**文本/输入回调不用 payloadRevision**（改用输入代际，§1.4，r5-F1）。
- 实例规则（r12-F3 重定义：**occurrence 与 composer key 分离**）：`RouteInstanceID` 即 **occurrence 身份**——同一 conversation 允许作为**新 occurrence** 再次入栈（`A→P→从 P 再开 A`：P 保留，返回先回 P 再回 A，**回发起页语义不破坏**，钉住现有测试 `testConversationOpenedFromCurrentPanelReturnsToThatPanel`）。**key 级持久状态 = per-key 文本 + per-key 附件/payload（经 §1.5 ComposerPayloadStore，EntryID 主键、scope 分区——v22 统一，全局附件数组废除）**；测试补 A 附件→B→回 A 恢复、revoke 清理；**`ComposerInputSession` 按 activation 实例化**（协议见 §1.4）——live adapter 全局唯一，绑定该 key 栈顶 occurrence。**同 key handoff**：按 §1.4 乘积 reducer——epoch N 在**双边 terminal**（producer finalization ∧ reservation 结算）刻物化并创建 N+1，S₂ 不等 N 的 close ack——**完整输入协议以 §1.4 为唯一权威**。`.conversationDraft` 同 draftID 仍单 occurrence（草稿无再入路径，准入聚焦既有）。

### 1.2 home 呈现节点（已定案）

`RoutePresentationNode = .home | .entry(RouteEntry)`；首层 pop 的 destination = `.home`。

### 1.3 draft → thread 升格：coordinator event（v21 恢复完整协议）

`promoteDraft(instanceID, draftID → threadID, originScope, clientIntentID, sendStage)`：

- entry payload 变更（bump payloadRevision），不动拓扑、不失效进行中 pop；无过渡不重建 host；迟到升格（instance 已出栈）只迁 domain/live-store 数据，绝不重插 path。
- **gateway scope 结算**：升格携带发起时不可变 origin scope。到达时 scope 非当前 → 不改当前 path/lease，数据落 **origin scope 分区的 composer/domain store**（切回可见，§2.1a）。
- **send 阶段确定转移表**（配合 §1.6 发送协议）：`threadCreatedButNotDispatched` → origin scope outbox（持久化成功 = `.failedRetryable` 安全重试；失败 = typed failure 原子恢复 draft）；`dispatchInFlight` → 只 reconcile 绝不自动重发（§1.6 ambiguous 出口）；`serverAcknowledged` → 正常迁移。dispatch 次数单独断言。
- duplicate-draft：同 draftID 单 occurrence（§1.1）；升格冲突 → `PayloadConflictSet`（§1.5）。
- 测试：升格 × pop 四相位 × finish/cancel × gateway switch 全组合（最终 store/path/lease/optimistic/outbox/dispatch 次数写死）；迟到升格；三阶段各终态。

### 1.4 输入协议（唯一权威，r15-F1 统一）

**事件身份六元组**：`(ComposerKey/alias, ComposerInputSessionID, InputSessionEpoch, PayloadGeneration, ReservationID?, InputSequence)`——`ReservationID` 在 sealed 窗口内的事件必填；**`ComposerInputSession` 铸造时捕获 `PayloadLifecycleToken`/revision（r34-F2）：§1.4 一切 durable reducer 事件（edit/close/drain 落库）以及 `beginSend`/`PayloadGenerationReset` 在同一事务 CAS `token == captured ∧ active`**——`DestinationDiscarded`/`PayloadEntryDiscarded` 赢时终结 InputFinalizationLease、后续输入一律拒绝（旧 adapter 送达的 edit/close 不会写回已销毁 Entry）；**`ComposerInputSession` 永久绑定 `{GatewayScopeID, scopeEpoch}`**；reducer 先校验 scope lifecycle 再于**该 scope 分区**的 alias 表解析 key（不同 gateway 同名 key 互不可见）。**scope 化 composer（v21，按老板指令直接建新语义）**：gateway switch = 旧 scope suspended——composer 状态（per-key 文本+payload store 条目）**保留在 origin 分区，切回恢复**；切换时旧 scope 的 live session 确定性 close（§1.4a cancel 事件，drain 落 origin 分区）；suspended 期间迟到 edit/close/result 落 origin 分区、不触当前 scope；logout = revoked：**逐 delivery-state 结算（r19-F5）**——`notDispatched/未尝试` 记录安全取消并擦除 payload；`transportAttempted/ambiguous` 原子转 `UserDisposition.scopeRevoked`：擦除 payload、保留有界 correlation evidence。**`DeliveryEvidenceIngress`（r20-F4 独立入口）**：迟到 frame 经专用 ingress——只读取认证来源 + correlation ID、更新 evidence tombstone，**内容不进 domain/composer**；scope 水位 gate 作用于 domain 事件，evidence ingress 与之正交不冲突。测试：revoke 回调与 frame 同帧两序；draft/payload 条目清理。旧"switch 即清空全部草稿"行为**废除**。测试：logout × §1.6 每个 kill 边界 × 迟到 frame × 重新认证。测试：G1 起草→切 G2（G2 空 composer）→G1 迟到事件落 G1 分区→切回 G1 恢复原状；revoked 零复活。

- **`PayloadGeneration`（发送代际）**：`beginSend` 触发 §1.6 `SendCommitBarrier` 的 **seal（provisional reservation）**——同步捕获 payload + `clientIntentID`、G→G+1 预留、UI 乐观清空；**线性化点是 `durableCommitted`**（v23，seal 可被 provisional 撤销，撤销表见 §1.6）。durable 后该 send 的异步成败不再推进代际、不得覆盖新代 follow-up。显式 reset（= `PayloadGenerationReset`，§1.5 五事件——只推进代际，不销毁 Entry）是唯一的用户级代际边界（r35-F2：**"context replacement" 表述删除**——宿主/key 切换属 session handoff，由 `InputSessionEpoch` 表达，不推进 PayloadGeneration）；**升格与 close 都不是**。
- **`InputSessionEpoch`（会话代际）**：activation handoff 边界。**final 物化 = 双边 terminal（r22-F1 唯一时序）**：`ProducerFinalizationPhase` terminal（drain 完成/六类 cancel）**∧** `ReservationPhase` terminal（idle/committed/revoked；无活跃 barrier 时该边平凡成立）——两边都 terminal 才固化 N final、创建 N+1（§1.4a 的"drain 完成刻固化"仅适用于无活跃 barrier 的常态；有 barrier 时按 §1.6 分支物化：committed→G+1 缓冲、revoked→G+2 合并）。与 PayloadGeneration 正交。
- **`InputSequence`**：epoch 内单调写序。
- **reducer 决定表**：

**乘积 reducer 12 格表（r23-F1 真内联，唯一权威）**：`ReservationPhase{none, sealed, committed, revoked} × ProducerFinalizationPhase{live, finalizing, terminal}`。**物化与 close 只发生在双边 terminal**（producer terminal ∧ reservation ∈ {none,committed,revoked}）；**producer 事件的拒绝只以 `finalSequence` 已固化为准**，与 reservation 结算先后无关；但须先通过下方六元组身份表，closing epoch 的异 generation 事件恒为审计，不进入同 generation 的 sequence fault 判定：

| Reservation \ Producer | live | finalizing（drain 中，finalSequence 未固化） | terminal（finalSequence 已固化） |
|---|---|---|---|
| **none**（无 barrier） | 常规写当前草稿 | 合法事件（含听写 result）继续入 drain 缓冲 | 拒绝（>finalSequence=fault；≤=去重审计） |
| **sealed**（窗口内） | 写 G+1 provisional 缓冲 | 入 drain 缓冲（G+1 视角） | 拒绝同上 |
| **committed** | 写 G+1（已转正草稿） | **合法事件继续写 G+1 drain 缓冲（听写 result 不丢）** | 拒绝同上 |
| **revoked** | 写 G+2（合并草稿） | **合法事件写 G+2 drain 缓冲** | 拒绝同上 |

（"旧 reservation 事件按终局记录去重/审计"仅适用于 **producer terminal 之后**到达的事件；terminal 前一律按上表入缓冲。）下方决定表各行在此乘积上取值（Core 测试 12 格 × commit/revoke × 两种 terminal 顺序 × 听写 result 文本终值断言）。**SessionID×epoch 绑定（r16-F3）**：一个 epoch 恰好绑定一个 `ComposerInputSessionID`；adapter 在同一 activation 内重建时**必须沿用原 SessionID 与 sequence 空间**，否则走新 epoch handoff——SessionID 与 epoch 不匹配的事件按契约错误处理。

| 事件 | 处理 |
|---|---|
| 当前 epoch + 当前 payload gen，seq > lastApplied | 应用到当前草稿 |
| 当前 epoch + 当前 payload gen，seq ≤ lastApplied | 重复/乱序：忽略（幂等去重） |
| 当前 epoch + 旧 payload gen（beginSend 已推进） | 拒绝回写草稿，仅审计 |
| 当前 epoch + **future payload gen** | 契约错误：log fault + 丢弃 |
| SessionID 与 epoch 绑定不符 | 契约错误：log fault + 丢弃 |
| closing epoch N + 同 gen，seq ≤ finalSequence | 仅审计去重；**finalText 一经 close 固化不可变** |
| closing epoch N + 异 gen | 仅审计，不动 closing record 主体 |
| closing epoch N + 同 gen，seq > finalSequence | producer 契约错误：log fault + 丢弃 |
| 重复 close(N) | 幂等 |
| retired / unknown epoch | 拒绝 |

- **§1.4a 输入终结状态机（r18-F1 唯一权威时序，事务的正交状态）**：

| 刻 | 动作 |
|---|---|
| **release（commit 决策刻，MainActor 临界区）** | activation `live → finalizingInput`；冻结新用户输入；同步 `unmarkText()` + drain marked text；**立即清 FocusState + resign**（r18-F2：焦点拆解不等异步 producer）；**提交 canonical path**（§4）——三者同刻完成，导航与键盘都不被异步 producer 劫持 |
| **producer terminal（drain 完成刻）** | pending 异步 producer 的终结由 `InputFinalizationLease` 持有（脱离 first responder）。producer 集合空时分两况（r24-F1）：**reservation 边已 terminal**（none/committed/revoked）→ 进入下行「双边 terminal 事务」；**reservation 仍 sealed** → 只持久化 **`producerDrained(finalSequence, drainBuffer)`** 记录——**不物化 final、不建 N+1、不发 close** |
| **双边 terminal（producer drained ∧ reservation 结算）** | **单一 durable 事务**：按 reservation 终局选代（committed→G+1 / revoked→G+2）→ 物化 `finalText`（drainBuffer 归入所选代）→ 创建 epoch N+1 initial snapshot → 发布 **exactly-once** close 屏障，`finalizingInput → closing` |
| **close ack** | closing record 落库 ack → `retired`；ack 前 pinned |

  - producer registry：marked text（同步）、听写（异步 result）、scribble 等；**确定性 cancel terminal（六类必达事件，不依赖定时器）**：`sceneInactive`、superseded、scope suspend/revoke、host teardown、**事务 settle terminal（r19-F3 活性保证：terminal 刻仍 pending 的 producer 确定性 cancel → 生成 `producerDrained`；若 reservation 边已 terminal 再进入双边事务物化）**——release→terminal 即 producer 等待窗口，窗口内 result 收录、窗口后到达丢弃（有界且事件驱动）；
  - **同 key** destination adapter 授予 = terminal(committed+visible) **∧** N final 已固化（授予前 destination 无输入可丢）；异 key/非 composer 不受影响；
  - **finalizer 入预算（r18-F2）**：outstanding `InputFinalizationLease` 与 retained adapter 计入 host ≤ 4 与内存预算；callback 永不返回场景由**六类 cancel 事件逐项兜底**（`sceneInactive` / `superseded` / `scope suspend` / `scope revoke` / `host teardown` / `事务 settle terminal`，任一到达即终结）；**terminal 兜底仅作用于已进入 finalizingInput 的 committed 路径**——cancelled+visible 的 source 保持 live，不受 terminal cancel 影响；
  - **原生 SwiftUI TextField 无法保证此序则必须换有序 UIKit adapter**；`endEditing` 仅作辅助；
  - 测试：marked range 中/日文组合态 commit；听写 pending→result（落 N final）/pending→failure/cancel；result 于 terminal 事件**前**到达→落 N final；terminal 先发生→cancel 固化、其后 result 被已吊销 capability 拒绝（双边界两用例）；callback 永不返回 × 六类 cancel 事件逐一兜底；连续跨异 key 导航累积 finalizer 不破预算；后台/注销组合。
- **`beginSend × release` 两序定义（r21-F1：final 延迟物化）**：
  - beginSend 先（release 落 sealed 窗口内）：producer drain 完成只落 `producerDrained` 记录（`awaitingBarrier`）；barrier 结算后走**双边 terminal 事务**（§1.4a 表）：durableCommitted → final = G+1 缓冲；revoked → final = G+2 合并草稿（T+U）。同 key `inputReady` = 双边 terminal 事务完成 ∧ barrier idle。
  - release 先：epoch N 进入 finalizingInput 后**拒绝 beginSend**；N final = drain 后草稿；epoch N+1 initial = N final，后续 beginSend 在新 epoch 推进代际。
  - 测试：两次连续 seal + 第一轮迟到回调（ReservationID 判别）+ release-before-fsync-failure（final 物化 = T+U@G+2）。
- **alias 生命周期（r16-F1：与文本空值解耦）**：升格原子建立 `draftID → threadID` alias，属 ComposerKey 解析层。**`beginSend`/reset/文本为空都不回收 alias**（现有 store 空文本即删 key 的行为只作用于文本条目，不作用于 alias 表）。回收条件（全部满足才可）：① identity discard/revoke；② source key 无 active/closing session；③ 无待 ack 的 closing record；④ 无进行中 promotion。**alias 回收的产生（r17-F2/r33-F4 更名）**：除 `DestinationDiscarded`/`PayloadEntryDiscarded` 外，**promotion 完成即令 source draft 进入 `retiring`**——②③④ 归零时产生 **`AliasSourceRetired`**（§1.5 五事件：只回收 alias 条目，不触碰 Entry/token/capability/feedback），正常使用即有界。alias 不变量冻结（r18-F4）：**`aliasCount == activeRetiringSourceCount`** 恒成立，全部 session/ack/promotion drain 后 **== 0**；alias 表 bytes 独立上限 64KB；churn 断言：**500 次 promotion（线程均保留）drain 后 aliasCount==0 且 RSS 稳态**。测试另含：**空草稿 promotion 后再输入**（send 清空→升格→G+1 follow-up 经 alias 落 T——retiring 期间 alias 仍有效，不得 unknown-key 拒绝或写孤立 D）。
- 测试：send 悬挂期间 follow-up 完整（成功/失败）；发送清空后旧 payload gen 回调拒绝；同 epoch 逆序拦截；**reducer 决定表全行**；SendBarrier × release **全中间相位矩阵**（epoch/payload gen/文本/缓冲归宿逐格断言）；close 屏障（≤finalSequence 去重、>finalSequence fault、重复 close 幂等）；epoch handoff 全组合（N+1 edit→N close、反序、连续两次、同 key push/pop/-N/inactive/superseded × 迟到 close × 新输入、事务 terminal 兜底 cancel）；close/ack 前 LRU churn 不丢 record；retired 后拦截；CJK 分段；send→promotion→follow-up。

### 1.5 ComposerPayloadStore 与宿主激活缝（v21：直接建干净架构）

- **实例规则**：见 §1.1（occurrence 与 composer key 分离；InputSessionEpoch 按 activation 实例化，活 adapter 绑栈顶 occurrence）。
- **`ComposerPayloadStore`**：文本、附件、pending picker/upload operation 统一入店；主键 = 不可变 **`ComposerPayloadEntryID`**；**Entry 内子项有独立身份（r28-F2）**：`attachmentID`/`operationID` 为子项键——operation terminal（failedTerminal/cancelled）**只删除该 O 的 `stagedAssetID` + operation record**，同 Entry 的文本与其他附件不动；**EntryID 仅在内容为空且无 delivery/feedback/alias 引用时回收**（feedback 引用在场则 Entry 保留，反馈永有匹配对象）。测试：文本+两附件其一失败/取消，其余内容与反馈可达性不变。（threadID/draftID/alias 只是索引）；**按 GatewayScope 分区**；**废除全局 `composerAttachments`**。payload 是**用户数据**（与 RouteInstanceStateStore 缓存分离、不入 LRU）：raw binary 落受保护磁盘暂存（排除备份、复制前原子配额预留，内存只留缩略图/路径/metadata）；operation record terminal 前 pinned；send/reset/移除/scope revoke 确定性清理本地文件；配额超限 = 用户可见失败。**per-key 附件随 key 保留**（旧"切 key 清空全局数组"行为废除——按新架构语义）。
- **`OperationCapabilityKey`（r23-F2 独立 schema，不复用输入六元组）** = `{scope identity/epoch, EntryID, generation, ReservationID?, branch, operationID}`——context、manifest、DB schema、恢复表统一使用该 key。**`ScopeBoundOperationContext`**：presentation request 接受（lease token 铸造）同刻创建不可变 `{OperationCapabilityKey, 固定 client/configuration}`；operation 一切网络步骤只经该 context（禁用全局 `client()`）——G1 文件永远传到 G1。suspended：继续，结果落 origin 分区；revoked：取消拒收。
- **`OperationCapability` 状态机（r27-F1 增 superseded）**：`requested → preparing → uploading → completed | failedRetryable | failedTerminal | cancelled | superseded`；移除/reset/revoke 先原子 CAS 到 cancelled + 吊销 capability 再清文件；completion 事务**三重 CAS（r31-F1）**：OperationCapabilityKey 全字段匹配 ∧ 状态==uploading ∧ **scope lifecycle 有效 ∧ identity lifecycle 有效**才提交；identity 已失效 → **只做 operation 归档 + 文件清理，不创建 feedback、不建 Entry 引用**。异步 completion 凭 capability 落 EntryID，不经 host writeAuthority。
- **process-death manifest + 逐状态恢复表（r19-F7/r23-F2）**：无密 `{OperationCapabilityKey, staged path, state, uploadAttempted}` 持久化。重启结算：`requested/preparing`（picker 已消失）→ 原子 cancel + 清理 staging；`uploading 且未 attempted` → scope/auth 匹配可安全重试（请求未发出过）；**`uploading 且 attempted` → 不自动续跑**（gateway 每请求生成新 UUID 路径，续跑必产生第二份远端文件）——转 failed-retryable 由用户显式重附；gateway 幂等上传（P0-G）落地后才开放 attempted 自动续跑。测试：上传请求发送前/服务端写入后响应前/ack 后逐点 kill；连续两次 kill/relaunch（failedRetryable 不衰变）；无 operation 的 seal→输入→release→drained commit→kill 序列（G+2 + close exactly-once）；**gateway switch/logout × failedTerminal × relaunch**（suspended 反馈只在切回后显示一次、revoked 零 UI）。
- **`beginSend × pending operation` = `.payloadPreparing`**（对齐 Mac，r26-F1 扩展）：条目有 `requested/preparing/uploading` **或 `failedRetryable`** operation 时返回 `.payloadPreparing`——**不推进 generation、不发送**；`failedRetryable` 必须先**显式重附或移除**：重附 = **原子转移事务** `failedRetryable(O1) → supersededBy(O2)`：同事务内吊销 O1 capability、铸造 O2（新 OperationCapabilityKey）、把 **stagedAssetID + 配额预留 + manifest ownership** 转给 O2——**`superseded` 不清理文件**（区别于 cancelled）；用户选择新文件的重附（r28-F1/r29-F2）= 新文件先入 **`pendingReplacement`**，同刻持久化 **`ReplacementRecord{ReplacementID, scope, EntryID, oldKey(O1 完整 CapabilityKey), newKey(O2，swap 时填写), reservation?/branch?, stagedAssetID, reservedBytes, phase}`**（manifest 家族；**仅 `ReservationID != nil` 时要求 matching send-ledger 先行，nil 的常规重附直接走 scope 分区 durability 事务**——发送前附件无 SendReservation，r30-F1）→ **单一原子事务**：激活 O2 + 配额/manifest ownership 转移 + supersede O1 + ReplacementRecord phase=committed——事务失败 O1 原样保留；**启动/scope revoke 恢复**：phase 未 committed 的记录统一 abort（释放 reservedBytes 配额 + 删 provisional 文件），committed 的按 **newKey 唯一恢复 O2**；提交后才清旧文件；**ReplacementRecord 回收表（r31-F3，按 O2 终态 × scope lifecycle）**：`completed`=targetMapping+文件结算后回收；`failedTerminal`=清理 journal 与 feedback durable 落库后回收；`cancelled`=文件结算后回收；`superseded`=下一 owner 事务提交后回收（**连续重附链逐环回收，不累积**）；scope `revoked`=随 scope settlement 回收；`failedRetryable`=保留（活跃 manifest 持有），被新 ReplacementRecord supersede 后按 superseded 行回收。测试：连续重附 churn/kill-relaunch/revoke，断言 record、文件、配额元数据最终有界归零。测试三断言（有效 owner 恒一、磁盘无孤儿、配额归零）× **nil/non-nil reservation 两路** × kill/revoke/relaunch。移除 = cancelled 行处理。failpoint（ENOSPC/fsync/kill-relaunch）断言**任一时刻恰有一个有效文件 owner**。测试：failedRetryable 在场 beginSend 不推进代际；重附后旧 operationID 事件拒绝。
- **`PayloadConflictSet` = durable candidate list（r19-F6）**：冲突载荷是**用户数据**——每个候选持稳定 EntryID 入持久化列表，由 payload 配额计量，**永不静默丢弃**；**既有 EntryID 的 candidate membership 元数据成本固定预留**（配额只挡新增二进制）；**准入 fail-closed（r21-F3）**：candidate 准入与 domain promotion 在**同一 durable 事务**——准入无法 durable 提交（ENOSPC/IO/fsync）则 **promotion 整体不提交**，source draft/key 保持权威（升格稍后重试），不存在"promotion 成功但载荷无主"的中间态；failpoint 测试覆盖 candidate insert/pin/promotion 各边界的 ENOSPC/fsync/crash + relaunch 后可达性；`recovery ≤ 1` 与 `pendingConflictDecision` 只是该列表的 **UI 投影**（下次获得 interaction owner 时逐个呈现抉择：swap/丢弃）；"未使用的草稿"chip（VoiceOver/保留至显式丢弃/入配额）——语义进跨端 canonical spec，Mac 实现列跟进。测试：3+ 并发冲突、kill/relaunch 列表存续、quota-full promotion × session drain × relaunch 后可达性。
- **`ComposerHostActivation` 逐相位状态机**（时序唯一权威 = §1.4a）：全局至多一个 live adapter；非持有 host 只读 snapshot（全局 wiring 只在持有者建立）；`live → finalizingInput → closing → transferred|retained`；cancel 不进 finalizing；close-drain 见 §1.4a；相位 × disposition 全矩阵测试。
- 测试：file/photo/camera 延迟完成 × pop/deep-link/promotion/gateway-switch × 双 host 全组合（双 fake gateway 记录 request body，G1 文件零命中 G2）；beginSend×pending×success/failure/cancel；session 退休后 completion 落对条目；冲突两分支+满槽 pending decision+重启存续；10 图/多文件/跨 20 route/suspend/revoke 的 RSS+磁盘清理+恢复。

- **durable feedback 状态机（r28-F3，权威正文）**：`pending(scope, EntryID, operationID, kind) → presented → acknowledged | archived`。**呈现载体 = 持久化 inline failure chip**（Entry 上的常驻 UI，不是一次性 toast）：匹配 EntryID 的 host 取得 interaction owner 时显示（B 在栈顶不冒泡 A 的失败）；**durable acknowledge 与用户动作事务绑定（r29-F3/r32-F3）**：`failedRetryable` 的重附 ack 并入成功 swap 事务；**`failedTerminal` 的重附 = 原子事务 `admitFreshOperation(O2) + acknowledgeFeedback`**（failedTerminal 事务保留**稳定 attachment slot/lineage tombstone 至 feedback 达 terminal（r33-F3：`acknowledged` 或 `archived` 均在同一事务释放 tombstone）**——O2 凭 lineage 落位原 slot，无需已删除的 oldKey）；移除的 ack 并入成功 cancel/delete 事务——动作事务失败（ENOSPC/fsync）则 feedback 保持 pending/presented（chip 不消失，失败状态仍可操作）；仅"确认"（无副作用动作）可独立 durable ack。呈现前/首帧前 crash → chip 重启依旧显示；ack 后 crash → 不再显示；scope revoked → `archived` 零 UI；**identity 事件五分类（r32-F1/r34-F3）**：
  - `AliasSourceRetired(draftID)`：升格后 source alias 退休——**只回收 alias 条目，不触碰 Entry/capability/feedback**（O 已随 targetMapping 属 T）；
  - `RouteOccurrenceSuperseded(RouteInstanceID)`：单个 occurrence 出栈/失效——**不销毁共享 key 的 Entry**（A→P→A 场景安全）；
  - `DestinationDiscarded(threadID|draftID, revision)`：领域删除（thread/draft 删除、routeInvalidated 判定不可达）——触发下述失效事务；
  - `PayloadGenerationReset(EntryID, generation)`（r33-F1/r34-F1）：**普通清空**——只清当前代文本/附件/该代 operation，保留 Entry token、历史 DeliveryRecord 与 ambiguous 出口；**准入条件 = `barrier idle ∧ producer live`**（sealed/finalizing 窗口一律拒绝，UI 在该毫秒级瞬态禁用清空动作——不存在 Reset × Reservation × Finalization 组合态，seal 后 U@G+1 的归宿只由 barrier 结算决定）；测试：sealed/finalizing 中 reset 被拒 × commit/revoke 两分支 × kill/relaunch；
  - `PayloadEntryDiscarded(EntryID, revision)`：**显式终局销毁**（用户删除整个草稿身份/scope revoke 结算）——触发失效事务。
  **capability 绑定稳定 `PayloadLifecycleToken`**（随 EntryID 铸造、promotion 保持 active 只迁映射）；仅 `DestinationDiscarded`/`PayloadEntryDiscarded` 可触发吊销。**token 三态收敛协议（r35-F1）**：`active → discarding → discarded`——discard 事件先置 `discarding`：**拒绝**普通输入（edit/close/beginSend/reset）与一切新 admission；**允许**失效事务持有的专用 **settlement capability** 原子收敛在途状态，产物统一落 **`DiscardFinalizationTombstone`（r36-F1）**：terminal 记录只含 `{token, discard revision, session, epoch, finalSequence, disposition}`——**不含文本/附件/路径**（满足销毁与 logout 的 payload 清除要求）；**settlement 事务内直接 terminal，不等待 adapter ack**；既有 `closePendingAck` 记录原子转入 tombstone（pinned 契约正常终结），迟到 ack 幂等拒绝。**tombstone 有界回收（r37-F3）**：GC 条件 = token 已 discarded ∧ 该 token 全部 descendant/evidence 已独立结算——达成后 tombstone 可回收（迟到 ack 此后按 unknown token 拒绝，语义不变）；数量/字节计入持久化预算（correlation tombstone 同池）；churn 断言：**500 次多 session discard + relaunch 后 tombstone 池稳态有界**。**settlement 三正交分量（r37-F1/F2 重构）**：
  1. **DeliveryRecord 结算（与 barrier 相位无关，r37-F1）**：对该 Entry 的**每一个** DeliveryRecord 按其自身 phase 独立 CAS——`notDispatched → cancelledByDiscard`（清 envelope）；`transportAttempted/ambiguous → evidence`；`acknowledged → terminal evidence`；transport 调度与 settlement 竞争由 record CAS 决胜（barrier 早已 idle 的历史 ambiguous 记录照常结算）。
  2. **Reservation 结算**：活跃 barrier 强制 revoked（envelope+缓冲入清理，不入 G+2；其新建 DeliveryRecord 若已存在则并入分量 1）。
  3. **Session 集合结算（逐 session reducer，r37-F2/r38-F1）**：**membership 按稳定 `PayloadLifecycleToken`/EntryID 枚举**——session 铸造时已捕获 token（§1.4），settlement 遍历**该 token 捕获过的全部 `(sessionID, epoch)`**，跨 promotion/alias 迁移不变（D→T 升格后 D 时代的 S₁ 仍在 token 名下，不会因 key 枚举而漏结）；`ComposerKey/alias` 只用于事件路由，不参与 settlement 枚举与 discarded 前置检查（前置检查同样按 token）。下表对集合内每个 session 独立执行；tombstone 唯一键 = `(token, discardRevision, session, epoch)`：

| Reservation（该 session 视角） \ Producer | live/finalizing | closePendingAck | retired |
|---|---|---|---|
| none/已结算 | 终结 finalizer→tombstone | 原子转 tombstone，迟到 ack 幂等拒 | 无事（已 terminal） |
| sealed（该 session 的活跃 barrier） | 见分量 2 后再走本行 none 列 | 同左 + 转 tombstone | 分量 2+清理 |

**token → discarded 的前置断言**：reservation 已结算 ∧ **session descendant 集合（active/finalizing/closePendingAck）为空** ∧ **全部 DeliveryRecord 已达 terminal/evidence**。（两类 discard 同规则；kill/failpoint 于任一收敛步 → 重启凭 ledger/manifest/tombstone 续收敛，终态唯一。）**token = 一切 durable payload mutation 的统一准入门（r33-F2）**：presentation result 落账、operation manifest admission、pendingReplacement 建立、swap 事务、`admitFreshOperation`、operation 状态转移、completion——**全部 CAS `token == captured ∧ active`**；token 已失效 → 拒绝 + 清理 provisional 文件，不得新建 manifest/文件 owner/operation。**失效事务**（后两类）原子完成：`pending|presented → archived` + 释放 Entry feedback 引用 + 吊销 token 下全部未 terminal capability + **identity-discard override 表结算已 terminal 但持资源的状态（r32-F2）**：`failedRetryable` 原子取消（清文件/配额/manifest）、`pendingReplacement`/未提交 ReplacementRecord abort（释放 reservedBytes+删 provisional 文件）、committed ReplacementRecord 按 newKey 结算、attempted DeliveryRecord 只留 correlation evidence——Entry/record/文件/配额最终归零。promotion 恒保持 EntryID（r31-F2）。测试：A 后台失败×B 在顶×pop 回 A；呈现请求前/首帧前/ack 前后 kill/relaunch；重附/移除动作事务 ENOSPC/fsync/kill 边界（失败后 chip 仍在）；**后台失败后 destination 删除×relaunch×churn 回收；completion × identityDiscard/routeInvalidated 两序 × 各持久化边界 kill/relaunch，断言零复活**。

### 1.6 发送协议：durable at-most-once（v21 恢复，客户端不依赖服务端幂等）

- **`SendCommitBarrier`（r21-F1 定稿）**：每 EntryID 至多一个、短生命：`idle → sealed(provisional) → durableCommitted|revoked → idle`。**`durableCommitted` 是唯一线性化点**。seal 同步铸造 **`SendReservationID`**（每次 seal 唯一）并推进 **PayloadGeneration（单调、永不复用——撤销也消耗该代际）**：捕获 envelope@G（tag=ReservationID）、预留 G+1、UI 乐观清空；窗口内新输入入 G+1 provisional 缓冲（tag=ReservationID，reducer 持而不落 durable）。**结算（成功与撤销都先持久化最终 follow-up snapshot，再回 idle/触发 inputReady）**：durable ack → G+1 转正、缓冲落库释放、`DeliveryRecord` 独立进 transport；fsync 失败 → **revoked**：`T(envelope) + U(缓冲)` 顺序合并**落入新代 G+2 草稿**（G+1 作废但不复用），撤销记录含 ReservationID。**迟到回调判别**：一切事件携带 ReservationID——第一轮 seal 的迟到回调（旧 ReservationID）与第二轮 seal（新 ReservationID + 更高代际）不可混淆，旧 reservation 事件：producer terminal 前按 12 格表（committed→G+1 缓冲、revoked→G+2 缓冲）；terminal 后按终局记录去重/审计。**附件/capability 分支表（r22-F2 修正）**：`DeliveryRecord` 在 durable commit 后 **payload 永久不可变**——post-seal 创建的 picker/upload operation（capability 绑定 (EntryID, **ReservationID**, branch)）**恒属 follow-up composer**：committed 分支 → 留在 G+1 composer 草稿（绝不并入已密封的上一条消息）；revoked 分支 → 重映射到 G+2 合并草稿。`ScopeBoundOperationContext` 与 process-death manifest 均以 **OperationCapabilityKey** 为身份（§1.5），completion 按该 key 全字段匹配，重启后凭 manifest + reservation 终局/ledger 记录重建映射。测试：seal→picker→commit/revoke/kill/relaunch 全组合，断言 **S1 永不出现 follow-up 附件**。**generation/reservation 分配器 = durable hi-lo 水位**（批量预提升落盘、内存区间分配、单 seal 无同步 fsync、跨重启永不复用、水位≠发送成功）。**provisional reservation ledger（r24-F2 规范化）**：schema = `{scopeIdentity, scopeEpoch, EntryID, ReservationID, G, G+1, terminalOutcome(committed|revoked|nil), targetMapping(EntryID+generation|nil)}`。**admission 顺序铁律（r25-F1 扩展）：ledger 写入 → 一切 durable descendant（`producerDrained` 记录、operation manifest）→ 网络动作**——**`producerDrained` 的持久化与 ledger upsert 同一原子事务**（纯输入无 operation 的路径同样先建 ledger 记录）；不存在无 ledger 的 durable descendant。**启动合成终局 = 单一 durable 事务五步**：①"已分配无终局"合成 `revoked` ②分配 G+2 ③payload 迁移/ConflictSet 归位 ④manifest 状态更新 ⑤targetMapping 落库——每个写入边界配 kill/failpoint 测试。**operation × uploadAttempted × scope 交叉恢复矩阵（内联规范）**：

| operation state × attempted | scope active | scope suspended | scope revoked |
|---|---|---|---|
| requested/preparing（picker 已消失） | cancel + 清 staging；EntryID/G 不变；upload=0；UI 无 | 同左（落 origin 分区） | cancel + 清 staging + 擦 payload |
| uploading × 未 attempted | 安全重试；文件保留；成功落 targetMapping 代际；upload=1 | 挂起（origin 分区），切回续跑 | cancel + 清 staging + 擦 payload |
| uploading × attempted | **不自动续跑**：转 failed-retryable、文件保留供重附、upload 不增、UI 可见"上传未完成" | 同左（origin 分区） | 转 scopeRevoked、清 staging、留 correlation evidence |
| **completed** | 附件按 targetMapping 归位于目标代际；**staging 删除**；upload=1；UI 正常显示附件；mapping 落库后记录可回收 | 同左（origin 分区） | payload 擦除、evidence 归档后回收 |
| **failedRetryable**（含 attempted 转入） | **文件保留**供重试/重附；EntryID/G 不变；upload 不增；UI 可见"上传未完成，可重试"；**跨任意次 relaunch 保持本行**（不衰变为 terminal） | 同左（origin 分区，切回可见） | 转 cancelled 行处理 |
| **failedTerminal**（不可恢复错误） | **单一 durable 事务（r30-F2）**：O 转 failedTerminal + 插入 `feedback.pending` + 建立 Entry feedback 引用 + 登记文件清理——物理删文件在提交后**幂等执行**（各写入边界 failpoint：不存在"附件消失且无 chip"的中间态）；upload 不增 | 反馈持久化于 origin 分区，按 feedback 状态机呈现 | 清理 + 诊断归档，**零用户 UI** |
| **cancelled** | **仅删该 O 的 stagedAssetID + operation record（r29-F1）**；同 Entry 文本与其他附件不动；UI 无；记录即可回收（整 Entry 清理只由显式 reset 或 scope revoke 定义） | 同左 | 同 active |
| **superseded** | **文件不清理**（ownership 已转 O2）；O1 记录归档即可回收；upload 不增 | 同左 | O2 一并按 revoked 结算 |

（synthetic revoked 的 G+1 → G+2 durable 映射即 targetMapping；12 格输入 reducer 管文本、本矩阵管 operation，二者正交。）禁止无终局网络重试；放弃 provisional payload = 显式 cancel + 清文件 + 可见反馈。**kill 语义拆分（r25-F1）**：`producerDrained` durable commit **前** kill → 窗口内缓冲 U 可丢（声明行为），重启回 G/T；commit **后** kill → U 已 durable，启动恢复**必**将其归入 G+2 并 close（不再宣称无条件丢失）；envelope 侧同理（outbox 未提交=无记录）。已消耗的 generation/ReservationID 空间作废不复用；**ledger 有记录者启动时已合成 revoked 终局**（按 12 格表处理），ledger 无记录的迟到旧 reservation 回调按 unknown reservation 拒绝并审计。**多发送并发（r20-F2）**：barrier 释放即可开下一次 beginSend；多个 `DeliveryRecord` 各自独立走 transport/reconcile/ambiguous（busy 续发/Queue-Steer 语义保留）——S1 ambiguous 不阻塞 S2。**durable 配额与背压（r21-F2）**：非 terminal DeliveryRecord 设 per-scope 与全局上限（**per-scope ≤ 64 条 / 全局 ≤ 256 条 / envelope bytes 计入 payload 磁盘配额**）；**seal 前预检**——超限时不 seal、草稿保留、用户可见反馈（"待确认的发送过多"）；**非 terminal 记录永不静默淘汰**（terminal 后转有界 correlation tombstone）。测试：连续离线/ambiguous 发送到限、重启、回收稳态。**乘积表**（SendCommitBarrier × InputFinalization，含**附件/picker mutation** 不只文本）：release 落窗口 → finalization 冻结 G+1 provisional 视角（envelope 不属 N final）；suspend/revoke 于窗口 → 先结算 barrier（commit 或撤销）再结算 scope；同 key `inputReady` = finalization settled ∧ barrier idle。测试：全格 + S1 ambiguous×S2 acknowledged + 连续两次 sealed + 乱序 frame + revoke + fsync 失败后迟到双代回调 + relaunch。fsync 不上 MainActor。
- transport 尝试前原子推进 `transportAttempted`；`notDispatched`（持久化证明未越传输边界）= 唯一"安全重试"。
- **ambiguous 可终结用户出口**：呈"发送状态不明"+ 两显式动作——**恢复为草稿**（撤 optimistic 行、envelope 经 `PayloadConflictSet` 回 composer 不覆盖活跃 follow-up、记录终结 `abandoned`）/ **重发副本**（明示可能重复、新 clientIntentID、原记录 `supersededByDuplicate`）；`DeliveryEvidence × UserDisposition` 正交建模，迟到 server frame 只推进 evidence（自动收口为 acknowledged）；终结记录清 payload 只留有界 correlation tombstone。
- **kill/relaunch 矩阵**：outbox commit 前后/attempted 后 transport 前/transport 后 response 前/ack 后逐点杀进程重启，终态 ∈ {acknowledged, 安全重试, 用户可终结}，永不悬挂；crash-before-transport dispatch=0 且出口可用。
- **新建会话多段写（r19-F2 如实建模）**：create thread → optional bind → chat start 每段持久化 CAS 状态。**消息级认领成立**（chat-start 的 `origin_id` 链路实测存在）；**create 级认领当前 gateway 不可证**（`/api/threads` 不回 metadata、无 (scope,createIntentID) 唯一索引）——**create-response 丢失如实建模为 ambiguous**（并入 §1.6 出口：恢复为草稿 / 明示可能产生重复 thread 的重建），**不承诺"无幽灵/重复 thread"**；该保证的启用条件 = P0-G 的 `(scope, createIntentID)` 唯一索引 + 查询接口（已列 P0-G 前置项）。
- **跨端契约**：`DurableDeliveryState` 进 canonical conversation-state spec（`conversation-state.md`/`states.json`/双端 fixtures），iOS 先行实现，**Mac 对齐列跟进任务**。
- **gateway 幂等账本（纯服务端，独立任务）**落地后：ambiguous 升级自动安全重试；原子 create+dispatch 命令落地后：多段写折叠。客户端架构今日即按上述干净语义建成，不等待。

## 2. 导航意图与事务

### 2.1 NavigationIntent：准入时序 + 优先级格 + 依赖声明（r5-F3/F4/F8）

1. `prepare(intent)` 只读整备；产出 **typed `PrepareOutcome`**：`ready` / `userVisibleNotFound`（保留现 route-not-found UI，`25478f6ee:GaryxMobileModel+Navigation.swift:229`）/ `retryableFailure`（保留现产品反馈与重试语义）/ **`authenticationRequired`（r7-F5：走认证/连接反馈，不得伪装成 not-found）** / `cancelledOrStale`（静默）/ `internalFault`（log fault）——产品可见错误反馈语义（not-found/重试 UI 沿用现有呈现）。

**1a. GatewayScope 生命周期（r8-F6 闭合）**：`active ⇄ suspended → revoked`（切 gateway = 旧 scope suspended、可切回恢复；logout = revoked 不可逆）。**全局单 active 不变量**，scope 切换走 CAS。**logout 原子动作**：revoke 当前 epoch、清理或隔离该 scope 的领域数据与 lease（composer/payload/outbox 按 §1.5-1.6 scope 化清理）。**有界拦截（r8-F6）**：不留逐 epoch tombstone——每个 gateway identity 持久化单调 **`revokedThroughEpoch`** 水位，迟到事件 epoch ≤ 水位一律拒绝；**未知/已删除 epoch 默认按 revoked 处理**。重新认证产生新 scope identity（epoch > 水位）。认证屏障期间普通 intent → `PrepareOutcome.authenticationRequired`。测试：logout × 迟到导航/升格事件双顺序；重登同/异身份零复活；**500 次 switch/logout churn 后水位单调且拦截任意旧 epoch 回调**。
2. 非 terminal 事务期间只排队，绝不 admit。
3. **优先级格 + 强制效果合并（r6-F3）**：优先级 `安全强制类（logout、routeInvalidation） > gatewayScopeChange > ordinaryNavigation`；低类不能 supersede 高类，高类到达丢弃低类排队项。**同类合并策略按 coalescing key 分治，不是无差别 last-wins**：
   - `logout` 与 `routeInvalidation` 是**不同 coalescing key 的幂等安全效果**，互不丢弃、幂等 merge；`routeInvalidation ⊔ logout` 结果由 logout 主导，同时保留 invalidation 要求的 fallback 清理；两者双顺序结果一致（交换律测试钉死）；
   - `gatewayScopeChange` 同 key 才 last-wins；
   - `ordinaryNavigation` 同类 last-wins（不变）。
   **认证屏障**：logout 获准后建立 authentication barrier——普通 intent 一律拒绝并走 `PrepareOutcome.authenticationRequired`（认证/连接反馈，非 not-found），到重新认证（新 scope identity，§2.1a）后才可解析；不存在"logout 后自动在新 scope reprepare"。
4. **依赖声明**：prepared intent 二选一——
   - `absolute`：与旧栈无关（deep link 到指定 thread 等），admit 时安全 rebase 到当前栈（CAS 只验 gateway scope + 同类 epoch）；
   - `relative`：携带 base `RouteInstanceID + payloadRevision + stackRevision`（如"在当前页上推 detail"），基础变化 → 按 intent 声明 reprepare 或丢弃。
5. 抢占（仅高优先级类）：先按相位收敛旧事务 terminal，再原子 admit。

测试：A/B 完成两序；resolver 无视 cancellation；modal × forced × 后到普通 deep link（forced 必须仍执行）；连续两次 gateway switch；relative intent 在 pop finish/cancel 后的**最终 canonical path 写死断言**；PrepareOutcome 六型行为。

### 2.2 四相位 × 五维 owner（内联定稿表，r11-V12-4）

Owner 维度：**canonical**（path 真相）、**data**（live store lease/网络）、**page interaction**（页面按钮/modal/页面级导航）、**focus & a11y continuity**（键盘与辅助焦点连续性）、**transition control**（容器手势）。

| 相位 | canonical | data | page interaction | focus/a11y continuity | transition control |
|---|---|---|---|---|---|
| （无事务，active） | top | top | top | top | edge-pan eligible |
| pre-commit | source | source | **冻结** | source（键盘不动、无播报） | tracking |
| cancel-settle | source | source | **冻结** | source | **仅 coordinator edge regrab** |
| commit-settle | **destination**（释放刻原子写） | **destination**（可预载） | **冻结** | source 残留冻结（键盘已按 §4 在 commit 边界清除） | 无（新手势不 eligible） |
| terminal | destination | destination | 按 §2.3 disposition | 按 §2.3 disposition | 新事务可开始 |

生命周期严格序 `mounted → appeared → active ⇄ inactive → disappeared`。fake-host 断言：两 settle 相位点击/弹 modal 无效、cancel-settle regrab 有效。

### 2.2a 相位 × 能力矩阵（内联定稿表）

| 能力 | active(terminal) | pre-commit / cancel-settle 的 source | commit-settle 的 source | commit-settle 的 destination | staged/inactive（predecessor 等） |
|---|---|---|---|
| 缓存读/快照渲染 | ✓ | ✓ | ✓ | ✓ | ✓（骨架，不发网络） |
| 网络/流/轮询（`.task(id: phase)`） | ✓ | ✓（data owner 未失） | ✗（data owner 已是 destination） | ✓（预载） | ✗（必须 cancel） |
| modal/sheet/fileImporter | ✓ | ✗（page interaction 冻结） | ✗ | ✗（推迟 terminal） | ✗ |
| focus/键盘 | ✓ | ✓（focus continuity 归 source） | ✗（commit 边界已清，§4） | ✗（§4） | ✗ |
| hitTest / a11y 可见 | ✓ | ✗（过渡中冻结） | ✗ | ✗ | ✗（`accessibilityHidden` + hitTest off） |
| 全局 mutation | 经 intent/store 通道 | 仅输入协议通道（§1.4） | 仅 closing drain（§1.5） | ✗ | ✗（`onDisappear` 禁写路由） |
| **live composer adapter（§1.5）** | ✓（持有者） | ✓ live | finalizingInput/closing（producerDrained 记录→双边 terminal 事务→close→drain 至 ack，§1.4a） | ✗ | ✗（只读 snapshot） |

**activation × outcome×visibility 结果表（r13-F2）**：

前置条件（r14-F3）：source 非 composer → 无 close/closing（no-op）；destination 非 composer → 无 adapter 授予；**同 key**（source/destination 同 composer key）→ destination 以 epoch N+1（含 initial snapshot）续作；**异 key** → destination 用**自己 key 的 session epoch** 开新会话，source 的 N+1 预留仅留给该 key 下次 activation。

| terminal 组合 | live adapter 归属（在上述条件内） |
|---|---|
| committed + visible | destination（同 key=epoch N+1 续作；异 key=自身 key epoch） |
| committed + inactive | 挂起；scene active 且未 supersede 时 destination 建 session |
| committed + superseded | closing 照常 drain；adapter 归下一事务裁定 |
| cancelled + visible | source 回 live（cancel 不曾 close，无需重建） |
| cancelled + inactive | source 保持非活；scene active 恢复 live |
| cancelled + superseded | 归下一事务裁定 |

### 2.3 terminal：结果 × 可见性正交（r5-F5）

```
TerminalOutcome = committed | cancelled
Visibility      = visible | superseded | inactive
```

| 组合 | focus / screenChanged / modal 资格 |
|---|---|
| committed + visible | `screenChanged(argument:)` 于页面 terminal exactly-once；**composer 获焦另设条件**（r18-F2）：`inputReady(N final 已固化) ∧ visible ∧ active` 达成刻执行一次，期间被 supersede 则永久取消 |
| committed + inactive | **挂起，恢复 active 且未被 supersede 时补执行一次** |
| committed + superseded | 零动作，原子 handoff |
| cancelled + visible | 焦点/播报零动作（从未离开 source）；**恢复 source page interaction** |
| cancelled + superseded | 零动作；**不恢复**，原子 handoff 到下一事务 |
| cancelled + inactive | 零动作；page interaction **保持冻结**，scene active 且未被 supersede 时才恢复 |

（r6-F5：page interaction 同样由 visibility gate。）测试：六个组合全部进 owner 断言（focus/播报/interaction 三面），含 pre-commit 后台取消恢复零播报、commit-settle 后台提交恢复补一次。

### 2.4 事件 × 相位决定表（内联定稿表）

| 事件 | pre-commit | cancel-settle | commit-settle |
|---|---|---|---|
| recognizer cancelled（系统抢占） | → cancel-settle | 继续 | 继续 |
| sceneInactive | cancel 终态（cancelled, inactive） | 同左 | commit 终态（committed, inactive） |
| rotation / 非键盘 geometry | **不取消**：按进度重推导 wrapper frame 继续 | 同左 | 同左 |
| 键盘几何 | 忽略（§4 过滤） | 忽略 | 忽略 |
| routeInvalidated / gateway 强制 | cancel 终态（cancelled, superseded） | 同左 | commit 终态（committed, superseded） |

### 2.5 PresentationLease（r5-F6，泛化 modal barrier）

- **凡 route host 发起的系统呈现**（`sheet`/`fullScreenCover`/`fileImporter`/PhotosPicker/share/preview controller/adaptive popover——按"呈现事实"而非 modifier 白名单）持有 `PresentationLease`。
- **token 树集合（r6-F4）**：lease 是 tokenized set 带父子关系（真实嵌套存在：Agents fullScreenCover 内再开 Avatar sheet）——**集合非空即 barrier**；内层 dismiss 只释放自己的 token；dismiss 父呈现时 **exactly-once 释放整棵 descendant 子树**。
- **获取时刻（r6-F4）**：lease 在 presentation request 被接受、**设置 `isPresented` binding 之前同步获取**（消灭"binding 已 true、UIKit 未建立、deep link 先 admit 卸载 presenter"的同帧竞态）；token 状态机 `requested → presented → dismissing → released`，**presentation 失败也终结 token**（不泄漏 barrier）。
- barrier 语义：普通根导航 intent 排队（同类 last-wins）；不换 path、不卸 presenter host。
- **lease 释放 = exactly-once coordinator event**，归一程序化 dismiss（`dismiss(animated:completion:)` completion / SwiftUI `onDismiss`）与交互式 `presentationControllerDidDismiss`（互斥触达，coordinator 去重）。
- **result-bearing token 的 join 态（r14-F4）**：fileImporter/PhotosPicker/camera 等带结果呈现，token 状态机扩展 `dismissedAwaitingResult / resultRecorded`——**dismissal 与 result disposition 两者都到齐才 release**（PhotosPicker 的 selection 变化与 dismiss 任意先后皆覆盖）；用户取消、presentation failure、父级强制 dismiss 产生**显式无-result terminal**（同刻终结关联 OperationCapability，§1.5）并 release。
- 强制类 intent：请求程序化 dismiss → 等整棵 lease 释放 → 原子切根；hard-snap 必须等无动画 dismiss completion。
- 测试：每类 presentation 获取/释放；嵌套 dismiss（内层释放后 barrier 仍在）；父级强制 dismiss 整树 exactly-once；presentation-start × deep-link 同帧竞态；presentation 失败终结 token；程序化/交互式两路径；hard-snap 等待。

## 3. 容器渲染器

### 3.1 path diff 决策表（内联定稿表）

| diff | 行为 |
|---|---|
| 尾部 +1 | push 过渡（来源 intent 决定动画/即时） |
| 尾部 -1 | pop（四相位事务，交互/非交互） |
| 尾部 -N（N>1） | 单次过渡到目标层（中间层不闪现），生命周期按最终态结算 |
| replace top（不同 instance） | crossfade/即时，无横移 |
| 同 instance destination 升格（promoteDraft） | 无过渡、无生命周期扰动 |
| 同 composer key 再次打开 | 新 occurrence 正常 push（§1.1）；activation 绑到新栈顶 occurrence |
| reset `[]` | pop-to-home 过渡 |
| 整链替换（deep link） | 无过渡直达 + 生命周期重结算 |
| 非法 diff（中段修改等） | 归一整链替换 + log fault |

容器只拥根内容层；modal/form/tool-trace 原生 `NavigationStack` 不动；`routeDismiss`/`routePush` 经 Environment 注入；state restoration 非目标。

### 3.2 内存与状态店（r5-F9 补计量）

- mounted host ≤ 4；白名单字段；pinned = 保留集免淘汰；永久出栈即清理；**容量约束只作用于 evictable entries**：32 entries / 2MB，**2MB 按每字段类型定义的 `estimatedCostBytes`（编码后字节数）计量**；LRU 淘汰；淘汰降级 = 白名单回默认；**pinned 总成本超预算 → 只记 budget fault，绝不违反免淘汰契约**。
- 性能门冻结值（A4a）：host ≤ 4；20 层 RSS 增量 ≤ 100MB；过渡期子树 body 重算 = 0/帧 @120Hz；hitch 不回退；weak-deinit 全收；500 次 churn 后 **`evictableEntryCount` ≤ 32 且 `evictableCost` ≤ 2MB**（r6-F6：约束只作用 evictable；total/pinned 另报指标）且 RSS 稳态。retired-session tombstone（§1.4）计入 evictable 计量。

## 4. 焦点、键盘与迟到回调（v7 改用输入代际）

- route focus coordinator + focus token（不变）；pre-commit/cancel 不动键盘；destination 只在 terminal(committed+visible) 获焦；键盘几何过滤。
- **commit 权威步骤 = §1.4a release 刻**：冻结输入 + 同步 unmark/drain + 清 FocusState/`resignFirstResponder` + 原子写 path **同一 MainActor 临界区完成**；close 于**双边 terminal 事务**发出（producer drained ∧ reservation 结算，§1.4a——异步 drain 由 `InputFinalizationLease` 持有，脱离 first responder）；ack 由 capability 独立保证。**不存在"先 close 再写 path"的旧时序**。
- **迟到回调**：携带 §1.4 完整**六元组**，经 alias 重定向，按乘积 reducer 处理；不查询栈顶 selection。
- 测试：键盘三元组断言；CJK finish pop；升格 × pop 并发迟到回调（文本不丢）；键盘展开 finish/cancel/regrab。

## 5. 手势仲裁（内联定稿表）

**机制**：容器持公开 UIKit leading/trailing edge-pan recognizer；**edge zone 判定用 `gestureRecognizer(_:shouldReceive:)` 捕获的 touch-down 物理坐标快照**（`shouldBegin` 用快照，非 begin 时刻 location）；`shouldBeRequiredToFailBy` 令 descendant scroll/pan 等待 edge-pan 失败。Core 只做逻辑判定（logical leading/trailing + progress），adapter 按 `layoutDirection` 换物理坐标与方向符号。

| 竞争面 | start zone | winner | adapter 机制 |
|---|---|---|---|
| 返回 pop（push 页） | logical leading edge ≤ 20pt（touch-down 快照） | pop | 容器 leading edge-pan；descendant 被 require-to-fail |
| 抽屉（home node） | logical leading edge | drawer | 同一 leading edge-pan，按 node 语义分发 |
| task tree 开/关 | logical trailing edge | task tree | trailing edge-pan，同机制 |
| markdown/tool/attachment 横向 ScrollView | 内容区（非 edge zone） | scroll | edge-pan shouldBegin=false，互不干扰 |
| 横滚起点落在 edge zone | edge zone | 导航手势 | scroll 等待 edge-pan；begin → scroll cancel |
| composer 聚焦收键盘 DragGesture | 非 edge zone | 收键盘 | 保持 SwiftUI；edge 起点归 edge-pan |
| row swipe（行内水平） | 行内非 edge | row swipe | 非 edge 不冲突；edge 起点归导航 |
| modal 呈现中 | — | modal | 容器 edge-pan isEnabled=false |
| task tree 已打开 | — | task tree 关闭 | pop 不 eligible（现业务规则） |
| 纵向滚动 | 任意 | 滚动 | 轴锁（现 decideAxis 语义入 Core） |

真宿主测试：LTR/RTL 实坐标（RTL 下位移/视差/阴影反号）；5pt 起点移到 25pt（归导航）；非 edge 起点反向移入 edge zone（归内容）。

## 6. 过渡视觉与 commit 判定

- 空间过渡（默认）：outgoing 1:1；incoming 逻辑视差 -30% + scrim + 阴影（对照 spike 轨迹）；commit = 投影落点过半；不变式：快甩短距必 commit、慢拖中距必 cancel。
- **VisualPolicy 冻结（r5-F7）**：事务 begin 时从已合 main 的 `GaryxAccessibilityTransitionPolicy`（`03d999a38` 前已入，`Sources/GaryxMobileCore/GaryxAccessibilityTransitionPolicy.swift`）+ reduceMotion 冻结 `.spatial / .crossFade / .immediate`：crossFade/immediate **禁止 wrapper 位移、视差与移动阴影**（透明度/瞬切呈现），物理 commit 判定照常；事务中偏好变化只影响下一事务。
- 测试：两偏好组合 × 事务中切换（当前事务不变、下一事务生效）。

## 7. 测试计划（五层，v21）

1. **Core**：§2.1 优先级格/依赖声明/PrepareOutcome 六型；升格结算全组合（store/path/lease/optimistic/outbox/dispatch 次数写死）；输入协议 §1.4-1.4a 全部用例（**六元组/乘积 reducer 全分支**（Reservation×Finalization 逐格）/两序延迟物化/终结状态机/close 屏障/epoch handoff 全组合/alias 生命周期与不变量/**reservation 水位跨重启永不复用/双边 terminal 顺序两序/post-seal picker 归属/旧 reservation 回调拒绝**）；ComposerPayloadStore/OperationCapability/ConflictSet/ScopeBoundOperationContext 状态机全用例；scope 生命周期与水位 + scope 化 composer 结算；**identity 五事件矩阵（r34-F3 实文）**：AliasSourceRetired 不误杀已迁 operation / RouteOccurrenceSuperseded 不动共享 Entry / DestinationDiscarded、PayloadEntryDiscarded × **每类 admission（result/manifest/pendingReplacement/swap/fresh/transition/completion）与 §1.4 输入事件（edit/close/beginSend/reset/**drain/producerDrained/双边 terminal 事务**）** × barrier phase 双序零复活零新建、discarding 收敛语义（纯逻辑分支） / PayloadGenerationReset 准入（sealed/finalizing 拒绝）与出口保留；发送协议状态机（DeliveryEvidence×UserDisposition）；terminal 结果×可见性；事件×相位；path diff 全行；仲裁 LTR/RTL；物理不变式。
2. **Instrumented fake-host**：settle 冻结/regrab；screenChanged=1 于页面 terminal；composer 获焦于 inputReady∧visible∧active 一次；cancelled 后台恢复零播报；superseded 零副作用；PresentationLease 各类型 × 两 dismiss 路径 × result join 态 × 同帧竞态 × hard-snap；staged host 零 lifecycle 写入；activation × outcome×visibility 全矩阵；LRU 降级；churn 稳态；VisualPolicy 三型。
3. **XCUITest**：finish/cancel/regrab/深栈/首层；deep link 排队；键盘三元组 + CJK finish（真 UIKit host marked range 组合态）+ 听写 pending 两分支；升格并发输入；同 key occurrence（A→P→A、A→A、深链）；RTL 实坐标；边缘两用例；横滚共存；`.payloadPreparing` 锁、ambiguous 出口、冲突 chip、配额失败 + 各面 VoiceOver。
4. **性能/内存**：§3.2 全门 + alias 不变量（drain 后 =0、bytes ≤64KB）+ finalizer 入预算 + payload 磁盘配额清理。
5. **持久化 harness（真实 kill→relaunch）**：§1.6 发送矩阵逐边界；多段 create 每段"服务端提交后客户端确认前"死亡；**sealed 窗口 operation manifest × reservation outcome 恢复矩阵**；**全 operation state × destination 删除 × kill/relaunch 资源归零矩阵**；**discarding 收敛全边界真实 kill（r36-F3/r37）**：进入 discarding/强制 revoke/producerDrained 合成/closePendingAck 转 tombstone/**逐 DeliveryRecord CAS**/资源清理/token discarded 每个持久化边界杀进程重启续收敛（含无 operation 路径、**barrier idle+多 record 混合三相位+transport 竞争**、**S₁ pendingAck+S₂ live 与多 pending ack 乱序 ack×crash**、**D:S₁ closePendingAck→promote D→T→T:S₂ live/finalizing→discard T 或 scope revoke（逐 session tombstone 写入间 crash，断言两 session 均结算、token discarded、alias 回收、零 pinned finalizer/host）**、500 次多 session discard churn）；operation manifest 恢复/取消；ambiguous 出口自身 crash 原子性；出口 × 迟到 frame 竞争；durable 三合一 fsync 断言；recovery/pendingConflictDecision 存续；scope 化 composer 重启恢复。

## 8. 交付切分（v21）

A1/A2 **已合 main**；**A3** = 路由模型 + intent 准入 + 事务 coordinator + **Core 层输入/Payload/Operation/发送 reducer 与协议 + `ComposerDurabilityStore` 接口与 fake store**（concrete 存储不在此批）；**A4a** = 容器 renderer + fake routes（VisualPolicy 接线）+ 性能门；**A4d-1（先于 A4b，r20-F7 唯一归属）** = concrete durability 层一次成型：DB/schema/payload store/outbox/`commitSend(...)` 三合一事务 + 故障注入（ENOSPC/fsync 失败/provisional 撤销/send×release×revoke）；**A4b（依赖 A4d-1）** = conversation/live-store/focus 迁移 + composer 输入子系统（有序 UIKit adapter/激活缝/finalizing）+ PayloadStore 接线与磁盘暂存；**A4c** = 其余 route + PresentationLease（含 result join）+ presentation→operation context 桥接 + a11y 矩阵；**A4d-2** = transport 与出口 UX（ambiguous 双出口/多段 create ambiguous 建模/scope 化结算）+ 持久化 harness 全矩阵；**A5** = 抽屉/task tree/row swipe + 仲裁 + 全回归。

**范围边界（v23）**：**P0-G 收缩为纯 gateway 侧**——服务端幂等账本、原子 create+dispatch 命令、`prompt-attachments` TTL/删除所有权、**`(scope, createIntentID) → threadID` 唯一索引 + 查询接口（r20-F5：恢复"无幽灵/重复 thread"保证的启用条件）**；落地后客户端按 §1.6 声明升级（ambiguous 自动重试、多段写折叠）。**Mac 对齐跟进任务**：DurableDeliveryState canonical spec 双端 fixtures 本批先行定义（iOS 实现），Mac 消费为独立任务。

## 9. 开放问题

过渡曲线参数实现期校准；深栈 -N 单次过渡视觉实现期定。
