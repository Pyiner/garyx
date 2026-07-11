# Mac / iOS Recent 列表 task 筛选 Tab

状态：产品方向已确认；已按最新主线逐节复核并修订，待设计 review。

原始设计基线：`origin/main` @ `1a4230bde`。

复核基线：`origin/main` @ `2a22eb93d`（2026-07-12）。

修订记录：

- v2（2026-07-12）：逐节复核 §3–§10。主架构成立；补齐成功空页的
  primed 状态、head-refresh 失败状态、Mac IPC 的 Gateway scope 归属、常驻
  controller 生命周期、iOS snapshot/filter phase、双 feed mutation 与当前无
  iOS task-create 入口等实现边界。完整核对结论见 §13。

关联设计：

- `docs/design/bot-recent-threads-commands.md`：定义
  `GET /api/recent-threads?tasks=include|exclude|only`、过滤后分页和默认兼容语义。
- `docs/design/task-1802-ios-home-list-refresh-loadmore.md`：定义 iOS Recent
  列表现有 refresh/load-more pager、不破坏式 head refresh 和请求 epoch。

## 1. 目标

Mac Recent rail 与 iOS 首页 Recent 区域增加两个互斥筛选 Tab：

| 产品 Tab | 中文 | wire 参数 | 默认 | 行集 |
|---|---|---|---:|---|
| `All` | 全部 | `tasks=include` | 是 | 普通 thread + task backing thread |
| `Chats` | 对话 | `tasks=exclude` | 否 | 排除 `thread_type="task"` |

产品要求：

1. 冷启动、Gateway 切换后默认 `All`，保持当前 Recent 成员集不变；
2. 筛选只作用于 Recent，Pinned 不受影响；
3. 筛选必须发生在服务端分页之前，不能先取一页再由客户端删除 task；
4. Mac 与 iOS 使用同一标签、默认值和 wire 映射，但采用各自原生交互；
5. Tab 切换不改变当前打开的 thread，也不影响其它 thread 入口。

## 2. 非目标

- 不增加 `Tasks only` Tab；现有 `tasks=only` API 保留给其它消费者。
- 不改变 Gateway SQL、route 响应 envelope 或排序规则。
- 不筛选 Pinned、Workspace/Bot drilldown、Task tree、deep link 或当前会话。
- 不让 iOS Widget 或 Automation existing-thread picker 跟随主页 Tab。
- 本期不增加 task 行 badge、不增加搜索、不持久化用户最近一次 Tab。
- 不用客户端本地过滤作为旧 Gateway fallback；本地过滤会让 `total`、
  `has_more` 和 offset 分页失真。

## 3. 现状与可复用契约

### 3.1 Gateway 已具备完整数据契约

`garyx-gateway/src/routes.rs::list_recent_threads` 已将 `tasks` 解析为
`RecentThreadTaskFilter`，并通过 `GaryxDbService::list_recent_threads_page`
在同一显式读事务中完成过滤域内的 count + page。API 上限为 200 行，响应包含：

- `threads`
- `count`
- `total`
- `limit`
- `offset`
- `has_more`

本设计不新增 Gateway 生产代码。两个客户端显式发送 `tasks=include` 或
`tasks=exclude`，不依赖省略参数的隐式默认值。

### 3.2 Mac 当前 Recent 不是独立 feed

`desktop/garyx-desktop/src/renderer/src/app-shell/AppShell.tsx` 当前直接把
`desktopState.threads` 映射为 `recentThreadRows`。稳定态该 slice 来自
`garyx-client/threads.ts::fetchThreads` 的 `/api/threads?limit=1000`，同时还服务
Workspace/Bot 分组、Pinned 修复和其它全量线程用途。冷启动的
`getStateFast()` 会先以同一 `/api/threads` route 取 200 行并按 id 补齐 Pinned，
随后 `getState()` 恢复 1000 行语义；它并未调用 `/api/recent-threads`。

因此不能把 `DesktopState.threads` 改成某个 Recent 筛选结果；否则会把一个
局部 UI filter 扩散为全局数据丢失。Mac 需要独立的、renderer session 内的
Recent feed controller。

### 3.3 iOS 当前只有一份 feed + pager

`GaryxGatewayClient.listRecentThreads` 当前只发送 `limit`、`offset`；
`GaryxMobileModel` 只有一份 `recentThreadIds` 和一个
`GaryxHomeThreadListPager`。Widget snapshot 与 Automation thread picker 也读取
这份 Recent 顺序。

因此不能在切换 Tab 时直接覆写现有 `recentThreadIds`：这样会让 Widget 与
Automation picker 意外变成 `Chats` 视图。iOS 需要 filter-keyed feed，并明确区分：

- `allRecentThreadIds`：辅助消费者的稳定 `All` 数据；
- `visibleRecentThreadIds`：主页当前 Tab 的展示数据。

## 4. 跨平台状态模型

### 4.1 精确 domain 类型

产品标签使用 `All` / `Chats`，内部 domain 使用精确语义，避免把未来其它
non-task 类型错误建模成 chat：

```text
RecentThreadFilter.all      -> tasks=include -> label All
RecentThreadFilter.nonTask  -> tasks=exclude -> label Chats
```

每个 Gateway scope 下维护两个独立 feed：

```text
RecentThreadFeed {
  orderedThreadIds
  isPrimed
  nextOffset
  hasMore
  isRefreshingHead
  isLoadingMore
  headRefreshFailure
  loadMoreFailure
  epoch
  localMutationSequence
  lastSuccessfulRefreshAt
}
```

Thread summary 仍按 `thread_id` 放在共享 cache 中；两个 feed 只保存顺序和分页
状态，避免复制 title、runtime、avatar 等 presentation 数据。`isPrimed` 由一次
成功采用的 head page（包括成功空页）置 true；不能用 `nextOffset > 0` 推断，
因为成功空页的 offset/count 都是 0。`headRefreshFailure` 只记录该 feed 是否需要
unavailable/retry presentation，不保存跨 Gateway 的 Error 对象。

### 4.2 生命周期

- 冷启动：`selectedFilter = .all`。
- Gateway scope 变化：清空两个 feed、递增 epoch、重置为 `.all`。
- 同一 app session 内关闭/reopen Recent rail 或进入/退出会话：保留 Tab 与 cache。
- App 重启：不恢复上次 Tab；产品默认必须稳定为 `All`。
- Tab 切换：有已 primed cache 立即渲染，并在后台做 head refresh；未 primed 才
  显示该 Tab 自己的 loading placeholder。首次 head refresh 失败后显示该 feed 的
  unavailable 状态，不能伪装成成功空态。

### 4.3 并发与响应归属

每个请求都携带逻辑键：

```text
(gatewayScope, filter, feedEpoch, pagerTicket)
```

Mac 的 IPC result 必须带 main process 实际使用的 normalized Gateway URL scope；
renderer 同时校验 ticket scope 和 result scope。仅在 renderer 发请求前读取一次
`desktopState.settings.gatewayUrl` 不足以证明归属，因为 Gateway 切换可能与 IPC
handler 取 settings 交错。iOS 由 `gatewayRuntimeGeneration/currentGatewayScopeId` +
filter-bearing Core ticket + pager epoch 共同组成同一归属键。

完成时写回请求所属 feed，而不是“当前选中的 feed”。这允许用户快速来回切换，
同时保证：

- `All` 的迟到响应不会覆盖 `Chats`；
- 切回 `All` 时仍可采用它已完成的响应；
- Gateway 切换前的响应因 scope/epoch 不匹配被丢弃；
- 两个 filter 的 offset、失败 gate、retry 互不污染。

### 4.4 过滤与分页不变量

客户端必须把 server page 原样应用到对应 feed：

- `All` 只请求 `tasks=include`；
- `Chats` 只请求 `tasks=exclude`；
- offset 由该 feed 最近一次采用的 server page 推进；
- `total`/`has_more` 只解释为该 filter 域内的值；
- 不读取 thread record body 判定 task，不扫描、不本地删行补页。

head refresh 与 load-more 的 id merge/dedupe 不是 task filtering fallback。两个平台
load-more 都从各自 feed 的 cursor 发起；为吸收本地 archive/delete 造成的小幅 offset
左移，采用与现有 iOS pager 相同的 5 行 overlap，cursor 始终按服务端实际返回的
`page.offset + page.count` 推进。

## 5. 产品交互

### 5.1 Mac

Recent secondary rail 结构：

```text
┌──────────────────────────┐
│ ◷ Recent             [‹] │
│ ┌─────────┬────────────┐ │
│ │   All   │   Chats    │ │
│ └─────────┴────────────┘ │
│ thread row               │
│ thread row               │
│ ...                      │
└──────────────────────────┘
```

交互规格：

- segmented control 位于 rail title 下、thread list 上，宽度铺满可用内容区；
- 普通 selected state 使用 monochrome/primary，不使用绿色或运行色；
- `All` 在左，`Chats` 在右；不显示 count，避免未选 Tab 的额外 count 请求和跳动；
- 键盘可聚焦，左右方向键切换；容器有 `Recent filter` accessible label；
- 切换不关闭 rail、不改变当前打开 thread、不自动 archive 或 unpin；
- 空态分别为 `No recent threads`、`No recent chats`；
- 有旧 cache 时 refresh 失败仍保留 rows，并显示该 feed 的非阻塞 retry footer；
- load-more 在接近列表尾部触发，失败只影响当前 feed。

### 5.2 iOS

iOS 首页保持 Pinned + Recent 信息架构：

```text
Pinned
  pinned row

Recent
  ┌─────────┬────────────┐
  │   All   │   Chats    │
  └─────────┴────────────┘
  recent row
  recent row
```

交互规格：

- 在现有 `Recent` header 下增加一个独立 flat `List` row；
- 使用原生 SwiftUI `Picker` + `.pickerStyle(.segmented)`，不自绘 segmented
  背景、分割线和选中颜色；
- Pinned section 在 Picker 之外，因此 pinned task 在 `Chats` 下仍可见；
- Dynamic Type 下允许 control 占满一行，Recent 标题不与 Picker 强行并排；
- VoiceOver label 为 `Recent filter`，选项为 `All` / `Chats`；
- filter selection 与 feed 状态进入 `GaryxMobileCore`/model，SwiftUI view 只绑定
  snapshot 和 action；
- near-tail prefetch、固定 footer、pull-to-refresh 继续复用现有 pager 语义，但
  作用于当前 selected feed。

## 6. Mac 架构落点

### 6.1 Main/preload contract

在 shared contracts 定义 renderer 可见的窄类型，在
`desktop/garyx-desktop/src/main/garyx-client/threads.ts` 新增实际 HTTP mapping：

```ts
type RecentThreadTaskFilter = 'include' | 'exclude';

type DesktopRecentThreadsPage = {
  gatewayScope: string;
  threads: DesktopThreadSummary[];
  count: number;
  total: number;
  limit: number;
  offset: number;
  hasMore: boolean;
};

fetchRecentThreads(settings, { tasks, limit, offset })
```

再通过窄 IPC 暴露：

```ts
window.garyxDesktop.listRecentThreads({ gatewayScope, tasks, limit, offset })
```

`gatewayScope` 是归属校验值，不是 renderer 自选的请求 URL。handler 通过
`resolveSettings()` 捕获实际 settings，校验其 normalized URL 与 input scope 一致，
由 main process 构造 query，并把同一 scope 放进 result；不一致作为 stale request
拒绝。`tasks` 在 main process 做 runtime enum 校验（不能只靠 TypeScript），limit /
offset 也做有限非负整数校验。page size 固定为 100，Gateway 上限 200 不变。

### 6.2 Renderer controller

新增独立 `useRecentThreadFeeds`（或等价 controller），拥有：

- session-local `selectedFilter`；
- filter-keyed page state；
- open/switch/head-refresh/load-more/retry；
- archive/delete 后的双 feed 本地移除；
- Gateway URL 变化时 reset。

不要把 feed 写入 persisted `DesktopState`，也不要改变 `DesktopState.threads` 的
全量用途。收到 Recent page 后，可把 summary 与 renderer 已有 summary map 合并用于
avatar/runtime 展示，但 Recent 的顺序只来自 page feed。

controller 的纯 reducer/state machine 与 React IO hook 分离，headless 测试直接驱动
ticket 和 page。hook 由常驻 `AppShell` 持有，只把 snapshot/actions 传给条件渲染的
rail；若把 hook 放进 rail 自身，关闭 rail 会 unmount 并违反 §4.2 的 session cache /
selection 保留契约。每个 async completion 只按自己的 ticket 写所属 feed；archive
乐观删除保存两个 feed 的 rollback token，失败恢复两个原顺序。

### 6.3 Component seam

新增 `RecentConversationSidebar` 作为 domain wrapper，内部复用
`ThreadConversationSidebar`。共享 sidebar 只按需要增加通用 slot：

- `headerAccessory?: ReactNode`；
- `listFooter?: ReactNode`；
- `onNearListEnd?: () => void`（共享 scroll container 计算，不把 Recent 语义下沉）。

不把 filter state、API 参数或 Recent 文案塞进通用 Workspace/Bot sidebar。

## 7. iOS 架构落点

### 7.1 Core 类型与 Gateway client

在 `GaryxMobileCore` 增加：

```swift
public enum GaryxRecentThreadFilter: CaseIterable, Equatable, Hashable, Sendable {
    case all
    case nonTask

    public var tasksQueryValue: String {
        switch self {
        case .all: "include"
        case .nonTask: "exclude"
        }
    }
}
```

`GaryxGatewayClient.listRecentThreads` 改为：

```swift
listRecentThreads(
    filter: GaryxRecentThreadFilter = .all,
    limit: Int = 30,
    offset: Int = 0
)
```

默认请求显式发送 `tasks=include`，filter 逻辑与 URL mapping 留在 Core。

### 7.2 Filter-keyed pager

用 Core 中的 `GaryxRecentThreadFeeds` 包住两份
`GaryxHomeThreadListPager` + ordered ids + `isPrimed`/head failure。请求 ticket 是
`(filter, innerPagerTicket)`；Core 的 `completeRefresh` / `completeLoadMore` 接收 ticket
和 page ids，内部只修改 ticket 指向的 feed，而非读取可变的 current selection。
archive/delete/pin 等本地 surgery 通过 wrapper 同时 bump 两个 inner pager 的
`localMutationSequence`。`reset()` 清两份 ids/status、reset 两个 pager（epoch 递增）
并把 selection 置回 `.all`。

现有语义拆成两个明确 projection：

```text
allRecentThreadIds      = feeds[.all].orderedThreadIds
visibleRecentThreadIds  = feeds[selectedFilter].orderedThreadIds
```

`HomeProjectionActor` / `GaryxHomeThreadListSnapshot` 使用
`visibleRecentThreadIds`。snapshot 同时携带 `selectedRecentFilter` 与 selected feed 的
`primed/loading/headFailure/footer` presentation；否则两份 ids 恰好相同时 Picker
selection 不会发布，inactive feed 的 loading/error 也容易泄漏。以下消费者显式
使用 `allRecentThreadIds`：

- `persistRecentThreadsWidgetSnapshot`；
- `GaryxRecentThreadsWidgetSnapshotProjector` 输入；
- `garyxAutomationThreadOptions` 的 recent 输入。

不保留语义含糊、会随 Tab 变化的公共 `recentThreads` 计算属性；调用方必须选
`allRecentThreads` 或 `visibleRecentThreads`。

### 7.3 Refresh cadence

- load-more 只请求当前 selected feed；
- pull-to-refresh 等待 selected feed + pins（包括 pinned/selected summary backfill），
  保持用户可见完成边界；selected ticket 必须在这次 App-layer transaction 的最后
  一个 await 之后才 complete；
- `All` 是 Widget/Automation 的辅助 canonical feed：当 selected 为 `nonTask` 时，
  同一 refresh trigger 并发启动一个 coalesced `All` head refresh。辅助请求持有自己
  的 filter ticket、单独提交 summary/ids；它不等待 pins、不延长 pull-to-refresh
  spinner；
- 10s silent loop 刷新 selected feed，并确保 `All` head refresh 仍按既有 cadence
  更新；两个请求均是 projection-backed indexed page read；
- completion/failure 时重新判断 ticket.filter 是否仍 selected：inactive feed 的错误
  不 toast、不改 selected placeholder/footer；selected feed 沿用现有 user-action
  toast / background transient-status 策略。

## 8. 本地 mutation 规则

| 事件 | `All` feed | `Chats` feed | Pinned / 其它消费者 |
|---|---|---|---|
| archive/delete thread | 移除 id、两 pager `noteLocalMutation` | 同左 | 从 Pinned 移除；既有回滚覆盖全部 feed |
| 创建/首次发送普通 chat | 头部 upsert | 头部 upsert | summary cache 同步 |
| 新 task backing thread | 下一次 `tasks=include` head page 采用；有明确 task-create completion 的平台立即触发 All head refresh | 不插入、不本地判型；由 `tasks=exclude` page 决定成员集 | Task tree 自己更新 |
| title/runtime 变化 | 只更新共享 summary | 只更新共享 summary | 两 feed 顺序不因非 recency 字段重建 |
| pin/unpin | Recent feed 成员不变 | Recent feed 成员不变 | section projection 负责 pinned/recent 去重 |
| Gateway scope 切换 | reset | reset | selected filter 回到 `All` |

任何乐观 mutation race 都复用现有 local-mutation sequence 规则：旧 page completion
不能复活刚被 archive/delete 的 row。回滚必须恢复两个 feed 原顺序，而不是只恢复
当前可见 Tab。

复核事实：iOS 当前没有 task-create 管理入口，只有 conversation task-tree 读取；不能
为了表格对称添加一个无调用方的“本地 task upsert” API，也不能根据 title/body 猜
task。iOS 的 new-task Core 测试因此驱动交错的 server pages：All ticket 采用 task id，
Chats ticket 的满页不含该 id。Desktop task-create 若拿不到完整
`DesktopThreadSummary`，只触发 All head refresh，不构造假 summary。

pin/unpin 虽不改变两个 Recent id 数组，仍同时 bump 两个 pager 的 local mutation
sequence：当前 refresh transaction 还携带 pins/backfill 快照，旧 completion 不得把
pin surgery 前的组合状态写回。archive rollback token 包含两 feed 顺序/pager mutation
版本以及 Pinned/cache 的既有快照。

## 9. Loading、错误和 cache

- selected feed 未 primed、head refresh 进行中且无 rows：显示该 feed 的 skeleton；
- 未 primed 的首次 head refresh 失败：显示该 feed 的 unavailable presentation
  （Mac 带 Retry；iOS 保留 pull-to-refresh 并按 source 展示 toast/status），不能显示
  `No recent …`；
- 切换到有 cache 的 feed：立即显示 cache，head refresh 不清空 rows；
- head refresh 失败：保留 cache；user action 可 toast，background 只更新 transient
  status；
- load-more 失败：固定高度 footer + 显式 Retry，不自动重试 storm；
- inactive feed 的 loading/error 不改变当前 Tab 的 placeholder/footer；
- 空态由“当前 feed 已成功 primed 且 rows 为空”产生，不能把未加载或首次失败误判
  为空。Mac/iOS 的空态分别按 filter 使用 `No recent threads` / `No recent chats`。

## 10. 版本边界

本功能与已支持 `tasks` 参数的 Gateway 同版本发布。旧 Gateway 可能忽略未知 query
参数，客户端无法仅靠一页响应可靠判断其是否真正支持过滤。

因此：

- 不做分页后的本地过滤 fallback；
- 不根据当前页“刚好没有 task”推断能力；
- 如果未来要求 app 连接旧 Gateway 仍支持 `Chats`，应在 Gateway runtime/capability
  响应加入显式 feature bit，再决定禁用 Tab 或提示升级；不在本任务猜版本。

默认 `All` 在旧/新 Gateway 上都保持当前成员集，因此基础列表兼容不变。

## 11. 测试计划

### 11.1 既有 Gateway 契约

继续依赖并保留：

- `tasks=include|exclude|only` 行集与非法值 400；
- 过滤先于 limit/offset；
- filter 域内 `total`/`has_more`；
- count + page 同一显式 read transaction；
- `tasks=exclude` 零 ThreadStore 扫描 instrumented regression；
- 省略参数与 `tasks=include` 默认响应兼容。

### 11.2 Mac headless

- gary-client：两个 filter 的 URL、page mapping、非法 renderer input 拒绝；
- preload/IPC：参数透传但 URL 只由 main 构造；actual/expected Gateway scope 不符时
  拒绝，旧 scope result 不可提交；
- controller：默认 `All`、filter cache 隔离、offset 隔离、快速切换迟到响应、
  Gateway reset、成功空页 primed、首次失败不冒充空态、head merge、overlap load-more
  dedupe、失败 retry、close/reopen rail 保留 session state；
- mutation：archive/delete 同时移除两个 feed，失败回滚两个 feed；
- regression：`DesktopState.threads` 仍保留全量 `/api/threads` 数据，Workspace/Bot/
  Pinned 不因 Recent filter 丢行；
- component：tab accessible name/selected state/左右键、空态、cached refresh、footer。

### 11.3 iOS Core

- `GaryxRecentThreadFilter` wire mapping；默认 URL 显式 `tasks=include`；
- 两 feed pager 的 refresh/load-more ordering matrix；
- ticket filter/epoch 防串页与 Gateway stale completion；
- `visibleRecentThreadIds` 随 Tab 切换，`allRecentThreadIds` 不变；
- 成功空页 primed、首次失败 unavailable、selected/inactive loading/error/footer 隔离；
- `HomeProjectionActor`/legacy parity 在 filter 切换（即使两 feed ids 相同）时发布正确
  Picker selection 和 selected feed phase；
- Pinned task 在 `nonTask` 下仍进入 Pinned section；
- Widget snapshot 与 Automation options 始终读取 `All`；
- archive/delete/new chat/title/runtime/pin mutation table 逐项；new task 用交错 server
  pages 证明只进入 All，而不是测试不存在的 iOS task-create 调用方；
- 新 Core 文件同时进入 SwiftPM 与 Xcode project（如使用 xcodegen，校验 pbxproj）。

### 11.4 端到端与视觉验收

种子必须把 task 与 chat 交错，并让第一页中 task 数量足以暴露“客户端先分页再
过滤”的错误：

1. 首次进入 Mac/iOS：默认 `All`，task 可见；
2. 切 `Chats`：第一页满额来自 non-task 过滤域，无 task；
3. load-more：绝对顺序、无重复、`has_more` 正确；
4. 快速 `All → Chats → All`：无串页、无闪回；
5. pinned task 始终可见；
6. 在 `Chats` 下 archive chat，切回 `All` 不复活；
7. iOS Widget/Automation picker 仍含合法的 All feed rows；
8. Mac packaged app 用 CDP 检查 rail 布局、键盘与 light/dark；
9. iOS 检查 Dynamic Type、VoiceOver、Reduce Transparency 和真实 `List` 滚动。

## 12. 实现顺序

1. 共享契约：Mac/iOS client filter enum + page mapping + URL tests；
2. Mac：独立 Recent IPC/feed controller，不碰 `DesktopState.threads`；
3. Mac：segmented UI、两 feed pagination/error/mutation tests；
4. iOS Core：filter-keyed feeds/pagers、all vs visible projection、测试矩阵；
5. iOS app：segmented Picker、refresh/load-more wiring、Widget/Automation consumer
   拆分；
6. 跨平台 E2E、packaged Mac 视觉检查、iOS accessibility 检查；
7. 更新活文档中的 Recent UI 说明并删除实现期临时兼容符号。

该顺序先固定 wire 与状态所有权，再接 UI，避免出现“Tab 已可点但分页仍共享”或
“主页筛选误改 Widget/Automation”的中间态。

## 13. 2026-07-12 主线逐节核对记录

核对方法：从干净 worktree fetch `origin/main`，rebase 到 `2a22eb93d`，按 §3–§10
逐项搜索符号、读取实现与既有测试。下表中的“修订”已同步进正文，不是实现期 TODO。

| 节 | 结论 | 当前主线证据 | 不符处、理由与影响 |
|---|---|---|---|
| §3.1 Gateway | PASS | `routes.rs::parse_recent_thread_task_filter` / `list_recent_threads`；`garyx_db/mod.rs::RecentThreadTaskFilter` / `list_recent_threads_page_inner`；route/DB tests 覆盖 include/exclude/only、400、过滤后分页、同一 read transaction、零 store scan | 无生产契约缺口；上限仍为 200，响应仍含 threads/count/total/limit/offset/has_more。本任务不改 Gateway 生产代码。 |
| §3.2 Mac 现状 | PASS-WITH-DOC-FIX | `AppShell.tsx::recentThreadRows` 直接 map `desktopState.threads`；`threads.ts::fetchThreads` 默认 `/api/threads?limit=1000`；`store.ts::getDesktopStateFast` 使用同 route 的 200 行 fast page | 原文漏了 fast hydration，且“recent page”容易误读成 `/api/recent-threads`。已澄清；核心结论（不能改 DesktopState.threads 语义）不变。 |
| §3.3 iOS 现状 | PASS | `GaryxGatewayClient.listRecentThreads` 只发 limit/offset；`GaryxMobileModel` 的 `recentThreadIds` + `threadListPager`；`HomeProjectionActor`、`persistRecentThreadsWidgetSnapshot`、Automation 三处 `model.recentThreads` | 消费者清单成立；Widget projector 本身只消费传入 ids，真正需要改的是 model input；Automation 仍会把 recent 与 cached threads 去重合并，改名的是 recent 输入语义，不删除其既有 fallback。 |
| §4 状态/并发 | PASS-WITH-DESIGN-FIX | 现有 iOS pager已有 epoch ticket、双 in-flight、failure revision、local mutation sequence；Mac 尚无 Recent feed | 原 feed 无 `isPrimed`，无法区分成功空页与未加载；无 head failure，首次失败会被误画为空态；Mac IPC 未结构化证明 actual Gateway scope。均已补齐。主架构无需重做。 |
| §5 UI | PASS | Mac `ThreadConversationSidebar` 已有共享 header/list/row/resize seam；iOS `GaryxHomeThreadListView` 是 native flat `List`，Recent header 与 rows 已分开；仓库已有原生 `.segmented` Picker 先例 | Mac 需增加通用 accessory/footer/near-end seam；iOS 新 Picker 作为独立 flat row。没有与平台 UI 规则冲突。 |
| §6 Mac 落点 | PASS-WITH-DESIGN-FIX | shared API 位于 `shared/contracts/desktop-api.ts`；preload 在 `preload/index.ts`；IPC handler 集中于 `main/index.ts`; HTTP mapper 在 `main/garyx-client/threads.ts` | 原文只写 TS enum，不能满足非法 renderer input 拒绝；补 main runtime validation。原文未说明 hook 若随 rail unmount 会丢 session cache；改为 AppShell 常驻 owner。 |
| §7 iOS 落点 | PASS-WITH-DESIGN-FIX | Core pager在 `Sources/GaryxMobileCore/GaryxHomeThreadListPager.swift`；model IO 在 `GaryxMobileModel+ThreadList.swift`；projection capture/actor/reducer/store 已成链；gateway reset 调 `threadListPager.reset()` 并清 ids | 只替换 ids 不足以让 Picker/phase 经 actor snapshot 发布；已要求 snapshot 携带 selection + selected feed phase。辅助 All request 不得复用 current selection，也不得延长 pull spinner。 |
| §8 mutation | PASS-WITH-DESIGN-FIX | archive rollback 当前快照 `recentThreadIds`；pin 每次 `noteLocalMutation`；chat create 只插 summary cache，尚未插 recent id；iOS 无 task-create API/UI | 双 feed 要把 archive rollback 扩成两份顺序；chat create 补双 feed upsert。删除无调用方的“iOS 本地 task upsert”假设，new task 由显式 server filter page 验证/采用。 |
| §9 loading/error/cache | PASS-WITH-DESIGN-FIX | 当前 `recentPlaceholder` 仅用 rows + global `isLoadingThreads`；pager 的 `nextOffset == 0` 同时覆盖未 primed 与成功空页 | 现有单 feed 尚可借请求时序掩盖，拆 feed 后会直接串状态。已改为 selected feed 的 primed/loading/head-failure/footer projection，并定义首次失败 unavailable。 |
| §10 版本边界 | PASS | 全仓无 recent-task filtering capability bit；旧 Gateway 忽略 query 时客户端不能从单页证明支持 | 保持同版本发布、无本地 filtering fallback、无能力猜测。 |

### 13.1 取舍结论

- 不触发 `garyx-develop-a-feature` 的并发双设计重做：Gateway filter-before-page、两份
  feed、共享 summary cache、All/visible consumer split 四个核心前提均成立。
- 修订选择“补状态与归属证明”，而不是在 UI 层加判断：primed/failure/filter/ticket
  都进入纯状态机/Core，UI 只绑定 snapshot。
- 不为表格对称制造无调用方的 iOS task-create mutation；用真实 server filtered page
  覆盖 new-task 行集，避免客户端重新拥有 task 分类事实。
