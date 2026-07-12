# Garyx 桌面横向布局状态机 + 面板扩窗：对抗式设计评审

日期：2026-07-12
评审基线：仓库 `48e1a6e1b`
评审对象：[`final.md`](./final.md)
对照材料：A 版、B 版、`CLAUDE.md`、`docs/agents/desktop-ui.md` 与当前 AppShell 实现

## 结论

**总体：FAIL。结论：DO-NOT-SHIP（设计修订前不得进入实现）。**

裁决选中的大方向大多正确：`reduce/project` 分层、480 窗口下限、350 主区硬保护、side tools 只保留 right-docked rail、不为扩窗搬动窗口、绝对 bounds + revision、窗口不做 tween。这些可以保留。

但当前 final 不是一个闭合的可实现协议。最关键的问题是：final #2 把 bounds effect 限死在 `PANEL_TOGGLE` 分支，而 final #5/#7/#14/#15/#21/#22 又分别要求 stale 后重算、fixed-mode 退出对账、动画后缩窗、晚回执处理、跨屏恢复和净向量替换。后六项都需要在非 toggle 事件上继续一个由用户动作发起的 transaction。照现在的文字实现，快速开关、route 清栏、capsule 首开/末关、最大化期间关闭和 100ms 后迟到的 accepted ack 都会留下错账或多余窗口宽度。

六个指定评审面全部未过：

| 评审面 | 结论 | 核心原因 |
|---|---|---|
| 1. 裁决自洽性 | **FAIL** | #2 与 #7/#15 冲突；B I2/I3/I4 与 final #7/#9 冲突；#13/#25 与 final §2 无条件引用 A §15.2/§15.3 冲突 |
| 2. 完备性遗漏 | **FAIL** | route rail、capsule union、workspace preview、Escape/route cleanup、task-tree DOM policy、compact 临时展开没有统一的 transaction 映射 |
| 3. repo 合约 | **FAIL** | side-tools 单 presentation 已修正；但 #17 仍把 shell 几何分散在多个 feature CSS，且遗漏现有 drag-region/document-order 合约 |
| 4. 可测性 | **FAIL** | I2/I3/I4 按 final 已经为假；“stable frame”无类型；Phase 0 只能覆盖 helper，不能充当完整 AppShell oracle |
| 5. 实现风险 | **FAIL** | 100ms late-accepted ack 无归宿；“只处理最新 ack”会丢失已发生的物理变更；live resize 只直写 vars 不能原子更新 presentation attrs |
| 6. 迁移路径 | **FAIL** | 当前 245px sidebar、无 960 auto-hide、minWidth 1180；final 又引入 275/240、960/961、480，Phase 1–3 不可能天然零行为变化 |

## 阻断项与可达反例

### F1. effect 的“事件分支限制”与 transaction 生命周期矛盾

对应：final #2、#5、#7、#14、#15、#21、§2 事件骨架；B §3.3；A §6–§7、§11。

可达反例 1（快速 open→close）：

1. revision=`R`、窗口宽 `W`，用户打开 side tools；seq=1，target=`W+330`，expected=`R`。
2. ack 前用户立即关闭；seq=2，target=`W`，expected 仍为 `R`。
3. main 串行先应用 seq=1，物理窗口已变成 `W+330`、revision=`R+1`；seq=2 因 expected revision 旧而 stale。
4. A §6.4 要求 renderer “只把最新 ack 投回”，final #2 又禁止 `WINDOW_BOUNDS_REJECTED` 产生 effect。结果是 panel 已关、窗口仍为 `W+330`，且 seq=1 的真实 funding 可能从未入账。

绝对 target 只能让一次请求幂等，不能自动让一串 CAS 失败的请求收敛。**所有实际改变过 native bounds 的 accepted result 都必须按 revision 折叠，不能因 sequence 旧而丢弃；sequence 只决定最新 desired intent。**

可达反例 2（fixed mode）：normal 下已 funded 的 panel → maximize → 关闭 panel。final #7 要保留账并在 unmaximize 后缩回；但 unmaximize 是 `WINDOW_MODE_CHANGED`，final #2 明令该分支不能产 effect。若不缩，账永远悬空；若清账，又回到了被 final 否决的 B 方案。

可达反例 3（关闭动画）：final #15 要到 `PRESENTATION_ANIMATION_FINISHED`、DOM commit 之后才缩窗。若 effect 在 toggle 分支返回，会过早执行；若在 animation/frame-commit 分支返回，又违反 #2。

**具体修订建议：**

- 把 final #2 从“effect 构造只存在于 USER_TOGGLE 分支”改为“只有用户/用户导航动作可以铸造 `LayoutCauseToken`；任一 bounds command 必须携带仍存活的 cause token”。
- `WINDOW_BOUNDS_APPLIED/REJECTED`、`WINDOW_MODE_CHANGED`、`FRAME_COMMITTED`、timeout 可以继续同一 token 做 retry/repay/reconcile，但纯 responsive projection 事件不能凭空创建新的 funding claim。
- 区分三层状态：`desiredIntent`、`confirmedFunding`、`inFlightTransactions`。不要让一个 `ledger` 同时承担愿望、已执行物理事实和延迟对账。
- 对 logical close 明确规则：responsive hidden/collapsed 不改 intent、也不还账；route/thread cleanup 若把 intent 真正置为 closed，就必须偿还已有 funding，即使 close 不是按钮 toggle。

### F2. 100ms watchdog 与 late ack 没有安全语义

对应：final #5、#14；A §6.1、§6.4、§7。

`BrowserWindow.setBounds(..., false)` 在 main 内是同步调用，但 `ipcRenderer.invoke` 的排队、main event loop 和返回消息没有 100ms 上界。以下时序完全可达：main 在 95ms 已调用 `setBounds`，renderer 在 100ms 触发 watchdog 并按 constrained 打开，accepted result 在 110ms 才回来。

当前 final 没说明迟到 accepted result 如何处理：

- 忽略它：native window 已扩大，但 funding 未确认；下次打开可能二次扩窗。
- 接受它：watchdog frame 与 accepted frame 会再次跳变；若期间又有 toggle，可能反向覆盖最新 intent。
- 把 timeout 当失败并发新请求：旧请求不可取消，两个绝对 target 仍需 revision/reconcile 协议。

**具体修订建议：**

- 新增显式事件 `OPEN_DEADLINE_EXPIRED(transactionId)`、`CLOSE_DEADLINE_EXPIRED(transactionId)`、`FRAME_COMMITTED(transactionId)`；timer 只负责 dispatch，纯 reducer 不读时间。
- 100ms 是“开始显示 constrained fallback 的视觉 deadline”，不是取消或失败。transaction 保持 pending，直到 main 给出 accepted/rejected 的权威 snapshot。
- main result 必须回显 `transactionId/epoch/sequence`、实际应用后的 bounds 与 revision。任何 revision 更新的 accepted result 都要折叠；若 intent 已 supersede，reducer 随后发一个携带原 cause 的 reconcile command。
- main 队列在尚未执行时可丢弃已有更高 sequence 的旧请求；一旦 `setBounds` 已发生，就不能把其 result 当作无事发生。
- 给 open/close watchdog 和 animation finish 都加 transaction token，旧 timer/旧 `animationend` 对已 reopen/replaced 的 panel 必须是 no-op。

### F3. 当前 AppShell 的所有横向入口不能映射到 final 的事件集

对应：final #1、#2、#14、#22、§2 事件集；A §3.2；B §2.2、§3.2。

| 实际入口 | 当前代码证据 | final 的缺口 |
|---|---|---|
| global sidebar 普通 toggle | `useLayoutResizeController.ts:75-89`；两个点击面在 `AppShell.tsx:4260-4269,4865-4879` | 普通 toggle 可映射；但 compact 分支走同一 callback，却被 B 视为无 effect、A 视为 manual override，final 未裁定它是否 funding-eligible |
| L2 recent/bot/workspace rail | `AppShell.tsx:4318-4367,4428-4475`；数据/route cleanup 在 `1809-1856` | rail open/close/switch 不是单一 toggle；`ROUTE_RAIL_CHANGED` 在 #2 下不能扩/缩窗，funded rail 自动消失后会留宽 |
| side tools header | `AppShell.tsx:4591-4594` | 可映射，但只看 `inspectorOpen` 会漏掉 capsule 对同一 rail 的占用 |
| workspace file preview 自动拉开 side tools | `AppShell.tsx:1563-1565,2819-2825` | 是用户动作的后续 effect，不经过 header toggle；需要保留 cause token |
| capsule tab 首开/末关 | state 在 `AppShell.tsx:582-585`；首开 `4003-4025`；close/整栏关闭 `3784-3799`；route/thread 清空 `2771-2777` | panel occupancy 是 `inspectorOpen || openCapsuleTabs.length>0`（`3814-3819`）。必须在 union 的 0↔1 边沿建 transaction；单个 tab toggle 不是 panel toggle |
| thread logs toggle/replace | `AppShell.tsx:4595-4602`；Escape/route/no-thread cleanup 在 `2779-2805,2846-2850` | 打开 logs 同时用多个 React setter 清 side tools/capsules；final #22 要一次净向量，但事件集中没有 `REPLACE_RIGHT_PANEL` 或完整 desired vector transaction |
| 四类 panel 拖宽 | controller 的 pointer/keyboard 路径在 `useLayoutResizeController.ts:220-287,373-530` 与 `AppShell.tsx:916-967` | v1 可映射到 PANEL_RESIZE_* 且无 bounds；但 current sidebar/L2 pointercancel 实际 commit，不是 A 的 rollback，迁移必须明确是否改行为 |
| task tree dock/overlay | `ThreadTaskTreePopover.tsx:210-233` 仍用 DOM width + ResizeObserver；CSS 另有 `gateway-panels.css:117-206` container policy | final frame 声称输出 task-tree presentation，但影响面/事件接线没有移除这份 JS DOM policy；否则仍不是单一决策源 |

还需特别约束 side-tools auto-hidden：hidden 必须保留 `ThreadSideToolsPanel` 内部 `openTools/activeTabKey`（`SideToolsPanel.tsx:386-391`），并把 Browser `WebContentsView` bounds 置零；不能把 `hidden` 实现成当前条件渲染式 unmount 后再声称 “hidden≠closed”。传给 ThreadPage 的 `inspectorOpen` 也应取 effective visibility，否则 rail 已 auto-hidden 时 task tree 仍被错误隐藏。

**具体修订建议：**

- 用一个归一化入口替代观察若干 React boolean：`LAYOUT_INTENT_CHANGED { previousOccupancy, nextOccupancy, cause, transactionId }`，一次携带四 panel 的完整 desired occupancy。
- `cause` 至少区分 `user-panel`、`user-route`、`system-cleanup`、`responsive-presentation`、`hydrate`。只有前两类可新建 funding；任何类都可偿还一个已存在且 intent 被真正关闭的 funding。
- side-tools↔logs 必须是一个 full-vector replace event；不能寄希望于 React 恰好把多个 setter 永远批成一个 render。
- 单独裁定 compact temporary open：是 in-window temporary presentation，还是一次可 funding 的显式打开。当前 final 同时继承了 A/B 两种答案。
- 把 `ThreadTaskTreePopover` 的 `docked` 改成 frame prop；保留 Browser/Terminal/composer 的 ResizeObserver，因为它们是 native child-view/终端/纵向几何机制，不是横向 policy。禁止笼统删除所有 ResizeObserver。

### F4. hydrate/reload 无法兑现“不二次扩窗”

对应：final #6、#7、#19、§2 验收；A §5.3、§11.3、§15.3。

可达反例：side tools 已 funded 330px 且打开，renderer reload。`inspectorOpen` 与 `openCapsuleTabs` 都是 renderer-local 初始空状态（`AppShell.tsx:582-585,656-657`），而 final 把 funding ledger 也放 renderer state。reload 后 HYDRATE 只能“认领当前可见 tracks”，看不到已经丢失 intent 的 side tools；多出来的 330px 被误认成 base。下一次首开 side tools，在大 workArea 上会再次申请 330px，直接违反 A §15.3 的 reload 不二次扩窗验收。

另一个未定义分支是 maximized/fullscreen 中 HYDRATE：final #6 说按“当前宽度”认领，#7 又禁止把 maximized bounds 写进 `normalBaseBounds`，两者不能同时执行。

**具体修订建议：**

- main 的 per-window executor 保存一个**不含布局政策的 opaque acknowledged session**：`normalBaseBounds + confirmedFundingByPanel + lastAppliedRevision`，跨 renderer epoch、只到 BrowserWindow 销毁为止；HYDRATE 认领该 session。或者删除 renderer reload 的不二次扩窗承诺并在 reload 主动 reset bounds，二选一，不能靠可见 track 猜。
- maximized/fullscreen hydrate 必须使用 snapshot 的 `normalBounds` 初始化 normal ledger，当前 content bounds 只用于 fixed-mode projection。
- 部分 funding 的稳定分配顺序必须在 schema 中写死；不能只写“稳定 panel 顺序”。
- `rendererEpoch` 需要注册/接管协议；新 epoch 接管后，旧 epoch 已排队请求只可返回 superseded，不能继续改窗。

### F5. 不变量 I1–I5 不能按 final 原样成立

对应：final #19；B §3.3；A §1、§15.2。

- **I2 已被裁决推翻。** B 的 I2 是 `main >= MAIN_MIN(540)`；final #9 明确把 540 政名为 logs dock comfort，并允许 conversation 到 350。
- **I3 与 final 多处冲突。** B 的 I3 是非 USER_TOGGLE effect 恒 null；fixed-mode reconcile、stale retry、动画后 close、跨屏恢复都需要非 toggle event 继续既有 transaction。
- **I4 直接为假。** B 的 I4 是 `ledger[p]存在 ⇒ intent[p].open`；final #7 明确要求 fixed mode 中 intent 已关闭但 ledger 保留到退出后对账。
- **I5 契约已变形。** B 的 I5 针对 delta + main clamp；final #5 采用 absolute target + expected revision，应分别在纯 geometry helper 与 main executor 断言。
- **I1 缺少 stable 的可判定类型。** open 时 native viewport 已变宽、ack 尚未来时，final #14 又要求保留旧 columns。此时 columns 若仍求和旧 viewport，I1 失败；若 project 立即让 main 吃掉新余量，又没有做到“旧 columns 暂时保留”。
- funding 反转恒等只在“无 user move/resize、无 mode/display 变化、所有请求已 settled”的前提下成立，当前 #19 没写前提。

**建议替换为以下可执行不变量：**

1. `StableFrame`（显式 discriminated union）中，所有 in-flow track 之和等于该 frame 的 `contentViewportWidth`。
2. 所有 accepted stable plan 的 `primaryThreadWidth >= 350`；非法 snapshot/viewport 必须 reject，不伪造恒等式。
3. `threadLogs.presentation=docked ⇒ threadMainWidth >= 540`；540 不约束 side tools 或普通 conversation。
4. responsive presentation 改变不修改 requested intent；只有偏好 commit 修改 persistence。
5. 仅 user/display-origin 的**权威宽度 snapshot**更新 responsive basis。
6. 每个 bounds command 都有一个存活的 user-cause token；snapshot-only transition 不能 mint token，但可继续/终止已有 token。
7. `confirmedFunding + normalBaseBounds` 等于最近一次 accepted normal target；intent closed 时允许 funding 暂存的唯一原因是带 token 的 deferred reconcile。
8. 在无外部几何变化且 transaction 全 settled 的限定下，funded open→close 恢复原 bounds。
9. 所有 accepted physical result 按 window revision 单调折叠；旧 sequence 不得抹掉更新 revision 的物理事实。

这些不变量可用表驱动 headless tests 断言；DOM commit、native drag region、实际 Electron event ordering 不能伪装成 headless 可证，必须另做 packaged 验收。

### F6. CSS owner、drag region 与 live resize 仍违反/遗漏 repo 合约

对应：final #17、#18；B §3.6–§3.7；`CLAUDE.md:89-93`；`docs/agents/desktop-ui.md:42-50,73-80`。

side tools 不做 overlay 的修正是 **PASS**。其余仍有三个问题：

1. **owner stylesheet 仍不闭合。** B §3.7/ final #17 按现文件原位改：top-level `.app-shell` 在 `gateway-setup.css:305-329`，`.conversation`/side-tools grid 在 `workspace-rails.css:1318-1377`，thread layout 在 `conversation.css:927-980`，collapsed rail 在 `sidebar.css:740-750`。这正是 repo 合约禁止的“全局 shell recipe 散落在多个 feature 文件”。“加 owner contract test”不能让分散本身合法。
2. **grep 承诺不可执行。** 当前 `styles/` 有 135 处 `minmax(`/`@media`/`@container`，绝大多数是 feature 内部或纵向布局。B Phase 3 的全目录 grep 会误杀 settings、composer、capsules 等合法 responsive；应只扫描 horizontal shell selector/property 白名单。
3. **drag-region 合约被完全漏掉。** `AppShell.tsx:4865-4879` 与 `sidebar.css:701-734` 明确记录 Electron drag/no-drag 按 document order 合成、最后一个 carveout 必须常驻且保持最后 child。状态机将运行时改变 top-strip columns、header actions 和面板 DOM；final 没有 source contract，也没有 packaged 的点击/拖窗验收。

live user resize 也没有被 B §3.7 的“同帧直写 cssVars”完全解决：numeric vars 在 rAF 写入，但 `data-sidebar-state`、`data-right-dock-state`、task-tree/log presentation 若仍等 React commit，会出现一帧“新数字 + 旧 presentation”。而 final 的 A 事件集只写 `WINDOW_SNAPSHOT_CHANGED`，没有明确保留 B 的 renderer-local `VIEWPORT_RESIZED`；若等 main→renderer snapshot IPC，就更不可能同一 paint 更新。

**具体修订建议：**

- 新建/指定一个真正 always-loaded 的 `styles/app-shell.css` owner；把顶层 `.app-shell`、主 conversation grid、L1/L2/right tracks、shell resizers、collapsed/hidden geometry与 drag/no-drag recipe 收口进去。用现有 `sidebar-footer-design.test.mjs:146-165` 的“import exactly once + selector 不可逃逸”模式做合同测试。
- contract 只禁止 owner 横向 selectors 使用 `fr/minmax` 或 width/container breakpoints；不要全局禁止 feature CSS。
- 新增 source contract：carveout 始终是 app-shell 最后 child、不可条件渲染；packaged 测试在 sidebar/L2/right panel 各开合与扩窗后，验证 toggle/header buttons 可点击、空白 title strip 能拖窗。
- controller 必须有一个同步 `applyFrame(root, frame)`，在同一次 pre-paint 操作中写完全部 px vars 与 `data-*` attrs；React 只跟随该 external-store frame 渲染，不得形成第二份政策状态。
- 增加本地 `VIEWPORT_RESIZED_DURING_NATIVE_SESSION` 渲染事件，并以 main 的 `will-resize/will-move` session + 权威 snapshot/revision 收口 origin。当前 Electron typings 已说明 `will-resize/will-move` 只对手动操作触发，可用于区分 programmatic setBounds。
- packaged live-resize 验收逐帧记录 `sum(columns)-innerWidth`、`primaryThreadWidth` 和 presentation revision；不能只看最终截图。

### F7. Phase 0–3 的“零行为变化”承诺不成立

对应：final #18、#20、#25、§2 验收；A §3.3、§15；B §2.2、§4。

当前事实是：

- BrowserWindow `minWidth` 仍是 1180（`src/main/index.ts:443-447`）。
- 当前 sidebar 默认/min 都是 245（`useLayoutResizeController.ts:53,380-383`）；A 改为默认 275/min 240，B 保留当前 245，final 没有逐项裁定。
- 当前 Garyx 没有 960/961 side-tools auto-hide；open 时 CSS 固定插入至少 320+10（`workspace-rails.css:1345-1352`）。
- 当前 task tree 同时靠 DOM ResizeObserver 与 container query 决策。
- sidebar、rail、side-tools 宽度是 session state；只有 `garyx.sidebarCollapsed` 和 thread logs width 有现成持久化。final #20 却笼统说“宽度偏好经 persistence adapter 写”，没有键、owner 或迁移语义。

因此 Phase 2 若直接消费 final solver，会同时引入 275/240、960 auto-hide、350 capacity 链等新行为；不可能一边说“Phase 0 oracle 全绿”，一边说“逐像素守恒”。

final §2 还无条件引用 A §15.2/§15.3，重新把 P2 混进 v1：A §15.2 要测 right `xCompensation`，A §15.3 要求拖宽每 rAF 发送 absolute window effect；这与 final #13/#25 明确把 native resize session/x compensation 推迟到 P2 冲突。A §15.3 的约 370ms surface spring 也与 final #16“沿用现有动画，370ms 不强换”冲突。

**具体修订后的迁移顺序：**

1. **Phase 0a：入口盘点与 legacy oracle。** 对纯 helper 建表测试；对完整 AppShell 用 packaged/CDP 录制归一化结构快照（rect、computed tracks、class/attrs、intent），不要把动态截图 byte-diff 当 oracle。1180 以下只能标为 CDP emulation/纯函数样本，不是假装真实 native baseline。
2. **Phase 0b：归一化事件适配但仍走旧 controller。** 四 panel 所有 writer 先汇入一条 occupancy event log，证明 route/capsule/cleanup 无漏口；不改变 UI。
3. **Phase 1：纯 machine 同时有 `legacy` 与 `expand-v1` policy。** legacy projection 才与 Phase 0 shadow 对拍；expand-v1 的刻意行为差异单独 golden-test，不能拿“不同”当回归。
4. **Phase 2：controller 换芯但启用 legacy policy。** 引入 atomic frame writer，保留 245、无 960 auto-hide、minWidth1180，验证结构快照一致。
5. **Phase 3：在单一 owner stylesheet 下 px 化。** 先把 task-tree JS/CSS policy 改为 frame prop，再删 policy observer/query；packaged rect oracle 必须一致。
6. **Phase 4：一次性启用 expand-v1 feature。** 此时才切 480、明确的 panel constants、960/961、350 chain 与 bounds IPC；用 fault injection 覆盖 delayed ack、reject、stale、maximize、display change。feature off 必须完整回到 legacy policy，而不是只停 IPC、却留下新 auto-hide/常量。
7. 把 A §15 验收拆成 `v1` 与 `P2` 两张表；v1 删除 native drag bounds effect、right x compensation 与强制 370ms 条目。

## 25 个决策点逐项裁决

| # | PASS/FAIL | 评审结论与必须修订 |
|---|---|---|
| 1 | **PASS** | `reduce/project` 分层是正确边界；但 command/transaction phase 必须按 F1 补齐，不能只返回一个瞬时 effect。 |
| 2 | **FAIL** | 类型级限制过强，阻断 retry/repay/reconcile。改为“bounds command 必须持有 user-cause token”，不是“只能从某 event branch 返回”。 |
| 3 | **PASS** | `responsiveBasisWidth` 能切断 panel-machine 跨断点反馈；要求 origin 来自权威 resize session，并测 duplicate/out-of-order snapshot。 |
| 4 | **PASS** | funding vector 比简单 ledger 正确；实现时拆分 confirmed funding、desired intent、pending transaction。 |
| 5 | **FAIL** | absolute target + revision 仍缺完整 CAS 收敛与物理 ack 折叠；“只处理最新 ack”错误。按 F1/F2 修。 |
| 6 | **FAIL** | 可见 track 认领无法覆盖 renderer reload 丢失的 right-panel intent，也与 maximized hydrate 冲突。需 per-window acknowledged session。 |
| 7 | **FAIL** | deferred reconcile 方向正确，但与 #2、B I4 直接冲突；补 cause-token continuation 和 fixed-mode 多次 open/close/replacement 表。 |
| 8 | **FAIL** | “窗口居中”不是可执行谓词，且与 A 的“未贴边”措辞不同。写成明确不等式，例如 `leftGap>2 && rightGap>=delta+2`，并说明 vertical/out-of-workArea 分支。 |
| 9 | **PASS** | 480 + 350 自洽，540 仅作 logs dock comfort；测试必须区分 outer bounds、content bounds、primary thread width。 |
| 10 | **PASS** | 降级全序可用；transaction 必须携带 trigger，system resize 无 trigger 时不得套用“拒绝外部 resize”的语义。 |
| 11 | **PASS** | 与 repo side-tools 单 right-docked presentation 合约一致；hidden 不能 unmount 丢状态。 |
| 12 | **PASS** | logs docked/overlay 是现有且受 repo 允许的双态；将 540 写成 implication invariant。 |
| 13 | **PASS** | v1 拖宽只做窗口内分配显著降风险；需明确 pointercancel/keyboard 是否守恒现行为，并从 v1 验收删掉 native bounds effect。 |
| 14 | **FAIL** | 100ms timeout、late accepted ack、native resize-before-ack、superseded open 都没闭合。按 F2 增加 deadline/transaction/result 协议。 |
| 15 | **FAIL** | animation finish 后才产生/执行 shrink 与 #2 冲突；缺 `FRAME_COMMITTED`、tokenized watchdog、无退出动画 panel 的即时分支。 |
| 16 | **FAIL** | “窗口无 tween”本身 PASS；但 final §2 无条件引用 A §15.3 的 370ms 验收，和本条“不强换现有 170ms”矛盾。重写 v1 验收。 |
| 17 | **FAIL** | B 的逐文件表继续分散 shell owner；遗漏 task-tree JS observer、drag-region、atomic vars+attrs。按 F6 收口。 |
| 18 | **FAIL** | 四阶段纪律合理，但没有 legacy policy gate 就不能零行为变化；shadow 也只能比较 legacy 语义。按 F7 重排。 |
| 19 | **FAIL** | I2/I3/I4 已被 final 自己推翻，I1 无 stable 类型，I5 仍是旧 delta 契约。用 F5 的新不变量集替换。 |
| 20 | **FAIL** | collapse preference 语义正确；其他 width persistence 两版并未真正一致，当前也无 sidebar/rail/side-tools 存储。明确 v1 哪些是 session-only、哪些有 adapter/key。 |
| 21 | **FAIL** | display 选择/不跨屏规则可保留；但 A §11.2 的 move-end 自动补 funding 需要非 toggle command，和 #2 冲突。决定“保持 reflow 到下次显式动作”或允许既有 cause reconcile。 |
| 22 | **FAIL** | 净向量原则正确，但没有 full-vector replace event，无法保证当前多个 React setter 永远一笔提交。增加声明式 replacement transaction。 |
| 23 | **PASS** | 原地升级 model、shared contract、main executor 的落点合理；影响面需补 `ThreadTaskTreePopover.tsx` 与 app-shell owner stylesheet。 |
| 24 | **PASS** | minHeight 760 留在本任务外，边界清楚。 |
| 25 | **FAIL** | P2 清单本身合理，但 final §2 又无条件引入 A 的 P2 resizer/xCompensation 验收，实际没有隔离。拆分验收后才可 PASS。 |

## 打开分支必须补成的状态表

final #8 + #14 至少要明确以下归宿，当前只完整描述了前两行的一部分：

| 初始判定 | main/时间结果 | 必须的最终状态 |
|---|---|---|
| constrained（贴边/fixed/余量不足） | 不发 IPC | 立即输出 constrained accepted frame；funding=0 |
| expandable | deadline 前 accepted | 按 actual snapshot 确认 funding，再一次 commit 新 columns/surface |
| expandable | 明确 rejected | 用返回 snapshot 输出 constrained frame；funding=0 |
| expandable | 100ms 未回 | 显示 constrained fallback，但 transaction 仍 pending |
| watchdog 后 | late accepted | 折叠物理 revision/funding；若 intent 仍 open，切到 funded frame；若已 supersede，立即 reconcile 到最新 absolute target |
| watchdog 后 | late rejected | 结束 pending，保留 constrained frame |
| 任意 pending | 快速反向 toggle / right-panel replace | 更新 desired intent；旧 accepted result 仍按 revision折叠，再以同一 cause 链收敛 |
| 任意 pending | maximize/fullscreen/display/workArea 变化 | 旧 expected revision 失效；返回权威 snapshot，按 fixed/constrained 或 deferred reconcile 收敛 |

关闭也需要对称表：funding=0 不发 shrink；有 funding 时 exit/420ms → frame commit → apply；期间 reopen/replace 使旧 finish/watchdog no-op；fixed mode 只记 deferred，退出后 continuation。

## Phase 0 与测试可行性判定

**纯状态机最终可以 headless 测，完整 Phase 0 现状 oracle 不能只靠现有 headless tests。** 当前项目没有 React/jsdom testing harness，现有测试能直接覆盖的是纯 helper。实际执行的 focused baseline：

```text
npm run test:unit -- \
  src/renderer/src/app-shell/responsive-layout-model.test.mjs \
  src/renderer/src/app-shell/diagnostics-helpers.test.mjs \
  src/renderer/src/app-shell/components/side-tools-panel-model.test.mjs

15 tests passed, 0 failed
```

这证明现有 720/980 helper、logs dock helper、side-tools clamp 和 capsule tab union helper 可被 characterization；它**没有**证明 AppShell 的 route effects、React batching、DOM grid、native bounds 或 drag regions。后者需要 F7 所述的 normalized packaged trace。

实现前最低测试门槛：

- reducer：全事件序列枚举，含 rapid reverse、replace、timeout、late result、fixed enter/exit、reload epoch takeover。
- projection：F5 新不变量、480/720/721/960/961/980/981、四 panel union 与 capacity chain。
- shared/main executor：fake BrowserWindow/screen 下 CAS、queue coalescing、actual bounds readback、sender/window binding、workArea TOCTOU、revision monotonic。
- controller：虚拟 clock + atomic frame writer，断言 vars 与 `data-*` 同 revision；frame commit 之前不发 close shrink。
- packaged：真实 `setBounds` 序列、故障注入的 150ms delayed ack、maximize/fullscreen、display move/removal、native live resize、Browser side-tool child view、drag/no-drag 点击与拖窗。

## 最终裁决

保留 final 的产品方向和数值锚，但必须先修订 #2/#5/#6/#7/#14/#15/#17/#18/#19/#20/#21/#22/#25，并把 §2 的 A 验收引用改成明确的 v1 子集。修订前，该设计在可达时序上会产生 panel 已关但窗口未缩、late ack 无账、renderer reload 二次扩窗，以及 responsive/route 入口绕过 funding 的问题。

**DO-NOT-SHIP。**
