# iOS 线程列表统一化 —— 设计 v10（§1-§9 已 PASS 定稿；§10 增补待复审）

作者：Gary
日期：2026-07-17
基线：main `3dff1111a`（§10 基线：main `c050ea8b3`，S1/S2 已合入）

修订记录：
- **v10**（2026-07-17）：按 #TASK-2368 增补评审（FAIL，F-01..F-06）重写 §10：① cutover 改**双向精确重建**——补 `visible−recent` 之外还删 `recent−visible_live` 孤儿行（实测 5 条无 canonical 记录），parity 测试改双向 EXCEPT（F-01）；② 谓词表述修正：删除 `is_recent_thread_excluded` 整个 helper（hidden 判定本就独立存在），`excluded_from_recent` 派生恒 false（F-02）；③ 中间态闭合：S5 功能层面一次退役到位——canonical 存量 96 条 flag 同 cutover 清洗、创建/更新入口 strip 废弃 key（覆盖旧客户端）、router 镜像键删除、automation wire 停发 `excludeFromRecent`；S4 只删死代码（列/adapter 镜像/capability 门禁/last-open 门禁）（F-03）；④ side chat 归一走 **canonical 层**：按 canonical side-chat 选择器把 `body.hidden` 归一 true 再同事务派生投影，禁止只翻投影列/硬编码条数（F-04）；⑤ marker 采用 `import_generation_cutover_gate` 模式按代重跑（F-05）；⑥ `activity_seq` 重排算法合同钉死：fresh 连续块 `[H+1,H+N]`、H=max(meta 高水位,行 max)、完整全序定义、meta/索引/marker 同事务（F-06）；⑦ 确认不 rotate incarnation（评审专项核定）。
- **v9**（2026-07-17）：新增 §10 成员集严格对齐增补（老板裁决：automation 生成线程=普通线程去特殊化；全部列表共享同一数据宇宙、查询期过滤）。新切片 S5：`is_recent_thread_excluded` 收窄 hidden-only、cron 停写 `exclude_from_recent`、side chat 改挂 hidden、一次性 cutover `recent_membership_v2`（补行 + activity_seq 插位重排 + side chat 双隐）；`exclude_from_recent` 概念残余面并入 S4 删除。§5.1/§2.4 的成员集论述由 §10 取代。
- **v8**（2026-07-17）：按七轮评审（FAIL，R7-F01/F02）修订：① summaries 窗口 raw 段排序补齐现行确定性 tiebreak：`favorited_at DESC, thread_id ASC`（毫秒同值可达；补"499 recent + 2 同毫秒 raw 反向插入"的第 500 位合同测试）（R7-F01）；② canonical q **完全归服务端所有**（SQL 匹配 + cursor digest）；**客户端 provider identity 改用 trim 后原始 q + 单调 instanceID**，撤销"客户端 identity 用规范化 q"的自相矛盾表述——客户端永不复刻 `normalize_for_search`（R7-F02）。
- **v7**（2026-07-17）：按六轮评审（FAIL，R6-F01..F04）修订：① favorites snapshot 的 acceptance predicate 纳入 **`requestFlavor` + `capabilityGeneration`**——能力位转 `supported` 时取消在途 legacy 请求并**拒绝其 completion**，增强 snapshot 作为 replacement barrier 落地（R6-F01）；② 增强信封定义**独立的 summaries 窗口合同**：服务端在同一事务按"最终组合顺序"（favorites∩recent 按 activity_seq DESC + 余下 raw favorites 按 favorited_at DESC，与 reducer 现语义同构）取前 500 个成员为窗口，`summaries` = 窗口内 `default_list_hidden=0` 行，新增 `summaries_truncated`（组合成员超窗即 true，**不依赖既有只算 join 半边的 `truncated`**）；窗口内无 summary=hidden、超窗=`summaries_truncated`，两义分离（R6-F02）；③ `q` 规范化合同：先 trim 再 normalize，规范化后空串等同省略；cursor/provider identity 用规范化后 q；长度上限 = **规范化后 100 个 Unicode scalar**，超限 400（R6-F03）；④ §4.4 标题"双 wire adapter"改"三 wire adapter"（R6-F04）。
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
- `q=<子串>`（可选；跨 **title / workspace_dir / agent_id / last_message_preview 四字段**的大小写不敏感子串过滤——保持 picker 现有搜索语义不缩窄。合同（R6-F03/R7-F02）：服务端先 trim（对齐 picker 现行为）再 `normalize_for_search`，**规范化后空串等同省略参数**；长度上限 = 规范化后 **100 个 Unicode scalar**，超限 400。**canonical q 完全归服务端所有**（SQL 匹配 + cursor digest，cursor 对客户端 opaque）；**客户端 provider identity = trim 后原始 q + 单调 instanceID**（隔离在途请求/晚到响应用，不参与匹配语义，客户端永不复刻规范化例程）；见"q 分支"）
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
  - **独立 summaries 窗口合同（R6-F02 定稿，不复用既有 `truncated`——它只统计 favorites∩recent join 半边，501 excluded 时恒 false）**：服务端在**同一读事务**内按"最终组合顺序"计算窗口——favorites∩recent 成员按 `activity_seq DESC`，余下 raw favorites 按 **`favorited_at DESC, thread_id ASC`**（R7-F01：逐字对齐现行 `garyx_db/mod.rs:3857` 排序，`favorited_at` 毫秒精度同值可达，缺 tiebreak 会在 500 边界选错成员）接续——取前 500 个成员（**hidden 占窗口位**）；`summaries` = 窗口内 `default_list_hidden=0` 行；增强信封新增 `summaries_truncated: bool`（组合成员总数 > 500 即 true）。语义分离：**窗口内无 summary = hidden 不渲染；窗口外 = `summaries_truncated=true`**。测试钉住：501 全 excluded、499 recent + 2 excluded、窗口内 hidden 三用例。
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
5. **favorites**（R6-F01 定稿）：snapshot 整替机制与现有 reducer（unresolved fence/verify/补偿 CAS）**全部保持**；行摘要改由 snapshot 的 `include_summaries` **同事务**返回（§3.3）。**客户端单次 commit 合同**：`completeSnapshot` 返回显式 **accepted/rejected**，acceptance predicate = 现有 ticket/epoch/incarnation/revision 全套判定 **+ `requestFlavor`（legacy/enhanced）+ `capabilityGeneration`**；能力位转 `supported` 即升 capabilityGeneration：**取消在途 legacy 请求、拒绝其一切 completion**（legacy 回包不得先发布无 summaries 状态再等 trailing），随即以增强 snapshot 作为 **replacement barrier** 落地。**仅 accepted** 的响应在同一 `@MainActor` owner commit 内完成 summaryById write-through + lease swap + membership 整替，**最后只发布一次**（消灭现状"reducer 先整替、外层再写摘要、`didSet` 中间发布一次"的半更新帧；rejected 响应对 cache/lease/membership 零副作用——epoch ABA 与 flavor 混线旧响应均不能写 cache）。无客户端二次 hydration、无新 fence 面、hidden favorite 结构上不可发布、excluded 已收藏行可见（`.removeOnly` 入口成立）。网关无该能力时（§5.5）请求不带参数，行为与今天完全一致。

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

- **三 wire adapter（Core）**：
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
   - **`unknown/unsupported` → `supported` 跃迁**：升 `capabilityGeneration`、**取消在途 legacy snapshot 并拒绝其 completion**（§4.2.5），强制重取 favorites snapshot（带 `include_summaries`）作为 replacement barrier——覆盖网关升级后重连与"probe 失败期已发出 legacy 请求"的交叠窗口（R6-F01）；
   - probe 是 owner 持有的共享 in-flight：**等待者取消不取消 probe 本身**；N 个并发等待者只产生一个 probe、一次增强重取；
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
| S1 gateway | §3.1 新路由 + §3.2 增列/cutover + §3.3 favorites `include_summaries` + 对齐断言 | `cargo test -p garyx-gateway --lib`：全分支 query-plan（USING INDEX/无 TEMP B-TREE）、cursor scope/tasks/q/incarnation 失配 400、NULL/仅 created/混合格式/键前移后移分页用例、cutover 幂等回填（三列）、`normalize_for_search`+`instr` 行为矩阵（composed/decomposed、ß、sigma、`%`/`_`/反斜杠/全角/NUL 字面量）、exclusion helper 复用断言、favorites snapshot 同事务摘要/hidden 过滤/组合顺序窗口 `summaries_truncated`（501 全 excluded、499 recent+2 excluded、窗口内 hidden、**499 recent+2 同毫秒 raw 反向插入的第 500 位 tiebreak** 四用例）/**省略参数信封逐字节不变**、`q` trim/空串等同省略/100·101 scalar 边界、既有端点信封 characterization、bot `/threads`≡Chats 对齐 |
| S2 Core | §4.1 PinLease 缓存 + §4.2 provider/store 泛化（favorites 单次 commit）+ §4.3 代际/hub 状态机 + §4.4 三 adapter/capabilities + §5.5 能力位状态机 | SwiftPM：ABA 回归、LRU 501+ 成员回读、PinLease owner 别名/`didSet` oldValue/Sendable 反例回归 + 释放点全枚举（重叠来源/提前返回不泄漏/picker q 换代原子释放/selected slot swap）、favorites `completeSnapshot` accepted/rejected（rejected 零副作用、accepted 单发布、epoch ABA 与 requestFlavor 混线旧响应均不写 cache、supported 跃迁取消 legacy 在途并拒绝其 completion）、excluded 已收藏行可见且 removeOnly 可用、hidden favorite 恒不发布、能力位三态（冷启动 await probe/single-flight N 等待者单 probe 单重取/等待者取消不取消 probe/错误保持 unknown/升级重连强制重取 barrier）、hub 四态跨 store 一致性、ambiguous barrier 对照 `GaryxRecentThreadFeedsTests.swift:390-444` 现有用例等价、三 adapter 形状对照（真实捕获 + 与 Rust helper 共用 fixture 矩阵的合成 payload）、capabilities 全表、首页守恒 characterization |
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

## 10. 成员集严格对齐（老板 2026-07-17 裁决，独立切片 S5）—— v10 按 #TASK-2368 评审修订

### 10.1 裁决

1. **定时任务（automation）生成的线程是普通线程，不做任何特殊化**。
2. **所有线程列表（首页 Recent / 文件夹 / bot `/threads` / widget / 桌面 Recent）共享同一份数据宇宙，差别只在查询期过滤方式**（workspace 过滤、tasks 过滤、favorites、渠道呈现），不在写入期成员排除。

终态两投影成员集**严格相等** = visible live 线程（canonical `hidden` 为假）。hidden 成为**唯一**可见性概念；`exclude_from_recent` 概念整体退役。§2.4/§5.1 的成员集论述由本节取代。

### 10.2 现状实测（2026-07-17 本机库，#TASK-2368 复核后）

- `visible thread_meta` 3107 vs `recent_threads` 3031。
- `visible − recent` = 81：77 automation 生成（`cron.rs:1315-1325` 写死两标志）+ 4 条 legacy 桌面 Side chat（metadata.hidden=true 但顶层 hidden 缺失）。
- **`recent − visible` = 5：孤儿行，无 `thread_meta`、无 canonical `thread_records`**（两表无外键约束）。
- canonical 存量 exclusion flag 共 **96** 条（77 automation + 19 side chat，顶层与 metadata 两层）。
- Side chat 创建路径（`side-chat-ops.ts:82`）**本就写 hidden:true**，额外多写了 exclusion flag；宿主面板打开 side chat 按 session binding 直验 thread/transcript，不受 hidden 阻断（评审已核）。

### 10.3 S5 变更（功能层面一次退役到位，S4 只删死代码）

1. **写点停产（增量闭合）**：
   a. `cron.rs` automation 建线程停写 `exclude_from_recent`（`automation_id`/`automation_thread_mode` 保留——automation 域自身归属标记，drilldown 投影独立读 `automation_thread_runs WHERE mode='generated_thread'`，与列表成员判定无关，评审已核不受影响）；
   b. 桌面 side chat 创建停写 `exclude_from_recent`（hidden 已在写，无需改）；
   c. **服务端创建/更新入口统一剥离废弃 key**：`exclude_from_recent`（两拼写、两层级）在 metadata 入库前 strip——chokepoint 覆盖旧桌面客户端与任意 API 写入面；router `threads.rs` 镜像键表删除该键。
2. **成员集谓词**：删除 `is_recent_thread_excluded` 整个 helper（recent 投影的 hidden 判定本就独立存在，保留）；`thread_meta.excluded_from_recent` 派生改**恒 false**（列本体 S4 删）。
3. **wire 停回放**：`automation.rs:1277` drilldown DTO 停发 `excludeFromRecent` 字段（S2 adapter 对缺失字段解析为非 excluded，中间态自洽；adapter/capability/last-open 门禁等客户端消费面在 S4 删除，清单见 10.5）。
4. **一次性版本化 cutover `recent_membership_v2`**（`import_generation_cutover_gate` 模式：记录 `based_on_import_generation`，legacy 归档恢复重导入后**按代重跑**——评审 F-05），单事务内五步：
   a. **canonical 归一**：按 canonical side-chat 语义选择器（实现时核实确切标记，禁止硬编码条数/用 exclusion flag 猜测）把 legacy side chat 的 `thread_records.body.hidden` 归一为 true；
   b. **canonical 洗存量 flag**：全部 canonical record body 剥离 `exclude_from_recent`（顶层+metadata，96 条级）；
   c. **精确重建目标集（双向）**：以 canonical live universe 为准——补 `visible − recent` 行，**删除 `recent − visible_live` 孤儿行**（评审 F-01：现库 5 条）；
   d. **`activity_seq` 重排（评审 F-06 算法合同）**：取 fresh 连续块 `[H+1, H+N]`，`H = max(recent_threads_meta 高水位, 现行 max(activity_seq))`；全序定义 = 现有行按当前 `activity_seq ASC` 保持相对序，新行按 `(last_activity_ts, thread_id)` 与现有行的 `last_active_at` 归并插位（同刻现有行在前，再按 `thread_id`；时间戳缺失/不可解析按 epoch 0 沉底）；按该序赋 `H+1..H+N`（fresh block 天然规避 UNIQUE 冲突，实测 3501 行 ~0.01s）；同事务更新 meta 高水位与索引；
   e. durable marker 与数据同事务提交（崩溃=整体回滚下次重跑，幂等）。
5. **不 rotate store incarnation**（评审专项 (a) 已核：favorites CAS 只依赖 incarnation+revision，`activity_seq` 仅排序用、无跨启动持久引用；widget 不持久化 seq/cursor；客户端以 `server_boot_id` 变化整体重置分页即足够）。

### 10.4 影响面（有意的，全端生效）

1. Recent（iOS/桌面/widget）开始显示 automation 生成线程；bot `/threads` 同步（≡ Chats 过滤）。
2. Side chat 全部 hidden：从文件夹与 Recent 双隐；宿主面板直开路径不受影响。
3. 既有"automation 线程不在 recent""excluded favorite 不可见"类行为断言**有意改写**（评审专项 (e)）；信封形状零变化。
4. 成员集相等成为长期回归钉：**双向 `EXCEPT`** parity 测试（不只测 missing 一侧）。

### 10.5 概念残余面删除清单（S4 执行，S5 后全部失活）

- `thread_meta.excluded_from_recent` 列 + `/api/thread-summaries`/favorites summaries wire 字段；
- Swift 三 adapter 的 exclusion 解析镜像 + `GaryxThreadRowCapabilities` favorite 的 `.removeOnly/.none` excluded 门禁（收敛为 visible 即 `.addAndRemove`）+ `GaryxMobileModel+ThreadLifecycle.swift:70` last-open 门禁（评审 F-03 补列）；
- 相关 fixture/测试矩阵中 exclusion 用例改写为"字段不存在"合同。

### 10.6 S5 验证

- **双向** membership parity：`visible_live EXCEPT recent` 与 `recent EXCEPT visible_live` 皆空（长期回归钉）；
- cutover：幂等、按 import generation 重跑、插位序（构造混合活动时间 + 同刻 tiebreak 用例）、孤儿行删除、canonical flag 清洗（顶层+metadata 零残留 SQL 断言）、side chat canonical hidden 归一、重排后 favorites snapshot 相对序守恒、meta 高水位/UNIQUE/索引一致性；
- 创建入口剥离废弃 key 的行为测试（老客户端 payload 注入 exclusion → 入库后 canonical 无该键）；
- 既有 `tasks=` 三值过滤语义不变；信封 characterization 全数保持；
- `cargo test -p garyx-gateway --lib` 全绿 + tier1 --changed。
