# bot recent 线程命令改版 —— 综合设计 v2.1（已过设计 review，可开工）

作者：Gary（综合自设计 A / 设计 B）
日期：2026-07-11
基线：main `12a8a6e3d060`

修订记录：
- **v2.1**（2026-07-11）：#TASK-2113 复核 **PASS**；吸收 2 条非阻塞备注（§8 绑定服务测试措辞修正；§3.1 defer 项并入启动期 `endpoint_thread_map` 重建）。
- **v2**（2026-07-11）：按 #TASK-2113 评审意见修订——① 新增 endpoint holder 不变量 + 一次性去重迁移（§3.2、§4.2，评审意见 1）；② 绑定流四处扫描全量盘点 + 执行路径唯一化（§3.1，意见 2）；③ 兼容承诺改写为「成员集/信封不变 + `thread_type` 值有意变化」并列出受影响出口与客户端核实结论（§0/§7/§8，意见 3）；④ 细节修正：reader trait 删 `workspace_dir`、count/page 显式读事务、群聊暴露面含 preview、快照内存上界给数（§2.2/§3/§5，意见 4）。
- v1：初版，#TASK-2113 评审 FAIL（五裁决全部维持，前提①②③核实属实）。

来源文档（评审时请对照阅读，细节规格以引用节为准）：

- **设计 A**（claude，#TASK-2111）：评审工作区中的 `design-A-claude.md`
- **设计 B**（codex，#TASK-2112）：评审工作区中的 `design-B-codex.md`

## 0. 需求

1. bot 渠道内的线程列表命令改为**全局 recent 线程列表**（过滤 task 线程），支持**翻页**。
2. 绑定线程命令基于该列表选择。
3. `GET /api/recent-threads` 支持按 task 过滤；省略参数时**成员集、排序、分页、响应信封、行 schema 全部不变**（Mac/iOS 零行为变化）。注意：task 行的 `thread_type` 字段值随数据模型修正**有意**从 `"chat"` 变为 `"task"`（影响出口与客户端核实见 §7）。
4. 移除旧「bot 自建线程列表」功能及其扫描实现。

## 1. 综合结论（谁取谁）

| 维度 | 采用 | 说明 |
|---|---|---|
| task 判定 / 数据模型 | **B** | durable `thread_kind="task"` → `recent_threads.thread_type`；写路径修正 + 一次性迁移。否决 A 的 `task_projection` 反连接 |
| API 参数形状 | **A** | `tasks=include\|exclude\|only`，默认 `include`；非法值 400 |
| API/DB 实现 | **B** | 单一 filtered-page 操作（count+page 同分支、同 hop、**同一显式读事务**）、三条静态 SQL 分支、两个 partial index |
| reader seam | **B**（模式锚点用 A） | gateway-neutral trait 注入 router；**无扫描 fallback**，缺 reader = 显式不可用错误；注册模式镜像既有 `TaskProjectionReader`（`tasks.rs:1083`） |
| 列表 UX | **A 骨架 + B 细节** | 绝对序号跨页、页大小 10、越界显式报错、`Current:` 页尾兜底（A）；参数非法回 usage 不进模型、短 id 尾注、首末页提示（B） |
| 绑定命令 | **A** | `/bindthread <n>` 按累积快照绝对序号绑定；否决 B 的 `/thread <row>` |
| 绑定守卫 | **A + B 叠加** | A：下沉 `validate_thread_accepts_bot_binding` 到 router 共用；B：绑定前 SQL 点查 revalidate + `/newthread` 清快照 |
| `/threadprev`/`threadnext` | **B** | 弃用，语义并入 `/threads next|prev` 翻页；一个 release 的隐藏迁移提示后删除。否决 A 的「保留换源」 |
| endpoint 绑定路径 | **B + 评审补强** | 绑定流**四处**全库扫描全量盘点（§3.1）；先建立「每 endpoint ≤1 canonical holder」不变量（一次性去重迁移，§3.2/§4.2）再切投影点查；HTTP bind / `/newthread` / `/bindthread` / detach 共用绑定服务 |
| 移除面 / 测试计划 | **A∪B**，以 B 为主 | B 的引用面盘点更全（re-exports、Weixin fallback 保留项、历史文档不改写原则） |

## 2. 关键分歧裁决（#TASK-2113 评审已逐条核实维持）

### 2.1 task 过滤事实：durable `thread_kind`（B）vs `task_projection` 反连接（A）→ 采 B

- B 方案：修正写路径让 task backing thread 持久携带 `thread_kind="task"`（三个写入点：`create_task` 的 `ThreadEnsureOptions`、`set_task_on_record` ensure、`remove_task_from_record` 保留），复用 `recent_threads.thread_type` 既有列 + 既有派生函数 `thread_summary_type_from_record`；存量行走一次性版本化迁移（§4.1）。
- 裁决理由：① task 删除保留 backing thread（已核实：`garyx-router/src/tasks.rs:625-643`、`remove_task_from_record` :1047-1057、gateway 响应 `thread_retained:true`）——A 的反连接会让保留线程在 task 删除瞬间泄漏回 bot 列表；② `thread_type` 列与派生函数既存，B 是修模型、A 是绕模型；③ 单一过滤事实源。
- 评审核实（#TASK-2113）：前提①（删除保留线程）、②（`create_task` 未写 `thread_kind`，全仓 `thread_kind: Some` 零命中）、③（`thread_title_source="task"` 仅 task 服务写、写入点常量互斥、无假阳性路径）**全部属实**。
- A 的反连接方案与其论证保留为被否决记录（A §3.4/§5.3）。

### 2.2 绑定命令形态：`/bindthread <n>` 绝对序号（A）vs `/thread <row>` 页内序号（B）→ 采 A

- `/thread` 与 `/threads` 一字之差且都带数字参数，翻页与改绑定（有副作用）误触面不可接受；页内序号只认「最后显示页」限制大，绝对序号 + 累积快照允许绑任何浏览过的行。
- 叠加 B 的防护：绑定前 SQL 点查 revalidate（`SELECT 1 FROM recent_threads WHERE thread_id=?1 AND thread_type<>'task'`）；`/newthread` 后清除本 endpoint 快照。
- 快照参数（v2 定量）：内存态、per-endpoint 累积映射（绝对序号 → thread_id+title）**上限 200 条**（20 页浏览深度，超出淘汰最早条目）、endpoint 上下文 LRU 上限 512、**全局条目预算 20,000**（超出淘汰最久未用 context）。单条 ≈300 B（canonical id 44 B + title ≤64 字符 + 结构开销）→ **最坏 ≈6 MB，现实使用（endpoint 数十、翻页数页）< 1 MB**，可接受。重启失效有引导文案。不设 TTL（revalidation 已挡有害情形；绑「一小时前看过且仍有效的行」正是用户所指）。
- 兼容直填 canonical thread id（`thread::<uuid>`）绕过快照，仍过全部守卫（A）。

### 2.3 `/threadprev`/`threadnext`：保留换源（A）vs 弃用（B）→ 采 B

- 保留 = 两套导航心智模型（历史走位 + 翻页列表）+ 一条额外冷重建通路，收益只有肌肉记忆。
- 过渡采 B：parser 保留隐藏识别一个 release，回不可变更的迁移提示（防旧命令跌进模型 run 或静默换绑），菜单立即不展示，之后删除。
- 翻页命令 = `/threads <page>`、`/threads next`、`/threads prev`（B §4.1 语法，含 `/threads@bot_name 2` 寻址形式）。

### 2.4 测试 fallback：`ScanRecentThreadsReader`（A）vs 无扫描 fallback（B）→ 采 B

- 生产 crate 常驻一份契约禁止形态的代码 + parity 维护负担，否决；缺 reader = 显式「temporarily unavailable」错误，任何路径不回落扫描；测试注入确定性 fake reader。

### 2.5 API 参数形状：`tasks=include|exclude|only`（A）vs `is_task` 三态 bool（B）→ 采 A

- 默认值 `include` 自文档化、`only` 语义直读、与 CLI blank 三分严格参数哲学一致；非法取值 400。
- 实现按 B：handler 解析为枚举 → DB 层三条静态 SQL 分支，count 与 page 同分支、同一次 blocking hop、**包在同一显式 read transaction 内**（WAL 下同连接裸跑两条语句不构成同一读快照；顺带修复 `routes.rs:1600-1607` 现存微竞态）；`total`/`has_more` 在过滤域内成立；响应行结构不变、不加新字段；两个 partial index（task / non-task 各一，按 `last_active_at DESC, thread_id ASC`）。

## 3. 架构通路

```
                          +-> GET /api/recent-threads?tasks=...   (HTTP)
recent_threads 过滤分页 SQL +
（GaryxDb 单一实现）        +-> RecentThreadPageReader seam -> router /threads、/bindthread revalidate

thread_channel_endpoints 点查 -> EndpointBindingMutator（绑定服务，唯一绑定变更入口）
    -> /api/channel-bindings/bind、/api/bot/bind、/newthread、/bindthread、detach
```

- router 侧定义 gateway-neutral 类型 + 两个 async trait（`RecentThreadPageReader { page, contains_selectable_thread }`、`EndpointBindingMutator`），gateway 实现包 `GaryxDbService`，`AppStateBuilder::build` 注入（模式先例 `register_task_projection_reader`，`garyx-router/src/tasks.rs:1083`）。不走 loopback HTTP。
- reader trait 只过 bot 格式化所需字段：`thread_id`、`title`、`last_message_preview`、`last_active_at`（v2：**删去 `workspace_dir`**——§5 输出模板不使用；未来若加 workspace 行再随需求加回）。HTTP 响应继续用现有 record/runtime-summary mapper，runtime-summary 附着行为不变。
- **执行路径唯一化（v2，消除 v1 §2.2/§3 歧义）**：`/bindthread` 的执行**只**走 `EndpointBindingMutator` 服务；v1 引用 A 的 `endpoint_binding_for_thread(...) → bind_endpoint_runtime` 路径**作废**（该路径的 ChannelBinding 取/造经 `list_known_channel_endpoints` 扫描合并，见 §3.1-④）。ChannelBinding 构造改为：有投影 holder 行 → 直接从 `thread_channel_endpoints` 行取字段；无 holder（未绑定 endpoint 首次命令）→ 从 inbound 请求/known-endpoint 注册表元数据构造。「缺失即无前 owner」，不是扫描的理由（B §5.2 step 1）。

### 3.1 绑定流全库扫描全量盘点（v2，评审意见 2）

同族扫描共**四处**，逐条处置：

| # | 扫描点 | 位置 | 触发 | 处置 |
|---|---|---|---|---|
| ① | bind 移除旧 holder | `garyx-router/src/threads.rs:603,616-641` | 每次绑定 | **本次改**：投影点查旧 holder + 已知记录点写（§3.2 不变量建立后合法） |
| ② | detach 对称扫描 | `threads.rs:702,710-741` | 解绑 | **本次改**：同① |
| ③ | `sync_endpoint_delivery_timestamp` | `threads.rs:649+`；`bind_endpoint_runtime` 经 `set_last_delivery_with_persistence` / `clear_last_delivery_for_chat_with_persistence` → `persist_delivery_context`（`delivery.rs:53-162`）每次绑定必触发 | 绑定 + 普通消息投递 | **绑定服务内改**：delivery context 持久化只对已知 previous/target 记录点写，不调该扫描 helper。**普通消息投递路径的该 helper 显式 defer**（范围外，独立 follow-up 立项），§8 零扫描断言边界相应收窄 |
| ④ | `endpoint_binding_for_thread` + current-thread 回退 | `garyx-router/src/router/threading/threads.rs:443+,:457` → `list_known_channel_endpoints`（`threads.rs:785+`，注册表+全店扫描合并） | 绑定构造 | **本次改**：`/bindthread`/`/newthread`/HTTP bind 的绑定构造改从投影行/inbound 元数据取（见上），不再进此路径 |

宣称边界（诚实版）：本设计消灭**命令与绑定路径**上的全部扫描（①②③-绑定分支④）；③ 的普通消息投递分支与启动期 `endpoint_thread_map` 重建（`threading/threads.rs:118`，同走 `list_known_channel_endpoints` 扫描合并）为既存问题、显式 defer 并入**同一个 follow-up**（holder 不变量落地后可直接改读 `thread_channel_endpoints`，顺手事）。

### 3.2 endpoint holder 不变量 + 一次性去重迁移（v2，评审意见 1）

- **问题**：现状 bind/detach 扫描从**所有**携带该 endpoint 绑定的记录移除，`is_preferred_thread_binding` 多 holder 择优逻辑即多 holder 状态实际存在的证据（历史数据/boot import 可产生）。`thread_channel_endpoints` 是 `endpoint_key` 单列 PK（`garyx_db/mod.rs:1673-1689`），派生 `ON CONFLICT(endpoint_key) DO UPDATE` 最后写者胜（:2300-2343）——**投影结构上无法表达多 holder**。若直接切点查+单点写：其余 holder 的 canonical 绑定永久无人清理、其记录任何无关写入都会翻转投影行（ghost binding）、`validate_thread_accepts_bot_binding` 会误判「被其它渠道占用」永久拒绝。（TASK-2099 教训：设计 seam 先问旧扫描的边缘基数投影能否表达。）
- **修**：切换点查**之前**，跑一次性版本化迁移 `endpoint_holder_dedup_v1`（与 §4.1 同款边界）：按 endpoint 分组全部 canonical `channel_bindings`，按既有 `is_preferred_thread_binding` 择优规则**留一删余**，单事务完成 canonical 记录更新 + `thread_channel_endpoints` 同步派生，marker 落 `projection_states`，失败 abort 初始化。
- **不变量**：「每 endpoint 在 canonical `thread_records` 中 ≤1 holder」。迁移建立、此后全部绑定变更经序列化的 `EndpointBindingMutator` 自维持。写入 `docs/agents/repository-contracts.md` 更新项（§6 文档面）。
- 服务算法（规格 = B §5.2，在不变量下成立）：点查投影旧 holder → 已知 id 点读目标并校验 → 旧 holder 点写移除 → 目标点写 upsert → 更新内存 map 与 delivery context（已知 id 点写）。不宣称新的跨记录原子语义，保持两次已知记录写。

## 4. 一次性迁移（两个，同款边界）

共同边界：boot import（`import_thread_records_if_needed`）**之后**、开始 serving 之前；版本 marker 落既有 `projection_states`（含 `source_row_count`，零行可记 0）；单事务、set-based SQL（SQLite JSON update 写 canonical + 同事务同步受影响投影）；失败整体 abort 初始化；不进读路径、不进 timer、不走 `list_keys`、不读退役 JSON 归档。这是 `ensure_*_columns`（`mod.rs:2458` 先例）同类的启动迁移，不是契约禁止的 backfill/reconcile 层（#TASK-2113 已核实该论证成立）。

### 4.1 `recent_task_thread_kind_v1`（规格 = B §6.5）

- 证据集：`task_projection.thread_id` ∪ 已有 `thread_kind="task"` 记录 ∪ `thread_title_source="task"`（回收已删 task 的保留线程；**绝不**按 `#TASK-N` 标题前缀猜）。
- 写 canonical `thread_records.body.thread_kind="task"` + 同步 `recent_threads.thread_type` / `thread_meta.thread_type`；保留 `updated_at`/`last_active_at`/标题（分类迁移不是用户活动）。
- 迁移前已删 task 且被手动改名的历史线程无法可靠回收——不猜；rollout 后 durable kind 使歧义不再产生。

### 4.2 `endpoint_holder_dedup_v1`（v2 新增，规格见 §3.2）

## 5. 命令 UX 规格

页大小 10，绝对序号，输出模板（渠道中立纯文本）：

```text
Recent threads · page 2/4 (32 total)
11. Fix login flow [a1b2c3d4] ⬅️
12. Weekly report draft [e5f6a7b8]
...
20. Refactor thread store [c9d0e1f2]

/threads 3 → next · /bindthread <n> → switch · /newthread → create
```

- 行 = 绝对序号 + title（title 为派生占位时回退 `last_message_preview` 截 48 字；whitespace/控制字符折叠，Unicode 安全截断）+ 短 id 尾注（display-only，不回解析）+ 当前线程 ` ⬅️`。
- 当前绑定线程不在本页/被过滤：页尾 `Current: {title}`（脚注，不注入行）。
- 空列表 / `page<1` / 非数字 / 越界：分别为引导文案、usage、usage、显式报错（`Page 7 is out of range (4 pages). Use /threads 4.`），不静默钳位；参数非法一律本地回复不进模型。
- `next`/`prev` 基于本 endpoint 最近一次成功显示页；无状态（含重启后）从第 1 页开始；首/末页给 `Already on the first/last page.` 提示并重渲染。
- 投影读失败：`Recent threads are temporarily unavailable. Try again.`，绝不回落扫描。
- 群聊暴露面（v2 补全）：全局列表跨 surface 可见属任务指定方向（与 Mac/iOS 同数据面）；暴露内容含线程**标题**与占位回退时的 **`last_message_preview` 消息片段**。`/threads` 执行处已有 `is_group`，未来要收紧有便宜开关位（A §5.2），本次不做。
- 命令目录：可见项 `newthread`、`threads`（`args_hint="[page|next|prev]"`）、`bindthread`（`args_hint="<n>"`）；`reserved_command_names` 增 `bindthread`（过渡期保留 `threadprev`/`threadnext`）；telegram 菜单经既有 `sync_command_menu_once` 自动 resync。

## 6. 移除面（A §4 ∪ B §3.4/§7，#TASK-2113 核实无漏项无误删）

- `navigation.rs`：`thread_matches_binding`、`latest_message_text_for_thread`（仅 navigation 内部消费）、`summarize_thread_list_text`、`fallback_thread_list_label`、`list_binding_threads`、`ensure_thread_history`、`navigate_thread`、`navigate_thread_with_rebuild`、`list_user_threads_for_account`。
- `contracts.rs::ThreadListEntry` 及 `router/mod.rs`、`lib.rs` re-exports（公开符号，仓内无消费者，release note 提及）。
- `threading/mod.rs::binding_thread_history/binding_thread_index`、`router/mod.rs::MAX_HISTORY`；`switch_to_thread` 简化为只更新当前绑定映射。
- `local_commands.rs` 三个旧分支重写为 `ListRecent(PageRequest)` / `BindRecent{n}` / deprecated 变体；catalog/inbound 旧变体更新。
- 绑定服务落地后：`threads.rs` bind/detach 的扫描体、`sync_endpoint_delivery_timestamp` 的绑定路径调用、`endpoint_binding_for_thread` 的扫描合并分支（§3.1 处置表为准）。
- **保留**：`latest_assistant_message_text_for_thread`（Weixin 投递 fallback，`weixin.rs:4337`）；通用 current-thread map（普通路由/回复路由/auto-recovery/`/newthread` 在用）。
- 测试面：router `tests/inbound|navigation|dispatch`、`telegram/tests.rs`、`feishu/tests.rs`、`gateway/commands/tests.rs` 相应重写。
- 文档面：更新 `docs/architecture/command-list-design.md`（命令语法/菜单/弃用提示）、`docs/concepts/threads-and-workspaces.md`（`tasks` 参数）、`docs/agents/repository-contracts.md`（durable task `thread_kind`/`thread_type` 不变量 + **endpoint ≤1 holder 不变量** + bot 命令走 reader seam）；历史设计文档不改写。

## 7. 兼容影响

- **Mac/iOS 默认请求**：成员集、排序、分页、信封、行 schema 零变化（不发 `tasks` 参数）；mobile 既有 URL 断言（只发 limit/offset）保留为回归测试。
- **`thread_type` 值变化（有意，v2 明确）**：迁移 + 写路径修正后，task 线程在一切经 `thread_summary_type_from_record` 的出口值从 `"chat"` 变 `"task"`。受影响出口清单：默认 `/api/recent-threads` 行、`/api/threads`、`GET /api/threads/:key`（`routes.rs:1399/1514`）、`api.rs:1077` 摘要、`automation.rs:1132` snapshot、runtime_context `thread.kind`（:105）。客户端核实结论（#TASK-2113）：**无任何行为开关**——desktop 仅 memo 等值比较（`threads.ts:533`/`thread-model.ts:87`）、iOS recent 行模型不解码 `thread_type`、task-forest 用自己的 kind 字段。该变化正是本设计的目的（task 身份可见、可过滤）。
- 产品语义（有意）：`/threads` 从「本 endpoint 自建/关联线程」变为「全局 recent 非 task 线程」。
- 用户自定义 shortcut 撞 `bindthread`/过渡期保留名：按既有 reserved-name 遮蔽机制 + release note。
- 重启后快照失效：`/threads <page>` 永远可用，绑定需先重新列一页。
- gateway HTTP 面：`/api/bot/bind`、`/api/channel-bindings/*` 外部契约不变（内部实现换绑定服务）。
- 多 holder 存量数据经 `endpoint_holder_dedup_v1` 一次性收敛（择优规则与现状读取偏好一致，用户可见绑定不变）。

## 8. 测试计划（headless 优先；A §6 ∪ B §9 + v2 增补）

- **router 纯函数**：parser（页码/next/prev/`@bot_name` 寻址/非法参数/溢出）、pager 边界、formatter（空页/单页/多页绝对序号/越界/`Current:` 尾行/preview 回退/Unicode 截断）、快照状态机（命中/未浏览/缺失/直填 id/条目与 context 双上限淘汰/`newthread` 清除）、守卫链（不存在/task 拒绝/跨渠道跨账号拒绝/幂等）、deprecated 命令零变更、缺 reader 显式错误且零扫描。fake reader + fake binding backend。
- **gateway SQLite**：三档过滤的行集/排序/`total`/`has_more`/offset 钳位；过滤先于 limit/offset；count+page 同一显式读事务（并发写入下一致性用例）；task 创建单事务写出 canonical+两投影 task kind；task 删除保留 kind；`recent_task_thread_kind_v1` 全套用例（legacy overlay 升级、title-source 回收、`#TASK-N` 标题不误伤、零 task 记完成、首启 import 先于 marker、失败原子、不重跑）；**`endpoint_holder_dedup_v1` 全套用例（v2）**：多 holder 种子 → preferred 保留其余清除、投影单行与 canonical 一致、幂等不重跑、失败原子；两个 partial index 查询计划断言。
- **默认响应回归（v2 拆分）**：种子含 task 行时，非 task 行**字节不变** + task 行**仅 `thread_type` 值变化**（其余字段字节不变）；无 task 种子时整响应字节不变。
- **零扫描 instrumented 断言（v2 收窄边界）**：`/threads`、`/bindthread`、`/newthread`、HTTP bind/detach 与 `GET /api/recent-threads?tasks=exclude` 全路径无 `list_keys`/记录体遍历；普通消息投递路径不在断言范围（§3.1-③ defer 项）。
- **route 层**：省略/exclude/only/非法 400/隐藏与 generated-automation 排除不变。
- **绑定服务**：点查旧 holder 零扫描、双记录点写后单一投影 owner 且**无 ghost 残留**（对**旧 owner** 记录写无关字段后，投影仍指向 target）、delivery context 已知 id 点写、幂等、旧 owner 缺失/目标不兼容/写失败/串行并发的显式结果；HTTP bind 入口与 `/newthread`、`/bindthread` 共享行为；无 holder 的首次绑定从 inbound 元数据构造。
- **渠道 e2e（薄）**：telegram 菜单目录（含弃用项不可见）、`/threads` 含他端创建行且排除 task、`/threads next` + `/bindthread` 绑定精确 id 且后续消息落目标线程、feishu 同款冒烟 + 寻址形式、一个 subprocess-plugin host 用例证明参数穿透。
- 消费端兼容：mobile URL 断言（只发 limit/offset）+ 既有解码测试。
- 验收 `rg`：旧符号与旧输出串清零（弃用识别器与文案除外）。

## 9. 实现顺序（B §10 + v2 调整）

1. task-kind 写路径修正 + `recent_task_thread_kind_v1` 迁移（数据契约先独立正确）；
2. 过滤分页 DB 函数（显式读事务）+ HTTP `tasks` 参数 + 默认兼容回归；
3. reader seam 注入 + 纯 parser/pager/formatter/快照状态机；
4. **`endpoint_holder_dedup_v1` 迁移 → 投影化绑定服务**（含 delivery context 点写与绑定构造改造），既有 bind/detach 调用方迁移；
5. 替换原生命令（列表/绑定/弃用提示）；
6. 渠道/目录测试与活文档更新（含 repository-contracts 两条不变量）；
7. 删除全部旧符号（§3.1 处置表 + §6），跑聚焦 + fast tier 验证。

该顺序保证任何时刻不会 ship 一个泄漏 task 线程的列表或一个产生 ghost binding 的绑定服务。
