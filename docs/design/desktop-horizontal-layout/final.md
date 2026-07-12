# Garyx 桌面横向布局状态机 + 面板扩窗 — 综合设计裁决(final v4)

日期:2026-07-12(v4:第三轮复审唯一残留——desiredOccupancy checkpoint 时机——修订)
综合自:A 版 [`design-a-codex.md`](./design-a-codex.md)、B 版 [`design-b-claude.md`](./design-b-claude.md)(rev2)。
评审记录:[`review.md`](./review.md)(首轮 15 FAIL → v2 闭合 8 → v3 闭合残留 3 组)。
本文件是**裁决层**:实现以本文为准,细节查对应原文;与两版原文冲突处一律以本文为准。

## 0. 已拍板的产品行为(用户)

1. 面板显式开合 = 窗口宽度扩展/缩回,内容整体平移,不挤压主消息区;约束下(贴边/最大化/全屏/空间不足)降级为窗口内 reflow,主消息区硬保护 350px。**例外**:compact 视口下的 sidebar 临时展开是窗口内 overlay(#27),不适用扩窗规则。
2. 整个横向布局是 **JS 纯函数状态机的计算题**:布局决策 100% 在状态机,CSS 只消费输出(px 变量 + presentation 属性),不用 flex 挤压/media query/DOM 测量承载 responsive 决策。

## 1. 核心协议(v3)

### 1.1 Bounds authority:铸造权与还账权分离(v3 修订)

动窗权限分两种,合称 `BoundsAuthority`:

```
BoundsAuthority =
  | UserCauseToken                                    // 可扩窗、可缩窗
  | RepayProof { fundingIds, expectedSessionRevision } // 只能缩窗(减少已确认 funding),绝不能扩窗
```

- 只有 **user-panel** 与 **user-route** 两类 cause 可以铸造 `UserCauseToken`(一次用户动作 = 一个 token = 一个 transaction)。
- **每条 `ConfirmedFunding` 自带长期 `repayAuthority`**,存活至该 funding 被偿还——不依附短期 in-flight transaction。`RepayProof` 即引用它的减量凭证。
- **system-cleanup(Escape、route/thread 清理、远端数据失效)偿还 funding 用 `RepayProof`**:合法缩窗、无需近期用户 token。这解决「token 已 settled 消亡后 Escape 关面板无 authority 还账」。
- `WINDOW_BOUNDS_APPLIED/REJECTED`、`WINDOW_MODE_CHANGED`、`FRAME_COMMITTED`、`*_DEADLINE_EXPIRED` 等非用户事件可**延续或终止**既有 authority(retry/repay/reconcile),不能铸造 `UserCauseToken`。
- 纯 responsive 事件(viewport/断点/auto-hide)不能创建任何 authority——「responsive 不动窗」由铸造权收口。
- 状态三层:`desiredIntent` / `confirmedFunding`(含各自 repayAuthority)/ `inFlightTransactions`(含 deadline、supersession 链、deferred-funding 态)。
- logical close 规则:responsive hidden/collapsed **不改 intent、不还账**;intent 真正置 closed(无论按钮还是 cleanup)时必须偿还既有 funding——user 动作用其 token,cleanup 用 RepayProof。

### 1.2 归一化 intent 入口(v3:responsive 移出 intent 事件)

- 单一事件 `LAYOUT_INTENT_CHANGED { previousOccupancy, nextOccupancy, cause, transactionId }`,一次携带四面板完整 desired occupancy(全向量)。
- `cause ∈ { user-panel, user-route, system-cleanup, hydrate }`;仅前两类铸 token;system-cleanup 走 RepayProof;hydrate 无 authority。
- **responsive-presentation 不属于 intent 事件**(它按 §3-4 不改 requested intent):断点收起、auto-hide、overlay 降级由 environment 事件(`WINDOW_SNAPSHOT_CHANGED`/`VIEWPORT_RESIZED*`)驱动 projection 直接得出,走独立通道,不进 `LAYOUT_INTENT_CHANGED`。
- 现有全部入口的映射(对照 AppShell 现实现):
  - sidebar 普通 toggle → user-panel;
  - **compact 临时展开 → 裁定为 in-window temporary presentation(compact-overlay),不占列、不 funding、无窗口 effect**(v1 未裁,现裁定);
  - L2 rail open/close/switch(recent/bot/workspace)→ user-route 全向量事件(switch 是 replace,不是 close+open);funded rail 因 route cleanup 消失 → system-cleanup repay;
  - side tools 占用 = `inspectorOpen || openCapsuleTabs>0` 的 **union 0↔1 边沿**才是 panel transaction;单个 capsule tab 增减不是;
  - workspace 文件预览自动拉开 side tools → 用户动作的后续,携带 user-route cause;
  - thread logs toggle 同时关 side tools/capsules → **full-vector replace transaction**,净向量一次 effect(330→370 = +40),不依赖 React batching;
  - Escape/route/no-thread cleanup → system-cleanup。

### 1.3 CAS 收敛、command 全量化与 ack 折叠(v3 修订)

**Command 契约(v3:funding 归属进入原子契约)**:

```
WindowBoundsCommand {
  authority                    // UserCauseToken | RepayProof
  expectedWindowRevision       // 物理 bounds CAS
  expectedSessionRevision      // funding map CAS(双 revision 同时参与)
  targetBounds                 // 绝对值
  targetNormalBaseBounds
  targetFundingByPanel         // 完整目标 funding map(replace 的 +40 由此归因)
  targetDesiredOccupancy       // v4:与 bounds/funding 同一目标快照
  transactionId / epoch / sequence
}

WindowLayoutSessionCommand {   // v4 新增:不动窗的 session checkpoint
  expectedSessionRevision
  desiredOccupancy
  epoch / sequence
}
```

main 在**同一串行操作**内完成:`validate → setBounds → read actual bounds → commit acknowledged session`;result 返回**完整 acknowledged session**(normalBaseBounds + fundingByPanel + desiredOccupancy + windowRevision + sessionRevision)。bounds 已改而 funding 未提交的中间态不存在。

**Desired occupancy checkpoint(v4)**:每次 `LAYOUT_INTENT_CHANGED` 在 **transaction start** 立即提交 `WindowLayoutSessionCommand`(CHECKPOINT_DESIRED_OCCUPANCY)——包括 constrained、funding=0、system-cleanup 这些不会发 bounds command 的 transaction。**Ordering**:checkpoint 成为 acknowledged session 之后,transaction 才进入动画、deferred 或 bounds 阶段;checkpoint stale → 用返回 session 重算后 retry;epoch takeover 恢复最后 acknowledged checkpoint。由此 session 的 desiredOccupancy 永远不落后于用户意图,reload 撞上任何中间态都能正确恢复或偿还。

- **所有实际改变过物理 bounds 的 accepted result 必须按 revision 单调折叠入账,不得因 sequence 旧而丢弃**;sequence 只决定最新 desired intent。
- **Rejection 按 reason 分流**(不得统一降级为 constrained):
  - `stale` — 用返回的权威 snapshot 重算;若仍 expandable,以原 token retry;
  - `fixed-mode` — transaction 转入 **deferred-funding**(§1.4),不结束;
  - `outside-work-area` — settle 为 constrained,funding=0;
  - `superseded` — 折叠物理事实,由 supersession 链的**当前 head token** 发 reconcile 收敛;
  - `invalid` — 协议错误,走诊断/断言路径,不与空间不足等价。
- **Supersession 链语义**:第二次用户动作铸造 token2 并 supersede token1;token1 的 accepted result 照常按 revision 折叠,但**只有链头 token2 有权发后续 reconcile**;被取代的 token1 不再发起 command。
- main 队列:尚未执行且已被更高 sequence 覆盖的旧请求可丢弃(回 superseded);一旦 `setBounds` 已发生,必须如实回报。

### 1.4 打开/关闭 transaction 状态表(全分支归宿)

打开(v3:fixed 与 constrained 分离,fixed 走 deferred-funding):

| 初始判定 | main/时间结果 | 最终状态 |
|---|---|---|
| constrained(贴边/余量不足,mode=normal) | 不发 IPC | 立即输出 constrained accepted frame;funding=0,transaction settle |
| **fixed(maximized/fullscreen)** | 不发 IPC | 窗口内 reflow 开面板;**transaction 转 deferred-funding:token 存活挂起**;退出 fixed 时若 normal workArea 可容纳,以该 token 补 funding 扩窗;不可容纳则 settle 为 constrained(funding=0) |
| expandable | deadline 前 accepted | 按实际 acknowledged session 确认 funding,一次 commit 新 columns + surface 动画 |
| expandable | rejected(按 §1.3 reason 分流) | stale→原 token retry;fixed-mode→转 deferred-funding;outside-work-area→constrained settle;superseded→链头收敛;invalid→诊断 |
| expandable | 100ms 未回 | 显示 constrained fallback;**transaction 保持 pending,不取消不失败** |
| watchdog 后 | late accepted | 折叠 acknowledged session;intent 仍 open → 切 funded frame;已 supersede → 链头 token reconcile 到最新 target |
| watchdog 后 | late rejected | 按 reason 分流后 settle 或转 deferred |
| 任意 pending | 快速反向 toggle / 右栏 replace | token2 铸造并 supersede token1;旧 accepted 仍按 revision 折叠,仅链头 token2 发反向 reconcile |
| 任意 pending | maximize/fullscreen/display/workArea 变化 | 旧 expected revision 失效;按权威 snapshot 走 deferred-funding/constrained 或 deferred reconcile |

关闭(对称):funding=0 → 只删 track 不发 shrink;有 funding → 退出动画完(`PRESENTATION_ANIMATION_FINISHED`,420ms watchdog,均 token 化)→ `FRAME_COMMITTED` → 发 shrink;期间 reopen/replace 使旧 timer/animationend 对该 panel **no-op**;fixed mode 中只记 deferred reconcile,退出后以原 token continuation 缩回。无退出动画的 panel 走即时 `FRAME_COMMITTED` 分支。

- deadline 由显式事件表达:`OPEN_DEADLINE_EXPIRED / CLOSE_DEADLINE_EXPIRED / FRAME_COMMITTED (transactionId)`;timer 只 dispatch,**reducer 不读时间**。
- 100ms 语义 = 「开始显示 constrained fallback 的视觉 deadline」,不是取消。

### 1.5 Hydrate、fresh session 与 reload(v3 修订)

- main 的 per-window executor 持有**不含布局政策的 opaque acknowledged session**:`normalBaseBounds + confirmedFundingByPanel(含 repayAuthority) + desiredOccupancy + windowRevision + sessionRevision`,跨 renderer epoch 存活,至 BrowserWindow 销毁。**`desiredOccupancy` 在每个 transaction start 即 checkpoint(§1.3 v4),不等 settle**;main 不解释其语义(opaque)。关闭动画中 reload → session 已是 closed,HYDRATE 按 orphanedFunding 规则立即 RepayProof 偿还,关闭不复活;贴边 funding=0 的 open 也已 checkpoint,reload 后 intent 正确恢复。
- **冷启动(fresh session)**:新增 `CLAIM_INITIAL_LAYOUT` command——仅 fresh session 可用、不改 bounds;renderer 按写死顺序 sidebar → rail → sideTools → threadLogs 提交初始可见 track 的认领**与初始 desiredOccupancy(参与双 revision CAS)**,main 原子建立 `fundingByPanel + desiredOccupancy` 与 `normalBaseBounds = 当前宽 − Σ认领`。由此启动即展开的 sidebar(245)是 funded:首次显式关闭会正确缩窗(否则违反 §0)。
- **reload(非 fresh)**:HYDRATE 认领既有 session,并用 session 内 `desiredOccupancy` 副本恢复 intent(而非从可见 track 猜)。**orphanedFunding 规则**:session 中 funding 存在但 desiredOccupancy 显示该面板已 closed(如 reload 撞上 in-flight close 丢失)→ hydrate 完成后立即以该 funding 的 `RepayProof` 偿还,完成未竟的 close;绝不复用 orphaned funding 冒充新扩窗。
- maximized/fullscreen 下 hydrate:用 snapshot 的 `normalBounds` 初始化 normal 台账;当前 content bounds 只用于 fixed-mode projection。
- `rendererEpoch` 注册/接管协议:新 epoch 接管后,旧 epoch 已排队请求一律回 superseded,不得再改窗。
- hydrate 本身永不扩窗(cause=hydrate 不铸 token;唯一允许的 bounds 动作是上述 orphaned repay,持 RepayProof)。

## 2. 裁决表(v2)

| # | 决策点 | 裁决 |
|---|--------|------|
| 1 | 状态机 API | reduce/project 两层(B)+ A 的 plan 字段;补 §1 的 transaction/command 生命周期 |
| 2 | responsive 不动窗保证 | **v3:BoundsAuthority 分权(§1.1)**——UserCauseToken(可扩可缩,仅用户动作铸造)与 RepayProof(只减不增,system-cleanup 还账用),替代事件分支封锁 |
| 3 | 断点基准 | A `responsiveBasisWidth`:仅 user/display origin 的**权威 resize snapshot** 更新;origin 来自 main 的 `will-resize/will-move` session 判定;测 duplicate/乱序 snapshot |
| 4 | 扩窗台账 | A funding vector,按 §1.1 拆三层(desiredIntent / confirmedFunding / inFlight) |
| 5 | IPC 竞态 | **v2 改:§1.3 CAS 收敛 + 物理 ack 按 revision 折叠** |
| 6 | 首次 hydrate | **v2 改:§1.5 main 侧 acknowledged session 认领制** |
| 7 | 最大化/全屏 | 不动 bounds 纯 reflow;**fixed 中 open 转 deferred-funding(token 挂起存活,退出后可容纳则补扩窗)**;fixed 中 close funded panel 记 deferred reconcile 以原 authority 退出后缩回;maximized bounds 永不写 normalBaseBounds;fixed 中多次 open/close/replace 的账目表在实现任务展开为测试表 |
| 8 | 贴边 | 不搬窗;**可执行谓词**:`expandable ⇔ mode=normal ∧ leftGap>2 ∧ rightGap≥delta+2 ∧ target ⊆ workArea`(DIP);其余 constrained;纵向越界同属 target⊆workArea 判定;B 左移补偿 → P2 |
| 9 | minWidth / 主区 | **480**;主区硬保护 **350**;540 重命名为 logs dock 舒适值;测试区分 outer bounds / content bounds / primaryThreadWidth |
| 10 | 降级链 | A §4.6 全序;transaction 携带 trigger;system resize 无 trigger,不套用「拒绝最新操作」语义 |
| 11 | side tools 不足 | auto-hidden(960/961,hidden≠closed);**hidden 是 effective visibility,不得 unmount**:保留面板内部 openTools/activeTabKey 状态,Browser `WebContentsView` bounds 置零;ThreadPage 的 `inspectorOpen` 取 effective visibility(否则 auto-hidden 时 task tree 被误藏) |
| 12 | thread logs | docked/overlay 双态保留;540 写成 implication 不变量(见 §3-3) |
| 13 | 手动拖宽 | v1 只做窗口内分配、不动窗;**pointercancel/keyboard 行为守恒现状**(现实现 pointercancel 是 commit 非回滚,v1 保持,A 的 rollback 语义随 resize session 一起进 P2);v1 验收删除一切 native bounds effect 条目 |
| 14 | 打开时序 | **v2 改:§1.4 状态表**(先扩后开 + 100ms 视觉 deadline + pending 保持 + late ack 折叠) |
| 15 | 关闭时序 | **v2 改:§1.4 对称表**(FRAME_COMMITTED 门 + token 化 watchdog + 无动画即时分支) |
| 16 | 动画 | 窗口无 tween;面板 surface **沿用现有动画时长(170ms enter 等)**,Codex 370ms 仅参考;**v1 验收不含 370ms 断言** |
| 17 | CSS 收口 | **v2 改:新建 always-loaded `styles/app-shell.css` owner**,收口顶层 `.app-shell`、conversation 主 grid、L1/L2/right tracks、shell resizers、collapsed/hidden 几何、drag/no-drag recipe(修复现状分散于 gateway-setup/workspace-rails/conversation/sidebar 四文件——那本身就违反 owner 合约);contract test 用 `sidebar-footer-design.test.mjs` 的「import exactly once + selector 不可逃逸」模式;**grep 只扫 horizontal shell selector/property 白名单**,不全局禁 feature CSS 的 minmax/media;变量 `--gx-*` + `data-*` presentation 属性 |
| 18 | 迁移 | **v2 改:七步 + legacy policy gate(§4)** |
| 19 | 不变量 | **v2 改:替换为 §3 九条** |
| 20 | 持久化 | v1 裁定:**现状即契约**——持久化仅 `garyx.sidebarCollapsed`(localStorage)与 `threadLogsPanelWidth`(DesktopSettings);sidebar/rail/sideTools 宽度 session-only;新增宽度持久化 → P2;responsive/临时态永不写偏好 |
| 21 | 多显示器 | display 选择(最大相交、不跨屏)保留;**v2 裁:move-end 不自动补 funding**——保持 constrained reflow 至下次显式动作(与 token 铸造权一致,move 不是 panel 动作;A §11.2 该项否决) |
| 22 | 面板替换 | **v2 改:§1.2 full-vector replace transaction**,净向量一次 effect |
| 23 | 模块落点 | responsive-layout-model.ts 原地升级;shared contracts;main window bounds executor;**影响面补:`ThreadTaskTreePopover.tsx`、新 `styles/app-shell.css`** |
| 24 | minHeight | 不动(760);Codex 600 记独立事项 |
| 25 | P2 清单 | ①bounds+isMaximized 持久化;②贴边左移补偿;③拖宽 resize session 扩窗(含 xCompensation、rollback 语义);④minHeight 600;⑤sidebar/rail/sideTools 宽度持久化。**v1 验收表与 P2 验收表分列(§5),v1 不含任何 P2 条目** |
| 26 | sidebar 常量(新增) | **保留 Garyx 现值 default/min 245、max 520**;不采 Codex 275/240(避免无收益的行为变化);Codex 值仅作锚参考 |
| 27 | compact 临时展开(新增) | in-window temporary presentation(compact-overlay):不占列、不 funding、无窗口 effect、不写偏好 |
| 28 | task tree(新增) | `ThreadTaskTreePopover` 的 docked 判定改为消费 frame prop(`presentation.taskTreeDocked`),删除其 JS DOM ResizeObserver policy 与 `gateway-panels.css` 容器查询;**保留 Browser/Terminal/composer 等非横向布局的 ResizeObserver**(它们是 native child-view/终端/纵向机制,不是横向 policy,禁止笼统删除) |
| 29 | 帧原子性(新增) | controller 提供同步 `applyFrame(root, frame)`:同一 pre-paint 操作写完**全部 px vars + `data-*` attrs**(同 revision);React 只跟随该 external-store frame 渲染,不得持有第二份政策状态;live native resize 走 renderer 本地 `VIEWPORT_RESIZED_DURING_NATIVE_SESSION` 事件同帧 applyFrame,origin 由 main 的 will-resize/will-move session + 权威 snapshot 收口 |
| 30 | drag region(新增) | source contract:drag/no-drag 按 document order 合成、最后一个 carveout 常驻且保持最后 child、不可条件渲染;packaged 验收:sidebar/L2/right 各开合与扩窗后,toggle/header 按钮可点击、空白 title strip 可拖窗 |

## 3. 不变量(v2,替换 I1–I5;全部表驱动 headless)

1. `StableFrame`(显式 discriminated union,区别于 pending/transition 帧)中,所有 in-flow track 之和 = 该 frame 的 `contentViewportWidth`。
2. 所有 accepted stable plan 的 `primaryThreadWidth ≥ 350`;非法 snapshot/viewport 必须 reject,不伪造恒等式。
3. `threadLogs.presentation = docked ⇒ threadMainWidth ≥ 540`;540 不约束 side tools 或普通 conversation。
4. responsive presentation 改变不修改 requested intent;只有偏好 commit 写 persistence。
5. 仅 user/display-origin 的权威宽度 snapshot 更新 responsive basis。
6. 每个 bounds command 持有合法 `BoundsAuthority`:UserCauseToken(仅用户动作铸造)或 RepayProof(只能减少已确认 funding,绝不扩窗);snapshot-only transition 不能铸造 UserCauseToken,可延续/终止既有 authority。
7. `confirmedFunding + normalBaseBounds` = 最近一次 acknowledged session;intent closed 而 funding 暂存的合法状态仅两种:带 authority 的 deferred reconcile,或 hydrate 发现的 orphanedFunding(必须立即 RepayProof 偿还)。
8. 无外部几何变化且 transaction 全 settled 的限定下,funded open→close 恢复原 bounds。
9. 所有 accepted 物理 result 按 window revision 单调折叠;旧 sequence 不得抹掉更新 revision 的物理事实。

headless 边界:DOM commit、native drag region、真实 Electron 事件序 **不属于** headless 可证范围,归 packaged 验收(§5)。

## 4. 迁移路径(v2:legacy policy gate)

关键前提(评审 F7 坐实):现状 = sidebar 245 / 无 960 auto-hide / minWidth 1180 / task tree 双源决策。**expand-v1 的常量与新行为绝不混入 Phase 0–3**。

- **Phase 0a 入口盘点 + legacy oracle**:纯 helper 表驱动测试;完整 AppShell 用 packaged/CDP 录制**归一化结构快照**(rect、computed tracks、class/attrs、intent),不用动态截图 byte-diff;1180 以下宽度只标为 emulation/纯函数样本,不冒充真实 native 基线。
- **Phase 0b 事件归一化(UI 不变)**:四面板全部 writer 汇入一条 occupancy event log(仍走旧 controller),证明 route/capsule/cleanup 无漏口。
- **Phase 1 纯状态机,双 policy**:machine 同时实现 `legacy` 与 `expand-v1` 两套 policy;**legacy projection 与 Phase 0 快照 shadow 对拍**;expand-v1 的刻意差异单独 golden-test(差异是特性不是回归)。
- **Phase 2 controller 换芯,启用 legacy policy**:引入 atomic `applyFrame`;保持 245 / 无 auto-hide / minWidth 1180;结构快照逐项一致。
- **Phase 3 CSS 收口 + px 化**:先把 task-tree JS/CSS policy 改 frame prop,再建 `app-shell.css` owner 收口、删 policy observer/query;packaged rect oracle 一致;owner contract test + 白名单 grep 断言上线。
- **Phase 4 一次切换 expand-v1**:minWidth 480、面板常量、960/961、350 降级链、bounds IPC 同时启用;fault injection 覆盖 delayed ack(150ms)、reject、stale、maximize、display change;**feature off = 完整回到 legacy policy**(不是只停 IPC 留下新常量)。
- **验收拆表**:v1 表与 P2 表分列(§5);A §15.2/§15.3 只取 v1 子集(删 xCompensation、native drag bounds、370ms)。

## 5. 验收

**headless(实现前最低门槛)**
- reducer:全事件序列枚举——rapid reverse、replace、timeout、late accepted/rejected、fixed enter/exit、reload epoch takeover、supersede 链;**v4 新增三条:funded close→退出动画中 reload(orphaned repay 触发)、constrained open→reload(intent 恢复)、checkpoint stale→retry/epoch takeover**。
- projection:§3 九不变量;宽度档 480/720/721/960/961/980/981/1116/1280/1480/1920 × 面板组合 × 窗口模式;capacity 降级链。
- shared/main executor(fake BrowserWindow/screen):CAS、queue coalescing、实际 bounds readback、sender/window 绑定、workArea TOCTOU、revision 单调。
- controller:虚拟时钟 + atomic frame writer——vars 与 data-* 同 revision;FRAME_COMMITTED 之前不发 close shrink。

**packaged(v1 表)**
- 正常可扩:bounds ΔW = funded track 净差;左侧面板开后内容整体右移且 conversation 宽差 ≤1px;右侧面板开后 conversation rect 不变。
- 贴边/超 workArea/max/fullscreen:bounds 零变化,窗口内 reflow,main ≥350,必要时 hidden/collapsed/overlay 降级。
- 断点:720/721、960/961、980/981 精确;programmatic resize 不触发(basis 机制)。
- setBounds 次数语义(v3):每个 bounds command **至多**一次原子 setBounds;无冲突的 expandable/funded transaction **恰一次**成功调用;constrained/funding=0 transaction **零次**;supersession 链可含多个 command,最终物理 revision 序列与 oracle 一致。故障注入 150ms delayed ack 走 §1.4 表归宿;reload/hot-reload 不二次扩窗(含 reload-during-close 的 orphaned repay);maximize 退出、显示器移除后 normal base/funding 恢复;冷启动 CLAIM_INITIAL_LAYOUT 后首次关 sidebar 正确缩窗。
- live native resize 逐帧记录 `sum(columns) − innerWidth`、`primaryThreadWidth`、presentation revision(不能只看末帧截图)。
- drag region:各面板态下按钮可点、空白 strip 可拖窗(§2-30)。
- 流程:`npm run build:ui` → focused unit → `npm run dist:dir` → 重启 packaged app → attach 新 renderer,不测旧 bundle。

## 6. P2 留档

①窗口 bounds+isMaximized 持久化(Codex 式);②贴边左移 x 补偿;③拖宽 resize session 原生扩窗(xCompensation/rollback);④minHeight 600;⑤sidebar/rail/sideTools 宽度持久化;⑥Codex 275/240 sidebar 常量对齐(如未来想要)。
