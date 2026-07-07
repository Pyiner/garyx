# Super Junior 通过 cc-switch 路由失效调查说明

> 日期：2026-07-07
>
> 状态：调查结论 + 修复方案说明。本文不包含生产代码改动。
>
> 修订（2026-07-07）：修复方案改为"下层统一"——删除 Desktop 的
> `providerMetadata.provider_env` 通道，env 解析全部收敛到 gateway/bridge。
> 原 deep-merge 方案降级为已否决备选，见文末。

## 背景

`super-junior` 是 Garyx custom agent，provider 是 `claude_code`，模型为
`claude-opus-4-8`，预期通过 cc-switch proxy 路由到 Super Relay 的
`model_api/experimental_0630`。

约束：

- 不覆盖 Claude Code 的全局配置。
- cc-switch 只打开 proxy，其他人仍手动配置。
- usage 统计只看 token，不需要价格。
- `super-junior.provider_env` 是 custom agent 的运行时 env 来源。

## 现象

同一个 `super-junior` 配置下：

- CLI `garyx thread send` 可以走 cc-switch。
- Garyx Mac App 里选择 `Super Junior` 后发消息，没有走 cc-switch。

这不是 `super-junior` 没有配置 env。实测线程 metadata 中已经存在：

```json
{
  "provider_env": {
    "ANTHROPIC_AUTH_TOKEN": "PROXY_MANAGED",
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721",
    "ANTHROPIC_MODEL": "claude-opus-4-8"
  }
}
```

cc-switch proxy 本机监听也正常：

```bash
lsof -nP -iTCP:15721 -sTCP:LISTEN
```

## 复现证据

### CLI 成功走 proxy

命令：

```bash
garyx thread create \
  --agent super-junior \
  --workspace-dir /Users/test/repos/garyx \
  --json

garyx thread send thread <thread_id> \
  '只回复 pong，不要输出其他内容。' \
  --timeout 180 \
  --json
```

cc-switch DB 中有 Super Relay 记录：

```text
2026-07-07 04:03:49 UTC
app_type=claude
provider_id=super-relay-model-api-experimental-0630
model=model_api/experimental_0630
request_model=claude-opus-4-8
input_tokens=46368
output_tokens=19
status_code=200
```

查询方式：

```bash
sqlite3 ~/.cc-switch/cc-switch.db "
select
  datetime(created_at,'unixepoch') as ts,
  app_type,
  provider_id,
  model,
  request_model,
  input_tokens,
  output_tokens,
  cache_read_tokens,
  cache_creation_tokens,
  status_code
from proxy_request_logs
where provider_id like '%super-relay%'
   or model like '%experimental_0630%'
order by created_at desc
limit 20;
"
```

### Mac App 没走 proxy

通过 Mac App 的 `openChatStream` 发同样的 `super-junior` 消息后，cc-switch DB 中没有
`super-relay-model-api-experimental-0630` 记录，只出现普通 Claude session：

```text
2026-07-07 04:06:45 UTC
app_type=claude
provider_id=_session
model=claude-opus-4-8
request_model=claude-opus-4-8
input_tokens=14585
output_tokens=4
status_code=200
```

这说明 Claude Code 子进程最终没有收到
`ANTHROPIC_BASE_URL=http://127.0.0.1:15721`，否则请求会进入 cc-switch proxy 并记录为
Super Relay provider。

补充：截图里较早的 `你好` 线程创建于 `2026-07-07 03:18 UTC`，而
`super-junior` agent env 更新时间是 `2026-07-07 03:52 UTC`。因此那条早期对话本身不能
证明新 env 失效；后续通过 Mac App live path 重新发送后，才确认当前 Mac path 仍未生效。

## 链路对比

CLI 和 Mac App 都进入 gateway chat run，但 payload 不同：

- CLI 使用 `/api/chat/ws` 的 `op=start`，不带桌面全局 `providerMetadata`。
- Mac App 使用 `/api/chat/start`，并带上 desktop 构造的 `providerMetadata`。

相关代码点：

- Desktop 发送入口：
  `desktop/garyx-desktop/src/main/gary-client/stream.ts` 的 `openChatStream`。
- Desktop provider metadata 构造：
  `buildProviderMetadata(settings)`。
- Gateway 合并 run metadata：
  `garyx-gateway/src/application/chat/prepare.rs` 的 `build_provider_run_metadata`。
- Claude Code 子进程 env 注入：
  `garyx-bridge/src/claude_provider.rs` 从 run metadata 的 `provider_env` 合并到子进程 env。
- Agent runtime snapshot 支持 `provider_env`：
  `garyx-models/src/agent_reference.rs` 的 `agent_runtime_snapshot_metadata` /
  `merge_thread_agent_runtime_snapshot`。

## 根因

直接根因是 Mac App 发送时带了 `providerMetadata.provider_env`，gateway
`build_provider_run_metadata` 用 `extend` 把整张 `providerMetadata` 合入 run
metadata，`provider_env` 键被 desktop 的 env 先占坑；随后 bridge
`backfill_bound_agent_runtime_metadata` 回填 agent snapshot 时是 existing-wins
（`or_insert`），发现 `provider_env` 已存在就跳过，agent 的
`ANTHROPIC_BASE_URL` 因此永远进不了 run metadata。

当前 Desktop metadata 构造有两个问题：

1. `buildProviderMetadata` 会从桌面全局 provider env 设置生成一张 `provider_env`。
2. 即使 Codex 不是 api-key 模式，也会写入一个空的 `CODEX_API_KEY` 字段。这样
   `Object.keys(providerEnv).length > 0` 恒成立，Mac App 很容易无意义地发出一张
   `provider_env`。

Gateway 当前合并方式是：

```rust
let mut run_metadata = build_chat_metadata(metadata, channel, account_id, from_id, run_id);
run_metadata.extend(provider_metadata);
```

`extend` 对顶层 key 是整键替换；`provider_env` 是一整张 map，所以 agent 里的
`ANTHROPIC_BASE_URL` 会被 Desktop 的 `providerMetadata.provider_env` 整张冲掉。

Claude provider 只会消费最终 run metadata 里的 `provider_env`：

```rust
let mut env = self.config.env.clone();
env.extend(task_cli_env(&options.metadata));
env.extend(metadata_string_map(&options.metadata, "provider_env"));
```

因此表现为：agent 线程里明明有 env，但 Claude Code 子进程没有拿到 proxy env。

## 为什么不是 Claude Code 配置问题

本次目标就是不改、不覆盖 Claude Code 全局配置。正确链路应当是：

```text
custom agent provider_env
  -> thread runtime snapshot
  -> run metadata provider_env
  -> garyx-bridge Claude provider
  -> Claude Code child process env
  -> ANTHROPIC_BASE_URL=http://127.0.0.1:15721
  -> cc-switch proxy
  -> Super Relay experimental_0630
```

CLI 已经证明这条链路可以成立。Mac App 失败说明问题发生在 Garyx 的 run metadata 组装层，
不是 cc-switch，也不是 Claude Code 全局配置。

另一个容易混淆的点：Finder 启动的 Mac App 不等于 shell 里的环境变量。但这里不应该依赖
Finder 进程 env；`super-junior.provider_env` 已经持久化在 agent/thread metadata 中，应由
Garyx runtime 显式传给 provider 子进程。

## 修复方案（下层统一）

### 设计原则

发消息 payload 在所有客户端同构：message + thread id（+附件），**不携带任何
provider env**。env 解析完全发生在服务端，单点在 bridge：

```text
provider config env (garyx.json providers.env, 全局 base)
  < agent/thread snapshot provider_env (bridge backfill)
  → provider 子进程 env
```

CLI 今天已经是这个模型（`/api/chat/ws` 的 `op=start` 零 env 处理），且工作正常；
Desktop 连接远程网关时也不发 env，同样正常。`providerMetadata` 是全仓唯一一条
客户端私有 env 通道（生产者只有 desktop `stream.ts`，消费点只有
`prepare.rs` 的 `extend`），历史包袱，整条删除。

### 1. Desktop 删除 `providerMetadata` 发送

- 删除 `buildProviderMetadata`，`openChatStream` 不再发送 `providerMetadata`。
- `CLAUDE_CODE_OAUTH_TOKEN` 不做任何特殊透传：它就是普通 env key，需要的人
  自己配进 agent 的 `provider_env`。随 `buildProviderMetadata` 一并删除。
- 桌面设置里的 `providerClaudeEnv` / `providerGeminiEnv` / `providerCodexApiKey`
  / `providerCodexAuthMode` 的唯一运行时效果就是 `buildProviderMetadata`，
  删除发送后即为死配置：同批删除这组设置项及 Provider 面板对应的 env 配置块，
  避免留下"看起来能配但无效"的 UI。
- 迁移说明：依赖桌面全局 env 的用户改为在 agent `provider_env`（`garyx agent
  update --env`）或 `garyx.json` 的 `providers.env` 配置。

### 2. Gateway 删除 `provider_metadata` 字段并封住 `metadata` 旁路

- `ChatRequest.provider_metadata` 及 `build_provider_run_metadata` 的
  `extend(provider_metadata)` 一并删除（desktop 与 gateway 同仓同发版本，
  按既定政策不做旧客户端兼容）。
- `ChatRequest.metadata` 是同一 bug 类的正门旁路：客户端在 `metadata` 里直接
  塞 `provider_env` 同样会占坑并被 bridge existing-wins 回填跳过。
  `prepare_chat_request` 入口按保留键表
  （`RESERVED_RUNTIME_METADATA_KEYS`，当前为 `provider_env`）strip 请求
  metadata；HTTP `/api/chat/start` 与 WS `start` 都汇入该入口，一处覆盖。
  内部 dispatch 直接构造 `AgentRunRequest` metadata，不经此边界，保留显式
  per-run 覆盖能力。
- run metadata 中的 `provider_env` 从此只有一个来源：thread 上的 agent
  runtime snapshot（bridge `backfill_bound_agent_runtime_metadata` →
  `merge_thread_agent_runtime_snapshot`）。

### 3. Bridge 回填保持单点 + 守卫测试

`backfill_bound_agent_runtime_metadata` 已是正确的单点合并入口（existing-wins，
`or_insert`）。不在 chat prepare 再加一层合并；补守卫测试钉住：

- thread metadata 存在 `provider_env.ANTHROPIC_BASE_URL` 时，
  `/api/chat/start` 与 `/api/chat/ws` 两条路径 dispatch 出的 run metadata
  都包含它。
- Claude provider 子进程 env 合并顺序：`config.env` < `task_cli_env` <
  `metadata.provider_env`。

### 4. 不改 Claude Code 全局配置

修复不写入 `~/.claude/settings.json`，不要求用户改 Claude Code 登录状态。只通过
Garyx run metadata 给 Claude Code 子进程注入本次 run 的 env。

## 测试与验收

### 单元测试

Gateway：

- `ChatRequest` 不再接受 `providerMetadata`（字段删除后未知字段被忽略，
  run metadata 中不出现请求侧 `provider_env`）。
- `build_provider_run_metadata` 产出不含任何请求侧 env 注入。

Desktop：

- `openChatStream` 请求体不含 `providerMetadata`。
- 设置契约中不再存在 `providerClaudeEnv` / `providerGeminiEnv` /
  `providerCodexApiKey` / `providerCodexAuthMode`。

Bridge（守卫）：

- thread metadata 存在 `provider_env.ANTHROPIC_BASE_URL` 时，backfill 后 run
  metadata 包含它；`/api/chat/start` 与 `/api/chat/ws` 两条路径行为一致。
- run metadata 显式带 `provider_env` 时 existing-wins 语义不变。

### 端到端验收

1. cc-switch proxy 监听：

   ```bash
   lsof -nP -iTCP:15721 -sTCP:LISTEN
   ```

2. `super-junior` agent 显示 proxy env：

   ```bash
   garyx agent get super-junior
   ```

3. CLI 发一条 `pong`，cc-switch DB 中出现：

   ```text
   provider_id=super-relay-model-api-experimental-0630
   model=model_api/experimental_0630
   input_tokens > 0
   output_tokens > 0
   status_code=200
   ```

4. Mac App 选择 `Super Junior` 发同样消息，cc-switch DB 中也出现同样 provider/model。

5. 不要求价格正确；本需求只验 token：

   ```text
   input_tokens
   output_tokens
   cache_read_tokens
   cache_creation_tokens
   ```

## cc-switch usage 统计口径

`experimental_0630` 应按实际 proxy log 的 `model` 聚合：

```text
model=model_api/experimental_0630
provider_id=super-relay-model-api-experimental-0630
request_model=claude-opus-4-8 或其他上游请求模型
```

展示时只需要 token：

```text
total_input_tokens = sum(input_tokens)
total_output_tokens = sum(output_tokens)
total_cache_read_tokens = sum(cache_read_tokens)
total_cache_creation_tokens = sum(cache_creation_tokens)
```

价格字段不参与本次需求。

## 风险与注意事项

- `provider_env` 里可能包含 token，thread logs / UI 展示应继续做 key 级隐藏或避免打印完整值。
- deep merge 只解决 map 覆盖问题，不改变 env 的安全模型。
- 老线程如果创建时没有 snapshot 到 `provider_env`，需要依赖当前 agent profile 回填或重新创建线程。
- iOS/Mobile 如果不发送 `providerMetadata.provider_env`，不会触发本次 Mac App 的覆盖问题；但仍应用同一套
  run metadata 合并测试覆盖，避免未来漂移。

## 建议落地顺序

1. Desktop 删 `buildProviderMetadata` / `providerMetadata` 发送与死配置
   （settings 契约、store、Provider 面板 env 块）。
2. Gateway 删 `ChatRequest.provider_metadata` 与 `extend(provider_metadata)`。
3. 补 bridge 回填守卫测试（两条 chat 路径 + existing-wins）。
4. 用 CLI 和 Mac App 各跑一次真实 `super-junior`，以 cc-switch DB 为最终验收。

## 已否决备选：gateway 对 `provider_env` deep merge

原方案是保留 `providerMetadata` 通道，gateway 合并时对 `provider_env` 按 key
deep merge（agent snapshot 为 base、请求侧为 overlay），desktop 侧只修
"非 api-key 模式也发空 `CODEX_API_KEY`"的触发器。

否决理由：deep merge 只是给一条本不该存在的客户端 env 通道打补丁。该通道
全仓只有 desktop 一个生产者，且远程网关场景 desktop 本来就不发（`isLocalGatewayUrl`
守卫），CLI/Mobile 从未使用。保留它意味着 env 有三个来源、两处合并语义，
以后仍可能出现"客户端 payload 挤掉 agent 配置"一类的错位。统一到服务端
解析后，这类问题结构性消失。
