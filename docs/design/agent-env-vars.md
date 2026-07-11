# Agent 环境变量（Env Vars）设计

> 设计阶段产物：只描述方案 / 契约 / 影响面 / 取舍，不含生产代码、不开实现子任务。
>
> 目标：让每个 Garyx **custom agent** 携带一组 env（key→value），CLI / Mac desktop / iOS
> 三端可编辑与展示；agent 起 run 时下游 provider 子进程能吃到这些变量。

## 修订记录

- **v2.1（回应 #TASK-1530 第 2 轮 NOT-PASS 的 1 个 blocker）**：iOS `unchanged` intent 文档内自相矛盾
  （Q3 说省略 `provider_env`、Q5/§4.5 说沿用 `baseAgent.providerEnv` 整张回写）。统一为
  **`unchanged`→省略 `provider_env`（nil，靠 gateway 缺省保留），只有 replace/clear 才发 map**；
  权威 env 只用于 seed + 算 replace diff；§5 测试改断言「未触碰 env 的 update 省略 `provider_env`」。
  同步 Q5 / §4.5 / §5 / §6。
- **v2（回应 #TASK-1530 NOT-PASS 的 3 个 blocker + 1 收窄）**：
  1. **桌面落点纠正**：实际渲染的是 `AgentsHubPanel`（不是死组件 `AgentsPanel`），§0.2/§4.4/Q5
     全部改指 `AgentsHubPanel` 并抽共享 payload 纯函数（Blocker 1）。
  2. **Q4 安全模型改写**：env 值是运行时能力，任何 provider/tool 回显都可能被持久化进
     transcript/thread-log（redaction 是按 key 名，救不了「命令回显 secret 值」）；改为**明确接受
     + UX 提示 + 只保证实现自身不主动打印 env map**，不做值级 transcript redaction（Blocker 2）。
  3. **iOS 保留语义补 intent**：Core env draft 显式区分 `unchanged/replace/clear`，编辑前必须先取
     **权威** env（cache 会剥掉 providerEnv），SwiftPM 测 restored-cache 不误清空（Blocker 3）。
  4. **§0.3 收窄**：`profile.provider_env` 编辑面向 custom agent；但 `garyx.json agents.*.env`
     （`AgentProviderConfig.env`）对默认 provider 独立存在，不写成「所有 env 仅 custom agent」的全局红线。
- v1：初稿。

---

## 0. 读码结论（带 file:line）—— 关键纠偏

任务描述里写「**核心缺口:garyx-bridge 里子进程 provider 完全不消费 provider_env**」。
**这个前提是错的**，来自 `rg provider_env garyx-bridge/src` 的搜索范围误判：bridge 侧字段
被重命名为 `env`，所以搜 `provider_env` 搜不到。逐行核对源码后确认（reviewer #TASK-1530 已独立复核成立）：

### 0.1 env 注入其实已全链路打通并有测试

profile 字段 → 运行期 config → 每个子进程 `Command`：

- `CustomAgentProfile.provider_env: HashMap<String,String>` — `garyx-models/src/custom_agent.rs:41`
  （serde `alias = "env"`/`"providerEnv"`，`skip_serializing_if = HashMap::is_empty`）。
- `to_provider_config()` 把它拷成 `AgentProviderConfig.env` — `custom_agent.rs:97`。
- `AgentProviderConfig.env: HashMap<String,String>` — `garyx-models/src/config.rs:200`。
- **运行期**取 config：非内建 standalone agent 走 `profile.to_provider_config()` —
  `garyx-bridge/src/multi_provider/lifecycle.rs:423`（native 模型注册路径 `:622` 同样）。
- `provider_factory.rs` 每个 builder 都 `env: agent_cfg.env.clone()`：
  claude `:37` / codex(+traex) `:73` / gemini `:99` / antigravity `:130` / native `:162`。
- 子进程 `Command` 真正吃到 env：
  - **Claude Code / cctty**：`claude_provider.rs:1050` `let mut env = self.config.env.clone();`
    → `ClaudeAgentOptions.env`（`:1089`）→ SDK `claude-agent-sdk/src/transport.rs:97-99`
    `for (k,v) in &self.options.env { cmd.env(k,v); }` → `spawn()`（`:121`）。
  - **Codex / Traex**：`codex_provider.rs:500` `resolve_runtime_codex_env` → `CodexClientConfig.env`
    → SDK `codex-sdk/src/transport.rs:164-168` `Command::new(codex_bin)` … `cmd.envs(&self.env)`
    → `spawn()`（`:177`）。
  - **Gemini CLI**：`gemini_provider.rs:1058` `command.envs(resolve_runtime_gemini_env(&self.config,…))`
    → `spawn()`（`:1060`）。
  - **Antigravity CLI**：`antigravity_provider.rs:1020` `command.envs(resolve_runtime_antigravity_env(…))`
    → `spawn()`（`:1025`）。
- 现存断言 env 已生效的测试：`garyx-bridge/src/claude_provider/tests.rs:827`
  `test_build_sdk_options_merges_config_env_and_metadata_env`（构造 `ClaudeCodeConfig{env}` 断言
  merged `sdk_opts.env`）、codex `codex_provider/tests.rs:281`、gemini `gemini_provider/tests.rs:178`。

**结论：claude code / codex / gemini / antigravity 四个子进程 provider 早已注入 env。** 因此本设计
**不重写注入**，而是（a）补齐三端「编辑 / 展示」的**表面缺口**，（b）修一个桌面数据丢失路径，
（c）补 native provider 的一个注入漏洞，（d）加**确定性 Command-level 测试**把行为**锁死**。

### 0.2 三处真实缺口

1. **CLI 无通用 env flag**。只有 `--provider-api-key` 特化：`garyx/src/commands.rs:3754-3774`
   把它映射成 `body["provider_env"] = json!({env_name: api_key})`——**单键整体替换**整张 map。
   没有 `--env K=V` / `--unset-env K`。（`cli.rs:1063-1179` 三个写命令的 flag 集）。
2. **Desktop 无通用 KV 编辑器 + 一条数据丢失路径 —— 且落在 active 组件 `AgentsHubPanel`**。
   AppShell 懒加载 / 渲染的是 `AgentsHubPanel`（`AppShell.tsx:325` lazy、`:10663`/`:10678` 渲染）；
   `AgentsPanel.tsx` **无人 import（死组件）**。`AgentsHubPanel.handleAgentSubmit`（`:1118`）里
   `providerEnv` 只从 apiKey 重建（`:1127-1131`）：native 且 apiKey 非空 → `{apiKeyEnv: key}`
   **整张替换**（丢别的 key）；否则 → `null`（gateway 保留）。**后果**：claude_code / codex / gemini_cli
   这些最需要 env 的 provider，桌面**根本没有入口**编辑通用 env；native provider 另配的 env 一存就被抹。
3. **iOS 完全没有 env 编辑 UI**（`GaryxMobileAgentsViews.swift` 的 `GaryxAgentFormContent:246-377`
   只绑 8 个字段）。`updateAgent`（`GaryxMobileModel+AgentsWorkspaces.swift:726`）**仅在**
   `catalogSnapshotRestored` 时回捞权威 agent（`:748`），并把 `baseAgent.providerEnv` 整张回传
   （`:779`，空则 nil）；`createAgent` 不带 providerEnv（`:703-712`）。→ 现状「保留但不可编辑」，
   且**编辑 UI 一旦落地，若把空 draft 当完整 map 发，会清空隐藏 env**（见 Q3/§4.5）。

### 0.3 存储与合并现状

- 持久化文件是 `~/.garyx/data/custom-agents.json`（**不是** `garyx.json`），
  `local_paths.rs:49-55`；`persist()` 过滤内建、明文全量覆盖写 —— `custom_agents.rs:271-285`。
- Gateway upsert 是 **merge**：`custom_agents.rs:211-213` `provider_env: requested.or_else(existing)`
  —— 给了 `Some(map)` 就**整张替换**、给 `None` 就**保留旧的**（无 per-key 合并）。
  入站 map 先 trim key/value、丢空 key —— `:144-156`。
- **无任何脱敏**：`custom_agent_response`（`api.rs:89-101`）原样返回 `provider_env`（明文）；
  `agent get --json` 也是明文。
- **本 feature 的编辑面向 custom（非内建）agent**：内建 agent 走 `default_provider_config_for_type`
  （`lifecycle.rs:421-428`），绕过 `to_provider_config()`；且 gateway 拒绝改内建
  （`custom_agents.rs:177-182`）。**但这不是「env 只存在于 custom agent」的全局红线**——`garyx.json`
  的 `agents.*` 就是 `AgentProviderConfig`（`config.rs:183`，含 `env` 字段 `:200`），可为默认 provider
  独立配置 env，只是**不在本 feature 三端编辑器的范围内**。要给「claude」加 env 有两条路：建
  `provider_type=claude_code` 的 custom agent（本 feature 覆盖），或手改 `garyx.json agents.*`（既有、不覆盖）。

---

## 1. 目标与非目标

**目标**
- 三端都能对 custom agent **配置 / 编辑 / 查看** env：CLI（create/update 通用 env flag + get/list 展示）、
  Mac `AgentsHubPanel`（通用 KV 列表编辑器）、iOS agent 编辑表单（native grouped 分组）。
- agent 起 run 时 **claude code / codex 子进程环境里真有配置的变量**，有**确定性测试**证明；
  其它 provider 逐个落实或明确豁免。
- 修掉桌面「保存丢 env」路径；补掉 native provider `exec_command` 不注入 env 的漏洞。
- 全量相关测试绿；三端语义一致；不破坏 agent update 的合并语义（未传字段保留，含头像）。

**非目标**
- 不重写 bridge 注入（已工作）。不引入 channel 层 env。不改 `garyx.json agents.*` 的编辑面。
- 不给内建 agent 加 env（架构边界）。不做 env 的服务端加密存储（config = 普通应用状态）。
- 不做 env 引用/插值/模板（值就是字面字符串）。
- 不做 provider 输出（transcript/thread-log）里 env **值**的自动 redaction（见 Q4 取舍）。

---

## 2. 设计问题逐条回答

### Q1. 语义统一：复用 `provider_env` 还是新字段？—— **复用**

**决定：复用 `provider_env` 这一张 map，不加新字段。** 理由：

- 它已经是**唯一**被所有子进程 provider 消费的 env 容器（§0.1）。新字段会造成「两个 env 源，
  谁覆盖谁」的二义性，且要改 5 个 `build_*_config` + 4 条 spawn 路径 + 三端契约，纯属倒退。
- 现有 `apiKey` **不是**独立字段，它本就是「往这张 map 里写一个众所周知的 key」的**快捷入口**：
  desktop `apiKeyEnvName()`（`AgentsHubPanel` 内）、CLI（`commands.rs:3759-3762`）都把
  `gpt→OPENAI_API_KEY` / `anthropic→ANTHROPIC_API_KEY` / `google→GEMINI_API_KEY`。

**语义收敛**：`provider_env` 是**唯一真相源**；`apiKey` 降级为「这张 map 里那个众所周知 key 的
派生视图」。通用 KV 编辑器与 apiKey 快捷入口**同编辑一张 map**：

- 通用 KV 编辑器展示 / 编辑**全部** key（含 `OPENAI_API_KEY` 那一行）。
- apiKey 快捷字段（仅 native provider 显示）= 读写这张 map 里对应众所周知 key 的便捷控件。
  实现取「**KV 编辑器为真相、apiKey 是便捷写入口**」的单向模型（apiKey 改动即写入 KV 那一行；
  KV 那一行的值反映到 apiKey 只读展示）——**避免双向绑定的状态一致性坑**（§6 风险）。

字段名保持 `provider_env`（profile 侧）/ `env`（`AgentProviderConfig` 侧）**不改名**
（既有 serde alias `env`/`providerEnv` 已兜底，改名纯 churn）。三端 UI 文案统一叫 **"Environment Variables"**。

### Q2. 注入范围：逐 provider

overlay 语义（**已是现状，保持**）：**继承 garyx 进程环境**之上按顺序 overlay，**后者覆盖前者**：

```
继承的进程 env  <  agent env (config.env)  <  task_cli_env(GARYX_* 身份)  <  desktop_*_env(每请求覆盖)
```

锚点：claude `claude_provider.rs:1050-1052`（`env.clone()` → `extend(task_cli_env)` → `extend(desktop_claude_env)`），
codex `codex_provider.rs:500-508`、gemini/antigravity `resolve_runtime_*_env` 同序。没有任何 spawn 调
`.env_clear()`，所以子进程**继承父环境**、agent env 只做叠加。

**关键不变量**：agent env 排在 `task_cli_env` **之前**，所以 **agent env 不能覆盖 `GARYX_THREAD_ID`
/ `GARYX_AGENT_ID` / `GARYX_TASK_ID` 等线程身份变量**（身份被保护）；`desktop_*_env` 排最后，可覆盖
agent env（保持既有优先级，本设计不改）。

| Provider | 是否注入 agent env | 落点 / 说明 |
|---|---|---|
| **Claude Code** | ✅ 已注入 | `transport.rs:97-99`（SDK `cmd.env`）。本设计加 Command-level 测试锁死。 |
| **cctty**（claude 的一个 mode）| ✅ 已注入 | 与 Claude Code **同一条** `ClaudeAgentOptions.env` → 同 transport；cctty 只改 `cli_path`/prefix args，不改 env 路径。随 claude 一并被测试覆盖。 |
| **Codex** | ✅ 已注入 | `codex-sdk/transport.rs:167-168`（`cmd.envs`）。加 Command-level 测试锁死。 |
| **Traex**（codex fork）| ✅ 已注入 | 共用 `CodexAppServerConfig` + codex-sdk transport；agent `config.env` 照常注入。注意 codex 对 Traex **跳过**的是 `desktop_codex_env`（`codex_provider.rs:502-508`，桌面每请求 OPENAI 覆盖），**不是** agent env。 |
| **Gemini CLI** | ✅ 已注入 | `gemini_provider.rs:1058`。测试见 §5。 |
| **Antigravity CLI** | ✅ 已注入 | `antigravity_provider.rs:1020`。测试见 §5。 |
| **in-process native**（Gpt / ClaudeLlm / GeminiLlm）| ⚠️ 部分 | LLM 调用是**进程内** HTTP（`garyx_native_provider.rs:473`），无子进程；env 用于**进程内**解析凭证（`resolve_runtime_env` → `LlmRuntimeContext.env`，`:1038`），这本就是 apiKey 快捷键落地方式。其 **MCP stdio 工具**子进程已注入 env（`native_capabilities.rs:673`）。**唯一漏洞**：`exec_command` 的 `zsh -lc` 子进程（`:805-809`）**没** `.envs()`。见下方决定。 |

**native `exec_command` shell 工具 —— 决定：注入 agent env（叠加在继承环境之上）。**
理由：一致性——agent 的 env 应到达它执行代码的**每一处**；capability(MCP) 工具已注入
（`native_capabilities.rs:673`），shell 工具不注入是不对称漏洞（例如 build 命令需要一个配置的
token / PATH 前缀）。改动极小：`exec_command_tool`（`garyx_native_provider.rs:784-813`）拿
`resolve_runtime_env(&self.config, metadata)`（需把 `metadata` 从 `run_tool:761-766` 传入）后
`command.envs(runtime_env)`，抽可测的 `build_exec_command(...)->Command`。**其输出泄漏风险在 Q4 统一处理**。

**PATH / 危险变量（LD_PRELOAD / DYLD_*）—— 决定：不在注入层拦截 / 不消毒。**
agent env 覆盖继承环境是**有意能力**，注入层保持 **dumb / 可预测**；客户端展示层对 well-known
敏感 / PATH 类 key 给**非阻断**提示即可。身份变量已被 overlay 次序保护。

### Q3. 合并语义：**wire 整张替换，客户端 read-modify-write，且带显式 intent**（三端一致）

- **Wire / 存储契约不变**：`provider_env` 出现 = **完整期望 map（整张替换）**；缺省 = **保留**。
  Gateway 保持 `custom_agents.rs:211` 的 `or_else` 语义，**不加 per-key merge 协议**。
  清空 = 发 `Some({})`（gateway trim 空后为空 map → 整张替换为空）。
- **客户端语义（三端统一）**：部分编辑用 **read-modify-write**，且 draft 带**显式 intent**，
  避免「空编辑器 = 清空」这类误伤（Blocker 3）：
  - `unchanged`：用户没碰 env → **不发** `provider_env`（gateway 保留）。
  - `replace(map)`：用户有增删改 → 发**完整**期望 map（`Some(map)`）。
  - `clear`：用户显式清空 → 发 `Some({})`。
  - 施加 diff 前，**必须**先拿到**权威** env（`agent get`）再 seed 编辑器（尤其 iOS：本地 catalog
    cache 会剥掉 `providerEnv`，见 §4.5 / Blocker 3）；不能拿被剥过的投影当完整 map 回写。
- **CLI flag 设计**（可重复）：`--env KEY=VALUE`（upsert 单 key）、`--unset-env KEY`（删单 key）、
  `--env-clear`（清空）。有任一 env flag → intent=replace/clear → 先 GET 当前 agent 合并 → 发整张 map；
  无 env flag → intent=unchanged → 不发 `provider_env`。`--provider-api-key`（保留兼容）降级为语法糖
  `--env <众所周知key>=<值>`，**并入**合并（不再整张替换、不丢别的 key）；gpt 未显式给 auth-source
  时仍自动置 `auth_source=api_key`（保留 `commands.rs:3771-3773`）。
- 空 key 行丢弃（gateway 也 trim）；**空 value 保留**（`KEY=` 合法，参考 codex 保留空
  `OPENAI_API_KEY` 的既有测试 `codex_provider/tests.rs:346`）。重复 key：序列化后者覆盖 + UI 非阻断提示。
  key 合法性：客户端 UX 校验正则 `^[A-Za-z_][A-Za-z0-9_]*$`（可放 `garyx-models` 共享 `is_valid_env_key`）；
  gateway 保持既有 trim-空 不加硬拒。

### Q4. secret 处理（安全模型 v2）

- **存储**：明文存 `custom-agents.json`（仓库约定：config 普通应用状态，**不加** token 特化
  redaction / merge / preservation 路径）。gateway 读路径保持明文——因为 iOS `updateAgent` 依赖 GET
  回捞完整 map 才能保留、desktop 编辑器也需读回值来编辑。**脱敏是纯展示层的事，与「存储 = 普通状态」正交。**
- **展示脱敏（只在展示层）**：desktop env 值输入 `type="password"` + 每行 show/hide 眼睛；
  iOS 值用 `SecureField` + 每行揭示；CLI `agent get`/`list` 人读输出打 key + 掩码值（如 `KEY=••••••`），
  `--json` 保持明文全量（脚本化显式 opt-in，help/文档注明不脱敏）。
- **真实泄漏模型（修正 v1 的错误表述）**：env **值**一旦注入 agent 执行环境，就是**运行时能力**——
  **任何** provider/tool 若在输出里回显它（例如 `echo $TOKEN`、或某工具打印环境），该值会随
  **tool result / assistant 输出**被持久化进 transcript / thread-log，且 thread-log 的 redaction 是
  **按 key 名**（`thread_logs.rs:84` 的 `is_sensitive_key`），**救不了「命令回显了 secret 值」**。
  具体路径：native `exec_command` 把 `stdout`/`stderr` 放进 tool result（`garyx_native_provider.rs:814-819`）
  → 持久化到线程消息（`run_management.rs:2040`）→ thread-log 摘要落普通 `message` 字段（`run_management.rs:1853`）。
  **注意这不是本 feature 或 native shell 独有**：claude/codex 子进程同样继承 env，也能回显。
- **决定：明确接受该固有风险 + UX 提示，不做值级 transcript redaction。** 理由：对任意工具输出做
  secret **值**匹配脱敏，误报率高、维护面大，且违反「config 普通应用状态、不加 token 特化路径」。取而代之：
  1. 三端编辑器在 env 分区放一句非阻断提示：*"Values become environment variables for this agent's
     runs and may appear in command output/logs. Don't store secrets you can't rotate."*
  2. **实现层保证**：注入路径**自身**绝不主动打印 / log env map（复核 claude/codex/gemini/antigravity
     spawn 与新加的 native shell 注入），并加测试断言（§5「no-proactive-log」）。

### Q5. 三端 UI 形态（Mac 为 IA 真相源）

**Mac `AgentsHubPanel`（active 组件，先定）**：编辑表单新增 **"Environment Variables"** 分区——一列
KV 行（`KEY` 输入 + `VALUE` password 输入 + 眼睛揭示 + 删除）+ 底部「Add variable」+ 非阻断提示（Q4）。
- 打开编辑时用 `agent.providerEnv` **全量** seed（按 key 字母序稳定排序，因 map 无序）。
- native provider 的 apiKey 快捷字段保留，作为 KV 那一行的便捷写入口（Q1 单向模型）。
- 保存：以 KV 编辑器的**整张 map**为准（含 apiKey 行）→ 修掉 `:1127-1131` 丢 key bug。
- 抽**纯函数** `buildProviderEnvPayload(envRows, providerType, apiKey)`（合并 / 去空 key / apiKey 并入 /
  返回整张 map 或 undefined 表示 unchanged）供 `handleAgentSubmit` 调用与单测。
- 若确认 `AgentsPanel.tsx` 全仓无 import（初查如此），可作为**可选清理**删除以免两份分叉；删除与否不阻断本 feature。

**iOS（跟随，native grouped）**：编辑表单加分组 Section "Environment Variables"——每行左 key、
右 `SecureField` 值 + 揭示；`swipeActions` 删除；一行「Add Variable」+ 提示。**不发明新概念**。
- draft / 校验 / 序列化 / seed-from-authoritative / **intent（unchanged/replace/clear）** / diff 逻辑进
  `GaryxMobileCore`（纯函数 + SwiftPM 测试）；app target 只做 SwiftUI 组合与绑定。
- 遵守 `mobile-ui.md`：字段名常驻左、值 / 控件在右；不靠 placeholder 当唯一 label；
  **restored 投影只用于展示，编辑前取权威数据再保存**。
- `updateAgent` 依 intent 应用：**unchanged→`providerEnv = nil`（省略 `provider_env`，靠 gateway
  缺省保留权威值，不回写快照）**；replace→`Some(map)`；clear→`Some([:])`。

**CLI**：`--env`/`--unset-env`/`--env-clear` + `--api-key` 语法糖（Q3）；`get`/`list` 人读掩码、
`--json` 明文（Q4）。三端语义一致（都表达 intent + 完整期望 map）。

---

## 3. 数据契约与不变量（红线）

1. `provider_env`（profile）/ `env`（`AgentProviderConfig`）是**唯一** env 真相源；不新增字段、不改名。
2. Wire：`provider_env` present = 整张替换、absent = 保留、`{}` = 清空。Gateway **不加** per-key merge。
3. 部分更新一律客户端 **read-modify-write + 显式 intent**；编辑前取权威 env 再 seed。
4. overlay 次序不变：`继承 < agent env < GARYX 身份 < desktop 覆盖`；agent env 不得覆盖 GARYX 身份。
5. 本 feature 三端编辑器面向 **custom（非内建）agent**；`garyx.json agents.*.env` 不在编辑范围（但客观存在）。
6. 存储明文、gateway 读路径明文（客户端依赖），脱敏只在**展示层**；不加 token 特化存储路径；
   **不做 provider 输出里 env 值的自动 redaction**（固有风险，明确接受 + UX 提示 + 实现不主动打印）。
7. 合并语义（未传字段保留，含 avatar / created_at / system prompt 等）**不破坏**。

---

## 4. 分层实现落点（带 anchor，供 review 逐条核）

### 4.1 garyx-models / gateway —— 基本不动
- `provider_env` 字段、`to_provider_config()`、gateway payload / merge **无需改**。
- （可选）`garyx-models` 加 `pub fn is_valid_env_key(&str)->bool` + 单测，供 CLI 复用。
- 不加 gateway 脱敏 / 不加 per-key merge（红线 §3.2/§3.6）。

### 4.2 garyx-bridge —— 补 native shell 注入 + 确定性测试
- `garyx_native_provider.rs` `exec_command_tool`（`:784-813`）注入
  `resolve_runtime_env(&self.config, metadata)` overlay env（`metadata` 从 `run_tool:761-766` 传入）；
  抽 `build_exec_command(...)->tokio::process::Command` 便于断言 env。复核不打印 env（Q4）。
- claude / codex / gemini / antigravity **不改注入逻辑**，只加 / 扩测试（§5）。

### 4.3 CLI（`garyx/src/cli.rs` + `commands.rs`）
- `cli.rs:1063-1179` 三个写命令加 `--env`（`Vec<String>` KEY=VALUE，可重复）、
  `--unset-env`（`Vec<String>`）、`--env-clear`（bool）。
- `commands.rs`：出现任一 env flag 时，`cmd_agent_update`/`upsert` 先 GET `/api/custom-agents/{id}`
  拿现有 `provider_env`（create 从空 map 起），按 clear→unset→upsert 施加，产出整张 map 塞进 body；
  `--provider-api-key` 改为并入这张 map（替换 `build_agent_mutation_body:3754-3774` 的整张覆盖）。
  无 env flag → 不发 `provider_env`（intent=unchanged）。
- `print_agent_summary`（`commands.rs:3924-3957`）人读输出加 env 段（key + 掩码值）；`--json` 明文。
- 解析 `KEY=VALUE`：按**首个** `=` 切分（value 可含 `=`）；空 key 报错；key 走校验。

### 4.4 Desktop（`AgentsHubPanel.tsx`，active 组件）
- `agentDraft`（HubPanel 内）加 `env: Array<{key,value}>`（有序、稳定排序）；打开编辑从
  `agent.providerEnv` 全量 seed；渲染 KV 分区 + 提示；apiKey 字段作 KV 那行的便捷写入口（Q1 单向）。
- 抽纯函数 `buildProviderEnvPayload(...)` 供 `handleAgentSubmit`（`:1118-1147`）调用与单测。
- `contracts.ts` / `gary-client.ts` 无需改（`providerEnv: Record<string,string>` 已在；
  create/update 已发 snake `provider_env`）。值输入 `type="password"` + 眼睛揭示。
- （可选）删死组件 `AgentsPanel.tsx` 以免分叉。

### 4.5 iOS（`mobile/garyx-mobile`）
- `GaryxMobileCore` 新增 env draft 纯逻辑文件：有序 rows 模型、`is_valid_env_key` 等价校验、
  重复 / 空 key 处理、**intent（`unchanged`/`replace([String:String])`/`clear`）**、
  `seed(fromAuthoritative:)`、`resolve(base:)->providerEnv 请求值`、diff + SwiftPM 测试
  （`Tests/GaryxMobileCoreTests/`）。**新文件必须 `xcodegen generate` 同步 pbxproj + 跑 xcodebuild 防假绿。**
- `GaryxMobileModel+AgentsWorkspaces.swift` `updateAgent`（`:726-810`）：**打开编辑器前取权威 env**
  （复用 / 扩展 `catalogSnapshotRestored` 回捞 `:748`，使编辑器 seed 的是权威 `providerEnv` 而非
  cache 剥过的投影）。该权威 env **只用于 seed 编辑器 + 计算 replace 的完整 map**，**不**用于 unchanged
  的回写。保存时按 intent 计算 `providerEnv` 请求值：**unchanged→`nil`（省略 `provider_env`，靠 gateway
  缺省保留；即改掉 `:779` 现在整张回写 `baseAgent.providerEnv` 的做法）**；replace→`Some(map)`；
  clear→`Some([:])`。`createAgent`（`:681-724`）：有 env draft 则发 `Some(map)`，否则不发。
- `GaryxMobileAgentsViews.swift` `GaryxAgentFormContent`（`:246-377`）加 Section + 绑定；
  `GaryxAgentEditSheet.fillDraft`（`:671-680`）/ `saveAgent`（`:682-697`）带 env draft + intent。
- `GaryxCustomAgentRequest`（`GaryxGatewayAgentModels.swift:551-619`，`provider_env` `:609`）已够用。

---

## 5. 测试策略（确定性、headless）

**必须项：claude / codex 子进程 Command env 含配置变量。**
- **Claude**：`claude-agent-sdk/src/transport.rs` 加 `#[cfg(test)]`——构造带
  `options.env = {"TEST_AGENT_ENV_KEY":"test-value"}` 的 transport，调 `build_command(None)`（`:65`），
  断言 `cmd.as_std().get_envs()` 含该键。airtight。
- **Codex**：从 `codex-sdk/src/transport.rs` `start()`（`:164-172`）抽 `build_command(&self)->Command`
  （只建不 spawn），单测断言 `get_envs()` 含配置变量。airtight。
- **native shell**：断言新 `build_exec_command` 的 Command env 含配置变量。airtight。
- **gemini / antigravity**：扩 `resolve_runtime_*_env` 测试断言配置变量在返回 map（该 map 下一行
  即 `command.envs(...)` 逐字喂入）；`gemini_provider/tests.rs:178` 已有骨架。
- **no-proactive-log（Q4）**：断言注入路径不主动把 env 值写进日志 / 结果——例如捕获 native shell
  注入的实现路径不含 env map 打印；spawn 处复核无 `tracing` env。
- **贯穿链**：`custom_agent/tests.rs`（`:79` 已调 `to_provider_config`）断言 `.env` 携带该 map；
  claude `tests.rs:827` 扩断言点。
- **CLI**：扩 `garyx/src/commands/tests.rs`（`:1463` 附近）——`--env A=1 --env B=2` 产出全 map；
  `--unset-env`；带现有 map 的 **GET-merge**（mock server 返回既有 `provider_env`，断言不丢 key）；
  `--api-key` 并入不丢 key；无 env flag 不发 `provider_env`。
- **Desktop**：`test:unit` 测 `buildProviderEnvPayload`（合并 / 去空 / apiKey 并入 / 保留全 key /
  unchanged 返回 undefined）；`build:ui` 类型守卫；打包 CDP 实机核 KV 编辑器（截图证据）。
- **iOS**：`GaryxMobileCore` SwiftPM 测 env draft 序列化 / 校验 / intent 解析 / seed；**新增
  restored-cache 场景测试**（Blocker 3）——用户**未触碰 env** 的 update 必须解析出 **intent=unchanged →
  请求里 `provider_env` 被省略（`nil`，靠 gateway 缺省保留），而非发送快照 `baseAgent.providerEnv`**
  （对齐 `GaryxMobileCatalogCacheTests.swift:5` 的剥离前提：即使 seed 的投影被剥空，unchanged 也只省略、
  不清空）；另测 replace 用权威 seed 得到完整 map、clear 发 `Some([:])`。
  `xcodebuild` 编过；模拟器真渲染核 native 分组表单。

**完成校验**：`cargo test`（models/gateway/bridge/CLI）+ desktop `test:unit` + `build:ui`
+ iOS `swift test` + `xcodebuild` 全绿。gateway 改动**不立即生效**：按
`scripts/build-local-cli.sh` 构建并重启 managed gateway 再端到端（起 run 验证子进程真拿到 env）。

---

## 6. 影响面 / 风险 / 取舍 / 兼容

- **兼容**：wire 契约、gateway merge、overlay 次序**均不变**；已有 apiKey agent、已有 `provider_env`
  数据零迁移。三端旧客户端仍工作（只是没有新编辑器）。
- **取舍**：客户端 read-modify-write（而非 gateway per-key merge）——换 gateway 简单 + 三端一致，
  代价是 CLI 在有 env flag 时多一次 GET（仅编辑路径）。secret 值不做 transcript redaction（Q4）——
  换避免高误报 / 守仓库约定，代价是需 UX 提示 + 用户自负回显风险。
- **风险**：
  - desktop apiKey↔KV **不做双向绑定**（Q1 单向：KV 为真相、apiKey 便捷写入口），规避状态一致性坑。
  - iOS：unchanged 恒省略 `provider_env`（天然安全）；残留风险在 **replace 路径**——若 seed 用了
    cache 剥空的投影而非权威 env，replace 的完整 map 会漏掉未展示的 key。故「打开编辑器前取权威 env」
    是 replace 正确性的前提；由 §4.5 + restored-cache 测试守护（Blocker 3）。
  - native shell 注入是行为变化（此前只继承父环境）+ 回显泄漏（Q4）——测试锁定 + UX 提示 + 明确接受。
- **明确豁免**：内建 agent（架构边界）；`garyx.json agents.*.env`（既有、不在编辑面）；
  in-process native 的 LLM HTTP 调用（无子进程，env 走进程内凭证解析，已工作）。

---

## 7. 非目标与红线（复述，便于 review 卡）

- ❌ 不重写 bridge 注入；❌ 不加新 env 字段 / 不改名；❌ 不加 gateway per-key merge 协议；
  ❌ 不加 token 特化存储 / redaction 路径、❌ 不做 provider 输出里 env 值的自动脱敏；
  ❌ 不给内建 agent / 不给 `garyx.json agents.*` 加编辑面；❌ 不做 env 插值 / 模板；
  ❌ agent env 不得覆盖 GARYX_* 身份变量；❌ 不在注入层消毒 PATH / 危险变量；
  ❌ 不做 apiKey↔KV 双向绑定（单向）。

## 8. 公共仓库卫生

fixture / 测试 / 文档一律合成占位：`TEST_KEY=test-value`、`${TOKEN}`、`OPENAI_API_KEY=test-openai-api-key`、
`/Users/test`、`Test User`、`1000000001`。**绝不**放真实 token / key / 个人数据；staging 前扫 diff。
