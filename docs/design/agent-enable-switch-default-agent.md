# Agent 启用开关 + 全局默认 Agent

日期：2026-07-16 · 状态：设计评审中（v4，回应 #TASK-2320 第三轮 4 条阻断；前两轮 13 条已关闭）

## 1. 需求与产品裁决（用户原话为准）

1. agent 增加启用/停用开关：**不开启的 agent 不可选**（各端 picker）、**CLI 也不可派活**。
2. **list 时明确展示可用性**（CLI、桌面、iOS 的管理列表）。
3. 多渠道搞定（gateway / CLI / desktop / mobile / bot 渠道 / automation）。
4. 支持**设置默认 agent**：默认 agent = 每次新建线程默认选中的那个。
5. **停用语义裁决（2026-07-15 用户明示）**：停用时，该 agent 名下**已开始/已绑定它的线程全部不受影响，可以继续调**；只是**不能新建线程、不能新建 task**。

## 2. 现状事实（三轮评审核验后版本，实现时逐条再核对）

- `CustomAgentProfile`（`custom_agent.rs:9`）无 enabled；存储 = gateway `CustomAgentStore`，`custom-agents.json` 裸 map（load `:69`、persist `:289` 过滤 built-in）；id 只校验非空。
- `resolve_agent_reference`（`agent_reference.rs:39`）被已有线程续跑消费（`prepare.rs:173`、`lifecycle.rs:422`、`run_management.rs:383`）——enabled 检查不能放这里。
- 默认解析现状：requested → current → `"claude"`（`agent_identity.rs:13,15`）；automation update 依赖 current（`automation.rs:503`）。
- **claude 物化/哨兵点全清单（三轮补齐）**：
  - task（`garyx-router/tasks.rs:423`）、cron（`cron.rs:1222,1227`）、CLI channels（`channels.rs:1147`）；
  - 五类内置 channel account serde 默认 claude（`config.rs:264,325`）、`ApiAccount.agent_id` 必填 String（`:443`）、onboard 显式写 claude（`onboard.rs:55`）、router 把账号值当 requested（`garyx-router/threads.rs:839`）；
  - desktop settings/channel-setup 补 claude（`gateway-settings.ts:259`、`channel-setup.ts:101`）；**desktop 共享序列化器会把显式 `"claude"` 删除**（`gateway-settings.ts:146,179`）；
  - **独立 Web shell** 把缺失物化成 claude（`use-web-settings-state.ts:41`），UI 是带 claude fallback 的自由文本（`WebSettingsPage.tsx:387,609`）；有正式 `build:web` 目标（`package.json:15`）；
  - **bridge 把 plugin account 的 None 解成 claude**（`multi_provider/lifecycle.rs:168`）；bot API 把 None 输出为空串（`routes.rs:3376,3448`）；
  - desktop route 把 claude 当"未指定"哨兵（`desktop-route.ts:185,223`）。
- **automation 现状**：`AutomationSummary.agent_id` 必填 String（`automation.rs:101`）；`CronJob.agent_id=None` 展示为 claude（`automation.rs:494,585`）、iOS 解码静默补 claude（`GaryxGatewayAutomationModels.swift:77`）；desktop 编辑恒发显式 agentId（`useAutomationController.ts:38,265`）、disabled 项重插成可选 unavailable（`AutomationDialog.tsx:376`）；iOS `ensureEditAgentSelection` 静默换绑（`GaryxMobileAutomationViews.swift:533,558`）。
- **task 工作区语义**：gateway 只为显式 executor/assignee 提前解析默认工作区（`gateway/tasks.rs:199`）；human `start=true` 默认 agent 在 router 更晚产生（`garyx-router/tasks.rs:423`）；中央创建会应用 agent `default_workspace_dir`（`agent_identity.rs:104`）而 TaskService 绕过；创建后修补只写 provider/runtime 不补工作区（`gateway/tasks.rs:867`）。
- **新建入口穷尽**（三轮 grep 确认）：直连中央 creator 的 HTTP 创建/fork/recovered session/cron generated-thread；经 `GatewayThreadCreator` 的 bot 入站、`/api/chat/start` 无 thread、`/newthread`、task 通知首建；唯一绕行 = `TaskService::create_task`（五个 agent 来源）。router 失败现状：Claude fallback（`threading/threads.rs:269`）、虚构 key（`:290`）、`build_dispatch_plan` 无错误通道、`/newthread` 压扁文案。
- 跨 crate：`TaskService` 在 garyx-router（依赖 `tasks.rs:370` 三项），router 不能反向依赖 gateway。
- 管理面快捷动作：iOS agent 行 Chat/Use（`GaryxMobileAgentsViews.swift:1419`）；desktop 表格/详情 Chat（`AgentsHubPanel.tsx:597`、`AgentFormDialog.tsx:680`）。
- 客户端顶层字段：desktop `agents.ts:200`、iOS `GaryxGatewayClient.swift:478` 均丢弃；desktop 草稿 `pendingAgentId` 必填 string（`AppShell.tsx:652`）、iOS 选中态必填 string（`GaryxMobileModel.swift:299`）。
- 验证规则：desktop `test:unit` 不做 TS 编译（须 `build:ui`），Web shell 须 `build:web`；iOS App-target 文件须 `xcodebuild`（`validation.md:19,29`）。

## 3. 设计

### 3.1 数据模型、持久化与共享解析层

- `CustomAgentProfile` 加 `enabled: bool`，serde 默认 true。
- `custom-agents.json` 升 versioned envelope（判别：顶层数字 `version` + 对象 `agents`；否则 legacy 裸 map，load 迁移、persist 恒写 v2）：

```json
{ "version": 2, "agents": { ... }, "disabled_builtin_ids": [...], "default_agent_id": "codex" }
```

- **garyx-models 承载唯一解析实现**：
  - `AgentAvailabilitySnapshot`：`[(agent_id, enabled, standalone, built_in, default_workspace_dir, provider/runtime 快照所需字段)] + default_agent_id`（**v4：快照含 `default_workspace_dir` 等完整绑定信息**，见 3.4c）；
  - 纯函数 `resolve_effective_default(snapshot) -> Option<agent_id>` 与 `ensure_enabled_for_new_binding(snapshot, id) -> Result<(), AgentBindingError>`；
  - gateway store、TaskService gate、automation、CLI 离线 picker 同一条链。
- `default_agent_id` 与 agent 同库同锁，不进 `garyx.json`。

### 3.2 API

- `GET /api/custom-agents`：每项带 `enabled`；顶层 `default_agent_id`（raw，可 null）+ `effective_default_agent_id`（**nullable，全停用为 null**）；management list **恒 200**。
- `PATCH /api/custom-agents/{id}/toggle` `{"enabled": bool}`：幂等 set、无并发 token、custom 推进 `updated_at`、同锁原子持久化。
- `PATCH /api/custom-agents/{id}/default`（空 body）：校验存在 + standalone + enabled；无 unset 端点，"可解析性"由 effective 可空表达。
- `PUT /api/custom-agents/{id}` `enabled: Option<bool>` 三态：create 缺省 true、update 缺省保留。
- 变更后 `bridge.replace_agent_profiles` + reload。
- **bot/channel 读 API 的继承态表达**（回应三轮 #1）：账号 `agent_id` 为 None 时**不得输出空串**（`routes.rs:3376,3448` 改），输出 `agent_id: null` + `effective_agent_id`（按当时快照解析），客户端据此展示"跟随全局（当前 X）"。

### 3.3 默认解析链与"未指定"三态合同

解析链（garyx-models 纯函数，保留 current 档）：

```
requested（显式；新建绑定 disabled → AgentDisabled，绝不 fallback）
→ current（既有绑定，enabled 不参与——裁决 5）
→ default_agent_id（enabled 且 standalone）
→ "claude"（enabled 时）
→ 第一个 enabled 的 standalone（稳定序：built-in 播种序 → custom 按 agent_id 字典序）
→ 无可用（隐式创建 → NoEnabledAgent；effective → null）
```

**channel account `agent_id` 三态合同——v4 补全传播面**（`Some(id)`=显式 override 含 claude；`None`=继承全局）：

- config 模型：五类内置 account + `ApiAccount` 统一 `Option<String>`，serde 默认 None；已持久化显式值（含 "claude"）不迁移。
- 写入方逐一拆物化/拆删除：onboard（`onboard.rs:55`）；desktop `gateway-settings.ts:259`/`channel-setup.ts:101` 不补 claude；**desktop 共享序列化器不得删除显式 `"claude"`**（`gateway-settings.ts:146,179`——round-trip 必须保值，"删默认值"逻辑与三态合同不相容，移除）；CLI channels 同理。
- **Web shell 纳入同一合同**（`use-web-settings-state.ts:41` 不物化；`WebSettingsPage.tsx:387,609` 自由文本改为 null=继承 + 显式值原样保留，展示 effective 提示）；若产品决定退役 Web shell 须用户拍板，本设计默认修复而非排除。
- 读取方：router `default_agent_for_channel_account` 返回 Option，None 走 resolver 默认档而非 requested 档；**bridge plugin account 的 None 不再解成 claude**（`lifecycle.rs:168`），改走同一 resolver。
- desktop/iOS 渠道设置下拉加"Default（跟随全局，当前 X）"档位；desktop route 以 null 为未指定哨兵（`desktop-route.ts:185,223`），claude 是普通显式 override。
- task/cron/CLI channels 的提前物化一并拆除。

**automation 的 agent 合同——v4 拍板：恒显式存储 + legacy 一次性迁移**（回应三轮 #2）：

- 与 channel account 相反，automation **不引入继承态**：`CronJob.agent_id` 恒为显式值。理由：automation 是"用户建时选定 agent 的计划任务"，动态跟随全局默认会让已有计划悄悄换 agent，风险大于收益；恒显式则 list/编辑展示恒真实。
- **legacy 迁移**：load 时把 `agent_id=None` 的既有 automation 一次性落为显式 `"claude"`（= 今日实际运行与展示行为，严格守恒），幂等持久化；迁移后不存在 None。
- API create 允许省略 `agent_id`：创建时按 effective default 解析并**落显式值**（创建时物化是"用户此刻的默认"，列表立即真实可见；与线程创建的运行时解析不同，不留悬空引用）。
- 由此 `AutomationSummary.agent_id` 保持必填、iOS 解码补 claude 逻辑删除（迁移后无缺失）；cron 派发路径恒有显式 agent，`cron.rs:1222,1227` 的物化点随迁移消亡。
- 客户端编辑合同（v3 既有）：跟踪 `agentChanged`、未动则 update 省略 `agent_id`（服务端 current 档）；disabled current 只读 missing 展示不可选（desktop `AutomationDialog.tsx:376` 不可选化、iOS `ensureEditAgentSelection` 禁静默换绑）；新建默认 = effective default、选项 enabled-only。

其余（同 v3）：automation update 显式改 disabled → 400；停用当前默认不改写 raw；允许停到一个不剩。

### 3.4 拦截点

**a) 创建意图**：`create_thread_for_agent_reference` 带意图——`Fresh` 拦、`Fork` 拦、`RecoverExistingSession` 豁免。

**b) typed 错误贯穿 router**：`AgentDisabled(agent_id)` + `NoEnabledAgent`（隐式创建且全停用）。两者同等：不进 Claude fallback（`threads.rs:269`）、不走虚构 key（`:290`）、`build_dispatch_plan` 补错误通道、`/newthread`/渠道回明确文案。覆盖 bot 入站、`/api/chat/start` 无 thread、`/newthread`、task 通知首建（该 bot 新会话整体不可用，首建失败记可见错误；已有通知线程照常）。已绑定 bot 返回既有 thread 不拦。

**c) TaskService gate（v4：`ResolvedAgentBinding` 携带完整绑定信息）**：

- garyx-router 定义 trait `NewTaskAgentGate`：输入候选绑定（五来源）与意图，输出 `Result<ResolvedAgentBinding, AgentBindingError>`；
- **`ResolvedAgentBinding` = canonical agent_id + provider/runtime metadata 快照 + `default_workspace_dir`**（回应三轮 #3：中央创建在 `agent_identity.rs:104` 应用 agent 默认工作区，TaskService 绕行路径必须等价）；TaskService 在落 record 前应用：显式 workspace → agent `default_workspace_dir` → 无；
- `TaskService::new` 把 gate 作为**必填构造参数**（fail-closed by construction）；测试注入显式 stub；
- gateway 注入 `CustomAgentStore` 背书实现（内部调 garyx-models 纯函数）；human auto-start 默认档同链解析。
- API 层 `resolve_task_executor_agent` 保留友好 400；强制线在 `TaskService::create_task`。

**d) assign**：新绑定 disabled 拒绝；既有同-agent task 返工放行。

**e) 明确不拦**（反向验证）：既有线程续聊/steer、thread send 返工、bridge 回填、recovered session、已绑定 bot、target-existing automation。

### 3.5 CLI

- `garyx agent list`：人读加启用/默认注记；`--json` 每项 `enabled` + 顶层 raw/effective（可 null）。
- `garyx agent enable|disable <id>`；`garyx agent default [<id>]`（显示 raw+effective / 设置）。
- `task create --agent` / `thread create --agent` 透传 `AgentDisabled`/`NoEnabledAgent`。
- channels add/login picker：garyx-models 共享 loader + 纯解析（离线可用）、滤 `standalone && enabled`、默认建议 = effective、提供"跟随全局"（写 None）。

### 3.6 桌面端（v4：badge 语义 + 空态合同）

- AgentsHubPanel：Enabled 列（Switch）+ 默认标记 + "Set default"（disabled 行隐藏/禁用 Chat、Set default；详情对话框同）。
- **默认标记语义**（回应三轮 #4）：**"Default" badge 锚定 raw**（用户配置的偏好）；当 raw 被停用（raw≠effective）时该 badge 转 muted 样式加注 "inactive"，**当前 effective agent 行加 muted 次级标记 "Acting default"**。CLI 同语义：raw 行 `default`、失活加 `(inactive)`、effective 行 `acting default`。
- **全停用空态合同**：`effective=null` 时——新建线程 composer 的 agent picker 显示"无可用 agent"空态、发送/创建禁用并提示去 Agents 管理启用；Add Bot、automation 新建同样禁用带提示；管理面恒可用（list 200）以便重新启用。**`pendingAgentId` 由必填 string 改为 nullable**，null 即空态。
- picker disabled 隐藏（`agent-options.ts` 与 standalone 同层）；新建草稿默认 = effective（替换硬编码初值/reset 点）；Chat（enabled agent）保持一次性 override。
- AddBotDialog/渠道设置下拉：默认建议 = effective、排除 disabled、"跟随全局（当前 X）"档位、已绑定 disabled 的账号 `.missing` 置灰；共享序列化器保值显式 claude（3.3）。
- Web shell 渠道表单按 3.3 三态修复。
- automation 表单按 3.3 客户端合同；desktop-route null 哨兵；client 解析顶层双字段。

### 3.7 iOS（v4：badge/空态对齐桌面）

- 管理列表：`GaryxStatusPill` Enabled/Paused + swipe Enable/Disable；disabled 行隐藏 Chat/Use；默认标记同 3.6 语义（raw badge + inactive 态 + acting default 次级标记）。
- picker：`makeTargets` 滤 `.enabled` → 四处 picker 全排除；bot picker 默认建议 = effective + "跟随全局"档位。
- **全停用空态**：选中态改 nullable（`GaryxMobileModel.swift:299` 必填 string 拆掉），composer/新建入口显示空态并禁发、提示去 Agents 启用；管理面可用。
- 两态拆分：`gatewayDefaultAgentId` 只读缓存（`Use` = set-default → 刷新）+ 草稿一次性 override；移除全部旁路持久写入。
- automation：编辑合同按 3.3；解码补 claude 删除（迁移后无缺失）。
- `GaryxAgentSummary`/`GaryxCustomAgentRequest` 加 `enabled`（request 三态）；`GaryxCachedAgent` 镜像（`decodeIfPresent` 默认 true）+ snapshot 升 version（旧快照安全丢弃）；client 解析顶层双字段。
- 默认解析/回退纯逻辑下沉 Core 配 SwiftPM 测试。

### 3.8 兼容性

- legacy 裸 map → load 迁移写 v2（单向）；channel 显式值不迁移、缺失按 None 继承；**automation legacy None → 显式 claude 一次性迁移（行为守恒）**。
- 旧 catalog 快照不匹配即丢弃。
- 不做旧网关兼容；老客户端 PUT 缺 enabled 由三态保护；服务端 gate 兜底。

## 4. 测试与验证计划（headless 优先）

- **gateway 存储/解析**：envelope 往返、legacy 迁移（含 `version`/`default` 命名 agent）；解析链全分支 + 稳定序 + default 停用回退 + 全停用 effective=null 且 list 200。
- **gateway API**：toggle（updated_at 推进 → stale PUT 409）、set-default 校验、PUT 三态、list 双字段；**bot/channel 读 API None → `agent_id: null` + `effective_agent_id`（非空串）**。
- **创建 gate**：Fresh/Fork/Recovered 差异化；bot 入站不 fallback/不虚构 key/明确文案；`/api/chat/start`、`/newthread`、task 通知首建；TaskService 五来源全拦 + gate 必填构造；assign 新旧绑定区分；全停用 → NoEnabledAgent。
- **task 工作区**（三轮 #3）：human `start=true` 不指定 agent/workspace、全局默认 custom agent 带 `default_workspace_dir` → task 落该工作区；与中央创建路径行为等价对照。
- **channel 三态 round-trip**（三轮 #1）：显式 "claude" 经 desktop 设置序列化、Web shell 表单、CLI 写回后**原样保值**；None=inherit 生效（onboard 的 api:main 吃全局默认）；字段缺失老配置行为守恒；bridge plugin None 走 resolver。
- **automation**（三轮 #2）：legacy None 迁移落显式 claude（幂等）；create 省略 agent → 落显式 effective；列表/编辑展示与实际运行一致（gateway/CLI/desktop/iOS 四层）；停用后只改名不拒绝不改绑（三层反例）。
- **反向（不拦）**：disabled 既有线程续聊、thread send 返工、target-existing automation 继续、generated automation 该次失败错误可见。
- **CLI**：list（raw+effective+注记）、enable/disable/default、channels picker、task create disabled 报错。
- **desktop**：agent-options 过滤、默认预选、AddBotDialog、disabled 行动作禁用、route 哨兵（全局 codex + 显式 claude 刷新不还原）、client 顶层映射、**全停用空态（composer 禁用+提示）**；`npm run test:unit` + `npm run build:ui` + **`npm run build:web`**。
- **iOS**：Core SwiftPM（过滤、decodeIfPresent、快照丢弃、两态、bot picker 默认、**nullable 选中态空态逻辑**）+ App target `xcodebuild`。

## 5. 修订记录

- v4（2026-07-16）：回应三轮 4 条——①channel 三态传播补全：desktop 序列化器禁删显式 claude、Web shell 纳入合同（含 build:web 验证）、bridge plugin None 走 resolver、bot API None 输出 null+effective；②automation 拍板恒显式存储 + legacy None 一次性迁移 claude（行为守恒），create 省略即落显式 effective，展示恒真实；③`ResolvedAgentBinding` 携带 default_workspace_dir 等完整绑定信息，TaskService 与中央创建等价；④默认标记锚定 raw + inactive/acting-default 语义、全停用客户端空态合同（两端选中态 nullable、禁发提示、管理面恒可用）。
- v3（2026-07-15）：automation 客户端合同；channel account 三态；TaskService 跨 crate（纯解析 + 必填 gate）；全停用服务端定义（NoEnabledAgent）；管理面 disabled 行动作。
- v2（2026-07-15）：创建意图豁免 recover；typed AgentDisabled 贯穿 router；TaskService 咽喉；versioned envelope；解析链保留 current；CLI channels picker；iOS 两态；PUT 三态 + toggle updated_at。
