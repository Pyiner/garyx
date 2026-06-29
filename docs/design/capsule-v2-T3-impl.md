# Capsule v2 — T3 iOS UI 实现设计

> 实现级设计稿。落点+契约+测试，配合 `docs/design/capsule-v2.md`（§4 iOS、§5.7 iOS
> dumb-render、§6 update/delete、§7 性能、§8 安全）。**只碰 `mobile/`**，不动
> gateway/bridge/models/desktop（避免与 T2 冲突）。T1 已合 main：render_state 契约
> （`GaryxRenderCapsuleCard`/`RenderUserTurnRow.capsule_cards`）+ bridge 写侧 marker 已落地，
> iOS `GaryxMobileRenderState`/`GaryxMobileRenderRows` 已容忍并把 `capsuleCards` 透传进
> `GaryxMobileTurnRow.capsuleCards`，mapper `messageRefs` 已忽略 capsule cards（不误判
> unresolved）。本任务只做 iOS **UI 渲染**与 present-over-conversation 路由。

---

## 0. 读码结论（带 file:line，真相源）

- 列表入口 `GaryxCapsulesView`（`App/.../GaryxMobileCapsuleViews.swift:6`）现为 grouped
  `List` + `GaryxCapsuleRow`（`:134`）+ `fullScreenCover` detail（`:50`，绑
  `model.capsuleHTMLState.selectedCapsuleId` 经 `onChange` 设 `detailCapsule`）。detail
  `GaryxCapsuleDetailView`（`:237`）顶部 title/description/metadata stack（`:311-333`）再嵌
  `GaryxCapsuleWebView`（`:378`：`.nonPersistent()` `:388`、JS allowed、无 native bridge、
  `loadHTMLString(html,baseURL:nil)` `:405`、外链 `UIApplication.shared.open`/未知 scheme cancel
  `:411-437`）。
- Core 模型 `GaryxGatewayCapsuleModels.swift`：`GaryxCapsuleSummary`（`:20`，`Decodable` only）、
  `GaryxCapsuleHTMLCacheKey`（`:102`，**当前 3 段键 `id+revision+htmlSha256`**）、
  `GaryxCapsuleHTMLLoadState`（`:118`，单选 load 状态机）。
- Client `GaryxGatewayClient.swift`：`listCapsules()`（`:507`）、`deleteCapsule(id:)`（`:512`）、
  `capsuleHTML(id:)`（`:516`，GET `/serve`）。错误类型 `GaryxGatewayError.httpStatus(Int,String)`
  （`:116`）**携带状态码** → 可干净区分 404(deleted) vs 瞬态。
- Model 状态 `GaryxMobileModel.swift`：`@Published var capsules`（`:210`）、`@Published var
  capsuleHTMLState`（`:211`）、`var capsuleHTMLCache: [GaryxCapsuleHTMLCacheKey:String]`（`:212`，
  **非 @Published**）。`GaryxMobileModel+Capsules.swift` 有 `refreshCapsules/openCapsule/
  loadSelectedCapsuleHTML/deleteCapsule/clearCapsuleDetailState/pruneCapsuleHTMLCache`。
- 路由 `GaryxMobileModel+Navigation.swift`：`openMobileRoute(_,source:)`（`:201`）→
  `openCapsuleRoute(_,source:)`（`:276`，**当前恒 `openPanel(.capsules)`+refresh+`openCapsule`**）。
  `GaryxMobilePanelOpenSource`（Core `GaryxMobileNavigationState.swift:452`）现仅
  `.current/.sidebar/.replace`，**无 `.conversation`**；唯一 `switch source` 在
  `openRoute`（`:581-592`）。
- 渲染 `GaryxMobileTurnViews.swift`：`GaryxMobileTurnRowsView.turnRowContent`（`:38`）渲染
  `userBlock` + `ForEach(activityRows)`，**尚未渲染 `row.capsuleCards`**。
- Shell：`GaryxRootNavigationView`（`App/.../GaryxMobileSidebarViews.swift:158`）用
  `NavigationStack(path:)` push `.conversation`/`.panel`；已有 `routeNotFoundStore` 驱动的
  `fullScreenCover`（`:209`）—— **narrow-store 驱动 cover 的现成先例**。`GaryxConversationView`
  （`App/.../GaryxMobileConversationViews.swift:143`，`@EnvironmentObject model`）渲染
  `GaryxMobileTurnRowsView`（`:353`）。
- project.yml（`:37-39`）按 `App/GaryxMobile` + `Sources/GaryxMobileCore` 路径 glob → 新文件需
  `xcodegen generate` 并提交 `GaryxMobile.xcodeproj/project.pbxproj`（否则 app 编不到、swift test
  假绿，见 [[project_ios_xcodegen_pbxproj_sync]]）。

---

## 1. 范围与非目标

**做（doc §4）**：卡片画廊（grid + WKWebView 缩略图）、并发/性能 planner（Core+测试）、去套娃
focused 预览、present-over-conversation、聊天卡片哑渲染。

**不做**：不引服务端缩略图、不持久化截图、不离屏 `takeSnapshot`（首版用 live thumbnail webview；
仅当实测必须才降级，本任务不做）、不动 render_state reducer/契约（T1 已定）、不本地推导插卡、不放松
sandbox/CSP、不动 gateway/bridge/models/desktop。

---

## 2. Core 变更（`Sources/GaryxMobileCore/`，SwiftPM 测试）

### 2.1 `GaryxCapsuleHTMLCacheKey` 收为 `(id, revision)`（doc §4.3 硬要求）

把缓存主键从 `id+revision+htmlSha256` 改为 **`id+revision`**。理由：`update_capsule` 必 `revision+=1`
（html 内容变 ⟺ revision 变，doc §0），`htmlSha256` 在键中冗余；且聊天卡 wire **不带 sha**，gallery/
focused 带 sha，若 sha 进键则三处缓存分裂成 `id:rev:sha` 与 `id:rev` 两套 → 双重 fetch + 双 WKWebView
（doc §4.3 明确禁止）。

```swift
public struct GaryxCapsuleHTMLCacheKey: Hashable, Equatable, Sendable {
    public var id: String
    public var revision: Int
    public init(id: String, revision: Int) { self.id = id.trimmed; self.revision = revision }
    public init(capsule: GaryxCapsuleSummary) { self.init(id: capsule.id, revision: capsule.revision) }
}
```

- `GaryxCapsuleSummary.htmlSha256` 字段**保留**（校验/调试/catalog cache 用），只是不进缓存键。
- 影响面：`GaryxCapsuleWebView.updateUIView` 的 `loadedKey` 串删 `htmlSha256` 段；
  `GaryxCapsuleHTMLLoadStateTests` 中「sha 区分键」用例改为「同 id+rev 不同 sha → 键相等」+「rev
  区分键」。`GaryxMobileCatalogCache`/`GaryxCachedCapsule` 不受影响（存的是 summary 不是键）。

### 2.2 `GaryxCapsulePreviewLoadPlanner`（新文件，纯值类型）

可见性准入：输入按出现顺序的 visible ids + maxActive → 输出前 N 个 active ids。onAppear/onDisappear
增量上报。

```swift
public struct GaryxCapsulePreviewLoadPlanner: Equatable, Sendable {
    public private(set) var maxActive: Int
    private var visibleOrder: [String]            // 出现顺序，去重
    public init(maxActive: Int, visibleOrder: [String] = [])
    public var visibleIds: [String] { visibleOrder }
    public var activeIds: [String] { Array(visibleOrder.prefix(max(0, maxActive))) }
    public func isActive(_ id: String) -> Bool
    @discardableResult public mutating func markVisible(_ id: String) -> Bool  // append if absent
    @discardableResult public mutating func markHidden(_ id: String) -> Bool
    public mutating func setMaxActive(_ n: Int)
    public func prunedVisibleOrder(keeping valid: Set<String>) -> [String]      // capsules 变更时剔除
}
```

语义：先到先得（FIFO 准入）——最早出现且仍可见的 N 个挂 WKWebView；滚动时顶部卡 onDisappear→移除→
让位给下一个。超过 N 的可见卡显 skeleton。max：iPhone(compact) 2 / iPad(regular) 4（doc §4.3/§7）。

### 2.3 `GaryxCapsuleChatCardPresentation`（新文件或并入 planner 文件，纯格式化）

聊天卡副信息文案（doc 把 formatting 放 Core）：把 `GaryxRenderCapsuleAction` → 本地化标签
（`.created`→"Created" / `.updated`→"Updated"），可选 `· r{revision}`。SwiftPM 测试覆盖。

```swift
public enum GaryxCapsuleChatCardPresentation {
    public static func subtitle(action: GaryxRenderCapsuleAction, revision: Int) -> String
    // e.g. "Created" / "Updated"
}
```

### 2.4 `GaryxMobilePanelOpenSource` 加 `.conversation`（Core）

```swift
public enum GaryxMobilePanelOpenSource: Equatable, Sendable {
    case current; case sidebar; case replace; case conversation
}
```

`openRoute` 的 `switch source`（`:583`）把 `.conversation` 并入 `.sidebar, .replace` 分支
（present-over-conversation 不走 `openRoute` push，此分支仅为穷举完备/兜底）。

### 2.5 Core 测试

- `GaryxCapsulePreviewLoadPlannerTests`：空→无 active；mark 3、max 2 → active=前 2；hide 首→active=
  后 2；max 4→全 active；重复 markVisible 幂等不重排；setMaxActive 重算；prune 到 valid 集。
- `GaryxCapsuleHTMLLoadStateTests`：更新 sha-不再区分键 / rev 区分键；`init(capsule:)` 取 id+rev。
- `GaryxCapsuleChatCardPresentationTests`：created/updated 文案。
- `GaryxMobileRenderStateMapperTests`：T1 已覆盖 capsule_cards 解码+透传+无 unresolved；按需补一条
  「旧帧缺字段→`[]` 不崩」（T1 已有 `testUserTurnDecodesMissingCapsuleCardsAsEmpty`，无需重复）。

---

## 3. App：卡片画廊（`GaryxMobileCapsuleViews.swift` 重写）

`GaryxCapsulesView` 保留 `.garyxPageBackground()` + `garyxAdaptiveTopBar`（标题单行 "Capsules"，
leadingButton 不变）+ `.task`/`.refreshable`。内容 `List{Section}` → `ScrollView { LazyVGrid }`：

- 列：`@Environment(\.horizontalSizeClass)` compact → 固定 2 列（`GridItem(.flexible)`×2）；regular →
  `adaptive(minimum:170, maximum:260)`。间距 12-14，水平 padding 16。
- 卡片 `GaryxCapsuleGalleryCard`：
  - 上半 `GaryxCapsulePreviewThumbnail`（§4）clipped rounded rect（圆角 14，hairline 描边），
    aspect ≈ 16:10。
  - 下半：title 单行 ellipsis（空→"Untitled Capsule"）+ 1 行 small metadata（updated relative ·
    owner badge，复用 `GaryxProviderPresentation`/现有 `GaryxCapsuleOwnerBadge`，去掉 byteSize/rev
    次要 chip，保持 compact；Mac IA 为准）。
  - 整卡 `Button`（plain）→ tap 打开 focused 预览；`contextMenu` 保留 Delete（destructive，确认）。
- Empty/loading：复用 `GaryxEmptyPanelView`/`GaryxLoadingPanelView`（去掉左栏视觉）；缩略图占位用
  shimmer/skeleton（**非 ProgressView spinner**，mobile.md）。
- 画廊持有 `@StateObject coordinator: GaryxCapsulePreviewLoadCoordinator`（§4），随 sizeClass
  `setMaxActive`；卡 onAppear→`markVisible(id)`、onDisappear→`markHidden(id)`。`refreshCapsules`
  后 coordinator prune 到现存 ids。

### focused 预览呈现（gallery 内）

保留 `fullScreenCover`，但驱动改为本地 `@State focusedCapsule: GaryxCapsuleSummary?`：
- 卡 tap：`focusedCapsule = capsule; Task { await model.openCapsule(capsule) }`。
- 深链 `openCapsuleRoute(.replace/.current)` → `openPanel(.capsules)`+`openCapsule` 设
  `capsuleHTMLState.selectedCapsuleId` → gallery `onChange(of: selectedCapsuleId)` 设
  `focusedCapsule` → cover 弹出（保留深链行为）。
- cover 内容 = `GaryxCapsuleFocusedPreviewView(capsule:)`（§5）；dismiss 清 `focusedCapsule` +
  `model.clearCapsuleDetailState()`。

---

## 4. App：缩略图 WebView + planner coordinator

### 4.1 `GaryxCapsulePreviewLoadCoordinator`（app target，ObservableObject 薄壳）

```swift
@MainActor final class GaryxCapsulePreviewLoadCoordinator: ObservableObject {
    @Published private(set) var activeIds: Set<String> = []
    private var planner: GaryxCapsulePreviewLoadPlanner
    init(maxActive: Int)
    func setMaxActive(_ n: Int); func markVisible(_ id: String); func markHidden(_ id: String)
    func prune(validIds: Set<String>)
    func isActive(_ id: String) -> Bool { activeIds.contains(id) }
    // 每次 mutate 后 recompute：next = Set(planner.activeIds)，变了才 publish
}
```

放 model 之外的窄 store（避免整 model `@Published` 在每次滚动 onAppear 重渲，守
[[project_ios_home_list_v4_rebuild]] 窄观测教训）。gallery `@StateObject` 持一个（全网格 ≤N）；
聊天卡每个 `GaryxMobileCapsuleChatCardsView`（每 turn）`@StateObject` 持自己的（每 turn ≤N，配合
LazyVStack 只挂可见 turn → 全局并发天然有界，无需跨 turn 线程串联）。

### 4.2 `GaryxCapsulePreviewThumbnail`（按需挂 WKWebView）

```swift
struct GaryxCapsulePreviewThumbnail: View {
    let capsuleId: String; let revision: Int; let isActive: Bool; let cornerRadius: CGFloat
    @EnvironmentObject var model; @State private var phase: Phase = .idle
    enum Phase: Equatable { case idle, loading, loaded(String), deleted, failed }
    // body：.loaded(html) 且 isActive → GaryxCapsulePreviewWebView；.deleted → "Capsule deleted"
    //       disabled 占位；.failed → 可重试占位；其它 → skeleton
    // .task(id: key(capsuleId,revision,isActive))：guard isActive；已 loaded 跳过；
    //   else await model.loadCapsulePreviewHTML(capsuleId:revision:) → set phase
}
```

- `isActive` false → 不进 webview 分支（离屏即卸载 webview，doc §7）；再 active 时 phase 多已
  `.loaded` → 立即复挂、不重 fetch。
- 缓存命中（gallery 先于 chat、或反之）：`loadCapsulePreviewHTML` 读共享 `capsuleHTMLCache[(id,rev)]`
  → 命中即返回，零重复 fetch（§2.1 共享键的目的）。

### 4.3 `GaryxCapsulePreviewWebView`（缩略图 UIViewRepresentable）

虚拟画布 760×480 渲染、缩放贴卡宽（doc §4.3）：

```text
GeometryReader(读卡宽 W) →
  Color.clear.frame(width:W, height:W*480/760)
    .overlay(alignment:.topLeading) {
        WKWebView 容器.frame(width:760,height:480)
          .scaleEffect(W/760, anchor:.topLeading)
    }
    .clipped()
```

WKWebView 配置（硬化，doc §8）：`websiteDataStore=.nonPersistent()`、JS allowed、
`javaScriptCanOpenWindowsAutomatically=false`、**无 `WKScriptMessageHandler`**、
`isUserInteractionEnabled=false`、`scrollView.isScrollEnabled=false`/`bounces=false`、
`isOpaque=false`、`loadHTMLString(html, baseURL:nil)`；navigation delegate 对子帧 allow、主帧
http(s)/mailto→`UIApplication.shared.open` + cancel、about→allow、未知 scheme→cancel（同
focused，缩略图禁交互所以基本不触发）。`updateUIView` 用 `(id:rev:html.count:hashValue)` 守卫只在
内容变时 reload。

---

## 5. App：focused 去套娃预览（`GaryxCapsuleDetailView` → `GaryxCapsuleFocusedPreviewView`）

doc §4.4：网页全屏专注，移除 title/description/metadata stack，极简 overlay。

- 结构：`GaryxCapsuleWebView`（**保留 focused 版**，允许交互）填满内容区 + 顶部细条
  overlay（`garyxAdaptiveTopBar` 风格 glass，~44pt）：
  - 左：close/back（`xmark`/`chevron.down`）→ dismiss。
  - 中（可选）：单行小字 title，透明底，不挡网页（doc 允许「可选短 title」）。
  - 右：refresh（`arrow.clockwise`，`loadSelectedCapsuleHTML(forceRefresh:true)`）+ overflow
    Menu（`ellipsis`）：Copy link（`GaryxMobileRouteLink.make(.capsule(id))` →
    `garyx://mobile/capsule?id=<id>`，`UIPasteboard`）/ Copy ID（裸 UUID）/ Delete（destructive，
    确认 → `deleteCapsule` 后 dismiss）。
- 内容态（复用现有 `model.capsuleHTMLState`/`loadSelectedCapsuleHTML`，doc §4.1「复用 loadState」）：
  loading → shimmer 占位；`.html` → webview；`.failure` → 失败占位（serve 404 文案统一 "Capsule
  deleted"，瞬态/5xx → 可重试，不误标 deleted，doc §6）。
- `.task(id: capsule.id)`：未选/未加载则 `openCapsule`/`loadSelectedCapsuleHTML`（同现有 detail）。
- 安全：webview 同 v1（nonPersistent / 无 message handler / baseURL nil / 外链 open / 未知 cancel）。

该视图被 gallery cover 与 conversation cover 共用。

---

## 6. present-over-conversation 路由（doc §4/§5.7/R10）

聊天卡 tap 必须在**当前会话上方** present focused 预览、关闭回会话，**不切 Capsules panel/overview**
（守 mobile-ui「drilldown never back to overview」）。

### 6.1 model 状态（app target，R10：preview 提升到 model）

`GaryxMobileModel.swift` 加 `@Published var conversationCapsulePreview: GaryxCapsuleSummary?`。

### 6.2 路由分流（`GaryxMobileModel+Navigation.swift`）

```swift
private func openCapsuleRoute(_ id, source) async {
    let capsuleId = id.trimmed; guard !capsuleId.isEmpty else { return }
    if source == .conversation { await presentConversationCapsulePreview(capsuleId); return }
    openPanel(.capsules, source: source); await refreshCapsules()
    guard let capsule = capsules.first(where:{$0.id==capsuleId}) else { showRouteNotFound(...); return }
    await openCapsule(capsule)
}
```

`presentConversationCapsulePreview`（`+Capsules.swift`）：
```swift
func presentConversationCapsulePreview(_ capsuleId: String) async {
    if capsules.first(where:{$0.id==capsuleId}) == nil { await refreshCapsules() }
    let capsule = capsules.first(where:{$0.id==capsuleId})
        ?? GaryxCapsuleSummary(id: capsuleId, title: "Capsule")   // 删后边缘：合成最小 summary，
    conversationCapsulePreview = capsule                          // 预览内 serve 404 → "deleted"
    await openCapsule(capsule)                                    // 复用 capsuleHTMLState 单选加载
}
func dismissConversationCapsulePreview() { conversationCapsulePreview = nil; clearCapsuleDetailState() }
```
- 不在 view 查 `/api/capsules` 判存在（layering，doc §5.7）；只走 public route/model。
- 删后边缘（缩略图已加载但点开瞬间被删）：合成最小 summary → focused 预览 serve 404 → "Capsule
  deleted"，**不弹 route-not-found alert**（避免会话上突兀全屏），不切 panel。
- gallery/deep-link 仍走非 conversation 分支（panel + focused），行为不变。

### 6.3 cover 挂载（`GaryxConversationView`）

会话 body 加：
```swift
.fullScreenCover(item: Binding(
    get: { model.conversationCapsulePreview },
    set: { if $0 == nil { model.dismissConversationCapsulePreview() } }
)) { capsule in GaryxCapsuleFocusedPreviewView(capsule: capsule) }
```
`GaryxCapsuleSummary` 已 `Identifiable`。present 在会话视图层 → 关闭回会话（满足 present-over-
conversation）。gallery cover 与此 cover 不会同屏（不同 route），共享 `capsuleHTMLState` 安全。

---

## 7. App：聊天卡片哑渲染（doc §5.7）

T1 已把 `row.capsuleCards: [GaryxRenderCapsuleCard]` 备好。view 只摆卡、不推导。

- `GaryxMobileTurnViews.swift` `turnRowContent`（`:38`）在 `ForEach(activityRows)` **之后**追加：
  ```swift
  if !row.capsuleCards.isEmpty {
      GaryxMobileCapsuleChatCardsView(cards: row.capsuleCards)
          .transition(.garyxTranscriptAppear)
  }
  ```
- `GaryxMobileCapsuleChatCardsView`（`GaryxMobileCapsuleViews.swift`）：纵向 stack（通常 1 张，compact），
  每张：上半 `GaryxCapsulePreviewThumbnail`（per-turn coordinator，onAppear/onDisappear 上报）、下半
  title（`card.title`，空→"Untitled Capsule"）+ subtitle（`GaryxCapsuleChatCardPresentation.subtitle`）。
  tap → `Task { await model.openMobileRoute(.capsule(card.capsuleId), source:.conversation) }`。
  缩略图 `.deleted` → disabled "Capsule deleted"，tap 禁用，不 `capsuleHTML`。
- mapper `messageRefs` 已不含 capsule cards（T1），不计 visible message id，不影响
  `isAwaitingInitialHistory`/分组。

---

## 8. 客户端 update/delete 语义（doc §6）

- `loadCapsulePreviewHTML(capsuleId:revision:)`（`+Capsules.swift`，多实例缩略图取数）：
  ```swift
  enum GaryxCapsulePreviewHTMLResult: Equatable { case html(String), deleted, failed }
  func loadCapsulePreviewHTML(capsuleId, revision) async -> GaryxCapsulePreviewHTMLResult {
      let key = GaryxCapsuleHTMLCacheKey(id:capsuleId, revision:revision)
      if let cached = capsuleHTMLCache[key] { return .html(cached) }
      do { let html = try await client().capsuleHTML(id:capsuleId); capsuleHTMLCache[key]=html; return .html(html) }
      catch let e as GaryxGatewayError where { if case .httpStatus(404,_) = e } { return .deleted }
      catch { return .failed }   // 瞬态/5xx/离线 → 可重试，绝不误标 deleted（doc §6）
  }
  ```
  只写非-@Published `capsuleHTMLCache`，结果回卡的 `@State`（不触发整会话重渲，守
  [[project_ios_home_list_scroll_jank]] 不让取数砸主线程/整页重算）。
- update 后 freshness：server render_state 升 revision（T1 全局最新）→ 卡 `(id,rev)` 键变 →
  缩略图/预览自然 refetch 新内容；gallery 同理（refreshCapsules 取新 revision）。
- delete：聊天 marker 仍在 → 卡仍派生，但 serve 404 → 缩略图/预览 disabled "Capsule deleted"。即时性
  靠重连/重开/下一帧（doc §6 首版可接受；不在本任务做 DELETE handler 推帧——那是 gateway，越界）。

---

## 9. 性能与安全（doc §7/§8）

- 性能：LazyVGrid + onAppear/onDisappear 只表达可见性，准入在 Core planner（≤2 iPhone / ≤4 iPad）；
  离屏卸载 webview；`capsuleHTMLCache` 按 `(id,rev)` prune（refresh/delete 时），首版保留量沿用现有
  prune（按现存 capsules 的键）。聊天卡靠 LazyVStack 只挂可见 turn + per-turn ≤N。
- 安全：缩略图/focused 同 v1 边界——`.nonPersistent()`、JS allowed、**无 message handler**、
  `loadHTMLString(baseURL:nil)`（meta CSP 仍生效）、缩略图禁交互、外链 `UIApplication.shared.open`、
  未知 scheme cancel；title/metadata 是 native text 非 HTML 注入；Copy link 用 deep link 不含 token。

---

## 10. 测试与验收

- **Core（headless 优先）**：`GaryxCapsulePreviewLoadPlannerTests`、`GaryxCapsuleChatCardPresentationTests`、
  更新 `GaryxCapsuleHTMLLoadStateTests`（键收为 id+rev）、`GaryxMobileRenderStateMapperTests`（T1
  覆盖，按需补）。`swift test` 全绿。
- **构建**：`xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator
  -configuration Debug build CODE_SIGNING_ALLOWED=NO` → **真看 BUILD SUCCEEDED**（不被 `tail` exit 0
  骗，[[project_ios_xcodegen_pbxproj_sync]]）。
- **xcodegen**：新 Core/app 文件后 `xcodegen generate` + 提交 `project.pbxproj`。
- 模拟器 boot 自验截图可选；三端 e2e 截图留 T4。

## 11. 文件清单

新增：`Sources/GaryxMobileCore/GaryxCapsulePreviewLoadPlanner.swift`（含 ChatCardPresentation 或拆
`GaryxCapsuleChatCardPresentation.swift`）、`Tests/GaryxMobileCoreTests/
GaryxCapsulePreviewLoadPlannerTests.swift`（+ presentation 测试）。

改：`GaryxGatewayCapsuleModels.swift`（键收为 id+rev）、`GaryxMobileNavigationState.swift`
（`.conversation` 源）、`GaryxMobileCapsuleViews.swift`（画廊/缩略图/focused/coordinator/聊天卡视图）、
`GaryxMobileTurnViews.swift`（渲染 capsuleCards）、`GaryxMobileConversationViews.swift`（conversation
cover）、`GaryxMobileModel.swift`（`conversationCapsulePreview`）、`GaryxMobileModel+Capsules.swift`
（`loadCapsulePreviewHTML`/`presentConversationCapsulePreview`/`dismiss`）、`GaryxMobileModel+Navigation.swift`
（`openCapsuleRoute` 分流）、`Tests/.../GaryxCapsuleHTMLLoadStateTests.swift`（键变更）、
`GaryxMobile.xcodeproj/project.pbxproj`（xcodegen）。

## 12. 风险

| # | 风险 | 缓解 |
|---|---|---|
| R1 | 改缓存键波及现有 v1 detail/cache | 仅删键中 sha（rev 已覆盖失效），全量改 `loadedKey` 串与一处测试；catalog cache 不碰 |
| R2 | 多 WKWebView 内存/卡顿 | Core planner ≤N 准入 + 离屏卸载 + LazyVGrid/VStack + 共享 (id,rev) 缓存零重复 fetch |
| R3 | present-over-conversation 时序/返回语义 | preview 提升到 model `@Published` + 会话层 `fullScreenCover(item:)`，dismiss 回会话；不切 panel；deep-link/gallery 仍走 panel 分支 |
| R4 | 删后边缘点开 | 合成最小 summary → focused serve 404 → "deleted"，不弹 route-not-found，不切 panel |
| R5 | 越 T1/契约线 | 只读 `row.capsuleCards` 摆卡，不扫 tool result、不本地查 capsules 决定插卡、不动 reducer/gateway |
| R6 | pbxproj 未提交 → 假绿 | xcodegen + 提交 pbxproj + 必跑 xcodebuild 看 SUCCEEDED |
