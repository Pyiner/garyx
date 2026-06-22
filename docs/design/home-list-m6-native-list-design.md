# iOS 首页 M6:native List 真 cell 复用 — 设计稿

> 任务 `#TASK-1205`。这是 v4 总设计(`docs/design/home-list-rebuild-v4.md`)§5 迁移表 **M5 行** 的实现
> (任务侧编号 M6;两处编号映射:任务 M6 = v4 §5 的 M5 = native List + badge 折叠 + timestamp 烘 row + swipe)。
> 依据 v4 §2「真 cell 复用」与 §5/§6.2/§6.3。实现者 = claude;设计/代码 review = codex(跨模型)。
>
> **rework v2(闭合 codex #TASK-1207 NEEDS-REWORK 两 finding)**:
> - BLOCKER1(badge 重启)→ §3.2/§3.3:`@State`+`.repeatForever` 改 **`PhaseAnimator`**(无布尔状态、生命周期绑定自动重启)+ packaged 重启复现核验。
> - MAJOR2(timestamp 假绿)→ §4.2/§8/§10:formatter **搬进 `GaryxMobileCore`**(可注入 `now`)、删 App 拷贝、**Core 直测生产实现**。
> - codex 非阻塞确认通过项(List identity/divider 折叠、native swipe parity、平铺非吸顶 header、sim+真机验收分离)维持不变。
>
> **rework v3(闭合 codex #TASK-1207 第 2 轮 MAJOR3)**:§4.2/§8/§10 纠正 Core 文件/app-target 集成——app target 把 `Sources/GaryxMobileCore`
> 编进**同一 app 模块**(显式 pbxproj source membership),故新 Core 文件**必须 `xcodegen generate` 提交 pbxproj 再跑 app xcodebuild**(swift test 自动纳入不够),
> 且 App 调用方**不加 `import GaryxMobileCore`**(同模块直接可见)。badge/timestamp 两原 finding codex 已判 PASS。

## 0. 范围与边界(先说清"不做什么")

**本任务只换 UI 容器,不碰数据层。**

- M6 与 **M5(数据层 actor cutover)并行**。M5 把渲染源切到 `HomeProjectionActor`/`GaryxHomeObservationStore`;
  M6 只把首页**容器**从 `ScrollView + LazyVStack` 换成 native `List`,**渲染源沿用现状**
  `GaryxHomeThreadListStore.snapshot.sections`(`.pinned` / `.recent`,类型 `[GaryxHomeThreadRow]`)。
- 合并后实测确认:M4 分支(本 worktree 已 `git merge garyx/42299b73`)新增了 `HomeProjectionActor`/
  `GatewayStreamActor`/`GaryxHomeObservationStore`,但首页 `GaryxHomeThreadListView` **仍 `@ObservedObject` 渲染
  `homeListStore.snapshot`**——actor 还没成为渲染源。所以 M6 的正确目标就是 `homeListStore.snapshot`,**不改 actor**。
- **整合 seam(给 Gary)**:List 只读 `snapshot.sections.pinned/recent`(`GaryxHomeThreadRow` 数组)+
  `snapshot.isLoadingThreads`。只要 M5 让渲染源继续吐出同形 `GaryxHomeThreadListSnapshot`,List 代码零改动即可接 M5 actor。
- **不合 main**(shadow 分支)。完成后正常结束 run,Garyx 自动 in_review,Gary 验收 + 整合 M5。

**不做**:数据层/actor/三源折叠/停流/archive 回滚边界(都属 M1–M5);其它 3 处 `GaryxSwipeActionRow` 调用方
(bot 抽屉 `:1404`、automation `:1016`、tasks `:290`)——只改 home 一处,`GaryxSwipeActionRow` 类型保留。

## 1. 现状(改前精确锚点)

文件 `App/GaryxMobile/GaryxMobileSidebarViews.swift`(合并后行号):

- **容器**(`:343` `threadListWithBottomBar`):`ScrollView(.vertical){ LazyVStack(spacing:0){ 顶 4pt spacer; sidebarThreadSections; 底 28pt spacer } }`
  + `.scrollDisabled(isSidebarDragActive)` + `.scrollDismissesKeyboard(.interactively)` + `.refreshable`。
- **sections**(`:368` `sidebarThreadSections`):header(`GaryxSidebarSectionHeader`,非吸顶,内联 padding)+
  `ForEach(sections.pinned/recent){ row in if row.showsDivider { GaryxSidebarRowDivider() }; GaryxHomeThreadButton(row:…).equatable() }`
  + 段间 `Color.clear.frame(height:10)` + 末尾 `GaryxSidebarThreadAutoLoadFooter`(`.onAppear` 触发 `loadMoreThreads`)。
  recent 为空时渲染 `GaryxSidebarLoadingRow`/`GaryxSidebarEmptyRow`。
- **行**(`:459` `GaryxHomeThreadButton: View, Equatable`,`== 比 row`):
  `body` 内 `presentation = row.presentation.withTrailingTimestamp(garyxFormattedTaskTimestamp(row.timestampValue))`
  → 包进自绘 `GaryxSwipeActionRow(actions:[pin/unpin, archive])` → `GaryxSidebarThreadRowView`;
  外加 `.onLongPressGesture`(`canArchive && !isRunning` 时)→ `.confirmationDialog` 确认 archive。
- **running badge**:`GaryxSidebarThreadRowView`(`:1549`)avatar overlay 在 `isRunning` 时挂 `GaryxAvatarTypingBadge`(`:1645`);
  其 `body` 用 **`TimelineView(.animation(minimumInterval: 1/30))`**——**每个可见 running avatar 一个 30fps 主线程 tick**。
- **timestamp**:`garyxFormattedTaskTimestamp`(`GaryxMobileDesignSystem.swift:407`)用内部 `Date()` 算相对时间。
  在 `GaryxHomeThreadButton.body` 算一次烘进 `presentation.trailingTimestamp`,由 `trailingMeta`(`:1745`)的 `Text` 渲染。
- iOS deployment target = **iOS 17**(`Package.swift`)。`List`/`.swipeActions`/`.contentMargins`/`TimelineView(.everyMinute)` 全可用。
- 性能 probe:`GaryxHomeScrollPerformanceProbe`(DEBUG)已含 `hitchTimeRatio`(CADisplayLink)+ `markRowBody`/`markHomeBody`/`markHomeListStoreApply`。**沿用,不改**——它是改前/改后对比的判据载体。

## 2. 改动一:`ScrollView + LazyVStack` → native `List`(真 cell 复用)

### 2.1 为什么 List 才是"真复用"
LazyVStack 只"懒创建",离屏行**不真回收**(SwiftUI 持有视图、不复用底层 cell);native `List` 背靠 UICollectionView/UITableView,
**离屏行真回收 + cell 复用 + 发 UIScrollView 滚动信号**(Instruments Animation Hitches / `XCTHitchMetric` 才有数据,这是"像 Telegram"的真机判据)。

### 2.2 容器结构(保持视觉 parity:**非吸顶 header**)
**不使用 `Section`**(`.listStyle(.plain)` 的 `Section` header 会吸顶,与现状内联滚走的 header 不一致)。改为把
header / divider / spacer / footer / 行 **全部平铺成 List row**,header 跟着滚动(parity)。

```swift
private var threadListWithBottomBar: some View {
    List {
        Color.clear.frame(height: 4).accessibilityHidden(true)   // 顶 spacer 行
        sidebarThreadSections                                    // header 行 + ForEach 行 + 段间 spacer + footer 行
        Color.clear.frame(height: GaryxSidebarMetrics.bottomBarClearance).accessibilityHidden(true)
    }
    .listStyle(.plain)
    .environment(\.defaultMinListRowHeight, 0)        // 4/10pt spacer 行按真实高度
    .scrollContentBackground(.hidden)                 // 隐藏 List 系统底,露出 garyxPageBackground
    .scrollDisabled(isSidebarDragActive)              // 拖拽锁(parity)
    .scrollDismissesKeyboard(.interactively)
    .refreshable { await refreshAll() }
}
```

`sidebarThreadSections` 里所有 row 统一加(用一层 `Group` 包住后施加,listRow 修饰会下传到每个叶子 row):
`.listRowInsets(EdgeInsets())` + `.listRowSeparator(.hidden)` + `.listRowBackground(Color.clear)`。
(行视图自身已带内边距;现状 LazyVStack 也不加外距,故零 listRowInsets = parity。系统分隔线全隐,沿用自绘 divider。)

### 2.3 稳定行身份 + divider 折叠
- 线程行身份 = `ForEach(sections.pinned/recent)`,`GaryxHomeThreadRow: Identifiable` → `id == thread.id`。
  recent 已 `filter !pinnedIdSet.contains` → **跨 section id 唯一**(不变量满足)。
- **divider 折叠进行内**:把 `GaryxSidebarRowDivider()` 从「ForEach 里的兄弟节点」搬进 `GaryxHomeThreadButton.body` 顶部
  (`row.showsDivider` 时)。这样**一个 thread.id 恰好对应一个 List cell**(否则 divider 当独立 row 会让"行数 ≠ 线程数"、
  id 难稳、复用错位)。视觉不变:divider 仍在每段第 2 行起出现在行上沿。
- header / spacer / loading / empty / footer 是非 ForEach 静态 row,List 用结构位置赋 identity(树形稳定)。

### 2.4 行内 List 修饰 + swipe(见改动四)
`GaryxHomeThreadButton.body` 改为:
```swift
var body: some View {
    #if DEBUG markRowBody #endif
    VStack(spacing: 0) {
        if row.showsDivider { GaryxSidebarRowDivider() }
        GaryxSidebarThreadRowView(
            presentation: row.presentation,            // 不再 withTrailingTimestamp
            avatar: row.avatar,
            liveTimestampValue: row.timestampValue,    // 自刷新(见改动三)
            onSelect: { onOpenThread(row.thread) },
            onUnpin: { onUnpinThread(row.id) }
        )
    }
    .swipeActions(edge: .trailing, allowsFullSwipe: false) { … }   // 见改动四
}
.equatable()   // 仍按 row 比;row 不变则 body 不重算 → cell 复用不打断
```
`row` 不变(`Equatable`)→ body 不重算 → List 复用 cell 时不重建内容;`row` 变(改名/running/置顶)→ 单行重算。

### 2.5 保留项(不破坏)
- 拖拽锁:外层 `GaryxRootNavigationView` 仍被 `.disabled(drawerDragActive)`(`GaryxMobileViews.swift:615`)→ `isEnabled=false`
  → 行 `onTapGesture` 的 `guard isEnabled` 拦截;内层 `.scrollDisabled`。两者 List 都支持,原样保留。
- `.task(id: homeListStore.snapshot.isHomeVisible)` silent refresh loop、`.garyxPageBackground()`、`.garyxAdaptiveTopBar`
  (`safeAreaInset(.top)`,List 尊重安全区)、auto-load footer 的 `.onAppear` 触发——全保留。

## 3. 改动二:running badge 折叠 O(1)(`GaryxAvatarTypingBadge`)

### 3.1 问题
每个可见 running avatar 一个 `TimelineView(.animation(1/30))` → N 个独立 30fps **主线程**重渲染调度源,滚动时与滚动渲染争抢。

### 3.2 方案:`PhaseAnimator`(iOS17 循环初始化器,render server 驱动,生命周期自动重启)

> **修订(codex BLOCKER1)**:初版用 `@State animating` + `.onAppear` 翻一次布尔 + `.repeatForever`。codex 正确指出:
> List 若在同一 identity 行 off→on 时**保留 `@State`**,`animating` 可能已是 `true` → `.animation(value:)` 无值跃迁 → CA 不重启。
> 改用 **`PhaseAnimator`(无 trigger 的循环初始化器)**——**没有任何 `@State` 布尔可卡死**,动画循环**绑定视图生命周期**:
> 视图出现即起循环、离屏移除即停、再出现重新起循环。**结构上免疫"复用后不重启"。**

三点波 = 一个亮点在三点间巡游(相位 `[0,1,2]` 循环,`easeInOut` 让 CA 在相邻相位间平滑插值):
```swift
struct GaryxAvatarTypingBadge: View {
    var isPaused = false
    var body: some View {
        Group {
            if isPaused {
                badge(activeDot: -1)                      // 全暗静态
            } else {
                PhaseAnimator([0, 1, 2]) { activeDot in   // 无 trigger = 自动连续循环
                    badge(activeDot: activeDot)
                } animation: { _ in .easeInOut(duration: 0.34) }
            }
        }
        .accessibilityLabel("Running")
    }
    private func badge(activeDot: Int) -> some View {
        HStack(spacing: 2.2) {
            ForEach(0..<3, id: \.self) { i in
                Circle().fill(Color(.systemGray))
                    .frame(width: 3.2, height: 3.2)
                    .opacity(i == activeDot ? 1.0 : 0.4)
            }
        }
        .frame(width: 22, height: 15)
        .background(Color(.systemGray5), in: Capsule())
        .overlay { Capsule().stroke(GaryxTheme.background, lineWidth: 2) }
    }
}
```
- **主线程开销**:相位每 0.34s 推进一次(content 闭包重算 3 个 circle,极廉),**帧间 opacity 由 CA 在 render server 插值 = per-frame 主线程 0**。
  仅可见 running badge 参与,~3 次/秒/badge,远低于原 30fps×N。
- **重启正确性(codex BLOCKER1 闭合)**:`PhaseAnimator` 无布尔状态;cell 复用时离屏视图被销毁、重现时重建 → 循环从头自动起。
  **packaged-app 复现核验**(写进 §8):① running 行从顶部滑出再滑回 → 三点恢复巡游;② 列表下方一个 running 行首次滑入 → 立即巡游。

### 3.3 为什么这才是真 O(1)(四选一论证,给 reviewer)
- **(A) 朴素「单 flag 全局翻一次」**:行不通——List cell 复用,后滚入 badge 已见 `animating==true`,`.animation(value:)` 不触发 → 晚到 cell 永不动。**否决**。
- **(B) 单主线程时钟(CADisplayLink/Timer)+ N badge 订阅**:tick 源收敛成 1,但每 tick 仍重渲 N 个可见 badge 叶子 → 仍 O(可见 running) per-frame **主线程**。没到 O(1)。
- **(C) `.onAppear` 布尔 + `.repeatForever`**:零 per-frame 主线程,但**重启不稳**(codex BLOCKER1:复用保留 `@State` 时不翻值不重启)。**否决**。
- **(D) 本方案 `PhaseAnimator`**:render server 插值 → per-frame 主线程 0;无布尔状态 → 复用/晚到 cell 重现即重启,**结构上正确**。**真 O(1) 主线程,不随可见 running 行数放大。选 D。**
- 与 v4 §5「list 级单一 phase(O(1))」关系:把「单一 30fps 主线程相位源」升级为「单一循环动画定义、render server 执行」,**更优**满足"O(1)/不随行数放大"。
  代价:各 badge 起拍 = 各自出现时刻,**不严格同相**(行间距远、不并列同框,肉眼无差,iMessage/Telegram 即如此)。
  视觉小变:由「多点 sine 叠加波」变「亮点巡游波」(3.2px 尺度下无感,列进 §6 parity 核)。**若要严格同相 / 还原 sine**,降级 (B):list 级单时钟 + badge 内部订阅相位。本稿默认 (D)。

## 4. 改动三:timestamp 烘进 row 自刷新(防相对时间冻结)

### 4.1 问题
List 真复用 + `.equatable()` 后,`row` 不变则 body 不重算;`garyxFormattedTaskTimestamp` 在 body 算一次 → "2m" 会一直停在 "2m"
(墙钟走了 10 分钟也不变),直到 row 因别的原因重投。**冻结。**

### 4.2 方案:formatter 搬进 Core(可注入 now)+ cell-local 每分钟自刷新叶子

> **修订(codex MAJOR2)**:初版只在 App 文件 `garyxFormattedTaskTimestamp` 上加 `now` 形参,但该函数在 **App target**
> (`GaryxMobileDesignSystem.swift:407`),而 `swift test` 只编 `GaryxMobileCore` → Core 测一份拷贝 ≠ 测生产、会假绿。
> 且仓库规则「mobile formatting/presentation 逻辑归 Core」。**改为:把纯 formatter 整体搬进 Core,可注入 now,Core 直测生产实现。**

数据层路 vs cell-local 路:v4 §5 两条都行。因 M6 与 M5 并行、**不得动数据层**,选 **cell-local**(叶子自刷新)。

- **搬迁(纯 Foundation,可整体进 Core)**:新建 `Sources/GaryxMobileCore/GaryxRelativeTimestamp.swift`,把
  `garyxFormattedTaskTimestamp` / `garyxThreadDate(from:)` / 私有 ISO8601 解析(`garyxISO8601Date` + 两个 formatter + NSCache)
  从 App `GaryxMobileDesignSystem.swift` **搬到 Core**;签名改 `garyxFormattedTaskTimestamp(_ value: String?, now: Date = Date()) -> String`
  (体内 `Date()` → `now`,其余逻辑不变)。
- **双构建模型(修订 codex MAJOR3,纠正初版错误)**:本仓 Core 同时被两种构建消费——
  ① **SwiftPM**(`Package.swift`):`GaryxMobileCore` 是独立 library,`GaryxMobileCoreTests` 用 `@testable import` 测它;`swift test` 按目录**自动纳入**新文件。
  ② **app 的 Xcode target**(xcodegen 从 `project.yml` 生成):`GaryxMobile` target 的 `sources` 同时含 `App/GaryxMobile` 与 `Sources/GaryxMobileCore`,
  **把 Core 目录编进同一个 app 模块**(`PRODUCT_MODULE_NAME=GaryxMobile`、`packageProductDependencies` 空,**不是**当 SwiftPM product 消费)。
  pbxproj 用**显式 source membership**,新文件不在里面 app 就编不到 → **新 Core 文件必须 `xcodegen generate` 并提交 pbxproj**
  (否则 `swift test` 假绿、app `xcodebuild` 挂 —— 即 reference `iOS xcodegen .pbxproj 要手动同步` 的坑)。
- **调用方**:因 app target 里 App 与 Core 同属一个模块,搬迁后 App 现有 9 处 `garyxFormattedTaskTimestamp(x)` + 2 处 `garyxThreadDate`
  **同模块直接可见、不需 import**(**不要**给 App 文件加 `import GaryxMobileCore`)。它们走默认 `now=Date()`、**行为不变**;App 文件**删除**这些函数定义(避免重复)。
- **新增叶子 `GaryxRelativeTimestampText`**(home 专用,放 SidebarViews,UI 留 App):
  ```swift
  struct GaryxRelativeTimestampText: View {
      let timestampValue: String?
      var body: some View {
          TimelineView(.everyMinute) { ctx in
              Text(garyxFormattedTaskTimestamp(timestampValue, now: ctx.date))
                  .font(GaryxFont.caption()).foregroundStyle(.tertiary).lineLimit(1)
          }
      }
  }
  ```
  `.everyMinute` 对齐墙钟整分触发,**只重渲这个 Text 叶子(1 次/分,可忽略)**,不重算行 body、不动 cell 复用。
- 接线:`GaryxSidebarThreadRowView` 加可选 `var liveTimestampValue: String? = nil`;`trailingMeta` 的时间分支改为
  「`liveTimestampValue != nil` → 渲 `GaryxRelativeTimestampText`,否则保持现状渲 `presentation.trailingTimestamp`」。
  home 传 `liveTimestampValue: row.timestampValue` 且 `presentation` 不再烘 `trailingTimestamp`;**其它调用方零改动**(继续传预格式化串)。

> 备注(整合 seam):若 Gary 整合 M5 时数据层改为「每分钟 tick 重投烘好的 timestamp」,可删掉此 cell-local 叶子改走数据层——
> 但那是 M5/整合范畴。M6 自洽用 cell-local。

## 5. 改动四:swipe → native `.swipeActions`(仅 home)

v4 §6.2 + 任务:M6 **先用 native**,full-swipe / 长按确认与自绘略分叉,**老板已认可先 native 看效果**,不一致再统一。

```swift
.swipeActions(edge: .trailing, allowsFullSwipe: false) {
    if row.canArchive {
        Button(role: .destructive) { Task { await onArchiveThread(row.thread) } }
            label: { Label("Archive thread", systemImage: "archivebox") }
    }
    Button { onTogglePinnedThread(row.id) }
        label: { Label(row.presentation.isPinned ? "Unpin thread" : "Pin thread",
                       systemImage: row.presentation.isPinned ? "pin.slash" : "pin") }
        .tint(Color(.systemGray))
}
```
- **顺序**:archive 在闭包首位 → 落在 trailing 最外侧,与自绘 `[pin, archive]`(archive 在最右/trailing-most)一致。
- `allowsFullSwipe: false`:自绘需「划开再点」,无全划即归档;native 设 false 保持(需显式两步,防误归档)。
- **`canArchive` 门**:自绘 swipe **没**对 `canArchive` 设门(automation 线程也露 archive、靠 model `canArchiveThreadId` 兜底拒绝);
  本稿对 native archive 加 `if row.canArchive` 门(automation 线程不露 archive)= **小幅更正**(更干净),已标注供 reviewer 裁。
  running 线程:自绘 swipe 原本**允许**归档(只有长按路径 gate `!isRunning`),native swipe 保持「允许」(model 仍是最终守卫),**不加 running 门**(swipe 路径 parity)。
- **长按确认 `.onLongPressGesture` + `.confirmationDialog`**:**M6 移除**。理由:① List row 上的 `.onLongPressGesture` 与 List
  滚动/swipe 手势易冲突;② 现状 swipe-tap 归档本就**无**确认(确认只在长按路径),移除长按只是去掉那条二级确认路径,
  swipe 归档行为不变;③ 任务/§6.2 明确「native-first、分叉可接受」。**记为已认可分叉**;若要 archive 确认,后续在 swipe archive 上挂
  `.confirmationDialog` 统一(非 M6)。

## 6. 视觉 parity checklist(packaged-app 逐项核)
- [ ] 顶 4pt / 段间 10pt / 底 28pt 间距;pinned 段头 + 段尾 10pt、recent 段头。
- [ ] 行高、avatar 38、内边距、选中态背景、pin 图标、subtitle、trailing timestamp 位置。
- [ ] divider 出现位置(每段第 2 行起、行上沿、左缩进对齐 avatar 右)。
- [ ] running badge 三点波视觉、capsule 描边(`GaryxTheme.background` 环)。
- [ ] header 随滚动滚走(非吸顶)。
- [ ] empty / loading / "Loading more" footer 文案与位置。
- [ ] 背景 = `GaryxTheme.background`(无 List 灰底/默认分隔线)。
- [ ] 安全区:top bar inset、底部不被 home indicator 压。

## 7. 测量计划(hitch gate)
- **判据**(任务完成标准):Instruments Animation Hitches / `XCTHitchMetric` 的 `hitchTimeRatio` **< 改前同机 baseline**。
- **改前 baseline**:在切 List 前(LazyVStack 版)用现有 `GaryxHomeScrollPerformanceProbe` 跑「running 线程 → popToHome → 滑动」
  窗口,记 `hitch_time_ratio` / `row_body` / `home_body`。切 List 后同机同操作复测,要求 hitch 比降低、`row_body` 在滑动中明显下降
  (复用 → 离屏不重算)。
- **真机依赖(显式风险)**:我的记忆与 v4 都指出 sim 吸收 CPU、**绝对卡顿真机才放大**;`XCTHitchMetric` 的"像 Telegram"判据需**物理设备**。
  我(agent)默认只有 simulator。计划:① sim 上做行为 parity + probe 相对对比(List vs LazyVStack,同机相对值);
  ② 真机 `hitchTimeRatio` 终判作为 **Gary/老板验收步**(我产出 build + sim 数据 + probe 方法,真机定值留 §6.3)。
  **请 reviewer 确认**:此真机终判归属是否可接受为"M6 交付 + 验收分离"。

## 8. 测试与验证
- **`swift test`(Core)全绿 + 新增 Core 直测**:formatter 已搬 Core(§4.2),新增 `GaryxRelativeTimestampTests` **直测生产实现**
  (不再测拷贝,闭合 codex MAJOR2):注入固定 `now`,断言边界('now'/'59m'/'1h'/'23h'/'1d'/'1mo'/'1y'/空值)+ **不冻结性质**
  (同一 `value`、`now` 推后 1h → 输出从 'Xm' 变 'Yh')。
- `xcodebuild` Debug **SUCCEEDED**(App target,跑 xcodegen 同步 pbxproj 后)。
- **pbxproj(修订 codex MAJOR3)**:新 Core 文件 `GaryxRelativeTimestamp.swift` 虽被 `swift test` 自动纳入,但 app target 用**显式 source
  membership**(Core 目录编进 app 模块)→ **必须 `xcodegen generate` 并提交 pbxproj**,再跑 app `xcodebuild`,否则 swift test 假绿、
  xcodebuild 编不到(见 reference `iOS xcodegen .pbxproj 要手动同步`)。`GaryxRelativeTimestampText`(SwiftUI 叶子)就近放 App `SidebarViews.swift`
  (免 App 新增文件);若仍新增 App 文件同样须 xcodegen + 提交 pbxproj。**验证必须真跑 app `xcodebuild`,别被 `swift test` 的绿骗。**
- packaged-app(iOS simulator 安装包)实测:swipe pin/unpin/archive、置顶重排、归档消失、下拉刷新、滚动跟手、
  **running badge:顶部滑出再滑回 + 下方首次滑入,三点都巡游**(§3.2 重启核验)、timestamp 跨分钟变化、drawer 拖拽时行不可点 + 不滚。

## 9. 风险与 open questions(给 codex 设计 review 重点)
1. **List 复用正确性**:divider 折叠进行内是否真让"1 thread = 1 cell"且复用不串(对照自绘 sibling-divider)?
   `.equatable()` + List 复用是否冲突(EquatableView 作 List row 的 identity/复用语义)?
2. **badge O(1)**(已按 BLOCKER1 改 `PhaseAnimator`):无布尔状态、生命周期绑定重启是否真闭合"复用不重启"?「亮点巡游」替「sine 叠加」视觉可接受?不严格同相可接受?
3. **timestamp 不冻结**(已按 MAJOR2 搬 Core):formatter 进 Core + 可注入 now + Core 直测 + cell-local `TimelineView(.everyMinute)` 叶子——是否够干净、够真测?
4. **swipe parity**:archive 加 `canArchive` 门、移除长按确认、`allowsFullSwipe:false`、动作顺序——分叉是否都在"老板已认可"范围内?
5. **非吸顶 header**:平铺 row(不用 Section)换非吸顶,是否最干净的 parity 取向?
6. **hitch gate 真机依赖**:§7 的"交付 + 真机验收分离"是否可接受。

## 10. 实现顺序
1. 先在 LazyVStack 版跑 probe 记 baseline(留档)。
2. **formatter 搬 Core**(`GaryxRelativeTimestamp.swift`,可注入 `now`)+ 删 App 拷贝(**不加 import**,同模块)+ **`xcodegen generate` 提交 pbxproj** + **Core 直测**(§4.2/§8)。
3. `GaryxAvatarTypingBadge` → `PhaseAnimator`(§3.2)。
4. `GaryxRelativeTimestampText`(`TimelineView(.everyMinute)` + Core formatter)+ `GaryxSidebarThreadRowView.liveTimestampValue` 接线。
5. `GaryxHomeThreadButton`:divider 折叠 + native swipeActions + 去 withTrailingTimestamp/长按。
6. 容器 `threadListWithBottomBar` + `sidebarThreadSections` 换 List + listRow 修饰。
7. `swift test` + `xcodebuild` + simulator 行为/视觉 parity + **badge 重启核验** + probe 对比。
8. 自开 codex 代码 review(`--agent codex --notify current-thread`)。
