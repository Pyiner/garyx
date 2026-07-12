# Garyx 桌面横向布局状态机与面板扩窗设计（A 版）

日期：2026-07-12
范围：`desktop/garyx-desktop`，仅设计，不含代码改动

## 1. 拍板结论

本方案不是复制当前 Codex 的窗口行为，而是在 Codex 数值锚点上做 Garyx
增强：

- **正常窗口、空间允许时**，用户显式打开左 sidebar、L2 会话列表、右 side
  tools 或 docked logs，原生窗口按该 panel 的完整 track 宽度扩展；关闭时只
  偿还这个 panel 曾获得的窗口宽度。conversation 在交易前后不变。
- 左 sidebar / L2 从左向右出现：窗口左边不动、右边增宽，后续内容整体右移。
  右 dock 在原 conversation 右侧追加：conversation 的屏幕位置和宽度均不变。
- **原生空间受限时**，包括目标 bounds 会越出当前 display workArea、最大化、
  全屏，窗口不移动、不退出当前模式，降级为窗口内 reflow。允许 conversation
  被挤窄，但不得低于 Codex 锚定的 **350px**；再不足时由状态机自动收起、隐藏、
  overlay 或拒绝最新操作。
- `BrowserWindow.minWidth` 从 1180 降到 **480**。Codex 的完整最小窗口是
  480×600；本任务只需改横向 `minWidth`，Garyx 现有 `minHeight: 760` 可保持，
  600 作为独立的纵向对齐项记录。
- 整个横向布局只有一个决策源：把
  `responsive-layout-model.ts` 升级为 **TypeScript/JavaScript 纯函数状态机**。
  它计算每个 track 的精确像素、有效/自动隐藏态和窗口 bounds effect。
- CSS 不再决定横向响应式行为：不靠 flex shrink、`minmax(0, 1fr)`、结构性
  media query 或 DOM 实测反推布局。CSS 只消费状态机输出的 CSS variables、
  inline width 和 `data-*` presentation 属性。
- 主进程仍独占 `BrowserWindow` bounds；renderer 拥有 panel intent 和布局
  状态机。主进程只校验并执行绝对 bounds effect，不复制一份布局规则。

核心稳定态不变量：

```text
sum(all effective horizontal tracks) == viewportWidth
conversationWidth >= 350

normal + expandable 的 panel 显式开合：
conversationWidth(after) == conversationWidth(before)  // 容差 1 DIP

constrained-reflow：
windowBounds(after) == windowBounds(before)
conversationWidth(after) <= conversationWidth(before)，但 >= 350
```

## 2. Codex 26.707 实测数据

### 2.1 方法与安全门

- attach 前先核验调试目标的 page title 为 `Codex`，并确认监听进程是已安装的
  ChatGPT/Codex 客户端，避免连接其他应用的调试会话。
- CDP 用于 DOM 尺寸、按钮开合、viewport 扫描和 rAF 动画采样。
- Swift `CGWindowListCopyWindowInfo` 以 16ms 采样原生窗口 bounds。
- `@electron/asar` 只读核验 app.asar 中的窗口最小值、持久化与全部
  `setBounds` 调用路径。
- 通过备份后临时修改 `electron-main-window-bounds` 并重启，复测真实窄窗口；
  原始本机状态在测量结束后恢复。

测试版本：ChatGPT/Codex `26.707.51957`，build `5175`；测试屏幕 DPR 2，
workArea 1920×955。bundle 默认窗口 1280×820；测量前恢复的持久窗口为
1116×811。

### 2.2 硬数据

| 项目 | 实测/源码核验 |
| --- | --- |
| 原生窗口最小尺寸 | **480×600**；写入 300×500 后重启，真实 inner size 被 clamp 到 480×600 |
| 面板开合与窗口 bounds | 1094px、600px、最大化等场景中，任一 panel 开/关的 16ms bounds 序列均为零变化；main bundle 没有 panel `setBounds` 路径 |
| 左 sidebar 默认宽 | **275px** |
| 左 sidebar 拖宽 | **240–520px**；向下拖过 240 即收起，重开恢复上次宽度 |
| 左 sidebar 动画 | JS spring，约 **370ms** settle；Reduce Motion 立即完成 |
| 左 sidebar 断点 | resize 中没有自动收起；仅启动初始态 `viewport <= 720` 收起、`>=721` 恢复持久 open，和 Garyx 720 常量精确吻合 |
| 右 panel 默认/最小宽 | **320px**；小于 320 的拖拽收起 |
| 右 panel 响应式 | `viewport <= 960` 自动隐藏、`>=961` 恢复，hidden 不等于 closed |
| Codex conversation 保护值 | 961 临界点实测恰为 **350px**；通过隐藏右 panel 保护，不是 CSS hard min |
| 主内容真实退化 | 左 sidebar 路径没有保护，窄窗可把 main 压到 0；这是缺陷，不作为 Garyx 对齐目标 |
| 贴边、最大化、全屏 | 无 panel bounds 分支，全部是固定窗口内 reflow |

### 2.3 对本方案的含义

当前 Codex **没有**“panel 开合 = 原生窗口扩展”。Garyx 的扩窗是用户明确拍板
的增强，不能在文档或测试命名中声称 1:1 复制。可直接采用的锚是：

- window min width 480；conversation protection 350；
- sidebar 默认 275、拖宽 240–520、拖过 240 收起；
- right dock 默认/最小 320，自动隐藏 960/961；
- 约 370ms JS spring；
- Garyx 已有 single/dual compact 720/980 继续保留，其中 720 得到本轮实测
  再确认。

## 3. 单一横向布局状态机

### 3.1 文件与函数边界

`responsive-layout-model.ts` 从若干 helper 升级为无副作用 owner，建议对外只暴露：

```text
transitionHorizontalLayout(previousState, event) -> {
  state: HorizontalLayoutState,
  plan: HorizontalLayoutPlan,
  effects: HorizontalLayoutEffect[]
}

deriveHorizontalLayout(state) -> HorizontalLayoutPlan
```

要求：不读 DOM、`window`、localStorage、时间、React state 或 Electron；相同输入
必须得到字节级相同输出。测试可以直接喂事件序列，无需渲染 AppShell。

### 3.2 输入、内部状态、输出

```text
HorizontalLayoutIntent
  globalSidebar: requestedOpen, preferredWidth
  conversationRail: requestedOpen, preferredWidth, kind(recent/workspace/bot)
  rightDock: requestedOpen, preferredWidth, kind(side-tools/thread-logs)
  compactManualOverrides

HorizontalLayoutEnvironment
  viewportWidthDip
  windowBounds / contentBounds
  workArea
  mode: normal | maximized | fullscreen
  resizeOrigin: user | panel-machine | display | mode

HorizontalLayoutState
  intent
  environment
  responsiveBasisWidth       // 只由真实 user/display resize 更新
  effectivePresentationByPanel
  normalBaseBounds
  fundedGeometryByPanel      // 每 panel 获得的 {widthDelta, xDelta}
  resizeSession
  pendingTransaction
  rendererEpoch / sequence / acceptedWindowRevision

HorizontalLayoutPlan
  columns                    // 每一横向 track 的精确 DIP
  presentations             // docked | overlay | hidden | collapsed
  reasons                   // user | compact | auto-hidden | capacity | fixed-mode
  conversationWidth
  headerDensity / taskTreePresentation
  windowEffect?             // absolute target + deltaWidth + xCompensation
```

事件至少包含：

```text
HYDRATE
PANEL_TOGGLE_REQUESTED
ROUTE_RAIL_CHANGED
PANEL_RESIZE_STARTED / PREVIEWED / COMMITTED / CANCELLED
WINDOW_SNAPSHOT_CHANGED
WINDOW_BOUNDS_APPLIED / WINDOW_BOUNDS_REJECTED
PRESENTATION_ANIMATION_FINISHED
WORKAREA_CHANGED / WINDOW_MODE_CHANGED
```

`requestedOpen` 与 `effectivePresentation` 必须分开。自动隐藏 right dock、compact
收 sidebar、logs overlay 都不改用户 intent；宽度恢复后由同一个纯函数恢复。
只有显式 toggle 和确认后的 resize commit 写偏好。

### 3.3 常量收口

```text
WINDOW_MIN_WIDTH = 480
MIN_CONVERSATION_WIDTH = 350

SINGLE_RAIL_COMPACT_WIDTH = 720
DUAL_RAIL_COMPACT_WIDTH = 980
RIGHT_DOCK_AUTO_HIDE_WIDTH = 960

SIDEBAR_DEFAULT_WIDTH = 275
SIDEBAR_MIN_WIDTH = 240
SIDEBAR_MAX_WIDTH = 520
CONVERSATION_RAIL_DEFAULT_WIDTH = 258
CONVERSATION_RAIL_MIN/MAX = 220/420
SIDE_TOOLS_DEFAULT/MIN_WIDTH = 320
THREAD_LOGS_DEFAULT/MIN/MAX = 360/280/760
RIGHT_DOCK_RESIZER_WIDTH = 10
```

现有 `SIDE_PANEL_MIN_MAIN_WIDTH = 540` 不再叫“主区最小宽”，重命名为
`THREAD_LOG_DOCK_COMFORT_WIDTH = 540`：它只决定 logs dock/overlay，真正硬保护
值是 350。现有 `TASK_TREE_DOCK_MIN_WIDTH = 1088` 也由同一 plan 输出，但 task
tree 不加入本任务四类 panel 的原生扩窗账本。

## 4. 纯函数求解顺序

每个事件严格按以下顺序求解，禁止 CSS 或 effect 再做第二次裁决：

1. **归一化 intent**：宽度 finite、整数 DIP、落在各自 min/max；side tools 与
   logs 仍互斥；Recent/workspace/bot 共享一个 `conversation-rail` id。
2. **更新 responsive basis**：仅 `resizeOrigin=user|display` 更新。主进程为 panel
   扩窗发回的 viewport 变化不更新 basis，避免开栏扩窗后跨断点触发反向状态。
3. **应用标准自动态**：
   - 无 L2 时 `<=720` 收 global sidebar，`>=721` 恢复；
   - 有 L2 时 `<=980` 收 global sidebar，`>=981` 恢复；
   - right dock `<=960` hidden，`>=961` 恢复；
   - 用户在 compact/hidden 状态再次显式打开，建立 session manual override，
     允许本次打开；下一次真实用户 resize 或显式关闭清除 override。
4. **决定 geometry mode**：若 normal、当前窗口未横向贴 workArea 边缘，且目标
   原生 bounds 完整位于当前 workArea 内，为 `expandable`；否则为
   `constrained-reflow`。边缘判定使用固定 2 DIP tolerance，不得先移动窗口找空间。
5. **求 panel presentation**：logs 不满足 540 舒适值时 overlay；side tools 只有
   repo 规定的 right-docked presentation，不能借本任务增加 overlay。
6. **守住 350**：constrained 下先在窗口内挤到 350；仍不足时保护最新显式
   trigger，按“非 trigger right dock hidden → global sidebar collapsed → logs
   overlay → 非 trigger L2 hidden”降级。trigger 单独加 350 仍放不下则拒绝最新
   open/resize，保留上一个 accepted plan。
7. **输出精确 columns 与可选 window effect**。任何稳定 plan 都满足 columns
   求和等于 viewport、conversation `>=350`。

480px 最窄窗口因此不会靠 flex 碰运气：常规自动态下 sidebar collapsed、right
dock hidden；若 L2 也放不下 350，则 L2 暂时 hidden，intent 保留。

## 5. 正常窗口的扩窗账本

### 5.1 Track 贡献

| panel id | side | 默认横向贡献 |
| --- | --- | ---: |
| `global-sidebar` | left | effective sidebar width，建议默认 275 |
| `conversation-rail` | left | rail width，默认 258 |
| `side-tools` | right | 10px seam + panel width，默认 330 |
| `thread-logs` | right | docked 时 10 + width，默认 370；overlay 为 0 |

resizer 若是覆盖 hit area 不重复计宽。side tools↔logs 一次事件做声明式替换，
330→370 的原生净变化是 +40，不能先缩 330 再扩 370。

### 5.2 Funding vector

状态机不只记“panel 是否可见”，还给每个 panel 记原生窗口实际资助的几何向量：

```text
fundedGeometryByPanel[id] = {
  widthDeltaDip,
  xCompensationDip
}
```

- 正常按钮开左/右 panel：若可扩，`widthDelta = trackDelta`、`xCompensation = 0`；
  窗口从右侧增宽。
- 受限 reflow 打开的 panel：funding 为 0。关闭它只删 track，不缩原生窗口。
- 关闭 funded panel：精确反转它自己的 vector；不按当前 panel 宽度猜测。
- 用户移动/原生缩放后重建 `normalBaseBounds`，但保留已确认 funding；所有目标
  从 base + funding 绝对求值，不累计浮点 delta。
- side tools↔logs、多个 panel 同时因 route 改变时，求所有 funding 的净向量后
  只发一次 bounds effect。

IPC 对诊断暴露 delta，但执行的是绝对 `targetBounds`，所以重试不会重复加宽。

### 5.3 首次 hydrate

启动不能因为默认 sidebar 已显示就再扩一次。先用当前 viewport 算 compact /
auto-hidden，再认领合法可见 tracks：

```text
visibleTrackWidth = sum(effective docked tracks)
normalBaseWidth = max(WINDOW_MIN_WIDTH, currentWidth - visibleTrackWidth)
hydratableFunding = currentWidth - normalBaseWidth
```

hydrate 本身永远无 bounds effect。通常宽窗口可完整认领所有 visible tracks；若
靠近最小宽度，只认领 `hydratableFunding`，剩余视为 constrained reflow。部分
funding 按稳定 panel 顺序分配，保证同一输入可复现。第一次关闭最多缩到 480，
不会穿透 BrowserWindow minimum。

## 6. 开合、动画与竞态时序

### 6.1 Expandable 打开

1. `PANEL_TOGGLE_REQUESTED` 进入 `preparing-open`，旧 columns 暂时保留。
2. 纯函数输出绝对 target bounds；controller 发 IPC。
3. 主进程原子 `setBounds(target, false)`，回 accepted revision。
4. `WINDOW_BOUNDS_APPLIED` 后状态机一次性输出新 columns；panel surface 做约
   370ms JS spring reveal。

3→4 之间只会短暂多出空白，不会出现 conversation 被压窄的一帧。原生窗口
首版不 tween；Codex 本身也没有 bounds 动画，逐帧 native animation 没有实测锚。

### 6.2 Expandable 关闭

1. logical intent 先关，track reservation 保留，surface 做退出 spring。
2. `PRESENTATION_ANIMATION_FINISHED`（另有 420ms watchdog）后，状态机先输出
   width=0；React DOM commit 后的 layout effect 才发送缩窗 effect。
3. 主进程反转该 panel funding，原子缩窗并 ack。

顺序必须是“先删 track、后缩窗口”：主区可短暂变宽，不能先缩窗造成挤压。
Reduce Motion 跳过 spring/watchdog，仍走相同状态迁移。

### 6.3 Constrained reflow

无 window effect，columns 在当前 viewport 内一次重算。开合仍可做 surface
spring，但 allocation 由状态机给出的终值控制；CSS animation 不参与宽度决策。

### 6.4 Latest-wins

每个 renderer 有 epoch，事件 sequence 单调递增，主进程 window revision 也单调
递增。主进程串行执行；旧 sequence 返回 `stale`。renderer 只把最新 ack 投回
状态机。快速双击、route 瞬切、capsule open 后立即 close 都以完整目标 ledger
重算，不发送“盲加 320 / 盲减 320”。

## 7. 主进程 / renderer IPC 边界

### 7.1 Snapshot 与 effect

shared contract 建议包含：

```text
WindowLayoutSnapshot
  revision
  bounds / contentBounds / normalBounds / workArea
  mode
  displayId / scaleFactor
  eventOrigin: user-resize | user-move | panel-machine | display | mode

WindowBoundsEffect
  rendererEpoch / sequence / expectedWindowRevision
  targetBounds
  deltaWidthDip / xCompensationDip
  reason: panel-open | panel-close | panel-resize | reconcile

WindowBoundsResult
  accepted / revision / snapshot
  rejection?: stale | fixed-mode | outside-work-area | invalid
```

API：`getWindowLayoutSnapshot()`、`applyWindowBoundsEffect(effect)`、
`subscribeWindowLayoutSnapshot(listener)`。

### 7.2 权责

- renderer 的状态机是唯一**布局政策** owner，决定 columns、自动态和期望 bounds。
- 主进程是唯一**窗口机制** owner：核验 sender、revision、finite DIP、normal mode、
  target 完全位于 snapshot 的 workArea，然后原子执行。
- 主进程不自行 collapse panel、不修改用户 intent、不再写一套 720/960/980/350
  规则。环境已变化就拒绝并返回新 snapshot；renderer 把
  `WINDOW_BOUNDS_REJECTED` 投入同一纯函数重算 constrained plan。
- controller 给自己触发的 resize 标 `panel-machine`；状态机不把它当真实用户
  resize，彻底切断 resize feedback loop。

## 8. CSS 退化为纯渲染层

状态机输出并由 AppShell 设置的变量至少包括：

```text
--gx-sidebar-width
--gx-conversation-rail-width
--gx-shell-main-width
--gx-conversation-width
--gx-right-resizer-width
--gx-right-panel-width
--gx-thread-main-width
--gx-thread-log-resizer-width
--gx-thread-log-panel-width
```

同时输出 `data-sidebar-state`、`data-right-dock-state`、
`data-thread-logs-presentation`、`data-header-density`。CSS 可以用 grid 摆放这些
**已经算好的固定 track**，但不得再通过 `minmax(0, 1fr)`、flex shrink 或
`@media (max-width: ...)` 决定 panel 是否存在、main 剩多少、logs 是否 overlay。

conversation track 设 `width: var(--gx-conversation-width); flex: none`，不再另设
CSS min/max 参与裁决；350 由 plan 和 dev/test assertion 保证。现有 conversation
header 在 `@media (max-width: 720px)` 隐藏 logs label 的规则也应改读
`data-header-density`。`prefers-reduced-motion` 不是布局政策，可以保留。
其他 feature 内部的响应式 media query 不属于本次横向 app-shell 迁移。

完整 shell 几何继续放在 always-loaded owner stylesheet，并加 contract test：
owner import 不可移除；app-shell 横向结构不得新增尺寸 media query；渲染后的
每个 track 必须与 plan 精确相等。

## 9. 手动拖宽

拖宽也必须走同一事件机，不允许 controller 直接 `setState` 再从 DOM 猜宽度。

### 9.1 Normal + expandable

为保留现有“分隔线跟手”并维持 conversation 宽度，使用 resize session：

1. `PANEL_RESIZE_STARTED` 捕获 start bounds、start panel width、funding 和 workArea。
2. pointer move 只发送绝对 candidate width；每个 rAF 最多一个
   `PANEL_RESIZE_PREVIEWED`。状态机输出精确 columns 和绝对 bounds effect。
3. 左 panel 变宽 `d`：`deltaWidth=d, xCompensation=0`，右边缘随指针向右；
   右 panel 变宽 `d`：`deltaWidth=d, xCompensation=-d`，保持窗口右边缘与
   resizer 相对指针稳定。这个 x 向量记入该 panel funding，关闭时一并反转。
4. controller 只保留一个 in-flight native resize，后续 preview 覆盖旧 pending；
   主进程应用 absolute target，绝不排队累加 delta。
5. pointer up flush 最后一帧并 `COMMITTED`，accepted 后才持久化；cancel 反转到
   session start snapshot。

直接拖动不是动画，不走 370ms spring。程序化 resize origin 不触发断点。

### 9.2 触边 / fixed mode

一旦 preview target 会越出 workArea，状态机停止原生增宽并切到 constrained
reflow：panel 仍跟手，但 max candidate 由
`viewport - otherEffectiveTracks - MIN_CONVERSATION_WIDTH` 纯数学得出。空间连
panel min 都放不下时，按第 4 节降级或回滚，不让 CSS 把 main 压穿 350。

- sidebar 240–520；拖过 240 的 commit 解释为显式 close，重开恢复最后合法宽度。
- L2 保留 220–420。
- side tools min/default 320；logs 280–760。
- keyboard Arrow/Home/End 各自是一笔完整 transaction，复用相同求解器。
- hidden panel 改 preferred width 不改窗口；下次有效显示时以最终值统一求解。

## 10. 720 / 960 / 980 与用户偏好

- `BrowserWindow.minWidth=480` 固定不随 panel 增加；否则 720/960/980 会再次变
  成死代码。
- 真实用户拖窗更新 `responsiveBasisWidth`；panel-machine resize 不更新。
- global sidebar：无 L2 时 720/721；有 L2 时 980/981。
- right dock：960/961，hidden 不改 requested open。
- 用户缩窄触发 auto close/hide 后，状态机反转该 panel 已有 funding；用户拉宽
  恢复时若 workArea 可容纳，再扩窗恢复。若不可容纳，恢复为 constrained
  reflow 或继续 hidden。
- `garyx.sidebarCollapsed` 继续只表达用户宽屏偏好。compact 自动态、临时手动
  open 和容量降级绝不覆盖它。
- sidebar、rail、side-tools 的宽度偏好由外部 persistence adapter 喂给状态机；
  thread logs 继续用 `DesktopSettings.threadLogsPanelWidth`。只有 commit 后写入，
  本轮不要求把现有存储强行合成一处。

## 11. 屏幕边缘、多显示器、最大化与全屏

### 11.1 边缘规则

扩窗只在窗口未横向贴边时尝试保持当前 `x/y`、向右增加 width；**不为了容纳
panel 把整个窗口向左搬**。因此：

- 贴 workArea 左边或右边（2 DIP tolerance）：bounds 不变，constrained reflow。
- 位于中间但右侧余量不足：同样保持 bounds，constrained reflow。
- 只有位于中间且右侧完整容纳 target 时才原生扩窗。
- 关闭只反转真实 funding；当初没扩的 panel 不会让窗口凭空缩小。
- 右 panel 手动拖宽的 `xCompensation` 只服务直接操控的指针锚点；若它会越出
  workArea，同样转 reflow，不把窗口推到屏外。

这与当前 Codex 的贴边结果兼容于“固定窗口内 reflow”，同时在有真实余量时
实现 Garyx 增强。

### 11.2 多显示器

- 用 normal bounds 最大相交面积选择 display；并列时用窗口中心所在 display。
- target 必须完整落在该 display workArea；不跨屏找空间、不自动传送到邻屏。
- 用户 move-end 到另一 display 后更新 snapshot 并重算；如果此前 constrained
  panel 现在可完整资助，可在 move-end 后一次扩窗，恢复 conversation 打开前宽度。
- 监听 display add/remove/metrics/scale 变化；先由主进程给新 snapshot，再由
  状态机算 auto state。显示器拔除时 OS/main 负责把窗口放回有效 workArea，
  renderer 不猜坐标。

### 11.3 最大化与全屏

- 不调用 `setBounds`，不自动退出；全部使用 constrained reflow。
- 在 fixed mode 中 open/close 只改 columns，normal funding ledger 仍保留。
- 若 maximized/fullscreen 期间关闭了一个原先 funded panel，记录 deferred normal
  reconcile；退出 fixed mode 后再反转 normal funding。
- 若 fixed mode 中新开 panel，退出后若 normal workArea 可容纳，为它补 funding；
  否则继续 reflow。
- maximized/fullscreen bounds 永远不写成 `normalBaseBounds`。

## 12. 从现状迁移到一个状态机

### 12.1 散落逻辑归属表

| 当前逻辑 | 迁移后 |
| --- | --- |
| `useLayoutResizeController` 内多个 width/open/resizing `useState` | `HorizontalLayoutState.intent/resizeSession` |
| `window.resize` 中直接算 compact | `WINDOW_SNAPSHOT_CHANGED` 事件；纯函数按 origin 更新 basis |
| `ResizeObserver`、`currentConversationWidth()`、`currentThreadLayoutWidth()` | 从布局政策路径删除；宽度由 viewport 减各精确 track 得出 |
| `diagnostics-helpers` 基于 DOM width clamp | 接收 plan 给出的纯数值 budget，或并入状态机 constraint helper |
| `threadLogsDocked = isDockedSidePanel(...)` | `plan.presentations.threadLogs` |
| `AppShell.tsx` 的 `showConversationSideTools`、class/style 分支 | AppShell 只提交 intent，渲染 `plan.columns/presentations` |
| CSS `grid-template-columns` 的弹性剩余宽、720 media query | 固定 CSS vars + `data-*`，由 plan 决定 |
| renderer 根据 `window.innerWidth` 猜程序化/用户 resize | 主进程带 origin/revision 的 snapshot |
| `index.ts` 只有静态 BrowserWindow bounds | 主进程 window bounds executor + IPC，`minWidth:480` |

### 12.2 分阶段落地

1. 给现有 720/980、logs 540、各 resizer 范围补 characterization tests。
2. 在 `responsive-layout-model.ts` 定义 state/event/plan/effect，并先以
   `constrained-reflow` 模式复现现有布局；所有测试无 DOM。
3. 把 `useLayoutResizeController` 缩成 actor adapter：收外部 intent/snapshot、
   dispatch、执行 persistence/IPC effect；不再持有第二套布局 state。可以最终
   重命名为 `useHorizontalLayoutMachine`。
4. AppShell 与 ThreadPage 切到 plan 驱动的具体 columns；删除 policy 用的
   ResizeObserver/DOM measurement，迁移 app-shell CSS responsive 决策。
5. shared/preload/main 加 revisioned bounds IPC；先接 sidebar，再接 L2、side
   tools、logs 单交易替换。
6. 接 responsive auto hide/restore、fixed-mode deferred reconcile、多显示器。
7. 最后接 resize session 的 rAF coalescing；移除旧 refs、direct setters 和重复
   clamp helper。

迁移期可以让新 machine 以 dev-only shadow mode 对相同事件计算 plan 并记录差异，
但不能长期保留新旧两个 controller 同时写布局。

## 13. 影响面

- `src/main/index.ts`：`minWidth:480`；挂载 window bounds executor 与 display /
  mode / user-resize 事件源。
- 新的主进程机制模块，例如 `src/main/window-layout-controller.ts`；只校验与执行。
- `src/shared/contracts/`、`src/preload/index.ts`、`GaryxDesktopApi`：snapshot /
  effect/result/subscribe。
- `app-shell/responsive-layout-model.ts`：升级为唯一纯函数 state machine。
- `app-shell/useLayoutResizeController.ts`：迁移成薄 actor，最终可改名。
- `app-shell/AppShell.tsx`、`components/ThreadPage.tsx`：intent 输入与 plan 消费。
- `app-shell/diagnostics-helpers.ts`：删除 DOM 反馈 clamp。
- always-loaded `styles/gateway-setup.css`、`workspace-rails.css`、
  `conversation.css`、`sidebar.css`：只渲染固定输出；去掉横向 policy。
- 对应 model、controller、preload/main contract 与 packaged Electron 测试。

不影响 gateway/router/thread render_state、移动端或 TestFlight。

## 14. 取舍

- **行为高于当前 Codex**：Codex 当前纯 reflow；Garyx 只用它的数值和退化模式
  做锚，正常态按用户要求扩窗。
- **受限时不搬窗口**：贴右/空间不足不做左移 clamp，避免窗口跳动；代价是
  conversation 会缩到 350，panel 再多时会自动隐藏或被拒绝。
- **JS 是政策，CSS 是画笔**：类型、事件和 IPC 增多，但所有组合都能 headless
  穷举，消除 CSS/DOM/resize observer 反馈环。
- **按钮开合原子 bounds，surface spring**：窗口没有 tween，换取无竞态、可逆；
  约 370ms 只用于 panel surface。
- **拖宽走 rAF absolute bounds**：比 pointer-up 才扩窗复杂，但能保持分隔线
  跟手、conversation 不挤压；coalescing 与 revision 防止 IPC 堆积。
- **350 是硬保护，540 是 logs 舒适值**：不再混为一个“main min”。

## 15. 实现与验收清单

### 15.1 实现步骤

1. 固化常量与纯状态机事件/输出 schema。
2. 完成状态机 headless tests 和 shadow comparison。
3. 完成 CSS fixed-output 迁移及 owner stylesheet contract。
4. 完成 main/preload IPC 与 origin/revision。
5. `minWidth` 改 480；禁止动态改成 `480 + panels`。
6. 接四类 panel 的 open/close funding 和互斥替换。
7. 接 720/721、960/961、980/981 与 350 protection。
8. 接 edge/fixed/multi-display constrained reflow。
9. 接 resize session、240 drag-collapse 和键盘操作。
10. packaged app + CDP/CGWindow 复测 bounds、DOM columns、动画和状态恢复。

### 15.2 无 UI 状态机矩阵

- 宽度：480、600、720/721、960/961、980/981、1116、1280、1480、1920。
- panel：none、S、L2、S+L2、right tools、docked/overlay logs、S+L2+right、
  tools↔logs、capsule-only。
- window mode：normal expandable、右侧空间不足、贴左、贴右、maximized、
  fullscreen。
- event sequence：hydrate、open/close、双击、route replace、user resize、
  panel-machine resize、move display、display removal、mode enter/exit、stale ack。
- resizer：sidebar 240/520/<240 collapse、rail 220/420、tools 320、logs 280/760、
  right x compensation、cancel/commit/persistence。
- 每个 plan 断言 columns 求和、conversation `>=350`、intent 与 effective state
  分离、非 user resize 不改 responsive basis。

### 15.3 Electron / packaged 验收

- 正常可扩：bounds width delta 等于 funded track 净差；left/L2 打开后内容整体
  右移且 conversation width 差 `<=1px`；right 打开后 conversation rect 不变。
- 贴右/超 workArea/max/fullscreen：bounds 零变化，conversation 在窗口内 reflow
  且不低于 350；需要时 right hidden/sidebar collapsed/logs overlay。
- 精确断点：720/721、960/961、980/981；programmatic resize 不触发。
- native open/close 每 transaction 只一次 atomic `setBounds`；surface spring 约
  370ms，Reduce Motion 立即。
- 拖宽每 rAF 最多一个 absolute effect、无队列增长、resizer 不脱离指针、取消
  可逆、commit 后偏好与 ARIA 值一致。
- renderer reload/hot reload 不二次扩窗；最大化退出、第二显示器拔出后 normal
  base/funding 可恢复。
- `npm run build:ui`、focused unit/smoke、`npm run dist:dir` 后重启 packaged app，
  再 attach 新 renderer 实测，不测试旧 bundle。
