# iOS 线程列表统一化 —— 设计 v6（待复审）

作者：Gary
日期：2026-07-17
基线：main `3dff1111a`

修订记录：
- **v6**（2026-07-17）：按五轮评审（FAIL，R5-F01..F04）修订：① favorites **客户端单次 commit 合同**——`completeSnapshot` 返回显式 accepted/rejected，仅 accepted 在同一 `@MainActor` owner commit 内完成 summaryById write-through + lease swap + membership 整替并只发布一次；`summaries` 定义为 **lookup payload**（排序权威仍是 reducer 现有 recent-activity + raw-favorite fallback 组合），cap/truncated 与现行 500 行窗口合同对齐（R5-F01）；② 能力位定义为按 gateway runtime epoch 隔离的 **unknown/supported/unsupported 状态机**（single-flight probe、404 才降级、错误保持 unknown、unknown→supported 强制重取 snapshot、reset/reconnect 清位）；并按实测修正旧网关事实：favorites handler 无 Query extractor，未知参数被**忽略返回 200 legacy 信封**而非 400（R5-F02）；③ 搜索 SQL 由 `LIKE/ESCAPE` 改为 **`instr(search_text, ?) > 0`**（天然字面子串、免转义、NUL 安全），测试矩阵补反斜杠/尾反斜杠/NFKC 全角兼容字符/NUL（R5-F03）；④ 术语统一：五 provider 实现、三 adapter、§6 favorites 措辞（R5-F04）。
- **v5**（2026-07-17）：按四轮评审（FAIL，R4-F01..F06）修订：① favorites 摘要**移入服务端**——snapshot 加可选 `include_summaries=true`，同一读事务返回 membership + `thread_meta` 摘要（只含 `default_list_hidden=0` 行），省略参数时旧信封逐字节不变；**撤销客户端逐 ID hydration**（R4-F01 fence 竞态与 R4-F02 hidden 泄露一并根治）；② `PinLease` 所有权定形：只由不可复制 `@MainActor final class` owner sidecar 持有，pager/feed 纯状态 struct 零改动，emitted snapshot 不携带 lease，补 selected-thread 独立 slot（R4-F03）；③ exclusion 谓词取"逐字节镜像现 helper"裁决：只认 snake_case、camelCase 判非 excluded、automation camel DTO 独立第三 adapter、双端同 fixture 矩阵合同测试（R4-F04）；④ 搜索规范化定义为共享 `normalize_for_search` = NFKC + default case fold（仅服务端实现），行为合同测试矩阵含 composed/decomposed、ß/ẞ、final sigma、通配符转义（R4-F05）；⑤ §4.6 拆正 pinned-section 补摘要路径归属、widget 触发补 pin reorder/rollback/reconcile 一族（R4-F06）；⑥ `/api/thread-summaries` 404 升格为**通用能力位**：旧网关下 favorites 请求不带新参数（规避 `deny_unknown_fields` 400）。
- **v4**（2026-07-17）：按三轮评审（FAIL，R3-F01..F06）修订：① Favorites provider 对 snapshot 缺失摘要的 membership ID 走既有 `/api/threads/:id` 有界 hydration，摘要到达前不发布裸 membership（R3-F01）；② pin 改 **`PinLease` RAII 句柄**并穷尽枚举释放点；picker `q` 纳入 provider 实例身份，q 变化 = cancel+升代+原子释放旧 pins（R3-F02）；③ `ambiguous` 定义为**保留态 reconstruction barrier**（按 instanceID 提交、失败粘性、旧 ticket 不得清除、淘汰冷加载视为权威完成；Favorites 继续由现有 reducer 裁决，hub 只做下游 fan-out）（R3-F03）；④ §4.6 补 recent feed 写事务五路径与 widget 重投影三触发点（R3-F04）；⑤ `q` 保持现有**四字段**搜索语义（title/workspace/agent/preview），新增 unicode casefold `search_text` 派生列，撤销 title-only 缩窄（R3-F05）；⑥ exclusion 谓词双端同构：新投影派生复用 recent 投影同一 helper，legacy adapter 镜像其层级/Bool 与字符串 truthy 强制转换/generated-mode 规则，配合成 payload 合同测试（R3-F06）；⑦ 修正"covering index"用词。
- **v3**（2026-07-17）：按 #TASK-2362 二轮评审（FAIL，R2-F01..F06）修订：① keyset 排序键改为**规范化非空整数列 `sort_updated_at_us`**（与 excluded 标志同一次 cutover 回填），补 NULL/仅 created/混合格式/键移动的分页语义与测试（R2-F01）；② 索引族改为 scoped/unscoped 双列族 + 显式 `DESC` + `default_list_hidden=0` partial 谓词；响应信封补 `store_incarnation_id`/`server_boot_id`，cursor 内嵌 incarnation、失配 400；`has_more` 用 limit+1 免 COUNT（R2-F02）；③ summaryById 增加 **ref-count pin**（resident membership/选中/widget/composer 引用不被逐出）；picker 增加 SQL-backed `q` 搜索参数，撤销"本地前缀过滤"倾向（R2-F03）；④ §4.6 扩为**读写路径所有权表**，补 lifecycle optimistic/rollback、favorites 合并、gateway reset、composer 插入等全部写路径归属（R2-F04）；⑤ mutation hub 升级为 **began/committed/rolledBack/ambiguous 事务状态机**（携带 mutationID/runtime epoch/权威 revision，ambiguous 跨 scope replacement）（R2-F05）；⑥ 定义**双 wire adapter**（新 DTO 与 legacy `/api/threads/:id` 形状归一化，兼容 exclusion 标志三种拼写/位置），Favorite capability 区分添加/移除（excluded 已收藏必须可 Unfavorite）（R2-F06）；⑦ 404 旧网关分类收紧：仅精确 HTTP 404 视为旧网关，401/403/5xx/解码/网络错误走普通错误/重试。
- **v2**（2026-07-17）：按一轮评审（FAIL，F-01..F-11）修订：文件夹改 `thread_meta` scoped keyset 新路由、砍批量点查 API、消费者穷尽表、feed 实例代际、capability 模型、`/api/threads` 保留、版本偏斜合同等。
- v1：初版，一轮评审 FAIL。

## 0. 需求（老板原话拆解）

1. iOS app 里多处线程列表**割裂**：首页 Recent 一套，抽屉 workspace（"文件夹"）另一套——数据不同步、观感不一致。
2. 点文件夹进列表**特别卡**。
3. 文件夹列表**不支持长按归档**等首页已有手势。
4. 首页 / 文件夹 / bot 会话 / automation 列表**组件同构、数据同源、手势一致**。
5. bot 渠道文本线程列表与 app 列表**同源**。

## 1. 现状盘点（一轮评审核实修正后）

### 1.1 四套数据路径并存

| 列表 | 端点 | 投影 | 分页 | 关键差异 |
|---|---|---|---|---|
| 首页 recent | `/api/recent-threads` | `recent_threads`（keyset by `activity_seq`） | cursor | 完整 `RecentThreadRecord`，信封含 `store_incarnation_id`/`server_boot_id` |
| 首页 Favorites filter | `/api/thread-favorites/snapshot`（+辅助 recent feed） | favorites + join `recent_threads` | snapshot 整替 | 不走 `/api/recent-threads` 主路径 |
| workspace 文件夹 & bot 摘要源 | `/api/threads?limit=1000&offset=…` **循环拉全量**；另有逐 ID `/api/threads/:id` 修补路径 | `thread_meta` | offset | 用 `label`（非 `title`） |
| automation triggered threads | `/api/automations/{id}/threads` | automation run 投影 | offset，单页 50 无 load-more | camelCase、内嵌 `thread` 摘要 |
| bot 渠道 `/threads` 文本列表 | router `RecentThreadPageReader` | `recent_threads`（offset 读法，固定 `RecentThreadFilter::Exclude`） | offset 文本翻页 | 成员集 = Home **Chats（nonTask）** filter |

### 1.2 两个投影的真实成员集（一轮评审实测结论）

- `recent_threads`：排除 hidden / `exclude_from_recent` / `generated_thread`；不要求有时间或消息。
- `thread_meta`：收录所有 canonical 线程，只记录 `default_list_hidden`。
- 归档在同一事务删除 record + 两投影 + pin + favorite → 归档线程两边都不存在。
- 实测差集 81 条全部是 Recent exclusion（77 条 generated automation）→ 文件夹今天的成员集 = 全量 live 线程。

### 1.3 时间戳现实（二轮评审实测结论，v3 方案直接由此决定）

- `thread_meta.updated_at` 是 **nullable 原始 TEXT**，投影只复制字符串，不回退 `created_at`、不规范化格式；写接口无时间单调性校验。
- raw TEXT 排序下 `00:00:00Z` 会排在更晚的 `00:00:00.500+00:00` 之前；NULL 行在 `updated_at < cursor` continuation 下**永久漏行**（实测复现）。
- 当前 iOS 内存排序实为 `parse(updatedAt ?? createdAt ?? distantPast)`，同时间按 title tiebreak。

### 1.4 客户端割裂与卡顿根因（评审核实属实）

- 首页独享整套打磨栈（store/off-main 投影/epoch 票据 pager/stale-while-refresh feeds/sections cache/native `List`）；drilldown 全不用：父 section `@EnvironmentObject` 整 model 观察（整段重建真因）、`model.threads` 每渲染现算分组、`ScrollView + VStack + ForEach` 无懒加载、进入时同步全量循环拉取。
- 行包装两套（equatable/live 时间戳/长按菜单 vs 即时构造/静态时间戳/仅 swipe）。首页已有能力裁剪先例（automation target 禁 Archive；服务端拒绝 active/automation-managed 归档）。

## 2. 设计原则与关键裁决

1. **根因修复，不做客户端 workaround**。
2. **两投影各司其职，一份呈现栈**：`recent_threads` = recency feed（首页 + bot `/threads`），**零改动**；`thread_meta` = 全量 live 线程摘要清单（文件夹、逐 ID 点查、picker）。共享的是**摘要 DTO、row wrapper、capability/action 层、presentation store 基座**；membership feed 按领域独立。
3. **条件查询走 SQL 投影**：静态 SQL 分支 + 索引，不加扫描、不加读时修复。
4. **成员集裁决**：文件夹保持今天的"全量 live 线程"成员集；排序语义 = 今天的"时间降序"，服务端化为规范排序键（§3.1）。tiebreak 从 title 改为 `thread_id`（有意变化，§5.2）。
5. **Mac app 是 IA 真相源**，不发明新概念。

## 3. 服务端设计

### 3.1 新路由：`GET /api/thread-summaries`

**参数**：
- `workspace_dir=<绝对路径>`（可选；精确匹配）
- `tasks=include|exclude|only`（可选，默认 `include`）
- `q=<子串>`（可选；跨 **title / workspace_dir / agent_id / last_message_preview 四字段**的大小写不敏感子串过滤——保持 picker 现有搜索语义不缩窄；长度上限 100，超限 400；见"q 分支"）
- `cursor=<opaque>`（可选）
- `limit=<1..100>`（可选，默认 30；非法 400）

**成员集**：全部 canonical live 线程，沿用现行 `default_list_hidden=0` 过滤谓词。

**排序键（R2-F01 根治）**：新增 `thread_meta.sort_updated_at_us INTEGER NOT NULL`——每次投影写内派生：`parse_rfc3339(updated_at) ?? parse_rfc3339(created_at) ?? 0`（微秒 epoch；容忍 `Z`/`+00:00`/亚秒混合格式，解析失败按缺失回退）。排序 `sort_updated_at_us DESC, thread_id DESC`。**非空、整数、全序**，keyset 无 NULL 分支。

**键移动语义（明确写死）**：页面是时间点切片。排序键**前移**（变新）的行：已服务页不回收，下次 head refresh 出现在头部，客户端按 `thread_id` 去重（保留最头部出现）；排序键**后移**的行：可能重复出现在后续页（同样去重）或在 refresh 前暂不可见——与 recent feed 现有分页语义同构，接受。

**响应信封（R2-F02）**：
```json
{
  "threads": [ThreadSummaryRow…],
  "next_cursor": "…|null",
  "has_more": false,
  "store_incarnation_id": "…",
  "server_boot_id": "…"
}
```
`has_more` 由 `limit+1` 取行判定，**不做 COUNT**。`ThreadSummaryRow` 字段：`thread_id`、`title`（服务端从 `thread_label` 映射，统一命名）、`workspace_dir`、`thread_type`、`provider_type`、`agent_id`、`created_at`、`updated_at`、`message_count`、`last_user_message`、`last_assistant_message`、`last_message_preview`、`recent_run_id`、`active_run_id`、`worktree`、`excluded_from_recent`。

**cursor 合同**：opaque base64 JSON `{v:1, scope, tasks, q, incarnation, sort_key, thread_id}`；`scope` = `sha256(workspace_dir ?? "")`。请求参数与 cursor 内嵌 scope/tasks/q 失配 → 400；`incarnation` ≠ 当前 store incarnation → 400（恢复/换库后旧 cursor 不可续页）；解析失败 → 400。客户端另按信封身份对（incarnation, boot）变化主动整体重置分页（镜像 recent feeds 现行为）。

**索引族（R2-F02）**：
- scoped：`(workspace_dir, sort_updated_at_us DESC, thread_id DESC)`
- unscoped：`(sort_updated_at_us DESC, thread_id DESC)`
- 两族各配 visible / task / non-task partial index，`default_list_hidden=0` 进 partial 谓词（镜像 recent 投影的 partial index 模式）。
- 全分支 `EXPLAIN QUERY PLAN` 测试：断言 `USING INDEX` 供序（17 列行查询非 covering，断言以实际计划输出为准）、**无 `USE TEMP B-TREE`**（显式 `DESC` 列序）。

**q 分支（R4-F05 定稿）**：定义共享例程 `normalize_for_search` = **Unicode NFKC 规范化 + default case fold**（Rust 侧单一实现，如 `unicode-normalization` + `caseless`，具体 crate 实现者定，**行为以测试矩阵为合同**）。`thread_meta` 新增派生列 `search_text TEXT NOT NULL DEFAULT ''` = `normalize_for_search(title + '\n' + workspace_dir + '\n' + agent_id + '\n' + last_message_preview)`，写路径同事务派生；`q` 参数进 SQL 前经**同一函数**处理（该函数只存在于服务端；客户端本地过滤已整体撤销，无需复刻）。`q` 非空时静态 SQL 分支 **`instr(search_text, ?) > 0`**（R5-F03 定稿：天然字面子串语义、无 wildcard/escape 面、TEXT 内 NUL 安全——`LIKE` 的反斜杠转义序、NFKC 把全角 `％＿＼` 折成元字符、NUL 截断三反例全部结构性消灭），同 cursor 键续页；有界投影表过滤（LIMIT 截断、单表、无 record body 读取），不在 plan 断言范围但有行为合同测试矩阵：composed/decomposed（`é` vs `e`+组合重音）、`ß`/`ẞ`/`SS`、希腊 final sigma、`Éclair`↔`éclair`、`%`/`_`/反斜杠/尾反斜杠字面量、NFKC 全角兼容字符（`％＿＼`）、含 NUL 文本、空 title 行由 workspace/agent/preview 命中。`q` 与 `workspace_dir`/`tasks` 可组合。

**实现模式**：镜像 bot-recent-threads 改版模式——page 查询单分支静态 SQL、显式读事务；count 不存在（信封无 total）。

**与 `/api/threads/:id` 的关系**：`:id` 点查继续作为逐线程摘要读路径，**信封不改**；形状差异由客户端三 adapter 归一（§4.4）。

### 3.2 `thread_meta` 增列与一次性 cutover

- 新列：`excluded_from_recent INTEGER NOT NULL DEFAULT 0`、`sort_updated_at_us INTEGER NOT NULL DEFAULT 0`、`search_text TEXT NOT NULL DEFAULT ''`（§3.1 q 分支）。
- `excluded_from_recent` 派生**直接调用 recent 投影现有的 exclusion helper**（`recent_thread_projection.rs:156-179` 那套：top-level 与 metadata 两层、Bool 与字符串 `true/yes/1` truthy 强制转换、两层 `automation_thread_mode` generated 规则）——**同一函数，不重写谓词**（R3-F06 服务端半边）。
- 三列写路径同事务派生；**同一个**一次性版本化 cutover `thread_meta_summary_v1`（boot import 后运行、durable marker、幂等），单次存量扫描同时回填三列。无第二次大迁移。

### 3.3 不动的部分（显式声明）

- `/api/recent-threads`：零改动。
- `/api/threads`（offset 列表）与 `/api/threads/:id`：**端点与信封保留**（desktop 主进程/desktop web/CLI 在用）；仅 iOS 停用全量循环用法。
- `/api/thread-favorites/snapshot` 加可选参数 `include_summaries=true`（R4-F01/F02 根治，R5-F01 定稿合同）：**同一 SQLite 读事务**内在既有 membership/revision/incarnation 之外追加 `summaries`（`thread_meta` 派生的 `ThreadSummaryRow`，**只含 `default_list_hidden=0` 行**）。省略参数时旧信封**逐字节不变**（characterization 钉住）。
  - **`summaries` 是 lookup payload，不是排序权威**：按 `thread_id` 索引消费；行排序权威保持 reducer 现有语义（recent-activity 顺序 + raw-favorite `favorited_at` fallback 组合）。
  - **cap/truncated 合同与现行 500 行窗口逐字对齐**（现行实测：501 membership → 501 IDs + 500 rows + `truncated=true`）：`summaries` 覆盖同一 500 窗口内的全部非 hidden membership；窗口内"无 summary = hidden 不渲染"，窗口外由既有 `truncated=true` 表达——两义不混。
  - **不存在任何客户端逐 ID hydration/fence**。favorites reducer/fence/CAS 与 pins 端点全部不动。旧网关事实（R5-F02 实测修正）：favorites handler 无 Query extractor，未知参数被**忽略并返回 200 legacy 信封**（无 summaries），非 400——故旧网关下行为自动等同今天，能力位的作用是**升级后强制重取**（§5.5）。
- bot 渠道 `/threads`：不动；对齐断言测试钉住其成员集/排序 ≡ `/api/recent-threads` `tasks=exclude`（Home **Chats** filter）。
- automation `/api/automations/{id}/threads`：端点不动，客户端接 `hasMore` 补 load-more。

## 4. 客户端设计（GaryxMobileCore + App）

### 4.1 两层所有权 + 缓存 pin（R2-F03 根治）

- **`GaryxThreadSummaryCache`（Core，新）**：`summaryById` 唯一"按 ID 取摘要"真相源，**`PinLease`（RAII 句柄）+ LRU**（R3-F02 根治）：
  - pin 是**引用型 `PinLease` 句柄，只能由不可复制的 owner 持有**（R4-F03 定形）：lease 挂在 `@MainActor final class` 的 presentation/store owner 私有 sidecar 上（现 `GaryxHomeThreadListStore` 一族本就是 final class）；**pager/feed 纯状态 struct 继续只持 `[String]` ID**——Equatable/Sendable 状态机零改动；emitted snapshot 不携带 lease；`invalidate` 不暴露给任何可复制别名；cache 不反向强持有 lease。由此排除"struct 副本别名提前释放 / `didSet` oldValue 历史副本延迟释放 / Sendable stored property 破坏"三反例；
  - **释放点穷尽枚举**（每处配测试，所有提前返回路径必须释放）：page replace/remove、feed 淘汰/reset、gateway scope epoch reset、**当前打开/选中线程变更含回到 draft（selected-thread 独立 lease slot 原子 swap）**、picker `q` 更换/sheet 关闭/已选 target 更换、widget 写完成/取消/skip（含 `+ThreadPersistence.swift:8-38` 的多处提前返回）、composer settle/cancel 全部移除路径（`+Composer.swift:293-347`）、bot entries replacement；
  - LRU 只逐出零引用条目；pinned 不计容量上限（上限只约束无引用池，默认 500）；配"501+ 成员滚动回读"、"重叠 pin 来源"、"提前返回不泄漏"回归测试。
- **scope membership store**：只持成员顺序 + 分页/过渡状态；行内容从 summaryById 解引用；runtime overlay 走既有 runtime 合并路径落 overlay 层。

### 4.2 membership provider 抽象

`GaryxThreadListMembershipProvider` 输出规范化成员页快照（有序 id 列 + 分页态 + 伴随摘要回填）。五实现：
1. **recent**：现 feeds/pager 零改动；
2. **workspace(path)**：`/api/thread-summaries?workspace_dir=…`，复用 `GaryxHomeThreadListPager` 纯状态机；
3. **botConversations(groupId)**：bot console/endpoints 派生（非线程分页），摘要走 summaryById + 逐 ID `/api/threads/:id` 补缺；
4. **automationThreads(id)**：既有端点 + load-more 页驱动；
5. **favorites**（R5-F01 定稿）：snapshot 整替机制与现有 reducer（unresolved fence/verify/补偿 CAS）**全部保持**；行摘要改由 snapshot 的 `include_summaries` **同事务**返回（§3.3）。**客户端单次 commit 合同**：`completeSnapshot` 返回显式 **accepted/rejected**（现有 ticket/epoch/incarnation/revision 全套判定的结果外露）；**仅 accepted** 的响应在同一 `@MainActor` owner commit 内完成 summaryById write-through + lease swap + membership 整替，**最后只发布一次**（消灭现状"reducer 先整替、外层再写摘要、`didSet` 中间发布一次"的半更新帧；rejected 响应对 cache/lease/membership 零副作用——runtime epoch ABA 旧响应不再能写入 cache）。无客户端二次 hydration、无新 fence 面、hidden favorite 结构上不可发布、excluded 已收藏行可见（`.removeOnly` 入口成立）。网关无该能力时（§5.5）请求不带参数，行为与今天完全一致。

通用 presentation/action store（`GaryxHomeThreadListStore` 泛化）消费任一 provider 快照；`.recent(all)` 保留 pinned 段 + 拖拽重排。**automation picker** 用 unscoped provider + `q` 服务端搜索（撤销"本地前缀过滤"——只滤已加载页会漏未翻页目标）。**`q` 是 provider 实例身份的一部分**（R3-F02）：q 变化 = cancel 在途 + 实例升代 + **原子释放旧页全部 PinLease**；旧实例任何晚到响应（含首屏）按实例代际丢弃——cursor digest 拦不住的"旧 q 首屏晚返回"时序由代际闭合。picker 已选 target 的摘要经点查 + 独立 lease 保活，不随搜索页释放。

### 4.3 feed 注册表与 mutation hub（R2-F05 根治）

- **实例代际/淘汰**（二轮已确认成立，保持）：feed 实例携带单调 `instanceID`，ticket 带 instanceID，completion 校验失配即丢；workspace feed LRU 上限 4，淘汰即 cancel 在途 + 冷加载重入；recent 三 feed 常驻。ABA 回归测试。
- **`GaryxThreadMutationHub` = 事务状态机（非成功通知总线）**：
  - 事件：`began / committed / rolledBack / ambiguous`，携带 `mutationID`、mutation 种类与目标、gateway runtime epoch、权威结果（committed 附服务端权威 membership/revision 数据，如 pin resolve 结果）；
  - 所有 resident store（含 summaryById）订阅：同一 `mutationID` 下**同步进入 pending → committed/rolledBack**，跨 scope 一致呈现请求中/失败/回滚态；
  - **`ambiguous` = 保留态 reconstruction barrier**（R3-F03，非"发起一次刷新"）：每个相关 resident store 按自己的 `instanceID` 提交 authoritative replacement；replacement 携带独立 generation，**旧 ticket 不得消费/清除后排队的 replacement**；replacement 失败**粘性保留** pending 标志等下次重试（对齐 `GaryxRecentThreadFeeds.swift:338-359` 现语义）；store 被淘汰后冷加载视为该 store 的权威完成；archive ambiguous 立即撤销行的 archiving motion、**旧快照保留到 replacement 到达**；
  - **Favorites 例外条款**：favorites 的 ambiguous/verify/补偿 CAS 语义继续由现有 `GaryxFavoritesState` reducer 独占裁决，hub 对 favorites 只做其**下游 fan-out**（把 reducer 已裁决的结果播给其他 scope），绝不替代该 reducer；
  - 现有 Home archive（begin/commit/cancel/ambiguous replacement，`GaryxMobileModel+Bots.swift:248-323`）与 pin（begin/resolve/rollback，`+ThreadPersistence.swift:63-100,133-155`）逻辑**重构为 hub 的参考实现**，characterization 测试钉住首页行为守恒。

### 4.4 wire 归一化与能力模型（R2-F06 根治）

- **双 wire adapter（Core）**：
  1. `ThreadSummaryRow → GaryxThreadSummary`（新路由）；
  2. legacy `/api/threads/:id` record → `GaryxThreadSummary`：兼容 `label→title`；exclusion 判定**逐字节镜像服务端 helper 现谓词**（R4-F04 裁决：取选项 1，不扩 helper 不重投影）——**只认 snake_case `exclude_from_recent`**（camelCase metadata key 服务端不认，adapter 同样必须判非 excluded）、top-level 与 metadata 两层查找、值接受 Bool 与字符串 truthy（`true`/`yes`/`1`）、两层 `automation_thread_mode` generated 规则；**不改服务端信封**；
  3. automation 端点 camelCase 内嵌 `thread` DTO → **独立第三 adapter**（本就是不同 wire 形状，不与 canonical 谓词混写）。
  三 adapter 归一到同一 `GaryxThreadSummary`。测试双轨：真实捕获 payload 对照 + **合成 payload 合同测试**（同一 fixture 矩阵喂 Rust helper 与 Swift adapter：字符串 truthy/falsey、camelCase key（双端都应判非 excluded）、两层错位、缺失字段），保证同一 canonical record 双端谓词输出恒等（capability 不随入口漂移）。
- **capabilities**：
```
struct GaryxThreadRowCapabilities {
    canOpen, canPin, canArchive: Bool
    favorite: .addAndRemove | .removeOnly | .none
    archiveStrategy: .thread | .botEndpoint | .none
}
```
  - Core 单一派生函数 + 测试；输入：摘要 flags、当前 favorite 状态、automation target 集、active run 态、bot entry 能力。
  - 规则：excluded 且未收藏 → `favorite = .none`（新增不可达）；excluded 且已收藏 → `.removeOnly`（**Unfavorite 恒可用**，历史/桌面/API 产生的 excluded favorite 不被困住）；automation target → `canArchive=false`；active run 归档保持服务端裁决 + 客户端预门禁；bot 会话行 `archiveStrategy=.botEndpoint`；不 openable 占位行全关。
  - 首页切同一派生函数，characterization 证明现行为逐条不变。

### 4.5 行组件与容器

- 合并两套 wrapper → `GaryxThreadListRowButton`：equatable、预算 row 输入（action 闭包注入）、live 时间戳全列表生效、长按菜单 + swipe 按 capabilities 渲染、`openSource` 参数化（首页 `.replace` / drilldown `.current`；openThread 唯一打开路径）。
- 新建 sibling `GaryxListPanelScaffold`（内嵌 native `List`）承载线程列表面；`GaryxPanelScaffold` 保留给非列表 panel。
- drilldown section 观察窄 store，摆脱 `@EnvironmentObject` 整体重建。

### 4.6 `model.threads` 读写路径所有权表（R2-F04 根治；S3 逐行核销）

| 路径 | 位置 | 归属 |
|---|---|---|
| workspace 分组 `sidebarWorkspaceThreadGroups` | `GaryxMobileSidebarViews.swift:1085` | workspace membership store |
| bot 摘要查表 `sidebarThreadSummary` | `+Presentation.swift:450` | summaryById（点查补缺） |
| Home 投影/drawer 发布 | `GaryxMobileModel.swift:105-109`、`+Presentation.swift:141-188` | recent feeds + summaryById |
| open/restore/deep-link 缓存 | `+AgentsWorkspaces.swift:94-199,256-275` | summaryById + `/api/threads/:id`（打开中线程 pin） |
| widget snapshot/标题同步/reconcile | `+ThreadList.swift:535-599,892-899` | recent feed 页 + summaryById（widget 集 pin） |
| run-state 合并 | `+ThreadRunState.swift:115-120` | runtime overlay → summaryById |
| queued composer fallback | `+Composer.swift:365-379` | summaryById（composer 引用 pin） |
| workspace 建议、pinned/recent 映射 | `+Presentation.swift:584-615` | recent 页 + summaryById |
| automation 创建/编辑/picker | `GaryxMobileAutomationViews.swift:384-385,518-520,999-1019` | picker provider（`q` 搜索）+ 已选 target 点查 pin |
| **新线程插入/重命名/runtime optimistic+rollback** | `+ThreadLifecycle.swift:232-252,360-475` | mutation hub（began/committed/rolledBack）+ summaryById write-through；新线程插入 = 对 recent 与所属 workspace store 的 membership insert 事件 |
| **composer 创建线程插入** | `+Composer.swift:608` | 同上（hub membership insert） |
| **favorites snapshot 合并** | `+ThreadFavorites.swift:100-119` | favorites membership store + summaryById write-through |
| **bot/pin required summary 合并** | `+StateSync.swift:49-70` | summaryById write-through |
| **archive 本地删除** | `+ThreadPersistence.swift:164-176` | hub `committed(archive)` 事件 |
| **gateway reset** | `+Gateway.swift:133` | 注册表级 reset：feeds registry + summaryById + hub 随 gateway scope epoch 整体清位 |
| **catalog restore/debug fixture** | 各处 | summaryById seed 入口（测试专用路径显式标注） |
| **recent head refresh 前快照 + commit** | `+ThreadList.swift:53-119,439-498` | recent membership store 的 ticket-accepted 事务：快照/commit/selection rebind 全在 store commit 边界内完成，summaryById write-through 随 commit 同步（R3-F04） |
| **auxiliary All merge** | `+ThreadList.swift:183-200` | 同上：辅助 feed 页作为 store 事务的一部分 merge，不再写 `model.threads` |
| **pinned section 补摘要（读 `/api/thread-pins`）** | `+ThreadList.swift:226-262` | pins membership + summaryById write-through——该路径服务的是 recent(all)/favorites 共享的 **pinned 段**，不是 favorite membership（R4-F06 更正）；favorites 行摘要走 §3.3 同事务 snapshot |
| **ambiguous auxiliary replacement merge** | `+ThreadList.swift:400-417` | hub `ambiguous` barrier（§4.3）内的 store replacement 提交 |
| **load-more merge** | `+ThreadList.swift:627-675` | recent membership store 的 loadMore ticket acceptance（pager 语义不动） |
| **widget 重投影触发** | `+CatalogCache.swift:68-70`、`+StateSync.swift:30-33`、`+AgentsWorkspaces.swift:1144-1146`、**pin reorder 成功/rollback/remote reconcile**（`+PinnedOrder.swift:136` 一族） | widget publisher 改订阅 recent store 快照 + pinned order 的 commit 变更（发布边界 = store commit），彻底不读 `model.threads`（R3-F04/R4-F06） |
| `refreshWorkspaceAndBotThreads()` 全量循环 | `+ThreadList.swift:708-739` | **删除** |

终态 `model.threads` **整字段删除**；S3 验收含"grep 零残留读写点"，残留即 FAIL。optimistic/rollback 一律经 hub，禁止 store 私有双写。

## 5. 行为变化（有意的）

1. 文件夹成员集不变、时间降序语义不变；取数改 keyset 分页 + stale-while-refresh + mutation fan-out（"不同步"消失，无行静默消失）。
2. **同时间戳 tiebreak 从 title 改为 `thread_id DESC`**（服务端确定性排序的代价，行序仅在同微秒时间戳内可能与今天不同）。
3. 手势增强：drilldown 获得长按菜单（按 capabilities 裁剪）；excluded 线程 Favorite 新增不可达、移除恒可用。
4. automation 列表可翻页到底；drilldown 时间戳变 live。picker 搜索四字段语义保持不变（服务端化，覆盖未翻页候选属增强）；`q` 长度上限 100 为新约束。
5. **版本偏斜合同（R5-F02 定稿：能力位 = 按 gateway runtime epoch 隔离的三态状态机）**：
   - 状态 `unknown / supported / unsupported`，初始与 gateway reset/reconnect 后均为 `unknown`；
   - **首个能力消费者**（favorites 冷启动 snapshot、workspace drilldown、picker，谁先到谁触发）统一 `await` 同一个 **single-flight probe**：`GET /api/thread-summaries?limit=1`；HTTP 200 → `supported`；**精确 HTTP 404** → `unsupported`；**401/403/5xx/网络/解码错误 → 保持 `unknown`**（本次消费者按普通错误/重试呈现，绝不永久降级）；
   - favorites 冷启动在 probe 结果前不发 snapshot 请求（await probe；probe 失败保持 unknown 时按今天的无参数请求发出，成功后若转 `supported` 走下一条强制重取）；
   - **`unknown/unsupported` → `supported` 跃迁强制重取 favorites snapshot（带 `include_summaries`）**——覆盖网关升级后重连场景；
   - `unsupported` 下：文件夹列表与 picker 增强模式显式"网关版本过旧，请升级"空态，picker 降级为 recent 已加载页 + 同一提示；favorites 请求不带新参数（旧网关 handler 无 Query extractor，未知参数被忽略返回 200 legacy 信封，行为等同今天）；bot hydration/首页/automation 走既有端点不受影响；
   - 测试：冷启动新/旧网关、probe 各类失败保持 unknown、并发消费者共享 single-flight、升级重连强制重取。不做静默 fallback、不留旧全量 dump 双路径。

## 6. 明确不做

- 不改桌面端列表（另案）。
- 不动 `/api/recent-threads`、`/api/threads`、`/api/threads/:id`、pins 端点契约；favorites snapshot 仅加可选 `include_summaries`（省略时逐字节不变，§3.3）。
- 不给 bot 文本列表加 workspace 过滤/新命令。
- 不动 pinned 全局语义（workspace scope 无独立 pin 段）。
- 不动 openThread 路由与转场。
- favorites reducer/fence/CAS 语义与 pins 端点不动（§3.3 仅给 snapshot 加可选同事务摘要面）。

## 7. 交付切片

| 切片 | 内容 | 验证 |
|---|---|---|
| S1 gateway | §3.1 新路由 + §3.2 增列/cutover + §3.3 favorites `include_summaries` + 对齐断言 | `cargo test -p garyx-gateway --lib`：全分支 query-plan（USING INDEX/无 TEMP B-TREE）、cursor scope/tasks/q/incarnation 失配 400、NULL/仅 created/混合格式/键前移后移分页用例、cutover 幂等回填（三列）、`normalize_for_search`+`instr` 行为矩阵（composed/decomposed、ß、sigma、`%`/`_`/反斜杠/全角/NUL 字面量）、exclusion helper 复用断言、favorites snapshot 同事务摘要/hidden 过滤/500 窗口 truncated 对齐/**省略参数信封逐字节不变**、既有端点信封 characterization、bot `/threads`≡Chats 对齐 |
| S2 Core | §4.1 PinLease 缓存 + §4.2 provider/store 泛化（favorites 单次 commit）+ §4.3 代际/hub 状态机 + §4.4 三 adapter/capabilities + §5.5 能力位状态机 | SwiftPM：ABA 回归、LRU 501+ 成员回读、PinLease owner 别名/`didSet` oldValue/Sendable 反例回归 + 释放点全枚举（重叠来源/提前返回不泄漏/picker q 换代原子释放/selected slot swap）、favorites `completeSnapshot` accepted/rejected（rejected 零副作用、accepted 单发布、epoch ABA 旧响应不写 cache）、excluded 已收藏行可见且 removeOnly 可用、hidden favorite 恒不发布、能力位三态（冷启动 await probe/single-flight/错误保持 unknown/升级重连强制重取）、hub 四态跨 store 一致性、ambiguous barrier 对照 `GaryxRecentThreadFeedsTests.swift:390-444` 现有用例等价、三 adapter 形状对照（真实捕获 + 与 Rust helper 共用 fixture 矩阵的合成 payload）、capabilities 全表、首页守恒 characterization |
| S3 App | §4.5 行统一/List scaffold/窄 store + §4.6 所有权表逐行核销 + 全量 dump 删除 + 404/错误分类呈现 | xcodebuild 构建 + SwiftPM headless（真实捕获数据）；`model.threads` grep 零残留；手势 capability 清单逐面核对；xcodegen pbxproj 同步提交 |
| S4 清理 | iOS 侧 `label` 兼容层删除（限不再消费的层）、旧 wrapper 删除 grep 断言、死代码清扫 | 全量 grep 盘点 + tier1 |

每切片独立评审到 PASS 再进下一片；S1 先行。

## 8. 验收标准

1. 首页与文件夹：同事务投影供数 + 统一 mutation 状态机 + 统一刷新机制，同一线程两处状态（含请求中/失败/回滚态）一致。
2. 进入文件夹 = 一次 keyset page 请求（网络层断言），native List 懒加载；千线程 workspace 首屏行构建数 ≤ 首屏 + 预取窗口。
3. drilldown 长按菜单 + swipe 与首页同组件同 capability 规则。
4. 四列表面共用：摘要 DTO（三 adapter 归一）、`GaryxThreadListRowButton`、capabilities 派生、presentation store 基座；旧 wrapper 删除 grep 断言。
5. bot `/threads` ≡ 首页 Chats filter（测试钉住）。

## 9. 开放问题（复审请裁决）

1. picker 已选 target 不在当前搜索结果页时的呈现（本版：经点查 + 独立 lease 显示在"已选"区）——是否符合 Mac app 语义，S3 实现时对照。
