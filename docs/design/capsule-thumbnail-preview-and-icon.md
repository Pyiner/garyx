# Capsule 画廊缩略图 + Icon：诊断与方案设计（#TASK-1455）

状态：**走查 + 方案设计（不实现）**。本文回答 A1/A2/A3/A4 + icon 重设计，给出
推荐方案，等拍板后再实现。

参照截图：iOS **Capsules 画廊页**，三张卡。本仓 `~/.garyx/capsules` 里正好有三个
真实 capsule，与截图一一对应（下文统一用 cap1/2/3 指代）：

| 卡位 | DB 标题 | 文件 | 字节 | 现象 |
| --- | --- | --- | --- | --- |
| 左下 cap1 | Capsule 功能讲解 | `cap-1.html` | 14362 | **正常撑满** |
| 右上 cap2 | 聊天卡片·iOS 验收 | `cap-2.html` | 1252 | **整片空白 + 灰色实心椭圆** |
| 左上 cap3 | 移动端消息状态修复 | `cap-3.html` | 7102 | **内容偏上、两边/下方白边没撑满** |

> 说明：cap1/2/3 为本机 dogfood 的真实 capsule（`~/.garyx/capsules/` 下），本文用合成文件名指代以保
> 公共仓库卫生；复现时按字节数/结构特征即可对上。

三个 capsule 的结构差异是理解全部问题的钥匙：

- cap1：完整 `<!doctype html>`，深色**不透明** body，首屏 `.slide{min-height:100vh}`。
- cap2：完整文档，深色**不透明** body，`html,body{height:100%}` + `display:flex;
  justify-content:center`（内容**垂直居中**）。
- cap3：**HTML 片段**——以 `<div class="wrap">` 开头，**无 `<!doctype>`/`<html>`/
  `<body>`、无 viewport meta**；`.wrap{max-width:780px;margin:0 auto}`，**body 背景透明**。

---

## A3 — 缩略图三端现状实现（"是不是每次 HTML 实时渲染？"）

**结论先行：是。三端缩略图都是每次用 WebView/iframe 实时渲染整份 capsule HTML，
没有任何一端、任何一层做过图片预渲染。** 缓存的是 **HTML 文本**（按 `id:revision`），
不是图片。

### 数据流（gateway → 端）

```
agent 调 MCP capsule_create/update(html)         （HTML 由 agent 生成，gateway 零生成）
        │
        ▼
gateway: atomic_write_capsule_file → ~/.garyx/capsules/{uuid}.html   （文件系统存 HTML 文本）
         garyx_db.create_capsule → CapsuleRecord{ id,title,revision,html_sha256,byte_size,… }
                                                  （DB 只存元数据 + sha256，HTML 不入库）
        │  端侧请求
        ▼
GET /api/capsules/{id}/serve → serve_capsule()    （读文件 → validate → inject_csp_meta → 返回整份 HTML）
        │  Content-Security-Policy 头 + <meta> 双注入；NO viewport meta；Cache-Control: no-store
        ▼
端侧把 HTML 喂进 WebView/iframe 实时渲染
```

关键源码：
- `garyx-gateway/src/garyx_db/mod.rs:360` `CapsuleRecord`：`id,title,description,thread_id,
  run_id,agent_id,provider_type,html_sha256,byte_size,revision,created_at,updated_at`。
  **没有任何缩略图/预览图字段**，`html` 也不在 DB。
- `garyx-gateway/src/capsules.rs:153` `capsule_file_path` → `~/.garyx/capsules/{uuid}.html`；
  `:21` `CAPSULE_MAX_HTML_BYTES = 5MB`；`:350` `serve_capsule`；`:303` `inject_csp_meta`
  （只注 CSP，**不注 viewport**）；`:23` `CAPSULE_CSP`（`style-src 'unsafe-inline'` 故内联
  样式可渲染）。
- `garyx-gateway/src/mcp/tools/capsule.rs:66` `create_inner`：HTML 完全由调用方提供，
  gateway 只做大小/UTF-8/资源引用合法性校验（拒 `file://`、相对路径），**不生成、不包裹、
  不规整**。
- 列表 `capsule_list`/`GET /api/capsules` 只回 `serve_path: /api/capsules/{id}/serve`，**无
  预览图 URL**。

### iOS（截图所在端）

- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileCapsuleViews.swift`
  - `GaryxCapsulePreviewThumbnail`（:212）：状态机 `idle / loading / loaded(html) /
    deleted / failed`。**只有 `isActive==true`（被 planner 准入）时才挂 WKWebView**；否则走
    `default` 分支显示 `capsule.fill` SF Symbol 占位（:269）。
  - `GaryxCapsuleThumbnailWebView`（:306）：虚拟画布 `virtualWidth = 760`（:308）；
    `scale = cardWidth/760`，`virtualHeight = cardHeight/scale`；WKWebView frame = `760 ×
    virtualHeight`，`.scaleEffect(scale, anchor: .topLeading)`（:316，**左上角锚点**）+
    `.clipped()`（:318，裁掉溢出）。
  - `GaryxCapsuleThumbnailWebRepresentable`（:324）：`loadHTMLString(html, baseURL: nil)`
    （:351），`nonPersistent`、禁滚动/交互、`isOpaque=false; backgroundColor=.clear`
    （**透明背景**），无 `didFinish/didFail`/超时回调。**不注入 viewport**（与详情页相反，
    见下）。
  - 卡片容器（`GaryxCapsuleGalleryCard` :136）：缩略图 `.aspectRatio(16/10, .fit)`（:147），
    满铺无圆角无边框，圆角/描边在卡壳上。
- 取数 / 缓存：`GaryxMobileModel+Capsules.swift:51` `loadCapsulePreviewHTML(id,revision,
  forceRefresh)` → `GET /serve`，缓存键 `(id,revision)`；404→`evictingCapsule`（驱逐该 id 全
  revision）+ `capsuleHTMLCacheEpoch &+= 1`（进 `.task(id:)` 身份触发重对账）。
- **并发准入**：`GaryxCapsulePreviewLoadCoordinator(maxActive: 2)`（:12），`maxActivePreviews
  = horizontalSizeClass == .regular ? 4 : 2`（:15）；纯函数 `GaryxCapsulePreviewLoadPlanner`
  （Core）`activeIds = visibleOrder.prefix(maxActive)`，`isActive(id) = index < maxActive`。
- 详情页（**另一条路径**，#TASK-1453）：`GaryxCapsuleWebView`（:520）注入
  `GaryxCapsuleViewport.ensuringMobileViewport`（device-width + 禁缩放）。**缩略图故意不注、
  走桌面宽缩放**——A2 要处理的就是这条缩略图路径。

### Desktop（Electron/React）

- `desktop/garyx-desktop/src/renderer/src/app-shell/components/CapsuleLivePreviewFrame.tsx`：
  `<iframe sandbox="allow-scripts" srcDoc={html}>`，**固定虚拟画布 `1024×640`（16:10）**
  （:10），`transform: scale(width/1024); transform-origin: top left`（styles.css:17820），
  卡 `aspect-ratio:16/10; overflow:hidden`（:17806），背景 `#fff`。无 `onload/onerror`/超时。
- 共享 HTML store `capsule-html-store.ts`：模块级单例，键 `id:revision`，`MAX_CONCURRENT = 4`
  （**只限并发 fetch，不限 iframe 挂载数**），`useSyncExternalStore` 按键订阅，404→`deleted`。
- `useInViewport`（IntersectionObserver）门控 `active`：**可见的卡都会挂 iframe**（fetch 被
  ≤4 节流，但渲染不被卡死）。占位是无图标的灰 skeleton（:94），无椭圆。
- 导航 icon 用 Lucide `Package`（icons.tsx:158）——与 iOS `capsule.fill` **不一致**。

### 一句话现状

> 三端都是「拉 HTML → 实时 WebView/iframe 渲染」。**iOS 把"准入渲染数"硬限到 2/4 张，
> desktop 只限并发 fetch（渲染所有可见卡）。** 没有任何预渲染图片；缓存的是 HTML 文本。

---

## A1 — 部分缩略图加载不出来（右上"整片空白 + 灰色实心椭圆"）

### 根因：iOS preview 准入上限 `maxActive=2` < 同屏可见卡数 → 第 3 张永远停在占位

**那个"灰色实心椭圆"不是渲染失败，是 `capsule.fill` 的 idle 占位。**

- iPhone(compact) 画廊是 **2 列网格**（`GridItem(.flexible()) ×2`），3 个 capsule 占 2 行，
  启动即**全部同屏可见**（无需滚动）。
- 3 张卡的 `onAppear` 几乎同时 `markVisible`，planner `activeIds = visibleOrder.prefix(2)`
  只准入**前 2 张**；第 3 张 `isActive=false` → `GaryxCapsulePreviewThumbnail.reconcile()`
  里 `guard isActive else { return }` 早返回 → phase 停在 `.idle` → `content` 的 `default`
  分支渲染 `Image(systemName: "capsule.fill").foregroundStyle(.tertiary)`（灰色实心胶囊/椭圆）
  叠在 `Color.primary.opacity(0.045)` 浅灰底上。
- 因为 3 张都没滚出屏，`markHidden` 永不触发 → 第 3 张**永远拿不到准入槽** → 永久占位。
  planner 的 doc 注释自己写明：*"the rest render a skeleton until an earlier card scrolls
  off and frees a slot"*——但**漏了"同屏可见卡数 > maxActive 时根本没有卡会滚出"**这一情形。

**为什么排除"HTML 渲染失败"**：
1. 占位是 `capsule.fill`（idle/loading/!isActive 都走 `default` 分支）。`.failed` 会显示
   `exclamationmark.triangle` + "Preview unavailable" 文案，`.deleted` 显示 `trash` + "Capsule
   deleted" 文案。老板看到的是**纯椭圆、无文案** → 必是 idle（未准入），不是 failed/deleted。
2. **确定性渲染复现**（见下）证明 cap2 在给定视口后渲染完美（深色底 + 居中渐变标题），它
   "空白"纯粹是没被挂载。

哪张卡成为"受害者"取决于 `onAppear` 竞态（截图里是 cap2/右上），但**结构性 bug 恒成立：
3 张同屏 + maxActive=2 → 必有一张永久占位**。iPad(regular) maxActive=4，3 张能全渲（所以这
是 iPhone 特有）。

### Desktop 对照

Desktop **没有这个永久饿死 bug**：`useInViewport` 给每张可见卡都挂 iframe，`MAX_CONCURRENT=4`
只节流 HTML fetch（短暂 skeleton 级联后都会渲出）。Desktop 的问题集中在 A2 + 实时渲染的性能
天花板，不在 A1。

### 确定性复现（headless，无 UI）

**复现一（A1 主根因，纯函数 Core 测试）**——planner 是纯值类型，可单测：

```swift
// 现有 GaryxCapsulePreviewLoadPlanner 行为，可加进
// mobile/garyx-mobile/Tests/GaryxMobileCoreTests/
func testGalleryStarvesThirdVisibleCardOnCompact() {
    var p = GaryxCapsulePreviewLoadPlanner(maxActive: 2)   // iPhone compact
    p.markVisible("cap3"); p.markVisible("cap1"); p.markVisible("cap2")  // 三张同屏
    XCTAssertTrue(p.isActive("cap3"))
    XCTAssertTrue(p.isActive("cap1"))
    XCTAssertFalse(p.isActive("cap2"))      // ← 第三张永远不准入 → 永久 capsule.fill 占位
    // 关键：三张都没 markHidden（没滚出屏），第三张永远拿不到槽
}
```

`isActive("cap2") == false` 直接等价于"右上那张永远只显示灰椭圆占位"。这是**确定性、可重放、
无需模拟器**的复现。

**复现二（佐证 cap2 渲染本身没问题）**——用 Chrome（Chromium/Blink，playwright `channel:'chrome'`）渲染
三个真实 capsule 到 iOS 缩略图等效视口 `760×475`：脚本
`scratchpad/capsule-repro.mjs`，截图见 artifact。cap2 在 760×475 下**渲染完美**（深色底 +
居中渐变标题完整可见）→ 反证它的"空白"不是渲染失败，是未准入。

---

## A2 — 渲染出来的缩略图没撑满、两边/下方白边

### 根因：把"作者自定义的整份文档 HTML"硬塞进固定 16:10 卡 + 左上角裁切，capsule 根本没按 16:10 卡面排版

缩略图把任意 HTML 渲染进 `760×475`（iOS）/`1024×640`（desktop）画布，左上锚点缩放 + 裁切。
而 capsule 是作者按"可滚动整页/居中海报/桌面宽"写的，不是按 16:10 封面写的。三类典型失配：

1. **垂直居中型**（cap2：`height:100%` + `justify-content:center`）：内容是一条**居中带**，
   不是从顶部铺。渲染测量：`.k` 起点 y=117/475，内容带约 y=118–357。给了正确视口能看到，
   但在更窄/更扁的裁切里这条带容易偏离"从顶铺满"的预期。
2. **片段 / 居中窄容器 + 透明 body**（cap3）：`.wrap{max-width:780px;margin:0 auto}` +
   **body 背景透明 `rgba(0,0,0,0)`**。
3. **桌面满铺型**（cap1：不透明深底 + `100vh` 首屏 + 内容顶对齐）：唯一**正常撑满**的。

### 决定性证据（真实 capsule 渲染测量，`scratchpad/capsule-repro.mjs`）

| capsule | 视口 | body 背景 | 主内容块 | 结果 |
| --- | --- | --- | --- | --- |
| cap1 | 760×475 | `rgb(10,14,22)` 不透明 | `.slide` top=0,h=541 | **满铺**（深底 + 内容顶对齐，6026px 高顶裁） |
| cap2 | 760×475 | `rgb(10,14,22)` 不透明 | `.k` 内容带居中 y≈117–357 | 渲染完美（居中带）；空白是 A1 占位非渲染 |
| cap3 | 760×475 | **透明** | `.wrap` left=0,**w=760** | `.wrap` 撑满 760 宽（max-width 780 未触发）→ **无侧 gutter** |
| cap3 | **980×612** | **透明** | `.wrap` **left=100,w=780** | **左右各 100px 透明 gutter** → 透出卡面浅底 = "两边白边没撑满" |

**关键发现**：cap3 在 760 宽布局时 `.wrap` 撑满 760、无 gutter；在 980 宽布局时 `.wrap` 780 居中、
左右 100px 透明 gutter。而 cap3 **没有自己的 viewport meta**，iOS WKWebView 缩略图又**不注入
device-width viewport**（详情页才注，#TASK-1453）→ WKWebView 按 ~980 桌面宽回退布局 → 命中
"980 居中 gutter" 这一档。**body 透明** → gutter 透出卡片 `Color.primary.opacity(0.045)` 浅底
= 老板说的"两边还有边框、没左右撑满"。

> 即：**A2 的"两边白边"= 无 viewport 的 capsule 走 980 桌面宽回退 + 居中窄容器 + 透明 body。**
> 截图证据：cap3@980 看得到两侧浅色竖条；cap3@760 这两条消失。

垂直方向的"下方白边"同源：透明 body + 内容（居中带 / 短内容）在 16:10 裁切里没盖满 → 没盖到的
区域透出卡面浅底。

### 撑满策略（要定清楚的裁切/缩放规则）

固定卡比例 **16:10**（三端已统一）。对"任意作者 HTML"，**没有任何缩放能保证它'填满'一个 16:10
封面**——这正是 A4 要解的根本矛盾。可分两层处理：

**第一层（立即可做，降伤，仍是实时渲染）**：
1. **缩略图也注入 device-width viewport**（复用 `GaryxCapsuleViewport`，把视口宽钉到画布宽
   760，不走 980 回退）→ 直接消除 cap3 这类"侧边 gutter"（证据：cap3@760 无 gutter）。
   - 取舍：这会让"为桌面宽设计"的 capsule 在缩略图里以窄宽重排，可能与详情观感不同。但缩略图
     本就是"示意"，窄宽重排通常比"两侧大白边"更可接受；且与详情页 viewport 策略一致。
2. **缩略图画布给不透明中性底**（深色，贴合多数 capsule 深底；不要 `.clear`）→ 透明 body 的
   capsule 不再闪出浅色卡底；裁切露白处也是中性深色而非刺眼浅灰。
3. 维持左上锚点 + 顶裁（`object-position: top`），让"从顶部铺"的内容（cap1/cap3）撑满；居中型
   （cap2）仍是居中带，但配不透明底后观感可接受。

**第二层（根治，配合 A4）**：固定 16:10、**渲染一次后截图成封面图**（cover），按 `object-fit:
cover` + 顶锚点裁切定死，画廊只显这张图。这样"撑满 + 一致"由截图时一次性定死，不再受作者 HTML
垂直/水平排版摆布。详见 A4。

> 注：第一层只是"少留白"，**无法让任意 capsule 真正撑满**——根治依赖第二层的封面图。

---

## A4 — 性能 + 架构方案评估（核心决策）

### A4.1 现状到底卡不卡？——性能问题不是"量大时假设"，它已经是 A1 的根因

每张实时缩略图 = 1 个 WebView（iOS WKWebView 各起一个 WebContent 进程，内存/CPU 重）/ 1 个
iframe（desktop，含布局 + 脚本）。**iOS 把准入数硬限到 2/4 正是因为更多并发 WebView 会卡/吃内存
——这个上限直接制造了 A1 的饿死。** 换句话说：

> 现状的性能天花板**不是"100 个会卡"的远虑，而是"3 个就已经崩"的近忧（A1 就是它的症状）**。
> 把 `maxActive` 调高能救 A1，但等于把"省下来的卡顿"还回去：滚动大画廊时不停 mount/unmount
> WebView，正是当初设上限的原因。A1 与 A4 是同一枚硬币的两面。

desktop iframe 比 WKWebView 轻，但大画廊里几十个实时 iframe（布局 + 脚本）仍是真实开销。

### A4.2 老板提议：gateway 后端预渲染静态图 —— 评估

gateway 是 **Rust 跨平台二进制**（含 Linux release）。"把 HTML 渲成图"在 Rust 里只有这些路：

| 方案 | 能渲真实 HTML+CSS+JS？ | 代价 / 阻塞 |
| --- | --- | --- |
| headless Chromium（chromiumoxide/CDP，或 shell 系统 Chrome） | ✅ | 浏览器 ~150–300MB；**绝不能嵌进 release 二进制**（撞体积门）；要么强依赖用户机装 Chrome（按 `GARYX_*` 解析、缺则报错），对每台机是重依赖 |
| 系统 WKWebView（macOS）| ✅ | **macOS-only**，破 Linux gateway 跨平台；要 Swift/ObjC 旁挂或 wkhtmltoimage |
| `resvg`/`usvg`（纯 Rust）| ❌ 只渲 SVG | capsule 是带 JS 的完整 HTML，渲不了 |
| 纯 Rust HTML 渲染器 | ❌ | 不存在能吃真实 CSS/JS 的成熟库 |

**结论：gateway 侧预渲染是错的层。** 它要么嵌浏览器撞体积门，要么强依赖外部浏览器/破跨平台。收益（一张静态图）可以在
客户端用**已经存在的浏览器引擎**几乎零成本拿到。

### A4.3 折中：客户端"渲染一次 → 截图缓存图"（**推荐**）

端侧本就各有渲染引擎，渲完截一张图、按 `(id, revision, rendition)` 缓存，之后只显图：

- **iOS**：首次（或 revision 变）挂 WKWebView 渲染 → `WKWebView.takeSnapshot(with:)` → UIImage →
  写盘缓存（caches/App Group）。画廊默认显缓存图（普通 image grid，零 WebView）；仅 cache-miss 时
  才临时挂一个 WebView 渲染→截图→落盘。
- **Desktop**：Electron **自带 Chromium**，离屏渲染 / `webContents.capturePage()` 截 PNG 缓存；
  画廊显缓存图。截图成本几乎为零（引擎已在）。
- **缓存键含 rendition，别用裸 `id:revision`**：同一 capsule 在不同画面比例不同 —— 画廊卡 `16:10`
  （`GaryxMobileCapsuleViews.swift:147`）、iOS 聊天卡 `16:9`（:627）。若截图跨画面复用，键必须带
  rendition（比例 + 目标像素，如 `id:revision:16x10@2x`），否则会把 16:10 的图错显到 16:9 的卡。
- 失效：`revision` 变 → 键失效 → 重渲重截；删除 404 → 连同图一起驱逐（沿用现有 epoch/evict）。

**为什么它同时解 A1 + A2 + A4：**
- A1：缓存图无并发渲染成本 → **不再需要 `maxActive` 上限** → 所有可见卡瞬时显示，永不饿死。
- A2：截图时一次性定死 16:10 + 顶锚点 cover 裁切 + 不透明底 → 撑满与一致由截图固化，不再受
   作者 HTML 排版摆布。
- A4：稳态零实时 WebView（50 张 capsule = 50 张图，像任意图片网格）→ 不卡。仅"新建/更新"那一张
   渲一次。

**代价**：首次仍要实时渲一次（一次性）；多一层图片缓存盘占用；截图分辨率要选好（@2x/@3x）。
相比 gateway 嵌浏览器，这点代价极小，且**不碰 release 体积门、不破跨平台**。

**与老板"后端预渲染"的关系**：收益（画廊显静态图、不实时渲 HTML、比例固定）**完全一致**，但把
渲染放在**已有引擎的客户端**而非要给 Rust gateway 塞浏览器。唯一 gateway 更优的场景是"封面图需
跨设备完全一致 + 可分享"，且要求每台 gateway 机必有浏览器——当前都不成立，故不推荐。

### A4.4 方案对比与推荐

| 方案 | 解 A1 | 解 A2 撑满 | 解 A4 大量卡顿 | 代价/风险 | 跨平台/体积门 |
| --- | --- | --- | --- | --- | --- |
| **现状**（实时渲 + maxActive 上限）| ❌（上限制造饿死）| ❌ | ❌（3 张就触顶）| — | OK |
| 立即热修（提 maxActive + 注 viewport + 不透明底）| ✅ | ◐（少留白，非真撑满）| ❌（把卡顿换回来）| 小，**可先发** | OK |
| gateway 预渲染静态图 | ✅ | ✅ | ✅ | **大**：嵌/依赖浏览器、撞体积门、破 Linux | **撞门/破跨平台** |
| **客户端渲一次→截图缓存（推荐）** | ✅ | ✅ | ✅ | 中小：首渲一次 + 图缓存 | **OK** |
| 干脆不显 HTML 预览（只显 icon+标题+元信息）| ✅ | n/a | ✅ | 丢掉可视预览（老板要的是视觉画廊）| OK |

**推荐：**
1. **立即热修（独立小改，可先发解决 A1/A2 观感）**：
   - A1：iOS 准入策略改为"**所有当前可见卡都准入渲染**"（或把 `maxActive` 提到 ≥ 同屏最大可见
     卡数，并按真实视口可见性而非 FIFO 准入），消除永久占位。
   - A2：缩略图路径也注入 device-width viewport（消 980 gutter，已证）+ 缩略图画布改不透明
     中性深底（消透明 body 透出浅卡底）。
2. **根治（推荐主方向）**：上 **客户端"渲一次→截图缓存图"**，固定 16:10 + 顶锚点 cover 裁切。
   稳态零实时 WebView，A1/A2/A4 一并解决；先 iOS（截图所在端、痛点最重），desktop 随后（Chromium
   自带、几乎免费）。
3. **不做** gateway 侧 HTML→图预渲染（错的层）。

> 注意：热修与根治可分两步走——热修先发，让画廊"立刻全渲、少留白"；根治再上截图缓存，拿到
> "比例固定 + 不卡"。也可直接上根治、跳过热修（截图缓存天然包含热修的收益）。由老板定节奏。

---

## B — Capsule Icon 重设计

### 现状（先查实"灰椭圆"用在哪）

老板说的"实心椭圆/像药片" = **`capsule.fill` SF Symbol**（一个**水平实心胶囊/药片**形状），iOS
一处定义、**三处复用**：

- `GaryxMobileNavigationState.swift:85` `case .capsules: "capsule.fill"` → 导航栏 tab icon、
  画廊空状态 icon、**缩略图 idle/loading 占位**（A1 右上那个椭圆就是它）。
- desktop 用 Lucide **`Package`**（icons.tsx:158）→ 与 iOS **不一致**（一个药片、一个包裹）。

所以重设计要**一套替换全部**：导航 icon + 空状态 + iOS 缩略图占位 + 对齐 desktop。方向：表达
"**装着宝贝 / 魔力的胶囊容器**"，**明确不要水平药片观感**——做成**竖直容器/小瓶/舱**里装一点
"宝物/光"。

### 三个 SVG 方案（24×24，`currentColor`，自包含，可复用）

设计原则：① **竖直**朝向（站立的容器/小瓶，避开水平药片）；② 内含一个"宝物/魔力"符号（火花 /
宝石 / 光球）；③ 单色 `currentColor` 描边可当 SF-Symbol 式字形（导航/占位），可叠 accent 渐变
当空状态/hero。下面是字形本体；渲染预览见配套 artifact。

**方案 A — Spark Capsule（竖直容器 + 火花）**：竖直胶囊舱，正中一枚四角星火花。最克制、
16px 也清晰、最"胶囊本格"（靠竖向 + 火花去药片化）。

```svg
<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"
     stroke-linejoin="round" stroke-linecap="round">
  <rect x="7.5" y="2.5" width="9" height="19" rx="4.5"/>
  <path d="M12 9c.34 1.7 1 2.36 2.7 2.7-1.7.34-2.36 1-2.7 2.7-.34-1.7-1-2.36-2.7-2.7 1.7-.34 2.36-1 2.7-2.7Z"
        fill="currentColor" stroke="none"/>
</svg>
```

**方案 B — Gem Vial（小瓶 + 宝石）**：带瓶颈瓶塞的小瓶，瓶身里一颗切面宝石。"瓶中藏宝"，
最有"宝贝"叙事。

```svg
<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"
     stroke-linejoin="round" stroke-linecap="round">
  <path d="M9 3h6"/>
  <path d="M10 3v3.2c0 .5-.2 1-.6 1.4C8 9 7.5 10.4 7.5 12v5.5A3 3 0 0 0 10.5 20.5h3A3 3 0 0 0 16.5 17.5V12c0-1.6-.5-3-1.9-4.4-.4-.4-.6-.9-.6-1.4V3"/>
  <path d="M12 11l2 2-2 2-2-2 2-2Z" fill="currentColor" stroke="none"/>
</svg>
```

**方案 C — Glow Pod（光舱 + 光球 + 微光）**：竖直圆舱，中缝开合线，内有一颗实心光球 + 一点
微光，"装着一团魔力光"。最柔和、最"魔力"（小尺寸下元素略多）。

```svg
<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"
     stroke-linejoin="round" stroke-linecap="round">
  <rect x="6.5" y="3" width="11" height="18" rx="5.5"/>
  <path d="M6.5 11c2 1.1 9 1.1 11 0" stroke-opacity="0.35"/>
  <circle cx="11.3" cy="14.4" r="2.3" fill="currentColor" stroke="none"/>
  <path d="M15.1 7.6c.22.95.6 1.33 1.55 1.55-.95.22-1.33.6-1.55 1.55-.22-.95-.6-1.33-1.55-1.55.95-.22 1.33-.6 1.55-1.55Z"
        fill="currentColor" stroke="none"/>
</svg>
```

推荐顺序：**B（Gem Vial）** 领衔——"瓶中藏宝"最贴"装着宝贝的容器"、最不像药片、记忆点最强；
**A（Spark Capsule）** 作极简/最稳备选（最小尺寸最清晰、保留 capsule 母题）；**C（Glow Pod）**
为偏活泼的"魔力光"选项。三个都给两种用法：**单色描边**（导航 / 占位，跟随 `.tint`/`currentColor`）；**accent 渐变填充**
（空状态 / hero，用 capsule 既有 `--a1/--a2/--a3` 青→靛→粉渐变）。建议**三端统一选同一个**，
顺手把 desktop 的 Lucide `Package` 换成同款 SVG（消除 iOS/desktop 不一致）。

> 占位场景额外建议：缩略图 idle/loading 占位现在直接用导航 icon。可换成"新 icon + 极淡呼吸/
> 微光"以区分"加载中"与"概念"；但若采纳 A4 截图缓存，**占位出现的频率会大幅下降**（稳态显图），
> 占位主要只在首渲那一刻闪现。

---

## 复现资产与验证

- `scratchpad/capsule-repro.mjs`：playwright + Chrome channel，渲染三个真实 capsule 到
  `760×475`（iOS 缩略图等效视口）与 `980×612`（cap3 无 viewport 桌面回退）；输出测量 JSON +
  截图。布局行为（垂直居中 / `100vh` / 片段顶流 / 16:10 顶裁 / 980 居中 gutter）属 CSS 规范，
  WebKit/WKWebView 与 Chromium 一致；唯一引擎差异（无 viewport 默认布局宽）已通过显式测 760 与
  980 两档覆盖，不依赖引擎自动回退。
- 截图：`repro-cap1…fills`（满铺）、`repro-cap2…blank`（渲染完美 → 反证非渲染失败）、
  `repro-cap3…760`（无 gutter）、`repro-cap3…980fallback`（两侧 100px gutter）。
- A1 纯函数复现见上（planner `isActive` 断言）。

## 不变量 / 红线（本方案遵守）

- 不破 `render_state` 哑渲染红线：capsule 卡的存在/分组仍由 server `render_state.capsule_cards`
  驱动；缩略图渲染/截图缓存是**纯展示层**，不改 transcript 结构。
- 不在 gateway 引入 HTML→图渲染运行时（守 release 体积门 + 跨平台，与"不嵌 Bun"同红线）。
- Mac 不是缩略图正确性的真相源；本文以"客观语义 + 真实 capsule 渲染证据"为基准，两端对照。
- 公共仓库：本文与配套 artifact 不含真实个人数据；icon 为自包含 SVG。
