# Agent 启用开关 + 全局默认 Agent

日期：2026-07-15 · 状态：设计评审中（v3，回应 #TASK-2320 第二轮 5 条阻断；第一轮 8 条已关闭）

## 1. 需求与产品裁决（用户原话为准）

1. agent 增加启用/停用开关：**不开启的 agent 不可选**（各端 picker）、**CLI 也不可派活**。
2. **list 时明确展示可用性**（CLI、桌面、iOS 的管理列表）。
3. 多渠道搞定（gateway / CLI / desktop / mobile / bot 渠道 / automation）。
4. 支持**设置默认 agent**：默认 agent = 每次新建线程默认选中的那个。
5. **停用语义裁决（2026-07-15 用户明示）**：停用时，该 agent 名下**已开始/已绑定它的线程全部不受影响，可以继续调**；只是**不能新建线程、不能新建 task**。

## 2. 现状事实（两轮评审核验后的版本，实现时逐条再核对）

- `CustomAgentProfile`（`garyx-models/src/custom_agent.rs:9`）无 enabled；仅 `built_in`、`standalone`。
- 存储 = `CustomAgentStore`（gateway crate，`custom_agents.rs:43`），文件 `~/.garyx/data/custom-agents.json` 为**裸 `HashMap<agent_id, profile>`**（load `:69`、persist `:289`）；built-in 播种且 persist 过滤；id 只校验非空（任意字符串合法，含 `default`/`version`）。
- `resolve_agent_reference`（`agent_reference.rs:39`）被已有线程续跑消费（`prepare.rs:173`、`lifecycle.rs:422`、`run_management.rs:383`）——enabled 检查不能放这里。
- 默认解析现状：`selected_agent_reference_id` = requested → current → `"claude"`（`agent_identity.rs:13,15`）；automation update 依赖 current（`automation.rs:503`）。
- **claude 提前物化点（完整清单，二轮评审补齐）**：
  - task（`garyx-router/src/tasks.rs:423`）、cron（`cron.rs:1222`）、CLI channels（`channels.rs:1147`）；
  - **五类内置 channel account 的 serde 默认 = claude**（`config.rs:264,325`），`ApiAccount.agent_id` 是必填 `String`（`config.rs:443`）；
  - onboarding 自动建的 API account 显式写 claude（`onboard.rs:55`）；router 把该值当 requested 送入新线程（`garyx-router/src/threads.rs:839`）；
  - desktop settings 与 main-process channel setup 补 claude（`gateway-settings.ts:259`、`channel-setup.ts:101`）；
  - **desktop route 把 claude 当"未指定"哨兵**（`desktop-route.ts:185,223`）。
- **新建线程/task 生产入口**（两轮无截断 grep 确认穷尽）：直连 `create_thread_for_agent_reference` 的 HTTP 创建 / fork / recovered session（`routes.rs:2642/2779/2812`）/ cron generated-thread；经 `GatewayThreadCreator`（`app_bootstrap.rs:317`）的 bot/API 入站、无 thread 的 `/api/chat/start`（`prepare.rs:609`）、`/newthread`、task 通知首建 bot thread（`task_notifications.rs:254`）；**绕过中央 creator 的 `TaskService::create_task`**（`garyx-router/src/tasks.rs:412/423/440`，五个 agent 来源：executor/runtime/assignee/auto-start actor/human 默认）。
- router 入站失败现状：非 Claude 配置自动 fallback Claude（`threading/threads.rs:269`）、两败返回虚构 `new_thread_key()`（`:290`）、`build_dispatch_plan` 无错误通道（`planning.rs:435`）、`/newthread` 压扁文案（`local_commands.rs:301`）；已绑定 bot 直接返回既有 thread（`:240`）。
- **跨 crate 边界**：`TaskService` 在 `garyx-router`，仅依赖 thread/counter/projection（`tasks.rs:370`）；router 不能反向依赖 gateway（`garyx-router/Cargo.toml:10`）；enabled/default 状态在 gateway 的 `CustomAgentStore`。
- **automation 客户端现状**：desktop 编辑时**恒发送显式 `agentId`**（`useAutomationController.ts:38,265`），disabled 项被过滤后还被重插成可选 `unavailable`（`AutomationDialog.tsx:376`）；新建硬编码优先 claude；iOS 编辑时 `ensureEditAgentSelection` 会把被过滤的 current **静默换成**别的 agent 并显式提交（`GaryxMobileAutomationViews.swift:533,558`）。
- **管理面快捷动作**：iOS agent 行有 `Chat`/`Use`（`GaryxMobileAgentsViews.swift:1419`）；desktop 表格与详情有 `Chat`（`AgentsHubPanel.tsx:597`、`AgentFormDialog.tsx:680`）。
- CLI channels add/login 有独立 picker 直读裸 json、只滤 standalone（`channels.rs:1041/1089`）。
- desktop client 丢弃 list 顶层字段（`agents.ts:200`）；iOS 同（`GaryxGatewayClient.swift:478`）；iOS catalog 缓存 restore 版本精确相等否则删除（`+CatalogCache.swift:35`）。
- 验证规则：desktop `test:unit` 不做 TS 编译，须配 `npm run build:ui`；iOS App-target 文件 SwiftPM 不编译，须 `xcodebuild`（`docs/agents/validation.md:19,29`）。

## 3. 设计

### 3.1 数据模型、持久化与**共享解析层**

- `CustomAgentProfile` 加 `enabled: bool`，serde 默认 `true`。
- `custom-agents.json` 升 **versioned envelope**（v2 判别：顶层含数字 `version` + 对象 `agents`；否则 legacy 裸 map，load 即迁移、persist 恒写 v2）：

```json
{ "version": 2,
  "agents": { "<agent_id>": { ...profile... } },
  "disabled_builtin_ids": ["antigravity"],
  "default_agent_id": "codex" }
```

- **garyx-models 承载的不只是 envelope loader，还有唯一的解析实现**（回应二轮 #3）：
  - `AgentAvailabilitySnapshot`：`[(agent_id, enabled, standalone, built_in)] + default_agent_id` 的纯数据快照；
  - 纯函数 `resolve_effective_default(snapshot) -> Option<agent_id>`（3.3 的链）与 `ensure_enabled_for_new_binding(snapshot, id) -> Result<(), AgentBindingError>`；
  - gateway store、TaskService gate、automation、CLI 离线 picker **全部调这同一份纯函数**，只是各自提供 snapshot 来源。杜绝三份解析链。
- `default_agent_id` 与 agent 同库同锁，不进 `garyx.json`。

### 3.2 API

- `GET /api/custom-agents`：每项带 `enabled`；顶层加 `default_agent_id`（raw，可 null）+ `effective_default_agent_id`（**nullable：全停用时为 null**）。**management list 恒 200**——全停用时管理 UI 必须仍能加载以重新启用（回应二轮 #4）。
- `PATCH /api/custom-agents/{agent_id}/toggle`，body `{"enabled": bool}`：built-in/custom 一致；幂等 set；无并发 token；custom 翻转**必推进 `updated_at`**（stale `expected_updated_at` 表单 409）；同锁原子持久化。
- `PATCH /api/custom-agents/{agent_id}/default`（空 body）设默认：校验存在 + standalone + enabled，否则 400。**无 unset 端点**；"默认是否可解析"由 effective 值表达（可为 null），raw 值持久保留。
- `PUT /api/custom-agents/{agent_id}` 的 `enabled` 为 `Option<bool>` 三态：create 缺省 true；update 缺省保留现值。
- toggle / set-default 后 `bridge.replace_agent_profiles` + reload。

### 3.3 默认解析链与**"未指定"三态合同**

解析链（garyx-models 纯函数，保留 current 档）：

```
requested（显式指定；新建绑定 disabled → AgentDisabled 拒绝，绝不 fallback）
→ current（update/续用流程保持既有绑定，enabled 不参与——裁决 5）
→ default_agent_id（存在 且 enabled 且 standalone）
→ "claude"（enabled 时）
→ 第一个 enabled 的 standalone（稳定序：built-in 播种序 claude/codex/traex/antigravity → custom 按 agent_id 字典序）
→ 无可用（隐式创建 → NoEnabledAgent 错误；effective 值 → null）
```

**channel account 的 `agent_id` 改三态合同**（回应二轮 #2，claude 物化面覆盖 config 层）：

- 五类内置 channel account + `ApiAccount` 的 `agent_id` 统一为 `Option<String>`：`Some(id)` = 账号显式 override（**含显式 "claude"**）；`None` = 新建线程时继承 global effective default。
- serde 默认从 claude 改为 `None`。已持久化的显式值（含 "claude"）**保持为 override 不迁移**——无法区分"默认落盘"与"用户选的"，保守保留现行为；字段缺失的老配置按 None 继承全局默认（未设默认时 effective 仍落 claude，行为不变）。
- 拆物化点：onboard 不再写 claude（`onboard.rs:55`）；desktop `gateway-settings.ts:259` / `channel-setup.ts:101` 不再补 claude（留空 = 继承）；CLI channels 同理；router `default_agent_for_channel_account` 返回 Option，None 走 resolver 默认档而非 requested 档。
- desktop/iOS 渠道设置 UI 的 agent 下拉加"Default (跟随全局)"档位表达 None。
- **desktop route 哨兵修正**：`desktop-route.ts:185,223` 以 `null` 表示未指定；claude 是普通显式 override（否则全局默认 = codex 时，用户显式选 claude 刷新后被还原）。
- task（`garyx-router/tasks.rs:423`）、cron（`cron.rs:1222`）、CLI channels（`:1147`）的提前物化一并拆除。
- automation update 不动 agent → current 档保持（即使 disabled）；显式改成 disabled → 400。停用当前默认不改写 raw 值；允许停到一个不剩。

### 3.4 拦截点

**a) 中央创建意图**：`create_thread_for_agent_reference` 带意图参数——`Fresh`（HTTP 创建、cron generated-thread）拦、`Fork` 拦、`RecoverExistingSession` **豁免**（既有会话恢复，裁决 5）。

**b) typed 错误贯穿 router**：两个结构化错误——`AgentDisabled(agent_id)`（显式/配置指向的 agent 已停用）与 **`NoEnabledAgent`**（隐式创建且全停用，无具体 id；回应二轮 #4）。两者同等保证：**不进** Claude fallback（`threads.rs:269`）、**不走**虚构 `new_thread_key()`（`:290`）、`build_dispatch_plan` 补错误通道、`/newthread` 与渠道回复给出明确文案。覆盖 bot 入站、无 thread 的 `/api/chat/start`、`/newthread`、task 通知首建 bot thread（该 bot 新会话整体不可用，通知首建失败记可见错误；已有通知线程照常——设计决策，可否决）。已绑定 bot 返回既有 thread 的路径不拦。

**c) TaskService gate 的跨 crate 设计**（回应二轮 #3）：

- `garyx-router` 定义 trait（如 `NewTaskAgentGate`）：输入 task 的候选绑定（五来源推导结果）与意图，输出 `Result<ResolvedAgent, AgentBindingError>`；
- **`TaskService::new` 把 gate 作为必填构造参数**——生产不可能"忘注入"而 fail-open（fail-closed by construction）；测试注入显式 permissive stub；
- gateway 注入 `CustomAgentStore` 背书的实现，内部调 garyx-models 纯函数（3.1），与 HTTP 层/automation/CLI 同一条解析链；
- API 层 `resolve_task_executor_agent` 保留做友好 400；强制线在 `TaskService::create_task`，覆盖五个 agent 来源（human auto-start 默认档也经同一纯函数解析全局默认）。

**d) assign 区分新旧绑定**：新绑定到 disabled → 拒绝；task thread 已绑定同一 disabled agent 的 assign/返工 → 放行。

**e) 明确不拦**（须反向验证）：已有线程发消息/续跑/steer、`thread send` 唤醒已有 task、bridge 对已绑定 agent 的 run 派发与回填、recovered session、已绑定 bot 既有线程、target-existing automation 继续运行。

**f) automation 客户端编辑合同**（回应二轮 #1）：

- 编辑表单**单独跟踪 `agentChanged`**：未动 agent 的 update **省略 `agent_id`**（让服务端 current 档生效）——desktop 现在恒发显式 agentId（`useAutomationController.ts:265`）必须改；
- current 已 disabled 时在表单中**只读/missing 展示、不可作为可选项重插**（`AutomationDialog.tsx:376` 的 `unavailable` 重插逻辑改为不可选）；iOS `ensureEditAgentSelection` **不得静默换绑**（`GaryxMobileAutomationViews.swift:533` 改为保留 current 展示为 missing）；
- 新建 automation：默认 = effective default、选项 = enabled only（desktop 硬编码 claude 处一并改）。

### 3.5 CLI

- `garyx agent list`：人读加启用/默认注记；`--json` 每项 `enabled`，顶层 raw + effective（可 null）。
- `garyx agent enable|disable <id>`（toggle 端点）；`garyx agent default [<id>]`（无参显示 raw+effective，有参设置）。
- `task create --agent` / `thread create --agent` 透传 `AgentDisabled`/`NoEnabledAgent` 明确报错。
- channels add/login picker 改走 garyx-models 共享 loader + 纯解析函数（离线可用），滤 `standalone && enabled`，默认建议 = effective default，且提供"跟随全局"档位（写 None）。

### 3.6 桌面端

- AgentsHubPanel：Enabled 列（Switch，toggle API）+ Default badge + "Set default" 行动作。
- **disabled 行的发起类动作合同**（回应二轮 #5）：`Chat`（表格 `AgentsHubPanel.tsx:597` + 详情 `AgentFormDialog.tsx:680`）与 `Set default` 对 disabled agent **隐藏或禁用**——服务端 400 是兜底不是替代。
- picker（ComposerForm 等）disabled 直接隐藏（与 standalone 同层，`agent-options.ts`）；新建草稿默认 = effective default（替换 `pendingAgentId` 硬编码初值与 reset 点）；`Chat`（enabled agent 的）保持一次性 override。
- AddBotDialog / 渠道设置下拉：默认建议 = effective default、排除 disabled、加"跟随全局"档位；已绑定 disabled 的账号行 `.missing` 置灰。
- automation 表单按 3.4f。
- desktop client 解析 list 顶层双字段（`agents.ts`）；desktop-route 哨兵修正按 3.3。

### 3.7 iOS

- 管理列表：`GaryxStatusPill` Enabled/Paused + swipe Enable/Disable；**disabled 行隐藏 `Chat`/`Use`**（`GaryxMobileAgentsViews.swift:1419`；回应二轮 #5）。
- picker：`makeTargets` 加 `.filter(\.enabled)` → 新线程 sheet / target popover / automation agentRow / bot 设置 picker 全排除；bot picker 默认建议 = effective default + "跟随全局"档位。
- 两态拆分：`gatewayDefaultAgentId`（effective 的只读缓存，唯一写入 = 服务端同步；`Use` = 调 set-default → 刷新）+ 草稿一次性 override。移除全部旁路持久写入（create/update 后 set、`setNewThreadAgentTarget` 持久分支、`ensureSelectedAgentTarget` 落盘）。
- automation 编辑按 3.4f：current disabled 保留为 missing 展示，不静默换绑，未动 agent 不提交 agent_id。
- `GaryxAgentSummary`/`GaryxCustomAgentRequest` 加 `enabled`（request 三态）；`GaryxCachedAgent` 镜像（显式 `decodeIfPresent` 默认 true）+ snapshot 升 version（旧快照安全丢弃）；iOS client 解析顶层双字段。
- 默认解析/回退纯逻辑下沉 `GaryxMobileCore` 配 SwiftPM 测试。

### 3.8 兼容性

- legacy 裸 map → load 迁移、persist 写 v2（单向）；channel account 显式值不迁移、缺失字段按 None 继承。
- 旧 catalog 快照版本不匹配即丢弃重建。
- 不做旧网关兼容设计；老客户端 PUT 缺 `enabled` 由三态保护；老客户端可见 disabled agent 时服务端 gate 兜底。

## 4. 测试与验证计划（headless 优先）

- **gateway 存储/解析**：v2 envelope 往返、legacy 迁移（含 `version`/`default` 命名 agent 不歧义）；纯解析链全分支（requested/current/default/claude/first-enabled/**NoEnabledAgent**）、fallback 稳定序（改 display_name 不影响）、default 停用回退、全停用 effective=null 且 list 200。
- **gateway API**：toggle（含 custom 推进 updated_at → stale PUT 409）；set-default 校验；PUT 三态（update 缺省保留停用态）；list 顶层双字段。
- **创建 gate**：Fresh/Fork 拦、Recovered 豁免；bot 入站 disabled 不 fallback / 不虚构 key / 渠道明确文案；`/api/chat/start` 无 thread、`/newthread`、task 通知首建；`TaskService::create_task` 五来源全拦 + **gate 必填构造（编译层面无 fail-open 路径）**；assign 新绑定拦 / 既有同-agent 放行；全停用时隐式创建 → NoEnabledAgent。
- **channel 三态**：`None=inherit` 生效（onboard 建的 api:main 吃到全局默认）、显式 "claude" 保持 override、字段缺失老配置行为守恒（未设默认时仍 claude）。
- **反向（不拦）**：disabled 既有线程续聊、thread send 返工、target-existing automation 继续、generated automation 该次失败且错误可见、automation 无关 update 不改绑（**服务端 + desktop + iOS 三层：停用后只改名保存成功且不改绑**）。
- **CLI**：list（raw+effective）、enable/disable/default、channels picker 离线共享 loader + 滤 disabled、task create --agent disabled 报错。
- **desktop**：`agent-options` 过滤、默认预选、AddBotDialog、disabled 行 Chat/Set default 禁用、desktop-route "全局默认 codex + 显式 claude override 刷新不还原"、client 顶层映射；**`npm run test:unit` + `npm run build:ui`**（test:unit 不做 TS 编译）。
- **iOS**：Core SwiftPM（makeTargets 过滤、decodeIfPresent、快照丢弃、两态纯逻辑、bot picker 默认）+ **App target `xcodebuild`**（多处改动在 App target，SwiftPM 不编译）。

## 5. 修订记录

- v3（2026-07-15）：回应二轮 5 条——①automation 客户端合同（agentChanged/省略 agent_id/missing 只读/新建吃 effective default）；②claude 物化清单扩到 config 层：channel account `agent_id` 三态合同（Option + None 继承全局）、onboard/desktop settings/channel-setup 拆物化、desktop route null 哨兵；③TaskService gate 跨 crate 设计（garyx-models 唯一纯解析 + router trait + 必填构造 fail-closed + gateway 注入）；④全停用状态定义（effective nullable、list 恒 200、NoEnabledAgent typed 错误同禁 fallback）；⑤管理面 disabled 行 Chat/Use/Set default 隐藏或禁用；验证补 build:ui 与 xcodebuild。
- v2（2026-07-15）：回应一轮 8 条（意图分类豁免 recover、typed AgentDisabled 贯穿 router、TaskService 咽喉、versioned envelope、解析链保留 current、CLI channels picker、iOS 两态、PUT 三态 + toggle updated_at）。
