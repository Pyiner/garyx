# Agent 启用开关 + 全局默认 Agent

日期：2026-07-16 · 状态：设计评审中（v5，回应 #TASK-2320 第四轮 6 条阻断；前三轮 17 条已关闭）

## 1. 需求与产品裁决（用户原话为准）

1. agent 增加启用/停用开关：**不开启的 agent 不可选**（各端 picker）、**CLI 也不可派活**。
2. **list 时明确展示可用性**（CLI、桌面、iOS 的管理列表）。
3. 多渠道搞定（gateway / CLI / desktop / mobile / bot 渠道 / automation）。
4. 支持**设置默认 agent**：默认 agent = 每次新建线程默认选中的那个。
5. **停用语义裁决（2026-07-15 用户明示）**：停用时，该 agent 名下**已开始/已绑定它的线程全部不受影响，可以继续调**；只是**不能新建线程、不能新建 task**。

**统一判据（v5 起全文以此为准）：enabled 只约束「产生新绑定」的动作；不产生新绑定的动作（纯配置、既有绑定的使用/派生）一律不受 enabled 与全停用状态约束。**

## 2. 现状事实（四轮评审核验后版本，实现时逐条再核对）

- `CustomAgentProfile`（`custom_agent.rs:9`）无 enabled；`CustomAgentStore` 持久化 `custom-agents.json` 裸 map（load `:69`、persist `:289-297` 过滤 built-in）；`delete_agent`（`:273-283`）可删任意 custom（含未来的 raw default）。
- `resolve_agent_reference`（`agent_reference.rs:39`）被已有线程续跑消费（`prepare.rs:173`、`lifecycle.rs:422`、`run_management.rs:383`）——enabled 检查不能放这里。
- 默认解析现状：requested → current → `"claude"`（`agent_identity.rs:13,15`）；automation update 依赖 current（`automation.rs:503`）。
- **claude 物化/哨兵点全清单（四轮补齐）**：
  - task（`garyx-router/tasks.rs:423`）、cron（`cron.rs:1227-1229`，None/空串/纯空白都视为 claude，`automation.rs:494-500` 同）、CLI channels（`channels.rs:1147`）；
  - channel account = **四类 typed（Telegram/Discord/Feishu/Weixin）+ `ApiAccount`，共五类**，serde 默认 claude（`config.rs:264,325`）、`ApiAccount.agent_id` 必填 String（`:443`）、onboard 显式写 claude（`onboard.rs:55`）、router 把账号值当 requested（`garyx-router/threads.rs:839`）；
  - desktop settings/channel-setup 补 claude（`gateway-settings.ts:259`、`channel-setup.ts:101`）；desktop 共享序列化器删除显式 `"claude"`（`gateway-settings.ts:146,179`）；
  - Web shell 物化 claude（`use-web-settings-state.ts:41`、`WebSettingsPage.tsx:387,609`）；`build:web` 目标在 `package.json:17`；
  - bridge 把 plugin account None 解成 claude（`multi_provider/lifecycle.rs:168`）；bot 读 API 把 None 输出为空串（`routes.rs:3376,3448`）；
  - desktop route claude 哨兵（`desktop-route.ts:185,223`）；
  - **desktop side-chat fork：`side-chat-ops.ts:84` `sourceThread?.agentId || pendingAgentId || "claude"`——legacy 无 agentId 源线程会把 claude 物化成显式 requested 传入 Fork**。
- **automation 现状**：`AutomationSummary.agent_id` 必填 String（`automation.rs:101`）；**target-existing automation 不创建线程、用目标线程自身 agent（`automation.rs:858-874` 目标存在即跳过 agent 校验；`cron.rs:1256-1269` 仅 generated 进中央创建）**；`CronService::load` 先读磁盘、再被 `garyx.json` config jobs 覆盖 `agent_id`、最后持久化（`cron.rs:748-781`）——**config 是权威覆盖源，磁盘一次性迁移会被 merge 覆盖**；`InternalDispatch` jobs 合法使用 `agent_id=None`（`schedule_followup.rs:172-187`、`quota_resend.rs:186-207`）；desktop 编辑恒发显式 agentId（`useAutomationController.ts:38,265`）、disabled 重插可选 unavailable（`AutomationDialog.tsx:376`）；iOS `ensureEditAgentSelection` 静默换绑（`GaryxMobileAutomationViews.swift:533,558`）、解码补 claude（`GaryxGatewayAutomationModels.swift:77`）。
- **bridge 状态面**：`multi_provider/state.rs:22-32` 只有 agent profiles + provider configs，无 availability/default 快照；`replace_agent_profiles` 只收 `Vec<CustomAgentProfile>`、`reload_from_config` 只收 `&GaryxConfig`——**bridge 目前没有获知 raw/effective default 的通道**。
- task 工作区：中央创建应用 agent `default_workspace_dir`（`agent_identity.rs:104`）而 `TaskService::create_task` 绕过（`gateway/tasks.rs:199,867`）。
- 新建入口穷尽（四轮 grep 确认）：直连中央 creator 的 HTTP 创建/fork/recovered session/cron generated；经 `GatewayThreadCreator` 的 bot 入站、`/api/chat/start`、`/newthread`、task 通知首建；唯一绕行 `TaskService::create_task`（五 agent 来源）。router 失败现状：Claude fallback（`threading/threads.rs:269`）、虚构 key（`:290`）、planning 无错误通道、`/newthread` 压扁文案。
- **bot 读 API 客户端现状**：desktop `DesktopBotConsoleSummary`/mapper 连 `agent_id` 都不保留；iOS bot models 只解析 `agentId`；Web settings 无获取全局 effective 的数据通道。
- 管理面快捷动作：iOS Chat/Use（`GaryxMobileAgentsViews.swift:1419`）；desktop Chat（`AgentsHubPanel.tsx:597`、`AgentFormDialog.tsx:680`）。
- 客户端顶层字段：desktop `agents.ts:200`、iOS `GaryxGatewayClient.swift:478` 丢弃；desktop `pendingAgentId` 必填 string（`AppShell.tsx:652`）、iOS 选中态必填 string（`GaryxMobileModel.swift:299`）。
- 验证：desktop `test:unit` 不做 TS 编译（须 `build:ui`）+ Web shell `build:web`；iOS App-target 须 `xcodebuild`。

## 3. 设计

### 3.1 数据模型、持久化、共享解析层与 **bridge 注入通道**

- `CustomAgentProfile` 加 `enabled: bool`，serde 默认 true。
- `custom-agents.json` 升 versioned envelope（判别：顶层数字 `version` + 对象 `agents`；否则 legacy 裸 map，load 迁移、persist 恒写 v2）：`{ "version": 2, "agents": {...}, "disabled_builtin_ids": [...], "default_agent_id": "codex" }`。
- **garyx-models 唯一解析实现**：`AgentAvailabilitySnapshot`（`[(agent_id, enabled, standalone, built_in, default_workspace_dir, provider/runtime 快照字段)] + default_agent_id`）+ 纯函数 `resolve_effective_default(snapshot)` / `ensure_enabled_for_new_binding(snapshot, id)`。
- **bridge 注入通道（回应四轮 #2）**：`replace_agent_profiles` 演进为携带完整 `AgentAvailabilitySnapshot` 的单一调用（如 `replace_agent_state(snapshot)`），快照在 **store 同一次锁内**构造；toggle / set-default / CRUD / delete 后原子推送刷新。bridge 内 plugin account None 的解析（3.3）与 gateway 走同一纯函数、同一快照。禁止 bridge 自行读文件或第二来源。
- `default_agent_id` 与 agent 同库同锁，不进 `garyx.json`。

### 3.2 API

- `GET /api/custom-agents`：每项 `enabled`；顶层 `default_agent_id`（raw，可 null）+ `effective_default_agent_id`（nullable）；management list 恒 200。
- `PATCH /api/custom-agents/{id}/toggle` `{"enabled": bool}`：幂等 set、无并发 token、custom 推进 `updated_at`、同锁原子持久化。
- `PATCH /api/custom-agents/{id}/default`：校验存在 + standalone + enabled；无 unset 端点。
- **删除 raw default（回应四轮 #3）**：`DELETE /api/custom-agents/{id}` 命中 raw default 时**同锁原子清空 `default_agent_id`**（不拒绝删除；raw → null，effective 走 fallback）。badge 语义见 3.6。
- `PUT /api/custom-agents/{id}` `enabled: Option<bool>` 三态：create 缺省 true、update 缺省保留。
- 变更后按 3.1 推送 bridge 快照 + reload。
- **bot/channel 读 API 继承态**：None 输出 `agent_id: null` + `effective_agent_id`（`routes.rs:3376,3448` 改）。**客户端传播点名（回应四轮 #6）**：desktop `DesktopBotConsoleSummary` 及 mapper 增加 `agentId`（nullable）+ `effectiveAgentId` 并入契约校验；iOS `GaryxConfiguredBot` 系列 models 增加可选 `effectiveAgentId` 解码；Web settings 以 bot 读 API 的 `effective_agent_id`（或 custom-agents list 顶层 effective）为"跟随全局（当前 X）"标签数据源。三端各配解码/展示测试。

### 3.3 默认解析链与"未指定"三态合同

解析链（garyx-models 纯函数，保留 current 档）：

```
requested（显式；新建绑定 disabled → AgentDisabled，绝不 fallback）
→ current（既有绑定，enabled 不参与——裁决 5）
→ default_agent_id（enabled 且 standalone）
→ "claude"（enabled 时）
→ 第一个 enabled 的 standalone（built-in 播种序 → custom 按 agent_id 字典序）
→ 无可用（隐式新建 → NoEnabledAgent；effective → null）
```

**channel account `agent_id` 三态合同**（`Some(id)`=显式 override 含 claude；`None`=继承全局）：

- config：五类 account（四 typed + Api）统一 `Option<String>`，serde 默认 None；已持久化显式值不迁移。
- 写入方拆物化/拆删除：onboard；desktop `gateway-settings.ts:259`/`channel-setup.ts:101` 不补 claude、**序列化器不得删除显式 "claude"**（`:146,179`）；Web shell 不物化、表单 null=继承 + 显式保值（退役与否须用户拍板，默认修复）；CLI channels 同理。
- 读取方：router `default_agent_for_channel_account` 返回 Option，None 走默认档；bridge plugin None 走 3.1 注入的同一快照/纯函数（`lifecycle.rs:168` 改）。
- **Add Bot / 渠道账号配置是纯配置动作，不产生新绑定：全停用时照常可保存**（继承档标签显示"跟随全局（当前无可用）"）；首次入站要新建线程时由服务端 gate 拒绝并回明确文案（回应四轮 #1）。
- desktop route null 哨兵（claude 为普通 override）；**desktop side-chat fork 物化拆除**（`side-chat-ops.ts:84`：源线程无 agentId 时传 null 交服务端解析，不物化 claude）。
- task/cron/CLI channels 提前物化拆除。

**automation 的 agent 合同（v5 按模式拆分，回应四轮 #1/#4）**：

- **generated（`CronJobKind::AutomationPrompt` 且生成新线程）**：agent 恒显式。create 省略 `agent_id` → 按 effective 解析落显式值（全停用时 create 该模式 400 NoEnabledAgent）；显式 disabled → 400；cron 派发经中央 Fresh gate。
- **target-existing（指向既有线程）**：**agent 从目标线程绑定派生，不要求 enabled、不受全停用限制**（`automation.rs:858-874` 现状即跳过校验，保持）；UI 不因全停用禁用该模式创建。
- **恒显式不变量仅限 `AutomationPrompt`**：`InternalDispatch` jobs 的 `agent_id=None` 合法且不受迁移影响。
- **legacy 归一化 = merge 后的常驻不变量，非一次性磁盘迁移**：`CronService::load` 在磁盘读入 + config jobs 覆盖**之后**、使用之前，把 AutomationPrompt 的 None/空串/纯空白 归一为显式 `"claude"`（严格守恒今日 `automation.rs:494-500`/`cron.rs:1227-1229` 行为）并随既有持久化落盘；config 权威源可能持续供给 None，故归一化每次启动幂等执行，不写回 `garyx.json`（不动用户配置文件）。
- 客户端编辑合同（既有）：`agentChanged` 跟踪、未动省略 `agent_id`；disabled current 只读 missing 不可选（desktop `AutomationDialog.tsx:376` 不可选化、iOS 禁静默换绑）；generated 新建默认 = effective、选项 enabled-only。

其余：automation update 显式改 disabled → 400；停用当前默认不改写 raw；允许停到一个不剩。

### 3.4 拦截点

**a) 创建意图**：`create_thread_for_agent_reference` 带意图——`Fresh` 拦、`Fork` 拦、`RecoverExistingSession` 豁免。

**b) typed 错误**：`AgentDisabled(agent_id)` + `NoEnabledAgent`。同等保证：不进 Claude fallback、不虚构 key、planning 补错误通道、`/newthread`/渠道明确文案。覆盖 bot 入站、`/api/chat/start` 无 thread、`/newthread`、task 通知首建（已有通知线程照常）。已绑定 bot 返回既有 thread 不拦。

**c) TaskService gate**：garyx-router trait `NewTaskAgentGate`，输出 **`ResolvedAgentBinding`（canonical id + provider/runtime 快照 + `default_workspace_dir`）**；TaskService 落 record 前应用「显式 workspace → agent 默认 → 无」，与中央创建 `agent_identity.rs:104` 等价；`TaskService::new` 必填构造（fail-closed）；gateway 注入 store 背书实现；五来源全覆盖。API 层保留友好 400。

**d) assign**：新绑定 disabled 拒绝；既有同-agent task 返工放行。

**e) 明确不拦**（反向验证）：既有线程续聊/steer、thread send 返工、bridge 回填、recovered session、已绑定 bot、**target-existing automation（创建与运行）**、纯配置动作（Add Bot/渠道账号保存）。

### 3.5 CLI

- `garyx agent list`：人读加启用/默认注记；`--json` 每项 `enabled` + 顶层 raw/effective（可 null）。
- `garyx agent enable|disable <id>`；`garyx agent default [<id>]`。
- `task create --agent` / `thread create --agent` 透传 typed 错误。
- channels picker：共享 loader + 纯解析、滤 `standalone && enabled`、默认建议 = effective、"跟随全局"档位（写 None）；全停用时仍可保存继承档配置。

### 3.6 桌面端

- AgentsHubPanel：Enabled 列 + 默认标记 + "Set default"；disabled 行隐藏/禁用 Chat、Set default（详情框同）。
- **默认标记语义**：badge 锚定 raw；raw 停用 → raw 行 muted "Default (inactive)" + effective 行 muted "Acting default"；**raw 为 null（未设或因删除清空）→ 无 raw badge，effective 行 muted "Default (auto)"**。CLI 同语义。
- **全停用空态（v5 收窄为只限新绑定）**：`effective=null` 时——**新线程草稿** composer 空态禁发并提示去启用（**已有线程 composer 完全不受影响**）；generated automation 新建禁用带提示；**target-existing automation 创建、Add Bot/渠道账号保存照常可用**；管理面恒可用。`pendingAgentId` 改 nullable。
- picker disabled 隐藏；新建草稿默认 = effective；Chat（enabled）一次性 override；**side-chat fork 不物化 claude**（3.3）。
- AddBotDialog/渠道下拉：默认建议 = effective、排除 disabled、"跟随全局（当前 X / 当前无可用）"档位、已绑定 disabled `.missing` 置灰；序列化器保值显式 claude；Web shell 按 3.3。
- bot console 契约/映射增加 `agentId` + `effectiveAgentId`（3.2）；automation 表单按 3.3；route null 哨兵；client 解析顶层双字段。

### 3.7 iOS

- 管理列表：StatusPill + swipe Enable/Disable；disabled 行隐藏 Chat/Use；默认标记同 3.6（raw/inactive/acting/auto 四态）。
- picker：`makeTargets` 滤 `.enabled`；bot picker 默认建议 = effective + "跟随全局"档位。
- **全停用空态限新线程草稿**：选中态改 nullable（`GaryxMobileModel.swift:299`），新建草稿空态禁发提示；**已有线程 composer 不受影响**；target-existing automation/bot 配置照常。
- 两态拆分：`gatewayDefaultAgentId` 只读缓存（`Use` = set-default → 刷新）+ 草稿一次性 override；移除全部旁路持久写入。
- automation：编辑合同按 3.3；解码补 claude 删除（服务端归一化后 AutomationPrompt 无缺失）。
- models：`GaryxAgentSummary`/`GaryxCustomAgentRequest` 加 `enabled`（request 三态）；bot models 加 `effectiveAgentId`；`GaryxCachedAgent` 镜像（`decodeIfPresent` 默认 true）+ snapshot 升 version；client 解析顶层双字段。
- 默认解析/回退纯逻辑下沉 Core 配 SwiftPM 测试。

### 3.8 兼容性

- legacy 裸 map → load 迁移写 v2（单向）；channel 显式值不迁移、缺失按 None 继承；automation AutomationPrompt None/空白 → merge 后归一显式 claude（每启动幂等，不写回 garyx.json）。
- 旧 catalog 快照不匹配即丢弃；不做旧网关兼容；老客户端 PUT 缺 enabled 由三态保护；服务端 gate 兜底。

## 4. 测试与验证计划（headless 优先）

- **存储/解析**：envelope 往返、legacy 迁移（含 `version`/`default` 命名 agent）；解析链全分支 + 稳定序 + default 停用回退 + 全停用 effective=null 且 list 200；**删除 raw default 同锁清空 + effective fallback + badge 状态**。
- **API**：toggle（updated_at → stale PUT 409）、set-default 校验、PUT 三态、list 双字段、bot API None → null + effective。
- **bridge 快照**：toggle/set-default 后原子刷新；**默认 claude→codex 热切换后 plugin account None 路由同步变化**。
- **创建 gate**：Fresh/Fork/Recovered 差异化；bot 入站不 fallback/不虚构 key/明确文案；`/api/chat/start`、`/newthread`、task 通知首建；TaskService 五来源 + 必填构造；assign 区分；全停用 → NoEnabledAgent。
- **task 工作区**：human `start=true` 全局默认带 `default_workspace_dir` → 落该工作区，与中央路径等价对照。
- **channel 三态 round-trip**：显式 "claude" 经 desktop 序列化、Web shell 表单、CLI 写回保值；None=inherit 生效；缺失守恒；bridge plugin None 走 resolver。
- **automation**：**generated vs target-existing 拆分**（后者 disabled/全停用下创建与运行均正常）；归一化（None/空串/空白 → claude；config 覆盖源下**二次启动幂等**；InternalDispatch None 不受影响）；create 省略 agent 落显式 effective；四层展示与运行一致；停用后只改名不拒绝不改绑（三层反例）。
- **反向（不拦）**：disabled 既有线程续聊、thread send 返工、target-existing automation、generated 该次失败错误可见、**已有线程 composer 在全停用下照常可发**。
- **desktop**：agent-options 过滤、默认预选、AddBotDialog、disabled 行动作、route 哨兵、client 顶层映射、bot console 双字段映射、**side-chat legacy 源 fork 不物化**、全停用空态（限新草稿）；`test:unit` + `build:ui` + `build:web`。
- **iOS**：Core SwiftPM（过滤、decodeIfPresent、快照丢弃、两态、bot picker、nullable 选中态、bot models effective 解码）+ App target `xcodebuild`。
- **CLI**：list、enable/disable/default、channels picker、task create disabled 报错。

## 5. 修订记录

- v5（2026-07-16）：回应四轮 6 条——①统一判据「enabled 只约束新绑定」：target-existing automation 从目标线程派生不受限、Add Bot 纯配置全停用可保存、空态只限新线程草稿；②bridge 注入通道：同锁快照单一调用推送 + 热切换测试；③删除 raw default 同锁清空 + badge 四态（含 raw null 的 auto 态）；④automation 归一化改 merge 后常驻幂等不变量（含空串/空白），范围限 AutomationPrompt，不写回 garyx.json；⑤补 side-chat fork 物化点、修正账户类数（五类）与 build:web 行号；⑥bot effective 字段客户端传播点名（desktop console 契约/iOS models/Web 数据源）+ 三端测试。
- v4（2026-07-16）：channel 三态传播补全（序列化器/Web shell/bridge/bot API）；automation 恒显式 + 迁移（v5 修正为按模式拆分）；ResolvedAgentBinding 带工作区；badge 与全停用空态（v5 收窄）。
- v3（2026-07-15）：automation 客户端合同；channel 三态；TaskService 跨 crate fail-closed；NoEnabledAgent；管理面 disabled 行动作。
- v2（2026-07-15）：创建意图豁免 recover；typed AgentDisabled 贯穿 router；TaskService 咽喉；versioned envelope；解析链保留 current；CLI channels picker；iOS 两态；PUT 三态 + toggle updated_at。
