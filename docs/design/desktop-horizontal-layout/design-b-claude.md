# Garyx 桌面「横向面板开合 = 窗口宽度扩展」设计(claude 版,rev2)

任务:#TASK-2168(纯设计,双版并发之一)
日期:2026-07-12(rev2:采纳方案 A 后,按用户新增架构要求修订)
实测对象:ChatGPT desktop(Codex)26.707.51957 / Codex Framework 150.0.7871.115,macOS,1920×1080 显示器(attach 前已验证 title=Codex、url=app://-/index.html,且未连接其他应用调试会话)

rev2 变更:用户拍板方案 A(显式开合=窗口扩展、约束下降级挤压),并新增核心架构要求——**整个横向布局变成 JS 纯函数状态机的计算题,CSS 只消费状态机输出,不承载任何 responsive 决策**。本版围绕该要求重构 §2–§5。

---

## 0. TL;DR

- **实测更正一个前提**:当前版本 Codex **没有**「面板出现 → 窗口向外扩」行为。左边栏、右侧面板在任何窗口宽度(1094 / 600 / 最大化 1920,窗口 bounds 16ms 采样)开合都是**窗口不动、内部 reflow 挤压**;asar 逆向确认主进程 7 处 `setBounds` 无一与面板相关。用户要的行为是 Codex 之上的增强,Codex 实测数值做锚:最小窗口 480×600、边栏启动收起断点 ≤720(与 Garyx `SINGLE_RAIL_COMPACT_WIDTH=720` 精确吻合)、右面板 auto-hide 阈值 960/961、Codex 主区兜底 350px、「自动行为不动窗口」原则。
- **架构(rev2 核心)**:新建单一纯函数布局状态机 `horizontal-layout-model.ts`(由 `responsive-layout-model.ts` 升级),分两层:
  - `reduce(state, event) → { state', effect? }` —— 转移函数,吸收面板开合意图、拖宽、viewport/窗口环境变化、main 进程回执;**窗口 bounds 变更请求(delta + x 补偿)只能从 USER_TOGGLE 事件分支产生**(「responsive 不动窗」成为类型级保证,不是约定)。
  - `project(state) → LayoutFrame` —— 幂等投影,输出每栏精确 px、每面板呈现态(expanded/collapsed/docked/overlay/hidden)、CSS 变量表。
- **CSS 降级为渲染器**:grid 列全部消费状态机输出的 px 变量;删除 `minmax()`/`1fr` 份额协商、容器查询断点、CSS 层面的挤压语义。保留纯视觉(动画、材质、阴影、overlay 皮肤)。
- 行为规则不变(方案 A 已拍板):统一右扩、best-effort 扩窗、贴屏/最大化/全屏降级挤压、显式动作才动窗;`minWidth` 1180→**560**。

---

## 1. Codex 实测数据(设计准绳)

### 1.1 测量方法(可复现)

| 手段 | 用途 |
|---|---|
| 经目标校验的 CDP `Runtime.evaluate` | DOM 布局、面板宽度、按钮驱动开合、rAF 动画采样 |
| Swift `CGWindowListCopyWindowInfo` 16ms 轮询 | 原生窗口 bounds 时间序列(面板开合期间) |
| `@electron/asar` 解包已安装客户端的 `app.asar` 并逆向 `main-BHxSB3aK.js` | 窗口管理源码级确认(min size、持久化、setBounds 全量清点) |
| 改 Codex desktop state file 的 `electron-main-window-bounds` + 重启 | 精确控制原生窗口宽度(合成鼠标事件对该 app 窗口无效,AX position/size 属性不支持) |
| CDP `Emulation.setDeviceMetricsOverride` 宽度扫描 | 断点二分定位 |

### 1.2 窗口硬数据

| 项 | 实测值 | 证据 |
|---|---|---|
| 主窗口默认尺寸 | **1280×820** | bundle:`createWindow` 默认 `width:r=1280,height:i=820` |
| 主窗口最小尺寸 | **480×600** | bundle:`getPrimaryMinimumSize(){return{width:480,height:600}}`;live:持久化写 300×500 → 恢复后 innerWidth/innerHeight 精确 480×600(两次独立复现) |
| bounds 持久化 | `{x,y,width,height,isMaximized}`,quit 时写、启动时恢复 | `electron-main-window-bounds`;恢复时 `clampPrimaryWindowBounds`(min clamp)+ `isWithinDisplayWorkArea` 校验,不在任何屏内 → 丢弃回默认居中 |
| isMaximized 恢复 | 恢复为占满 workArea(实测 CG 1882×937 @19,39) | live 实测 |
| **面板开合 vs 窗口 bounds** | **任何面板、任何宽度:窗口 bounds 零变化** | winpoll 16ms 采样:宽 1094 开/关左边栏、开/关右面板;宽 600 同套;最大化 1920 开/关边栏 — 全部 `changes` 为空 |
| 主进程 setBounds 全量 | 7 处:avatar overlay×2、截屏 overlay、恢复窗口、录音窗定位、hotkey 窗布局、min-size 同步 | asar 逆向逐处核对,无面板路径 |

### 1.3 左边栏(threads sidebar)

| 项 | 实测值 |
|---|---|
| 默认宽 | 275 |
| 拖宽范围 | **240 – 520**(与 Garyx sidebar max 520 相同;Garyx min 245 vs Codex 240) |
| 拖到 <240 | 直接收起(drag-to-collapse);重开恢复上次宽度 |
| 宽度与开合状态 | 均持久化,跨重启恢复 |
| 开合动画 | **JS spring(computed `transition: all` 无 duration,非 CSS transition),≈370ms settle**,先加速后长减速尾(rAF 逐帧序列:275→0 用时 ~370ms) |
| resize 时自动收起断点 | **无**。Emulation 900/520/480 及真实窗口 600/480:边栏保持 in-flow 挤压主区,主区可被压到 205 甚至 **0px**(480 窗 + 520 边栏实测 main=0) |
| **启动时初始态断点** | **viewport ≤720 → 初始收起;≥721 → 恢复持久化 open 态**(真实窗口重启二分:720 关 / 721 开 / 740 开 / 900 开)|

> 721 断点与 Garyx `SINGLE_RAIL_COMPACT_WIDTH = 720`(`viewportWidth <= 720` 判 compact)精确吻合 —— f0dc27e48「align responsive thread layout with Codex」的对齐来源这次被实测坐实。

### 1.4 右侧面板(side tools 对应物)

| 项 | 实测值 |
|---|---|
| 默认宽 | 320,in-flow(z-index 41,空间不足时叠绘在主区上方) |
| 拖宽范围 | min 320(<320 拖拽即收起);max = viewport − 边栏 − 中栏保护宽(1200 视口实测最大 608) |
| **resize 自动隐藏** | **viewport ≤960 自动隐藏;≥961 自动恢复**(hidden ≠ closed,状态保留;二分:960 隐 / 961 显) |
| 961 阈值处中栏宽 | **恰好 350px** → Codex 的「主消息区保护值」= **350**,通过隐藏右面板实现,而不是扩窗 |
| 窄窗手动打开 | 允许;z-41 叠层,可把中栏压到 ~0(600 窗实测中栏剩 20px) |

### 1.5 主消息区

- **无硬 min-width**。唯一保护是右面板的 961 auto-hide(保 350);左边栏挤压完全无保护(main 0px 可达,属明显未打磨的退化态,**不作对齐对象**)。
- 会话内容列(composer-adjacent)实测 ~790px 上限。

### 1.6 对任务书四个问题的 Codex 答案

| 任务书要量的 | Codex 实测答案 |
|---|---|
| 最小窗口宽 | 480(min 480×600) |
| 主消息区最小宽 | 350(右面板 auto-hide 触发线);左边栏路径无保护 |
| 侧栏自动收起断点 | resize 无断点;**启动初始态断点 ≤720**;右面板 resize 断点 ≤960 |
| 各面板开合窗口 bounds 变化/动画 | **bounds 恒不变**;面板 reflow 用 ~370ms JS spring |
| 贴屏幕边缘开面板 | 无此场景(窗口从不因面板移动/变宽) |
| 最大化/全屏 | 纯 reflow(实测最大化 + 静态代码:min-size 同步显式跳过 maximized/fullscreen 窗口) |

---

## 2. 目标行为规范(normative,方案 A 已拍板)

### 2.1 行为规则

> R1:四个横向面板 —— 左主 sidebar、第二栏会话列表 rail、右侧 side tools dock、thread logs 面板 —— 由**用户显式动作**打开时:窗口**左缘锚定、右缘向外扩 panelW**,已有内容整体平移(左侧面板)或不动(右侧面板),不被挤压;显式关闭时窗口按台账缩回。
>
> R2:**扩窗是 best-effort**。按屏幕 workArea 计算实际可扩量 `appliedDelta ∈ [0, panelW]`;不足部分降级为窗口内挤压(由状态机解算,见 §3)。
>
> R3:**responsive 自动开合永不动窗口**。compact 断点自动收起/恢复 sidebar、宽度不足时 side-tools/thread-logs 的 docked→overlay 降级,一律窗口内重排。(Codex 同款原则;rev2 中升级为状态机的类型级保证,见 3.3。)
>
> R4:最大化、全屏窗口:面板开合一律窗口内重排,不动 bounds(Codex 实测行为)。
>
> R5(rev2 新增):**「谁多宽、谁收起、窗口怎么变」100% 由 JS 纯函数状态机决定**。CSS 不再承载 responsive 决策:无 media/container query 断点、无 `minmax()`/`fr` 份额协商、无 CSS min/max-width 交互;CSS 只消费状态机输出的具体 px 与呈现态 class。

用户原话逐条映射:「侧边栏向外/向右侧展开」「recent 消息列表也是向右推」→ 左侧面板打开时窗口右缘外扩、主内容右移;右侧 dock 打开时右缘外扩、主内容不动。统一为**右扩模型**。「这玩意变成 js 的计算题…状态机的设计,而不是样式」→ R5。

### 2.2 各面板参数(全部沿用现值,不新造)

| 面板 | 宽度来源 | min–max | 显式开合入口(AppShell) |
|---|---|---|---|
| 左 sidebar | `sidebarWidth`,默认 245 | 245–520 | `toggleSidebarCollapsed` 非 compact 分支(localStorage `garyx.sidebarCollapsed`) |
| 第二栏 rail | `railWidth`,默认 258 | 220–420 | `botConversationGroupId` / `workspaceConversationPath` / `recentThreadsRailOpen` 的 set/clear |
| side tools dock | `sideToolsPanelWidth`,默认 320 | 320–1180 | `onToggleInspector` + capsule tab 首开/全关 |
| thread logs | `threadLogsPanelWidth`(唯一持久化到 settings),默认 360 | 280–760 | `onToggleThreadLogs` |

### 2.3 主消息区最小宽:两级模型

| 级 | 值 | 语义 | 出处 |
|---|---|---|---|
| 舒适线 `MAIN_MIN` | **540** | 扩窗不足时,挤压最多到此;solver 的面板 clamp 输入 | Garyx 现值 `SIDE_PANEL_MIN_MAIN_WIDTH`(> Codex,不缩水) |
| 绝对线 `MAIN_ABS_MIN` | **350** | 低于此 → 侧面板转 overlay 的下限对齐值 | Codex 实测(961 阈值处中栏宽) |

推荐 v1 只用 540;350 作为 Codex 对齐参考入档。

### 2.4 窗口 minWidth

```
minWidth = MAIN_MIN(540) + SIDE_PANEL_RESIZER_WIDTH(10) + 余量(10) = 560
```

1180 → **560**(`src/main/index.ts:446`)。Codex 是 480,但 480 意味着主区 <540 常态化;若要完全对齐 480 须连带降 MAIN_MIN 至 ~470,不推荐。720(单栏)/980(双栏)断点全部落入可达区间。minHeight 760 本任务不动。

---

## 3. 布局状态机设计(rev2 核心)

### 3.1 模块与分层

新模块 `src/renderer/src/app-shell/horizontal-layout-model.ts`(`responsive-layout-model.ts` 升级并入),**零依赖纯函数**(不 import React/DOM/Electron),shared 侧可复用其中窗口几何函数给 main 进程。三个导出:

```ts
reduce(state: LayoutState, event: LayoutEvent): { state: LayoutState; effect: WindowAdjustRequest | null }
project(state: LayoutState): LayoutFrame        // 幂等投影,任何 state 随时可算
initialState(persisted: PersistedLayoutInputs, env: LayoutEnv): LayoutState
```

分两层的原因:**每栏宽度**是状态的幂等投影(给定 state 恒定),而**窗口 bounds 变更**是事件沿(只在显式开合的转移瞬间产生一次)。合在一个 `reduce` 里返回 `effect`,投影单独 `project`,两者都可表驱动单测。

### 3.2 State / Event / Frame 契约

```ts
type PanelId = "sidebar" | "rail" | "sideTools" | "threadLogs";

type LayoutState = {
  intents: {                            // 用户意图(显式态,可持久化)
    sidebarOpen: boolean;               // = !garyx.sidebarCollapsed
    railOpen: boolean;                  // 三种第二栏归一(bot/workspace/recent)
    sideToolsOpen: boolean;             // = inspectorOpen || openCapsuleTabs>0
    threadLogsOpen: boolean;
    compactSidebarOpen: boolean;        // compact 视口下的临时展开(瞬态)
  };
  widths: {                             // 用户拖宽持久值(未 clamp 的原始意图值)
    sidebar: number; rail: number; sideTools: number; threadLogs: number;
    sideToolsCustomized: boolean;       // 吸收现 sideToolsPanelWidthCustomizedRef
  };
  env: {
    viewportWidth: number;              // renderer 视口宽(DIP)
    windowMode: "normal" | "maximized" | "fullscreen";
    workAreaWidth: number;              // 当前屏 workArea 宽(main 推送)
    windowRightGap: number;             // 窗口右缘到 workArea 右界的距离(main 推送)
    windowLeftGap: number;              // 左缘到 workArea 左界距离
  };
  ledger: Partial<Record<PanelId, { appliedDelta: number; xShift: number }>>;
  pending: PanelId | null;              // 在途窗口请求(串行化)
};

type LayoutEvent =
  | { type: "USER_TOGGLE_PANEL"; panel: PanelId; open: boolean }        // 唯一产 effect 的事件
  | { type: "USER_DRAG_WIDTH"; panel: PanelId; width: number }          // 高频;solver 内 clamp
  | { type: "USER_TOGGLE_COMPACT_SIDEBAR" }                             // compact 临时展开,无 effect
  | { type: "VIEWPORT_RESIZED"; width: number }
  | { type: "WINDOW_ENV_CHANGED"; env: Partial<LayoutState["env"]> }    // mode/workArea/gap
  | { type: "ADJUST_APPLIED"; panel: PanelId; appliedDelta: number; xShift: number }  // main 回执
  | { type: "ADJUST_FAILED"; panel: PanelId };                          // IPC 失败→纯挤压降级

type WindowAdjustRequest = {
  panel: PanelId;
  deltaWidth: number;                   // + 开 / − 关(关取 ledger 值)
  reason: "panel-open" | "panel-close";
};

type LayoutFrame = {
  columns: {                            // 精确整数 px,不变量 I1:总和 = viewportWidth
    sidebar: number; rail: number; main: number;
    sideTools: number; threadLogs: number;
    resizerSideTools: number; resizerThreadLogs: number;   // 0 或 10
  };
  presentation: {
    sidebar: "expanded" | "collapsed" | "compact-overlay";
    rail: "open" | "closed";
    sideTools: "docked" | "overlay" | "closed";
    threadLogs: "docked" | "overlay" | "closed";
    taskTreeDocked: boolean;            // 吸收 1088 容器查询
    compactViewport: boolean;           // 720/980 断点结果
  };
  cssVars: Record<`--layout-${string}`, string>;   // 适配层直接铺到 :root / 容器
};
```

### 3.3 转移规则与解算器

**`reduce` 各事件分支**:

| 事件 | state 变化 | effect |
|---|---|---|
| `USER_TOGGLE_PANEL` open | `intents[p]=true` | normal 窗口且非 compact 强制:`{deltaWidth:+clampedPanelW, reason:"panel-open"}`;maximized/fullscreen/在途 pending → null(纯挤压) |
| `USER_TOGGLE_PANEL` close | `intents[p]=false` | ledger 有账 → `{deltaWidth:−applied, reason:"panel-close"}` 并清账;无账 → null |
| `USER_DRAG_WIDTH` | `widths[p]=w`(drag-to-collapse:低于 min−阈值 → 转为 close 意图,同 Codex) | null |
| `VIEWPORT_RESIZED` / `WINDOW_ENV_CHANGED` | env 更新;进入 maximized 时清空 ledger | **恒 null**(R3) |
| `ADJUST_APPLIED` | `ledger[p]={applied,xShift}`;`pending=null` | null |
| `ADJUST_FAILED` | `pending=null` | null |

> **R3 的类型级保证**:`effect` 的构造只出现在 `USER_TOGGLE_PANEL` 分支。断点自动收起、overlay 降级、viewport 变化在代码结构上**不可能**产生窗口请求 —— 死循环被构造排除,而非靠约定。单测直接断言:对全事件空间中非 USER_TOGGLE 事件,effect 恒为 null。

**`project` 解算器(预算分配,单向无协商)**:

```
预算 B = viewportWidth
1. compactViewport = B ≤ (railOpen ? 980 : 720)                    // 现有断点原样
2. sidebar 列:
   compactViewport → 0(compact-overlay 由 compactSidebarOpen 决定,浮层不占列)
   否则 sidebarOpen → clamp(widths.sidebar, 245, 520),再 clamp 至 B − MAIN_MIN 预算
3. rail 列:railOpen → clamp(widths.rail, 220, 420),预算同上递减
4. sideTools:open →
   docked 宽 = clamp(widths.sideTools 或默认策略, 320, B − 已分配 − MAIN_MIN − 10)
   若解不出 ≥320 → presentation="overlay"(列宽 0,浮层宽独立解算)      // 吸收 isDockedSidePanel
5. threadLogs 同 4(280–760;与 sideTools 互斥由 AppShell intent 保证)
6. main = B − Σ其余列(吃余数,≥MAIN_MIN 由 2–5 的预算扣减保证;
   极端不足(B < minWidth 理论不可达)→ main 兜底 0 并全面板 overlay)
7. taskTreeDocked = main ≥ 1088                                     // 吸收容器查询
```

不变量(全部入单测):
- **I1** Σcolumns = viewportWidth(整数,main 吃舍入余数);
- **I2** main ≥ MAIN_MIN,否则必有面板已 overlay/closed;
- **I3** 非 USER_TOGGLE 事件 effect ≡ null;
- **I4** ledger[p] 存在 ⇒ intents[p].open(关闭/最大化即清);
- **I5** effect.deltaWidth 经 main 侧 clamp 后目标宽 ∈ [minWidth, workArea.width]。

### 3.4 窗口几何函数(shared,main 进程消费)

```ts
// src/shared/contracts/window-expansion.ts —— main 与单测共用
computeWindowExpansion({ bounds, workArea, delta, minWidth }):
  { bounds: Rectangle; appliedDelta: number; xShift: number }
```

语义:右缘优先扩;右侧不足左移 x 补偿(至 workArea.x);仍不足打折;缩回对称并归还 xShift;clamp [minWidth, workArea.width]。多显示器:workArea = `screen.getDisplayMatching(win.getBounds()).workArea`,不跨屏。

### 3.5 IPC 契约(不变,rev1 定稿)

```ts
// channel: "garyx:adjust-window-width"   (preload: garyxDesktop.adjustWindowWidth)
type AdjustWindowWidthInput  = { deltaWidth: number; reason: "panel-open"|"panel-close"; panelId: PanelId };
type AdjustWindowWidthResult = { appliedDelta: number; xShift: number; windowState: "normal"|"maximized"|"fullscreen" };
```

main 端:fullscreen/maximized 短路 applied=0;`computeWindowExpansion` 算 bounds;`setBounds` 一次到位无动画;每窗口 promise 串行队列。main 同时在窗口 `resize`/`maximize`/`unmaximize`/`enter/leave-full-screen`/`display-metrics-changed` 时推送 `WINDOW_ENV_CHANGED`(现有订阅型通道模式:`garyx:window-env` + subscribe)。

### 3.6 适配层(useLayoutResizeController 的去逻辑化)

`useLayoutResizeController` 重写为**薄事件源 + 渲染器**,不再含任何决策:

- **进**:DOM/IPC 事件 → `dispatch(LayoutEvent)`:window resize → VIEWPORT_RESIZED;pointer 拖宽 → USER_DRAG_WIDTH(高频路径保留现有 rAF 直写通道,但写的是 `project` 重算后的 cssVars,不是裸宽度);四个开合入口 → USER_TOGGLE_PANEL;main 推送 → WINDOW_ENV_CHANGED / ADJUST_APPLIED。
- **出**:`project(state).cssVars` 铺到容器(`useLayoutEffect` + 拖拽/live-resize 期间 rAF 直写 DOM,先例:现 railWidth rAF 直写);`presentation` 映射为容器 class/attr;`effect` 转 `garyxDesktop.adjustWindowWidth` 调用,回执 dispatch ADJUST_APPLIED/FAILED。
- 持久化副作用留在适配层:`garyx.sidebarCollapsed`(localStorage)、`threadLogsPanelWidth`(saveSettings)——写入时机 = 对应事件 reduce 之后,值取自 state。

时序与抖动(维持 rev1 决策):同 tick 先 reduce+project 上屏(面板列即时出现,内容沿用 170ms enter 动画),effect 异步发 IPC,窗口 1–2 帧后变宽;IPC 失败 = 纯挤压模型,失败安全。**窗口瞬时、无窗口动画**(拒绝跨进程动画同步;Codex 的 370ms spring 是窗口内动画,不构成窗口动画先例)。

### 3.7 CSS 的角色:决策清零,视觉保留

**删除/替换(决策类)**:

| 现状 CSS 决策 | 位置 | 替换 |
|---|---|---|
| `grid-template-columns: var(--spacing-token-sidebar) minmax(0,1fr)` 及三栏变体(`fr` 份额协商) | `gateway-setup.css:305-322` | `var(--layout-col-sidebar) var(--layout-col-rail) var(--layout-col-main)` 全 px |
| `.conversation.with-side-tools` 的 `minmax(320px, var(--side-tools-panel-width))`(CSS 层 min clamp) | `workspace-rails.css:1345-1373` | `var(--layout-col-side-tools)` 精确 px(clamp 已在 solver) |
| thread-layout docked 三列 `minmax(280px, var(--thread-log-panel-width))` / overlay 单列切换 | `conversation.css:927-980` | 列宽全 px;docked/overlay 由 `presentation.threadLogs` class 驱动 |
| `@container thread-task-tree (min-width: 1088px)` | `gateway-panels.css:182` | `frame.presentation.taskTreeDocked` → class |
| sidebar 收起 `visibility:hidden` + 0 宽的隐式配对 | `sidebar.css:743-756` | class 由 `presentation.sidebar` 驱动,宽度由 `--layout-col-sidebar:0` |
| `.thread-main { min-width: 0 }` 等参与挤压的 min/max | `conversation.css:992` 等 | 删除决策义;保留 `min-width:0` 仅作为防溢出的渲染卫生(不再是挤压参与者) |

**保留(纯视觉)**:`side-tools-panel-enter` 170ms 进场动画、文件浏览器内部折叠 transition、vibrancy/材质/背景、边框圆角阴影、resizer hover/active 样式、overlay 浮层的定位皮肤与 z-index(浮层「是否存在」由状态机,长相由 CSS)。

**main 列的 px 化说明**:目标态 main 列 = 状态机输出的精确 px(I1 保证总和恒等视口宽)。live resize 撕裂(窗口已变宽、变量未更新的一帧)用「resize 事件同帧直写 cssVars」消除(resize 派发先于该帧 paint;现有 rail 拖宽 rAF 直写是同款先例)。迁移期(Phase 3 前)允许 main 列暂用 `minmax(0,1fr)` 作为**纯减法承接器**——其值恒等于 solver 的 main 输出(其余列全 px、main 无 min/max 时 `1fr` 无协商语义),决策仍 100% 在 JS;Phase 3 收口为 px。

### 3.8 边界场景决策表(维持 rev1,机内化)

| 场景 | 决策 | 在状态机中的位置 |
|---|---|---|
| 右缘贴 workArea 右界开面板 | 右扩 0 → 左移 x 补偿 → 仍不足打折,余量挤压 | `computeWindowExpansion`(3.4) |
| 窗口已占满 workArea 宽 | applied=0,纯挤压(断点可能触发 compact,main clamp 540,再不足 overlay) | solver 降级链(3.3) |
| 最大化 / 全屏 | 不动 bounds,纯重排 | reduce 的 USER_TOGGLE 分支短路 + main 短路(双保险) |
| 多显示器 | clamp 当前屏 workArea,不跨屏;跨屏摆放取重叠最大屏 | 3.4 |
| 显式关面板缩窗跨 720/980 断点 | sidebar responsive 收起,不级联缩窗 | I3(类型级) |
| 开面板后用户手动缩窗再关面板 | 缩回 min(ledger 值, 当前宽−minWidth) | ledger + main clamp |
| 手动拖面板宽 | 只重分配窗口内空间,不动窗口(Codex 同,实测) | USER_DRAG_WIDTH 无 effect |
| Dock/菜单栏 workArea 变化 | 不追赶;下次请求按新 workArea clamp | WINDOW_ENV_CHANGED 无 effect |
| 面板开着重启 | v1 窗口回 1480×940(现状);P2 建议 Codex 式 bounds 持久化 | initialState |
| IPC 失败/超时 | ADJUST_FAILED → 无账,行为=纯挤压 | reduce |

---

## 4. 迁移路径(现有逻辑 → 状态机)

### Phase 0 — Characterization(先钉现状)
对以下现有函数/行为建表驱动测试,作为 Phase 1 的守恒 oracle:`isCompactSidebarViewport`、`resolveSidebarCollapsed`、`clampThreadLogsPanelWidth`、`clampSideToolsPanelWidth`、`defaultSideToolsPanelWidth`、`isDockedSidePanel`、`isDockedTaskTree`、四组拖宽 min/max、720/980 断点行为。输入输出矩阵直接从现实现采样生成。

### Phase 1 — 建状态机(纯模块,无接线)
`horizontal-layout-model.ts`:reduce/project/initialState + 3.4 窗口几何函数。收敛映射:

| 现有散落逻辑 | 归宿 |
|---|---|
| `responsive-layout-model.ts` 全部(720/980/1088/540/10 常量与四函数) | solver 规则 1/4/7 + 常量区 |
| `diagnostics-helpers.ts` 三个 clamp/default 函数 | solver 规则 4/5(文件保留仅转发,或直接删并改引用) |
| `useLayoutResizeController` 内联的 `Math.max(245,Math.min(520,…))`、`Math.max(220,Math.min(420,…))` | USER_DRAG_WIDTH 分支 clamp |
| `resolveSidebarCollapsed` 的 compact/user 双轨 | presentation.sidebar 三值 |
| `sideToolsPanelWidthCustomizedRef` | state.widths.sideToolsCustomized |
| resize 监听里的重夹逻辑(`handleResize`) | VIEWPORT_RESIZED → project(自动成立) |
| (新)扩窗台账/回执 | ledger + ADJUST_* 事件 |

单测:Phase 0 矩阵回放(行为守恒)+ 不变量 I1–I5 + 扩窗全链(toggle→effect→applied→ledger→close→缩回)+ 断点/贴屏/最大化/失败降级场景表。

### Phase 2 — 适配层接线(行为守恒)
`useLayoutResizeController` 换芯为 3.6 适配层;CSS 未动(此时 cssVars 写入现有变量名 `--spacing-token-sidebar`/`--spacing-token-rail`/`--side-tools-panel-width`/`--thread-log-panel-width`,px 值与旧逻辑逐像素一致)。验收:Phase 0 oracle 全绿 + CDP 实测四面板开合/拖宽/断点像素对照。

### Phase 3 — CSS 决策清零
按 3.7 表替换 grid/minmax/container query 为 `--layout-*` px 变量与 presentation class;main 列收口为 px。验收:双侧渲染像素对照(oracle:改前后同输入截图/布局树 byte-diff 思路),grep 断言 styles/ 下无 `minmax(`、无布局向 media/container query(白名单纯视觉项)。

### Phase 4 — 窗口扩展启用
main:minWidth 560 + `garyx:adjust-window-width` handler + `garyx:window-env` 推送;preload 暴露;适配层放行 effect→IPC。此前 Phase 1–3 中 effect 通道已存在但适配层不发送(feature 开关),使窗口行为可独立回滚。

> Phase 1–3 是行为守恒重构(可独立合入、独立验证),Phase 4 是行为增量。顺序不可颠倒:先有单测得住的状态机,再让它开始指挥窗口。

---

## 5. 影响面与实现步骤清单

| 文件 | 改动 |
|---|---|
| `app-shell/horizontal-layout-model.ts`(新,吸收 `responsive-layout-model.ts`) | 状态机全量 + 单测 |
| `src/shared/contracts/window-expansion.ts`(新) | `computeWindowExpansion` + IPC 类型 + 单测 |
| `app-shell/diagnostics-helpers.ts` | clamp 函数迁出/转发 |
| `app-shell/useLayoutResizeController.ts` | 换芯为薄适配层(事件源+cssVars 渲染+持久化副作用) |
| `AppShell.tsx` | 四开合入口 dispatch 化;presentation class 接线 |
| `src/main/index.ts` | minWidth 560;adjust-window-width handler(串行队列);window-env 推送 |
| `src/preload/index.ts` | 两通道暴露 |
| `styles/gateway-setup.css` / `workspace-rails.css` / `conversation.css` / `gateway-panels.css` / `sidebar.css` / `base.css` | 3.7 表:决策删除,变量/class 消费 |

步骤:
1. Phase 0 characterization 矩阵。
2. Phase 1 状态机 + I1–I5 + 全链单测(含:非 USER_TOGGLE 事件 effect 恒 null 的全事件空间断言)。
3. Phase 2 适配层换芯,CDP 像素对照。
4. Phase 3 CSS 清零 + grep 合约断言(styles/ 无 minmax/布局断点)。
5. Phase 4 main/preload/IPC + feature 放行;characterization:显式关面板跨断点不级联、compact 自动收起零 IPC、部分扩展关闭只缩 ledger 值。
6. 手动回归:四面板拖宽、`garyx.sidebarCollapsed`、compact 临时展开、thread-logs 宽度持久化。
7. packaged 验证(dist:dir + 重启 app + 重新 attach 新 renderer):bounds 序列对照 §3.8 决策表。
8. P2(独立提案):窗口 bounds+isMaximized 持久化恢复(Codex 式)。

## 6. 已知风险与取舍

- 1–2 帧扩窗中间态:接受(内容 enter 动画掩盖);后备手段:开面板一帧内锁 main 列宽(不进 v1)。
- main 列 px 化的 live-resize 同帧写依赖 resize 事件时序:已有 rail rAF 直写先例;迁移期保留 1fr 减法承接兜底(3.7)。
- 状态机吸收面大(useLayoutResizeController 563 行大部分逻辑):以 Phase 0 oracle + 分 Phase 合入控风险;Phase 1–3 不改行为。
- 缩回时窗口右缘位置随台账回退,若用户中途手动移过窗:按 ledger 额缩、clamp 到 minWidth,不追求完美复位(决策表)。
- CI 无法验证真实窗口 bounds(步骤 7 真机 packaged 检查)。

## 附:测量会话原始记录索引(未入库)

- `poll_toggle.csv` / `poll_right.csv` / `poll_narrow_open.csv` / `poll_narrow_right.csv` / `poll_max.csv`:各场景窗口 bounds 16ms 采样(全部无变化)
- `codex_cdp.py`(CDP 工具:eval/emulate/watch)、`winpoll.swift` / `winlist.swift`(CGWindowList)
- app.asar 解包目录(main-BHxSB3aK.js 为主进程 bundle)
- rAF 动画序列、断点二分记录见任务线程(#TASK-2168)
