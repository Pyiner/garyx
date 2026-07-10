# Capsule v2.1 实现级设计：iOS 卡牌对齐 Mac + 卡片「打开来源对话」

#TASK-1437。v2 已上线（`docs/design/capsule-v2.md`，T1/T2/T3 已合 main）。本稿是
**实现级**收尾设计，只改 **desktop + iOS 两端 UI**，不动 gateway/router/bridge/models 契约。

## 0. 读码结论（带 file:line）

- Mac 画廊卡真相源 `desktop/.../renderer/src/styles.css:17687-17761`：
  - `.capsule-gallery-card`：`border-radius:12px`、`border:0.5px hairline`、`padding:0`、
    `overflow:hidden`、flex column、subtle 次级底色。
  - `.capsule-card-preview-shell`：满宽、`aspect-ratio:16/10`、`overflow:hidden`、
    底部 `border-bottom:0.5px hairline`（预览满铺、与下半文字之间一条细线）。
  - `.capsule-card-meta`：`display:grid; gap:3px; padding:10px 12px 12px`。
  - `.capsule-card-title`：`font-size:13.5px; font-weight:600`、单行省略、primary 色。
  - `.capsule-card-subline`：**单行文字**、`font-size:11px`(`--text-xs-plus`)、
    description-foreground（次级）色、单行省略。内容 `CapsulesPanel.tsx:115`
    = `${formatRelativeTime(updatedAt)} · ${creator}`（如 `9m · Test Agent`）；
    revision/byteSize 仅进 `title=` tooltip（`:116`），**不**进副信息。
  - creator 取法 `describeCreator`（`CapsulesPanel.tsx:84-99`）：
    `agents[agentId].displayName` → `agentId` → `providerType`(原串) → `"Agent"`。
- iOS 画廊卡现状 `mobile/.../App/GaryxMobile/GaryxMobileCapsuleViews.swift:129-179`：
  - 卡圆角 18（`:165-168`），内缩 `.padding(8)`，缩略图独立圆角 14 + 自带描边（`:139-148`）。
  - 副信息是**两个 pill**（`:155-160`）：`GaryxCapsuleMetadataChip("clock", time)` +
    `GaryxCapsuleOwnerBadge`（provider）。两个 chip struct 定义在 `:650-693`，
    **仅此处使用**（`grep` 确认无其它引用）。
  - `GaryxCapsuleOwnerBadge.ownerPresentation`（`:681-692`）走
    `GaryxProviderPresentation.make(agentId:providerType:fallbackName:)`，该函数
    **provider 非空时优先 provider**（`GaryxMobileIdentityPresentation.swift:48-69`），
    所以 iOS 显示「Claude Code」，而 Mac 同一卡显示 agent 名「Test Agent」→ **副信息文字不一致的根因**。
- iOS 焦点预览 `GaryxCapsuleFocusedPreviewView`（`:346-473`）：`fullScreenCover` 的内容视图，
  **被画廊和聊天卡共用**（gallery `:52-54` 绑 `galleryFocusedCapsule`；conversation
  `GaryxMobileConversationViews.swift:296-297` 绑 `conversationCapsulePreview`）。
  其 `Menu`（`:384-396`）现有 Copy Link / Copy ID / Delete。
- iOS 路由：`openMobileRoute(_:source:)`（`GaryxMobileModel+Navigation.swift:201-241`）开头调用
  `clearRouteDrivenDetailState()`（`:370-379`），**会把 `galleryFocusedCapsule` 与
  `conversationCapsulePreview` 置 nil**，再 `case .thread → openThread(id:)`（`:209-210`）。
  即一次 `openMobileRoute(.thread(id))` 既关闭预览 cover、又打开正常对话页。
- 契约都已带来源 thread id：`DesktopCapsuleSummary.threadId?`（`contracts.ts:352`）、
  `GaryxCapsuleSummary.threadId: String?`（`GaryxGatewayCapsuleModels.swift:24`，解码兼容
  `thread_id`/`threadId`）。`/api/capsules` summary 已返回。
- desktop 打开 thread 的现成入口：`openExistingThread(threadId)`（`AppShell.tsx:4035-4044`，
  内部 `setContentView("thread")`），`TasksPanel` 已用同一模式（`AppShell.tsx:10594-10598`）。

## 1. 范围与非目标

- 改：iOS 画廊卡视觉对齐 Mac（去 pill → 单行文字副信息）；两端预览页 ⋯ 菜单新增
  「打开来源对话」跳来源 thread 的**正常对话页面**（非预览）。
- **不改**：gateway/router/bridge/`garyx-models` 契约与 wire；`render_state`；
  capsule 列表 API；preview/thumbnail 安全配置（sandbox/CSP/`baseURL:nil`/非交互）；
  画廊卡 + 聊天卡的**主点击仍进预览**。
- **iOS 聊天卡（in-transcript）不在 #1 范围**：它本就无 pill（`:577-585` 标题 +
  `Created/Updated` 副标题），#1 仅针对**画廊卡**。
- 不引入 workspace/thread 的新顶层概念；不耦合 transcript 结构与 route。

## 2. 需求 1：iOS 画廊卡对齐 Mac

只改 `GaryxCapsuleGalleryCard`（`GaryxMobileCapsuleViews.swift:129-179`）+ Core 加纯
presentation + SwiftPM 测试。逐项对齐表（Mac 为真相源）：

| 项 | Mac 真相 | iOS 现状 | iOS 目标 |
|---|---|---|---|
| 卡圆角 | 12px | 18 | **12**（`background`/`overlay` 两处 `cornerRadius` 同改） |
| 卡描边 | 0.5px hairline | `GaryxTheme.hairline` 1px @18 | 保留 hairline，@12 |
| 卡底色 | subtle 次级 | `Color.primary.opacity(0.03)` | 保留（原生 subtle） |
| 卡内边距 | 0（预览满铺） | `.padding(8)` 包整卡 | **0**；改由 meta 自带 padding |
| 预览 | 满宽 16/10 + 底部 hairline 分隔 | 内缩圆角 14 缩略图 | **满铺顶部**（顶角随卡 clip 圆 12）+ 下方 0.5px hairline 分隔 |
| meta 容器 | grid gap 3 / padding 10·12·12 | VStack spacing 5 | VStack spacing **3** / padding top10 horiz12 bottom12 |
| 标题 | 13.5px w600 单行省略 primary | subheadline semibold 单行 primary | **保持** subheadline semibold 单行 primary |
| 副信息 | 单行文字 `time · creator` 11px 次级单行省略 | 两个 pill | **单行 `Text("time · creator")`** caption 次级 `lineLimit(1)` `.truncationMode(.tail)` |

要点与理由：

- **标题字号**不做像素级强对齐：iOS 用语义 Dynamic Type（`subheadline`，符合
  `mobile-ui.md` 原生要求），且与聊天卡标题（`:578` 同为 `subheadline semibold`）保持一致。
  对齐是「设计 token 级」（semibold 单行 primary 标题 / 更小次级单行副信息），非像素同值。
  这是有意取舍，请 reviewer 确认可接受。
- **满铺 vs 内缩**：选**满铺**（与 Mac 一致：预览顶部满铺、底部一条 hairline 分隔、下半
  padding 文字）。实现：外层 `VStack(spacing:0)` + `.clipShape(RoundedRectangle(12))`，
  缩略图传 `cornerRadius:0` 且**不画自身描边**，其下加一条
  `Rectangle().fill(GaryxTheme.hairline).frame(height:0.5)` 分隔线，再接 meta。
  - 为此给 `GaryxCapsulePreviewThumbnail` 增 `showsBorder: Bool = true`（默认 true，
    向后兼容聊天卡 `:568-574` 与画廊原行为），画廊传 `showsBorder:false`，
    gate 掉缩略图自身的 `.overlay { RoundedRectangle.stroke }`（`:220-223`）。其余
    （`.task` reconcile、内容渲染、安全配置）**不动**。
- **副信息 creator 优先级**（修同卡两端不一致根因），按此顺序逐档命中即返回：
  1. `agents.first{$0.id==agentId}?.displayName`（trim 非空）
  2. `agentId`（agentId 非空但 catalog 未命中）
  3. `providerType` → `GaryxProviderPresentation.displayName(for:)` **美化**（如 `claude_code`→「Claude Code」）
  4. `"Agent"`（全空兜底）
  - 与 Mac 唯一刻意差异：provider 兜底用美化名而非 Mac 原串（「claude_code」）——iOS 既有
    展示惯例、更原生，且该兜底是历史/无 agent 的边角分支（正常 capsule 有 agentId → 走 agent 名）。
    该美化行为由 §2.1 测试 pin 死。flag 给 reviewer。
- **副信息时间**：`capsule.formattedUpdatedAt`（`:701-703`，`garyxFormattedTaskTimestamp`）。
  有值 → `"\(time) · \(creator)"`；无值 → 仅 `creator`（镜像现状 `:156` 的空判，避免空 `·`）。
- **删死代码**：移除 `GaryxCapsuleMetadataChip`、`GaryxCapsuleOwnerBadge`（去 pill 后无引用）。
  保持「干净结构、不留补丁/死代码」。

### 2.1 Core 纯 presentation + 测试

在 `mobile/.../Sources/GaryxMobileCore/GaryxCapsulePreviewLoadPlanner.swift`（已含
`GaryxCapsuleChatCardPresentation`）追加同风格 enum（**不新建文件**，避免文件 churn；
Core 是 SwiftPM，无 pbxproj 影响）：

```swift
public enum GaryxCapsuleGalleryCardPresentation {
    /// Creator precedence: agent name → agentId → prettified provider → "Agent".
    public static func creatorName(
        agentId: String?,
        providerType: String?,
        agents: [GaryxAgentSummary],
    ) -> String { ... }

    /// "time · creator"，time 为空时仅 creator（不留悬挂的 "·"）。
    public static func subline(timeDisplay: String?, creator: String) -> String { ... }
}
```

`GaryxAgentSummary` 已是 Core 类型（`GaryxGatewayAgentModels.swift`）。
View 端只调这两个静态函数拼字符串，保持「presentation/business-rule 在 Core + 哑渲染」。

新增测试 `Tests/GaryxMobileCoreTests/GaryxCapsulePreviewLoadPlannerTests.swift`（已存在文件，
追加用例；SwiftPM 自动纳入，无 pbxproj）：
- `creatorName` 逐档断言：agent 命中优先；agent miss 用 agentId；无 agentId → **pin 美化 provider**（断言 `providerType:"claude_code"`
  → `"Claude Code"`，钉死美化兜底契约）；全空得 `"Agent"`。
- `subline`：有 time → `"9m · Test Agent"`；time 为空/`nil` → `"Test Agent"`。
- fixture 用合成占位（`Test Agent` / `1000000001` / 合成 UUID），无真实个人数据。

## 3. 需求 2：预览页 ⋯ 菜单「打开来源对话」

### 3.1 Desktop

- `components/CapsulesPanel.tsx`：
  - `CapsulesPanelProps` 增 `onOpenThread: (threadId: string) => void`。
  - `CapsulePreviewPage` 增参 `sourceThreadId: string | null` + `onOpenSourceThread: () => void`。
    在 overflow `FloatingActionMenuContent`（`:211-224`）**首项**加：
    ```tsx
    {sourceThreadId ? (
      <FloatingActionMenuItem onSelect={onOpenSourceThread}>
        <MessageSquare aria-hidden />
        {t('Open source conversation')}
      </FloatingActionMenuItem>
    ) : null}
    ```
    （`threadId` 缺失则**不显示**——正常 capsule 都有，缺失仅理论历史数据；保持菜单干净。
    `MessageSquare` 从 `lucide-react` 引入。）
  - Panel 主体：`sourceThreadId={previewCapsule?.threadId ?? null}`、
    `onOpenSourceThread={() => { if (previewCapsule?.threadId) onOpenThread(previewCapsule.threadId); }}`。
- `AppShell.tsx`（`:10576-10588` CapsulesPanel 渲染处）：加
  `onOpenThread={(threadId) => { trackUiAction("capsules.open_thread", async () => { await openExistingThread(threadId); }); }}`
  （**完全复用 TasksPanel 先例** `:10594-10598`；`openExistingThread` 切到 thread 视图并选中线程，
  URL hash 由既有 `replaceDesktopRoute` 副作用同步为 `#/thread/<id>`）。
- `i18n/index.tsx`：`zhCN` 加 `'Open source conversation': '打开来源对话'`（`:104` 区，英文 key 即英文文案）。

行为：画廊卡/聊天卡主点击仍进预览；预览页 ⋯ →「打开来源对话」→ 跳 `#/thread/<source>` 正常对话页。

### 3.2 iOS

- `GaryxCapsuleFocusedPreviewView` 的 `Menu`（`:384-396`）**首项**加（threadId 存在才显示）：
  ```swift
  if let threadId = capsule.threadId?.trimmingCharacters(in: .whitespacesAndNewlines),
     !threadId.isEmpty {
      Button {
          Task { await model.openMobileRoute(.thread(threadId)) }
      } label: {
          Label("Open source conversation", systemImage: "bubble.left.and.bubble.right")
      }
  }
  ```
- 走 public `openMobileRoute(.thread(id))`（符合 mobile-ui「openThread 统一路径」）。该调用
  `clearRouteDrivenDetailState()` 会清 `galleryFocusedCapsule`/`conversationCapsulePreview`
  → fullScreenCover 关闭；随后 `openThread(id:)` 打开正常对话页。**无需手动 `dismiss()`**，
  避免「先 dismiss 再异步 open」竞态。
- 因 `GaryxCapsuleFocusedPreviewView` 为画廊与聊天卡共用 → **一处改动同时覆盖两个入口**。
- 无新增 app 文件（仅编辑既有），故本需求不触发 xcodegen。

## 4. 文件清单

desktop：
- `src/renderer/src/app-shell/components/CapsulesPanel.tsx`（菜单项 + props）
- `src/renderer/src/app-shell/AppShell.tsx`（接 `onOpenThread`）
- `src/renderer/src/i18n/index.tsx`（zhCN 一条）

iOS：
- `App/GaryxMobile/GaryxMobileCapsuleViews.swift`（画廊卡重排 + 去 pill + 删两 chip struct +
  缩略图 `showsBorder` + 焦点预览菜单项）
- `Sources/GaryxMobileCore/GaryxCapsulePreviewLoadPlanner.swift`（加 `GaryxCapsuleGalleryCardPresentation`）
- `Tests/GaryxMobileCoreTests/GaryxCapsulePreviewLoadPlannerTests.swift`（加用例）

预计**无新增 app-target 文件 → 无需 xcodegen/pbxproj**。若实现中确需新 app 文件，则
`xcodegen generate` 并提交 `GaryxMobile.xcodeproj/project.pbxproj`。

## 5. 验证

desktop（完成标准）：
- `cd desktop/garyx-desktop && npm run build:ui && npm run test:unit` 全绿。
- packaged-app CDP 实测：`npm run dist:dir` → 退出残留 `Garyx` → 开安装版 → `playwright-cli attach
  --cdp=http://127.0.0.1:39222` → 进 Capsules 画廊 → 开一个 capsule 预览 → ⋯ →
  「打开来源对话」→ 断言落到来源 thread 的**正常对话页**（hash `#/thread/<source>`、contentView=thread）。

iOS（完成标准）：
- `cd mobile/garyx-mobile && swift test` 全绿（含新 Core 用例）。
- `xcodebuild ... build` 看到真 **BUILD SUCCEEDED**（不被 `| tail` 的 exit 0 蒙）。
- 模拟器实测（garyx-app-screenshots）：画廊卡**无 pill、单行 `time · creator` 文字**；
  截 iOS 画廊 + Mac 画廊**并排**确认卡牌一致；预览 ⋯ →「打开来源对话」→ 跳正常对话页面。

公共仓库：staged diff 扫真实个人数据，fixture 用合成占位。

## 6. 风险与取舍（请 reviewer 重点核）

1. **满铺 vs 内缩缩略图**：选满铺以真正贴近 Mac。代价是给共享缩略图加 `showsBorder` 参数 +
   一条分隔线。备选「保留内缩、仅缩圆角+去 pill」更小改动但与 Mac 不完全一致；本稿选满铺。
2. **标题字号非像素对齐**：iOS 用语义字体（原生 + 与聊天卡一致），设计 token 级对齐。
3. **provider 兜底美化**：iOS 末位 provider 兜底用美化名而非 Mac 原串（边角分支，正常走 agent 名两端一致）。
4. **menu 项缺 threadId 隐藏**（非 disabled）：正常 capsule 必有 threadId；隐藏保持菜单干净。
   任务允许「disabled 或不显示」。
5. iOS「打开来源对话」依赖 `clearRouteDrivenDetailState()` 清两个预览绑定的既有行为
   （`GaryxMobileModel+Navigation.swift:370-379`）——已逐行核对成立。
## 7. 一句话

iOS 画廊卡去 pill、按 Mac CSS 逐项对齐成满铺预览 + 单行 `time · creator` 文字副信息（creator 按
Mac 优先级，presentation 进 Core + 测试）；两端预览页 ⋯ 菜单加「打开来源对话」走各自正常 openThread
路径跳来源 thread 对话页，画廊/聊天卡主点击仍进预览。只碰 desktop + iOS、无契约改动。
