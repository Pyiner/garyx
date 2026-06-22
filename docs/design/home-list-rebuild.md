# iOS 首页线程列表滚动卡顿根治 — 唯一最终方案

## 0. 目标与边界

让 garyx iOS 首页线程列表(`GaryxHomeThreadListView`)滑动像 Telegram 一样顺,彻底根治「开一个 running 线程 → 返回首页(popToHome) → 滑/静止2s/恢复滑」时的「静止2秒→恢复滑卡几秒」卡顿。不是局部优化,是数据层架构重构 + 失效粒度重构,一次做对,不再贴补丁。

保留并复用已被验证做对的成果(不推倒):窄 `@ObservedObject` store + `.equatable()` 边界(`GaryxRootNavigationView`/`GaryxHomeThreadListView` 已只观察窄 store)、per-row `.equatable()`(`GaryxHomeThreadButton`)、store 双重去重(`previousInput==input` / `snapshot != next`)、4b00055f 的 off-main decode/merge/widget/avatar、reconcile 间隔放宽。

## 1. 经代码核实的真凶(main HEAD ee08b590)

「静止2s→恢复 hang」是一拨同步主线程 burst,由三条独立通道在「running 选中线程 + 已 popToHome 回首页」场景汇聚:

**通道1 — 流活在主 actor,且 popToHome 不停流。** `popToHome()`(`GaryxMobileModel+Navigation.swift:56-64`)只 pop navigationState + cancel reconcile/workflow poll,**不清 selectedThread、不调 stopSelectedThreadStream**;流只在 selectedThreadId 变 nil 时停(`GaryxSelectedThreadStreamPolicy`)。`runSelectedThreadStream` 的 `for try await line in bytes.lines`(`ThreadStream.swift:122`)与 per-event seq fixup(176-208)跑在 `@MainActor`;**4b00055f 已把 JSON decode(157-159)与 transcript merge(240-249)搬到 `Task.detached(.utility)` 离主线程,但 byte-line 迭代、per-event 记账仍在主 actor。**

**通道2 — 每帧/每3s 的 @Published 写触发 god-object objectWillChange 风暴。** `applyThreadRenderSnapshot`(255-273)**每帧** 写 `@Published renderSnapshotsByThread` + 构 `GaryxCachedTranscript` 拷贝 base.messages;3s leading-throttle flush(`flushSelectedThreadStreamWindow` 292-306)写 `@Published messages`(`setPreparedMessages`)+ `applyTranscriptRunState` 写 `@Published runStateByThread`。`GaryxMobileModel` 是单一 `ObservableObject`,任一 @Published 写 fire 单个 objectWillChange,reach `GaryxRootView.body`(`GaryxMobileViews.swift:12-13` 持 `@EnvironmentObject model` 读 `hasGatewaySettings`/`connectionState`)与任何已实现的 drawer。**关键:`applyTranscriptRunState` 在状态未变时早返回(`Threads.swift:1268`),所以「稳态 running 线程」每帧残留成本主体是 renderSnapshot/messages 写的 objectWillChange,不是 didSet 的 O(N) 重建。**

**通道3 — 每次 run-state 写在主线程重跑 O(N) 首页 projection。** `selectedThread` didSet(`GaryxMobileModel.swift:93-98`)同时 `applySelectedThreadStreamPolicy` + `refreshHomeThreadListSnapshot`;`runStateByThread` didSet(132-134)也 refresh。一次 flush 同时写两者 → refresh 跑两遍。`refreshHomeThreadListSnapshot`(`Presentation.swift:55-60`)在主线程构 `homeThreadListInput`,含 `homeThreadRunningThreadIds`(79-95)对全量 threads 的 O(N) busy-scan(三源:runTracker / runStateByThread / isThreadSummaryRunning)。`homeThreadListStore.apply` 里 `previousInput == input`(Core `:473`)→ `GaryxHomeThreadListInput.==`(`:197`)→ 构 `GaryxHomeThreadSectionsIdentityKey`(`:208/435`)做 `input.threads.map(displayThread)` — **全量 thread struct 拷贝,无条件每次 apply 都跑**(`sectionsCache` 内部缓存 `:415` 只挡 `build`,挡不住这个 `==` 比较的 IdentityKey 重建)。

「静止→恢复」形状:3s flush 是 leading-throttle;静止时 committed 帧累积进 `cachedTranscriptSnapshots`,flush 边界(常与恢复滑的 runloop 唤醒对齐)把累积窗口一次性 drain → 一拨主 actor @Published 写 → objectWillChange 风暴 + O(N) IdentityKey 重建,落在一个 runloop turn = 多秒 hang;静止越久窗口越大、burst 越大;sim 快 CPU 吸收绝对值故不复现,真机由 N/120Hz 放大。

**为什么换列表容器(A/B)不够:** A/B 只换首页列表容器(LazyVStack→List/UICollectionView),切不掉通道2 的 objectWillChange 风暴(它经 GaryxRootView/drawer 而非列表)。必须同时:(a) 把首页状态搬离主 actor(治通道1的首页副作用 + 通道3),(b) 用 @Observable 切断 objectWillChange 到首页路径(治通道2),(c) 真 cell 复用(滑动帧预算)。

## 2. 架构

```
                  后台 (off MainActor)                         @MainActor (UI)
  ┌──────────────────────────────────────┐         ┌──────────────────────────────────┐
  │ GatewayStreamActor                    │ run-state│ @Observable ConnectionModel       │
  │  byte-loop + seq planner + decode(已off)│ delta   │ @Observable DrawerChromeModel     │
  │  posts: render 帧 → 会话/render 路径    │  事件    │   ← GaryxRootView/drawer 窄观测     │
  │         run-state delta ───────────┐  │         │     (切断 objectWillChange 风暴)    │
  └──────────────────────────────────┐ │  │         └──────────────────────────────────┘
                                      ▼ ▼  │
  ┌──────────────────────────────────────┐│         ┌──────────────────────────────────┐
  │ HomeProjectionActor (单写者)          ││  hop    │ HomeStore (@MainActor)            │
  │  state: threads/recentIds/pinnedIds/  │└────────▶│  @Published snapshot              │
  │         runState(三源)/selectedId      │ snapshot │  appliedSeq 守(丢 stale)          │
  │  single mailbox ← 所有事件(seq 戳)     │ +diff    │  apply 预算 diff(O(changed))      │
  │  HomeProjectionReducer(纯, Core)       │ +seq    └───────────────┬──────────────────┘
  │  CollectionDifference(稳定 thread.id)  │                         ▼
  │  emit HomeSnapshot? (未变则 nil)        │         ┌──────────────────────────────────┐
  └──────────────────────────────────────┘         │ 哑视图: native List(真 cell 复用)  │
   ingest 事件 ← refreshThreads/pins/乐观pin/archive  │  喂同一 HomeSnapshot;滑动=布局+绘制 │
                                                     └──────────────────────────────────┘
```

`GatewayStreamActor` 与 `HomeProjectionActor` 是两个 Swift `actor`;`GaryxMobileModel` 退化为 `@MainActor` 薄 facade:转发用户意图为事件、渲染 actor publish 的 snapshot,不再 owns 首页状态。

## 3. 关键类型与不变量(详见 keyInvariants 字段)

- `HomeProjectionEvent`(Core, Sendable, 每个带 actor 赋的 monotonic seq):`recentThreadsIngested / pinsChanged / runStateDelta(带 basedOnSeq) / selectedThreadChanged / optimisticPin / optimisticArchive / optimisticRollback / loadingChanged`。
- `HomeProjectionReducer.reduce(state, event) -> (state, HomeSnapshot, CollectionDifference?)`:纯、确定、复用 `GaryxHomeThreadSectionsBuilder/Cache`;run-state **三源全折**(runTracker / runStateByThread / isThreadSummaryRunning),per-thread max-seq-wins。
- `HomeSnapshot` = 现有 `GaryxHomeThreadListSnapshot`(不可变、Sendable、Equatable),复用不改。
- `HomeProjectionActor`:owns state、drain mailbox(fold 全部 pending 再 diff,burst 坍缩成一个 snapshot)、算预算 diff(**稳定 thread.id 身份、同 id 行内容原地更新、无魔法阈值**)、hop 进 HomeStore。
- `HomeStore`(@MainActor,现有 store 提升):`@Published snapshot` + `appliedSeq` 守(丢 stale)+ apply 预算 diff。

**行 VM:** `GaryxHomeThreadRow` 保持 `Identifiable(id:thread.id)+Equatable+Sendable`。唯一改动:timestamp 在 reducer 里烘进 row(移走 `GaryxHomeThreadButton.body:472` 的 `garyxFormattedTaskTimestamp`),body 纯渲染零格式化。**注意 timestamp 是相对 now 的('2m'/'3h'),烘成静态后需 reducer 每分钟 tick 重投或 cell-local TimelineView 从 Date 格式化,否则首页停留时相对时间戳冻结(评审抓到的 reachable 回归)。**

## 4. 切断 objectWillChange 风暴(嫁接 Route C,窄用)

per-frame `setRenderSnapshot`/`setPreparedMessages` 写仍存在(会话面需要),但用 iOS 17 `@Observable`(floor 已核实 `.iOS(.v17)` + 8× `IPHONEOS_DEPLOYMENT_TARGET=17.0`,零成本)把 `GaryxRootView` 自身两个读 + drawer/shell chrome **窄迁移**到各自 `@Observable` model,使转写帧的 @Published 写不再唤醒首页路径任何 body。**不做全量 god-object @Observable 化**(剔除 Route C 高风险大迁移面:实测 22 处 `@EnvironmentObject` + 22 处 `$model.` binding,漏改静默停更新不编译报错)。

## 5. 主线程在滑动时做/不做

**做:** native List/UICollectionView 布局+绘制(真 cell 复用,off-screen 行回收);收 ≤1 snapshot/coalesced tick,apply 预算 diff(O(changed));per-row `==`;list 级单一 phase 的 typing badge tick(折叠 per-row 30fps TimelineView,O(1));NSCache avatar 命中测试(miss off-main)。

**不做:** JSON decode(off-main)、transcript merge(off-main)、`GaryxHomeThreadSectionsBuilder.build`(actor)、`homeThreadRunningThreadIds` O(N) 扫(actor 事件驱动)、`IdentityKey` O(N) displayThread 拷贝(actor)、god-object objectWillChange(@Observable 切断)、timestamp 格式化(烘进 row)、widget 序列化(off-main)。

## 6. popToHome 后的流(老板拍板点,见 openDecisions)

两选一:A) 流续连、只把 run-state 喂 actor、转写副作用不写 god-object;B) 停转写流、保 `selectedThreadStreamCursor`、dot 改由 recent_threads 投影列 + 15s reconcile 兜。推荐 A(配合 @Observable 后转写写已不唤醒首页)。**剔除原稿「heartbeat cadence」旋钮**(throttle 复活);若选 A 则 run-state 持续作事件流入、由 coalescing 而非 cadence 吸收 burst。

## 7. 三问

**像 Telegram:** 滑动主线程零数据活(状态搬 actor、burst 在 Core 坍缩 snapshotEmitCount==1)+ objectWillChange 切断(@Observable granularity gate 证)+ 真 cell 复用(native List)+ badge O(1)。**根治非补丁:** 状态所有权搬离主 actor + 失效粒度换基底,无任何 gate/flag/freeze/throttle;剔除两个隐性旋钮。**不改出新 bug:** 单写者+monotonic seq+同 reducer 乐观 = lost-update/乱序结构性免疫;三源 run-state 全折;render_state 哑渲染/origin_id 零抖动/recent_threads 写时投影/Core 分层全保;shadow parity gate(parityMismatchCount==0)cutover blocker、绝不双渲染、每步单 flag 回滚。

## 8. 迁移(详见 migrationPlan)

6 步,1-4 全 shadowable+回滚,步骤2-3 compute-but-not-render parity gate,步骤5 数据层 cutover、步骤6 换 List。pbxproj 须随新文件 `xcodegen generate` 并提交(TestFlight CI 不跑 xcodegen,漏了 swift test 假绿)。真机二分(临时禁 actor 喂流 / 临时停 popToHome 流)确认根因消失。

---

## 附录 A — 迁移步骤(migrationPlan)

### [1]
GaryxMobileCore 抽纯 HomeProjectionReducer + HomeProjectionEvent enum(recentThreadsIngested/pinsChanged/runStateDelta/selectedThreadChanged/optimisticPin/optimisticArchive/optimisticRollback/loadingChanged),(state,event)→(newState, HomeSnapshot, CollectionDifference?),稳定 thread.id 身份,复用 GaryxHomeThreadSectionsBuilder/Cache;三源 run-state max-seq-wins 全折。无 app 接线。headless SwiftPM 测 reducer 等价(parity corpus)+ off-main guard + lost-update 交错 + 三源 run-state。

- **gate:** reducer parity vs 老 store 输出 + off-main(Thread.isMainThread==false)+ lost-update 交错通过
- **rollback:** 删新 Core 文件,无引用、零行为变化

### [2]
引入 HomeProjectionActor(后台,owns 首页状态 + reducer + 预算 diff,full-reload 无魔法阈值)+ GaryxHomeProjectionGateway(@MainActor,捕获事件入单 mailbox,single-flight latest-wins)。SHADOW:现有 didSet 站点同时 emit 事件入 actor,actor 算 snapshot+appliedSeq,UI 仍渲染老 GaryxHomeThreadListStore。加 parityMismatchCount 断言(actor snapshot vs 老 store per appliedSeq)。

- **gate:** parityMismatchCount==0 over captured corpus + snapshotEmitCount==1(2s-drain replay)
- **rollback:** feature flag 关掉 dual-emit,老路权威不动

### [3]
SSE 流 byte-loop + per-event seq 记账搬进 GatewayStreamActor(离 @MainActor),posts (a) 不可变 render 帧给现有会话/render 路径 (b) 窄 run-state delta 作事件给 HomeProjectionActor;GaryxStreamSeqPlanner/resume-override/cursor/404-fallback/control-rewrite-refetch 逐字保留。仍 shadow:断言 emit 的 message id 与 resume cursor 与老路 byte-identical over 含 gap-reconnect/rewrite-refetch/404 的 corpus。

- **gate:** message ids + resume cursors 与老路逐字相等(含三条 self-heal 路径)
- **rollback:** StreamActor 回退到 @MainActor 扩展方法(flag),run-state 仍走老 @Published

### [4]
@Observable 窄迁移(嫁接 Route C 精华):GaryxRootView 自身两个读(hasGatewaySettings/connectionState)迁到窄 @Observable connection/identity model;drawer/shell 子视图从 @EnvironmentObject god-object 迁到窄 @Observable chrome model。枚举每个迁移视图读的属性。接 withObservationTracking granularity gate。NOT 全量 god-object @Observable 化。

- **gate:** withObservationTracking 双周期断言(转写写不失效首页 + 首页写确实失效首页);xcodebuild SUCCEEDED(防漏 binding 静默停更)
- **rollback:** 受影响子视图恢复 @EnvironmentObject model(机械回退);chrome model 并行填充故过渡期仍正确

### [5]
CUTOVER(gated on 全部 CI gate + parityMismatchCount==0):HomeStore(@MainActor)改从 HomeProjectionActor publish 的不可变 snapshot+diff 渲染,移除 ~11 处主线程 refreshHomeThreadListSnapshot didSet;run-state 只经 actor 事件流。LazyVStack 暂留(隔离数据层 cutover 与 render-engine 切换)。按 openDecision 拍板处置 popToHome 后的流。

- **gate:** 真机 probe model_publish→~0 + home_store_apply 主线程时长≈0(popToHome+running 窗口);真机二分确认根因消失
- **rollback:** 单 flag 翻回老 store.apply 路 + 恢复 didSet;actor 留作 dead code

### [6]
换 render engine(嫁接 Route A):ScrollView+LazyVStack→native List(真 cell 复用),喂同一 HomeSnapshot;timestamp 在 actor 烘进 row(移走 body 内 garyxFormattedTaskTimestamp);typing badge 折叠成 list 级单一 phase 驱动(O(1));swipe 按 openDecision 定 native 还是保自绘。pbxproj 须 xcodegen generate 并提交。

- **gate:** Instruments Animation Hitches / XCTHitchMetric hitchTimeRatio < 改前同机 baseline;packaged-app 视觉 parity + swipe/pin/archive 行为校验
- **rollback:** 只回退 view body 到 LazyVStack(同样消费 HomeSnapshot),数据层步骤1-5不动、仍享 off-main 收益

## 附录 B — 关键不变量(keyInvariants)

- 单写者:HomeProjectionActor 是首页状态唯一 mutator,所有输入(recent ingest / pins / run-state delta / 乐观 pin/archive / 乐观失败回滚)作为 seq-stamped 事件经单一 mailbox + 单一 reducer;消除当前 runStateByThread 与 selectedThread 双 didSet 触发的双重 refresh。乐观失败回滚必须作为更晚 seq 的事件入同一 mailbox,不得带外写 actor 状态。
- monotonic appliedSeq:每个 HomeSnapshot 带 actor 赋的单调递增 appliedSeq;@MainActor HomeStore 仅在 appliedSeq > 当前时 apply,慢 build 后到不会覆盖新 build(async-hop 乱序覆盖防护)。actor→main 的赋值必须经单一 serialized mailbox 或带 seq 比较,杜绝「compute 有序但 assignment 乱序」。
- 三源 run-state 全折:RunningThreadIds reducer 必须折 runTracker.isThreadBusy(本地乐观)+ runStateByThread[id].busy(网关流)+ isThreadSummaryRunning(thread)(recent_threads 投影 runState 列)三源;漏任一源会丢 running dot(尤其只靠 summary 列上点的远端 running 线程)。
- 稳定行身份:GaryxHomeThreadRow.id == thread.id,永不取自 seq 或 reconcile 漂移;CollectionDifference 与 List 复用依赖 id 稳定,漂移既破 diff 又重引 origin_id user-row 抖动类 bug。
- 乱序/迟到帧:run-state 按 per-thread max(basedOnSeq)-wins;GaryxStreamSeqPlanner.decide/connectionLastSeq/resume-override/404-fallback/control-rewrite-refetch 逐字保留在 actor 内,只搬执行线程不改 seq 记账,S5 resume cursor 不变。
- render_state 服务端权威:actor/会话面只存服务端 render snapshot 并经 GaryxMobileRenderStateMapper 哑映射;首页只消费 run-state 布尔;不引入任何端侧 user-turn/tool/tail-thinking/final 重算。
- empty-diff 真 no-op:reducer fold 后状态未变则不 emit snapshot,主线程不 apply、零 cell 重配;现有 churn-300→0 publish 验收测试移植到 actor 层断言 0 apply。
- @Observable 失效隔离:首页路径上任何 body 不得读跨多个存储属性的计算属性(否则 Observation 重新订阅全部、悄悄恢复粗失效);每个迁移视图读的属性须枚举审计,withObservationTracking gate 钉死「转写写不失效首页」。
- 跨 section id 唯一:pinned∪recent 不得含同一 thread.id(否则 List/diffable 重复 Identifiable id 行为未定义);pin/unpin 跨 section 移动作为单次 snapshot 内的 remove-old/insert-new。
- popToHome 后流处置一旦定为「续连只喂 run-state」,则该流 run-state 必须持续作为事件流入 actor(非降 cadence),由 coalescing 吸收 burst。

## 附录 C — 测量基线(measurementBaseline)

确定性测量分两层,CI gate 决定 cutover,真机 gate 做最终确认,先决定性后真机:

CI 决定性(SwiftPM,cutover blocker):
1. burst 坍缩 gate:replay 录制的「running 线程 open → popToHome → 2s暂停 → drain」事件序列穿过 HomeProjectionReducer,断言 N 帧入 → snapshotEmitCount==1、diff 数==1。证 burst 在 Core 坍缩、不在主线程。
2. off-main gate:在 Task.detached(.utility) 内对 200 行 fixture 跑 reducer+diff 断言 Thread.isMainThread==false(扩 testTranscriptPreparationCanRunOffMainActor:298-312 模式)。证 O(N) projection 结构上离开主 actor。
3. churn=零主线程活 gate:喂 300 run-metadata-only churn 事件断言 mainThreadProjectionCount==0,且首页路径不订阅 god-object objectWillChange(移植现有 churn-300→0 publish 验收)。
4. @Observable granularity gate:withObservationTracking 读 homeStore.snapshot,写 conversation renderSnapshot/messages,断言 onChange 未触发(须用两个注册周期分别证「该醒的醒、不该醒的不醒」,避免 one-shot 假绿)。device-free 证转写写无法失效首页。
5. lost-update gate:交错「乐观 pin 事件夹在两个 ingest 之间 + 乐观失败回滚事件」,断言最终 snapshot == 按 seq 最后赢者;两个乱序 hop 断言旧 appliedSeq 被拒。
6. parity gate(cutover blocker):新管线 compute-but-not-render,对录制真实 session 事件 log 断言新 HomeSnapshot.sections == 老 store.snapshot.sections per appliedSeq(parityMismatchCount==0)才切渲染源。

真机确认(handoff,非 per-commit):
7. 现有 GaryxHomeScrollPerformanceProbe(已核实 model_publish/home_store_apply/CADisplayLink hitchTimeRatio)在真机 swipe→静止2s→swipe 场景:断言 popToHome+running 线程窗口内 model_publish→~0、home_store_apply 主线程时长≈0、hitchTimeRatio 严格低于改前同机 baseline。
8. 换 native List 后(MEMORY:SwiftUI ScrollView 不发 scroll 信号是历史盲区,List 是真 UICollectionView 才发)用 Instruments Animation Hitches / XCTHitchMetric 量 hitch,作为「像 Telegram」的真机判据。

二分确认根因:临时禁后台 actor 喂流 / 临时停 popToHome 后的流,看 hang 是否消失,确认是哪条通道(流逐帧 vs objectWillChange 风暴)主导——MEMORY 记录 sim 快 CPU 吸收绝对卡顿、真机由 N/120Hz 放大,必须真机二分。

## 附录 D — 需老板拍板(openDecisionsForOwner)

1. popToHome 后选中线程的 SSE 流如何处置(两选一,有真实权衡,我不替你拍):A) 流继续连着、只把 run-state 作为事件喂 projection actor、转写副作用不再写 god-object(实现简单、running dot 即时、但选中线程的逐帧 byte-loop 仍在主 actor 醒,且多次开线程后多条流累积);B) popToHome 停转写流、保 selectedThreadStreamCursor、running dot 改由 recent_threads 投影列 + 15s reconcile 兜(彻底无主线程流活、但选中线程刚返回首页时 running dot 从流级实时降到 poll 级延迟,且重开线程要 cursor-resume)。我推荐 A(配合 @Observable 切断 objectWillChange 后,选中线程的转写写已不再唤醒首页),但 B 更彻底——是否接受 B 的 dot 延迟由你定。

2. 最终步换 native List 的范围:native .swipeActions 与现有自绘 GaryxSwipeActionRow(已核实 SidebarViews.swift:473,另一处 1399 复用)行为不完全一致(full-swipe/回弹/长按 archive confirmation 的 per-row presentation 在复用池里需重接线)。是接受首页 swipe 用 native(与非首页列表 swipe 行为分叉),还是要求两处一致(成本更高)?或先只做数据层+@Observable(步骤1-5)、List 容器留作 measurement-gated 可选(若真机 hitch gate 在数据层切完后已达标就不做)?

3. @Observable 迁移的边界:本方案只把 GaryxRootView 自身两个读 + drawer/shell chrome 窄迁移到 @Observable,不全量 god-object @Observable 化(剔除 Route C 高风险大迁移面)。但若你希望一次性把整个 god object @Observable 化(收益是所有面板都受益、代价是 22 处 binding 的 silent-failure 面),这是范围权衡,需要你定到哪一层为止。

4. 测量基线与达标阈值:CI 主 gate 用 Core snapshotEmitCount==1 + probe model_publish→0(确定性);真机用 XCTHitchMetric/Animation Hitches 的 hitchTimeRatio 严格低于改前同机 baseline。hitchTimeRatio 的具体预算阈值(例如 <0.05)需要你认可的真机基线数,我不替你定数。

## 附录 E — 路线选择

**主干:** 
主干选 Route D(三层后台数据-actor:GatewayStreamActor → HomeProjectionActor → @MainActor HomeStore + 单写者 mailbox + 不可变 HomeSnapshot),嫁接 Route C 的 @Observable 细粒度失效(只用在已经被验证为真凶残留的「god-object objectWillChange 风暴」通道,作为 Route D 之上的第二刀),最后用 Route A 的 native List 做真 cell 复用作为可选最终步(measurement-gated)。

为什么不是 A/B:经代码核实(GaryxMobileModel+ThreadStream.swift),applyTranscriptRunState 在状态未变时早返回(Threads.swift:1268),所以「稳态 running 线程」每帧主线程残留成本的主体不是 didSet 的 O(N) 重建,而是 applyThreadRenderSnapshot 每帧写 @Published renderSnapshotsByThread + 3s flush 写 @Published messages,这两个写触发 god-object objectWillChange,穿透到 GaryxRootView.body(Views.swift:12-13 持 @EnvironmentObject model 读 hasGatewaySettings/connectionState)和任何已实现的 drawer。A/B 只换首页列表容器(LazyVStack→List/UICollectionView),无法切断这条 objectWillChange 通道——四组评审里 A/B 的 contracts/perf lens 都点出「shell 仍 @EnvironmentObject god object、A/B 自认延后处理」是 mitigate 非 root-cause 的最大缺口。Route D 把首页状态所有权物理搬离 @MainActor god object + Route C 的 @Observable 切断 objectWillChange,两刀合起来才能让「静止2s→恢复」那一拨 burst 在主线程无活可干。

为什么 D 当主干而非 C 当主干:Route C 的 95→@Observable 迁移面巨大(实测 22 处 @EnvironmentObject + 22 处 $model. 双向绑定),漏改一个 binding 静默停更新且不编译报错,migrationSafety 被多组评审压到 6-7;而 Route D 的 single-mailbox + monotonic-seq + same-reducer-for-optimistic 是被三组评审一致认可的、对 lost-update/乱序最强的结构性答案,rootCauseFit 普遍给 9。所以以 D 的后台数据 actor 为骨,@Observable 只精准用在「切断 objectWillChange 到 GaryxRootView/drawer」这一窄通道(C 的精华),不做全量 god-object @Observable 化(剔除 C 被判高风险的大迁移面)。

**嫁接(graftedIdeas):**
- 从 Route C 嫁接:iOS 17 @Observable 的 per-keypath 细粒度失效——但只精准用于切断 god-object objectWillChange 到 GaryxRootView/drawer 这一条被四组评审一致认定为「A/B 无法覆盖的残留通道」,不做全量 god-object @Observable 化(剔除 C 的高风险大迁移面)。withObservationTracking 的确定性 granularity gate(读首页 snapshot、写 conversation renderSnapshot、断言 onChange 未触发)直接证「转写写无法失效首页」,是 device-free 的根治证明。
- 从 Route C 嫁接:typing badge 从 per-row TimelineView(.animation 30fps)折叠成 list 级单一 phase 驱动 / 移到 render-server CAAnimation off-main——这是 C 独有抓到的、与数据层无关的 scroll-hot-path 残留成本(已核实两处 1648/1684),A/B/D 都漏了。
- 从 Route A 嫁接:native SwiftUI List(UICollectionView 背书的真 cell 复用)作为最终渲染容器,而非 Route B 的全 UIKit UICollectionViewDiffableDataSource 重写——A 的 List 改动面更小、复用既有 GaryxSidebarThreadRowView 视觉零差异,且 A 自带「若 List 高度估算成问题再升级到 UICollectionViewDiffableDataSource 复用同一 HomeSnapshot」的逃生路;数据层 renderer-agnostic 这点 A/D 一致。
- 从 Route A/B/D 共识嫁接:HomeSnapshot 不可变 + 稳定 thread.id 作 diff 身份 + monotonic appliedSeq 防 async-hop 乱序覆盖(lost-update 防护)——四稿都收敛到这套,作为 actor→main hop 的核心不变量。
- 从 Route D 嫁接为骨架:single-mailbox + 同 reducer 处理乐观更新 + max-seq-wins run-state——三组评审一致认可对 lost-update/乱序最强的结构性答案,作为单写者主干。

**剔除(rejectedRoutesWithReason):**
- Route B(全 UIKit UICollectionView + UICollectionViewDiffableDataSource 重写)被否为主干:perf lens 评审点出其「最贵的结构改动(UICollectionView 重写)瞄准了最便宜的通道(首页 projection,实测 recent 上限 30 行、diffable apply 亚毫秒),而真正贵的转写 flush burst 通道(Channel 1)与 UICollectionView 正交」——成本/收益错配。且 UIHostingConfiguration 包 SwiftUI 行仍付 body 成本,headline「像 Telegram」是 contingent(自带「不行就退手写 UIKit cell」逃生口=可能返工)。其真正 load-bearing 的那一刀(把流的首页副作用 scope 掉)不需要全 UIKit 重写就能做。B 的 native List/diffable 思路被 A 以更小改动面吸收。
- Route C 作为主干被否(但精华被嫁接):95→@Observable 全量迁移面过大,实测 22 处 @EnvironmentObject + 22 处 $model. 双向绑定,漏改一个 binding 静默停更新且不编译报错;多组评审 migrationSafety 压到 6-7;S3(drawer/shell 迁移)被标 shadowable:false 且是最纠缠的一步——最可能 ship stale-UI bug 且恰是无法 shadow 验证的一步。改为只窄迁移真凶残留通道(GaryxRootView 两个读 + drawer chrome),其余靠 Route D 的后台 actor。
- 「popToHome 停流」作为根治手段被否(四组评审共识):是 flag 不是架构,拿卡顿换 staleness + 重开 S5 resume 成本 + running dot 失源;保留为老板可选的取舍点 B(若接受 dot 延迟),不作为默认根治。
- Route D 原稿的两个隐性旋钮被剔除:running-state「降到 heartbeat cadence」(评审判 throttle 复活,改为流持续喂事件 + coalescing 吸收 burst);CollectionDifference「超阈值 full-reload fallback」的魔法阈值(对 200 行重排常态触发使 diff 形同虚设,改为稳定 thread.id + 原地内容更新无阈值)。
- 「memoize IdentityKey 更狠 / 把 displayThread 拷贝缓存」被否:仍是 O(N) 主线程活,且不触及 objectWillChange 与流副作用两条 burst 通道,治标。

## 附录 F — 评审矩阵(4 路 × 4 lens verdict)

- **A-swiftui-list**: viable, viable, viable, viable
- **B-uikit-collection**: viable, viable, viable, viable
- **C-observation**: viable, viable, strong, strong
- **D-data-layer-actor**: viable, strong, viable, viable

## 附录 G — 三问详答

### 为什么像 Telegram 一样顺
Telegram 级顺滑 = 滑动时主线程只做布局/绘制 + 真 cell 复用 + 每帧零数据活,三者缺一不可。本方案逐条达成并有确定性 gate 守:

1. 主线程零数据活(根治 burst):首页状态(threads/recentIds/pinnedIds/run-state)所有权搬到后台 HomeProjectionActor;O(N) 的 homeThreadRunningThreadIds busy-scan(已核实 Presentation.swift:79-95)、GaryxHomeThreadSectionsIdentityKey 的 input.threads.map(displayThread) 全量 struct 拷贝(已核实 line 435,且 line 473 的 previousInput==input 比较每次都重建 IdentityKey→拷贝是无条件的)全部移入 actor。actor 把一拨事件 fold 成一个不可变 HomeSnapshot+CollectionDifference 再 hop 回主线程,主线程一次 O(changed) apply。SwiftPM gate:replay「2s暂停后drain」事件序列断言 snapshotEmitCount==1(burst 在 Core 里坍缩,不在主线程)。

2. 切断 objectWillChange 风暴(burst 的第二源):per-frame setRenderSnapshot/setPreparedMessages 写仍存在,但用 Route C 的 @Observable 把 GaryxRootView 与 drawer 子视图从 @EnvironmentObject god-object 改为只观察各自窄 @Observable chrome model,使转写帧的 @Published 写不再唤醒首页路径上任何 body。SwiftPM withObservationTracking gate:读 homeStore.snapshot,写 conversation 的 renderSnapshot/messages,断言 onChange 未触发——确定性证「转写写无法失效首页」。

3. 真 cell 复用(滑动帧预算):最终步换 native List(UICollectionView 背书,off-screen 行真回收,LazyVStack 缺的就是这个),喂同一个 HomeSnapshot;timestamp 在 actor 里烘进 row(移走 GaryxHomeThreadButton.body 的 garyxFormattedTaskTimestamp,已核实 SidebarViews.swift:472),body 纯渲染零格式化;typing badge 从 per-row TimelineView(.animation 30fps)(已核实两处 1648/1684)折叠成 list 级单一 phase 驱动,O(1) 不随可见 running 行数放大。

确定性测量(非靠眼睛):MEMORY 记录 SwiftUI ScrollView 不发 scrollingAndDecelerationMetric 是历史测量盲区;换 native List 后该信号可用,XCTHitchMetric/Animation Hitches 在真机 swipe→静止2s→swipe 上变得有意义,作为真机确认;CI 主 gate 是 Core 的 snapshotEmitCount==1 + 现有 GaryxHomeScrollPerformanceProbe 的 model_publish→0 + home_store_apply 主线程时长≈0。

### 为什么是根治不是补丁
这是「首页状态所有权从 @MainActor god object 物理搬到后台 serial actor」+「用 iOS 17 @Observable 把失效粒度从整对象降到属性」两处架构基底迁移,不是任何 gate/flag/freeze/「滚动时暂停刷新」/throttle 阀门。历史 D1/D2/D7-A 都只降频率(改 didSet 次数、加 .equatable 边界),活仍在主线程,任何 drain burst 把活重聚到恢复边界——正是「静止→恢复」症状成因。本方案让主线程结构上不再持有首页状态、不再有 projection/decode/merge 路径可跑:没有阀门可调、没有主线程快路可饿。明确拒绝四组评审都点过的伪根治(popToHome 停流=拿卡顿换 staleness+S5 成本+dot 失源、是 flag 非架构;throttle applyThreadRenderSnapshot;memoize IdentityKey 更狠=仍 O(N) 主线程活)。@Observable 那一刀同样是机制变更:ObservableObject 只有整对象一个失效粒度、无法「打」上粒度,只能换观测基底,iOS 17 per-keypath 追踪本身即粒度变更。并剔除 Route D 原稿两个隐性旋钮(heartbeat cadence、full-reload 魔法阈值)以保持零阀门。保留复用窄 store/.equatable()/per-row equatable/4b00055f off-main/reconcile 放宽全部成果作为新管线输入与不变量(硬约束6)。

### 为什么不会改出新 bug
不破坏既有契约的逐条论证 + shadow-mode 决定性守卫:

1. 单写者消除 lost-update(评审最担心的「后台预建 snapshot 盖掉主线程新鲜乐观输入」):所有输入(recent ingest / pins / run-state delta / 乐观 pin/archive 及其失败回滚)都作为 seq-stamped 事件进同一个 actor mailbox,经同一个 reducer,后到的 seq 赢。乐观 pin 失败回滚必须也作为更晚 seq 的事件入同一 mailbox(剔除 Route D 原稿对此的含糊),否则会从带外写 actor 重新引入竞态——这一点写进不变量并有交错测试守。

2. 乱序/迟到帧:run-state delta 带 frame.basedOnSeq,reducer 按 per-thread max-seq-wins,迟到的旧帧不能 un-set 新 running 态;GaryxStreamSeqPlanner.decide/connectionLastSeq/resume-override(已核实 ThreadStream.swift:182-218)逐字保留在 actor 内,只搬执行线程不改 seq 记账,S5 resume cursor 与 gap-reconnect/404-fallback 不变。

3. 三源 run-state 必须全折(评审抓到的真 reachable bug):已核实 homeThreadRunningThreadIds 合并 runTracker.isThreadBusy(本地乐观)+ runStateByThread[id].busy(网关流)+ isThreadSummaryRunning(thread)(来自 recent_threads 投影的 runState 列,Threads.swift:632-635)三源。RunningThreadIds 必须三源全折进 reducer,尤其「在别的设备上 running、本地流从未见过」的线程靠 summary 列上点——漏折会丢 running dot。有 Core 测试喂三源事件流断言每步 running 集匹配。

4. render_state 哑渲染不破:首页只消费 run-state 布尔,从不碰 transcript render_state;frame.renderState 仍 verbatim 经现有 GaryxMobileRenderStateMapper 哑映射到会话面(@Observable 改的是 GaryxRootView/drawer 的观测基底,不动 mapper)。无端侧 user-turn/tool/tail-thinking/final 重算。

5. origin_id 零抖动:首页不含 user message 行,结构上在 origin_id 路径外;diffable item id = thread.id(稳定、不被 reconcile/origin_id 重写漂移),message.id='history:seq-1'(已核实 ThreadStream.swift:195-196)与 user-row origin 优先留在会话面不动。Core 测试钉 reconcile 序列下 rowID 集不变。

6. recent_threads 写时投影:actor 的 threads/recentIds/pinnedIds 仍只来自网关 listRecentThreads/listThreadPins ingest 作为事件,不 rescan、不重排;只是 ingest 的输出从 god-object @Published 写变成 mailbox 事件。

7. Core 分层:reducer/diff/snapshot 全在 GaryxMobileCore + SwiftPM(保留 publishCount/sectionDerivationCount/acceptedInputCount 计数器并扩 diffCount/appliedSeq);app target 只放 actor 执行器 + SwiftUI 组合 + UIKit cell 桥接。off-main guard 测试(已核实 testTranscriptPreparationCanRunOffMainActor:298-312 的 Thread.isMainThread==false 模式)扩到断言 reducer/diff 在 actor 上跑。

8. shadow-mode parity gate 是 cutover blocker:新管线 compute-but-not-render,对录制的真实 session 事件 log 断言新 HomeSnapshot.sections == 老 store.snapshot.sections per appliedSeq(parityMismatchCount==0)才切渲染源;绝不双渲染(避免历史 dual-render 抖动)。每步单 flag 回滚到权威老路。

9. 已知坑显式规避:@Observable 迁移漏 binding 静默停更新——本方案只窄迁移 GaryxRootView 自身两个读(hasGatewaySettings/connectionState)+ drawer chrome,不全量迁移,把 silent-failure 面压到最小并枚举每个迁移视图读的属性;.pbxproj 须随新文件 xcodegen generate 并提交(MEMORY 记录 TestFlight CI 不跑 xcodegen,漏了 app 编不到而 swift test 假绿),列入迁移 gate。
