# Mac / iOS Recent 列表 task 筛选 Tab

状态：产品方向已确认，待实现。

基线：`origin/main` @ `1a4230bde`。

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
`desktopState.threads` 映射为 `recentThreadRows`。该 slice 来自
`garyx-client/threads.ts::fetchThreads` 的 `/api/threads?limit=1000`，同时还服务
Workspace/Bot 分组、Pinned 修复和其它全量线程用途。

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
  nextOffset
  hasMore
  isRefreshingHead
  isLoadingMore
  loadMoreFailure
  epoch
  lastSuccessfulRefreshAt
}
```

Thread summary 仍按 `thread_id` 放在共享 cache 中；两个 feed 只保存顺序和分页
状态，避免复制 title、runtime、avatar 等 presentation 数据。

### 4.2 生命周期

- 冷启动：`selectedFilter = .all`。
- Gateway scope 变化：清空两个 feed、递增 epoch、重置为 `.all`。
- 同一 app session 内关闭/reopen Recent rail 或进入/退出会话：保留 Tab 与 cache。
- App 重启：不恢复上次 Tab；产品默认必须稳定为 `All`。
- Tab 切换：有 cache 立即渲染，并在后台做 head refresh；无 cache 才显示该 Tab
  自己的 loading placeholder。

### 4.3 并发与响应归属

每个请求都携带逻辑键：

```text
(gatewayScope, filter, feedEpoch, pagerTicket)
```

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

在 `desktop/garyx-desktop/src/main/garyx-client/threads.ts` 新增：

```ts
type RecentThreadTaskFilter = 'include' | 'exclude';

type DesktopRecentThreadsPage = {
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
window.garyxDesktop.listRecentThreads({ tasks, limit, offset })
```

输入只接受枚举值，main process 构造 query；renderer 不拼 Gateway URL。page size
固定为 100，Gateway 上限 200 不变。

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

### 6.3 Component seam

新增 `RecentConversationSidebar` 作为 domain wrapper，内部复用
`ThreadConversationSidebar`。共享 sidebar 只按需要增加通用 slot：

- `headerAccessory?: ReactNode`；
- `listFooter?: ReactNode` 或 near-end callback。

不把 filter state、API 参数或 Recent 文案塞进通用 Workspace/Bot sidebar。

## 7. iOS 架构落点

### 7.1 Core 类型与 Gateway client

在 `GaryxMobileCore` 增加：

```swift
public enum GaryxRecentThreadFilter: Equatable, Sendable {
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
`GaryxHomeThreadListPager` + ordered ids。请求 ticket 必须记录 filter；调用方完成
请求时把 page 提交给 ticket 指向的 feed，而非读取可变的当前 selection。

现有语义拆成两个明确 projection：

```text
allRecentThreadIds      = feeds[.all].orderedThreadIds
visibleRecentThreadIds  = feeds[selectedFilter].orderedThreadIds
```

`HomeProjectionActor` / `GaryxHomeThreadListSnapshot` 使用
`visibleRecentThreadIds`；以下消费者显式使用 `allRecentThreadIds`：

- `persistRecentThreadsWidgetSnapshot`；
- `GaryxRecentThreadsWidgetSnapshotProjector` 输入；
- `garyxAutomationThreadOptions` 的 recent 输入。

不保留语义含糊、会随 Tab 变化的公共 `recentThreads` 计算属性；调用方必须选
`all` 或 `visible`。

### 7.3 Refresh cadence

- load-more 只请求当前 selected feed；
- pull-to-refresh 等待 selected feed + pins，保持用户可见完成边界；
- `All` 是 Widget/Automation 的辅助 canonical feed：当 selected 为 `nonTask` 时，
  同一 refresh trigger 可并发启动一个 coalesced `All` head refresh，但它不延长
  pull-to-refresh spinner；
- 10s silent loop 刷新 selected feed，并确保 `All` head refresh 仍按既有 cadence
  更新；两个请求均是 projection-backed indexed page read；
- inactive feed 的错误不 toast；selected feed 沿用现有 user-action toast / background
  transient-status 策略。

## 8. 本地 mutation 规则

| 事件 | `All` feed | `Chats` feed | Pinned / 其它消费者 |
|---|---|---|---|
| archive/delete thread | 移除 id、两 pager `noteLocalMutation` | 同左 | 从 Pinned 移除；既有回滚覆盖全部 feed |
| 创建/首次发送普通 chat | 头部 upsert | 头部 upsert | summary cache 同步 |
| 创建 task backing thread | 头部 upsert | 不插入 | Task tree 自己更新 |
| title/runtime 变化 | 只更新共享 summary | 只更新共享 summary | 两 feed 顺序不因非 recency 字段重建 |
| pin/unpin | Recent feed 成员不变 | Recent feed 成员不变 | section projection 负责 pinned/recent 去重 |
| Gateway scope 切换 | reset | reset | selected filter 回到 `All` |

任何乐观 mutation race 都复用现有 local-mutation sequence 规则：旧 page completion
不能复活刚被 archive/delete 的 row。回滚必须恢复两个 feed 原顺序，而不是只恢复
当前可见 Tab。

## 9. Loading、错误和 cache

- 首次加载 selected feed 且无 rows：显示该 feed 的 skeleton；
- 切换到有 cache 的 feed：立即显示 cache，head refresh 不清空 rows；
- head refresh 失败：保留 cache；user action 可 toast，background 只更新 transient
  status；
- load-more 失败：固定高度 footer + 显式 Retry，不自动重试 storm；
- inactive feed 的 loading/error 不改变当前 Tab 的 placeholder/footer；
- 空态由“当前 feed 已成功 primed 且 rows 为空”产生，不能把未加载误判为空。

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
- preload/IPC：参数透传但 URL 只由 main 构造；
- controller：默认 `All`、filter cache 隔离、offset 隔离、快速切换迟到响应、
  Gateway reset、head merge、load-more dedupe、失败 retry；
- mutation：archive/delete 同时移除两个 feed，失败回滚两个 feed；
- regression：`DesktopState.threads` 仍保留全量 `/api/threads` 数据，Workspace/Bot/
  Pinned 不因 Recent filter 丢行；
- component：tab accessible name/selected state/左右键、空态、cached refresh、footer。

### 11.3 iOS Core

- `GaryxRecentThreadFilter` wire mapping；默认 URL 显式 `tasks=include`；
- 两 feed pager 的 refresh/load-more ordering matrix；
- ticket filter/epoch 防串页与 Gateway stale completion；
- `visibleRecentThreadIds` 随 Tab 切换，`allRecentThreadIds` 不变；
- Pinned task 在 `nonTask` 下仍进入 Pinned section；
- Widget snapshot 与 Automation options 始终读取 `All`；
- archive/delete/new chat/new task/title/runtime/pin mutation table 逐项；
- selected/inactive feed 的 skeleton、empty、error/footer 状态隔离；
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
