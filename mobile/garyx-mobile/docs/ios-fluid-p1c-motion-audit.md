# iOS 流体交互 P1-C：动画令牌审计与迁移清单

依据：`ios-fluid-interface-optimization-plan.md` P1-C。基线为
`origin/main` `969744192`（P0-A A6 收官），审计范围是
`mobile/garyx-mobile` 的 App、Core、Widget 与测试源码。

## 1. P0-A 后重新审计

原始扫描命令：

```sh
rg -n --glob '*.swift' \
  '(withAnimation|\.animation\s*\(|\.spring\s*\(|easeInOut|easeOut|\.transition\s*\()' \
  mobile/garyx-mobile
```

生产源码（App + Core + Widget）在迁移前的原始命中如下：

| 入口 | 命中 | 说明 |
|---|---:|---|
| `withAnimation` | 21 | 全部是真实 SwiftUI 动画事务 |
| `.animation(` | 24 | 19 个 View modifier，另 5 个是带括号的 `TimelineView` 帧驱动；ink spinner 的 `TimelineView(.animation)` 无括号，不在这 24 个命中内 |
| `.spring(` | 8 | 4 个重复 morph、2 个 composer、2 个 image preview |
| `easeInOut` | 7 | 表单、头像、鉴权与 running badge |
| `easeOut` | 18 | toast、press、菜单、滚动、展开与 drilldown |
| `.transition(` | 31 | 23 个 SwiftUI transition，另 8 个是业务状态机同名方法 |

扩展扫描还发现原审计未覆盖的债务：

- `GaryxFavoriteStar` 有一个非手势 `.symbolEffect(.bounce)`；
- 5 类连续动画周期散落在 shimmer、ink spinner 与 running indicator；
- P0-A 新增 4 个内联 `SpringCurve` 参数入口。P0-A 的解析物理模型本身
  不是债务，但参数仍应从同一令牌点读取；
- `GaryxCapsuleChromeMorph` 与 `GaryxThreadRuntimeMorph` 的开合参数重复定义仍在。

## 2. 单一定义点与可访问性解析

唯一数值目录是 `Sources/GaryxMobileCore/GaryxMotion.swift`：

- `GaryxMotion.Token`：语义令牌；
- `Specification`：curve、cross-fade curve、kinetics 与 spatial effect；
- `resolve(_:preferences:)`：统一产出 `.spatial / .crossFade / .immediate`；
- `springCurve(for:)`：P0-A `SpringCurve` 的兼容入口；
- `GaryxMotionContext`：App 层唯一 SwiftUI `Animation` / `AnyTransition`
  适配器，消费方不再自行读取 Reduce Motion 或 cross-fade 偏好。

令牌构造器带有硬不变量：任何有过冲潜力的 curve（`dampingRatio < 1`，或
纵向控制点越出 `0...1` 的自定义 timing curve）只允许
`kinetics == .gestureRelease`。目录不提供会隐含系统基础 bounce 的
`.snappy` escape hatch。因此菜单、toast、press、程序化导航、无速度
snap-back 与 morph 无法误用带过冲曲线。

## 3. 令牌与消费面

| 令牌组 | 令牌 | 主要消费面 |
|---|---|---|
| Morph | `morphOpen`, `morphClose` | Capsule chrome、thread runtime chrome |
| 物理 settle | `settle`, `rowSwipe`, `snapBack`, `momentumSnapBack`, `cancelSnapBack` | route、drawer、task tree、row swipe、image dismiss、capsule dismiss |
| 导航/面板 | `drilldown`, `runtimeDrilldownExit`, `runtimeDrilldownEnter`, `panelResize`, `composerDrilldown` | workspace drilldown、runtime pager、composer popover |
| 临时反馈 | `toast`, `imageSaveFeedback`, `favoriteToggle` | 全局错误、图片保存、收藏星标 |
| Press/menu | `press`, `floatingPress`, `subtlePress`, `pressHighlight`, `messageMenu`, `threadMenuFocus`, `threadMenu` | FAB、行操作、composer、消息与 thread 菜单 |
| 内容变化 | `authenticationStep`, `disclosure`, `formDisclosure`, `avatarChange`, `avatarPreview`, `avatarLoading` | 登录、表单 disclosure、头像编辑/生成 |
| Transcript/list | `rowRemoval`, `threadListMutation`, `scrollLatest`, `tailThinking`, `scrollToTail`, `turnDisclosure`, `turnAutoDisclosure`, `streamingResize`, `transcriptAppear` | 首页列表与会话阅读面 |
| Composer/image | `composerPayload`, `composerPanel`, `imageZoom`, `imageZoomReset` | 附件区、添加面板、图片缩放 |
| 连续状态 | `loadingShimmer`, `thinkingShimmer`, `inkSpinner`, `runningTyping`, `runningOrbit` | skeleton、Thinking、toolbar loading、running badge |

文件级消费面：

- 基础接线：`GaryxMobileAccessibility.swift`, `GaryxMobileMotion.swift`；
- chrome/navigation：`GaryxMobileCapsuleViews.swift`,
  `GaryxMobileThreadRuntimeSettingsViews.swift`, `GaryxRouteStackContainer.swift`,
  `GaryxHorizontalRevealInteraction.swift`, `GaryxMobileViews.swift`,
  `GaryxMobileTaskTreeSidebarViews.swift`；
- transient/controls：`GaryxMessageActionMenu.swift`,
  `GaryxMobileStatusComponents.swift`, `GaryxHomeNewThreadFab.swift`,
  `GaryxMobileListComponents.swift`, `GaryxMobileComposerViews.swift`；
- content/transcript：`GaryxMobileConversationViews.swift`,
  `GaryxMobileTurnViews.swift`, `GaryxMobileMessageBubbleViews.swift`,
  `GaryxThreadListRowButton.swift`, `GaryxMobileSidebarViews.swift`,
  `GaryxMobileConversationStatusViews.swift`；
- forms/media：`GaryxMobileClaudeCodeAuthViews.swift`,
  `GaryxMobileFormComponents.swift`, `GaryxMobileAgentsViews.swift`,
  `GaryxImagePreview.swift`, `GaryxMobileDesignSystem.swift`。

历史上的两个 morph animation enum 已删除；各自只保留不重复的几何 metrics，
开合动画共同消费 `morphOpen` / `morphClose`。

## 4. 实际参数调整

除下表外，所有迁移保持原 curve 与时长。下表只处理明显违反“默认无过冲”
规则的状态驱动动画：

| 消费面 | 迁移前 | 迁移后 | 理由 |
|---|---|---|---|
| Capsule + thread runtime morph open | spring `0.42 / 0.76` | `0.42 / 1.00` | 程序化 morph，无释放动量 |
| Capsule + thread runtime morph close | spring `0.32 / 0.92` | `0.32 / 1.00` | 程序化收合，无释放动量 |
| Composer payload layout | spring `0.24 / 0.88` | `0.24 / 1.00` | 数据驱动布局变化 |
| Composer add panel | spring `0.22 / 0.82` | `0.22 / 1.00` | 按钮状态变化 |
| Composer add-popover drilldown | system `.snappy(duration: 0.22)` | `easeOut 0.22` | page 状态变化，无手势动量；移除系统基础 bounce |
| Gallery dismiss reset | spring `0.22 / 0.88` | `0.22 / 1.00` | 旧路径没有速度交接 |
| Image double-tap zoom | spring `0.28 / 0.86` | `0.28 / 1.00` | 离散双击命令 |
| Capsule cancelled drag | spring `0.28 / 0.90` | `0.28 / 1.00` | cancel 注入零速度 |
| Route / drawer / task tree / row programmatic or cancelled settle | spring `0.22 / 0.88` | `0.22 / 1.00` | 非手势释放或无动量 cancel |
| Favorite star | system `.bounce` | `easeOut 0.18` | 收藏提交不是动量手势 |

以下释放路径保留原 underdamped 曲线，因为它们把真实速度注入 analytic settle：

- route、drawer、task tree、image dismiss：`0.22 / 0.88`；
- row swipe：`0.22 / 0.88`；
- capsule release snap-back：`0.34 / 0.82`。

## 5. Reduce Motion / Cross-Fade

`GaryxAccessibilityTransitionPolicy.mode` 是三态真值源，令牌解析结果负责：

- `.spatial`：保留 token curve 与 scale/offset/move；
- `.crossFade`：opacity-safe 消费面保留淡化曲线，所有消费面都剥离
  scale/offset/move；纯空间 API 直接返回 `nil`；
- `.immediate`：返回 `nil` animation，并剥离 spatial effect。

连续 shimmer / spinner 在非 spatial 模式暂停；morph 在 cross-fade 模式直接使用
展开几何，只对 opacity 做动画。消费 View 不再出现各自的
`if reduceMotion` 或重复调用 `GaryxAccessibilityTransitionPolicy`。
`GaryxProductionRouteStack` 仍读取两个系统偏好，只为构造 P0-A 已有的
`GaryxRouteVisualPreferences`；实际空间 / 淡化 / 即时决策仍由 Core
`VisualPolicy` 完成，不是第二套动画分支。

## 6. 扫描白名单与验收命令

允许保留的命中：

- `GaryxMotion.swift` 是动画数值的唯一令牌目录；
- `GaryxMobileMotion.swift` 只把变量映射为 SwiftUI API，无裸数值；
- P0-A 物理 / settle-driver 单测中的
  `SpringCurve(response:dampingRatio:)` 数字是解析器测试向量；
- `GaryxMotionTests` 的 curve 字面量是对令牌目录解析结果的精确断言，不是
  产品消费方的第二处定义；
- `pumpUI(duration:)` / `pumpMainRunLoop(duration:)` 是测试时钟，不是产品动画；
- `.transition(.identity)` / `.transition(.opacity)` 没有 timing 参数；
- 业务 reducer 的 `transition(...)` 与 SwiftUI 无关；
- `TimelineView(.periodic)` / `.everyMinute` 是数据时钟，不是视觉 easing。
- `GaryxProductionRouteStack` 的系统偏好读取是 P0-A UIKit `VisualPolicy`
  的环境适配边界，不包含动画参数或局部策略判断。

产品源码的清零证据：

```sh
rg -n --glob '*.swift' \
  '(duration|response|dampingFraction|dampingRatio)\s*:\s*[0-9]' \
  mobile/garyx-mobile/App mobile/garyx-mobile/Sources mobile/garyx-mobile/Widget \
  | rg -v 'Sources/GaryxMobileCore/GaryxMotion.swift'

rg -n --glob '*.swift' \
  '(Animation\.(spring|ease|timing|snappy|smooth|linear)|withAnimation\s*\(\s*\.(spring|ease|timing|snappy|smooth|linear)|\.animation\s*\(\s*\.(spring|ease|timing|snappy|smooth|linear))' \
  mobile/garyx-mobile/App mobile/garyx-mobile/Sources mobile/garyx-mobile/Widget
```

两条命令实际输出均为 0；完整 SwiftPM、Xcode 与界面验证结果如下。

## 7. 验证结果

- 上述两条产品源码扫描均为 0；全仓所有 Swift 源码都位于
  `mobile/garyx-mobile`，没有仓外 Swift 消费面遗漏。
- `swift test`：1,414 tests，0 failures。
- `xcodebuild -target GaryxMobile -sdk iphonesimulator ... build`：
  iOS 26.5 SDK，`BUILD SUCCEEDED`。
- `xcodebuild -scheme GaryxMobile -destination
  'platform=iOS Simulator,name=iPhone 17 Pro,OS=26.5' test`：
  iPhone 17 Pro / iOS 26.5，退出码 0。
- iOS 26.5 慢放抽样使用显式 opt-in 的合成 fake-route fixture，只验证动画
  轨迹，不作为真实内容正确性的替代证据：spatial 模式 18 个 50 ms 采样步
  `curve=pass`，`backwards=0`、`bodyDelta=0`；cross-fade 中间帧只有透明度
  叠化、无空间位移；Reduce Motion immediate 路径单帧到达 terminal。

等价迁移面的曲线类型、时长、effect 与原值逐项保留；第 4 节列出的规则修正
是仅有的可感参数调整。
