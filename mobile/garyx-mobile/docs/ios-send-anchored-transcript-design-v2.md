# iOS Send-Anchored Transcript Design v2

Status: approved for implementation (redesign by Gary, 2026-07-24, after the
v1 implementation was reverted in `f76d40fff` for field regressions).
Supersedes the v1 plan in `ios-send-anchored-transcript-design.md`; product
goals are unchanged, the mechanism is redesigned around the v1 failure
analysis below.

## 1. v1 事故分析（重读代码结论）

v1（`b6630448a`，已 revert）为了实现"锚定期零自动滚动"，**删除了
`garyxBottomAnchoredTranscript()`**（`.defaultScrollAnchor(.bottom,
for: .initialOffset)` + `.defaultScrollAnchor(.bottom, for: .sizeChanges)`），
把系统级锚定换成了纯状态机 + 程序化 scrollTo。这一刀砍在了所有高频
路径上：

- **A1a 打开线程闪跳**：`.initialOffset` 没了 → 每次打开存量线程首帧
  落在顶部，再由 openingThread 重试链程序化跳底。
- **A1b 流式跟随退化成连环抖**：`.sizeChanges` 没了 → followingTail
  期间的内容增长不再被布局系统钉底，改由 `messagesChanged` 每帧发
  `[0,40,140]ms` scrollTo 链追赶——把本要消灭的抖动机制装到了以前
  丝滑的模式上（观看任何正在流式的 run 都中招）。
- **A1c 跟随静默死亡**：无系统钉底时增长使 `isNearBottom` 失效，
  一旦本会话用户手动滚动过，`metricsChanged` 把 followingTail 翻成
  browsingHistory → 跟随停止、回底按钮闪现。
- **A2 锚定态重锚 snap**：sendAnchored 中 `bottomChromeChanged` 返回
  无动画的 row/.top repair → 打多行草稿、键盘出没时画面猛跳。实际上
  底部 chrome 变化只裁视口下沿，锚在顶部的行根本不动，这个 repair
  纯多余。
- **A3 filler 单调不增 vs 键盘**：键盘弹起时发送，filler 以缩小的
  viewport 建立；键盘收起后视口变大 filler 不许回涨 → 锚顶 scrollTo
  clamp，锚不到位。
- **A4 验收盲区**：headless 测试测状态机决策表、模拟器走查只覆盖发送
  场景；A1 三条全在"打开/观看"路径上，清单没有它们。

## 2. 设计 v2：系统级锚定不拆，两模式共存

产品目标（2026-07-24 老板修订）：发送即锚顶、消息+thinking 同帧上屏、
回复在一屏内生长时锚点纹丝不动；**回复一旦长过一屏（出现屏下生长），
无缝转入贴底跟随**，不是停住等按钮；手势随时接管。机制全部重来：

### 2.1 defaultScrollAnchor 按会话切换角色（核心）

- `.defaultScrollAnchor(.bottom, for: .initialOffset)` **永远保留**：
  打开线程行为与现状完全一致（首帧即底部，无程序化跳动）。
- `.defaultScrollAnchor(isSendAnchored ? nil : .bottom, for: .sizeChanges)`：
  仅 sendAnchored 会话内挂空。API 本身接受 `UnitPoint?`，值变化不
  重建 ScrollView、不丢滚动位置。挂空后 UIScrollView 原生行为就是
  "视口下方内容增长不动 offset" → 锚定期零自动滚动**天然成立**，
  不需要在其他模式上做任何压制。
- followingTail / browsingHistory / 打开 / 历史 prepend 的现有语义
  **一行不改**——这是 v2 与 v1 的本质区别。

### 2.2 sendAnchored 会话（沿用 v1 概念，修正细节）

- 进入：发送 present 同帧的 `localSendPresented(anchorRowId:)` 信号
  （沿用 v1 的 `GaryxConversationLocalSendPresentation` 发布 +
  同一 observation 携带）。发出一条 `localSend` 链（重试表
  `[0,320,650,1000]ms`，首次带动画）把 `user_turn:<id>` 锚到 `.top`；
  链的 settle 判据沿用 v1 的 rowTargetViewportOffset。
- 会话内：`messagesChanged` / `thinkingIndicatorShown` /
  `metricsChanged` 不产生滚动请求（同 v1）；**`bottomChromeChanged`
  也返回 nil（修 A2）**；`composerFocused` 返回 nil（同 v1）。
- **耗尽移交（老板 2026-07-24 修订：出现屏下生长就要跟）**：锚点
  以下真实内容填满会话 floor（= 回复开始屏下生长、filler 已自然归零）
  时，会话「耗尽」：`sendRunSpaceExhausted()` 把锚定态无缝移交给
  followingTail，发出一次短动画 settle 贴到真实尾部，随后由系统
  sizeChanges 钉底接管持续跟随。耗尽判定用**内容坐标系**测量
  （filler 状态的 `isExhausted`），不用视口坐标——发送瞬间视口还没
  锚过去时不会误判。若读者此刻在浏览历史，耗尽只收掉 run space 标记
  （绝不拽人），近底重新武装跟随的基线语义随之恢复。
- 退出：耗尽移交（上条）；用户滚动手势 → browsingHistory（现有
  ownership 语义）；回底按钮 → followingTail + 贴底（此刻恢复
  `.bottom` sizeChanges，人已在底部，恢复瞬间无位移）；线程切换/
  占据变化 → reset。
- 回底按钮可见性沿用 v1 的 intrinsic-tail 判据
  （`isContentTailBelowViewport`：真实内容尾部在视口之下才显示，
  filler 空白不算）；正常路径下耗尽移交先于按钮出现，按钮主要服务
  浏览态。

### 2.3 filler v2：floor 规则（修 A3）

- 会话状态：`runSpaceFloor = max(runSpaceFloor, 当前有效 viewport)`
  （会话内单调不减），`filler = max(0, runSpaceFloor -
  contentBelowAnchorHeight)`。
- 视口变大（键盘收起）floor 跟着抬、filler 允许回涨——filler 的伸缩
  都发生在锚点下方、视口之外，不位移锚点，也不受 A3 clamp 问题影响。
- 内容超过 floor 后 filler = 0 并保持（内容不会缩回去，floor 只增）。
- 测量沿用 v1 的 intrinsic-tail 哨兵 + rowGeometryBox 机制（该部分
  v1 实测无问题，历史 prepend 保位测试全绿）。

### 2.4 thinking 同帧上屏（沿用 v1）

`GaryxTailThinkingPresentationMode`：pending-ack 窗口 `.immediate`
（绕过 0.2s 出现去抖）、服务端 thinking `.debounced`（已可见则不重置，
ACK 静默）、其余 `.hidden`。沿用 v1 实现。

## 2.5 v2.1 真机反馈修订（2026-07-24 晚，老板真机试用）

1. **手势即收场（会话状态合一）**：用户滚动手势开始的瞬间即退出锚定
   会话，run space 空白同帧收场（空白在视口之下，收场无感；人在空白
   里则 clamp 落到真实底部=“吸底”）。由此 `hasSendRunSpace` 独立
   生命周期整体删除——run space 与 `sendAnchored` 态同生共死，
   会话退出的 filler 收场由视图镜像（`isSendAnchored` 翻转）单一
   拥有；near-bottom 重武装抑制等衍生状态一并删除，非会话语义回到
   与基线完全一致。
2. **锚顶 top inset**：锚定位置 = 视口顶 + `conversationSendAnchorTopInset`
   （16pt 起步，模拟器对标题胶囊实调），给标题胶囊留呼吸距离。
   行目标滚动改为宿主 UIScrollView 精确 setContentOffset（镜像
   prepend-restore 模式，proxy.scrollTo 无法表达 inset），settle
   判据与 filler floor 同步扣除 inset。
3. **触觉与动画对齐**：发送触觉从 model 层（present 后立即）移到
   转录视图**首次授权写入**的瞬间，与锚顶动画同帧启动；动画条件从
   “index 0”改为“首次真实写入”（0ms 尝试因行未布局打空时，50ms
   早期重试补位并仍带动画，而不是无动画 snap）；localSend 重试表
   `[0,50,320,650,1000]ms`。
4. 进线程刷新前后位置不一致（偶发）由 #TASK-2697 独立复现定根因，
   不并入本修订。

## 3. Scope 边界

- 不触碰：服务端契约、SSE、桌面端、历史 prepend 机制、
  followingTail/browsingHistory/opening 语义。
- 相邻既有问题记 `docs/design/ios-send-anchor-review-debt.md`，
  不并入。

## 4. 验收标准（修 A4：必须覆盖"打开/观看"路径）

1. SwiftPM headless：状态机决策表（含 sendAnchored 分支 +
   **bottomChromeChanged 在锚定态返回 nil**）、filler floor 规则
   （含视口变大回涨）、thinking 三模式、**非锚定路径守护测试**：对
   同一事件序列断言 followingTail/opening/browsingHistory 的请求
   序列与基线语义完全一致。
2. 模拟器（iPhone 17 Pro Max / iOS 26.5 / light）走查清单：
   - 打开长存量线程：首帧即底部，无先顶后跳；
   - 观看一个正在流式的 run：贴底跟随丝滑，无程序化抖动；
   - 手动上滑 → 回底按钮 → 恢复跟随；
   - 存量线程发送：一次动画锚顶、thinking 同帧、流式零抖动；
   - 回复长过一屏：无缝转入贴底跟随（用户消息滑出屏顶、一路跟到
     run 结束），衔接处无跳变；锚定期/跟随期上滑均不被打断；
   - 多行草稿输入/键盘出没：锚定不 snap；
   - 键盘弹起状态发送 → 键盘收起：锚位仍正确（A3 回归）；
   - 新线程首发一致；历史下拉 prepend 保位；
   - follow-up 发送重新锚定。
3. **真机验收门（新增，铁律）**：模拟器全过后不直接合入 TestFlight
   流——合 main 后由老板真机试用确认体验，未确认前不视为交付完成。
