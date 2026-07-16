# iOS 线程列表统一化 —— 设计 v2（待复审）

作者：Gary
日期：2026-07-17
基线：main `3dff1111a`

修订记录：
- **v2**（2026-07-17）：按 #TASK-2362 评审（FAIL，F-01..F-11）全面修订。核心变化：① 文件夹列表数据源改为 **`thread_meta` 投影的 scoped keyset 新路由**，不再动 `/api/recent-threads`（F-01/F-03 根治：成员集保持今天的"全量 live 线程"，cursor 内嵌 scope digest）；② **砍掉 `thread_ids` 批量点查 API**，bot 摘要 hydration 复用既有 `/api/threads/:id` 点查 + Core 有界缓存（F-02 消解）；③ `model.threads` 消费者按评审盘点**穷尽列出**并定义两层所有权（summaryById 缓存 / scope membership store）（F-04）；④ feed 注册表增加**实例代际 + 淘汰策略 + mutation fan-out**规范（F-05）；⑤ 手势改为 **capability 模型**，不再"无条件全集"（F-06）；⑥ 澄清"共享的是 DTO/row/capability/presentation 层，membership feed 保持领域独立"（F-07）；⑦ `/api/threads` 端点确认保留（desktop/CLI 在用），仅 iOS 全量循环退役（F-08）；⑧ bot `/threads` 对齐目标明确为 Chats（nonTask）filter（F-09）；⑨ 给出 iOS/gateway 独立发版的版本偏斜合同：新路由 404 → 显式升级空态，不做静默 fallback 双路径（F-10）；⑩ 修正 §1 三处事实偏差（F-11）。
- v1：初版，#TASK-2362 评审 FAIL。

## 0. 需求（老板原话拆解）

1. iOS app 里多处线程列表**割裂**：首页 Recent 是一套，抽屉点 workspace（"文件夹"）进入的列表是另一套——数据不同步、观感不一致。
2. 点文件夹进列表**特别卡**。
3. 文件夹列表**不支持长按归档**等首页已有的手势。
4. 多处列表（首页 / 文件夹 / bot 会话 / automation 触发列表）应当**组件同构、数据同源、手势一致**。
5. bot 渠道输出的文本线程列表也要与 app 列表**同源**。

## 1. 现状盘点（v2 按评审核实修正）

### 1.1 四套数据路径并存

| 列表 | 端点 | 投影 | 分页 | 关键差异 |
|---|---|---|---|---|
| 首页 recent | `/api/recent-threads` | `recent_threads`（keyset by `activity_seq`） | cursor | 完整 `RecentThreadRecord` |
| 首页 Favorites filter | `/api/thread-favorites/snapshot`（+辅助 recent feed） | favorites + join `recent_threads` | snapshot 整替 | **不走** `/api/recent-threads` 主路径（v1 此处有误） |
| workspace 文件夹 & bot 摘要源 | `/api/threads?limit=1000&offset=…` **循环拉全量**；另有逐 ID `/api/threads/:id` 修补路径（`GaryxMobileModel+StateSync.swift:49-93`） | `thread_meta` | offset | 用 `label`（非 `title`） |
| automation triggered threads | `/api/automations/{id}/threads` | automation run 投影 | offset，单页 50 无 load-more | camelCase、内嵌 `thread` 摘要 |
| bot 渠道 `/threads` 文本列表 | router `RecentThreadPageReader` | `recent_threads`（offset 读法，固定 `RecentThreadFilter::Exclude`） | offset 文本翻页 | 仅 4 字段；成员集 = Home 的 **Chats（nonTask）** filter，非 All（`local_commands.rs:64-69`） |

### 1.2 两个投影的真实成员集（#TASK-2362 F-01 核实结论，照录）

- `recent_threads`：排除 hidden / `exclude_from_recent` / `generated_thread`，**不要求有时间或消息**（`recent_thread_projection.rs:80-170`）。
- `thread_meta`：收录**所有 canonical 线程**，只记录 `default_list_hidden`（用于默认列表隐藏过滤）。
- 归档在同一事务删除 record + 两投影 + pin + favorite（`garyx_db/mod.rs:1291-1329`）→ 归档线程在两边都不存在。
- 实测差集：3101 visible `thread_meta` vs 3025 recent，差 81 条全部是 Recent exclusion（其中 77 条 generated automation 线程）。

**推论**：文件夹列表今天显示的成员集 = 全量 live 线程（含 excluded/generated）；若换到 `recent_threads` 会让这 81 类线程从文件夹消失。v2 的裁决见 §2.1。

### 1.3 客户端状态承载割裂

- 首页独享：`GaryxHomeThreadListStore` + `HomeProjectionActor` + `GaryxHomeThreadListPager`（epoch 票据纯状态机）+ `GaryxRecentThreadFeeds`（stale-while-refresh）+ sections cache + native `List`。
- workspace / bots / automation drilldown 不用这套：父 section `@EnvironmentObject` 观察整个 model（这是整段重建的真因；行内 `GaryxSidebarThreadButton` 持有的是刻意不观察的普通引用——v1 归因有误），从 `model.threads` 每渲染现算分组，渲染在 `GaryxPanelScaffold` 的 `ScrollView + VStack + ForEach`，无懒加载无复用。

### 1.4 卡顿根因（评审已逐条核实属实）

1. 进入 drilldown `.task` 同步 `await` 全量循环 `/api/threads?limit=1000`（`GaryxMobileModel+ThreadList.swift:708-739`）。
2. `ScrollView + VStack + ForEach` 一次性构建全部行（`GaryxMobileComponents.swift:294-306`、`GaryxMobileSidebarViews.swift:1719-1752`）。
3. 父 section `@EnvironmentObject model`：model 任一 publish → 整段 ForEach 重算。
4. 分组无记忆化，每渲染重跑全量 `Dictionary(grouping:)` + 排序。

### 1.5 行组件与手势割裂

- 视觉行 `GaryxSidebarThreadRowView` 共享；外层包装两套（首页 `GaryxHomeThreadButton`：equatable/预算 row/live 时间戳/长按菜单/拖拽重排 vs 其余 `GaryxSidebarThreadButton`：即时构造/静态时间戳/仅 swipe+归档确认框）。
- 首页行已有能力裁剪先例：automation target 关闭 Archive（`GaryxHomeThreadListPresentation.swift:742-779`），服务端也拒绝 active/automation-managed 归档（`routes.rs:3392-3442`）。

## 2. 设计原则与关键裁决

1. **根因修复，不做客户端 workaround**。
2. **两投影各司其职，一份呈现栈**：
   - `recent_threads` = **recency feed**（首页 + bot `/threads`），语义、端点、成员集**完全不动**；
   - `thread_meta` = **全量 live 线程摘要清单**（文件夹列表、逐 ID 摘要点查、picker 候选集）。
   - 共享的是**摘要 DTO、row wrapper、capability/action 层、presentation store 基座**；membership feed 按领域独立（recent feed / workspace feed / bot endpoint 派生 / automation run 页）——这是对 F-07 矛盾的正面裁决。
3. **条件查询走 SQL 投影**：workspace 过滤 = `thread_meta` 上的 scoped keyset 静态 SQL 分支 + 索引；不加扫描、不加读时修复。
4. **成员集裁决（F-01）**：文件夹列表成员集**保持今天的"全量 live 线程"**（含 excluded/generated），排序保持 `updated_at` 降序（今天的内存排序语义，服务端化 + `thread_id` tiebreak）。理由：不让任何现有行静默消失；"同步感"由同事务投影 + 统一刷新/mutation 机制保证，而非强行同成员集。若产品后续想让文件夹≡Recent 过滤，是该路由上的一个过滤参数翻转，不影响本架构。
5. **Mac app 是 IA 真相源**：不发明新概念。

## 3. 服务端设计

### 3.1 新路由：`GET /api/thread-summaries`（`thread_meta` scoped keyset）

**参数**：
- `workspace_dir=<绝对路径>`（可选；精确匹配；workspace 身份 = 绝对路径字符串）
- `tasks=include|exclude|only`（可选，默认 `include`；语义与 recent-threads 现有参数一致）
- `cursor=<opaque>`（可选）
- `limit=<1..100>`（可选，默认 30；非法 400）

**成员集**：全部 canonical live 线程，沿用 `/api/threads` 现行 `default_list_hidden` 过滤谓词。**排序**：`updated_at DESC, thread_id DESC`（确定性 tiebreak）。

**响应信封**：
```json
{
  "threads": [ThreadSummaryRow…],
  "next_cursor": "…|null",
  "has_more": false
}
```
`ThreadSummaryRow` 字段：`thread_id`、`title`（**统一用 title 命名**，服务端从 `thread_label` 映射，消灭 label/title 割裂）、`workspace_dir`、`thread_type`、`provider_type`、`agent_id`、`created_at`、`updated_at`、`message_count`、`last_user_message`、`last_assistant_message`、`last_message_preview`、`recent_run_id`、`active_run_id`、`worktree`、`excluded_from_recent`（bool，见 §3.2）。

**cursor 合同（F-03 根治）**：opaque base64 JSON `{v:1, scope, tasks, updated_at, thread_id}`，`scope` = `sha256(workspace_dir ?? "")` 摘要。请求参数与 cursor 内嵌 scope/tasks 不匹配 → 400；cursor 解析失败 → 400。本路由 cursor 与 `/api/recent-threads` 的 v1 cursor 互不相干（不同路由不同 schema，不存在混用面）。

**实现模式**：镜像 bot-recent-threads 改版确立的 filtered-page 模式——count+page 同分支、同显式读事务、**静态 SQL 分支**（workspace×tasks 组合枚举），复合索引 `(workspace_dir, updated_at DESC, thread_id)` + tasks 分支 partial index，全分支 `EXPLAIN QUERY PLAN` 测试钉住命中。

**与 `/api/threads/:id` 的关系**：行形状对齐（同一 DTO 序列化）；`:id` 点查继续作为逐线程摘要读路径。

### 3.2 `thread_meta` 增列：`excluded_from_recent`

- 写路径同事务派生（与 `recent_thread_projection.rs:80-170` 同一排除谓词：`exclude_from_recent || generated_thread`；hidden 已有 `default_list_hidden`）。
- 一次性版本化 cutover `thread_meta_excluded_flag_v1`（boot import 后运行、落 durable marker，遵守既有 one-shot cutover 模式），从 record body 回填存量行。
- 用途：客户端 capability 模型判定 `canFavorite`（见 §4.4）；不改变任何既有查询的成员集。

### 3.3 不动的部分（显式声明）

- `/api/recent-threads`：**零改动**（参数、信封、cursor、成员集全部不变）。v1 曾提的 `workspace_dir`/`thread_ids` 参数全部撤销。
- `/api/threads`（offset 列表）：**端点保留**——desktop 主进程/desktop web/CLI/路由测试都在消费（F-08 盘点已确认）。仅 iOS 停用其全量循环用法。既有响应信封不变。
- `/api/thread-favorites/*`、pins：不动。excluded 线程可被 favorite 但不出现在 snapshot 的既有服务端行为**本次不改**，由客户端 capability 门禁使其不可达（§4.4）；服务端根治（snapshot 改 join `thread_meta`）列为后续独立小案（§9.1）。
- bot 渠道 `/threads`：不动。对齐断言测试钉住：其成员集/排序 ≡ `/api/recent-threads` 的 `tasks=exclude`（即 Home **Chats** filter，不是 All/Favorites——F-09 修正），title 语义一致。
- automation `/api/automations/{id}/threads`：端点不动（run 历史语义），客户端接已返回的 `hasMore` 补 load-more。

## 4. 客户端设计（GaryxMobileCore + App）

### 4.1 两层所有权模型（F-04 根治的地基）

- **`GaryxThreadSummaryCache`（Core，新）**：`summaryById` 有界缓存（LRU，容量 ~500），唯一的"按 ID 取摘要"真相源。写入源：各 feed 页回填、`/api/threads/:id` 点查、SSE/runtime 增量。所有"手里有 id 要摘要"的消费者读它。
- **scope membership store**：每个列表面只持有**成员顺序 + 分页/过渡状态**（行内容从 summaryById 解引用）。run-state 等 runtime overlay 继续走既有 runtime 合并路径，落到 summaryById 的 overlay 层。

### 4.2 membership provider 抽象（F-07 裁决落地）

```
protocol GaryxThreadListMembershipProvider {
    // 输出规范化的成员页快照（有序 thread_id 列 + 分页态 + 伴随摘要回填）
}
```
四个实现，领域各自独立：
1. **recent**：现 `GaryxRecentThreadFeeds`/pager，语义零改动；
2. **workspace(path)**：新 feed，走 `/api/thread-summaries?workspace_dir=…`，复用 `GaryxHomeThreadListPager` 纯状态机（epoch 票据/双轨道/localMutationSequence 不动）；
3. **botConversations(groupId)**：派生自 bot console/endpoints（**不是线程分页**，保持现有派生源），摘要 hydration 改为 summaryById + 逐 ID `/api/threads/:id` 补缺（既有修补路径的系统化，替代 `model.threads` 查表）；
4. **automationThreads(id)**：既有端点 + `hasMore` load-more 页驱动。

通用 **presentation/action store**（由 `GaryxHomeThreadListStore` 泛化而来）消费任一 provider 的规范化快照：快照/过渡态/sections cache/off-main 投影（`HomeProjectionActor` 泛化）全复用。`.recent(all)` 实例保留 pinned 段 + 拖拽重排（全局概念只在首页出现）。

### 4.3 feed 注册表：实例代际 + 淘汰 + mutation fan-out（F-05 根治）

- **实例代际**：feed 实例携带单调 `instanceID`；所有 ticket 增加 instanceID 字段，completion 校验"当前注册表里同 scope 的实例 == ticket 实例"，不匹配即丢弃（防 ABA 回写）。Core 补 ABA 回归测试（淘汰→重建→旧 completion 到达）。
- **淘汰策略**：workspace feed LRU 上限 4；淘汰时 cancel 在途请求（cancel + completion 双保险）。**被淘汰的 scope 重新进入 = 冷加载**（v1 "逐出后 stale-while-refresh"说法撤销——被逐出即无快照，不另建 snapshot cache）。recent 三 feed（all/nonTask/favorites）照旧常驻。
- **mutation fan-out**：新增 Core `GaryxThreadMutationHub`：pin/unpin/favorite/unfavorite/archive 的成功结果发布 `(threadId, mutation)`；所有存活 store 订阅并应用（成员移除/行更新/退场过渡），summaryById 同步更新。首页现有 archive/pin transition 逻辑（`GaryxMobileModel+Bots.swift:248-323`、`+ThreadPersistence.swift:49-170`）重构为该 hub 的订阅者，行为守恒（测试钉住首页现行为不变）。

### 4.4 行为能力模型（F-06 根治）

```
struct GaryxThreadRowCapabilities {
    canOpen, canPin, canFavorite, canArchive: Bool
    archiveStrategy: .thread | .botEndpoint | .none
}
```
- Core 单一派生函数（配 SwiftPM 测试），输入：摘要 flags（`excluded_from_recent`、`thread_type`）、automation target 集、active run 态、bot entry 能力（openable 与否）。
- 规则：`excluded_from_recent` → `canFavorite=false`（favorite 后不可见的服务端组合从 UI 不可达）；automation target → `canArchive=false`（保持现状）；active run 归档拒绝保持服务端裁决 + 客户端预门禁；bot 会话行 `archiveStrategy=.botEndpoint`（走 `archiveBotConversationEndpoint`）；不 openable 占位行全关。
- **首页也切到同一派生函数**，用 characterization 测试证明首页现行为逐条不变。
- 统一 wrapper 按 capabilities 渲染菜单/swipe 项——"手势全集"的准确表述是：**组件一个、动作按能力裁剪、能力规则全端一致**。

### 4.5 行组件与容器

- 合并 `GaryxHomeThreadButton` + `GaryxSidebarThreadButton` → `GaryxThreadListRowButton`：equatable、预算 row 输入（不持 model 引用，action 闭包注入）、live 时间戳（TimelineView）全列表生效、长按菜单 + swipe 按 capabilities 渲染、`openSource` 参数化（首页 `.replace` / drilldown `.current`，openThread 仍是唯一打开路径）。
- **新建 sibling 列表容器 `GaryxListPanelScaffold`**（内嵌 native `List`），线程列表面全部迁入；`GaryxPanelScaffold`（ScrollView）保留给非列表 panel——F-07 指出的嵌套问题以 sibling scaffold 解决（§9.3 裁决）。
- drilldown 各 section 改为观察自己的窄 store，model 只做 action 入口，摆脱 `@EnvironmentObject` 整体重建。

### 4.6 `model.threads` 消费者穷尽迁移表（F-04；实现批次逐行核销）

| 消费者 | 位置 | 迁移去向 |
|---|---|---|
| Home 投影/drawer 发布 | `GaryxMobileModel.swift:105-109`、`+Presentation.swift:141-188` | recent feeds（已有）+ summaryById，核实后摘除对 `threads` 的读 |
| workspace 分组 `sidebarWorkspaceThreadGroups` | `GaryxMobileSidebarViews.swift:1085` | `.workspace(path)` membership store |
| bot 摘要查表 `sidebarThreadSummary` | `+Presentation.swift:450` | summaryById（+逐 ID 点查补缺） |
| open/restore/deep-link 缓存 | `+AgentsWorkspaces.swift:94-199,256-275` | summaryById + `/api/threads/:id` |
| widget snapshot/标题同步/后台 reconcile | `+ThreadList.swift:535-599,892-899` | recent feed 页（widget 本就以 recent 为源）+ summaryById |
| run-state 合并 | `+ThreadRunState.swift:115-120` | runtime overlay → summaryById |
| queued composer fallback | `+Composer.swift:365-379` | summaryById |
| workspace 建议、pinned/recent 映射 | `+Presentation.swift:584-615` | recent 页 + summaryById |
| automation 创建/编辑/picker | `GaryxMobileAutomationViews.swift:384-385,518-520,999-1019` | picker 换 `/api/thread-summaries`（无 workspace 过滤 = 全量候选集分页），不再依赖"已加载 Recent 页 + 全量 dump"拼凑 |
| `refreshWorkspaceAndBotThreads()` 全量循环 | `+ThreadList.swift:708-739` | **删除** |

迁移完成后 `model.threads` **整字段删除**；S3 评审以"grep 零残留读点"为验收项，发现残留即 FAIL。

## 5. 行为变化（有意的，v2 修订）

1. **文件夹列表成员集不变、排序语义不变**（`thread_meta` 全量 live + `updated_at` 降序服务端化），变的是取数方式（keyset 分页替代全量 dump）与新鲜度机制（stale-while-refresh + mutation fan-out）——"不同步"由此消失，且无任何行静默消失。
2. **手势增强**：文件夹/bot/automation 列表获得长按菜单（Pin/Favorite/Archive，按 capabilities 裁剪）；excluded 线程不提供 Favorite（新增门禁，属修复）。
3. **automation 列表可翻页到底**（原只显示前 50）。
4. **drilldown 时间戳变 live**（原静态）。
5. **版本偏斜（F-10 合同）**：新 iOS + 旧 gateway → `/api/thread-summaries` 404 → 文件夹列表与 automation picker 增强模式显示**显式"网关版本过旧，请升级"空态**（不做静默 fallback、不保留旧全量 dump 双路径；老板既定原则：不做旧网关兼容设计，但偏斜必须**显式可见**而非静默坏掉）。bot 摘要 hydration、首页、favorites、automation 列表等均走既有端点，旧 gateway 下不受影响。picker 在 404 时降级为仅 recent 已加载页 + 同一升级提示。

## 6. 明确不做

- 不改桌面端列表（另案）。
- 不动 `/api/recent-threads`、`/api/threads`、favorites/pins 端点的任何请求/响应契约。
- 不给 bot 文本列表加 workspace 过滤/新命令。
- 不动 pinned 全局语义（workspace scope 无独立 pin 段）。
- 不动 openThread 路由与转场。
- favorites snapshot 的服务端 join 换源（§9.1）本次不做。

## 7. 交付切片

| 切片 | 内容 | 验证 |
|---|---|---|
| S1 gateway | §3.1 新路由（keyset+scope cursor+静态 SQL 分支+索引）；§3.2 增列+cutover；§3.3 bot `/threads`≡Chats 对齐断言测试 | `cargo test -p garyx-gateway --lib`：全分支 query-plan、cursor scope 不匹配 400、信封、cutover 幂等/回填、既有 `/api/threads`/recent-threads 信封不变 characterization |
| S2 Core | §4.1 summaryById + §4.2 provider 抽象/store 泛化 + §4.3 代际/淘汰/fan-out + §4.4 capabilities | SwiftPM：ABA 回归、LRU 淘汰取消、fan-out 各 store 应用、capabilities 派生全表、首页行为守恒 characterization |
| S3 App | §4.5 行统一/List scaffold/窄 store + §4.6 迁移表逐行核销 + 全量 dump 删除 + 404 升级空态 | xcodebuild 构建 + SwiftPM headless（真实捕获数据驱动）；`model.threads` grep 零残留；手势 capability 清单逐面核对；xcodegen pbxproj 同步提交 |
| S4 清理 | iOS 侧 `label` 兼容层删除（限不再消费 `/api/threads` 列表 DTO 的层）、死代码清扫、旧 wrapper 删除 grep 断言 | 全量 grep 盘点 + tier1 |

每切片独立评审到 PASS 再进下一片；S1 先行。

## 8. 验收标准（对应需求逐条）

1. 首页与文件夹列表：同事务投影供数 + 统一 mutation fan-out + 统一刷新机制，同一线程两处状态一致（"不同步"消失）。
2. 进入文件夹 = 一次 keyset page 请求（网络层断言），native List 懒加载；千线程 workspace 进入首屏行构建数 ≤ 首屏 + 预取窗口。
3. 文件夹/bot/automation 列表长按菜单 + swipe 与首页同组件同规则（capability 裁剪一致）。
4. 四个列表面共用：摘要 DTO、`GaryxThreadListRowButton`、capabilities 派生、presentation store 基座（membership provider 按领域独立）——旧 wrapper 删除可 grep 断言。
5. bot `/threads` 输出 ≡ 首页 Chats filter 成员集/排序（测试钉住）。

## 9. 开放问题（复审请裁决）

1. favorites snapshot 服务端 join 换 `thread_meta`（让 excluded 线程 favorite 后可见），作为后续独立小案的优先级——本设计仅以 capability 门禁使该组合 UI 不可达，是否可接受为过渡态？
2. `GaryxThreadSummaryCache` 容量与逐出策略（~500 LRU）是否需要按 widget/composer 等关键消费者加 pin 位？
3. automation picker 换全量候选集分页后是否需要配搜索框（候选 3000+ 时纯翻页可用性）——倾向 S3 内顺手加本地前缀过滤，复审裁决。
