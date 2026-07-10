# Capsule v2 — T2 Desktop UI 实现级设计

> 实现阶段产物。父设计稿 `docs/design/capsule-v2.md`（§3 Desktop / §5.6 Desktop dumb-render
> / §6 update·delete / §7 性能 / §8 安全）是契约真相源，本文只把 §3/§5.6 落成可评审的
> file-by-file 改动。**范围严格限定 `desktop/garyx-desktop/`**：不碰 gateway / bridge /
> garyx-models / iOS（避免与 T3 冲突）。T1（commit `65ea139e`）已合 main：
> `contracts.ts` 已有 `RenderCapsuleCard`/`RenderCapsuleAction`/`RenderUserTurnRow.capsule_cards?`，
> `render-view-model.ts` 已对新 union 形态做防御处理（但**尚未透传** `capsule_cards`，那是本任务的活）。

---

## 0. 读码结论（带 file:line，作为改动基线）

- 现有 `CapsulesPanel.tsx`（438 行）是 master-detail：左 `.capsules-list`（:331）右 `.capsules-detail`（:361），
  detail 显示 title/description/revision/byteSize/timestamp/id + Copy ID/Refresh/Delete + 单个
  `<iframe sandbox="allow-scripts" srcDoc>`（:418）。HTML cache key 现为 **3 段** `id:revision:sha`（:26-28）。
- HTML 取数链路已就绪：renderer `window.garyxDesktop.getCapsuleHtml(id)` → preload（`preload/index.ts:183`）→
  IPC `garyx:get-capsule-html`（`main/index.ts:942`）→ `gary-client.ts:4889 getCapsuleHtml`（带 auth 的
  `requestText('/api/capsules/{id}/serve')`，404 抛 `GatewayRequestError{status:404}`，`gary-client.ts:1611/1663-1693`）。
  `listCapsules`/`getCapsule`/`deleteCapsule` 同款已存在（`main/index.ts:932/937/947`，preload `:181-184`）。
- 路由：`desktop-route.ts` 的 `DesktopRoute` union（:5-17）**无** capsule 变体；`#/capsules` 现走
  `SIMPLE_VIEW_SEGMENTS`（:19-28，`capsules→view:'capsules'`），无视第二段。`parseDesktopRoute`(:106)/
  `contentViewForDesktopRoute`(:173)/`buildDesktopRouteHash`(:190)。
- AppShell 路由编织：`applyDesktopRoute`（:4122）按 `route.kind` 切 view；`currentDesktopRoute`（:1324）
  从 state 反推 route，由 `replaceDesktopRoute` effect（:4247）写 hash；hashchange/popstate 回灌
  `applyDesktopRoute(parseDesktopRoute())`（:4235）。`CapsulesPanel` 在 :10581 渲染（lazy，:330），
  rail `onOpenCapsules`（:10159）只 `setContentView('capsules')`。deep-link：`subscribeDeepLinks`（:4466）+
  `deepLinkEventHandlerRef`（:6835）switch `event.type`。
- deep-link 主进程：`deep-link.ts:67 parseDesktopDeepLink` 按 `action`(hostname) 分 thread/new/resume；
  `main/index.ts:555 queueDeepLink` 生成 trace label + dispatch。`DesktopDeepLinkEvent`（`contracts.ts:1555`）
  是 4 变体 union（open-thread/new-thread/resume-session/error）。
- 聊天渲染：`render-view-model.ts:272 buildThreadViewRows` 把 `render_state.rows` 翻成
  `UserTurnRow{userBlock, activityRows}`（:73-78），文件头（:11-20）锁「纯结构翻译、无分组/配对」契约；
  orphan turn（user 解析不到）走 `rows.push(...activityRows)`（:299）。`ThreadPage.tsx:928-937` 渲染 user_turn
  分支（userBlock + `activityRows.map(renderActivityRow)`）。`ThreadPageProps`（:245-380）已有导航回调先例
  `onOpenThreadById`（:378）。`buildThreadViewRowsWithLocalUsers`（:304）追加乐观本地 user 行（无 capsule）。
- 测试基线：`test:unit`（`package.json:25`）是**显式文件列表**（非 glob）→ 新增测试文件必须登记。
  已含 `desktop-route.test.mjs`、`render-view-model.test.mjs`、`gary-client.test.mjs`；
  `render-view-model.test.mjs:472` 的 “capsule_cards are tolerated but not rendered” 是 **T1 现态断言**，
  T2 要改成“透传但不计入 visible/blocks”。`getCapsuleHtml`/`listCapsules` **未**被任何 mjs 测试/electron-smoke 引用。
- 本地 gateway（:31337）真有 3 条 capsule，含一条 dogfood 讲解页 capsule(14KB) +
  两条 T1 合成 fixture（codex/claude）→ CDP 验收有真数据（验收时按本地实际 id 取，勿把真 id 写进提交物）。
- 约束：desktop `CLAUDE.md`（`*:focus{outline:none}` 有意保留、公共仓库无个人数据、暖中性+绿强调仅语义态、
  light only）；`desktop-ui.md` + product-ui skill（用 lucide、选中态单色非绿、provider/agent 走共享展示助手、
  transcript 一律 server render_state 派生不本地重算）。

---

## 1. 范围 / 非目标

**做（全部仅 `desktop/garyx-desktop/`）**：
1. 卡片画廊（gallery grid）替换 master-detail。
2. 抽 `CapsuleLivePreviewFrame`（card / preview 两态，三处复用）。
3. 去套娃预览页（thin toolbar + 近全屏 iframe）。
4. capsule preview 路由 `#/capsules/<id>` + deep link `garyx://capsules/<id>`。
5. 聊天 final 后 capsule 卡片哑渲染（读 `render_state` 的 `capsule_cards`，点击进 app 内 preview）。
6. AppShell 级共享 HTML 缓存（`id:revision` 键）+ 懒挂载 / 并发上限 / 删除 prune / 404→deleted。

**不做**：改 gateway/bridge/models/iOS；改 render_state reducer（卡片位置 server 已定）；本地扫 tool result /
查 capsules 决定插卡；新增 `capsule_get`；服务端缩略图；持久化截图；`allow-same-origin` / `webview`。

---

## 2. 共享 contract 与 404→deleted（结构化 IPC result）

> R1 评审采纳：放弃「`Promise<string>` + 跨 IPC 哨兵字符串」，改为**结构化 IPC result**（codex 评审更干净，且
> 彻底消除 Electron 给 IPC 异常 message 加前缀的脆弱性）。`getCapsuleHtml` 现仅 CapsulesPanel 用（被本任务重写）、
> 未被任何 mjs 测试/electron-smoke/mock 引用（已 grep 证），改返回类型 blast radius 受控。**deleted 是值、瞬态是 reject**：
> 删除→`/serve` 404→`{status:'deleted'}`；5xx/网络/离线→**抛原始错误**→renderer 归类 retryable，绝不误判 deleted（守 §6）。

### 2.1 `src/shared/contracts.ts`

(a) `DesktopDeepLinkEvent` union（:1555）追加：

```ts
  | {
      type: "open-capsule";
      url: string;
      capsuleId: string;
    }
```

(b) 新增 capsule HTML 取数 result 类型 + 改 `GaryxDesktopApi.getCapsuleHtml` 签名（:2211）：

```ts
export type DesktopCapsuleHtmlResult =
  | { status: "ok"; html: string }
  | { status: "deleted" };
// GaryxDesktopApi：
//   getCapsuleHtml: (capsuleId: string) => Promise<DesktopCapsuleHtmlResult>;
```

`RenderCapsuleCard`/`RenderCapsuleAction`/`RenderUserTurnRow.capsule_cards?` T1 已建，不改。

### 2.2 `gary-client.ts` / `main/index.ts` / `preload`：结构化 404→deleted

`gary-client.ts getCapsuleHtml`（:4889）返回 `Promise<DesktopCapsuleHtmlResult>`：成功 `{status:'ok',html}`；
`catch (GatewayRequestError && status===404)` → `{status:'deleted'}`；**其它错误原样 rethrow**（瞬态 → renderer reject → retryable）。
`main/index.ts:942` IPC handler 与 `preload/index.ts:183` 原样转发（无需 try/catch，deleted 已是返回值）。

```ts
// gary-client.ts
export async function getCapsuleHtml(
  settings: DesktopSettings,
  capsuleId: string,
): Promise<DesktopCapsuleHtmlResult> {
  const id = capsuleId?.trim() || "";
  if (!id) throw new Error("capsuleId is required");
  try {
    const html = await requestText(settings, `/api/capsules/${encodeURIComponent(id)}/serve`, {
      signal: AbortSignal.timeout(15000),
    });
    return { status: "ok", html };
  } catch (error) {
    if (error instanceof GatewayRequestError && error.status === 404) {
      return { status: "deleted" };
    }
    throw error; // 瞬态/5xx/离线 → retryable，绝不当 deleted
  }
}
```

> 统一缓存键 `capsuleHtmlCacheKey(id, revision) = `${id}:${revision}``（§3.3 / m6，绝不带 sha）由 §4 的 store 模块
> 导出（store + `CapsuleLivePreviewFrame` 的 iframe `key` 共用），不再单独建 `capsule-html.ts`。砍掉 CapsulesPanel
> 现 3 段键（:26-28）。

---

## 3. 路由：`#/capsules/<id>`

### 3.1 `desktop-route.ts`（含把 `currentDesktopRoute` 迁来，便于 headless round-trip 测）

- `DesktopRoute` union（:5）加 `| { kind: 'capsule'; capsuleId: string }`。
- `parseDesktopRoute`：在 `SIMPLE_VIEW_SEGMENTS` 查表（:165）**之前**插分支：
  `if (first === 'capsules' && second) return { kind: 'capsule', capsuleId: second };`
  （`#/capsules`（无 second）仍落 simpleView → `{kind:'view',view:'capsules'}` gallery，不变）。
- `contentViewForDesktopRoute`：`case 'capsule': return 'capsules';`。
- `buildDesktopRouteHash`：`case 'capsule': return `#/capsules/${encodeSegment(route.capsuleId)}`;`。
- **把 `currentDesktopRoute`（现 AppShell.tsx:1324 module-scope 纯函数）迁入 `desktop-route.ts` 并 `export`**：
  它只用纯输入对象 + `ContentView`/`SettingsTabId`（两类型 desktop-route.ts 已 import），无 React 依赖，迁移是干净重构、
  且让 round-trip 可 headless 单测（AppShell.tsx 因 import 海量 React/CSS 无法被 node:test 直接 import）。入参加
  `capsulePreviewId: string|null`；`view==='capsules'` 分支：`return capsulePreviewId ? {kind:'capsule',capsuleId:capsulePreviewId} : {kind:'view',view:'capsules'}`。
  AppShell 改为 `import { currentDesktopRoute } from './desktop-route'`。

### 3.2 AppShell：lifted `capsulePreviewId`（**含冷启动 seed，修 R1-B1**）

- **冷启动 seed**：`initialRouteValue`（AppShell:1521，mount 时 `parseDesktopRoute()` 一次）已含初始 route。
  state 初值必须从它派生，否则冷启动 `#/capsules/<id>` 会：`contentView` 初始化成 `capsules`（`contentViewForDesktopRoute`），
  但 `capsulePreviewId` 仍 `null` → replace effect（:4247）反写 `#/capsules` 把 id 抹掉。修法：
  ```ts
  const [capsulePreviewId, setCapsulePreviewId] = useState<string | null>(
    initialRouteValue.kind === "capsule" ? initialRouteValue.capsuleId : null,
  );
  ```
- `applyDesktopRoute`（:4124 switch）加：
  ```ts
  case 'capsule':
    setContentView('capsules');
    setCapsulePreviewId(route.capsuleId);
    return;
  ```
  并在 `case 'view'`（:4161）里：当 `route.view==='capsules'` 时 `setCapsulePreviewId(null)`（rail/gallery 语境清预览）。
- `replaceDesktopRoute` effect（:4247）：`currentDesktopRoute({..., capsulePreviewId})`（已迁 desktop-route.ts），依赖数组加 `capsulePreviewId`。
- rail `onOpenCapsules`（:10159）：`setContentView('capsules'); setCapsulePreviewId(null);`（rail 进 gallery 不带预览）。
- `<CapsulesPanel>`（:10581）传：`selectedCapsuleIdFromRoute={isCapsulesView ? capsulePreviewId : null}`、
  `onOpenCapsulePreview={(id)=>setCapsulePreviewId(id)}`、`onCloseCapsulePreview={()=>setCapsulePreviewId(null)}`。

> 路由是单一真相：CapsulesPanel **不自持** preview 选择，全部从 `selectedCapsuleIdFromRoute` 派生、改动经
> `onOpen/onClose` 冒泡到 AppShell→hash。这样 deep link / 聊天卡 / gallery 点击 / **冷启动** 四入口统一，Back=清 id=回 gallery。
> 冷启动正确性由 §10.1 的 `currentDesktopRoute({view:'capsules', capsulePreviewId:'X'})→capsule 路由` round-trip 测 +
> `parse('#/capsules/<id>')→capsule` 测共同守护，并在 §12 CDP 用「带 `#/capsules/<id>` hash 启动→直接落预览非 gallery」复核。

### 3.3 deep link `garyx://capsules/<id>`

- `deep-link.ts:parseDesktopDeepLink`：`action === 'capsules' || action === 'capsule'` 分支
  （仿 thread 分支：禁 `url.search`、要求 `firstSegment`、`segments.length>1` 报错）→
  `{ type:'open-capsule', url, capsuleId: firstSegment }`。`DEEP_LINK_USAGE` 文案补 `garyx://capsules/<id>`。
- `main/index.ts:queueDeepLink`（:560 trace label 三元）加 `event.type==='open-capsule' ? `open-capsule:${event.capsuleId}` : …`。
- AppShell `deepLinkEventHandlerRef`（:6838 switch）加
  `case 'open-capsule': await applyDesktopRoute({kind:'capsule',capsuleId:event.capsuleId}); return;`。

---

## 4. 共享 HTML store（AppShell 级，单例 + `useSyncExternalStore`）

新 `src/renderer/src/app-shell/capsule-html-store.ts`：模块级单例，gallery/preview/聊天卡共用。**不**把
`htmlByKey` 塞进 AppShell 的 `useState`（避免每次 HTML 落地整 shell 重渲——守 iOS 雅克教训的“窄观测”），
组件按 key 订阅，capsule A 的加载不触发 capsule B 的卡片重渲。

状态（`Map<cacheKey, Entry>`，`Entry = { status:'loading'|'ready'|'deleted'|'error', html?, error? }`）+
`generationById: Map<id, number>`（per-id epoch，**修 R1-B3 删除竞态**）。API：

```ts
export type CapsuleHtmlState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; html: string }
  | { status: 'deleted' }
  | { status: 'error'; message: string };

// 统一缓存键（§2.2，三处共用，不带 sha）
export function capsuleHtmlCacheKey(id: string, revision: number): string;
// 命令式（供事件回调/刷新/删除用）
capsuleHtmlStore.request(id, revision, { force?: boolean }): void   // 入队，遵守并发上限
capsuleHtmlStore.invalidateCapsule(id): void                        // 删除后调用：清队列 + bump generation + 置 deleted
// React hook（组件用）：active=false 时不请求、返回 idle/缓存命中
useCapsuleHtml(id: string, revision: number, opts: { active: boolean }): CapsuleHtmlState
```

并发：内部 `maxConcurrent = 4`，`inflight` 计数 + 待处理队列；`request` 命中缓存（ready/deleted）直接返回，
loading 中去重，否则入队，slot 空出再 `getCapsuleHtml`。`force` 跳过缓存并把该 key 置回 loading 重取
（serve 永远给最新内容；revision 升时键本就变，force 用于「内容原地被改但 revision 没变」的手动刷新）。

**结果分类（结构化，无哨兵）**：`getCapsuleHtml` resolve `{status:'ok',html}`→`ready`；`{status:'deleted'}`→`deleted`；
**reject**（瞬态/5xx/离线）→`error{message}`（retryable，绝不当 deleted，守 §6）。

**删除竞态 stale guard（R1-B3 核心）**：每次入队/起飞的 job 捕获起飞时的 `gen = generationById.get(id) ?? 0`。
`invalidateCapsule(id)` 做三件事：① `generationById.set(id, gen+1)`；② 从待处理队列移除该 id 的所有 job；
③ 把该 id 现有 keys 的 Entry 直接置 `{status:'deleted'}`（让当前已挂载的聊天卡/预览**立即**翻 tombstone，无需重 fetch）。
任何 in-flight job 完成时（resolve 或 reject）**先校验** `(generationById.get(id) ?? 0) === job.gen`，不等则**丢弃结果不写 store**
（IPC 无法真 abort 主进程 fetch，靠 gen 守卫吞掉迟到写）。→ 删除时同 id 正在飞的 fetch 完成后不会把已删 HTML 写回（守 §6
「删除后 /serve 404 → deleted」）。

取数 fetcher 默认 `window.garyxDesktop.getCapsuleHtml`，暴露 `__setCapsuleHtmlFetcherForTest(fn)` +
`__resetCapsuleHtmlStoreForTest()` 供 headless 测（注入受控 fetcher 验并发上限 / dedupe / 缓存命中 / force 重取 /
deleted 分类 / **delete-while-inflight 迟到写被丢弃**）。

> 「mounted iframe 数」由组件侧 IntersectionObserver 决定（只可视区挂 iframe）；「同时 fetch 数 ≤4」由 store 决定。
> 二者正交：gallery 滚动只让可视卡 `active=true` 去 `useCapsuleHtml` 请求，离屏卸载 iframe（脚本停跑）。

---

## 5. `CapsuleLivePreviewFrame`（三处复用基石）

新 `src/renderer/src/app-shell/components/CapsuleLivePreviewFrame.tsx`：

```ts
type Props = {
  capsuleId: string;
  revision: number;
  title: string;
  mode: 'card' | 'preview';
  active: boolean;            // card: IntersectionObserver 给；preview: 恒 true
};
```

- `const state = useCapsuleHtml(capsuleId, revision, { active })`。
- `state.status==='ready'` 才渲 `<iframe>`，否则渲 skeleton / `Capsule deleted` / error（card 模式错误也走静默 skeleton + 角标，不刷红块）。
- **安全恒定**：`sandbox="allow-scripts"`（无 `allow-same-origin`）、`srcDoc={state.html}`、`key={cacheKey}`（revision 变→换 frame）、`title`。
- **card 模式**：外层 `.capsule-frame-shell`（固定 aspect、`overflow:hidden`、`pointer-events:none`）；iframe 固定虚拟视口
  `width:1024 height:640` + `transform: scale(W/1024)` `transform-origin: top left` + `tabIndex={-1}` + `pointer-events:none`（双保险，untrusted HTML 吃不到点击/焦点）。scale 由容器宽度算（`ResizeObserver` 或 CSS `--capsule-card-scale`；首版用固定卡宽推导的常量 scale，避免每帧测量）。
- **preview 模式**：iframe `width:100% height:100%`、允许网页内交互（无 `pointer-events:none`）、sandbox 仍仅 `allow-scripts`。
- `active===false`：不挂 iframe（返回 skeleton），离屏即卸载。

---

## 6. 画廊 + 去套娃预览（重写 `CapsulesPanel.tsx`）

`CapsulesPanel` 改为「gallery 与 preview 两态」，由 `selectedCapsuleIdFromRoute` 切换：

```
selectedCapsuleIdFromRoute == null  → <CapsuleGallery>
selectedCapsuleIdFromRoute != null  → <CapsulePreviewPage capsuleId=...>
```

数据仍 `loadCapsules()`（`listCapsules`）；保留 list request-id 防竞态（:159）。删除 3 段 cacheKey（:26）、
`loadSelectedHtml`/`htmlByKey`/`htmlErrorById`/`htmlLoadingId`（:155-238，HTML 全交给 store）。

### 6.1 Gallery（新 `CapsuleGalleryCard.tsx` 或同文件子组件）

- header 沿用 compact title + count chip + Refresh（:300-323 复用）。
- grid：`.capsules-gallery-grid`，CSS `grid-template-columns: repeat(auto-fill, minmax(220px, 1fr))`，大屏卡宽上限 ~320px（`max-width` on card 或 `minmax(220px, 320px)`+居中）。
- 每卡 = `<button class="capsule-gallery-card">`（整体可点 → `onOpenCapsulePreview(capsule.id)`）：
  上半 `.capsule-card-preview-shell` 包 `<CapsuleLivePreviewFrame mode="card" active={visible}>`（`visible` 由
  `IntersectionObserver` 标记，含小 overscan）；下半 title 单行 ellipsis（空→`Untitled Capsule`，复用 `capsuleTitle`）
  + 副信息 `updated relative · creator`（复用 `formatRelativeTime`、`describeCreator`/`CreatorBadge`），revision/byteSize 进 `title`/次级。
- 空/加载/错误：沿用 `tasks-empty-state` / `tasks-state tasks-state-error`（:325-328）语义，去左栏视觉。
- 卡上不放 Delete（Delete 收进 preview 的 overflow，保持画廊干净；与 iOS contextMenu 分歧由 IA 主权 Mac 决定——
  桌面卡片整体即“打开”，删除在预览里，符合 restraint）。

### 6.2 Preview 去套娃（新 `CapsulePreviewPage.tsx`）

- `.capsule-preview-page` 占满内容区；**不**显示 description/revision/byteSize/timestamp/id 大块。
- 顶部 `.capsule-preview-toolbar`（~40px、半透明 + blur、不挡网页）：
  - 左：Back（lucide `ArrowLeft`）→ `onCloseCapsulePreview()`（回 gallery）。
  - 中：compact title（单行小字）。
  - 右：Refresh（lucide `RefreshCw`，`capsuleHtmlStore.request(id, rev, {force:true})` 强制重取）、
    Copy link（lucide `Link2`，复制 `garyx://capsules/<id>` → toast，复用 `copyTextToClipboard`）、
    overflow `⋯`（lucide `MoreHorizontal`）菜单：Copy ID（裸 UUID）/ Delete（destructive，`window.confirm` 后
    `deleteCapsule` → `capsuleHtmlStore.invalidateCapsule(id)` → `onCloseCapsulePreview()` + `loadCapsules()` + toast）。
- body：`<CapsuleLivePreviewFrame mode="preview" active>`，capsule 由 `capsules.find(id) || getCapsule(id)` 解析
  （deep link 直达时本地 list 可能未含 → 兜底 `window.garyxDesktop.getCapsule(id)` 取 summary 拿 revision/title；
  取不到或 serve 404 → disabled「Capsule deleted」+ 顶栏仍可 Back）。
- 简易 overflow 菜单：复用现有 shadcn/menu 模式（先查 app-shell 是否已有 DropdownMenu；无则用受控 `useState` + 绝对定位小菜单 + `pointer-events` 遮罩，避免引新依赖）。

---

## 7. 聊天 final 后 capsule 卡片（哑渲染）

### 7.1 `render-view-model.ts`（纯结构翻译，守文件头契约）

`buildThreadViewRows`：

- `UserTurnRow`（:73-78）加 `capsuleCards: RenderCapsuleCard[]`（import 类型）。
- user 解析到时 `rows.push({ kind:'user_turn', key, userBlock, activityRows, capsuleCards: row.capsule_cards ?? [] })`。
- orphan 分支（:299）：orphan 无 user 行，**不造假 userBlock**；若 `row.capsule_cards?.length`，在 `rows.push(...activityRows)`
  后追加轻量顶层行 `{ kind:'capsule_only', key:`capsule-cards:${row.id}`, capsuleCards }`。
  → `TurnRenderRow` 加 `| { kind:'capsule_only'; key:string; capsuleCards: RenderCapsuleCard[] }`（仅 orphan 且有卡才产）。
- `buildThreadViewRowsWithLocalUsers`（:304）本地乐观 user 行 `capsuleCards: []`。
- `capsuleCards`/`capsule_only` **不计入** `collectBlockMessageIds`/`representedMessageIds`（:113-165）——卡片不是消息，
  否则 represented 判定 / visible id 被污染。`representedMessageIdsForRows`（:148）的 row switch 对 `capsule_only` 直接跳过。

### 7.2 `ThreadPage.tsx`

- `ThreadPageProps`（:245）加 `onOpenCapsule?: (capsuleId: string) => void`（仿 `onOpenThreadById`:378）。
- user_turn 渲染分支（:928-937）在 `activityRows.map(...)` **之后**追加
  `{row.capsuleCards.length ? <CapsuleChatCardList cards={row.capsuleCards} onOpenCapsule={onOpenCapsule}/> : null}`；
  顶层 row 渲染加 `case 'capsule_only'` → `<CapsuleChatCardList>`。
- 新 `src/renderer/src/app-shell/components/CapsuleChatCard.tsx`：
  - `CapsuleChatCardList`：单列 compact（首版卡宽上限 ~360px）。
  - 每卡 `<button onClick={()=>onOpenCapsule?.(card.capsule_id)}>`：上半 `<CapsuleLivePreviewFrame mode="card" active={visible}>`（`visible` 由 IntersectionObserver；**聊天卡更保守**：同时 active ≤ 2，由 store 并发上限 + 可视区门控共同保证），下半 title + `action`（Created/Updated）小标。
  - serve 404 → `state.status==='deleted'` → disabled「Capsule deleted」，不重试加载（store 已缓存 deleted）。
- AppShell 两处 `<ThreadPage>`（:9361 side-chat、:9610 main）都传
  `onOpenCapsule={(id)=>{ setContentView('capsules'); setCapsulePreviewId(id); }}`（点聊天卡→app 内 preview，Back 回 gallery，守 §3.4）。

### 7.3 `render-view-model.test.mjs`：改 T1 断言（:472）+ 新增

- 改 :472「tolerated but not rendered」→ 断言 `rows[0].capsuleCards` 长度 1、`capsule_id` 对、**且** `visibleMessageIds`/
  `blockMessageIds(blocks)` 仍只含 `seq:1,seq:2`（卡片不入 visible/blocks）。这是**加强**断言非放松。
- 新增：orphan（user 解析不到）+ `capsule_cards` → 顶层 `capsule_only` 行。

---

## 8. CSS（`styles.css`）

替换 `.capsules-layout`/`.capsules-list*`/`.capsules-detail*`/`.capsules-runner-*`（:17661-17866）为：
`.capsules-gallery-grid`（auto-fill minmax 220/320）、`.capsule-gallery-card`（按钮、border/radius、hover 单色提亮、
上 preview 下 meta）、`.capsule-card-preview-shell`（aspect 16:10、overflow hidden、白底、pointer-events:none）、
`.capsule-live-frame`（虚拟视口 + scale + transform-origin）、`.capsule-preview-page`（占满）、
`.capsule-preview-toolbar`（~40px、`backdrop-filter:blur`、半透明 border-bottom）、`.capsule-chat-card*`。
保留 `.capsules-page*`/header/`.capsules-agent-badge`（:17606-17660/17756-17776）。暖中性 token、选中/hover 单色非绿、
soft border、subtle shadow，守 desktop `CLAUDE.md` 审美。Delete 用 `--color-destructive`（复用现 `.capsules-delete-button`）。

---

## 9. i18n

新可见串走 `t(...)`：`Capsule deleted`、`Copy link`、`Capsule link copied.`、`Back`、`Updated`/`Created`、
`Loading Capsule…`、空标题复用 `Untitled Capsule`。查 `src/renderer/src/i18n` 词典补齐英文 key + 中文翻译
（避免 Phase B review 抓到的“中文词典缺”非阻断项）。Copy ID/Delete/Refresh/`Untitled Capsule` 等已有 key 复用。

---

## 10. 测试（headless 优先）

均登记进 `package.json` `test:unit`（**显式列表，新文件必登记**）：

1. `desktop-route.test.mjs`（改）：`parse('#/capsules/<uuid>')→{kind:'capsule',capsuleId}` + `contentViewForDesktopRoute==='capsules'`；
   `buildDesktopRouteHash({kind:'capsule'})==='#/capsules/<encoded>'`；`#/capsules`（无 id）仍 `view:'capsules'`（回归保护）；
   **冷启动 round-trip（修 R1-B1）**：`currentDesktopRoute({contentView:'capsules', capsulePreviewId:'X', …})==={kind:'capsule',capsuleId:'X'}`、
   `capsulePreviewId:null` 时回 `{kind:'view',view:'capsules'}`（即 `currentDesktopRoute` 已迁 desktop-route.ts，可直接 import 测）。
2. `render-view-model.test.mjs`（改 :472 + 新增）：capsuleCards 透传、不入 visible/blocks；orphan→`capsule_only`；
   本地乐观 user 行 capsuleCards=[]。
3. **新** `src/main/deep-link.test.mjs`：`garyx://capsules/<id>`→`{type:'open-capsule',capsuleId}`；带 query / 多段报错；
   顺带覆盖既有 thread/new/resume（补 deep-link 现无测试的缺口，最小）。
4. **新** `src/renderer/src/app-shell/capsule-html-store.test.mjs`：注入受控 fetcher 验
   并发 ≤4、同 key dedupe、缓存命中不重取、`force` 重取、`{status:'deleted'}` 分类、瞬态 reject→error、
   **delete-while-inflight：invalidate 后迟到 resolve 被 gen 守卫丢弃、Entry 维持 deleted（修 R1-B3）**；`capsuleHtmlCacheKey` 形态。

> `CapsuleLivePreviewFrame`/gallery/preview/chat card 是 React DOM 组件（iframe/IntersectionObserver），
> headless node:test 不覆盖，靠 build:ui 类型 + CDP 实测把关（守任务“reviewer 真 build+CDP 实测”）。

---

## 11. 安全 / 性能不变量（验收 checklist）

安全（守 §8）：**capsule iframe**（`.capsule-live-frame`，三处统一组件）`sandbox="allow-scripts"` 且**无** `allow-same-origin`；
无 Electron `<webview>`（CDP `webviewCount===0`）；HTML 经主进程 auth fetch（renderer 不直连 gateway、不持 token）；
Copy link 用 `garyx://capsules/<id>`（不带 token）；title/meta 是 native text 非 HTML 注入；card 模式 `pointer-events:none`+`tabIndex:-1`。
> 注（R1 非阻断采纳）：CDP 断言**限定 capsule iframe**（按 class/选择器），不全局扫所有 `<iframe>`——仓库存在
> 非 capsule 的 `allow-same-origin` iframe，全局扫会误报。

性能（守 §7）：gallery/聊天卡 IntersectionObserver 懒挂载（离屏卸载 iframe）；store 同时 fetch ≤4；聊天卡同时 active ≤2；
离开 `contentView!=='capsules'` / preview 关闭 → 相关 iframe 卸载；HTML cache `id:revision` 命中复用、delete prune。

---

## 12. 验收

- `npm run build:ui`（tsc --noEmit + electron-vite build）绿；`npm run test:unit` 绿（含新增/改 4 类测试）。
- `npm run dist:dir` 装包 + 退旧进程 + CDP attach（`playwright-cli -s=<s> attach --cdp=http://127.0.0.1:39222`，连本地真 gateway :31337）：
  1. gallery 显示 3 条真 capsule（本地已有），卡片上半 live 预览渲染（非空白）。
  2. 点卡 → 去 chrome 预览近全屏；toolbar 有 Back/Refresh/Copy link/⋯；Back 回 gallery。
  3. 聊天里（开一条带 capsule 的线程）final 后出现 capsule 卡片，点击进 preview。
  4. **冷启动（修 R1-B1 复核）**：以 `#/capsules/<id>` hash 启动 app → 直接落 preview（非 gallery），hash 不被抹成 `#/capsules`。
  5. DOM 断言（**限定 capsule iframe**）：`[...document.querySelectorAll('.capsule-live-frame')].every(f => !f.sandbox.contains('allow-same-origin'))`、
     `document.querySelectorAll('webview').length===0`。

---

## 13. 风险 / 取舍

| # | 风险 | 缓解 |
|---|---|---|
| D1 | AppShell 巨组件接线（lifted state + 两处 ThreadPage + route 反推）易漏依赖/竞态 | 单一真相=route；`capsulePreviewId` 进 `currentDesktopRoute` 入参 + effect 依赖；rail/gallery/deep-link/聊天卡四入口都经 `applyDesktopRoute`/`setCapsulePreviewId` 同一路径；改完 grep `capsulePreviewId` 核全用点 |
| D2 | 404→deleted 误判（瞬态当 deleted） | 结构化 result：`getCapsuleHtml` 仅 `GatewayRequestError.status===404` 返 `{status:'deleted'}`，其余原样 rethrow→renderer `error` retryable；store 测覆盖瞬态 reject |
| D3 | card 模式 scale 计算每帧测量抖动/性能 | 首版用固定卡宽推导常量 scale（CSS var），不用每帧 ResizeObserver；虚拟视口固定 1024×640 |
| D4 | 共享 store 并发/dedupe/删除竞态写错 | store 纯逻辑抽出 + headless 注入 fetcher 测并发上限/dedupe/force/缓存命中/**delete-while-inflight gen 守卫** |
| D5 | orphan turn 插卡破坏“纯结构翻译”契约 | 不给 orphan 造假 userBlock；新增独立 `capsule_only` 顶层行，零分组逻辑 |
| D6 | 聊天卡导致 thread 上下文丢失（点卡跳走） | 父稿 §3.4 已批准：首版 Back 统一回 gallery，不维护跨-view return stack |
| D7 | 改 `render-view-model.test.mjs:472` 被当“放松测试” | 改后断言更强（透传 + 仍不入 visible/blocks），非删除；保留 unknown-kind 容错用例 |
| **R1-B1** | 冷启动 `#/capsules/<id>` 被抹成 gallery | `capsulePreviewId` 从 `initialRouteValue` seed（§3.2）+ `currentDesktopRoute` 迁 desktop-route.ts 加 round-trip 测 + CDP 冷启动复核 |
| **R1-B3** | 删除时同 id in-flight fetch 把已删 HTML 写回 | store per-id generation 守卫 + `invalidateCapsule` 清队列/置 deleted（§4）+ delete-while-inflight 测 |

---

## 14. 实现顺序（worktree）

1. `contracts.ts`（deep-link 变体 + `DesktopCapsuleHtmlResult` + getCapsuleHtml 签名）→ `gary-client.ts`(结构化 404→deleted) → `main/deep-link.ts`(+test) → `main/index.ts`(trace label)。
2. `desktop-route.ts`(迁入 currentDesktopRoute + capsule 路由 + test) → AppShell route/deeplink/lifted state 接线（含冷启动 seed）。
3. `capsule-html-store.ts`(+test，含 gen 守卫) → `CapsuleLivePreviewFrame.tsx`。
4. `CapsulesPanel.tsx` 重写（gallery + preview）+ CSS。
5. `render-view-model.ts`(capsule 字段 + test) → `CapsuleChatCard.tsx` → `ThreadPage.tsx`(行渲染 + onOpenCapsule) → AppShell 传 `onOpenCapsule`。
6. i18n 补串 → `build:ui` + `test:unit` 绿 → `dist:dir` + CDP 实测（含冷启动）。
7. 自开 code review 给 codex 到 100% PASS → 合 main。
