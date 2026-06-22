# iOS 首页线程列表滚动卡顿根治 — 最终设计 v3（最干净·一步到位）

> **v3 定稿说明**：老板钦定「最干净、一步到位、拒绝折中」。本文取代 v1（commit `3ff44fe2` 的三层 actor 综合稿）与 v2 折中稿（先轻后重/按需加重）。**终点是纯架构**：首页数据完全搬后台、UI 纯哑渲染不可变快照、退回首页彻底停流、native List 真 cell 复用——无任何「按需决定要不要做」的条件分支。补齐 codex 跨厂商 review `#TASK-1132` 挖出的 5 个坑。用 **shadow 影子模式分步安全落地、每步可单 flag 回滚、不打一个补丁**。

## 0. 目标
让 garyx iOS 首页线程列表滑动像 Telegram 一样顺，彻底根治「开 running 线程 → popToHome 回首页 → 滑/静止2s/恢复」的卡顿。**根治非补丁**：禁止任何 gate/flag/freeze/「滚动暂停刷新」/throttle 阀门（迁移期的 cutover flag 仅用于回滚，不进稳态产品逻辑）。**一步到位到纯架构**，不留尾巴。

## 1. 真凶（Gary + codex 双向 file:line 核实，保留）
「静止2s→恢复 hang」是一拨同步主线程 burst，三通道在「running 选中线程 + 已 popToHome」汇聚（「哪条主导」需真机二分，但 v3 一步到位把三条全治、不依赖定量）：
- **通道1 流活主 actor + popToHome 不停流**：`popToHome()`（`+Navigation.swift:56`）不清 selectedThread、不 `stopSelectedThreadStream`；`runSelectedThreadStream` 的 `bytes.lines`（`ThreadStream.swift:122`）+ seq 记账（`:182`）仍在 @MainActor（decode/merge 已 off-main `:157/:240`）。
- **通道2 @Published 写 → god-object objectWillChange**：`applyThreadRenderSnapshot`（`ThreadStream.swift:255`）写 `@Published renderSnapshotsByThread`；flush 写 `messages`+run-state。`GaryxShellView` 主体已被 `.equatable()`（`Views.swift:81`，`==` 只比 store 引用 `:488`）隔离，但**漏网点**仍订阅 god object：首页 footer `GaryxSidebarThreadAutoLoadFooter`（`SidebarViews.swift:845`）、running badge 30fps `TimelineView`（`:1648/1684`）、debug binding（`Views.swift:79`）、GaryxRootView 自身（`:19`）。
- **通道3 run-state 写主线程 O(N) projection**：`runStateByThread` didSet（`Model.swift:132`）→ `refreshHomeThreadListSnapshot`（`Presentation.swift:55`）构 `homeThreadRunningThreadIds` O(N) 三源扫（`:79`）；`IdentityKey` 全量 `input.threads.map(displayThread)` 拷贝（Core `:435`，`sectionsCache` `:415` 只挡 build 挡不住该比较）。

## 2. 最干净目标架构（一步到位，纯架构）

```
            后台 (off MainActor, Swift actor)                  @MainActor (纯哑渲染)
 ┌─────────────────────────────────────────┐        ┌────────────────────────────────┐
 │ GatewayStreamActor                       │run-state│ @Observable ConnectionModel    │
 │  byte-loop + seq planner + decode/merge  │ delta  │ @Observable HomeChromeModel    │
 │  posts: render帧→会话面 / run-state→投影  │ 事件   │  ← Root/footer/drawer 窄观测    │
 └───────────────────┬─────────────────────┘        │   (god-object 一个不留)         │
                     ▼                                └────────────────────────────────┘
 ┌─────────────────────────────────────────┐        ┌────────────────────────────────┐
 │ HomeProjectionActor (单写者 mailbox)      │  hop   │ HomeStore (@MainActor)         │
 │  纯 HomeProjectionReducer (Core)          │ snapshot│ @Published snapshot            │
 │  三源 run-state max-seq-wins / 稳定 id     │ +diff  │ appliedSeq 守(丢 stale)        │
 │  不可变 HomeSnapshot + CollectionDifference│ +seq  └──────────────┬─────────────────┘
 └─────────────────────────────────────────┘                       ▼
  ingest ← recent/pins/乐观pin/archive/visibility       ┌────────────────────────────────┐
                                                        │ native List (真 cell 复用)      │
                                                        │  喂 HomeSnapshot;滑动=布局+绘制 │
                                                        └────────────────────────────────┘
```

**五个"最干净"落点（全部到位，无条件分支）：**
1. **数据全搬后台**：首页状态（threads/running/pins/loading/visibility）所有权在 `HomeProjectionActor`，@MainActor 不持有、不计算。O(N) busy-scan + IdentityKey 拷贝从主线程消失。
2. **UI 纯哑渲染**：`HomeStore` 只 apply actor 算好的不可变 `HomeSnapshot` + `CollectionDifference`，零数据逻辑。
3. **god-object 一个不留**：首页路径上 root/footer/debug/pagination 全迁 `@Observable`/窄 store，转写帧 @Published 写无法唤醒任何首页 body。
4. **流彻底停（B②）**：`popToHome` 即 `stopSelectedThreadStream`，保 `selectedThreadStreamCursor`；running dot 改由 `recent_threads` 投影列 + reconcile 供给。主线程零流活。重开会话 cursor-resume 证不 stale。
5. **真 cell 复用**：`ScrollView+LazyVStack` → native `List`；timestamp 烘进 row（reducer 每分钟 tick 防相对时间冻结）；badge 折叠成 list 级单一 phase（O(1)）。

## 3. codex 5 坑的解法（全补，写进不变量）
1. **run-state 副作用边界（最大坑）**：`applyTranscriptRunState`（`Threads.swift:1266`）含会话生命周期副作用（provider ack/title/terminal cleanup/runTracker）。拆成「首页 projection 输入」(running 三源之一) + 「会话副作用」。**只前者入 actor**，后者留 @MainActor 会话路径。
2. **乐观 archive/pin 回滚边界**：archive（`Bots.swift:248/260/296`）牵连 `pendingThreadArchives`/删 endpoints/刷新 remote/`openNewThreadDraft`/失败回滚。定义为同一 mailbox 的 seq-stamped 事件，回滚是更晚 seq 事件，非首页副作用（endpoints/draft/remote）边界显式入不变量，不带外写 actor。
3. **popToHome 流**：B② 彻底停（§2.4），消除 v1 的自相矛盾。
4. **parity gate 扩展**：不止比 sections，比完整 rendered `HomeSnapshot`（含 `isLoadingThreads`/`isHomeVisible`/footer pagination/timestamp/badge 状态）+ side-effect counters + **batch 边界对齐**（actor coalescing 跳过老路中间态，按 batch 边界比、非 per-appliedSeq 比中间态）。
5. **home visibility/navigation 事件**：`HomeProjectionEvent` 加 `homeVisibilityChanged`/`navigationChanged`（`isHomeVisible` 驱动 silent refresh `SidebarViews.swift:335/449`），否则丢自动刷新。

## 4. 关键不变量
- **单写者**：所有首页输入（recent ingest/pins/run-state delta/乐观 pin/archive/回滚/visibility）作为 seq-stamped 事件经单一 mailbox + 单 reducer；乐观回滚必须更晚 seq 入同一 mailbox，不带外写。
- **monotonic appliedSeq**：每 snapshot 带单调 seq；HomeStore 仅 appliedSeq 更大才 apply（async-hop 乱序覆盖防护）。
- **三源 run-state 全折**：`runTracker.isThreadBusy`(本地乐观)+`runStateByThread[id].busy`(网关流)+`isThreadSummaryRunning`(recent_threads 投影列) 三源，per-thread max-seq-wins；漏任一源丢 dot。
- **run-state 副作用隔离**：只首页 projection 输入入 actor，会话生命周期副作用留 @MainActor。
- **稳定行身份**：row.id==thread.id，永不取自 seq/reconcile 漂移（否则破 diff + 重引 origin_id 抖动类 bug）。跨 section（pinned∪recent）id 唯一。
- **render_state 服务端权威**：首页只消费 run-state 布尔，不碰 transcript render_state；frame.renderState 仍经 `GaryxMobileRenderStateMapper` 哑映射到会话面。
- **recent_threads 写时投影**：首页数据只来自网关 ingest，不 rescan/不重排。
- **origin_id 零抖动**：首页不含 user message 行，结构上在路径外；row id 稳定。
- **Core 分层**：reducer/diff/snapshot 全在 GaryxMobileCore + SwiftPM；app target 只放 actor 执行器 + SwiftUI 组合 + UIKit cell 桥。
- **footer/loading 纳入首页快照**：pagination/loading 不再经 god-object @Published。

## 5. 迁移（shadow 分步、每步单 flag 回滚，终点=纯架构，全部执行）
| 步 | 内容 | gate | 回滚 |
|---|---|---|---|
| **M1** | Core 纯 `HomeProjectionReducer`（增量、三源全折、稳定 id、含 visibility 事件），(state,event)→(state,snapshot,diff) | reducer parity vs 老 store + churn-300→0 全量重算 + off-main guard + lost-update 交错 | 删新 Core 文件零引用 |
| **M2** | `HomeProjectionActor`（单写者 mailbox）+ `HomeProjectionGateway`(@MainActor 捕事件)。**shadow**：老 didSet 站点同时 emit 事件，UI 仍渲染老 store，断言 parityMismatchCount==0 | parity over captured corpus + snapshotEmitCount==1(2s-drain) | flag 关 dual-emit，老路权威 |
| **M3** | `GatewayStreamActor` 搬 byte-loop+seq 记账离 @MainActor（只移首页 projection 输入，§3.1 会话副作用不动）；**popToHome 彻底停流（B②）**+ cursor 保留 + dot 走投影列 | message id+resume cursor 与老路逐字相等（含 gap-reconnect/404/rewrite）；停流后 running dot 正确 | StreamActor 回退 @MainActor 扩展(flag) |
| **M4** | `@Observable` **彻底断 god object**：root(`hasGatewaySettings`/`connectionState`)+footer(pagination)+debug+drawer chrome，**一个不留**迁窄 @Observable/store | `withObservationTracking` 双周期（转写写不失效首页 + 首页写失效首页）+ xcodebuild SUCCEEDED | 受影响子视图恢复 @EnvironmentObject |
| **M5** | **cutover**：HomeStore 渲染 actor publish 的 snapshot+diff，移除主线程 `refreshHomeThreadListSnapshot` didSet；run-state 只经 actor 事件 | 真机 probe model_publish→~0 + home_store_apply≈0（popToHome+running 窗口）；真机二分根因消失 | 单 flag 翻回老 store.apply + 恢复 didSet |
| **M6** | native `List`（真复用）喂同一 snapshot；badge 折叠 list 级单一 phase；timestamp 烘 row（reducer tick）；swipe 按 §6 | Instruments Animation Hitches/XCTHitchMetric hitchTimeRatio < 改前同机 baseline；packaged-app swipe/pin/archive 行为校验 | 回退 body 到 LazyVStack（同消费 snapshot） |

pbxproj 须随新文件 `xcodegen generate` 并提交（TestFlight CI 不跑 xcodegen，漏了 swift test 假绿）。

## 6. 仍需老板拍 / 实现期确认
- **native List swipe**（M6）：native `.swipeActions` 与现有自绘 `GaryxSwipeActionRow`（`SidebarViews.swift:473`）full-swipe/回弹/长按 archive confirmation 行为不完全一致。最干净取向=统一为 native（首页与非首页 swipe 都迁 native，行为一致）；若成本过高，M6 内单独评估。
- **hitchTimeRatio 阈值**：真机 baseline 数，实现期 M6 定。

## 7. 三问
- **像 Telegram**：滑动主线程零数据活（数据全搬 actor、O(N) 消失、objectWillChange 全断、流停）+ 真 cell 复用 + badge O(1)。
- **根治非补丁**：状态所有权搬后台 + 失效粒度换基底 + 流停，全是架构迁移，无 gate/flag/freeze/throttle 稳态阀门；迁移 flag 仅回滚用。
- **不改 bug**：单写者+monotonic seq+同 reducer 乐观=lost-update/乱序结构免疫；三源全折；run-state 副作用边界；archive 回滚边界；render_state/origin_id/recent_threads/Core 分层全保；每步 shadow parity gate + 单 flag 回滚。
