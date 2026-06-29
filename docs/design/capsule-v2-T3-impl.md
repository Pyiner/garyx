# Capsule v2 — T3 iOS UI 实现设计

> 实现级设计稿。落点+契约+测试，配合 `docs/design/capsule-v2.md`（§4 iOS、§5.7 iOS
> dumb-render、§6 update/delete、§7 性能、§8 安全）。**只碰 `mobile/`**，不动
> gateway/bridge/models/desktop（避免与 T2 冲突）。T1 已合 main：render_state 契约
> （`GaryxRenderCapsuleCard`/`RenderUserTurnRow.capsule_cards`）+ bridge 写侧 marker 已落地，
> iOS `GaryxMobileRenderState`/`GaryxMobileRenderRows` 已容忍并把 `capsuleCards` 透传进
> `GaryxMobileTurnRow.capsuleCards`，mapper `messageRefs` 已忽略 capsule cards（不误判
> unresolved）。本任务只做 iOS **UI 渲染**与 present-over-conversation 路由。
>
> **rev2（codex #TASK-1434 NOT-PASS 后修订）**：B1 focused 预览改按 id 直取 HTML（不依赖
> `capsules` 查找），统一 `loadCapsulePreviewHTML` 取数 + 404 分类，移除 v1 单选
> `GaryxCapsuleHTMLLoadState`；B2 聊天卡因会话是 **eager VStack**（非 Lazy）改会话级「最近 N」
> 确定性准入（纯函数），全局严格 ≤N。

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
  `GaryxCapsuleHTMLLoadState`（`:118`，**v1 单选** load 状态机，仅被 v1 detail/model 用）。
- Client `GaryxGatewayClient.swift`：`listCapsules()`（`:507`）、`deleteCapsule(id:)`（`:512`）、
  `capsuleHTML(id:)`（`:516`，GET `/serve`）。错误类型 `GaryxGatewayError.httpStatus(Int,String)`
  （`:116`）**携带状态码** → 干净区分 404(deleted) vs 瞬态。
- Model 状态 `GaryxMobileModel.swift`：`@Published var capsules`（`:210`）、`@Published var
  capsuleHTMLState`（`:211`）、`var capsuleHTMLCache: [GaryxCapsuleHTMLCacheKey:String]`（`:212`，
  **非 @Published**）。`GaryxMobileModel+Capsules.swift`：`selectedCapsule`（`:5`，**只从
  `capsules` 查**）、`loadSelectedCapsuleHTML`（`:40`，**lookup 失败即早返回不 fetch**）、
  `refreshCapsules/openCapsule/deleteCapsule/clearCapsuleDetailState/pruneCapsuleHTMLCache`。
- 路由 `GaryxMobileModel+Navigation.swift`：`openMobileRoute(_,source:)`（`:201`，入口先
  `clearRouteDrivenDetailState()` `:205`）→ `openCapsuleRoute(_,source:)`（`:276`，**当前恒
  `openPanel(.capsules)`+refresh+`openCapsule`**）。`clearRouteDrivenDetailState`（`:364`）调
  `clearCapsuleDetailState()`。`GaryxMobilePanelOpenSource`（Core `GaryxMobileNavigationState.swift:452`）
  现仅 `.current/.sidebar/.replace`，**无 `.conversation`**；唯一 `switch source` 在
  `openRoute`（`:581-592`）。
- 渲染 `GaryxMobileTurnViews.swift`：`GaryxMobileTurnRowsView`（`:12`，唯一调用方
  `GaryxMobileConversationViews.swift:353`）`turnRowContent`（`:38`）渲染 `userBlock` +
  `ForEach(activityRows)`，**尚未渲染 `row.capsuleCards`**。
- 会话 `GaryxMobileConversationViews.swift`：`GaryxConversationView`（`:143`，`@EnvironmentObject
  model`）`messageScroll`（`:306`）是**故意的 eager `VStack`**（`:308-313` 注释说明为何不用
  LazyVStack）→ **所有 turn 行同时挂载，onAppear 非可见性信号**；坐标空间
  `.coordinateSpace(name:"garyx-conversation-scroll")`（`:401`）；body 末模块链在 `:287-291`
  （`garyxPageBackground`/`garyxAdaptiveTopBar`/`environment`）= conversation cover 挂点。
  `turnRows = model.selectedThreadTurnRows()`（`:326`）。
- Shell：`GaryxRootNavigationView`（`App/.../GaryxMobileSidebarViews.swift:158`）已有
  `routeNotFoundStore` 驱动的 `fullScreenCover`（`:209`）= narrow-store 驱动 cover 先例。
- project.yml（`:37-39`）按 `App/GaryxMobile` + `Sources/GaryxMobileCore` 路径 glob → 新文件需
  `xcodegen generate` 并提交 `GaryxMobile.xcodeproj/project.pbxproj`（否则 app 编不到、swift test
  假绿，见 [[project_ios_xcodegen_pbxproj_sync]]）。部署目标 **iOS 17.0**（`onScrollVisibilityChange`
  是 18+ → 不可用；可见性须 onAppear/onDisappear[懒]或确定性准入[eager]）。

---

## 1. 范围与非目标

**做（doc §4）**：卡片画廊（grid + WKWebView 缩略图）、并发/性能 planner（Core+测试）、去套娃
focused 预览、present-over-conversation、聊天卡片哑渲染。

**不做**：不引服务端缩略图、不持久化截图、不离屏 `takeSnapshot`（首版用 live thumbnail webview）、
不动 render_state reducer/契约（T1 已定）、不本地推导插卡、不放松 sandbox/CSP、不动
gateway/bridge/models/desktop。

---

## 2. Core 变更（`Sources/GaryxMobileCore/`，SwiftPM 测试）

### 2.1 `GaryxCapsuleHTMLCacheKey` 收为 `(id, revision)`（doc §4.3 硬要求）

把缓存主键从 `id+revision+htmlSha256` 改为 **`id+revision`**。

```swift
public struct GaryxCapsuleHTMLCacheKey: Hashable, Equatable, Sendable {
    public var id: String
    public var revision: Int
    public init(id: String, revision: Int) {
        self.id = id.trimmingCharacters(in: .whitespacesAndNewlines); self.revision = revision
    }
    public init(capsule: GaryxCapsuleSummary) { self.init(id: capsule.id, revision: capsule.revision) }
}
```

> 文档片段里的 `id.trimmed`/`.trimmed` 仅为简写；实现一律用
> `trimmingCharacters(in: .whitespacesAndNewlines)`（Core 无 `.trimmed` 扩展，codex 非阻断提醒）。

- 失效语义（**保守，非不安全**）：`update_capsule` 每次 `revision+=1`（`garyx_db/mod.rs:720`，含
  metadata-only update）→ html 内容变必伴随 revision 变；`(id,revision)` 至多对 metadata-only
  update **多失效一次**（重 fetch 同内容），**绝不漏失效**。`htmlSha256` 在键中冗余且聊天卡 wire
  **不带 sha** → 若 sha 进键则三处缓存分裂成 `id:rev:sha` 与 `id:rev` 两套（双重 fetch + 双
  WKWebView，doc §4.3 明确禁止），故收为 `(id,rev)` 共享。
- `GaryxCapsuleSummary.htmlSha256` 字段**保留**（catalog cache/调试）。影响面：`GaryxCapsuleWebView`
  的 `loadedKey` 串删 sha 段；cache-key 测试改为「同 id+rev 不同 sha → 键相等」。
  `GaryxMobileCatalogCache`/`GaryxCachedCapsule` 不受影响（存 summary 非键）。

### 2.2 移除 v1 单选 `GaryxCapsuleHTMLLoadState`（B1 结构性修复）

v1 `capsuleHTMLState`/`loadSelectedCapsuleHTML`/`selectedCapsule` 单选状态机有两条死路：(a) 只从
`model.capsules` 查 → 删后/合成 summary 查不到 → `loadSelectedCapsuleHTML` 早返回不 fetch → 无
404、无 "deleted"；(b) 单选无法驱动画廊多缩略图。**统一改为按 id 直取的
`loadCapsulePreviewHTML(id,revision,forceRefresh:)`（§8）+ 每视图 `@State`**，画廊缩略图/聊天卡缩略图/
focused 预览三处共用。`GaryxCapsuleHTMLLoadState` 及 model `capsuleHTMLState`、`openCapsule`、
`loadSelectedCapsuleHTML`、`selectedCapsule`、`isCapsuleHTMLLoaded`、`clearCapsuleDetailState` 删除
（被替换的 v1 detail 是唯一用户）；`capsuleHTMLCache`（`[键:String]`）**保留**为共享缓存。
`GaryxCapsuleHTMLLoadStateTests` 删除；cache-key 用例并入 `GaryxGatewayCapsuleModelsTests`。

### 2.3 `GaryxCapsulePreviewLoadPlanner`（新文件，**画廊**可见性准入）

画廊 = `ScrollView+LazyVGrid`（**懒**，onAppear/onDisappear 是真可见性）。planner 纯值类型：visible
ids 按出现顺序、prefix N（FIFO 准入），≤N 挂 WKWebView，余显 skeleton。

```swift
public struct GaryxCapsulePreviewLoadPlanner: Equatable, Sendable {
    public private(set) var maxActive: Int
    private var visibleOrder: [String]            // 出现顺序，去重
    public init(maxActive: Int, visibleOrder: [String] = [])
    public var activeIds: [String] { Array(visibleOrder.prefix(max(0, maxActive))) }
    public func isActive(_ id: String) -> Bool
    @discardableResult public mutating func markVisible(_ id: String) -> Bool  // append if absent
    @discardableResult public mutating func markHidden(_ id: String) -> Bool
    public mutating func setMaxActive(_ n: Int)
    public func prunedVisibleOrder(keeping valid: Set<String>) -> [String]
}
```

max：iPhone(compact) 2 / iPad(regular) 4（doc §4.3/§7）。

### 2.4 `GaryxCapsuleChatCardAdmission`（新，**聊天卡**会话级确定性准入，B2 修复）

会话是 **eager VStack** → onAppear 非可见性、20 个历史卡会全挂 → per-turn coordinator 失效、破 N 上限
（codex B2 反例）。改**会话级**纯函数准入：把全会话有序 card 实例 key（`"<turnId>:<capsuleId>"`，
按 turn 顺序 flatten）取 **suffix(N)**（最近 N 张，因默认滚到底→最近=最可能可见），全局严格 ≤N 个
缩略图 webview；非 active 卡显静态 shell（icon+title），tap 仍开 focused 预览。

```swift
public enum GaryxCapsuleChatCardAdmission {
    /// 有序实例 key（newest 在尾）→ active = 最近 N
    public static func activeKeys(orderedKeys: [String], maxActive: Int) -> [String] {
        Array(orderedKeys.suffix(max(0, maxActive)))
    }
}
```

用实例 key（含 turnId）而非裸 capsuleId：同一 capsule 在「create 轮 + update 轮」两 turn 各一卡
（doc §6），实例 key 唯一 → suffix(N) 严格按 webview 实例数封顶（HTML 缓存仍按 `(id,rev)` 共享，
同 capsule 零重复 fetch）。

### 2.5 `GaryxCapsuleChatCardPresentation`（新或并入，纯格式化）

`GaryxRenderCapsuleAction` → 本地化副信息（`.created`→"Created" / `.updated`→"Updated"）。Core+测试。

### 2.6 `GaryxMobilePanelOpenSource` 加 `.conversation`（Core）

```swift
public enum GaryxMobilePanelOpenSource { case current; case sidebar; case replace; case conversation }
```
`openRoute` 的 `switch source`（`:583`）把 `.conversation` 并入 `.sidebar, .replace` 分支
（present-over-conversation 不走 push，仅穷举完备/兜底）。

### 2.7 Core 测试

- `GaryxCapsulePreviewLoadPlannerTests`：空→无 active；mark 3、max 2→active 前 2；hide 首→后 2；
  max 4→全；幂等不重排；setMaxActive 重算；prune。
- `GaryxCapsuleChatCardAdmissionTests`：空→[]；3 key、max 2→**后 2**（最近）；max≥n→全；max 0→[]。
- `GaryxCapsuleChatCardPresentationTests`：created/updated 文案。
- cache-key（并入 `GaryxGatewayCapsuleModelsTests`）：同 id+rev 不同 sha 键相等；rev 区分键；
  `init(capsule:)` 取 id+rev。
- `GaryxMobileRenderStateMapperTests`：T1 已覆盖 capsule_cards 解码+透传+无 unresolved+旧帧缺字段→`[]`。

---

## 3. App：卡片画廊（`GaryxMobileCapsuleViews.swift` 重写）

`GaryxCapsulesView` 保留 `.garyxPageBackground()` + `garyxAdaptiveTopBar`（标题单行 "Capsules"，
leadingButton 不变）+ `.task`/`.refreshable`。内容 `List{Section}` → `ScrollView { LazyVGrid }`：

- 列：`@Environment(\.horizontalSizeClass)` compact → 固定 2 列；regular → `adaptive(minimum:170,
  maximum:260)`。间距 12-14，水平 padding 16。
- 卡 `GaryxCapsuleGalleryCard`：上半 `GaryxCapsulePreviewThumbnail`（§4，`isActive =
  coordinator.activeIds.contains(id)`、`cacheEpoch = model.capsuleHTMLCacheEpoch`，clipped 圆角 14 +
  hairline，aspect≈16:10）；下半 title 单行 ellipsis（空→"Untitled Capsule"）+ 1 行 small metadata（updated
  relative · owner badge，复用现有 `GaryxCapsuleOwnerBadge`/`GaryxProviderPresentation`，去掉
  byteSize/rev 次要 chip 保 compact）。整卡 `Button`（plain）→ tap 设 `model.galleryFocusedCapsule
  = capsule`；`contextMenu` 保留 Delete（destructive，确认）。
- Empty/loading 复用 `GaryxEmptyPanelView`/`GaryxLoadingPanelView`；缩略图占位 shimmer/skeleton
  （**非 ProgressView**，mobile.md）。
- 画廊 `@StateObject coordinator: GaryxCapsulePreviewLoadCoordinator`（§4），随 sizeClass
  `setMaxActive(2/4)`；卡 onAppear→`markVisible(id)`、onDisappear→`markHidden(id)`；`refreshCapsules`
  后 `coordinator.prune(现存 ids)`。

### focused 预览呈现（gallery cover）

`.fullScreenCover(item:)` 绑 `model.galleryFocusedCapsule`（`@Published`，§6）：卡 tap / 深链
`openCapsuleRoute(.replace/.current)`（§6）均设它 → cover 弹 `GaryxCapsuleFocusedPreviewView(capsule:)`；
dismiss 清空。**不再用 `capsuleHTMLState.selectedCapsuleId`/onChange**。

---

## 4. App：缩略图 WebView + planner coordinator

### 4.1 `GaryxCapsulePreviewLoadCoordinator`（app target，画廊用）

```swift
@MainActor final class GaryxCapsulePreviewLoadCoordinator: ObservableObject {
    @Published private(set) var activeIds: Set<String> = []
    private var planner: GaryxCapsulePreviewLoadPlanner
    init(maxActive: Int)
    func setMaxActive(_ n); func markVisible(_ id); func markHidden(_ id); func prune(validIds:)
    func isActive(_ id) -> Bool { activeIds.contains(id) }   // mutate 后 recompute，变了才 publish
}
```

放 model 外的窄 store（避免整 model `@Published` 在每次滚动 onAppear 重渲，守
[[project_ios_home_list_v4_rebuild]]）。**仅画廊用**（LazyVGrid 懒加载，可见性有效）。聊天卡不用
coordinator，用 §2.4 确定性 activeKeys（eager VStack 无真可见性）。

### 4.2 `GaryxCapsulePreviewThumbnail`（共享，按需挂 WKWebView）

```swift
struct GaryxCapsulePreviewThumbnail: View {
    let capsuleId: String; let revision: Int; let isActive: Bool
    let cacheEpoch: Int; let cornerRadius: CGFloat       // cacheEpoch：父读 model.capsuleHTMLCacheEpoch 下传
    @EnvironmentObject var model; @State private var phase: Phase = .idle
    enum Phase: Equatable { case idle, loading, loaded(String), deleted, failed }
    // body：.loaded(html) 且 isActive → GaryxCapsulePreviewWebView；.deleted → "Capsule deleted"
    //       disabled 占位；.failed → 可重试占位；else → skeleton
    // .task(id: ThumbKey(capsuleId, revision, isActive, cacheEpoch)) {
    //     guard isActive else { return }
    //     phase = (await model.loadCapsulePreviewHTML(capsuleId, revision)).toPhase()  // cache-first 内置
    // }
}
```

- **去掉「已 .loaded 跳过」守卫**：`.task(id:)` 本就按身份去重（id 不变不重跑）；任务每次按 cache/fetch
  **重对账**（cache 命中即返回、零重复 fetch）。`cacheEpoch` 进身份 → prune 真驱逐时（§8.1 #1 自增）
  **已挂载**缩略图 id 变 → task 重跑 → 对已删 capsule cache-miss→fetch→404→`.deleted`（修 codex r3：
  prune 不改 task id/不清本地 loaded phase 的盲点）。对仍存在 capsule = cache 命中、无 fetch、无变化。
- `isActive` 来源：画廊 = `coordinator.activeIds.contains(id)`；聊天卡 = `activeKeys.contains(实例key)`
  （§7 线程下传）。false → 不进 webview 分支（离屏卸载，doc §7）；再 active（id 变）→ task 重跑、
  cache 命中立即复挂。
- 缓存命中（gallery↔chat↔focused 共享 `(id,rev)`）：`loadCapsulePreviewHTML` 读 `capsuleHTMLCache`
  命中即返回，零重复 fetch。

### 4.3 `GaryxCapsulePreviewWebView`（缩略图 UIViewRepresentable）

虚拟画布 760×480 渲染、缩放贴卡宽（doc §4.3）：

```text
GeometryReader(卡宽 W) → Color.clear.frame(W, W*480/760)
  .overlay(.topLeading){ WKWebView容器.frame(760,480).scaleEffect(W/760, anchor:.topLeading) }.clipped()
```

WKWebView 硬化（doc §8）：`websiteDataStore=.nonPersistent()`、JS allowed、
`javaScriptCanOpenWindowsAutomatically=false`、**无 `WKScriptMessageHandler`**、
`isUserInteractionEnabled=false`、`scrollView.isScrollEnabled=false`/`bounces=false`、`isOpaque=false`、
`loadHTMLString(html, baseURL:nil)`；navigation delegate 子帧 allow / 主帧 http(s)/mailto→
`UIApplication.shared.open`+cancel / about→allow / 未知 scheme→cancel（缩略图禁交互基本不触发）。
`updateUIView` 用 `(id:rev:html.count:hashValue)` 守卫只在内容变时 reload。

---

## 5. App：focused 去套娃预览（`GaryxCapsuleDetailView` → `GaryxCapsuleFocusedPreviewView`）

doc §4.4：网页全屏专注，移除 title/description/metadata stack，极简 overlay。**按 id 直取，B1 修复。**

- 入参 `capsule: GaryxCapsuleSummary`（可为合成的 deleted summary）。本地 `@State phase: {loading,
  html(String), deleted, failed}`；`.task(id: "\(capsule.id):\(capsule.revision)")`：
  `await model.loadCapsulePreviewHTML(capsule.id, capsule.revision, forceRefresh: true)` → set phase
  （**不依赖 `model.capsules` 查找**，删后/合成也能 fetch→404→deleted；force-refresh 给即时权威 404，
  §8.1 防线 2 —— 全屏面永不陈旧）。refresh 按钮亦 `forceRefresh:true`。
- 结构：`GaryxCapsuleWebView`（**保留 focused 版**，允许交互）填满 + 顶部细条 overlay（glass，~44pt）：
  左 close/back（`xmark`/`chevron.down`）→ dismiss；中可选单行小字 title（透明底）；右 refresh
  （`arrow.clockwise` → `loadCapsulePreviewHTML(...,forceRefresh:true)`）+ overflow Menu（`ellipsis`）：
  Copy link（`GaryxMobileRouteLink.make(.capsule(id))` → `garyx://mobile/capsule?id=<id>`，
  `UIPasteboard`）/ Copy ID（裸 UUID）/ Delete（destructive，确认 → `deleteCapsule` 后 dismiss）。
- 内容态：loading → shimmer；`.html` → webview；`.deleted` → "Capsule deleted"；`.failed` → 可重试
  （瞬态/5xx → 不误标 deleted，doc §6）。
- 安全：webview 同 v1（nonPersistent / 无 message handler / baseURL nil / 外链 open / 未知 cancel）。

gallery cover 与 conversation cover 共用本视图。

---

## 6. present-over-conversation 路由（doc §4/§5.7/R10）

聊天卡 tap 在**当前会话上方** present、关闭回会话，**不切 Capsules panel/overview**。

### 6.1 model 状态（app target，R10）

`GaryxMobileModel.swift`：删 `@Published capsuleHTMLState`；加
`@Published var galleryFocusedCapsule: GaryxCapsuleSummary?`（画廊 cover）、
`@Published var conversationCapsulePreview: GaryxCapsuleSummary?`（会话 cover）、
`@Published var capsuleHTMLCacheEpoch: Int = 0`（缩略图失效信号，§8.1 #1）。两 cover 字段皆
`GaryxCapsuleSummary?`（已 `Identifiable`），present 同一 `GaryxCapsuleFocusedPreviewView`；不同 route
不会同屏。

### 6.2 路由分流（`GaryxMobileModel+Navigation.swift`）

```swift
private func openCapsuleRoute(_ id, source) async {
    let capsuleId = id.trimmingCharacters(in: .whitespacesAndNewlines); guard !capsuleId.isEmpty else { return }
    if source == .conversation { await presentConversationCapsulePreview(capsuleId); return }
    openPanel(.capsules, source: source); await refreshCapsules()
    guard let capsule = capsules.first(where:{$0.id==capsuleId}) else { showRouteNotFound(...); return }
    galleryFocusedCapsule = capsule          // 深链/面板：画廊 cover
}
```

`presentConversationCapsulePreview`（`+Capsules.swift`，**不再调 openCapsule**）：
```swift
func presentConversationCapsulePreview(_ id: String) async {
    if capsules.first(where:{$0.id==id}) == nil { await refreshCapsules() }
    conversationCapsulePreview = capsules.first(where:{$0.id==id})
        ?? GaryxCapsuleSummary(id: id, title: "Capsule")   // 删后边缘：合成→focused 按 id fetch→404→deleted
}
```
`clearRouteDrivenDetailState`（`:364`）：`clearCapsuleDetailState()` 删，改为
`galleryFocusedCapsule = nil; conversationCapsulePreview = nil`（导航到非 capsule route 时 dismiss
任意打开的预览；conversation 分流在 clear 之后 set，不被清）。
- 不在 view 查 `/api/capsules` 判存在（layering，doc §5.7）；只走 public route/model。删后边缘**不弹
  route-not-found**（合成 summary → focused 内显 deleted），不切 panel。

### 6.3 cover 挂载（`GaryxConversationView` body 末 `:291` 后）

```swift
.fullScreenCover(item: Binding(
    get: { model.conversationCapsulePreview },
    set: { if $0 == nil { model.conversationCapsulePreview = nil } }
)) { capsule in GaryxCapsuleFocusedPreviewView(capsule: capsule) }
```
present 在会话视图层 → dismiss 回会话。

---

## 7. App：聊天卡片哑渲染（doc §5.7）+ 会话级并发（B2）

T1 已把 `row.capsuleCards` 备好。view 只摆卡、不推导。

- `GaryxConversationView.messageScroll`（`:326` 拿 turnRows 处）算：
  ```swift
  let orderedKeys = turnRows.flatMap { t in t.capsuleCards.map { "\(t.id):\($0.capsuleId)" } }
  let activeChatCardKeys = Set(GaryxCapsuleChatCardAdmission.activeKeys(orderedKeys: orderedKeys,
      maxActive: horizontalSizeClass == .regular ? 4 : 2))
  ```
  传入 `GaryxMobileTurnRowsView(rows:, activeCapsuleCardKeys: activeChatCardKeys)`（新增可选参数，
  默认 `[]`；唯一调用方，无多站点）。
- **route-time 删除校验（§8.1 防线 3）**：`GaryxConversationView` 当 `!orderedKeys.isEmpty`（本线程含
  capsule 卡）时，于 appear / `selectedThread.id` 变触发一次 `await model.refreshCapsules()`
  （stale-while-refresh，幂等）→ 经 `capsules` didSet prune 失效已删 capsule 缓存 → 聊天缩略图 re-fetch
  → 404 → "deleted"。无 capsule 卡的线程不触发（省网络，守 [[project_ios_home_list_scroll_jank]]）。
- `GaryxMobileTurnViews.swift` `turnRowContent`（`:38`）在 `ForEach(activityRows)` **之后**追加：
  ```swift
  if !row.capsuleCards.isEmpty {
      GaryxMobileCapsuleChatCardsView(turnId: row.id, cards: row.capsuleCards,
          activeKeys: activeCapsuleCardKeys).transition(.garyxTranscriptAppear)
  }
  ```
- `GaryxMobileCapsuleChatCardsView`（`GaryxMobileCapsuleViews.swift`）：纵向 stack（通常 1 张，
  compact），每张上半 `GaryxCapsulePreviewThumbnail`（`isActive = activeKeys.contains("\(turnId):
  \(card.capsuleId)")`、`cacheEpoch = model.capsuleHTMLCacheEpoch`）、下半 title（空→"Untitled
  Capsule"）+ `GaryxCapsuleChatCardPresentation.subtitle`。tap → `Task { await
  model.openMobileRoute(.capsule(card.capsuleId), source:.conversation) }`。缩略图 `.deleted` →
  disabled "Capsule deleted"。
- mapper `messageRefs` 已不含 capsule cards（T1），不计 visible id，不影响 history 判定/分组。

---

## 8. 客户端 update/delete 取数语义（doc §6，B1 + 缓存失效 B-new）

`loadCapsulePreviewHTML`（`+Capsules.swift`，三处共用）：
```swift
enum GaryxCapsulePreviewHTMLResult: Equatable { case html(String), deleted, failed }
func loadCapsulePreviewHTML(capsuleId, revision, forceRefresh: Bool = false) async -> GaryxCapsulePreviewHTMLResult {
    guard hasGatewaySettings else { return .failed }
    let key = GaryxCapsuleHTMLCacheKey(id: capsuleId, revision: revision)
    if !forceRefresh, let cached = capsuleHTMLCache[key] { return .html(cached) }   // 仅缩略图常态走缓存
    let gen = gatewayRuntimeGeneration
    do { let html = try await client().capsuleHTML(id: capsuleId)
         guard gen == gatewayRuntimeGeneration else { return .failed }
         capsuleHTMLCache[key] = html; return .html(html) }
    catch let e as GaryxGatewayError {
        if case .httpStatus(404, _) = e {                       // 404 = 整 capsule 没了
            // 驱逐**全部** (id,*)（非仅当前 rev）+ bump epoch：否则同 capsule 另一 rev 的缓存缩略图
            // 经 epoch 重跑仍 cache-first 返回陈旧（codex code-review blocker 1）。
            let ev = GaryxCapsuleHTMLCachePruner.evictingCapsule(cache: capsuleHTMLCache, capsuleId: capsuleId)
            capsuleHTMLCache = ev.cache
            if ev.didEvict { capsuleHTMLCacheEpoch &+= 1 }
            return .deleted }
        return .failed }
    catch { return .failed }   // 瞬态/5xx/离线 → 可重试，绝不误标 deleted（doc §6）
}
```
> prune 逻辑抽成 Core 纯函数 `GaryxCapsuleHTMLCachePruner.pruned(cache:validCapsules:) -> (cache,
> didEvict)` 便于 headless 测试 prune-bump；`pruneCapsuleHTMLCache` 调它、`didEvict` 时 bump epoch。
> 404-bump 显式如上（codex r4 非阻断）。测试覆盖：prune 真驱逐→didEvict=true；无变化→false。
- 只写非-@Published `capsuleHTMLCache`，结果回各视图 `@State`（不触发整会话/网格重渲，守
  [[project_ios_home_list_scroll_jank]]）。
- update freshness：server render_state 升 revision（T1 全局最新）→ 卡 `(id,rev)` 键变 → 缩略图/预览
  自然 refetch；gallery refreshCapsules 取新 revision。

### 8.1 删除失效：缓存不得永久隐藏远程删除（codex B-new）

`/serve` 永远是删除权威（404=deleted，parent §6）。内存 `capsuleHTMLCache` 缓存命中会短路 404 →
**已缓存且被远程/他端删除的 capsule，重开聊天卡/预览会显陈旧内容、永不 404**（codex 反例）。根因：
缓存 prune 只挂在画廊专用 `refreshCapsules()`（`+Capsules.swift:23`），而保持 `capsules` 最新的**中心
catalog 刷新**（`+Gateway.swift:533 listCapsules` → `:602 capsules = value`）**不 prune 缓存**。三道防线：

1. **缓存 prune 接到 `capsules` 更新单点 + epoch 失效信号**：`@Published var capsules`（主文件
   `GaryxMobileModel.swift`）加 `didSet { pruneCapsuleHTMLCache(validCapsules: capsules) }`
   （`pruneCapsuleHTMLCache` 现为 `+Capsules.swift` 的 `private`，须改 internal 供跨文件 didSet 调用），
   覆盖**所有**更新路径（中心 catalog `:602`、gallery `refreshCapsules`、`deleteCapsule`、gateway reset）。
   任一 capsules 列表更新即驱逐**已删 capsule**（id 不在新列表）的 `(id,*)` 缓存。
   prune **真驱逐了条目时** `capsuleHTMLCacheEpoch &+= 1`（`@Published var capsuleHTMLCacheEpoch: Int`）：
   ```swift
   func pruneCapsuleHTMLCache(validCapsules: [GaryxCapsuleSummary]) {  // internal
       let validKeys = Set(validCapsules.map(GaryxCapsuleHTMLCacheKey.init))
       let before = capsuleHTMLCache.count
       capsuleHTMLCache = capsuleHTMLCache.filter { validKeys.contains($0.key) }
       if capsuleHTMLCache.count != before { capsuleHTMLCacheEpoch &+= 1 }   // 仅真驱逐才 bump，无 storm
   }
   ```
   epoch 经父（gallery/conversation 读 `model.capsuleHTMLCacheEpoch`）下传进缩略图 `.task(id:)` 身份
   → **已挂载**缩略图重对账（§4.2，修 codex r3 盲点）。`loadCapsulePreviewHTML` 内 404-evict 亦 bump
   epoch（传播给同 id 兄弟卡）。`refreshCapsules`/`deleteCapsule` 原显式 prune/filter 变冗余可删。
   **list 缺席只作缓存失效提示，非删除判定**（删除判定恒是 `/serve` 404 vs 200）→ 刚建但列表暂未含的
   capsule 至多多 fetch 一次（200 重缓存），**绝不误标 deleted**。epoch 仅在真删/真换 revision 时变
   （rare）→ 无重渲风暴；仍存在 capsule 重对账是 cache 命中、无 fetch。
2. **focused 打开 force-refresh**：focused `.task` 走 `loadCapsulePreviewHTML(..., forceRefresh:true)` →
   永远 `/serve` → 即时权威 404（用户实际打开的全屏面永不陈旧）。
3. **会话 route-time 校验**：`GaryxConversationView` `.task(id: "\(threadId):\(hasCapsuleCards)")` —— 用
   便宜的 `model.selectedThreadHasCapsuleCards`（直接扫原始快照 row 的 `capsuleCards`，不做整
   `selectedThreadTurnRows()` 映射）作 token 的一部分。token 在**换线程**与**capsule 卡首次出现**
   （history 晚于 selectedThread 到达）时都变 → 触发 `await model.refreshCapsules()`（幂等
   stale-while-refresh）→ 经 #1 prune+epoch → 聊天缩略图对已删 re-fetch → 404 → "deleted"。修
   codex code-review blocker 2（只 key 在 `selectedThread.id`、rows 未到就 return 之后不补跑的盲点）。
   无 capsule 卡的线程不触发（省网络）。
- 综合：focused 即时权威；缩略图经任一 capsules 刷新（中心 catalog / 画廊 / 会话 route-time）→ prune
  bump epoch → **已挂载**缩略图重对账失效 → 至多陈旧到「下次刷新/重开」（parent §6『重开后更新』可接受）。
  **关键保证**：缩略图即便暂时陈旧也无「陈旧*交互*」——用户**点卡**走 focused force-refresh→404→
  evict+bump epoch→传播回该缩略图显 deleted；故无 forever-stale 交互。DELETE handler 推帧是 gateway，
  越界不做。

---

## 9. 性能与安全（doc §7/§8）

- 性能：画廊 LazyVGrid + onAppear/onDisappear 可见性 + planner ≤N（2/4）；聊天卡会话级最近-N 准入
  ≤N；离屏卸载 webview；`capsuleHTMLCache` 按 `(id,rev)` prune（refresh/delete）。
- 安全：缩略图/focused 同 v1 边界（nonPersistent / 无 WKScriptMessageHandler / baseURL nil / 缩略图
  禁交互 / 外链 open / 未知 cancel）；title/metadata native text 非 HTML 注入；Copy link 用 deep link
  不含 token；meta CSP 仍生效（loadHTMLString 不继承 HTTP header）。

---

## 10. 测试与验收

- **Core（headless 优先）**：§2.7 列表全过。`swift test` 全绿。
- **构建**：`xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator
  -configuration Debug build CODE_SIGNING_ALLOWED=NO` → **真看 BUILD SUCCEEDED**（不被 `tail` exit 0
  骗，[[project_ios_xcodegen_pbxproj_sync]]）。
- **xcodegen**：新 Core/app 文件后 `xcodegen generate` + 提交 `project.pbxproj`。
- 模拟器 boot 自验截图可选；三端 e2e 截图留 T4。

## 11. 文件清单

新增：`Sources/GaryxMobileCore/GaryxCapsulePreviewLoadPlanner.swift`（含
`GaryxCapsuleChatCardAdmission` + `GaryxCapsuleChatCardPresentation`，或拆文件）、
`Tests/GaryxMobileCoreTests/GaryxCapsulePreviewLoadPlannerTests.swift`（+ admission/presentation 测试）。

改：`GaryxGatewayCapsuleModels.swift`（键收 id+rev；删 `GaryxCapsuleHTMLLoadState`）、
`GaryxMobileNavigationState.swift`（`.conversation`）、`GaryxMobileCapsuleViews.swift`
（画廊/缩略图/focused/coordinator/聊天卡视图）、`GaryxMobileTurnViews.swift`（渲染 capsuleCards +
透传 activeKeys）、`GaryxMobileConversationViews.swift`（算 activeKeys + conversation cover）、
`GaryxMobileModel.swift`（删 capsuleHTMLState；加 galleryFocusedCapsule/conversationCapsulePreview/
capsuleHTMLCacheEpoch；`capsules` 加 `didSet { pruneCapsuleHTMLCache(validCapsules: capsules) }` 单点
失效，§8.1 防线 1，prune 真驱逐 bump epoch，自动覆盖中心 catalog `+Gateway.swift:602`）、
`GaryxMobileModel+Capsules.swift`（loadCapsulePreviewHTML[+404 evict]/presentConversationCapsulePreview；
`refreshCapsules`/`deleteCapsule` 原显式 prune 变冗余可删；删
openCapsule/loadSelectedCapsuleHTML/selectedCapsule/isCapsuleHTMLLoaded/clearCapsuleDetailState）、
`GaryxMobileModel+Navigation.swift`（openCapsuleRoute 分流 `:285`→galleryFocusedCapsule +
clearRouteDrivenDetailState `:369` 清两字段）、`GaryxMobileModel+Gateway.swift`（`:132-134` 切网关
reset：`capsuleHTMLState=…` → 清 galleryFocusedCapsule/conversationCapsulePreview，保
`capsuleHTMLCache=[:]`）、删
`Tests/.../GaryxCapsuleHTMLLoadStateTests.swift`（cache-key 用例并入 GaryxGatewayCapsuleModelsTests）、
`GaryxMobile.xcodeproj/project.pbxproj`（xcodegen）。

## 12. 风险

| # | 风险 | 缓解 |
|---|---|---|
| R1 | 删 `GaryxCapsuleHTMLLoadState` 波及面 | 仅被替换的 v1 detail 用；统一 `loadCapsulePreviewHTML` 替代；grep 确认无他处引用；测试并入 |
| R2 | 聊天卡 eager VStack 并发不封顶（codex B2） | 会话级最近-N 确定性准入（纯函数+测试），实例 key 严格 ≤N webview，HTML 按 (id,rev) 共享零重复 fetch |
| R3 | 删后/合成 summary 取不到 /serve（codex B1） | focused/缩略图统一按 id 直取 + 404 分类于 `loadCapsulePreviewHTML`，不依赖 `capsules` 查找 |
| R3a | 缓存永久隐藏远程删除（codex B-new） | §8.1 三防线：`capsules` didSet 单点 prune（覆盖中心 catalog 刷新）+ focused force-refresh 即时权威 404 + 会话 route-time refreshCapsules（仅含卡线程）；/serve 恒为删除权威，list 缺席仅作缓存失效提示不误删 |
| R4 | present-over-conversation 时序/返回 | preview 提 model `@Published` + 会话层 `fullScreenCover(item:)`，dismiss 回会话；不切 panel；deep-link/gallery 走 panel 分支 |
| R5 | 越 T1/契约线 | 只读 `row.capsuleCards` 摆卡，不扫 tool result、不本地查 capsules 决定插卡、不动 reducer/gateway |
| R6 | pbxproj 未提交→假绿 | xcodegen + 提交 pbxproj + 必跑 xcodebuild 看 SUCCEEDED |
