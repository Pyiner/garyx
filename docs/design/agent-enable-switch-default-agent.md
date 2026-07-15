# Agent 启用开关 + 全局默认 Agent

日期：2026-07-15 · 状态：设计评审中

## 1. 需求与产品裁决（用户原话为准）

1. agent 增加启用/停用开关：**不开启的 agent 不可选**（各端 picker）、**CLI 也不可派活**。
2. **list 时明确展示可用性**（CLI、桌面、iOS 的管理列表）。
3. 多渠道搞定（gateway / CLI / desktop / mobile / bot 渠道 / automation）。
4. 支持**设置默认 agent**：默认 agent = 每次新建线程默认选中的那个。
5. **停用语义裁决（2026-07-15 用户明示）**：停用时，该 agent 名下**已开始/已绑定它的线程全部不受影响，可以继续调**；只是**不能新建线程、不能新建 task**。

## 2. 现状事实（三路探查结论，实现/评审时逐条核对）

- `CustomAgentProfile`（`garyx-models/src/custom_agent.rs:9`）无 enabled 字段；仅 `built_in`、`standalone`（默认 true）。
- 存储 = `CustomAgentStore`（`garyx-gateway/src/custom_agents.rs:43`），持久化到 `~/.garyx/data/custom-agents.json`；**built-in（claude/codex/traex/antigravity）内存播种、persist 时被过滤掉**（`persist_locked` `.filter(!built_in)`）；`upsert_agent` 拒绝 built-in、带 `WriteExpectation` 条件更新。
- 校验咽喉 = `resolve_agent_reference`（`garyx-models/src/agent_reference.rs:39`，`standalone` 在此拦）。**注意：它同时被已有线程续跑路径消费**（chat prepare `application/chat/prepare.rs:173-202`、bridge `provider_config_for_agent`）——enabled 检查**不能**放这里，否则违反裁决 5。
- 无全局默认 agent：`DEFAULT_AGENT_REFERENCE_ID = "claude"` 硬编码（`garyx-gateway/src/agent_identity.rs:13`），`selected_agent_reference_id` 做 requested → current → "claude" 兜底。
- 消费入口清单：`POST /api/threads`（`routes.rs:2812` → `create_thread_for_agent_reference`）、task create/assign（`tasks.rs` `resolve_task_executor_agent:715` / `validate_thread_runtime_allows_assignee:746`）、automation create/update（`automation.rs:503 resolve_automation_agent_id`）+ cron 派发（`cron.rs:1269` → `create_thread_for_agent_reference`）、bot 入站新线程（`garyx-router` `default_agent_for_channel_account`、`INBOUND_FALLBACK_AGENT_ID`）、CLI `agent`/`task create --agent`/`thread create --agent`（全部薄客户端，校验在服务端）。**CLI 无 `task assign` 子命令**（assign 仅 HTTP）。
- 开关先例：MCP `PATCH /api/mcp-servers/{name}/toggle`（无并发 token）、skills toggle、channel account `enabled`（list 展示 + 派发拦截）。
- 桌面：AgentsHubPanel 表格（Name/Provider/Type/Actions）；picker 过滤维度已有 `standalone`（`agent-options.ts` `buildAgentOptions`）；新建草稿 agent 初值 `pendingAgentId = "claude"` 硬编码（`AppShell.tsx:652`，reset 点 2845/3157/3335）；无任何默认 agent 持久化；编辑走 `expectedUpdatedAt` 条件更新。
- iOS：saved default = **纯本地 UserDefaults** `selectedAgentTargetId`（gateway-scoped，默认 "claude"），`Use` 是唯一 mutator 契约（但 create/update 成功后也偷偷 set，与文档相悖的现状）；picker 排除先例 = `GaryxMobileAgentTargetMapper.makeTargets` `.filter(\.standalone)`；停用展示先例 = bot 的 `GaryxStatusPill` Enabled/Paused + swipe Enable/Disable；catalog 缓存 `GaryxMobileCatalogCacheSnapshot` `currentVersion = 3`，加字段须镜像 `GaryxCachedAgent` 并升版本。

## 3. 设计

### 3.1 数据模型

- `CustomAgentProfile` 加 `enabled: bool`，serde 默认 `true`（旧 `custom-agents.json` 无字段 → 全启用，天然向后兼容）。
- **built-in 也可停用**（比如不想让人用 antigravity）。因 persist 过滤 built-in profile，持久化格式扩展：`custom-agents.json` 顶层增加 `disabled_builtin_ids: Vec<String>`；load 时对播种的 built-in 应用 `enabled=false`。旧格式（无该字段）照常加载。
- 默认 agent 存储：`custom-agents.json` 顶层增加 `default_agent_id: Option<String>`。与 agent 同库同锁同持久化路径，**不进 `garyx.json`**（这是应用状态不是配置，且避免 settings deep-merge 无校验写入成为旁路）。

### 3.2 API

- `GET /api/custom-agents`：每个 agent 序列化带 `enabled`；响应顶层加 `default_agent_id`（原始存储值，可为 null）。
- 新增 `PATCH /api/custom-agents/{agent_id}/toggle`，body `{"enabled": bool}`：built-in 与 custom 一致可用（绕开 upsert 的 built-in 拒绝）；镜像 MCP toggle 先例，**不带并发 token**（单字段翻转 last-write-wins 可接受）。custom agent 走 PUT 全量更新时也可携带 `enabled`（表单编辑顺手改）。
- 新增 `PUT /api/custom-agents/default`，body `{"agent_id": String}`：校验目标存在 + `standalone` + `enabled`，否则 400。GET 走 list 响应，不单开端点。
- toggle / set-default 成功后与现有 CRUD 一致：`bridge.replace_agent_profiles` + `reload_from_config`。

### 3.3 服务端默认解析链（替换 "claude" 硬编码）

未显式指定 agent 时（`POST /api/threads` 无 `agent_id`、CLI 省略 `--agent`、automation 未指定等）：

```
存储 default_agent_id（存在 且 enabled 且 standalone）
→ "claude"（enabled 时）
→ 第一个 enabled 的 standalone agent
→ 明确错误（全停用时新建失败，不静默兜底）
```

- 显式指定 disabled agent 的新建请求：**直接拒绝并明确报错，绝不 fallback**（用户指名要 A，不能悄悄给 B）。
- 停用当前默认 agent：**不改写存储的 `default_agent_id`**，解析时跳过走 fallback；重新启用即自动恢复为默认。
- 允许停到一个不剩：新建报明确错误即可，不做"最后一个不许停"的隐藏防护。

### 3.4 拦截点（enabled 只在「新建绑定」时检查）

新增独立 gate（如 `ensure_agent_enabled_for_new_binding`），**不进共享 `resolve_agent_reference`**（裁决 5：续跑路径不受影响）。逐点落位：

| # | 路径 | 位置 | 行为 |
|---|---|---|---|
| 1 | 新建线程（HTTP/CLI/cron） | `create_thread_for_agent_reference` | disabled → 400/明确错误 |
| 2 | fork 线程 | 同上（fork 也是新建线程） | 拦 |
| 3 | task create / assign | `resolve_task_executor_agent` + assign 校验 | disabled → UnknownAgent 同级的明确错误（不复用 UnknownAgent 文案，单独 `AgentDisabled`） |
| 4 | automation create/update 选 agent | `resolve_automation_agent_id` | disabled → 400 |
| 5 | automation cron 触发时 agent 已被停用 | cron 派发（经 #1） | 该次派发失败并**记录错误可见**（automation 状态/日志），不静默吞 |
| 6 | bot 渠道入站消息需要新建线程 / `/newthread` | router 入站新建路径 | 新会话拒绝并**回渠道明确错误文案**（"agent 已停用"）；已绑定线程照常对话 |

明确**不拦**的路径（实现与评审都要验证不受影响）：已有线程发消息/续跑/steer、`thread send` 唤醒已有 task 返工、bridge 对已绑定 agent 的 run 派发、recovered session 恢复。

### 3.5 CLI

- `garyx agent list`：人读输出每个 agent 加启用状态与默认标记（如 `Agent: codex (built-in, disabled)` / `Agent: claude (built-in, default)`）；`--json` 每项加 `enabled`，顶层加 `default_agent_id`。
- 新增 `garyx agent enable <id>` / `garyx agent disable <id>`（走 toggle 端点）。
- 新增 `garyx agent default [<id>]`：无参显示当前默认（含解析后生效值），有参设置（走 PUT default）。
- `garyx task create --agent <disabled>` / `garyx thread create --agent <disabled>`：透传服务端明确报错。

### 3.6 桌面端

- AgentsHubPanel：新增 **Enabled 列**（`Switch`，走 toggle API，模板=McpSettingsPanel）+ **Default 标记**（badge）+ 行动作 "Set default"。
- picker（ComposerForm 等）：**disabled 直接隐藏**，与 `standalone` 过滤同层实现（`agent-options.ts`）；管理面才展示停用态（对齐"不可选"字面语义 + standalone 先例；不选置灰方案，减少 picker 噪音）。
- 新建线程草稿默认选中 = gateway `default_agent_id` 解析值：替换 `AppShell.tsx` `pendingAgentId` 的 `"claude"` 硬编码初值与全部 reset 点。`Chat` 动作保持一次性 override 不变。
- 渠道设置里 agent 下拉同样排除 disabled；**已绑定** disabled agent 的账号行沿用 `.missing` 式置灰展示（可见不丢）。

### 3.7 iOS

- 管理列表：行加 `GaryxStatusPill` Enabled/Paused + swipe Enable/Disable（模板=bot 行）。
- picker：`GaryxMobileAgentTargetMapper.makeTargets` 增加 `.filter(\.enabled)`（与 standalone 过滤并列）→ 三个 picker（新线程 sheet / target popover / automation agentRow）自动排除。
- **`Use` 升级为写 gateway 全局默认**（PUT default）；本地 `selectedAgentTargetId` 降级为 gateway 值的本地缓存（离线/冷启预选用，联网后以 gateway 为准同步）。**不保留"仅本机默认"双轨**——单一全局默认，多端一致（这正是"多渠道搞定"的要求）。
- create/update 成功后**不再自动改默认**（现状偷偷 set 有跨设备副作用，且与 "Use owns default" 契约相悖，借机修正）。
- `GaryxAgentSummary`/`GaryxCustomAgentRequest` 加 `enabled`；`GaryxCachedAgent` 镜像 + `GaryxMobileCatalogCacheSnapshot` 升 version；默认解析/回退纯逻辑（现分散在 App 层 `ensureSelectedAgentTarget`/`newThreadAgentTargetId`）**下沉 GaryxMobileCore** 配 SwiftPM 测试。
- `ensureSelectedAgentTarget` / automation `ensureAgentSelection` 的回退逻辑感知 disabled（跳过停用项）。

### 3.8 兼容性

- 旧 `custom-agents.json`（无新字段）、旧 catalog 缓存快照均可加载；serde/Codable 默认值兜底。
- 不做旧网关兼容设计（既有裁决：desktop 与 gateway 同仓同发恒配套）；iOS 老版本对未知字段天然忽略，disabled agent 在老客户端仍可见可选，但服务端 gate 兜底拒绝——安全性以服务端为准。

## 4. 测试计划（headless 优先）

- **gateway**：store 持久化往返（含 builtin disabled、default_agent_id、旧格式加载）；toggle 端点（built-in + custom）；PUT default 校验（不存在/disabled/非 standalone → 400）；默认解析链全分支（default 停用回退、全停报错）；六个拦截点逐个拒绝 disabled；**反向用例：disabled agent 的已有线程继续发消息成功、已有 task thread send 返工成功**。
- **CLI**：list 人读/`--json` 输出、enable/disable/default 命令、task create --agent disabled 报错。
- **iOS Core SwiftPM**：makeTargets enabled 过滤、缓存升版本迁移、下沉后的默认解析纯逻辑（含 disabled 跳过）。
- **desktop**：`agent-options` 过滤、默认预选来源（`npm run test:unit`）。

## 5. 开放问题

无阻塞项。picker 隐藏 vs 置灰已在 3.6 拍板为隐藏；如用户倾向置灰可在实现前改一行结论。
