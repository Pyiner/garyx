# Agent 启用开关 + 全局默认 Agent

日期：2026-07-16 · 状态：设计评审中（v6，回应 #TASK-2320 第五轮 4 条阻断；前四轮 23 条已关闭）

## 1. 需求与产品裁决（用户原话为准）

1. agent 增加启用/停用开关：**不开启的 agent 不可选**（各端 picker）、**CLI 也不可派活**。
2. **list 时明确展示可用性**（CLI、桌面、iOS 的管理列表）。
3. 多渠道搞定（gateway / CLI / desktop / mobile / bot 渠道 / automation）。
4. 支持**设置默认 agent**：默认 agent = 每次新建线程默认选中的那个。
5. **停用语义裁决（2026-07-15 用户明示）**：停用时，该 agent 名下**已开始/已绑定它的线程全部不受影响，可以继续调**；只是**不能新建线程、不能新建 task**。

**统一判据：enabled 只约束「产生新绑定」的动作；纯配置、既有绑定的使用/派生一律不受约束。模式/形态转换若使一个 agent 将被用于未来的新线程，即视为新绑定。**

## 2. 现状事实（五轮评审核验后版本，实现时逐条再核对）

- `CustomAgentProfile`（`custom_agent.rs:9`）无 enabled；`CustomAgentStore` 持久化 `custom-agents.json` 裸 map（load `:69`；persist `:289-297` 过滤 built-in）；`delete_agent`（`:273-283`）可删任意 custom。**store 变更顺序 = 先改内存再写盘**（upsert `:254`、delete `:281`），`atomic_write` 只防文件撕裂（`atomic_write.rs:6`），写失败不回滚内存。
- `resolve_agent_reference`（`agent_reference.rs:39`）被已有线程续跑消费（`prepare.rs:173`、`lifecycle.rs:422`、`run_management.rs:383`）——enabled 检查不能放这里。
- 默认解析现状：requested → current → `"claude"`（`agent_identity.rs:13,15`）；automation update 依赖 current（`automation.rs:503,948,967`）。
- **外部 metadata 通道（五轮新发现）**：`/api/chat/start` 与 WS start 接受任意 `metadata`（`contracts.rs:38`），外部边界只剥 `provider_env`（`prepare.rs:120`），agent metadata 用 `entry.or_insert` 客户端值优先（`prepare.rs:196`）；bridge 直接消费 `metadata.agent_id`（`run_management.rs:80,403`）；`CreateThreadBody.metadata`（`routes.rs:901`）同类，中央 snapshot 合并保留调用方 `agent_id`（`agent_identity.rs:50`）。
- **claude 物化/哨兵点全清单**：task（`garyx-router/tasks.rs:423`）、cron（`cron.rs:1227-1229`；`automation.rs:494-500`；None/空串/纯空白均视 claude）、CLI channels（`channels.rs:1147`）；channel account = 四 typed + `ApiAccount` 共五类，serde 默认 claude（`config.rs:264,325`）、`ApiAccount.agent_id` 必填（`:443`）、onboard 写 claude（`onboard.rs:66`）、router 把账号值当 requested（`garyx-router/threads.rs:839`）；desktop `gateway-settings.ts:259`/`channel-setup.ts:101` 补 claude、序列化器删显式 claude（`gateway-settings.ts:146,179`）；Web shell 物化（`use-web-settings-state.ts:41`、`WebSettingsPage.tsx:387,609`；`build:web` 在 `package.json:17`）；bridge plugin None 解 claude（`lifecycle.rs:168`）；bot 读 API None 输出空串（`routes.rs:3376,3448`）；desktop route claude 哨兵（`desktop-route.ts:185,223`）；side-chat fork 物化（`side-chat-ops.ts:84`）。
- **automation 现状**：`AutomationSummary.agent_id` 必填（`automation.rs:101`）；target-existing 不创建线程、运行读目标线程（`automation.rs:858-874` 跳过校验；`cron.rs:1256-1269` 仅 generated 进中央创建；**运行取目标线程 `cron.rs:1938`，而摘要展示 job 字段 `automation.rs:585`**）；**target 创建省略 agent 时服务端/桌面会物化 claude 并显式存入 job**（`automation.rs:867,791`、`useAutomationController.ts:260`）——"目标线程 Codex、job 显式 Claude"是现存合法数据；**update 清除目标后用 current 解析**（`automation.rs:948,967`）——target→generated 转换可保留 disabled current；`CronService::load` 磁盘 → config jobs 覆盖 → 持久化（`cron.rs:748-781`）；`AutomationPrompt` 还含 Log job（`cron.rs:1531`）；`InternalDispatch` 的 None 合法（`schedule_followup.rs:172-187`、`quota_resend.rs:186-207`）；desktop 编辑恒发显式 agentId（`useAutomationController.ts:38,265`）、disabled 重插可选（`AutomationDialog.tsx:376`）；iOS 静默换绑（`GaryxMobileAutomationViews.swift:533,558`）、解码补 claude（`GaryxGatewayAutomationModels.swift:77`）。
- **bridge 状态面**：topology 与 agent profiles **不同锁**（`state.rs:22`），`replace_agent_profiles` 只更新一张表（`multi_provider.rs:118`），API 随后独立 reload topology（`api.rs:2841`）——两阶段有乱序/半应用窗口。
- task 工作区：中央创建应用 `default_workspace_dir`（`agent_identity.rs:104`）而 TaskService 绕过（`gateway/tasks.rs:199,867`）。
- 新建入口穷尽（五轮确认）：中央 creator 直连 = HTTP Fresh/Fork/Recover（`routes.rs:2812`）+ generated cron（`cron.rs:1269`）+ `GatewayThreadCreator`（`agent_identity.rs:158`，生产强制注入 `app_bootstrap.rs:325`，覆盖 bot 入站 / `/api/chat/start` / `/newthread` / task 通知首建）；唯一 raw 绕行 = `TaskService::create_task`（`tasks.rs:440`，五 agent 来源 `:412-426`）。router 失败现状：Claude fallback（`threading/threads.rs:269`）、虚构 key（`:290`）、planning 无错误通道、`/newthread` 压扁文案。
- bot 读 API 客户端：desktop `DesktopBotConsoleSummary`/mapper 不保留 agent_id；iOS bot models 只解析 agentId；Web settings 无全局 effective 数据通道。
- 管理面快捷动作：iOS Chat/Use（`GaryxMobileAgentsViews.swift:1419`）；desktop Chat（`AgentsHubPanel.tsx:597`、`AgentFormDialog.tsx:680`）。客户端顶层字段：desktop `agents.ts:200`、iOS `GaryxGatewayClient.swift:478` 丢弃；desktop `pendingAgentId`（`AppShell.tsx:652`）与 iOS 选中态（`GaryxMobileModel.swift:299`）必填 string。
- 验证：desktop `test:unit`（无 TS 编译）+ `build:ui` + `build:web`；iOS App-target 须 `xcodebuild`。

## 3. 设计

### 3.1 数据模型、持久化、共享解析层与 bridge 一致性合同

- `CustomAgentProfile` 加 `enabled: bool`，serde 默认 true。
- `custom-agents.json` 升 versioned envelope（判别：顶层数字 `version` + 对象 `agents`；否则 legacy 裸 map，load 迁移、persist 恒写 v2）：`{ "version": 2, "agents": {...}, "disabled_builtin_ids": [...], "default_agent_id": "codex" }`。
- **garyx-models 唯一解析实现**：`AgentAvailabilitySnapshot`（agent 元组含 enabled/standalone/built_in/default_workspace_dir/provider-runtime 字段 + `default_agent_id` + **`revision`**）+ 纯函数 `resolve_effective_default` / `ensure_enabled_for_new_binding`。
- **store 变更合同（回应五轮 #4）**：所有 mutation（upsert/delete/toggle/set-default）= **clone next state → persist next 到磁盘 → 成功后 swap 内存并产出 snapshot（revision 单调递增，锁内）**；持久化失败则内存不变、API 报错、不产生推送。现行"先改内存后写盘"顺序废弃。
- **bridge 一致性合同（回应五轮 #3）**：
  - snapshot 携带 store 锁内递增的 `revision`；bridge 应用端**丢弃 revision ≤ 已应用值的推送**（乱序推送安全）；
  - 废弃"snapshot push + reload topology"两阶段：gateway 侧引入**单一串行 reconcile 通道**（mutex/队列），把 `(AgentAvailabilitySnapshot, 派生的 provider topology/routes)` 作为**一个 coherent unit** 构造并一次发布给 bridge；bridge 读取方要么看到全旧、要么全新；
  - bridge 禁止读文件或第二来源；plugin account None 解析用该快照 + 同一纯函数（`lifecycle.rs:168` 改）。
  - HTTP toggle 无并发 token 与内部 revision 传播是两回事，互不替代。
- `default_agent_id` 与 agent 同库同锁，不进 `garyx.json`。

### 3.2 API

- `GET /api/custom-agents`：每项 `enabled`；顶层 `default_agent_id`（raw，可 null）+ `effective_default_agent_id`（nullable）；恒 200。
- `PATCH /api/custom-agents/{id}/toggle` `{"enabled": bool}`：幂等 set、无并发 token、custom 推进 `updated_at`，走 3.1 变更合同。
- `PATCH /api/custom-agents/{id}/default`：校验存在 + standalone + enabled；无 unset 端点。
- DELETE 命中 raw default：**同一 mutation 内原子清空 `default_agent_id`**（不拒绝删除）。
- `PUT` `enabled: Option<bool>` 三态：create 缺省 true、update 缺省保留。
- **外部绑定身份字段 reserved（回应五轮 #1）**：`agent_id`、`requested_provider_type` 等绑定身份 metadata 键定义为 **server-owned reserved fields**。外部 HTTP/WS 边界（`/api/chat/start`、WS start、`CreateThreadBody.metadata`）在剥 `provider_env` 的同一咽喉（`prepare.rs:120`）**一并剥除**；线程创建与 run 启动时由服务端以线程 canonical binding **强制覆盖**（`prepare.rs:196` 的 `or_insert` 改为 insert；`agent_identity.rs:50` 的合并不保留调用方值）。**不提供 metadata 形式的 one-off override**——显式选 agent 的唯一合法通道是 `CreateThreadBody.agent_id` 等 typed 字段（已走新绑定 gate）。
- bot/channel 读 API 继承态：None → `agent_id: null` + `effective_agent_id`；客户端传播点名：desktop `DesktopBotConsoleSummary`/mapper 加 `agentId`+`effectiveAgentId`；iOS `GaryxConfiguredBot` 系列加可选 `effectiveAgentId`；Web settings 以该字段（或 list 顶层 effective）为"跟随全局（当前 X）"数据源。

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

**channel account `agent_id` 三态合同**（`Some`=显式 override 含 claude；`None`=继承全局）：config 五类 account 统一 `Option<String>`、serde 默认 None、已持久化显式值不迁移；写入方拆物化/拆删除（onboard `onboard.rs:66`、desktop settings/channel-setup、**序列化器禁删显式 claude**、Web shell 修复、CLI）；读取方 router 返回 Option 走默认档、bridge plugin None 走 3.1 快照；Add Bot/账号保存是纯配置全停用照常（标签"跟随全局（当前无可用）"），首次入站新建才被 gate 拒；desktop route null 哨兵；side-chat fork 不物化（`side-chat-ops.ts:84` 源线程无 agentId 传 null）；task/cron/CLI 提前物化拆除。

**automation 的 agent 合同（v6：派生字段 + 模式转换 gate，回应五轮 #2）**：

- **范围限定 automation 的 AgentTurn 类 job**（`AutomationPrompt` 里的 Log job、`InternalDispatch` 一律不受本合同约束）。
- **generated**：agent 恒显式。create 省略 → 按 effective 落显式（全停用 400 NoEnabledAgent）；显式 disabled → 400；merge 后归一化：**仅缺失/空串/纯空白** → 显式 `"claude"`（守恒现行为，幂等，不写回 garyx.json）。
- **target-existing：job 的 agent 是派生值不是配置**。展示（summary/list/编辑表单）与运行**一律从目标线程 canonical binding 实时派生**，job 存储字段视为缓存、merge 后每次启动从目标线程重算刷新（**无论旧值为何**——修复"目标 Codex、job 显式 Claude"的现存脏数据）；目标线程不存在 → 摘要展示明确的 unavailable 态、该 job 派发失败记可见错误；legacy 目标线程无 agent 绑定 → 展示占位"随线程"、运行沿现行线程侧解析行为。
- **target→generated 模式转换 = 新绑定**（统一判据）：服务端在 update 落库前对转换后将用于新线程的 agent（含经 current 档保留的）**强制 enabled gate**；disabled → 400（客户端引导重选）。generated 内只改名（无模式/agent 变化）仍走 current 档放行。
- 客户端编辑合同：`agentChanged` 跟踪、未动省略 `agent_id`；disabled current 只读 missing 不可选、禁静默换绑；generated 新建默认 = effective、选项 enabled-only；target-existing 表单 agent 区展示"随目标线程（当前 X）"只读。

其余：automation update 显式改 disabled → 400；停用当前默认不改写 raw；允许停到一个不剩。

### 3.4 拦截点

**a) 创建意图**：`create_thread_for_agent_reference` 带意图——`Fresh` 拦、`Fork` 拦、`RecoverExistingSession` 豁免。

**b) typed 错误**：`AgentDisabled(agent_id)` + `NoEnabledAgent`。不进 Claude fallback、不虚构 key、planning 补错误通道、`/newthread`/渠道明确文案。覆盖 bot 入站、`/api/chat/start` 无 thread、`/newthread`、task 通知首建。已绑定 bot 返回既有 thread 不拦。

**c) TaskService gate**：router trait `NewTaskAgentGate` → `ResolvedAgentBinding`（canonical id + provider/runtime 快照 + `default_workspace_dir`，落 record 前应用「显式 workspace → agent 默认 → 无」）；`TaskService::new` 必填构造 fail-closed；gateway 注入 store 背书实现；五来源全覆盖；API 层保留友好 400。

**d) assign**：新绑定 disabled 拒绝；既有同-agent task 返工放行。

**e) 明确不拦**（反向验证）：既有线程续聊/steer、thread send 返工、bridge 回填、recovered session、已绑定 bot、target-existing automation（创建与运行）、纯配置动作。**已有 disabled agent 线程在 metadata reserved 字段清剿后必须照常续跑**（canonical binding 本来就是该 agent）。

### 3.5 CLI

- `garyx agent list`（人读注记 + `--json` enabled/raw/effective）；`agent enable|disable <id>`；`agent default [<id>]`。
- `task create --agent` / `thread create --agent` 透传 typed 错误。
- channels picker：共享 loader + 纯解析、滤 `standalone && enabled`、默认建议 = effective、"跟随全局"档位；全停用仍可保存继承档。

### 3.6 桌面端

- AgentsHubPanel：Enabled 列 + 默认标记 + "Set default"；disabled 行隐藏/禁用 Chat、Set default。
- 默认标记四态：raw 正常 = "Default"；raw 停用 = raw 行 muted "Default (inactive)" + effective 行 muted "Acting default"；raw null = effective 行 muted "Default (auto)"。CLI 同语义。
- 全停用空态（限新绑定）：新线程草稿 composer 空态禁发（已有线程 composer 不受影响）；generated automation 新建禁用；target-existing 创建、Add Bot/账号保存照常；管理面恒可用；`pendingAgentId` 改 nullable。
- picker disabled 隐藏；新建草稿默认 = effective；Chat 一次性 override；side-chat fork 不物化。
- AddBotDialog/渠道下拉：默认建议 = effective、排除 disabled、"跟随全局"档位、已绑定 disabled `.missing` 置灰；序列化器保值显式 claude；Web shell 修复。
- bot console 契约加双字段；automation 表单按 3.3（target 模式 agent 区只读"随目标线程"）；route null 哨兵；client 解析顶层双字段。

### 3.7 iOS

- 管理列表：StatusPill + swipe Enable/Disable；disabled 行隐藏 Chat/Use；默认标记四态同 3.6。
- picker：`makeTargets` 滤 `.enabled`；bot picker 默认建议 = effective + "跟随全局"。
- 全停用空态限新线程草稿；选中态改 nullable；已有线程 composer 不受影响。
- 两态拆分：`gatewayDefaultAgentId` 只读缓存（`Use` = set-default → 刷新）+ 草稿一次性 override；移除全部旁路持久写入。
- automation：编辑合同按 3.3（target 模式只读展示派生值）；解码补 claude 删除。
- models：`GaryxAgentSummary`/`GaryxCustomAgentRequest` 加 enabled（三态）；bot models 加 `effectiveAgentId`；`GaryxCachedAgent` 镜像（decodeIfPresent 默认 true）+ snapshot 升 version；client 解析顶层双字段。
- 纯逻辑下沉 Core 配 SwiftPM 测试。

### 3.8 兼容性

- legacy 裸 map → load 迁移写 v2（单向）；channel 显式值不迁移、缺失 None 继承；automation AgentTurn job：generated 缺失/空白归一 claude、target-existing 每启动从目标线程重算（含修复历史显式错值）；均幂等、不写回 garyx.json。
- 旧 catalog 快照不匹配即丢弃；不做旧网关兼容；老客户端 PUT 缺 enabled 由三态保护；服务端 gate 兜底；**老客户端携带的 metadata.agent_id 被 reserved 清剿静默忽略**（行为 = 服务端 canonical 覆盖，无破坏）。

## 4. 测试与验证计划（headless 优先）

- **存储/解析**：envelope 往返、legacy 迁移；解析链全分支 + 稳定序 + 回退；删除 raw default 原子清空 + badge；**持久化失败注入：内存不变、API 报错、无 snapshot 推送**。
- **API**：toggle/set-default/PUT 三态/list 双字段/bot API null+effective；**外部 metadata reserved：enabled A + `metadata.agent_id=B(disabled)` → 线程与 run 均落 A；`CreateThreadBody.metadata.agent_id` 不能制造绑定不一致；已有 disabled B 线程照常续跑**。
- **bridge 一致性**：默认热切换后 plugin None 路由同步；**barrier 乱序推送（旧 revision 后到被丢弃）**；**半应用窗口不可见（topology 与 snapshot 同单元发布）**。
- **创建 gate**：Fresh/Fork/Recovered；bot 入站不 fallback/不虚构 key/明确文案；`/api/chat/start`、`/newthread`、task 通知首建；**全停用下 Add Bot 保存成功 + 首次入站返回 NoEnabledAgent 的闭环**；TaskService 五来源 + 必填构造；assign 区分；task 工作区等价。
- **automation**：generated/target 拆分；**legacy target job 显式错值（目标 Codex、job Claude）→ 派生重算后四端展示与运行一致**；**target→generated 转换 enabled/disabled/全停用三态**（disabled current 被 gate 拒）；归一化幂等（config 覆盖源二次启动）；Log/InternalDispatch 不受影响；目标线程缺失/无绑定的展示与失败路径；停用后只改名不拒绝不改绑（三层）。
- **反向（不拦）**：disabled 既有线程续聊、thread send 返工、target-existing 创建与运行、generated 该次失败错误可见、全停用下已有线程 composer 照常。
- **desktop**：agent-options 过滤、默认预选、AddBotDialog、disabled 行动作、route 哨兵、client/bot console 映射、side-chat legacy fork、空态（限新草稿）；`test:unit` + `build:ui` + `build:web`。
- **iOS**：Core SwiftPM（过滤、decodeIfPresent、快照丢弃、两态、bot picker、nullable 选中态、effective 解码）+ `xcodebuild`。
- **CLI**：list、enable/disable/default、channels picker、task create disabled 报错。

## 5. 修订记录

- v6（2026-07-16）：回应五轮 4 条——①绑定身份 metadata 键（agent_id/requested_provider_type 等）定义为 server-owned reserved，外部边界与 provider_env 同咽喉剥除、服务端 canonical 强制覆盖（or_insert→insert），不提供 metadata one-off override；②automation 合同重构：范围限 AgentTurn、generated 仅缺失/空白归一、target-existing 的 agent 改为从目标线程实时派生（修历史显式错值）、target→generated 转换按新绑定强制 gate；③bridge 一致性：snapshot 带锁内单调 revision + 应用端丢弃旧版 + (snapshot, topology) 单一 reconcile 通道 coherent 发布；④store 变更合同 clone→persist→swap，失败不改内存不推送。
- v5：统一判据「enabled 只约束新绑定」；bridge 注入通道；删除 raw default 清空；automation 归一化常驻幂等；side-chat/事实修正；bot effective 客户端传播。
- v4：channel 三态传播补全；automation 显式存储（v5/v6 演进为按模式拆分+派生）；ResolvedAgentBinding 带工作区；badge/空态。
- v3：automation 客户端合同；channel 三态；TaskService fail-closed；NoEnabledAgent；管理面动作。
- v2：创建意图豁免 recover；typed AgentDisabled；TaskService 咽喉；versioned envelope；current 档；CLI channels picker；iOS 两态；PUT 三态。
