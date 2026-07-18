# Claude Subagent 内部工具事件：主线程会话不可见化

Status: approved v3 (v2 passed design review #TASK-2419; v3 = owner scope narrowing, 2026-07-18)
Task: #TASK-2417 (reproduction), #TASK-2419 (design review), #TASK-2420 (implementation)
Date: 2026-07-18

## 问题

Claude Code 的 `Agent` 工具会派生 subagent。subagent 内部的工具调用事件（`tool_use` /
`tool_result`，stream-json 中带 `parent_tool_use_id` 指向顶层 `Agent` 调用）目前被
Claude bridge 采纳：发出 `StreamEvent`、加入 `session_messages`、落库为主线程
committed records，最终渲染成主线程的独立工具组行。

用户可见症状：主对话 transcript 里，除了 `Agent` 工具调用行本身，还出现
「已运行 N 条命令，使用了 M 个工具」的独立工具组行——那是 subagent 的内部活动。

确定性复现：#TASK-2417（无 UI 失败测试 + 真实记录脱敏 fixture）。采纳链路：

1. `claude-agent-sdk/src/parse.rs` 解析 `parent_tool_use_id`；
2. `garyx-bridge/src/claude_provider.rs` 只抑制 subagent 文本，工具事件照常提取、
   写入 `parent_tool_use_id` metadata、发 `StreamEvent`、进 `session_messages`
   （现有单测 `suppresses_subagent_text_but_keeps_tool_trace` 锁死了这个错误行为）；
3. `garyx-bridge/src/multi_provider/persistence.rs` 无条件把这些消息 append 为
   committed records。

历史数据（截至 2026-07-18）：4,010 个 transcript 中 214 个受影响、47,531 条已
committed 子事件。嵌套形状实测：顶层 `Agent` use/result 均无父字段；二级 `Agent`
47 个、更深层 237 条，全部满足 `parent_tool_use_id != 自身 id`。

## 行为定裁（owner 2026-07-18，两次）

1. subagent 本身 = 主线程的一个普通工具调用：`Agent` 的 `tool_use` + 最终
   `tool_result`（subagent 的最终回报）。**subagent 内部的工具事件压根不进入主
   线程会话**——不发流事件、不进会话消息、不落库。
2. **修复只在源头（采纳层）。渲染层不加特判；历史已落库的子事件保持原样，旧
   线程里的泄漏行可忍受**——不做迁移、不做回填、不做派生层过滤。

## Nested 判定规则

一条 SDK envelope 判定为 **nested**，当且仅当：

> `parent_tool_use_id` 经 trim 后非空，且 ≠ 该事件自身的 canonical
> `tool_call_id`（`tool_use_id`）。

- `tool_use` 与 `tool_result` **都直接按自身 envelope 的字段判定**。不跨消息
  回连、不维护「已抑制 child id 集合」之类的会话内状态——bridge 明确支持没有
  先看到 use 的 result 流（恢复 / 中途接入 / 孤儿 result），集合方案会漏杀；
  父字段在 SDK envelope 上自带（`claude-agent-sdk/src/parse.rs`）。
- `parent_tool_use_id == 自身 id`（self-parent）视为顶层，不抑制（现有 bridge
  单测已覆盖此形状的顶层 result）。
- 多层嵌套（subagent 再派 subagent）自然满足该规则：每层子事件的父字段指向其
  直接父调用，≠ 自身 id，全部判定为 nested（与上文 ledger 实测一致）。
- 判定实现放 bridge 采纳路径一处即可（无跨 crate 消费者）。

## 方案：仅采纳层（garyx-bridge claude provider）

**Scope 判定在消息采纳入口最先执行**（`Message::Assistant` / `Message::User`
处理的第一步），nested envelope 对主线程流状态**零副作用**：

- 不发 `StreamEvent`、不进 `session_messages`（因此不进 persistence snapshot、
  不落库——ledger 里根本不会出现）；
- 不改写 `response_text`、不产生 segment boundary、不清除 / 设置
  `assistant_text_in_flight`、不触碰任何 trailing 文本状态——nested envelope
  根本不进入主线程流状态机（现状：nested user result 会先清
  `assistant_text_in_flight`，nested assistant 会先产生 segment boundary，这些
  副作用一并消除）；
- nested envelope 仍可参与进程存活 / 健康判断（收到任何 envelope 都证明子进程
  活着），但仅此而已；
- 顶层 `Agent` 调用照常：`tool_use` 正常采纳；其 `tool_result`（subagent 最终
  回报）正常采纳；
- 删除并翻转 `suppresses_subagent_text_but_keeps_tool_trace` 单测——旧行为
  （保留 tool trace）裁定为错误行为，直接改，不留开关、不留兼容路径；
- 消息 metadata 不再写 `parent_tool_use_id`（没有嵌套消息会被保留）。SDK 解析层
  （`claude-agent-sdk`）保留该字段解析——它是判定输入。

### 明确不做（owner 裁决）

- `garyx-models` render / run-state reducers：**零改动**。不加 nested 过滤。
- `garyx-channels` committed replay：**零改动**。
- gateway SSE：零改动（本来也不该动，ledger/cursor 契约）。
- 历史已落库的 47k 条子事件：不删、不迁移、不过滤。旧线程继续按现状渲染，
  可忍受；新 run 从源头即干净。

## 影响面

- 仅 `garyx-bridge` claude provider 采纳链（入口判定 → stream 输出 /
  session_messages → persistence 输入）。
- 服务端派生层、渠道、desktop / mobile 客户端全部零改动。
- 新 run 的 ledger / SSE 体积下降；主线程会话中 subagent 呈现为一次普通工具
  调用（`Agent` use + 最终 result）。
- 可观测性取舍：subagent 内部工具活动不再存在于 Garyx 主线程 ledger（Claude
  Code 自身 session 文件仍有）。裁定为接受，不加配置开关。

## 验证（全部在 garyx-bridge / SDK 层）

- 判定单测：顶层（无父字段）、nested（父≠自身）、self-parent 顶层 result、
  两级嵌套、空串 / 空白父字段（trim 后为空 = 顶层）。
- 精确事件序列测试：「顶层 assistant 文本 → 后台 nested use / nested result
  交错 → 顶层 Result」——断言主线程 `response_text` / segment boundary /
  `assistant_text_in_flight` 全程不受 nested 影响，stream 输出与
  `session_messages` 无 nested 事件，顶层 `Agent` use/result 正常保留。
- 孤儿 nested result（无前置 use）被抑制；self-parent 顶层 result 被保留。
- provider → persistence 集成断言：最终 reconcile 不把 child 带回；ledger 与
  `committed_message` 的 seq 连续。
- #TASK-2417 的 render 层复现测试不再是验收标准（渲染层行为不改），不合入；
  其脱敏 fixture 形状可改造为 bridge 层测试输入。
- `cargo test -p garyx-bridge --all-targets` 全绿；`scripts/test/rust_tier1_fast.sh
  --changed` 起步。

## 评审记录

- Round 1（#TASK-2419）：FAIL，4 findings——采纳层未阻断 nested 对主线程流状态
  的副作用；「已抑制 child-id 集合」方案漏杀孤儿 result；影响面漏 channels
  committed replay；验证清单不足。v2 全部吸收。
- Round 2（#TASK-2419）：v2 PASS / SHIP。
- v3（owner 裁决）：scope 收窄——去掉派生层（reducers）与渠道 replay 过滤，
  历史数据可忍受；只保留采纳层修复。采纳层方案与判定规则与已 PASS 的 v2 逐字
  一致，未引入新设计面。
