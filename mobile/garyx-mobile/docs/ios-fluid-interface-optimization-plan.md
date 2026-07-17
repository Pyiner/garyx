# Garyx iOS 流体交互优化计划

依据：Apple *Designing Fluid Interfaces* (WWDC 2018) 及相关设计原则（response / direct manipulation / interruptibility / spring behavior / velocity handoff / momentum projection / spatial consistency / rubber-banding / materials / haptics / reduced motion / typography）。

基于对 `mobile/garyx-mobile`（313 个 Swift 文件）的全量审计，2026-07-17。

---

## 现状诊断

**好的部分（保持，作为范式推广）：**

- 架构分层正确：手势/滚动/morph 决策逻辑在 `GaryxMobileCore` 纯函数状态机（可 SwiftPM 无 UI 测试），SwiftUI 只做 wiring。
- **标题胶囊 morph 是模范实现**（`GaryxChromeMorph.swift`）：anchor preference 锚定触发源、开合同一 reducer 对称路径、reduceMotion 跳到终态、内容 `.transition(.identity)`。消息长按菜单同样源锚定。
- 所有自定义 drag 都 1:1 跟手；capsule 下拉 dismiss 已用速度投影（`velocityProjectionSeconds = 0.20`，UIKit 侧还自采样速度补 UIKit 丢帧）；行滑动已有 predicted-end 判定 + 触觉。
- reduceMotion 覆盖良好（12 文件 43 处）。

**差距（按理念逐条对照）：**

| # | 理念条目 | 现状 | 严重度 |
|---|---|---|---|
| 1 | §5 速度交接 | 松手后速度只用来**判定**去哪，settle 一律播固定 spring，drag→动画有可感的"缝" | 高 |
| 2 | §9 rubber-band | **全 app 零处**，边界一律硬 clamp | 高 |
| 3 | §3 可中断 | drag 被 gate 挡住不能中途再抓（`canStartOpeningDrag` 等） | 中 |
| 4 | §14 reduceTransparency | 中央玻璃 modifier `GaryxAdaptiveGlassModifier` **无分支**，全部 chrome 在开启后仍是全玻璃 | 高（无障碍硬伤） |
| 5 | §15 Dynamic Type | `GaryxFont` 全语义角色固定 point size（117 处 + 20 处裸 `.system(size:)`），阅读面不随用户字号缩放 | 高 |
| 6 | §4 spring 纪律 | ~28 处魔法数 ease + 13 处散落 spring；两个**完全相同**的 morph 参数 enum 重复定义 | 中 |
| 7 | §13 触觉 | 发消息、pin/收藏、capsule dismiss 提交时刻**无触觉**；零处 `.prepare()` 预热 | 中 |
| 8 | §1 按压反馈 | ~50+ 处 `.buttonStyle(.plain)` 无 pointer-down 反馈，仅 4 个自定义样式 | 中 |
| 9 | §7 空间一致性 | 51 处 sheet/fullScreenCover 走系统默认动画，无源锚定；capsule gallery 用 fullScreenCover 而非从缩略图 morph | 低 |
| 10 | §12 materialize | 玻璃 chrome 进出场多为纯 `.opacity` fade，无 blur+scale 同动 | 低 |
| 11 | §7 滚动 | transcript prepend 的 `scrollTo` 粗粒度 fallback（`:730`）与流式尾随 double-scrollTo（`:823`）是已知可能跳变点 | 低（待复现确认） |

---

## 批次计划

### P0-A 手势物理：速度交接 + rubber-band + 可中断（核心手感批，收益最大）

> **状态（2026-07-18）**：本节为初版审计描述，**权威规格已移至 `ios-fluid-p0a-gesture-physics-design.md`（**版本以该文档内版本史为准**，此处不再记具体号）**——A0 spike 证伪系统手势路线后改为自有导航容器；手势面修正为五处 + image preview 双通道（补 row swipe 与 UIKit bridge）；返回语义实态为三活一死（`.workspaceBotsOverview` 死语义待删）。**A1/A2 已验收合 main（origin/main 03d999a38）**，A3-A5 等设计 PASS。

以 `GaryxCapsuleDragDismiss` 的投影模型为蓝本：

0. **返回上一页（本批最高优先，违反最严重的路径）**：所有 push 页 `.toolbar(.hidden, for: .navigationBar)` 杀掉了原生 swipe-back；自定义 leading-edge 返回手势（`GaryxMobileViews.swift:741,762`）在返回类分支里 `sidebarDragOffset = 0` —— 滑动全程页面纹丝不动，松手过阈值后播 0.16s ease（`sidebarDrilldown`）硬切页。零跟手、零速度交接、零可中断。改为自定义交互式返回过渡：当前页 1:1 跟手滑出 + 上一页视差滑入（对齐 iOS 原生 back swipe 视觉），松手速度投影判定 + 速度注入 settle，中途可反悔。注意"返回"有五种语义（`GaryxMobileLeadingEdgeAction`: popToHome / mainPanelBack / settingsOverview / workspaceBotsOverview / openSidebar），不全是 NavigationStack pop，须自定义过渡统一覆盖而非简单恢复系统手势；过渡状态机落 Core。

另统一四处既有 drag：

1. **左抽屉**（`GaryxMobileViews.swift:726-846`）、**右缘 task tree**（`GaryxMobileTaskTreeSidebarViews.swift:188-294`）、**image preview 下拉**（`GaryxImagePreview.swift:615` + Core `GaryxImagePreviewDismissGesture.swift`）、**capsule snapback**（`GaryxMobileCapsuleViews.swift:699`）：
   - 松手时把释放速度注入 settle spring（`initialVelocity`），消除 drag→动画的缝；
   - 判定目标用动量投影 `project(v, d≈0.998)` 而非当前位置最近边（capsule 已有，推广）；
   - `GaryxImagePreviewDismissGesture.shouldDismiss`（`:62`）加 velocity 项——现在快甩短距不触发 dismiss，违反直觉；
   - 边界超出改 rubber-band 渐进阻尼，替换硬 clamp；
   - 拆掉「动画中不能起 drag」的 gate，允许中途再抓（从 presentation value 起新动画）。
2. 投影/阻尼/判定全部落 `GaryxMobileCore` 纯函数 + SwiftPM 测试（含释放速度→settle 初速换算、rubber-band 曲线、mid-flight re-grab 状态转移）。

**验证**：Core 单测全绿 + 真机/模拟器逐帧慢放对比（快甩 vs 慢放手、边界拖拽、动画中途抓回）。

### P0-B 无障碍硬伤：reduceTransparency（小而必须）

> **状态：✅ 已交付合 main（origin/main 25478f6ee，2026-07-18）**——Reduce Transparency 中央分支 + prefersCrossFadeTransitions 共享策略 + iOS 26-only 政策入 AGENTS/CLAUDE。评审 #TASK-2398 PASS。

- `GaryxAdaptiveGlassModifier`（`GaryxMobileDesignSystem.swift:229-280`）加 `accessibilityReduceTransparency` 分支：升不透明度、去 blur，一处改全 app 生效（composer、FAB、morph 面、capsule 面板、task tree 面板、image preview chrome 全部走它）。
- 顺手补 `prefersCrossFadeTransitions`：偏好交叉淡化的用户把滑动类 transition 换 fade。

**验证**：开关 Reduce Transparency / Reduce Motion 截图对比各主要 chrome。

### P1-C 动画 token 收敛：一套 GaryxMotion 设计令牌

- 合并两个参数完全相同的 morph enum（`GaryxCapsuleChromeMorph` 与 `GaryxThreadRuntimeMorph`，同为 0.42/0.76 + 0.32/0.92）；
- 建统一 token 集（morph / settle / drilldown / toast / press / rowSwipe），替换 ~28 处魔法数 ease 与 8+ 处 inline spring；
- 立规则并写进 token 注释：**默认 critically damped（dampingFraction≈1，无过冲）；只有手势带动量的释放（甩、抛）才允许 bounce（~0.8）**。凭空出现的菜单/toast 过冲即违规。

**验证**：改动是纯等价替换 + 少量参数归一，逐面走查动画无回归。

### P1-D 触觉 + 按压反馈

- 补触觉：发送消息（`sendLocalDraft`，`GaryxMobileComposerViews.swift:631`）、pin/收藏切换、capsule dismiss 越过阈值时刻；关键 generator 加 `.prepare()` 预热（现在零处，首次触觉可能迟滞）。原则：**只加在有意义的提交时刻**（成功/错误/落位），不过度。
- 建共享 `GaryxPressableRowStyle`（scale ~0.97 + opacity，reduceMotion gated，参照现有 `GaryxItemActionMenuButtonStyle`），铺到 ~50 处 `.buttonStyle(.plain)` 的可点行。

**验证**：真机手感走查 + 触觉与视觉同帧（不允许动画滞后于触觉）。

### P2-E Dynamic Type：阅读面字号相对化

- `GaryxFont`（`GaryxMobileDesignSystem.swift:109-153`）的阅读角色（body/callout/subheadline/footnote/caption）改 `UIFontMetrics`/text-style 相对字号（现有 `scaledCallout` 模式推广）；transcript、设置、菜单等阅读面优先。
- **边界待拍板**：固定字号是当年为 pinned chrome 几何有意设计的（代码注释明说）。建议 chrome（导航胶囊、FAB、composer 附件条）保留固定或设缩放上限，阅读面完全跟随用户字号。
- 布局间距同步查 fixed pt → 相对单位，避免大字号破版。

**验证**：Dynamic Type 最大档 + 辅助功能特大档截图矩阵（MD5 对比法已有先例）。

### P0-G gateway 投递加固批（纯服务端，v21 收缩）

> **状态：已立项待启动**。老板 2026-07-18 指令（架构最漂亮、历史直接改）后，客户端投递架构（ComposerPayloadStore/ScopeBoundOperationContext/durable outbox/at-most-once 出口/scope 化 composer）**已全部拉回 P0-A 直接建新**（设计 v21）。本批只剩纯 gateway 侧：① dispatch admission 持久化幂等账本（key=(scope,threadID,kind,clientIntentID)，重复请求返回既有 run）；② 原子 create+dispatch 命令（折叠客户端多段写）；③ `prompt-attachments` TTL/删除所有权（`workspace_files.rs:383`）；④ **`(scope, createIntentID) → threadID` 唯一索引 + 查询接口**——iOS 恢复"无幽灵/重复 thread"保证的启用条件（P0-A 设计 §1.6，当前 create-response 丢失如实建模为 ambiguous）。落地后客户端按 P0-A 设计 §1.6 声明升级。另有 **Mac 对齐跟进**：DurableDeliveryState canonical spec 的 Mac 消费实现。

### P2-F 空间一致性 + materialize（打磨批）

- capsule gallery 打开从 fullScreenCover 改为**从缩略图 anchor morph**（复用 GaryxChromeMorph 的 anchor preference 模式，进出同路径）；
- 玻璃 chrome 进出场从纯 opacity fade 改 blur+scale 同动的 materialize；
- 小编辑器类 fullScreenCover（Skills/Settings/Commands 的编辑页）评估改 sheet + detents（产品判断，逐个过）；
- 顺手核 transcript prepend `scrollTo` fallback（`GaryxMobileConversationViews.swift:730`）与尾随 double-scrollTo（`:823`）是否可复现跳变，可复现则按 bug 流程走。

**验证**：逐帧慢放 + 进出场路径对称性走查。

---

## 工程约束（全批次共用）

- 决策逻辑一律进 `GaryxMobileCore` + SwiftPM 测试；app target 只做 SwiftUI wiring（repo 既有规则）。
- iOS 26 Liquid Glass 两条硬规则：glass 直接施加于内容视图（勿放 `.background {}`）；交互玻璃控件必须显式 `.contentShape`。
- 新增文件必须 `xcodegen generate` 并提交 pbxproj；验证以 `xcodebuild` 为准（swift test 对 app target 假绿）。
- 每批独立成 task，走对抗评审到 100% PASS 再合 main；UI 类改动优先无 UI 测试（真实数据驱动 Core 断言），确需视觉验证走模拟器截图。

## 建议推进顺序

**P0-A → P0-B 先行**（一个管手感、一个管无障碍硬伤，互不干扰可并行）；P1-C/D 随后；P2-E 需要你先拍板 chrome 固定字号的边界；P2-F 最后打磨。
