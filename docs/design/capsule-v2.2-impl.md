# Capsule v2.2 实现级设计：聊天卡片侧栏预览 + 去标题数字 badge

#TASK-1442。v2/v2.1 已上线（`docs/design/capsule-v2.md`、`capsule-v2.1-impl.md`，T1/T2/T3 +
v2.1 已合 main）。本稿是 v2.2 两个 UI 收尾的**实现级**设计，只改 **desktop**；iOS 经核
查为 **no-op**（见 §4）。不动 gateway/router/bridge/`garyx-models` 契约、`render_state`、
capsule API、preview 安全配置（sandbox/CSP/IPC 取 HTML）。

## 0. 读码结论（带 file:line）

### 桌面聊天卡片现状（route 跳整页）
- 聊天卡 `CapsuleChatCard`（`CapsuleChatCard.tsx:9-42`）：`onClick={() => onOpenCapsule?.(card.capsule_id)}`
  （`:24`），prop `onOpenCapsule?: (capsuleId: string) => void`（`:11-14`）。
  `CapsuleChatCardList`（`:49-66`）同签名透传。
- `RenderCapsuleCard` 契约（`shared/contracts.ts:1463-1469`）：`{ id, capsule_id, title, revision, action }`。
- `ThreadPage` 接 `onOpenCapsule?: (capsuleId: string) => void`（`ThreadPage.tsx:383`），**三处** `CapsuleChatCardList`
  均直接透传（`:754-758`、`:945-949`、`:961-965`），prop 不在 ThreadPage 内做任何处理。
- AppShell 两处接线，**当前都是 route 跳整页预览**：
  - **主对话** ThreadPage（`AppShell.tsx:9774-9777`）：`setContentView("capsules"); setCapsulePreviewId(capsuleId);`
  - **side-chat 内嵌** ThreadPage（`AppShell.tsx:9487-9490`）：同上。side-chat 这个 ThreadPage 渲染在
    `sideToolsPanel` 里（右侧栏内），见下。
- `setContentView("capsules") + setCapsulePreviewId(id)` → `currentDesktopRoute` 串成 `{kind:'capsule'}`
  → hash `#/capsules/<id>`（`desktop-route.ts:277-281`、`:227-228`），即**离开对话进整页预览**。

### 桌面对话已有的 split/detail 模式（要参考/复用的真相源）
- 对话主区是一个 grid `<main className={conversationClassName} style={conversationStyle}>`（`AppShell.tsx:10316-10319`），
  内含 header + `conversation-body`（`:10403-10683`）+（条件）右侧栏。
- 右侧栏当前唯一占位者 = **inspector / side-tools panel**：`showConversationSideTools`
  （`AppShell.tsx:9571-9573`）= `inspectorOpen && contentView==="thread" && sideToolsPanel`，
  其中 `sideToolsPanel`（`:9533-9569`）**要求 `activeWorkspacePath` 非空**（无 workspace 时为 null）。
  渲染在 `</section>` 之后：resizer + `{sideToolsPanel}`（`:10684-10703`）。
- 布局靠 CSS 类：`conversationClassName` 在 `showConversationSideTools` 时加 `with-side-tools`
  （`:9581`），`conversationStyle` 设 `--side-tools-panel-width`（`:9586-9590`）。
- CSS `.conversation.with-side-tools`（`styles.css:2634-2661`）是 **3 列 grid**：
  `minmax(0,1fr)`（主）/ `10px`（resizer）/ `minmax(520px, var(--side-tools-panel-width,720px))`（面板）；
  header/body 钉 col1、resizer 钉 col2、`.thread-side-tools-panel` 钉 col3。
- side-tools 的**可拖拽 resize** 机制深度耦合 workspace：宽度状态 `sideToolsPanelWidth`
  + refs（`:1627/1793/1798`）、pointer/键盘 resize handler（`:2329/2346`）、ResizeObserver 钳制
  effect（`:4388/4495`），**全部 gate 在 `inspectorOpen && contentView==="thread" && activeWorkspacePath`**。
  capsule 卡片可出现在**任何 thread（含无 workspace）**，故 capsule 面板**不能直接复用 side-tools 那套**。
- 右侧栏入场动画 keyframe `side-tools-panel-enter`（`styles.css:4169-4178`，从右滑入 + 淡入，170ms）可复用。
- inspector / thread-logs 互斥已有先例：`onToggleInspector` 关 logs（`:10388-10393`）、
  `onToggleThreadLogs` 关 inspector（`:10394-10399`）。

### 整页预览组件（复用预览帧，不复用整页壳）
- `CapsuleLivePreviewFrame`（`CapsuleLivePreviewFrame.tsx:54-138`）：唯一 capsule 渲染器，
  `mode="preview"` 时 `active` 恒 true、不受 viewport 门控、走 IPC 取 HTML + opaque-origin
  `sandbox="allow-scripts"` iframe。**直接复用**。
- 整页壳 `CapsulePreviewPage`（`CapsulesPanel.tsx:154-267`）顶栏含 Back/Refresh/CopyLink/⋯
  (含「打开来源对话」/CopyID/Delete)，是为**整页 route** 定制；侧栏面板要**极简顶栏**，故
  另写小组件复用既有 CSS 类（`.capsule-preview-toolbar`/`.capsule-preview-title`/
  `.capsule-toolbar-button`/`.capsule-preview-body`，`styles.css:17800-17864`），不复用整页壳。

### 标题数字 badge
- 桌面 `CapsulesPanel`（`CapsulesPanel.tsx:452-461`）标题行 `capsules-page-title-row`：
  `<h1>{t('Capsules')}</h1>` + `<span className="tasks-status-chip tone-progress">{capsules.length}</span>`（`:455-456`）。
  **要去掉的就是这个 `<span>` 计数 chip。**

### i18n / 测试
- i18n 现有 `'Close': '关闭'`（`i18n/index.tsx:90`）、`'Open source conversation'`（`:118`）、
  `'Untitled Capsule'`（`:121`）。本稿**无需新增 i18n key**（面板 aria-label 用 capsule 标题、关闭用 `t('Close')`）。
- `test:unit`（`package.json:25`）跑固定 `.test.mjs` 列表。`grep` 确认**无任何测试**断言
  `tasks-status-chip` / `onOpenCapsule` / `capsules.length` / `CapsuleChatCard`。
- `desktop-route.test.mjs` 测的是 `#/capsules/<id>` route（`:123-167`）——本稿**不动 desktop-route.ts**
  （画廊/deep link 整页 route 保留），故该测试不受影响。
- `build:ui` = `tsc --noEmit && electron-vite build`（`:16`），会**跨文件类型检查** `onOpenCapsule`
  签名变更（CapsuleChatCard / ThreadPage / AppShell 三处必须一致）。

## 1. 范围与非目标
- 改（仅 desktop）：① 去掉 Capsules 标题数字 badge；② **聊天**卡片点击 → 当前对话视图**右侧 split
  panel** 显示 capsule 预览（不 route 跳转、对话留在左主区、可关闭回对话）。
- **不改**：画廊卡片点击（仍整页/detail 预览，`#/capsules/<id>` route + deep link 保留）；
  gateway/router/bridge/`garyx-models` 契约与 `render_state`；capsule API；preview/thumbnail
  安全配置；side-tools/inspector 既有行为；desktop-route.ts。
- **iOS 不改**（§4 论证）：无 capsule 数字 badge 可去；聊天卡 present-over 保持不动；窄屏不做侧栏。
- 不引入新顶层概念；不耦合 transcript 结构与 route。

## 2. 需求 1：去掉桌面标题数字 badge
`CapsulesPanel.tsx:455-457`，删掉计数 `<span>` 一行：
```diff
   <div className="capsules-page-title-row">
     <h1 className="capsules-page-title">{t('Capsules')}</h1>
-    <span className="tasks-status-chip tone-progress">{capsules.length}</span>
   </div>
```
保留 `capsules-page-title-row` 容器（其 flex/gap 对单 h1 无副作用），最小改动。`capsules` 变量
其它用途（gallery 渲染、空态判断）不动。

## 3. 需求 2：聊天卡片 → 右侧 split panel

### 3.1 数据流 / 签名变更（预览帧需 capsuleId + revision + title）
`CapsuleLivePreviewFrame` 需 `capsuleId`/`revision`/`title`；聊天卡 `RenderCapsuleCard` 三者都有。
把卡片点击回调从「传 capsuleId 字符串」改为「传整张 `RenderCapsuleCard`」，让 AppShell 直接拿到
revision/title 渲染面板（不必再从 render_state 反查，最直接）。

- `CapsuleChatCard.tsx`：
  - `onOpenCapsule?: (card: RenderCapsuleCard) => void`（`:11-14` 与 `:49-54` 两处签名）。
  - `onClick={() => onOpenCapsule?.(card)}`（`:24`）。
- `ThreadPage.tsx`：prop 类型 `onOpenCapsule?: (card: RenderCapsuleCard) => void`（`:383`）；
  需在既有 `@shared/contracts` 类型 import 块（`:52` 区）补 `type RenderCapsuleCard`。
  三处 `CapsuleChatCardList` 用法**不变**（透传）。

revision 用**卡片自身的 `card.revision`**（与该卡缩略图一致——点哪张看哪张那一版），不另发请求取
最新，保持「点击即所见」且零额外网络。

### 3.2 新组件 `CapsuleConversationPanel.tsx`（极简顶栏 + 复用预览帧）
新文件 `app-shell/components/CapsuleConversationPanel.tsx`（desktop 新文件**不涉及 pbxproj**）：
```tsx
import { X } from 'lucide-react';
import { useI18n } from '../../i18n';
import { CapsuleLivePreviewFrame } from './CapsuleLivePreviewFrame';

export function CapsuleConversationPanel({
  capsuleId, revision, title, onClose,
}: { capsuleId: string; revision: number; title: string; onClose: () => void }) {
  const { t } = useI18n();
  return (
    <aside aria-label={title} className="capsule-conversation-panel">
      <header className="capsule-preview-toolbar">
        <span className="capsule-preview-title">{title}</span>
        <button
          aria-label={t('Close')} className="capsule-toolbar-button"
          onClick={onClose} title={t('Close')} type="button"
        >
          <X size={16} />
        </button>
      </header>
      <div className="capsule-preview-body">
        <CapsuleLivePreviewFrame active capsuleId={capsuleId} mode="preview" revision={revision} title={title} />
      </div>
    </aside>
  );
}
```
- 顶栏 = 标题（左，`flex:1` 撑开）+ 关闭 X（右）。**不放「打开来源对话」**：聊天卡的来源对话就是
  当前对话（你已经在里面），该项在内联面板里是冗余 no-op；整页预览（gallery/deep link）保留它仍正确
  （那里你不在对话内）。这是与整页壳的刻意差异。
- title 兜底：AppShell 侧用 `card.title?.trim() || t('Untitled Capsule')` 解析后传入（镜像聊天卡 `CapsuleChatCard.tsx:19`）。

### 3.3 AppShell 状态 + 接线 + 互斥
- 新状态：`const [capsulePanelCard, setCapsulePanelCard] = useState<{ card: RenderCapsuleCard; threadId: string } | null>(null);`
  （存整张卡 + **来源 threadId**；revision/title 在 card 内）。`RenderCapsuleCard` 从 `@shared/contracts` import。
  > **存来源 threadId 的理由**（采纳 codex 设计审建议，消除「切线程一帧残留」）：仅靠 effect 清理是在
  > paint **之后**跑，切线程那一帧会短暂显示旧线程的 capsule 面板。把来源 threadId 存进 state，并让派生
  > 门控要求 `capsulePanelCard.threadId === selectedThreadId`（见下），切线程当帧即不匹配→面板**同步隐藏、零残帧**；
  > effect 仅负责事后清 state（避免切回原线程重现）。比 `useLayoutEffect` 更稳、门控自解释。
- **主对话**卡片点击（`AppShell.tsx:9774-9777`）改为开内联面板，并保证右侧栏单占位（`selectedThreadId`
  在主对话 ThreadPage 渲染时必非空——capsule 卡只现于已落盘 thread）：
  ```tsx
  onOpenCapsule={(card) => {
    if (!selectedThreadId) return;
    setCapsulePanelCard({ card, threadId: selectedThreadId });
    setInspectorOpen(false);
    setThreadLogsOpen(false);
  }}
  ```
- **side-chat 内嵌** ThreadPage（`AppShell.tsx:9487-9490`）**保留整页 route**（仅适配新签名）：
  ```tsx
  onOpenCapsule={(card) => {
    setContentView("capsules");
    setCapsulePreviewId(card.capsule_id);
  }}
  ```
  理由：side-chat 这个 ThreadPage **本身就渲染在右侧栏内**（`sideToolsPanel`），在右侧栏里再开一个
  右侧 split 不成立；保持其既有整页预览行为（零回归），内联 split 只服务**主对话**卡片。
- 互斥与渲染（`AppShell.tsx:9571-9590` 派生 + `:10684-10703` 渲染）。派生门控带 **threadId 匹配**（零残帧）：
  ```tsx
  const showConversationCapsulePanel = Boolean(
    capsulePanelCard &&
      capsulePanelCard.threadId === selectedThreadId &&   // 切线程当帧即不匹配 → 同步隐藏
      contentView === "thread",
  );
  const showConversationSideTools = Boolean(
    inspectorOpen && contentView === "thread" && sideToolsPanel,
  ) && !showConversationCapsulePanel;            // capsule 面板优先占右栏
  ```
  `conversationClassName` 增 `with-capsule-panel`（与 `with-side-tools` 互斥，因二者派生互斥）：
  ```tsx
  showConversationSideTools ? "with-side-tools" : null,
  showConversationCapsulePanel ? "with-capsule-panel" : null,
  ```
  右栏渲染处改为二选一（capsule 面板用固定宽、**无 resizer**）：
  ```tsx
  </section>
  {showConversationCapsulePanel && capsulePanelCard ? (
    <CapsuleConversationPanel
      capsuleId={capsulePanelCard.card.capsule_id}
      revision={capsulePanelCard.card.revision}
      title={capsulePanelCard.card.title?.trim() || t('Untitled Capsule')}
      onClose={() => setCapsulePanelCard(null)}
    />
  ) : showConversationSideTools ? (
    <>
      <div className="side-tools-resizer" ... />   {/* 原样不动 */}
      {sideToolsPanel}
    </>
  ) : null}
  ```
- inspector / logs 反向互斥：`onToggleInspector`（`:10388-10393`）与 `onToggleThreadLogs`
  （`:10394-10399`）各加 `setCapsulePanelCard(null)`，保证打开 inspector/logs 时关掉 capsule 面板
  （右栏永远单占位）。
- 生命周期清理（事后释放 state，避免切回原线程重现）：
  ```tsx
  useEffect(() => { setCapsulePanelCard(null); }, [selectedThreadId, contentView]);
  ```
  **残帧已由上面的 threadId 派生门控消除**（切线程当帧 `threadId !== selectedThreadId` → 同步不渲染）；
  此 effect 只在 paint 后清 state。开面板的点击 handler 不改 `selectedThreadId`/`contentView`，故不会在
  开面板那一帧被误清（已逐项核对：handler 只动 `capsulePanelCard`/`inspectorOpen`/`threadLogsOpen`）。
  点另一张卡 → 覆盖 state → 面板换 capsule；点 X → 置 null → 回纯对话。

### 3.4 CSS（新增专用布局类 + 面板壳）
不复用 `with-side-tools`（其 resize 机制 workspace-耦合、min 520 偏宽、带 resizer 列）；新增专用类，
**复用** capsule 预览既有顶栏/正文样式与入场动画：
```css
/* 对话右侧 capsule 预览面板：固定宽、可关、不可拖拽 resize（与 with-side-tools 互斥占右栏）。 */
.conversation.with-capsule-panel {
  grid-template-columns: minmax(0, 1fr) clamp(360px, 40%, 540px);
  grid-template-rows: auto minmax(0, 1fr);
}
.conversation.with-capsule-panel .conversation-header { grid-column: 1; grid-row: 1; }
.conversation.with-capsule-panel .conversation-body   { grid-column: 1; grid-row: 2; }
.conversation.with-capsule-panel .capsule-conversation-panel { grid-column: 2; grid-row: 1 / span 2; }

.capsule-conversation-panel {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: #fff;
  border-left: 0.5px solid var(--color-token-border);
  animation: side-tools-panel-enter 170ms var(--ease-out-expo) both;  /* 复用右滑入场 */
}
@media (prefers-reduced-motion: reduce) {
  .capsule-conversation-panel { animation: none; }
}
```
- 顶栏/正文复用 `.capsule-preview-toolbar` / `.capsule-preview-title` / `.capsule-toolbar-button`
  / `.capsule-preview-body`（已存在，`styles.css:17800-17864`），视觉与整页预览一致。
- 宽度 `clamp(360px, 40%, 540px)`：随窗口自适应、合理；主区 `minmax(0,1fr)` 保留。桌面窗口常态够宽；
  若极窄另议（side-tools 在 `styles.css:17315-17320/17506` 有窄屏 override，本面板按需后续可补，非本稿必需）。

### 3.5 行为总览
- 画廊卡 / deep link：**不变**，整页 `#/capsules/<id>` detail 预览（顶栏含「打开来源对话」）。
- 主对话聊天卡：点 → 右侧内联 split 出预览，对话留左主区；点 X / 切线程 / 切视图 → 关闭回对话。
  打开 inspector 或 thread-logs 会替换/关闭它（右栏单占位）。
- side-chat 内聊天卡：保持整页 route（内嵌侧栏内无法再开右侧 split）。

## 4. iOS：no-op 论证（带证据）
任务列「iOS（去 badge）」，但**经核查 iOS 不存在 capsule 数字 badge，且聊天卡 present-over 明确不动**，
故 iOS **零代码改动**，仅做无回归验证：
- `GaryxCapsulesView` 头部（`GaryxMobileCapsuleViews.swift:26-37`）= `leadingButton +
  GaryxPanelHeaderTitle("Capsules") + Spacer`，**无任何计数 chip/badge**。
- `GaryxPanelHeaderTitle`（`GaryxMobileComponents.swift:349-371`）只渲染 `Text(title)`（无 count 入参/无 badge）。
- `grep` 全 app：`capsules.count`/`capsuleCount`/`.badge(` 在 capsule 相关视图**零命中**。
- 任务明确「iOS 聊天卡保持 present-over（不动）；iOS 窄屏不做侧栏」。
→ iOS 验证项 = `swift test` 绿 + `xcodebuild BUILD SUCCEEDED` + 模拟器确认「标题无数字 badge（本就无）、
  聊天卡仍 present-over」。无新增文件 → 无 xcodegen/pbxproj。

## 5. 文件清单
desktop：
- `src/renderer/src/app-shell/components/CapsulesPanel.tsx`（删标题计数 span）。
- `src/renderer/src/app-shell/components/CapsuleChatCard.tsx`（`onOpenCapsule` 签名→传 card + onClick）。
- `src/renderer/src/app-shell/components/ThreadPage.tsx`（prop 类型→card；import `RenderCapsuleCard`）。
- `src/renderer/src/app-shell/components/CapsuleConversationPanel.tsx`（**新文件**）。
- `src/renderer/src/app-shell/AppShell.tsx`（新 state + 主/side-chat 两 handler + 互斥派生 +
  className + 右栏渲染二选一 + inspector/logs handler 加清理 + 切换清理 effect）。
- `src/renderer/src/styles.css`（`.conversation.with-capsule-panel` + `.capsule-conversation-panel`）。

iOS：**无**。

## 6. 验证
desktop（完成标准）：
- `cd desktop/garyx-desktop && npm run build:ui && npm run test:unit` 全绿（含 `tsc --noEmit` 类型检查）。
- packaged-app CDP 实测：`npm run dist:dir` → 退出残留 `Garyx` → 开安装版 → `playwright-cli attach
  --cdp=http://127.0.0.1:39222`，逐项断言：
  ① Capsules 画廊标题旁**无数字 badge**；
  ② 进一个**有 capsule 聊天卡**的对话，点聊天卡 → 右侧 split 出 capsule 预览、**对话仍在左主区**
     （contentView 仍 thread、hash 不变为 `#/capsules/...`）、点关闭回纯对话；
  ③ 画廊卡点击仍整页/detail 预览（`#/capsules/<id>`、contentView=capsules）——不变。

iOS（完成标准）：
- `cd mobile/garyx-mobile && swift test` 全绿；`xcodebuild ... build` 真 **BUILD SUCCEEDED**（不被 `| tail` exit 0 蒙）。
- 模拟器：标题无数字 badge（本就无）、聊天卡仍 present-over。

公共仓库：staged diff 扫真实个人数据；本稿无 fixture 改动。

## 7. 风险与取舍（请 reviewer 重点核）
1. **新增 `with-capsule-panel` 而非复用 `with-side-tools`**：side-tools 的 resize 机制深度耦合
   `activeWorkspacePath`（无 workspace 不可用），capsule 卡可现于任何 thread；面板做成固定宽（无 resizer）
   是更小且正确的面，避免改动一堆 workspace-gated 的 resize effect。代价是多一个布局类。备选「泛化右栏
   为单一可拖拽机制」改面大、风险高，本稿不取。flag 给 reviewer。
2. **聊天卡签名 `capsuleId:string` → `card:RenderCapsuleCard`**：因预览帧需 revision+title。透传链只改
   三处类型 + 一处 onClick + 两处 AppShell handler；`tsc --noEmit` 守一致性。
3. **revision 用 card 自身版本**（非取最新）：与该卡缩略图一致、零额外请求；若 capsule 已更新，面板显示点击那一版
   （与整页预览取最新略有差异，但「点哪张看哪张」更符合内联预览语义）。flag 给 reviewer。
4. **面板不含「打开来源对话」**：聊天卡来源即当前对话，内联面板里冗余；整页预览仍保留该项。
5. **右栏单占位互斥**：开 capsule 关 inspector/logs，开 inspector/logs 关 capsule；切线程/视图清 capsule。
   均已逐行核对触发点，无「开面板那帧被清理 effect 误清」竞态。
6. **side-chat 聊天卡保持整页 route**：内嵌侧栏无法再开右侧 split，保留零回归行为。flag 给 reviewer（可否接受）。
7. **iOS no-op**：基于「无 badge + present-over 不动」的核查结论；若 reviewer 认为任务隐含别处 iOS badge，请指出具体位置。

## 8. 一句话
桌面去掉 Capsules 标题计数 chip；主对话聊天卡点击改为在当前对话右侧开一个固定宽、可关闭的内联
split 面板（新增 `with-capsule-panel` 布局 + 复用 `CapsuleLivePreviewFrame` preview 与既有预览顶栏样式），
对话不离开、不 route 跳转；画廊卡 / deep link 整页预览与 side-chat 行为保持不变；iOS 经核查为 no-op。
