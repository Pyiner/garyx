# Agent 启用开关 + 全局默认 Agent

日期：2026-07-16 · 状态：设计评审中（v8，回应 #TASK-2320 第七轮 2 条阻断；前六轮 29 条已关闭）

## 1. 需求与产品裁决（用户原话为准）

1. agent 增加启用/停用开关：**不开启的 agent 不可选**（各端 picker）、**CLI 也不可派活**。
2. **list 时明确展示可用性**（CLI、桌面、iOS 的管理列表）。
3. 多渠道搞定（gateway / CLI / desktop / mobile / bot 渠道 / automation）。
4. 支持**设置默认 agent**：默认 agent = 每次新建线程默认选中的那个。
5. **停用语义裁决（2026-07-15 用户明示）**：停用时，该 agent 名下**已开始/已绑定它的线程全部不受影响，可以继续调**；只是**不能新建线程、不能新建 task**。

**统一判据：enabled 只约束「产生新绑定」的动作；纯配置、既有绑定的使用/派生一律不受约束。模式/形态转换若使一个 agent 将被用于未来的新线程，即视为新绑定。**

## 2. 现状事实（六轮评审核验后版本，实现时逐条再核对）

- `CustomAgentProfile`（`custom_agent.rs:9`）无 enabled；`CustomAgentStore` 持久化 `custom-agents.json` 裸 map（load `:69`；persist `:289-297` 过滤 built-in）；`delete_agent`（`:273-283`）可删任意 custom。**store 变更顺序 = 先改内存再写盘**（upsert `:254`、delete `:281`），`atomic_write` 只防文件撕裂（`atomic_write.rs:6`），写失败不回滚内存。
- `resolve_agent_reference`（`agent_reference.rs:39`）被已有线程续跑消费（`prepare.rs:173`、`lifecycle.rs:422`、`run_management.rs:383`）——enabled 检查不能放这里。
- 默认解析现状：requested → current → `"claude"`（`agent_identity.rs:13,15`）；automation update 依赖 current（`automation.rs:503,948,967`）。
- **外部 metadata 通道**：`/api/chat/start` 与 WS start 接受任意 `metadata`（`contracts.rs:38`），外部边界只剥 `provider_env`（`prepare.rs:120`），agent metadata 用 `entry.or_insert` 客户端值优先（`prepare.rs:196`）；bridge 直接消费 `metadata.agent_id`（`run_management.rs:80,403`）。**但（六轮核验）该合并组不只含身份**：`agent_runtime_metadata`（`agent_reference.rs:70`）同时返回 `model`/reasoning/tier/system prompt，且现有优先级契约 = **请求 > 线程模型单元 > agent 默认**（`provider.rs:150`，回归测试 `prepare/tests.rs:289`；创建时 typed `body.model` 写线程模型单元 `routes.rs:2727`，续跑先合并线程单元 `prepare.rs:169`）——整组改 insert 会破坏合法模型优先级。**`CreateThreadBody.metadata`（`routes.rs:901`）不经过 `prepare.rs:120`**，走 `routes.rs:2727` → creator，是独立边界；中央 snapshot 合并保留调用方 `agent_id`（`agent_identity.rs:50`）。
- **claude 物化/哨兵点全清单**：task（`garyx-router/tasks.rs:423`）、cron（`cron.rs:1227-1229`；`automation.rs:494-500`；None/空串/纯空白均视 claude）、CLI channels（`channels.rs:1147`）；channel account = 四 typed + `ApiAccount` 共五类，serde 默认 claude（`config.rs:264,325`）、`ApiAccount.agent_id` 必填（`:443`）、onboard 写 claude（`onboard.rs:66`）、router 把账号值当 requested（`garyx-router/threads.rs:839`）；desktop `gateway-settings.ts:259`/`channel-setup.ts:101` 补 claude、序列化器删显式 claude（`gateway-settings.ts:146,179`）；Web shell 物化（`use-web-settings-state.ts:41`、`WebSettingsPage.tsx:387,609`；`build:web` 在 `package.json:17`）；bridge plugin None 解 claude（`lifecycle.rs:168`）；bot 读 API None 输出空串（`routes.rs:3376,3448`）；desktop route claude 哨兵（`desktop-route.ts:185,223`）；side-chat fork 物化（`side-chat-ops.ts:84`）。
- **automation 现状**：`AutomationSummary.agent_id` 必填（`automation.rs:101`）；target-existing 不创建线程、运行读目标线程（`automation.rs:858-874` 跳过校验；`cron.rs:1256-1269` 仅 generated 进中央创建；**运行取目标线程 `cron.rs:1938`，而摘要展示 job 字段 `automation.rs:585`**）；**target 创建省略 agent 时服务端/桌面会物化 claude 并显式存入 job**（`automation.rs:867,791`、`useAutomationController.ts:260`）——"目标线程 Codex、job 显式 Claude"是现存合法数据；**update 清除目标后用 current 解析**（`automation.rs:948,967`）——target→generated 转换可保留 disabled current；`CronService::load` 磁盘 → config jobs 覆盖 → 持久化（`cron.rs:748-781`）；`AutomationPrompt` 还含 Log job（`cron.rs:1531`）；`InternalDispatch` 的 None 合法（`schedule_followup.rs:172-187`、`quota_resend.rs:186-207`）；desktop 编辑恒发显式 agentId（`useAutomationController.ts:38,265`）、disabled 重插可选（`AutomationDialog.tsx:376`）；iOS 静默换绑（`GaryxMobileAutomationViews.swift:533,558`）、解码补 claude（`GaryxGatewayAutomationModels.swift:77`）。**wire 层（六轮核验）全链要求非空 `agent_id: String`**：服务端 `automation.rs:99`、desktop mapper 强制非空（`automations.ts:178`）、iOS 必填 String 缺失补 claude（`GaryxGatewayAutomationModels.swift:8`）、iOS cache 必填（`GaryxMobileCatalogCache.swift:206`）——null/空串会炸契约，哨兵串会伪造身份。**legacy 无绑定线程可在运行期经 task assign 获得绑定**（`gateway/tasks.rs:825`）——启动时缓存的派生值会过期。
- **bridge 状态面**：topology 与 agent profiles **不同锁**（`state.rs:22`），`replace_agent_profiles` 只更新一张表（`multi_provider.rs:118`），API 随后独立 reload topology（`api.rs:2841`）——两阶段有乱序/半应用窗口。**配置独立热更新是生产路径**（`app_state.rs:283`、`api.rs:2017`、`gateway.rs:232`；bridge 测试 `multi_provider/tests.rs:4636` 明确要求不动 agent store 即热更模型）——bridge 更新的版本排序不能只挂在 agent store 计数上。**`get_or_create_provider` 会在 topology 发布前原地改共享 provider 默认值**（`lifecycle.rs:323`）——"全旧或全新"需要显式 staging/swap 边界。
- task 工作区：中央创建应用 `default_workspace_dir`（`agent_identity.rs:104`）而 TaskService 绕过（`gateway/tasks.rs:199,867`）。
- 新建入口穷尽（五轮确认）：中央 creator 直连 = HTTP Fresh/Fork/Recover（`routes.rs:2812`）+ generated cron（`cron.rs:1269`）+ `GatewayThreadCreator`（`agent_identity.rs:158`，生产强制注入 `app_bootstrap.rs:325`，覆盖 bot 入站 / `/api/chat/start` / `/newthread` / task 通知首建）；唯一 raw 绕行 = `TaskService::create_task`（`tasks.rs:440`，五 agent 来源 `:412-426`）。router 失败现状：Claude fallback（`threading/threads.rs:269`）、虚构 key（`:290`）、planning 无错误通道、`/newthread` 压扁文案。
- bot 读 API 客户端：desktop `DesktopBotConsoleSummary`/mapper 不保留 agent_id；iOS bot models 只解析 agentId；Web settings 无全局 effective 数据通道。
- 管理面快捷动作：iOS Chat/Use（`GaryxMobileAgentsViews.swift:1419`）；desktop Chat（`AgentsHubPanel.tsx:597`、`AgentFormDialog.tsx:680`）。客户端顶层字段：desktop `agents.ts:200`、iOS `GaryxGatewayClient.swift:478` 丢弃；desktop `pendingAgentId`（`AppShell.tsx:652`）与 iOS 选中态（`GaryxMobileModel.swift:299`）必填 string。
- 验证：desktop `test:unit`（无 TS 编译）+ `build:ui` + `build:web`；iOS App-target 须 `xcodebuild`。

## 3. 设计

### 3.1 数据模型、持久化、共享解析层与 bridge 一致性合同

- `CustomAgentProfile` 加 `enabled: bool`，serde 默认 true。
- `custom-agents.json` 升 versioned envelope（判别：顶层数字 `version` + 对象 `agents`；否则 legacy 裸 map，load 迁移、persist 恒写 v2）：`{ "version": 2, "agents": {...}, "disabled_builtin_ids": [...], "default_agent_id": "codex" }`。
- **garyx-models 唯一解析实现**：`AgentAvailabilitySnapshot`（agent 元组含 enabled/standalone/built_in/default_workspace_dir/provider-runtime 字段 + `default_agent_id` + **`agent_state_revision`**）+ 纯函数 `resolve_effective_default` / `ensure_enabled_for_new_binding`。
- **变更与发布合同（v8：单一线性化点，回应七轮 #1/#2）**：
  - **版本域**：`agent_state_revision`（agent store 提交时递增）与 `config_revision`（config 提交时递增）构成**源版本向量**；`reconcile_generation` 每次发布递增，排序整个 unit。
  - **channel-owned latest-state**：所有触发源（boot、agent CRUD/toggle/default、settings reload `api.rs:2017`、文件热更 `app_state.rs:283`、CLI reload `gateway.rs:232`、MCP config）**只投递 dirty 标记，不携带状态**；串行 reconcile 通道在出队时**读取两个源当时的最新已提交版本**（可合并多个 dirty，coalescing），据此构造 unit 并**在选定最新双版本之后才分配 generation**。unit 记录 `(config_revision, agent_state_revision)`。由此杜绝"A2 mutation 读到未替换的 C1、却拿到更大 generation 把 C2 顶回 C1"的时序（七轮 #1 反例）。
  - **agent mutation 管线重排（废 v6 的 persist→swap→publish 三段）**：mutation 经同一串行通道执行——**① staged 构造完整 unit（含以最新 config 派生 provider topology/routes，可失败）→ ② persist 到磁盘（可失败）→ ③ swap store → ④ 原子 publish**。①②任一失败 = store/磁盘/published 三者全部不变、API 返回明确错误（不再有 store 已变而 topology 构造失败的 500 分裂，修 `api.rs:2816-2860` 现状）；③④在同一临界区内完成，④是不可失败的指针交换。
  - **单一读面 = last-published snapshot**：新绑定 gate（Fresh/Fork）、Task gate、effective default 解析、`GET /api/custom-agents` 的 effective 值、bridge plugin None 路由**一律读 last-published snapshot**，不直接读 store（`agent_identity.rs:27-32,90-103` 的直读改造）。store 只是写侧持久化后端。由此 store swap 与 publish 之间不存在外部可见窗口——所有消费者在 publish 前看到的都是完整旧世界。
  - staged 构造期间**不得原地修改共享 provider 默认值**（`get_or_create_provider` `lifecycle.rs:323` pre-publish 原地写改 staged）；bridge 应用端丢弃 `reconcile_generation ≤` 已应用值的推送；bridge 禁止读文件或第二来源（`lifecycle.rs:168` 改走快照）。
  - HTTP toggle 无并发 token 与内部版本传播是两回事，互不替代。
- `default_agent_id` 与 agent 同库同锁，不进 `garyx.json`。

### 3.2 API

- `GET /api/custom-agents`：每项 `enabled`；顶层 `default_agent_id`（raw，可 null）+ `effective_default_agent_id`（nullable）；恒 200。
- `PATCH /api/custom-agents/{id}/toggle` `{"enabled": bool}`：幂等 set、无并发 token、custom 推进 `updated_at`，走 3.1 变更合同。
- `PATCH /api/custom-agents/{id}/default`：校验存在 + standalone + enabled；无 unset 端点。
- DELETE 命中 raw default：**同一 mutation 内原子清空 `default_agent_id`**（不拒绝删除）。
- `PUT` `enabled: Option<bool>` 三态：create 缺省 true、update 缺省保留。
- **外部绑定身份字段 reserved（v7：精确键集 + 分边界，回应六轮 #1）**：
  - **reserved key 集精确定义且共享为常量**（garyx-models 单一来源）：仅**绑定身份/敏感键**——`agent_id`、`requested_provider_type`、`provider_env`（已有）。**`model`、reasoning、tier、system prompt 不在 reserved 集**，保持现有 fill-only（`or_insert`）语义与「请求 > 线程模型单元 > agent 默认」优先级契约（`provider.rs:150`）不变。
  - **各真实边界分别清剿**（不存在"同一咽喉"）：HTTP/WS chat 边界在 `prepare.rs:120` 现有剥除点扩展 reserved 集；`CreateThreadBody.metadata` 在 `routes.rs:2727` → creator 的入口处独立剥除。
  - 剥除后由服务端以线程 canonical binding 写入身份键：`prepare.rs:196` 与 `agent_identity.rs:50` **仅对 reserved 身份键**改为 canonical 覆盖，其余键合并语义不动。
  - **不提供 metadata 形式的 one-off override**——显式选 agent 的唯一合法通道是 `CreateThreadBody.agent_id` 等 typed 字段（已走新绑定 gate）。
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

**automation 的 agent 合同（v6 派生字段 + 模式转换 gate；v7 补 typed wire contract 与实时 current）**：

- **范围限定 automation 的 AgentTurn 类 job**（`AutomationPrompt` 里的 Log job、`InternalDispatch` 一律不受本合同约束）。
- **generated**：agent 恒显式。create 省略 → 按 effective 落显式（全停用 400 NoEnabledAgent）；显式 disabled → 400；merge 后归一化：**仅缺失/空串/纯空白** → 显式 `"claude"`（守恒现行为，幂等，不写回 garyx.json）。
- **target-existing：job 的 agent 是派生值不是配置，wire 层用 typed 状态表达（v7，回应六轮 #3）**。`AutomationSummary` 增加 **`agent_resolution: "resolved" | "follow_thread" | "target_missing"`** + **nullable `effective_agent_id`**：generated 恒 `resolved` + 显式 id；target-existing 恒 `follow_thread`，`effective_agent_id` = 目标线程 canonical binding 的**实时派生值**（线程无绑定时为 null，UI 展示"随线程"占位）；目标线程不存在 → `target_missing`（`effective_agent_id: null`，UI 明确 unavailable 态，该 job 派发失败记可见错误）。**禁用 null/空串塞进现必填 `agent_id` 和任何哨兵串**（desktop mapper `automations.ts:178`、iOS models `GaryxGatewayAutomationModels.swift:8`（删 claude 回填）、iOS cache `GaryxMobileCatalogCache.swift:206`（升版本）、CLI 展示同步适配新字段）。job 存储字段降级为缓存、merge 后每启动从目标线程重算（无论旧值——修复"目标 Codex、job 显式 Claude"脏数据）。
- **target→generated 模式转换 = 新绑定**（统一判据）：服务端在 update 落库前对转换后将用于新线程的 agent **强制 enabled gate**；**current 必须实时读取目标线程当时的绑定**（不得用启动时缓存——无绑定线程可在运行期经 task assign 补绑定，`gateway/tasks.rs:825-847`；已有不同绑定会被 `:776-780` 拒绝）；目标缺失或无绑定时**要求显式选择 enabled agent**（否则 400 明确报错引导重选）；disabled → 400。generated 内只改名（无模式/agent 变化）仍走 current 档放行。
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
- **API**：toggle/set-default/PUT 三态/list 双字段/bot API null+effective；**外部 metadata reserved：enabled A + `metadata.agent_id=B(disabled)` → 线程与 run 均落 A；`CreateThreadBody.metadata.agent_id` 不能制造绑定不一致；已有 disabled B 线程照常续跑；「custom agent 默认模型 + typed 线程模型」在创建与续跑两条路径均保持「请求 > 线程单元 > agent 默认」不被 reserved 清剿破坏**。
- **bridge 一致性**：默认热切换后 plugin None 路由同步；**barrier 乱序推送（旧 generation 后到被丢弃）**；**同 agent_state_revision 下纯 config C1→C2 正常应用**；**barrier 固定「C2 staged 中 + A2 并发 mutation」等 config/agent 交错顺序 → 最终发布必为 C2/A2**；**半应用窗口不可见（staged + 原子 swap，构造期无共享默认值原地写）**；**在 swap 与 publish 之间暂停 → 并发 Fresh/Task/channel 读一律看到完整旧世界（单一读面）**；**staged 构造失败 → store/磁盘/published 全不变 + API 明确报错**。
- **创建 gate**：Fresh/Fork/Recovered；bot 入站不 fallback/不虚构 key/明确文案；`/api/chat/start`、`/newthread`、task 通知首建；**全停用下 Add Bot 保存成功 + 首次入站返回 NoEnabledAgent 的闭环**；TaskService 五来源 + 必填构造；assign 区分；task 工作区等价。
- **automation**：generated/target 拆分；**legacy target job 显式错值（目标 Codex、job Claude）→ 派生重算后四端展示与运行一致**；**typed wire contract：`agent_resolution` 三态 + nullable `effective_agent_id` 在 desktop mapper/iOS models（无 claude 回填）/iOS cache（升版本）/CLI 全链解码**；**target→generated 转换 enabled/disabled/全停用三态**（disabled current 被 gate 拒）+ **目标删除、启动后无绑定线程经 task assign 获得绑定再转换（current 取实时绑定非启动缓存）、目标缺失/无绑定转换须显式重选**；归一化幂等（config 覆盖源二次启动）；Log/InternalDispatch 不受影响；停用后只改名不拒绝不改绑（三层）。
- **反向（不拦）**：disabled 既有线程续聊、thread send 返工、target-existing 创建与运行、generated 该次失败错误可见、全停用下已有线程 composer 照常。
- **desktop**：agent-options 过滤、默认预选、AddBotDialog、disabled 行动作、route 哨兵、client/bot console 映射、side-chat legacy fork、空态（限新草稿）；`test:unit` + `build:ui` + `build:web`。
- **iOS**：Core SwiftPM（过滤、decodeIfPresent、快照丢弃、两态、bot picker、nullable 选中态、effective 解码）+ `xcodebuild`。
- **CLI**：list、enable/disable/default、channels picker、task create disabled 报错。

## 5. 修订记录

- v8（2026-07-16）：回应七轮 2 条——①channel-owned latest-state：触发源只投 dirty 标记，通道出队时读双源最新已提交版本（coalescing）后构造 unit，generation 在选定最新双版本后分配，unit 记录 `(config_revision, agent_state_revision)`；②单一线性化点：agent mutation 管线重排为 staged 构造 → persist → swap → 原子 publish（前两步可失败且失败即全不变），所有决策读面（Fresh/Task gate、effective default、bridge 路由）统一到 last-published snapshot，消除 swap 与 publish 间可见窗口与失败分裂；另按核验把 task assign 措辞修正为"补绑定"（`tasks.rs:825-847`，不同绑定被 `:776-780` 拒）。
- v7（2026-07-16）：回应六轮 3 条——①reserved 键集精确化：仅 `agent_id`/`requested_provider_type`/`provider_env`（garyx-models 共享常量），model/reasoning/tier/system prompt 保持 fill-only 优先级契约；HTTP/WS 与 CreateThreadBody 各自真实边界分别清剿；②bridge 版本域拆分：`agent_state_revision`（拒旧 agent snapshot）+ `reconcile_generation`（每次发布递增、排序整个 unit），全部刷新入口收敛单一 reconcile 通道，staged 构造 + 原子 swap（禁 pre-publish 原地写共享默认）；③automation typed wire contract：`agent_resolution` 三态 + nullable `effective_agent_id` 全链适配（禁哨兵串），target→generated 的 current 实时读目标线程、目标缺失/无绑定须显式重选。
- v6（2026-07-16）：回应五轮 4 条——①绑定身份 metadata 键（agent_id/requested_provider_type 等）定义为 server-owned reserved，外部边界与 provider_env 同咽喉剥除、服务端 canonical 强制覆盖（or_insert→insert），不提供 metadata one-off override；②automation 合同重构：范围限 AgentTurn、generated 仅缺失/空白归一、target-existing 的 agent 改为从目标线程实时派生（修历史显式错值）、target→generated 转换按新绑定强制 gate；③bridge 一致性：snapshot 带锁内单调 revision + 应用端丢弃旧版 + (snapshot, topology) 单一 reconcile 通道 coherent 发布；④store 变更合同 clone→persist→swap，失败不改内存不推送。
- v5：统一判据「enabled 只约束新绑定」；bridge 注入通道；删除 raw default 清空；automation 归一化常驻幂等；side-chat/事实修正；bot effective 客户端传播。
- v4：channel 三态传播补全；automation 显式存储（v5/v6 演进为按模式拆分+派生）；ResolvedAgentBinding 带工作区；badge/空态。
- v3：automation 客户端合同；channel 三态；TaskService fail-closed；NoEnabledAgent；管理面动作。
- v2：创建意图豁免 recover；typed AgentDisabled；TaskService 咽喉；versioned envelope；current 档；CLI channels picker；iOS 两态；PUT 三态。
