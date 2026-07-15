# Agent 启用开关 + 全局默认 Agent

日期：2026-07-15 · 状态：设计评审中（v2，逐条回应 #TASK-2320 第一轮 FAIL 的 8 条阻断）

## 1. 需求与产品裁决（用户原话为准）

1. agent 增加启用/停用开关：**不开启的 agent 不可选**（各端 picker）、**CLI 也不可派活**。
2. **list 时明确展示可用性**（CLI、桌面、iOS 的管理列表）。
3. 多渠道搞定（gateway / CLI / desktop / mobile / bot 渠道 / automation）。
4. 支持**设置默认 agent**：默认 agent = 每次新建线程默认选中的那个。
5. **停用语义裁决（2026-07-15 用户明示）**：停用时，该 agent 名下**已开始/已绑定它的线程全部不受影响，可以继续调**；只是**不能新建线程、不能新建 task**。

## 2. 现状事实（v2 已按评审核验修正，实现时逐条再核对）

- `CustomAgentProfile`（`garyx-models/src/custom_agent.rs:9`）无 enabled 字段；仅 `built_in`、`standalone`（默认 true）。
- 存储 = `CustomAgentStore`（`garyx-gateway/src/custom_agents.rs:43`），持久化 `~/.garyx/data/custom-agents.json`；**当前文件格式是裸 `HashMap<agent_id, profile>`**（load `custom_agents.rs:69`，persist `:289`）；built-in 内存播种、persist 时过滤；agent id 目前只校验非空（`:179`），**任意字符串都是合法 id**（含 `default`、`version` 等）。`PUT /api/custom-agents/{agent_id}` 路由使 `default` 也是合法可更新 id（`route_graph.rs:214`）。
- 校验咽喉 `resolve_agent_reference`（`agent_reference.rs:39`）**确实被已有线程续跑路径消费**（chat prepare `prepare.rs:173`；bridge 每次 run 回填并 resolve `lifecycle.rs:422`、`run_management.rs:383`）——enabled 检查不能放这里（裁决 5）。
- 无全局默认：`DEFAULT_AGENT_REFERENCE_ID = "claude"`（`agent_identity.rs:13`）；`selected_agent_reference_id` 链 = **requested → current → "claude"**（`:15`），automation update 依赖 `current` 保持原 agent（`automation.rs:503`）。
- **多处在进 resolver 前把空值提前物化成 "claude"**：task（`garyx-router/src/tasks.rs:423`）、cron（`cron.rs:1222`）、CLI channels（`garyx/src/commands/channels.rs:1147`）——全局默认要生效必须拆掉这些提前物化。
- **新建线程/新建 task 的完整生产入口**（第一轮评审穷尽 grep 核对）：
  - 直连 `create_thread_for_agent_reference`：HTTP 普通创建、fork、**recovered session**（`routes.rs:2642/2779/2812`）、cron generated-thread（`cron.rs:1269`）。
  - 经 `GatewayThreadCreator`（`app_bootstrap.rs:317` 注入）：普通 bot/API 入站、**无 thread 的 `/api/chat/start`**（`prepare.rs:609`）、`/newthread`、**task 通知首建 bot thread**（`task_notifications.rs:254`）。
  - **绕过中央 creator**：`TaskService::create_task` 直接 `create_thread_record`（`garyx-router/src/tasks.rs:412/423/440`）；task 的 agent 来源有五：executor（`tasks.rs:171`）、`runtime.agent_id`（`:189`）、agent assignee（`:192`）、auto-start actor、human auto-start 硬编码 claude。
  - router 入站失败行为：非 Claude 配置**自动 fallback 到 Claude**（`router/threading/threads.rs:269`）、两败后**返回 `new_thread_key()` 而非错误**（`:290`）、`build_dispatch_plan` 无错误通道（`planning.rs:435`）、`/newthread` 压扁错误文案（`local_commands.rs:301`）。已绑定 bot 创建前直接返回既有 canonical thread（`threads.rs:240`）。
  - **CLI channels add/login 有独立 agent picker，直接读裸 `custom-agents.json` 文件**、只过滤 standalone（`channels.rs:1041/1089`）。
- 开关先例：MCP `PATCH .../toggle`（无并发 token）、skills toggle、channel account `enabled`。
- 桌面：AgentsHubPanel 表格；picker 过滤维度已有 standalone（`agent-options.ts`）；新建草稿初值 `pendingAgentId="claude"` 硬编码（`AppShell.tsx:652`，reset 2845/3157/3335）；**AddBotDialog 也硬编码优先 claude**（`AddBotDialog.tsx:224`）；desktop client 的 list 映射丢弃响应顶层字段（`agents.ts:200`）。
- iOS：本地默认 `selectedAgentTargetId`（UserDefaults，gateway-scoped）；**写入者不止 `Use`**：create/update 成功后 set、`setNewThreadAgentTarget` 持久分支（`+AgentsWorkspaces.swift:339`）、catalog 同步 fallback `ensureSelectedAgentTarget`（`+StateSync.swift:27`）；bot 设置也有 agent picker 且硬编码 claude（`GaryxMobileBotSettingsViews.swift:301`）；iOS client 同样丢弃 list 顶层字段（`GaryxGatewayClient.swift:478`）；catalog 缓存 restore **要求版本精确相等，不等即删除旧快照**（`+CatalogCache.swift:35`），`GaryxCachedAgent` 是 synthesized Codable（缺 key 不会自动用属性默认值）。

## 3. 设计

### 3.1 数据模型与持久化（v2：versioned envelope）

- `CustomAgentProfile` 加 `enabled: bool`，serde 默认 `true`。
- **`custom-agents.json` 升级为 versioned envelope**，不在裸 map 上加顶层字段：

```json
{ "version": 2,
  "agents": { "<agent_id>": { ...profile... } },
  "disabled_builtin_ids": ["antigravity"],
  "default_agent_id": "codex" }
```

- 格式判别：顶层对象含 `version`（数字）**且** `agents`（对象）→ v2；否则按 legacy 裸 map 解码（legacy 里即使存在名为 `version`/`agents` 的 agent，其 value 是 profile 对象而非数字/agent-map，判别不歧义）。load 到 legacy 即在内存迁移，**下次 persist 一律写 v2**；不回写 legacy。
- envelope 的解析/序列化放 **garyx-models**（新模块，如 `custom_agent_state.rs`），gateway store 与 CLI 共用同一 loader（解决 CLI 直读文件在新格式下解析炸掉的问题，见 3.5）。
- built-in 停用状态记 `disabled_builtin_ids`；custom 停用状态记在各自 profile 的 `enabled`；load 时对播种 built-in 应用 disabled 列表。
- `default_agent_id` 与 agent 同库同锁同持久化，不进 `garyx.json`（应用状态非配置，且避免 settings deep-merge 成为无校验旁路写入）。

### 3.2 API（v2：路径不占用 agent-id 命名空间；PUT 三态；toggle 刷新 updated_at）

- `GET /api/custom-agents`：每个 agent 带 `enabled`；响应顶层加 **`default_agent_id`（raw，可 null）+ `effective_default_agent_id`（按 3.3 解析后的生效值）**。desktop/iOS client 现在丢弃顶层字段，需改为解析这两个字段。
- `PATCH /api/custom-agents/{agent_id}/toggle`，body `{"enabled": bool}`：built-in 与 custom 一致可用；幂等 set 语义；无并发 token；**对 custom 必须同时推进 `updated_at`**（使停用后旧表单携带的 `expected_updated_at` 条件更新 409 失效，防止静默复活）；同锁内原子持久化。
- **设默认 = `PATCH /api/custom-agents/{agent_id}/default`**（空 body）：把该 agent 设为默认；校验存在 + standalone + enabled，否则 400。资源子路径不占用 agent-id 命名空间（v1 的 `PUT /api/custom-agents/default` 与合法 id `default` 冲突，废弃）。不提供 unset（默认恒可解析，见 3.3）。
- `PUT /api/custom-agents/{agent_id}` 的 payload `enabled` 为 **`Option<bool>` 三态**：create 缺省 → `true`；update 缺省 → **保留现值**（防旧客户端表单不发 `enabled` 把刚停用的 agent 静默重新启用）。
- toggle / set-default 成功后与现有 CRUD 一致：`bridge.replace_agent_profiles` + reload。

### 3.3 默认解析链（v2：保留 current；拆掉提前物化；定义稳定顺序）

完整链（`selected_agent_reference_id` 演进，**保留 current 档**）：

```
requested（显式指定；新建绑定时 disabled → 拒绝，绝不 fallback）
→ current（update/续用类流程保持既有绑定，enabled 不参与判断——既有绑定即裁决 5 范畴）
→ 存储 default_agent_id（存在 且 enabled 且 standalone）
→ "claude"（enabled 时）
→ 第一个 enabled 的 standalone agent（稳定顺序：built-in 按播种序 claude/codex/traex/antigravity，再 custom 按 agent_id 字典序；不依赖可编辑的 display_name）
→ 明确错误（全停用时新建失败，不静默兜底）
```

- **拆掉提前物化**：task（`garyx-router/tasks.rs:423`）、cron（`cron.rs:1222`）、CLI channels（`channels.rs:1147`）等把空值提前写成 `"claude"` 的点全部改为传 None 透传 resolver，否则全局默认永远轮不到。
- automation update 不动 `agent_id` → `current` 档保持原绑定（即使已停用，改无关字段不触发改绑也不报错）；显式改成 disabled agent → 400。
- 停用当前默认：**不改写存储的 `default_agent_id`**，解析时跳过走 fallback；重新启用自动恢复。
- 允许停到一个不剩：新建报明确错误，不做"最后一个不许停"防护。

### 3.4 拦截点（v2：创建意图分类 + typed 错误贯穿 + TaskService 咽喉）

**a) 中央创建函数带意图**。`create_thread_for_agent_reference` 增加意图参数：

| 意图 | enabled gate |
|---|---|
| `Fresh`（HTTP 普通创建、cron generated-thread） | 拦 |
| `Fork` | 拦（fork 是新建线程） |
| `RecoverExistingSession` | **豁免**（恢复既有会话 = 既有绑定，裁决 5；v1 无差别拦会误伤 `routes.rs:2642/2779` 恢复路径） |

**b) typed 错误贯穿 router**。新增结构化 `AgentDisabled` 错误（区别于 UnknownAgent，携带 agent_id），贯穿 `resolve_or_create_inbound_thread → resolve_thread_for_request / build_dispatch_plan → 渠道回复 / API 响应`：

- `AgentDisabled` **不得进入** router 现有的 Claude fallback（`threads.rs:269`）——用户配了 A，A 停用 ≠ 悄悄换 B；
- 不得走"两败返回 `new_thread_key()`"的虚构 thread id 路径（`threads.rs:290`）——失败即错误，`build_dispatch_plan` 侧补错误通道；
- `/newthread` 不压扁文案（`local_commands.rs:301`），渠道内回明确"agent 已停用"提示；
- 覆盖普通 bot 入站、**无 thread 的 `/api/chat/start`**、`/newthread`、**task 通知首建 bot thread**（`task_notifications.rs:254`：bot 的 agent 停用则该 bot 新会话整体不可用，通知首建线程同样失败并**记录可见错误日志**；已有通知线程照常投递——此为设计决策，可被否决）。
- 已绑定 bot 直接返回既有 canonical thread（`threads.rs:240`）的路径**不拦**（既有绑定）。

**c) task 的咽喉在 `TaskService::create_task`**。API 层 `resolve_task_executor_agent` 保留做友好 400，但**强制 gate 落在 `TaskService::create_task` 最终绑定处**（`garyx-router/tasks.rs:412/423/440` 直接 `create_thread_record`，是唯一能覆盖全部五个 agent 来源的位置：executor / runtime.agent_id / agent assignee / auto-start actor / human auto-start 默认）。任何将绑定到**新** task thread 的 agent 必须 enabled。

**d) assign 区分新旧绑定**（v1 过宽）：

- 新绑定（task thread 尚未绑定该 agent，或改绑到另一 agent）→ disabled 拒绝；
- **task thread 已绑定同一 disabled agent 的再 assign / 返工 → 放行**（既有绑定继续调，裁决 5）。

**e) 明确不拦**（实现与评审都要有反向验证）：已有线程发消息/续跑/steer、`thread send` 唤醒已有 task、bridge 对已绑定 agent 的 run 派发与 metadata 回填、recovered session、已绑定 bot 的既有线程、target-existing automation 继续运行。

### 3.5 CLI

- `garyx agent list`：人读输出加启用状态与默认标记（`disabled` / `default` 注记）；`--json` 每项加 `enabled`，顶层加 `default_agent_id` + `effective_default_agent_id`。
- 新增 `garyx agent enable <id>` / `garyx agent disable <id>`（走 toggle 端点）。
- 新增 `garyx agent default [<id>]`：无参显示 raw + effective 两个值；有参设置（走 set-default 端点）。
- `garyx task create --agent <disabled>` / `garyx thread create --agent <disabled>`：透传服务端 `AgentDisabled` 明确报错。
- **channels add/login 的 agent picker**（`channels.rs:1041/1089`）：改用 garyx-models 共享 envelope loader（网关不在也能读），过滤 `standalone && enabled`；默认建议值改为 effective default 而非硬编码 claude。

### 3.6 桌面端

- AgentsHubPanel：新增 **Enabled 列**（Switch，走 toggle API）+ **Default badge** + 行动作 "Set default"。
- picker（ComposerForm 等）：**disabled 直接隐藏**，与 standalone 过滤同层（`agent-options.ts`）；管理面才展示停用态。不选置灰方案，减少 picker 噪音。
- 新建线程草稿默认选中 = `effective_default_agent_id`：替换 `AppShell.tsx` `pendingAgentId` 硬编码初值与全部 reset 点（652/2845/3157/3335）。`Chat` 保持一次性 override。
- **AddBotDialog**（`AddBotDialog.tsx:224`）与渠道设置 agent 下拉：默认建议 = effective default，排除 disabled；**已绑定** disabled agent 的既有账号行沿用 `.missing` 式置灰（可见不丢）。
- desktop client（`agents.ts`）解析 list 顶层 `default_agent_id` / `effective_default_agent_id`。

### 3.7 iOS（v2：拆两态 + 枚举全部本地默认写入者）

- 管理列表：行加 `GaryxStatusPill` Enabled/Paused + swipe Enable/Disable（模板=bot 行）。
- picker：`makeTargets` 增加 `.filter(\.enabled)` → 新线程 sheet / target popover / automation agentRow / **bot 设置 agent picker** 全部排除 disabled；bot picker 默认建议改 effective default（`GaryxMobileBotSettingsViews.swift:301`）。
- **状态拆两个**，取代单一 `selectedAgentTargetId`：
  1. `gatewayDefaultAgentId`：gateway `effective_default_agent_id` 的**只读本地缓存**（离线/冷启预选用），唯一写入路径 = 服务端同步；`Use` 动作 = 调 set-default 端点 → 成功后刷新缓存。
  2. 新线程草稿 override（现有 pending 机制）：一次性，不持久。
- **移除全部旁路持久写入**（v1 只删 create/update 不够）：create/update 成功后的 set、`setNewThreadAgentTarget` 持久分支、`ensureSelectedAgentTarget` 的 fallback 写入（`+StateSync.swift:27`）——fallback 改为只影响展示态，不落盘。`Use` 成为语义上唯一的默认修改入口（经由 gateway）。
- `GaryxAgentSummary`/`GaryxCustomAgentRequest` 加 `enabled`（request 侧三态可选，对齐 3.2）；`GaryxCachedAgent` 镜像 `enabled`（**显式 `decodeIfPresent` 默认 true**，不依赖 synthesized Codable）+ snapshot 升 version；**兼容承诺修正：旧版本快照被安全丢弃、冷启走网络重建**（restore 版本精确匹配是现有行为，不承诺跨版本加载）。
- 默认解析/回退纯逻辑（含 disabled 跳过）下沉 `GaryxMobileCore` 配 SwiftPM 测试。

### 3.8 兼容性

- 旧裸 map `custom-agents.json` → load 时迁移、persist 写 v2（单向，不支持回滚到旧二进制后再读 v2 文件——与本仓"不做旧版本兼容设计"裁决一致）。
- 旧 catalog 缓存快照：版本不匹配即丢弃重建（现有行为），不承诺跨版本加载。
- 不做旧网关兼容设计；老客户端不发 `enabled` 的 PUT 由 3.2 三态语义保护；老客户端仍显示 disabled agent 可选时，服务端 gate 兜底拒绝。

## 4. 测试计划（headless 优先；v2 合入评审列出的缺口）

- **gateway 存储**：v2 envelope 往返；legacy 裸 map 迁移（含名为 `version`/`default` 的 agent id 不歧义）；builtin disabled + default_agent_id 持久化。
- **gateway API**：toggle（built-in/custom；custom 推进 `updated_at`，stale `expected_updated_at` PUT 409）；set-default 校验（不存在/disabled/非 standalone → 400）；PUT 三态（update 缺省 `enabled` 保留停用态）；list 返回 raw + effective。
- **默认解析**：requested/current/default/claude/first-enabled/error 全分支；default 停用回退；fallback 顺序稳定性（改 display_name 不影响）；提前物化拆除后 task/cron/channels 真正吃到全局默认。
- **创建 gate**：Fresh/Fork 拦、**Recovered 豁免**三类差异化；bot 入站 disabled 不 fallback 不虚构 thread id、渠道收到明确文案；`/api/chat/start` 无 thread、`/newthread`、task 通知首建 thread 的可见错误；`TaskService::create_task` 五种 agent 来源全拦；assign 新绑定拦 / **既有同-agent task 返工放行**。
- **反向（不拦）**：disabled agent 既有线程继续发消息成功；`thread send` 返工成功；target-existing automation 继续跑；generated automation 该次失败且错误可见；automation 无关 update 不改绑。
- **CLI**：list 人读/`--json`（raw+effective）、enable/disable/default 命令、channels picker 走共享 loader 且过滤 disabled、task create --agent disabled 报错。
- **iOS Core SwiftPM**：makeTargets enabled 过滤；`GaryxCachedAgent` decodeIfPresent 默认；旧版本快照丢弃路径；两态拆分后 Use/同步/草稿 override 纯逻辑；bot picker 默认建议。
- **desktop**：`agent-options` 过滤、默认预选来源、AddBotDialog 默认建议（`npm run test:unit`）。

## 5. 修订记录

- v2（2026-07-15）：回应 #TASK-2320 第一轮 FAIL——①创建意图分类豁免 recovered session；②typed `AgentDisabled` 贯穿 router、禁 Claude fallback/虚构 thread id，补 `/api/chat/start` 与 task 通知首建；③task gate 下沉 `TaskService::create_task` 覆盖五来源，assign 区分新旧绑定；④持久化改 versioned envelope，设默认改 `PATCH .../{id}/default` 避开 id 命名空间；⑤解析链保留 current、拆提前物化、定义稳定 fallback 序、API 加 effective 值；⑥补 CLI channels picker（共享 loader）与双端 Add Bot；⑦iOS 拆两态、枚举全部旁路写入、缓存兼容承诺改丢弃重建；⑧PUT enabled 三态 + toggle 推进 updated_at。
