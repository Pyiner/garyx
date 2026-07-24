# iOS Send-Anchored Transcript Design

Status: approved for implementation (design by Gary, 2026-07-24; requested by
the boss the same day).

## 1. 问题

在存量线程里发消息后的体验很差，两个症状：

1. **连环抖动**：发送后所有消息跟着抖、闪好几次。
2. **上屏感知延迟**：用户感觉自己的消息和 thinking 要等 ACK 才出现。

### 现状事实（2026-07-24 调查结论，实现前需用复现测试再验证）

- 乐观上屏**已经是即时的**：`presentOptimisticSend`（`GaryxMobileModel+Composer.swift`）
  在 durable barrier 的 `present` 闭包里同步插入
  `localState=.optimistic`、id=`origin:<clientIntentId>` 的本地 user 行；
  `showsPendingAcknowledgement` 让 thinking 本地立即亮（服务端
  `tailActivity` 随后接管）。感知延迟大概率是抖动/闪烁掩盖了即时性，
  以及 `GaryxTailThinkingPresentationState` 的 0.2s 出现去抖。
- 发送后的滚动是**贴底跟随**：乐观 append、服务端 render frame、thinking
  出现、流式增量……每次内容变化都让
  `GaryxConversationScrollState` 发出 `TailScrollRequest`，视图层按
  `[0,40,140]ms` 重试表执行 `proxy.scrollTo(bottomAnchor, anchor: .bottom)`。
  一次发送引发的多个帧 × 每帧多次重试 = 连环滚动抖动。
- 行身份对账契约：服务端 committed user 行复用同一个 `origin:*` id，
  `GaryxMobileRenderStateMapper` / `GaryxTranscriptMerge` 凭 id 就地替换
  乐观行恰好一次；`GaryxMobileMessageGeometry` 刻意排除 localState 等
  不影响高度的字段。**但"全体消息闪"提示 optimistic→committed 或
  local→snapshot 切换时可能存在行身份/结构性重建**，需复现确认。

## 2. 目标行为（产品规格）

发消息后的转录行为整体改为 **send-anchored（锚顶）** 模型，对齐
"每次发送都像开新的一屏"的心智：

1. **发送即锚顶**：本地发送 present 的同一时刻，刚发的用户消息滚动到
   视口顶部（`anchor: .top`，带动画的一次滚动），旧消息全部顶上去。
   新线程首发与存量线程发送走同一机制，不做特判。
2. **用户消息 + thinking 同帧上屏**：乐观 user 行与 thinking 指示器同时
   出现（发送场景绕过 thinking 出现端的 0.2s 去抖——刚发送的屏下方没有
   任何会与之闪烁竞争的内容）。ACK / 服务端帧对账完全静默：行身份、
   行几何、滚动位置在 optimistic→committed 物化前后零变化。
3. **锚定期间零自动滚动**：sendAnchored 会话内，内容变化（thinking→
   流式→工具→final）不再产生任何自动滚动请求。回复内容在锚定消息
   下方向下生长；超出一屏后停在原地，由 scroll-to-bottom 按钮承担
   "回到尾部"职责（按钮现有出现条件：尾部内容在视口之下）。
4. **用户所有权不变**：锚定期间用户手动滚动 → 现有 ownership 语义接管
   （browsingHistory）；点 scroll-to-bottom → 恢复 followingTail 贴底跟随。
   打开线程（openingThread 跳底）、历史 prepend 保位等既有行为不变。
5. **再次发送重新锚定**：run 进行中的 follow-up 发送同样触发新的
   sendAnchored 会话，锚到新消息。

## 3. 设计

### 3.1 滚动状态机：新增 sendAnchored 模式（GaryxMobileCore）

`GaryxConversationScrollState`（`GaryxConversationScrollPolicy.swift`）的
anchoring 从 `{followingTail, browsingHistory}` 扩展为
`{followingTail, sendAnchored(anchorRowId), browsingHistory}`：

- 新事件 `localSendPresented(anchorRowId:)`：进入 sendAnchored，返回一次
  anchor-to-top 滚动请求（复用 openingThread 级别的几何重试表兜底，
  首次尝试带动画）。
- sendAnchored 态下 `messagesChanged` / `thinkingIndicatorShown` /
  `metricsChanged` / `contentChanged` **不产生滚动请求**（消灭抖动的核心）。
  `bottomChromeChanged` 的 repair 语义按锚点（而非尾部）修复。
- `userScrollInteractionChanged` → browsingHistory（现有语义）；
  `scrollToBottomTapped` → followingTail + 贴底（现有语义）；
  `threadOpened` → 复位（现有语义）。
- scroll-to-bottom 按钮可见性条件扩展：sendAnchored 且尾部内容在视口
  之下时同样显示。
- 滚动请求类型需能表达 target row + `.top` 锚（现有 `TailScrollRequest`
  只表达贴底；按最漂亮的形状重构请求类型，不加平行副本）。

### 3.2 底部 run-space filler（GaryxMobileCore 纯状态 + 视图接线）

锚顶要求锚点以下有至少一屏内容。于是在 bottom anchor 之前插入一个
filler spacer，由 Core 纯状态 `GaryxSendAnchorFillerState`（新文件，
SwiftPM 测试）驱动：

- 每个 sendAnchored 会话开始时建立：
  `filler = max(0, viewportHeight - bottomChromeClearance - 锚点行到内容尾部的实测高度)`。
- **会话内单调不增**：内容向下生长多少，filler 收缩多少，保证
  "锚点以下总高度 ≥ 一屏"恒成立且锚点永不位移；内容变矮（如 thinking
  消失被正文原子替换）时 filler 不回涨，杜绝反向跳动。
- 内容高度 ≥ 一屏后 filler = 0，之后保持 0。
- 会话结束（threadOpened 复位 / 新发送重建）时按新会话重算。
- filler 的存在不改变底部 anchor、历史 prepend 保位、bottomChrome
  clearance 的既有语义。
- present 闭包内同帧完成：append 乐观行 + 建立 filler + 发出锚顶滚动
  请求，避免 scrollTo 因内容不足被 clamp。

### 3.3 行身份稳定契约（闪烁根因，repro-first）

实现的第一步是**用真实捕获的帧序列做无 UI 复现**（bug 方法论）：

- 从真机/模拟器抓一次"存量线程发消息 → thinking → 流式 → final"的
  完整 render frame + 本地乐观行序列（真实数据，不手造）。
- 喂给 `GaryxMobileRenderStateMapper`，断言整个序列中：
  - row id 序列是**前缀稳定**的（历史行 id 不变，只有尾部追加/更新）；
  - 乐观 user 行 → committed 的 row id 不变、替换恰好一次；
  - 不存在任何一帧导致既有行整体换 id（那就是"全体消息闪"的根因）。
- 复现出的身份漂移（若有）在 mapper/merge 层修复为结构性稳定，
  而不是在视图层打补丁。若复现不出身份漂移，则闪烁根因即滚动请求
  风暴，由 3.1 消灭——两条根因路径都必须有测试钉住。
- 同样复现"上屏延迟"感知：断言 present 同帧本地行 + thinking 可见
  （`showsTailThinking` 无 0.2s 延迟，发送路径直通）。

### 3.4 旧行为测试的处置

`GaryxMessageSendJitterReproTests`、
`GaryxActiveThreadBottomReachabilityReproTests`、
`GaryxConversationScrollPolicyTests` 等钉住"发送后贴底"的用例按**新规格
更新**（发送→锚顶、锚定期零滚动请求），不保留旧行为兼容分支。
历史 prepend 保位（`ReadingAnchorRestore`）相关测试必须原样保绿。

## 4. 影响面

- `GaryxMobileCore`：`GaryxConversationScrollPolicy.swift`（状态机扩展、
  滚动请求类型重构）、新 `GaryxSendAnchorFillerState`、
  `GaryxMobileRenderState.swift` / `GaryxTranscriptMergeModel.swift`
  （仅当复现出身份漂移时修）、`GaryxConversationRoutePresentation.swift`
  （thinking 直通）。
- App 层：`GaryxMobileConversationViews.swift`（filler 视图、锚顶
  scrollTo 执行、观察接线）、`GaryxMobileModel+Composer.swift`
  （present 闭包通知 scrollState/filler）。
- **产品可见行为变化**：发送后不再贴底跟随，改为锚顶 + 内容向下生长
  + scroll-to-bottom 按钮回尾。这是本设计的目的本身，按铁律直接改，
  不保旧行为开关。
- 不触碰：服务端 render_state 契约、SSE 协议、桌面端。

## 5. 验收标准

1. SwiftPM headless 测试全绿（真实计数核对，含 3.3 的两条根因回归门、
   3.1 状态机新态决策表、3.2 filler 单调性）。
2. 模拟器 iPhone 17 Pro Max / iOS 26.5 / light mode 手动走查：
   - 存量长线程发消息：消息一次动画滑到视口顶部，thinking 同帧出现
     在下方，整个流式过程**零抖动零闪烁**；
   - 回复超一屏后停住，scroll-to-bottom 按钮出现且可用；
   - 锚定期间上滑浏览历史不被打断；
   - 新线程首发行为与存量线程一致；
   - 历史下拉 prepend 保位无回归；打开线程仍跳底。

## 6. Scope 边界

本设计只覆盖"发送后的转录锚定 + 上屏即时性 + 抖动根因"。过程中发现的
相邻既有问题（其他滚动场景的毛刺、mapper 的无关缺陷等）一律记入
`docs/design/ios-send-anchor-review-debt.md` 独立立项，不并入本需求。
