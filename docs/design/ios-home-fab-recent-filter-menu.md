# iOS 首页改版:新建线程 FAB + Recent 筛选下拉菜单 + 筛选持久化

状态:#TASK-2161 综合定稿,实现基准。

修订(2026-07-12,上线后用户决定):移除筛选按钮右上角的 active dot,
非默认筛选的可见提示只保留「Recent · Chats」标题尾注;本文其余关于
dot 的段落仅作历史设计记录,不再是实现要求。

基线:`origin/main` @ `26821a59e`(2026-07-12,含 #TASK-2153 / #TASK-2158
已落地的 Recent filter tabs 实现)。实现前已 fetch 并确认当前 worktree HEAD 与
`origin/main` 一致,Recent 同域没有更新的主线演进。

综合来源:

- 设计 A `ios-home-fab-recent-filter-menu.md`:作为 UI、组件、测试与风险主干。
- 设计 B `ios-home-recent-filter-menu-fab.md`:采纳 app-global preference、首帧前恢复、
  Gateway reset 保留选择、显式首页 options、`Recent · Chats` 与单份底部避让。

关联设计:

- `docs/design/recent-thread-task-filter-tabs.md`:Recent 筛选的数据语义、
  双 feed 架构、wire 契约(`tasks=include|exclude`)。本设计**不改变其中任何
  数据语义**,只改交互形态,并推翻其 §2 中「本期不持久化用户最近一次 Tab」
  这一条非目标。
- `docs/design/task-1802-ios-home-list-refresh-loadmore.md`:refresh/load-more
  pager 语义,本设计完全复用。

## 1. 目标

已拍板的四个改动,全部只作用于 iOS 首页(Recent 线程列表页):

1. **新建线程入口改为右下角 FAB**:从顶栏右上角移除新建按钮,改为右下角
   常驻黑色圆形浮动按钮,白色「气泡+」图标(沿用现有 `plus.bubble` 语义),
   带投影浮在列表上方,列表滚动不隐藏,贴合底部安全区。质感对标 Manus iOS。
2. **All / Chats 分段控件改为右上筛选菜单**:整行移除分段控件、列表上移。
   在右上角原新建按钮位置放筛选图标按钮,点按弹出锚定其下方的下拉菜单,
   菜单项只有 `All` / `Chats` 两项,当前选中项打勾,点选即生效并
   收起,点空白处收起。
3. **筛选选择跨启动持久化**:下次打开 app 恢复上次选择。
4. **非默认筛选的可见状态提示**:选中非「All」时给出常驻可见提示
   (方案与论证见 §6)。

## 2. 非目标

- 不改变筛选语义与数据源:`GaryxRecentThreadFilter` 两个 case、
  `tasks=include|exclude` wire 映射、双 feed(`allFeed`/`nonTaskFeed`)架构、
  auxiliary All refresh、Widget / Automation picker 恒读 All feed —— 全部不动。
- 不扩展菜单项(不加 Tasks/自动化筛选)、不加搜索、不加 count。
- 不动 Mac Recent rail(Mac 保持 segmented control,平台各自原生交互,
  这是 #TASK-2153 §1.4 既定原则)。
- 不改 Pinned section:Pinned 不参与筛选(既有语义)。
- 不改 Gateway API、不加 capability 探测。
- 产品标签保持与 Mac 一致的 `All` / `Chats`(Mac 是标签真相源;
  「全部」是本文档对 `All` 的中文描述,不是新 UI 文案)。

## 3. 现状(改动锚点)

```
GaryxHomeThreadListView (App/GaryxMobile/GaryxMobileSidebarViews.swift)
├── .garyxAdaptiveTopBar { GaryxHomeHeaderView }        // 固定顶栏
│     ├── GaryxSidebarMenuButton (hamburger)
│     ├── "Garyx" 标题
│     └── Button(plus.bubble, 玻璃圆) → onStartNewChat  // ← 改为筛选菜单
├── List (.plain)
│     ├── [Pinned header + rows]
│     ├── GaryxSidebarSectionHeader("Recent", clock.fill)
│     ├── GaryxRecentThreadFilterPicker (segmented 行)   // ← 整行移除
│     ├── recent rows / skeleton / empty / unavailable
│     └── footer(不再叠加 bottomBarClearance spacer)
└── (无底部 chrome)                                      // ← 挂 FAB
```

数据流(全部复用,不改):

- 选中态:`GaryxRecentThreadFeeds.selectedFilter`(Core)→
  `GaryxMobileModel+Presentation` 捕获进 `HomeProjection` input →
  `GaryxHomeThreadListSnapshot.selectedRecentFilter` → view 哑渲染。
- 切换动作:view 回调 `onSelectRecentFilter` →
  `GaryxMobileModel.selectRecentThreadFilter(_:)`(guard 同值 →
  `feeds.select` → `refreshThreads(source: .userAction)`;selected 为
  `.nonTask` 时 `refreshThreads` 内部自动带起 auxiliary All refresh)。
- 冷启动目标时序:`GaryxMobileModel.init(defaults:)` 从注入 defaults 读取 app-global
  preference,用恢复值初始化 `GaryxRecentThreadFeeds`,然后才允许首个 Home snapshot
  与首个网络请求。恢复 `.nonTask` 时不得先发布/请求 `.all`。
- Gateway 切换目标时序:`resetGatewayRuntimeState()` →
  `resetThreadListPagination()` → `recentThreadFeeds.resetFeedData()`。只清双 feed 的
  页数据/epoch,保留 app-global selection;不把它塞进
  `loadGatewayScopedUserState` / `saveGatewayScopedUserState`。
- debug snapshot/fixture 必须显式指定 selection(默认 `.all`),不能继承开发机
  `.standard` 中的历史 preference,保证截图和 UI fixture 可复现。

## 4. 视图结构改动

### 4.1 顶栏:新建按钮 → 筛选菜单按钮

`GaryxHomeHeaderView`(单一使用点,直接扩签名):

```swift
struct GaryxHomeHeaderView: View {
    let selectedRecentFilter: GaryxRecentThreadFilter
    let onOpenDrawer: () -> Void
    let onSelectRecentFilter: (GaryxRecentThreadFilter) -> Void
    // onNewChat 移除(动作移交 FAB)
}
```

右上角控件由 `Button(plus.bubble)` 换成 `GaryxRecentThreadFilterMenu`
(§5.2),沿用完全相同的 44pt 玻璃圆治具
(`garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial:
.ultraThinMaterial, in: Circle())` + 显式 `.contentShape(Circle())`),
所以顶栏几何、GlassEffectContainer(spacing: 10) 结构、Reduce Transparency
的不透明兜底全部不变。

`GaryxHomeThreadListView` 已持有 `homeListStore.snapshot` 与
`onSelectRecentFilter`,把 `snapshot.selectedRecentFilter` 和回调透传进
header 即可;header 仍是无状态哑视图。

### 4.2 移除分段控件行

删除 `sidebarThreadSections` 中的 `GaryxRecentThreadFilterPicker(...)` 调用
(SidebarViews ~L258-261)及组件文件本身。该行自带
`minHeight 44 + padding`,移除后 Recent rows 自然上移,无需补偿 spacer
(header 的 `.padding(.bottom, 4)` 保留)。

空态文案 `No recent threads` / `No recent chats`、skeleton、unavailable、
footer、near-tail prefetch 分支全部不动 —— 它们读的是 snapshot,与控件形态
无关。

### 4.3 底部:FAB chrome

在 `GaryxHomeThreadListView.body` 的 `threadListWithBottomBar` 上挂共享
safe-area chrome(仓库既定规则,禁止本地 `ignoresSafeArea` patch):

```swift
threadListWithBottomBar
    .garyxFloatingBottomChrome {
        GaryxHomeNewThreadFab(action: onStartNewChat)
            .frame(maxWidth: .infinity, alignment: .trailing)
            .padding(.trailing, 20)
            .padding(.bottom, 8)
    }
```

行为说明:

- `garyxFloatingBottomChrome` 基于 `safeAreaInset(edge: .bottom)`。对
  `List` 这类滚动容器,safe-area inset 表现为**内容 inset**:行会从 FAB
  下方滚过、透过透明 chrome 可见,只是最后一行的停靠位在 FAB 上方 ——
  即「列表在 FAB 下滚动、FAB 常驻不随滚动隐藏」,与会话页 composer 的
  滚动关系同构,正是拍板要的 Manus 形态。
- chrome 根是 `Color.clear` 背景 + 仅 trailing 一个 56pt 按钮;clear 区域
  与 `Spacer` 不参与 hit-test,不会挡住透出的列表行、pull-to-refresh 或
  底部滚动。
- 删除既有 `bottomBarClearance`(28pt)spacer 与对应 metric。FAB 避让全部由
  `garyxFloatingBottomChrome` 的 safe-area inset 提供;列表尾部只保留普通 section
  收尾间距,避免旧 spacer 与 56pt chrome 高度叠成双份空白。目检最后一行应能完整
  停靠在 FAB 带上方且仍可点。
- `onHeightChange` 不需要(首页没有依赖 chrome 高度的二级布局)。
- FAB 只属于首页根:chrome 挂在 `GaryxHomeThreadListView` 上,会话页、
  drilldown、抽屉打开时的遮罩层级都不受影响(抽屉覆盖在首页之上,自然
  盖住 FAB)。

## 5. 组件设计

### 5.1 `GaryxHomeNewThreadFab`(新文件)

```swift
struct GaryxHomeNewThreadFab: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "plus.bubble")
                .font(GaryxFont.system(size: 20, weight: .semibold))
                .foregroundStyle(Color(.systemBackground))
                .frame(width: 56, height: 56)
                .background(Color(.label), in: Circle())
                .contentShape(Circle())
                .shadow(color: .black.opacity(0.18), radius: 16, x: 0, y: 8)
                .shadow(color: .black.opacity(0.08), radius: 3, x: 0, y: 1)
        }
        .buttonStyle(GaryxFabPressStyle())   // 按压 0.96 缩放 + 0.85 不透明度
        .accessibilityLabel("New chat")
    }
}
```

规格与论证:

- **尺寸** 56pt 圆(> 44pt 最小触达);图标 20pt semibold,沿用
  `plus.bubble` 保持「新建对话」图标语义不变(拍板点 1)。
- **配色** `Color(.label)` 底 + `Color(.systemBackground)` 图标:浅色模式
  即拍板的「黑圆 + 白气泡+」;深色模式自动反转为白圆黑标。这是仓库
  primary 控件既定配方(`GaryxPrimaryCompactButtonStyle` /
  `GaryxMiniIconButtonStyle(isPrimary:)` 同款),且纯黑圆在深色近黑页面
  背景上会消失,反转是唯一不需要额外描边的方案。monochrome 规则同时
  满足(绿色仍只留给运行/成功语义)。
- **投影** 双层(大半径环境影 + 小半径接触影),对标 Manus 的悬浮质感;
  会话页 scroll-to-bottom 按钮已有单层 `opacity(0.12)/radius 14` 先例,
  FAB 是页面唯一 primary 动作,略重一档。
- **不是玻璃**:拍板要求不透明黑圆,恰好绕开已知坑 ——
  `GaryxMobileConversationViews.swift` L220 注释记录了 iOS 26 glassEffect
  在 bottom chrome 内没有 hit-test 区域、点击穿透到列表行(26.2 实测)。
  FAB 用不透明 `background(_, in: Circle())` + 显式
  `.contentShape(Circle())`,不进 `GlassEffectContainer`。
- **Reduce Motion**:按压反馈只做透明度变化,缩放在 Reduce Motion 下禁用
  (style 内读 `accessibilityReduceMotion`)。
- **Dynamic Type**:FAB 是几何钉死的 chrome(与 title-capsule 同类),
  图标用固定字号变体;可达性由 44pt+ 触达面积与 accessibilityLabel 保证。

### 5.2 `GaryxRecentThreadFilterMenu`(新文件,替换 Picker 文件)

用原生 `Menu` + inline `Picker`,零自绘弹层:

```swift
struct GaryxRecentThreadFilterMenu: View {
    let selection: GaryxRecentThreadFilter
    let onSelect: (GaryxRecentThreadFilter) -> Void

    var body: some View {
        Menu {
            Picker("Recent filter", selection: Binding(get: { selection }, set: onSelect)) {
                ForEach(GaryxRecentThreadFilter.homeMenuOptions, id: \.self) { filter in
                    Text(filter.displayName).tag(filter)   // "All" / "Chats"
                }
            }
        } label: {
            ZStack(alignment: .topTrailing) {
                Image(systemName: "line.3.horizontal.decrease")
                    .font(GaryxFont.system(size: 16, weight: .semibold))
                    .foregroundStyle(.primary)
                    .frame(width: 44, height: 44)
                if selection.activeStatusLabel != nil {
                    GaryxRecentFilterActiveDot()
                }
            }
            .garyxAdaptiveGlass(.regular, isInteractive: true,
                                fallbackMaterial: .ultraThinMaterial, in: Circle())
            .contentShape(Circle())
        }
        .menuOrder(.fixed)                       // All 恒在 Chats 上方
        .accessibilityLabel("Recent filter")
        .accessibilityValue(selection.displayName)
    }
}
```

规格与论证:

- **锚定与收起**:原生 `Menu` 自动锚定在顶栏按钮下方弹出、点选项即提交
  并收起、点空白收起 —— 拍板的三条交互全部由系统行为免费提供,不写
  自定义 popover / overlay(仓库反对自绘系统件)。
- **打勾**:`Picker` inline 在 `Menu` 内渲染为带系统 checkmark 的菜单项,
  勾选态即当前 `selection`,monochrome(系统菜单样式),满足
  「当前选中项打勾」且不违反选中态配色规则。
- **图标** `line.3.horizontal.decrease`:SF Symbols 标准筛选语义,裸 glyph
  进玻璃圆,与 hamburger/原新建按钮同治具,不引入新图标语言。不用
  `*.circle` 变体(圆中圆)。
- **iOS 26 玻璃两条硬规则**:glyph 与 active dot 都放在 label 自身的前景
  `ZStack` 中,glass 直接挂在这个 label 内容上(非 `.background {}` 或挂在
  `Menu` 外层的 `.overlay`,避免系统重组 label 时吞掉状态点或被容器 hoist
  盖住前景);显式 `.contentShape(Circle())` 补 hit-test。与现有顶栏按钮写法一致。
- **回调链路不变**:`onSelect` 仍指向
  `model.selectRecentThreadFilter(_:)`,同值 guard、`.userAction` 刷新、
  auxiliary All refresh、错误 toast 策略全部复用,无新状态机。
- 菜单项只从 Core 的 `homeMenuOptions == [.all, .nonTask]` 渲染,不得读取
  `allCases`;未来 enum 增加内部能力时不会自动泄漏进首页。
- 菜单项标签用既有 `displayName`(All / Chats),与 Mac 完全一致,不引入中文
  「全部」文案。

### 5.3 删除 `GaryxRecentThreadFilterPicker.swift`

segmented 形态整体退役,组件文件删除(不留死代码);
`Picker("Recent filter")` 的 accessibility 标识由 Menu 的
`accessibilityLabel("Recent filter")` 接续,自动化脚本按 label 找控件不断。

## 6. 可见状态提示(拍板点 4 的方案与论证)

**方案:双点位、同一 Core 真相源
`GaryxRecentThreadFilter.activeStatusLabel`。`.all → nil`,
`.nonTask → "Chats"`;SwiftUI 只读该派生值决定 dot 与标题尾注,不复制 case switch。**

### 6.1 主提示:筛选按钮右上角 active dot(常驻 chrome)

```swift
struct GaryxRecentFilterActiveDot: View {
    var body: some View {
        Circle()
            .fill(Color.primary)                 // monochrome
            .frame(width: 7, height: 7)
            .overlay(Circle().stroke(GaryxTheme.header, lineWidth: 1.5))
            .offset(x: -6, y: 6)                 // 落在玻璃圆内缘 1-2 点钟位
            .accessibilityHidden(true)           // 状态由 accessibilityValue 表达
    }
}
```

选它的理由:

- **唯一不随滚动消失的点位**。Recent section header 会滚出屏幕,顶栏是
  首页唯一固定 chrome;用户深滚后仍能一眼看到「列表被筛过」,不会把
  「task 线程不见了」误判成数据丢失。
- **dot 是「非默认状态」的通用记号**(系统 App、Manus 同款用法),
  不改变图标本身语义;`Color.primary` 保持 monochrome,细描边用
  `GaryxTheme.header` 与玻璃面分离,浅/深色都可见。
- VoiceOver 不读 dot,读 `accessibilityValue`("Chats"),避免图形状态与
  朗读状态两套真相。

### 6.2 辅助提示:Recent 标题行尾注

`GaryxSidebarSectionHeader` 加可选 `statusLabel: String?`,与标题就近组合:

```
Recent · Chats
```

尾注只在 `activeStatusLabel != nil` 时出现。`Recent` 与分隔点维持现有
secondary section 样式,`Chats` 使用 caption semibold/primary;不把状态推到行尾,
不创建 capsule/badge。短文案保持单行并优先保留 `Chats`。

选它的理由:**在行集缺失的现场解释行集**。空态已有
"No recent chats" 文案,但非空列表里少了 task 行时没有任何就地解释;
尾注让「这一节当前是 Chats 视图」在扫视 rows 时即可读到,与 dot 互补
(dot 答「有没有筛选」,尾注答「筛的是什么」)。

### 6.3 被拒方案

| 方案 | 拒绝理由 |
|---|---|
| 按钮反色(黑圆白 glyph)表示激活 | 与 FAB 同视觉配方,页面出现两个「实心黑圆」,破坏单一 primary 动作层级;反色通常语义是「可执行动作」而非「已应用状态」。 |
| 图标换 `.fill` 变体 | 16pt 下 fill/stroke 差异读作字重变化而非状态,可辨识度不足;且双态图标在玻璃圆上产生两套轮廓。 |
| 彩色 dot(绿/蓝) | 违反仓库配色纪律:绿色保留给运行/成功语义,普通选中态必须 monochrome。 |
| 只留 `Recent · Chats` | 滚动后不可见,深滚场景失去提示,不满足「常驻可见」。 |
| 顶栏标题改「Garyx · Chats」 | 污染品牌标题位,且标题不是筛选控件,提示与控件分离违反就近原则。 |

## 7. Core 设计:筛选持久化

### 7.1 类型、首页选项与存储(GaryxMobileCore,SwiftPM 可测)

现有 filter/wire 语义不变,补三项显式 Core 契约:

```swift
public enum GaryxRecentThreadFilter: String, CaseIterable, Equatable, Hashable, Sendable {
    case all
    case nonTask

    public static let homeMenuOptions: [Self] = [.all, .nonTask]

    public var activeStatusLabel: String? {
        switch self {
        case .all: nil
        case .nonTask: "Chats"
        }
    }
}
```

菜单必须读 `homeMenuOptions`,不读 `allCases`;后者可以继续用于 Core 内部穷举和
测试,但未来 enum 扩 case 不得自动扩张首页产品面。`displayName` 仍严格为
`All` / `Chats`,`tasksQueryValue` 仍严格为 `include` / `exclude`。

新增 Core 文件 `GaryxRecentThreadFilterStorage.swift`,沿用 defaults 参数注入先例:

```swift
public enum GaryxRecentThreadFilterStorage {
    public static func persistenceValue(for filter: GaryxRecentThreadFilter) -> String
    public static func restoredFilter(from rawValue: String?) -> GaryxRecentThreadFilter
    public static func load(defaults: UserDefaults, key: String) -> GaryxRecentThreadFilter
    public static func save(
        _ filter: GaryxRecentThreadFilter,
        defaults: UserDefaults,
        key: String
    )
}
```

存储契约:

- key 固定为 `garyx.mobile.recentThreadFilter`。
- scope 是 **app 全局**,不拼 `currentGatewayScopeId`,不调用
  `scopedSettingsKey`,不进入 `loadGatewayScopedUserState` /
  `saveGatewayScopedUserState`。这是跨 Gateway 共用的观看偏好,不是网关业务数据。
- value 是稳定内部字面量:`.all → "all"`,`.nonTask → "nonTask"`;不存
  展示文案 `All` / `Chats`,也不存 wire 值 `include` / `exclude`。
- 缺失、空串或未知值一律回退 `.all`;load 只读且不回写,避免一次读取制造默认值
  或覆盖未来版本值。
- 只持久化 selection,不持久化 feed 行、分页游标、失败或 primed 状态。

`GaryxRecentThreadFeeds` 增加恢复值注入与保留选择的数据 reset:

```swift
init(
    pageLimit: Int,
    overlap: Int,
    selectedFilter: GaryxRecentThreadFilter = .all
)

mutating func resetFeedData() // 清双 feed/epoch,保留 selectedFilter
```

旧 `reset()` 的“同时把 selection 改回 All”语义退出 Gateway reset 路径;实现可删除
该 API,避免以后误用。双 pager reset 仍递增 epoch,切 Gateway 前的 ticket 必须失效。

### 7.2 model 初始化、写入与首帧时序(App 层薄接线)

`GaryxMobileModel.init(defaults:)` 必须在任何 Home projection snapshot 与任何网络
请求前完成恢复:

```swift
let restoredRecentFilter = GaryxRecentThreadFilterStorage.load(
    defaults: defaults,
    key: GaryxMobileSettingsKeys.recentThreadFilter
)
recentThreadFeeds = GaryxRecentThreadFeeds(
    pageLimit: Self.threadListPageLimit,
    overlap: Self.threadListPageOverlap,
    selectedFilter: restoredRecentFilter
)
```

该赋值发生在 init 内首次 `refreshHomeObservationSnapshot()` /
`emitHomeProjectionSnapshot()` 之前。启动后的 `connectAndRefresh` 直接从恢复后的
selection 取首张 ticket;恢复 `.nonTask` 时首个 visible 请求必须是
`tasks=exclude`,并按既有逻辑并发辅助 `All` head refresh。禁止先构造/发布
`.all` snapshot 后再异步 select。

用户点选路径保持同步状态、同步落盘、异步刷新:

1. guard 同值直接返回(不写 defaults、不重复 refresh);
2. `recentThreadFeeds.select(filter)`;
3. `GaryxRecentThreadFilterStorage.save(filter, defaults:, key:)`;
4. `refreshThreads(source: .userAction)`。

load 不走 `selectRecentThreadFilter`,不触发额外 refresh,也不回写 defaults。
`@AppStorage` 不得再建第二份 UI 状态。

### 7.3 Gateway reset 与 debug fixture

Gateway 切换只清业务 feed 数据,不改变 app-global preference:

```text
saveGatewayScopedUserState(只处理 agent/workspace 等既有 scoped state)
→ resetGatewayRuntimeState
→ resetThreadListPagination
→ recentThreadFeeds.resetFeedData
→ 切换 URL / loadGatewayScopedUserState
→ connectAndRefresh 使用保留的 selectedFilter
```

`resetFeedData()` 清空两 feed 的 ids、primed/failure/pager 状态并递增 epoch,但
`selectedFilter` 与 defaults 均保持不变。由此 Gateway profile 切换、当前 profile
URL/token/header 触发的 runtime reset、connect link 与 scope drift 都遵守同一契约。
相应更新既有“Gateway reset 后 selection == .all”测试矩阵为“数据清空、旧 ticket
拒绝、selection 保留”。

时序矩阵:

| 场景 | 顺序 | 结果 |
|---|---|---|
| 冷启动 | defaults load → feeds(restored selection) → 首个 Home snapshot → 首刷 | 恢复 Chats 无 All 闪帧,首个 visible 请求为 exclude |
| Gateway profile 切换 | resetFeedData → 换 scope → connect refresh | selection 与 preference 保留,新 Gateway 按同一 filter 首刷 |
| 编辑当前 profile/runtime reset | 同上 | 双 feed 清空,selection 保留 |
| key 缺失/未知 | init 恢复 `.all` | 不回写 defaults |
| debug snapshot/fixture | 显式注入 `.all`(需要时可显式 `.nonTask`)后 reset/prime | 不受开发机 defaults 污染 |

当前 `loadDebugSnapshot()` 在 reset/fixture 构造前显式 select 固定值;测试 model
全部使用独立 `UserDefaults(suiteName:)` 并清 domain。不能让本机
`UserDefaults.standard` 决定截图中的 menu、dot 或 `Recent · Chats`。

### 7.4 设置键

`GaryxMobileSettingsKeys` 增加非 scoped 常量:

```swift
static let recentThreadFilter = "garyx.mobile.recentThreadFilter"
```

## 8. 无障碍与适配清单

- 筛选菜单:label "Recent filter"、value 当前 `displayName`;menu 项由
  系统朗读含选中态。dot `accessibilityHidden`。
- FAB:label "New chat";56pt 触达;VoiceOver 顺序上位于列表之后
  (safeAreaInset 内容),rotor 可直达。
- Dynamic Type:menu 弹层系统自适应;顶栏/FAB/section header 属几何钉死
  chrome,沿用现有固定字号策略;尾注与 header 标题同字体同缩放策略。
- 深色模式:FAB 反色(§5.1);dot `Color.primary` + header 色描边;玻璃
  按钮沿用现有 fallback。
- Reduce Transparency:玻璃按钮已有 `ultraThinMaterial` fallback +
  顶栏不透明 `GaryxTheme.header` 底;FAB 本就不透明。
- Reduce Motion:FAB 按压只保留透明度反馈;menu 动画系统接管。

## 9. 测试计划(SwiftPM Core + app-target 集成 + 真实交互)

### 9.1 Core storage / presentation(SwiftPM)

新增 `GaryxRecentThreadFilterStorageTests.swift`:

1. 持久化字面量钉死:`persistenceValue(.all) == "all"`、
   `persistenceValue(.nonTask) == "nonTask"`;同时断言它们不是 display/wire 值;
2. save → 新 storage load 两 case 均 round-trip,模拟跨 model/跨启动;
3. key 缺失、空串、未知值(如 `"tasksOnly"`)都回退 `.all`;
4. load 不回写:读取缺失/未知值后 defaults 原值/缺失状态不变;
5. 可注入 defaults/key,测试 suite 清理;生产 key 字面量严格等于
   `garyx.mobile.recentThreadFilter`。

扩展 `GaryxRecentThreadFeedsTests.swift`:

6. `homeMenuOptions` 严格等于 `[.all, .nonTask]`,菜单不会由
   `allCases` 意外扩张;displayName 仍为 `All` / `Chats`;
7. `activeStatusLabel`:`.all → nil`,`.nonTask → "Chats"`;
8. 以 restored `.nonTask` 初始化时 selected/visible/presentation 指向 Chats feed,
   All feed 保持独立,首个 `requestRefresh()` ticket.filter 为 `.nonTask`;
9. `resetFeedData()` 清两 feed ids/primed/failure/pager,递增 epoch、拒绝旧
   refresh/load-more ticket,但保留 `.nonTask`;
10. 既有 wire mapping、双 feed 隔离、All/visible projection、archive rollback、
    local mutation、成功空页/失败/auxiliary All 相关矩阵继续全绿。

扩展 Home projection/actor 测试:即使两 feed ids 相同,以恢复 selection 生成的首个
snapshot 也必须是 `.nonTask`;状态变更不能被 row signature 去重掉。

### 9.2 App-target 初始化与副作用(xcodebuild)

使用独立 `UserDefaults(suiteName:)` 构造 `GaryxMobileModel`:

1. 预写 `"nonTask"` 后初始化 model,首次 Home snapshot 已是 `.nonTask`,没有
   `.all` 中间发布;
2. URLProtocol/spy 驱动首次 `connectAndRefresh`,第一个 visible Recent 请求为
   `tasks=exclude`;辅助 `tasks=include` 可并发但不得成为 visible ticket;
3. 用户选择后全局 key 立即更新;重复选择同项不改写、不重复 refresh;
4. `resetGatewayRuntimeState()` 后双 feed 数据清空、旧 completion 丢弃,selection
   与 defaults 均保留;把 Gateway A 切到 Gateway B 再刷新仍请求同一 filter;
5. 新建第二个 model 读取同一 suite 恢复 Chats,证明跨 model/cold-launch;
6. debug snapshot 显式固定 `.all`,即使 suite 预置 `nonTask` 也不污染 fixture;
7. 更新既有 Gateway reset 测试中“回到 All”的断言与错误说明,确保旧 auxiliary
   request 仍由 runtime generation/epoch 丢弃。

### 9.3 既有业务边界回归

- Pinned task 在 `.nonTask` 下仍进入 Pinned section;
- Widget snapshot 与 Automation existing-thread options 始终读取
  `allRecentThreadIds`,不随持久化 selection 改变;
- `.nonTask` 下 auxiliary All refresh 继续启动/coalesce,失败不污染 visible
  presentation,也不延长 pull-to-refresh completion;
- archive/delete/new chat/pin mutation、near-tail load-more、失败 retry、空态、
  快速 `All → Chats → All` 继续沿用现有 filter-bearing ticket;
- Workspace/Bot drilldown、Task tree、deep link 与当前会话不跟随首页筛选。

### 9.4 手工/视觉与点击验收(真实 Simulator/iOS 26 runtime)

- FAB:浅色为黑圆白标,深色由 `Color(.label)` /
  `Color(.systemBackground)` 自适应反转;双层投影、不透明、不进 glass container;
  列表滚动时常驻,最后一行完整停靠且可点;点按打开原有新线程 draft;
- 菜单:从右上按钮锚定,只有 All/Chats 且顺序固定,当前项系统 checkmark;点选即生效
  收起,点空白收起;无 segmented row;
- 状态提示:Chats 时 active dot 与 `Recent · Chats` 同时出现,All 时同时消失;
  深滚后 dot 仍常驻;
- 持久化:选 Chats → 杀进程 → 重开,首帧/首请求均为 Chats 且无 All 闪帧;
  切 Gateway 仍保留 Chats;
- iOS 26 点按筛选圆与 FAB 的 glyph 外缘/圆边,确认不穿透到底层 List row;
  FAB 带内透明区域中的列表行、swipe action、pull-to-refresh 仍可交互;
- 覆盖 light/dark、紧凑 iPhone、iPad/rotation、Dynamic Type XXXL、
  VoiceOver、Reduce Transparency、Increase Contrast、Reduce Motion;
- Widget/Automation 在首页 Chats 状态下仍展示合法 All feed 行。

## 10. 预计改动文件清单

| 文件 | 动作 | 内容 |
|---|---|---|
| `mobile/garyx-mobile/App/GaryxMobile/GaryxHomeNewThreadFab.swift` | 新增 | 56pt adaptive opaque FAB + Reduce Motion press style(§5.1) |
| `mobile/garyx-mobile/App/GaryxMobile/GaryxRecentThreadFilterMenu.swift` | 新增 | 系统 Menu/inline Picker + active dot(§5.2/§6.1) |
| `mobile/garyx-mobile/App/GaryxMobile/GaryxRecentThreadFilterPicker.swift` | 删除 | segmented 形态退役(§5.3) |
| `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileSidebarViews.swift` | 修改 | header 换菜单;删 picker 与 bottom clearance;挂共享 bottom chrome;组合 `Recent · Chats` |
| `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxRecentThreadFilterStorage.swift` | 新增 | app-global stable-string load/save 契约 |
| `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxRecentThreadFeeds.swift` | 修改 | `homeMenuOptions`、`activeStatusLabel`、restored init、`resetFeedData` |
| `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileSettings.swift` | 修改 | 非 scoped `recentThreadFilter` key |
| `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel.swift` | 修改 | 从 injected defaults 在首 snapshot/request 前初始化 selection |
| `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+ThreadList.swift` | 修改 | 用户选择同步落盘后沿用既有 refresh |
| `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+Messages.swift` | 修改 | pagination reset 改用保留 selection 的 `resetFeedData` |
| `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+Navigation.swift` | 修改 | debug fixture 显式固定 selection |
| `mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxRecentThreadFilterStorageTests.swift` | 新增 | §9.1 storage 契约 |
| `mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxRecentThreadFeedsTests.swift` | 修改 | options/status/restored init/reset 矩阵 |
| `mobile/garyx-mobile/Tests/GaryxMobileCoreTests/*HomeProjection*Tests.swift` | 修改 | 首 snapshot selection/同 ids 发布回归 |
| `mobile/garyx-mobile/Tests/GaryxMobileTests/*RecentThreadFilter*Tests.swift`(或同域既有测试) | 新增/修改 | init、首请求、持久化、Gateway reset、debug fixture 集成 |
| `mobile/garyx-mobile/project.yml` | 复核 | 递归 source path 应自动包含新文件,作为 xcodegen 输入确认 |
| `mobile/garyx-mobile/GaryxMobile.xcodeproj/project.pbxproj` | 再生成 | 新增/删除 Swift 文件后运行 `xcodegen generate` 并提交;SwiftPM 假绿不豁免 app target |

`GaryxMobileDesignSystem.swift` 不预计修改:直接复用
`garyxPageBackground`、`garyxFloatingBottomChrome`、
`GaryxAdaptiveGlassContainer` 与 `garyxAdaptiveGlass`,不增加重复 helper。

## 11. 风险与验证点

1. **iOS 26 Menu 玻璃 label 的 hit-test**:现有顶栏玻璃按钮 +
   `contentShape` 已验证可点,但 `Menu` label 与 `Button` label 的手势
   接管路径不同,实现时按 L220 注释的先例做一次 26.x 点按二分验证;
   若出现同类穿透,采用会话页 scroll-to-bottom 的「玻璃降级为
   `allowsHitTesting(false)` 背景装饰 + contentShape 承接点击」写法。实测还需
   确认 active dot 位于 label 前景 `ZStack`:iOS 26.2 会丢弃挂在 glass 之后或
   `Menu` 外层的 overlay,前景 sibling 写法可稳定显示且不改变 44pt 命中区。
2. **safeAreaInset 与 List 底部触达**:验证 FAB 带内透出的列表行可点
   (chrome clear 区不拦截)、swipe actions 在最后几行不受影响。
3. **恢复时序回归**:selection 必须在 model init 内从 injected defaults 恢复,
   且先于首个 Home snapshot/首请求;不得塞回多处调用的
   `loadGatewayScopedUserState`。Gateway reset 只调用 `resetFeedData`,debug snapshot
   显式覆盖 selection。用 §9.2 的顺序测试钉住无 All 闪帧、首请求 exclude 与
   Gateway 切换保留选择。
4. **审阅点**:`GaryxRecentThreadFilterPicker` 删除后,全仓 grep 确认无
   其它引用(含 UITests);`plus.bubble` 在顶栏的旧 accessibility
   label "New chat" 由 FAB 接续,自动化脚本无断点。
5. **产品面泄漏**:菜单只读 `homeMenuOptions`,review 与测试都禁止改回
   `allCases`;UI 标签保持 All/Chats,不能引入中文「全部」。
6. **持久化误分域**:全仓确认生产 key 只有
   `garyx.mobile.recentThreadFilter`,不出现 `.<scopeId>` 变体,且
   `loadGatewayScopedUserState` / `saveGatewayScopedUserState` 不读写该 key。
