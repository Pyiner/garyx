# iOS 线程列表统一化 —— 设计 v1（待评审）

作者：Gary
日期：2026-07-17
基线：main `3dff1111a`

## 0. 需求（老板原话拆解）

1. iOS app 里多处线程列表**割裂**：首页 Recent 是一套，抽屉点 workspace（"文件夹"）进入的列表是另一套——数据不同步、观感不一致。
2. 点文件夹进列表**特别卡**。
3. 文件夹列表**不支持长按归档**等首页已有的手势。
4. 多处列表（首页 / 文件夹 / bot 会话 / automation 触发列表）应当**组件同构、数据同源、手势一致**。
5. bot 渠道输出的文本线程列表也要与 app 列表**同源**。

## 1. 现状盘点（已实测走读，文件行号基于基线）

### 1.1 四套数据路径并存

| 列表 | 端点 | 投影 | 分页 | 关键字段差异 |
|---|---|---|---|---|
| 首页 recent（含 favorites filter） | `/api/recent-threads` | `recent_threads`（keyset by `activity_seq`） | cursor | 完整 `RecentThreadRecord`：`title`/`workspace_dir`/`thread_type`/`run_state`/`activity_seq` 等 |
| workspace 文件夹 & bot 摘要源 | `/api/threads?limit=1000&offset=…` **循环拉全量** | `thread_meta` | offset | 用 `label`（非 `title`），无 `activity_seq`/`store_incarnation_id` |
| automation triggered threads | `/api/automations/{id}/threads` | automation run 投影 | offset，**单页 50 无 load-more** | camelCase、内嵌 `thread` 摘要 |
| bot 渠道 `/threads` 文本列表 | router `RecentThreadPageReader` | `recent_threads`（offset 版读法） | offset 文本翻页 | 仅 4 字段（thread_id/title/preview/last_active_at） |

### 1.2 客户端状态承载割裂

- 首页独享一整套打磨过的基础设施：`GaryxHomeThreadListStore` + `HomeProjectionActor`（主线程外算 section）+ `GaryxHomeThreadListPager`（epoch 票据纯状态机）+ `GaryxRecentThreadFeeds`（按 filter 分 feed、stale-while-refresh）+ `GaryxHomeThreadSectionsCache`（记忆化）+ native `List`（cell 复用）。
- workspace / bots / automation 三个 drilldown 全部**不用**这套：直接 `@EnvironmentObject` 观察整个 model，从 `model.threads` 现算分组（`sidebarWorkspaceThreadGroups` 每次渲染重跑 `Dictionary(grouping:)`），渲染在 `GaryxPanelScaffold` 的普通 `ScrollView + VStack + ForEach`，无懒加载无复用。

### 1.3 行组件与手势割裂

- 视觉行 `GaryxSidebarThreadRowView` 是共享的，但外层包装分两套：
  - 首页 `GaryxHomeThreadButton`：equatable、store 预算好的 row、live 时间戳（TimelineView）、长按上下文菜单（Pin/Favorite/Archive）、pinned 拖拽重排。
  - 其余 `GaryxSidebarThreadButton`：持整个 model 引用、view body 里即时构造 presentation、静态时间戳、仅 swipe（Pin/Archive）+ 长按弹归档确认框，**无 Favorite、无菜单**。

### 1.4 卡顿根因（"点文件夹特别卡"）

1. 进入 drilldown 的 `.task` 同步 `await` `refreshWorkspaceAndBotThreads()`：**循环 `/api/threads?limit=1000` 直到 total**，全部线程 body 灌进 `model.threads`。
2. 无懒加载：普通 `ScrollView + ForEach` 一次性构建全部行。
3. `@EnvironmentObject model`：model 任一 publish（10s 刷新循环、run state、widget 持久化）→ 整段 ForEach 重算重建。
4. 分组无记忆化，每次渲染重算全量 grouping + 排序。

## 2. 设计原则

1. **根因修复，不做客户端 workaround**：数据源统一到服务端一个查询面，而不是在客户端给旧 dump 加缓存补丁。
2. **一份列表栈**：数据（scoped feed）→ 状态（store）→ 行（统一 wrapper）→ 手势（统一 action set），所有线程列表面复用；首页现有栈是基线，泛化而非重写。
3. **条件查询走 SQL 投影**（repository-contracts 铁律）：workspace 过滤加在 `recent_threads` 投影的 keyset 查询上，不引入新的扫描。
4. **Mac app 是 IA 真相源**：不发明新概念，只统一现有列表面的实现。

## 3. 服务端设计

### 3.1 `/api/recent-threads` 增加 scope 过滤

- 新增查询参数 `workspace_dir=<绝对路径>`（精确匹配，workspace 身份就是绝对路径字符串，遵守 workspace-paths 契约）。
- keyset 查询在 `recent_threads` 投影上加 `WHERE workspace_dir = ?` 分支；**静态 SQL 分支**（沿用 bot-recent-threads 改版确立的模式：filtered count+page 同分支、同显式读事务），新增 partial/composite index `(workspace_dir, activity_seq DESC)`。
- 省略参数时成员集、排序、分页、响应信封**全部不变**（Mac/桌面零行为变化）。
- 与既有 `tasks=include|exclude|only` 过滤正交组合。

### 3.2 批量摘要点查：`thread_ids` 过滤

- `/api/recent-threads` 新增 `thread_ids=<id,id,…>`（上限如 50，超限 400），对投影做**点查**（`WHERE thread_id IN (…)`，主键命中，非扫描），返回同形状行；与 `workspace_dir`/cursor 互斥（400）。
- 用途：bot 会话 drilldown、automation picker 等"手里有 thread id 要摘要"的消费者，替代现在依赖 `model.threads` 全量 dump 的 hydration。

### 3.3 `/api/threads` 全量 dump 用法退役

- iOS 不再调用 `refreshWorkspaceAndBotThreads()` 的全量循环。
- 实现阶段盘点 `/api/threads` 的其余消费者（桌面/CLI/测试）：若 mobile 是最后一个消费者则整端点删除；否则仅 mobile 侧退役，端点保留并在盘点结论里说明。**不留双路径**。
- `label` vs `title` 命名割裂随该用法退役自然消亡（客户端 `GaryxThreadSummary` 的 label 兼容层在最后清理批次删除）。

### 3.4 bot 渠道文本列表

- 已与首页同投影（`recent_threads`），**数据同源已成立**；文本 offset 翻页是渠道呈现形态的合理选择，保留。
- 本次仅做对齐断言：排序（`activity_seq`）、成员集（tasks 过滤语义）、title 语义与 app 一致，加测试钉住；不扩字段、不加 workspace 过滤（无产品需求）。

### 3.5 automation triggered threads

- 数据本质是 run 历史（带 `startedAt/finishedAt`），**保留** automation run 投影与端点；客户端补 load-more（端点已返回 `hasMore`，纯客户端接线）。

## 4. 客户端设计（GaryxMobileCore + App）

### 4.1 Core：scope 化 feed 层

- 新增 `GaryxThreadListScope`（Core，纯值类型）：`.recent(filter)`（现 all/nonTask/favorites）、`.workspace(path)`。
- `GaryxRecentThreadFeeds` 泛化为 `GaryxThreadListFeeds`：scope → feed（`GaryxHomeThreadListPager` 原样复用——它本就是纯状态机，epoch 票据/双轨道/localMutationSequence 语义不动）。workspace feed 用 LRU 上限（如 4 个）防止无限增长，被逐出的 feed 下次进入重走 stale-while-refresh。
- favorites 特殊拉取路径（snapshot + 辅助 feed）不动，只是归入同一 registry。

### 4.2 Core：store 与 presentation 泛化

- `GaryxHomeThreadListStore` 泛化为按 scope 实例化的 `GaryxThreadListStore`：
  - `.recent(all)` 保留 pinned 段 + 拖拽重排状态机（全局概念，只在首页出现）；
  - `.workspace(path)` 为平铺列表（无 pinned 段），复用同一快照/过渡态机制。
- `GaryxHomeThreadRow` 预算行模型、`HomeProjectionActor` 主线程外投影、sections cache 全部按 scope 泛化复用（改名去 Home 前缀，保留类型别名过渡一个批次）。
- 所有纯逻辑（scope、pager 组合、投影 reducer）留在 `GaryxMobileCore`，配 SwiftPM 测试。

### 4.3 App：行组件与手势统一

- 合并 `GaryxHomeThreadButton` 与 `GaryxSidebarThreadButton` 为单一 wrapper（暂名 `GaryxThreadListRowButton`）：
  - equatable + 预算 row 输入（不持 model 引用，action 以闭包注入）；
  - live 时间戳（TimelineView）全列表生效；
  - **手势全集**：长按上下文菜单（Pin/Unpin、Favorite/Unfavorite、Archive）+ swipe（Pin/Archive），所有线程行一致；
  - `openSource` 参数化（首页 `.replace`、drilldown `.current`，维持现状路由语义；openThread 仍是唯一打开路径）。
- bot 会话行的归档语义保留 endpoint 版（`archiveBotConversationEndpoint`）；行动作集按 entry 能力裁剪（不 openable 的占位行无线程动作），但**组件同一个**。

### 4.4 App：drilldown 容器改造

- workspace / bots / automation 三个 drilldown 列表从 `ScrollView + VStack + ForEach` 换 **native `List`**（与首页同基线，cell 复用）。
- 摆脱 `@EnvironmentObject model` 整体订阅：各 section 改为观察自己的窄 store（4.2 的 scope store / bot 会话摘要 store），model 只做 action 入口。
- workspace drilldown 数据改走 `.workspace(path)` feed：进入即首页同款体验——stale 快照立即上屏、后台刷新、near-tail 预取分页；**不再有全量 dump**。
- bot 会话 drilldown：条目仍派生自 bot console/endpoints（这是 endpoint 列表不是线程列表，数据源不变），线程摘要 hydration 改用 §3.2 批量点查 + Core 内小缓存，替代 `model.threads` 查表。
- automation drilldown：接 `hasMore` load-more，行换统一 wrapper。

### 4.5 `model.threads` 消费者迁移清单（实现批次逐一处理）

| 消费者 | 迁移去向 |
|---|---|
| `sidebarWorkspaceThreadGroups`（workspace 分组） | `.workspace(path)` scoped feed |
| `sidebarThreadSummary`（bot 会话摘要查表） | §3.2 批量点查 + 缓存 |
| `garyxAutomationThreadOptions`（automation 线程选择器） | recent feed 页数据（`allRecentThreads` 已有） |
| `refreshWorkspaceAndBotThreads()` 全量循环 | 删除 |

迁移完成后 `model.threads` 若无剩余消费者则整字段删除；有剩余则列明并逐个评估，不允许"顺手留着"。

## 5. 行为变化（有意的，需评审确认）

1. **文件夹列表成员集换源**：`thread_meta` 全量 → `recent_threads` 投影。若归档线程在 `recent_threads` 中被排除而旧 dump 包含，文件夹列表将不再显示归档线程——与首页一致，视为修复。实现前实测两投影成员集差异并在评审中列出。
2. **文件夹列表排序**：统一为 `activity_seq` 降序（首页语义），替代现在的 updatedAt 内存排序。
3. **手势增强**：文件夹/bot/automation 列表获得长按菜单（含 Favorite、Archive）——直接回应需求 3。
4. automation 列表从"只显示前 50"变为可翻页到底。

## 6. 明确不做

- 不改桌面端列表（另案）。
- 不给 bot 文本列表加 workspace 过滤/新命令。
- 不动 pinned 全局语义（不给 workspace scope 加独立 pin 段）。
- 不动 openThread 路由与转场。
- 不做任何"旧网关兼容"分支（desktop/gateway 同仓同发）。

## 7. 交付切片

| 切片 | 内容 | 验证 |
|---|---|---|
| S1 gateway | §3.1 workspace_dir 过滤 + 索引；§3.2 thread_ids 点查；§3.4 对齐断言测试 | `cargo test -p garyx-gateway --lib`（keyset 过滤/点查/信封不变各用例）|
| S2 Core | §4.1 scope feeds + §4.2 store/presentation 泛化 | SwiftPM 测试（pager scope 隔离、LRU 逐出、投影 reducer、快照）|
| S3 App | §4.3 行统一 + §4.4 容器改造 + §4.5 迁移清单 + 全量 dump 删除 | xcodebuild 构建 + SwiftPM headless 断言（用真实捕获数据）；手势清单逐面核对 |
| S4 清理 | `/api/threads` 消费者盘点与退役、label 兼容层删除、死代码清扫 | 全量 grep 盘点 + tier1 |

每切片独立评审到 PASS 再进下一片；S1 先行（S2/S3 依赖其端点）。

## 8. 验收标准（对应需求逐条）

1. 首页与文件夹列表同一投影同一排序，同一线程在两处状态一致（同步问题消失）。
2. 进入文件夹 = 一次 page 请求（网络层可断言），无全量循环；列表为 native List 懒加载。大仓千线程场景进入不卡（行构建数 ≤ 首屏 + 预取窗口）。
3. 文件夹/bot/automation 列表长按出完整菜单（Pin/Favorite/Archive），swipe 与首页一致。
4. 四个列表面共用同一 row wrapper + store 栈（代码层面可 grep 断言旧 wrapper 已删）。
5. bot `/threads` 输出与 app 首页成员集/排序一致（测试钉住）。

## 9. 开放问题（评审请裁决）

1. `recent_threads` 投影是否覆盖"该 workspace 全部应显示线程"（尤其历史久远无 activity 的线程）——若有缺口，方案是补投影列还是接受成员集差异？
2. bot 会话摘要 hydration：批量点查（本设计）vs gateway 在 `/api/bot-consoles` 响应里内嵌摘要（server 侧 join）——哪个更符合"客户端 dumb"方向？
3. `GaryxPanelScaffold`（ScrollView 容器）在其他非列表 panel 还在用，List 化仅限线程列表面还是值得抽一个通用列表 scaffold？
