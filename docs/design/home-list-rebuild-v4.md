# iOS 首页线程列表滚动卡顿根治 — 最终设计 v4（定稿候选）

> **v4 说明**：在 v3（最干净一步到位）基础上,整合 codex 二轮跨厂商 review `#TASK-1140`(verdict=REWORK)的 6 个 findings + Gary 独立交叉发现(M3 冗余),共 7 处修订。方向不变(数据全搬后台 / UI 纯哑渲染 / popToHome 彻底停流 B② / native List 真复用),补齐实现层坑。前序:v1 `3ff44fe2`、v3 `12a87284`。**仅 §6.1(远端 dot 10s 延迟接受度)待老板拍,其余已对齐。**

## 0. 目标
让 iOS 首页列表滑动像 Telegram 一样顺,根治「running 线程 → popToHome → 滑/静止2s/恢复」卡顿。根治非补丁:无 gate/freeze/throttle 稳态阀门(迁移 flag 仅回滚用)。一步到位到纯架构。

## 1. 真凶(双向核实) + 全部漏网订阅点
三通道在「running 选中线程 + 已 popToHome」汇聚:
- **通道1 流活主 actor + popToHome 不停流**：`runSelectedThreadStream` byte-loop(`ThreadStream.swift:122`)+seq 记账(`:182`)在 @MainActor;`popToHome`(`+Navigation.swift:56`)不停流。→ **v4 由 B② 停流根治(§2)。**
- **通道2 @Published 写 → god-object objectWillChange**：`applyThreadRenderSnapshot`(`ThreadStream.swift:255`)每变化帧写 `@Published`。`GaryxShellView` 主体已被 `.equatable()`(`Views.swift:81`,`==` 只比 store 引用 `:488`)隔离,但**漏网订阅点(首页静止态可达,全部要断)**:`GaryxRootView`(`Views.swift:13/19`)、footer `GaryxSidebarThreadAutoLoadFooter`(`SidebarViews.swift:845`)、debug binding(`Views.swift:79`)、**全局 error toast `GaryxGlobalErrorToastHost`(`StatusComponents.swift:92`,codex #1140 补)**、running badge 30fps `TimelineView`(`SidebarViews.swift:1648/1684`)。
- **通道3 run-state 写主线程 O(N) projection**：`runStateByThread` didSet(`Model.swift:132`)→`homeThreadRunningThreadIds` O(N) 三源扫(`Presentation.swift:79`)+`IdentityKey` 全量 `displayThread` 拷贝(Core `:435`)。

## 2. 最干净目标架构(v4：砍 M3,两层 actor)

> **修订1(Gary 发现,砍冗余)**：v3 的 `GatewayStreamActor`(搬 byte-loop)对首页**冗余**——B② 停流后首页静止态无流,通道1 已由停流根治;搬 byte-loop 只优化会话页、不属本任务。**砍掉,少一层 actor。** 会话页打开时流仍在 @MainActor 处理会话 transcript(原样),仅**额外 emit run-state delta 事件**给 HomeProjectionActor(不搬 byte-loop)。

```
        后台 (off MainActor)                       @MainActor (纯哑渲染)
 ┌──────────────────────────────┐       ┌────────────────────────────────┐
 │ HomeProjectionActor(单写者)   │ hop   │ HomeStore: @Published snapshot │
 │  纯 HomeProjectionReducer(Core)│snapshot│  appliedSeq 守 / apply diff    │
 │  三源 source-precedence 折叠   │+diff  └───────────────┬────────────────┘
 │  不可变 HomeSnapshot + Diff    │+seq                   ▼
 └──────────────┬───────────────┘        ┌────────────────────────────────┐
   事件 mailbox ▲                         │ native List(真 cell 复用)       │
   ← recent ingest / pins / run-state     │  喂 HomeSnapshot;滑动=布局+绘制 │
     / 乐观 pin·archive·rollback           └────────────────────────────────┘
     / visibility / loading
```

**五个"最干净"落点(全部到位):**
1. **数据全搬后台**：首页状态所有权在 `HomeProjectionActor`,O(N) busy-scan + IdentityKey 拷贝离开主线程。
2. **UI 纯哑渲染**：`HomeStore` 只 apply 不可变 snapshot + diff。
3. **god-object 一个不留**：root/footer/debug/**error toast**/pagination 全迁 `@Observable`/窄 store。
4. **流彻底停(B②)**：`popToHome` 即 `stopSelectedThreadStream`。**修订2(codex BLOCKER1)：停流前必须 drain/commit pending committed window,或把 final-flush 会话副作用(provider ack/title/runTracker complete/terminal cleanup)转为独立 main-actor lifecycle event——不能简单 `cancel` flush task(`ThreadStream.swift:64` 当前直接 cancel `:292` 的 flush,会丢这些副作用)。** running dot 由 recent_threads 投影列 + reconcile 供给(§6.1)。
5. **真 cell 复用**：native `List`;timestamp 烘进 row(reducer 每分钟 tick 防相对时间冻结);badge 折叠 list 级单一 phase(O(1))。

## 3. 实现层坑解法(codex 5 findings + Gary)
- **§3.1 run-state 副作用边界(双向确认可拆)**：`applyTranscriptRunState`(`Threads.swift:1266`)拆「首页 projection 输入(:1271 busy)」vs「会话生命周期副作用(:1274-1301 ack/title/terminal/runTracker)」,只前者入 actor。**修订2 补**:停流 drain 时这些 final-flush 副作用要保证执行(见 §2.4)。
- **§3.2 archive 回滚边界(修订3,codex BLOCKER2)**：归档 selected thread 失败回滚,当前只恢复 pins/recent/threads/lastOpened/widget/error,**漏恢复 `selectedThread`/`messages`/`draftThreadTitle`/composer/navigation**(`Bots.swift:260/266/296`,`Threads.swift:707`)。乐观 archive 作为 mailbox seq-stamped 事件,回滚事件**必须覆盖这些会话面状态**,边界写进不变量。
- **§3.3 三源折叠(修订4,codex MAJOR4,Gary 原写错)**：**不用 max-seq-wins**——三源无统一可比 seq(`runTracker` 状态机无 transcript seq `ConversationRunTracker.swift:78`、committed 有 seq、recent 只有 projection)。改 **source precedence**:本地乐观 `runTracker.isThreadBusy` > 网关流 committed `runStateByThread.busy`(带 seq) > `recent_threads` projection;跨源固定优先级、不比 seq,每源内部用自己 epoch/新鲜度。反例守:local busy 后旧 recent idle 晚到,**precedence 保证 idle 不覆盖 busy**(max-seq 会错覆盖)。**修订6(claude #1144,对称压制)**:可见布尔须用**压制语义而非 OR**——visible = 拥有新鲜槽的最高优先级源的 running 布尔;低优先级源**仅当**所有更高优先级源都无新鲜槽时才参与。OR 点亮会让陈旧"忙"盖掉新鲜"闲"(#1140 的镜像 bug),**busy→idle 与 idle→busy 两方向都要对称压制**。补判别性测试:高优先源新鲜 idle + 低优先源新鲜但内容陈旧 running → **visible IDLE**。
- **§3.4 shadow parity transaction 边界(修订5,codex MAJOR3)**：`refreshThreads`(`Threads.swift:187`)跨 await 依次写 isLoadingThreads/pins/recent/threads/selectedThread/loading,每个 didSet 触发 home apply。actor coalesce 中间态,parity **必须有 explicit transaction id / end marker**,按 transaction 边界比完整 rendered HomeSnapshot + side-effect counters,而非 per-write 比中间态。
- **§3.5 home visibility 事件**：`HomeProjectionEvent` 含 `homeVisibilityChanged`/`navigationChanged`(`isHomeVisible` 驱动 silent refresh)。

## 4. 关键不变量
- **单写者 + monotonic appliedSeq**：所有首页输入经单一 mailbox + 单 reducer;HomeStore 仅 appliedSeq 更大才 apply。乐观回滚更晚 seq 入同一 mailbox,不带外写。
- **三源 source-precedence(非 max-seq)**：见 §3.3,跨源固定优先级、每源内部新鲜度。
- **run-state 副作用隔离**：只首页 projection 输入入 actor,会话生命周期副作用(含停流 drain 的 final flush)留 @MainActor。
- **稳定行身份**：row.id==thread.id 永不漂移;跨 section id 唯一。
- **render_state 服务端权威 / origin_id 零抖动 / recent_threads 写时投影 / Core 分层**：全保。
- **god object 一个不留**：root/footer/debug/error toast/pagination 全离 god object @Published。
- **archive 回滚完整**：含 selectedThread/messages/draft/composer/navigation(§3.2)。

## 5. 迁移(v4：砍 M3 后 5 步,shadow 分步,每步单 flag 回滚,全部执行到纯架构)
| 步 | 内容 | gate | 回滚 |
|---|---|---|---|
| **M1** | Core 纯 `HomeProjectionReducer`(增量、**三源 source-precedence**、稳定 id、visibility 事件) | reducer parity vs 老 store + churn→0 + off-main + lost-update + **'旧idle不覆盖新busy'跨源用例** | 删新 Core 文件零引用 |
| **M2** | `HomeProjectionActor`(单写者 mailbox)+ shadow dual-emit。parity 含 **transaction 边界**(§3.4) | parityMismatchCount==0(按 transaction 边界)+ snapshotEmitCount==1(2s-drain) | flag 关 dual-emit |
| **M3** | `@Observable` **断 god object 一个不留**:root+footer+debug+drawer+**error toast** | `withObservationTracking` 双周期 + xcodebuild SUCCEEDED | 子视图恢复 @EnvironmentObject |
| **M4** | **cutover**:HomeStore 渲染 actor snapshot,移除主线程 didSet;run-state 经 actor 事件 + **B② 停流(补 drain §2.4)** | 真机 probe model_publish→~0 + home_store_apply≈0;停流 drain 不丢 ack/title/terminal;真机二分根因消失 | 单 flag 翻回老路 |
| **M5** | native `List` + badge 折叠 + timestamp 烘 row(reducer tick) | Instruments hitchTimeRatio < 改前 baseline;packaged-app swipe/pin/archive 行为校验 | 回退 body 到 LazyVStack |

pbxproj 随新文件 `xcodegen generate` 并提交。

## 6. 待老板拍 / 实现期定
### 6.1 ★远端 running dot 延迟(待老板拍,v4 暂按"接受"写)
B② 停流后,远端/别设备 running 线程的 dot 靠 `recent_threads` 投影列,而 home silent refresh 是 **10s interval**(`SidebarViews.swift:315/435`)、非实时。**Gary 暂按「接受 10s 延迟」定稿**(扫一眼列表够用、最符合"最干净停流")。若老板要更快 → 需加 push 推送或缩短 reconcile SLA(多成本、没那么干净),则 §2.4/§6.1 相应加机制。
### 6.2 native List swipe(M5 实现期)
native `.swipeActions` vs 自绘 `GaryxSwipeActionRow`(`SidebarViews.swift:473`)行为差异;最干净取向=统一 native。
### 6.3 hitchTimeRatio 阈值(M5 真机 baseline 定)

## 7. 三问
- **像 Telegram**：滑动主线程零数据活(数据搬 actor、O(N) 消失、objectWillChange 全断、流停)+真复用+badge O(1)。
- **根治非补丁**：状态所有权搬后台 + 失效粒度换基底 + 流停,全架构迁移,无稳态阀门;砍 M3 更少改动。
- **不改 bug**：单写者+monotonic seq+同 reducer 乐观=lost-update/乱序免疫;三源 precedence(非错的 max-seq);停流 drain 不丢会话副作用;archive 回滚完整;render_state/origin_id/recent_threads/Core 分层全保;每步 shadow parity(transaction 边界)+单 flag 回滚。
