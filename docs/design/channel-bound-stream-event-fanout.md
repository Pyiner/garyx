# 绑定 Channel 的 StreamEvent 扇出设计

## 状态

提案中。

本文记录目标设计：一个线程运行产生的 committed `StreamEvent`，应该分发给这个线程绑定的所有 channel 端点。当前为了兼容旧 subprocess plugin 保留的临时债务记录在 `TODO.md`。

可视化审阅版见同目录的 `channel-bound-stream-event-fanout.html`。

## 核心判断

Gateway/router 只负责回答两个问题：

- 哪个线程产生了 committed `StreamEvent`。
- 这些事件应该投递给这个线程绑定的哪些端点。

Gateway/router 不负责把事件渲染成 channel 消息。Markdown、图片、工具调用、发送失败语义、平台特殊规则，都属于 `garyx-channels`。

目标边界是：

```text
committed StreamEvent
  -> 按 thread_id 查绑定端点
  -> garyx-channels dispatcher dispatch_stream_event
  -> channel 自己渲染和发送
```

## 问题

Garyx 允许一个线程绑定到多个 channel 端点，例如 Feishu、Telegram、Discord、Weixin 或 subprocess plugin channel。用户期望是：线程只要绑定了端点，无论这次 run 从哪里触发，输出都能发给所有绑定端点。

当前实现还不够干净：

- 一些 gateway 入口会挂 bound-response callback。
- 一些内置 channel inbound 路径会自己订阅 committed replay，并用 channel 自己的路径发送。
- legacy subprocess plugin 只认识 `dispatch_outbound`，所以存在 `StreamEvent` 到旧 outbound 消息的 fallback。

这些分叉会让投递行为依赖 run 的来源，也会让 gateway 开始承担 channel 渲染职责。例如工具调用、边界事件、图片、Markdown、Telegram 特殊错误，都不应该在 gateway 做语义判断。

## 设计目标

- 绑定投递与 run 来源无关。API chat、内部/plugin 入口、内置 channel inbound、workflow、cron、automation、tool-triggered run、restart recovery 都走同一个 fanout 入口。
- Gateway/router 只做 event fanout，不做 channel 渲染。
- `garyx-channels` 暴露统一的 `dispatch_stream_event` 契约。
- 内置 channel 原生实现 `dispatch_stream_event`。
- 新 subprocess plugin 可以实现 host -> plugin 的 `dispatch_stream_event` RPC。
- 旧 subprocess plugin 由 channels 层 adapter 转成旧 `dispatch_outbound`。
- adapter 是兼容降级，不是主路径。
- 单个目标的事件顺序、边界、工具调用结构、投递结果都必须可观测。

## 非目标

- 不保留 gateway 里的 Markdown 解析、图片 strip、Telegram/Feishu/Discord/Weixin 特判。
- 不在本文完整定义 subprocess plugin 新协议。
- 不要求旧 plugin 在一个版本内全部升级成 stream-event native。
- 不改变单个 channel 如何渲染 Markdown、图片、工具调用或 ack。

## 目标模型

引入一个共享 fanout 服务，例如 `BoundStreamFanout` 或 `BoundEventFanout`。

它在 run 拥有稳定 `thread_id` 和 `run_id` 后挂载。挂载时 snapshot 当前线程绑定端点，并为每个目标创建 stream-event callback。

snapshot 是有意的产品语义：

- run 开始后用户修改 bot/channel 绑定，不影响当前 run。
- streaming input 追加到同一个 run 时，目标集合也不变。
- 新绑定从下一次 run 开始生效。

所有 run 来源都必须调用同一个挂载点：

- API chat。
- 内部/plugin 创建的 run。
- 内置 channel inbound handler。
- subprocess plugin inbound handler。
- workflow、cron、dream、automation、scheduled run。
- tool-image 或其它 tool-triggered assistant run。
- gateway restart 后需要恢复 callback 的 in-flight run。

共享 fanout 应该是 bound channel delivery 的唯一 committed-replay 挂载点。channel inbound handler 只负责启动或追加线程 run，然后依赖共享 fanout；不要再自己挂一套 origin endpoint 的 committed-replay callback。

## 端点身份和去重

fanout target 必须携带足够的信息，让 channel 能定位自己的原生目的地，同时不要求 gateway 理解 channel 规则。

目标字段应包括：

- channel kind 或 `channel_id`。
- `account_id`。
- Garyx `thread_id`。
- target endpoint key。
- 原生目的地 scope，例如 chat、topic、thread、conversation。
- channel delivery target type/id。
- channel 原生 chat/conversation id。
- `run_id`。

必须存在一把 canonical endpoint identity，所有去重都从这把身份派生。绑定去重和 origin callback 排除必须使用同一套身份。

关键约束：

- identity 必须包含能区分原生目的地的 scope，不能只用 `chat_id` 或 `delivery_target_id`。
- 同一个 chat 下的不同 topic/thread 不能被合并。
- origin target 没有语义特权。迁移期如果还存在 direct callback，只有完整 endpoint identity 一致时才能排除重复。
- 不能用更粗的 `delivery_target_id` 比较，把同 chat 的另一个 topic 错误排除。
- hidden child thread 和 workflow child thread 不能隐式继承父线程 channel 绑定；fanout 只看子线程自己的 persisted bindings。

## Dispatcher 契约

`garyx-channels` 应暴露 dispatcher-level stream event API。示意：

```rust
fn build_stream_event_callback(
    &self,
    target: StreamDispatchTarget,
) -> Result<StreamEventCallback, StreamEventCallbackError>;
```

callback 接收原始 `StreamEvent` 和投递 metadata。必须保留：

- 单个目标内的事件顺序。
- segment flush、`Done` 等边界。
- 结构化 `ToolUse` / `ToolResult`。
- `SessionBound`、`ThreadTitleUpdated` 这类不一定渲染的事件；可以显式忽略，但必须是有意忽略并可观测。
- channel-specific ack 行为。

`UserAck` 是 origin-sensitive 事件。它确认的是某个端点排队输入已被消费，不代表所有绑定端点都有 ack。fanout 契约必须二选一：

- 只把 `UserAck` 发给 origin endpoint。
- 或者携带 originating endpoint identity，让非 origin channel 自己忽略，避免无意义地切分输出气泡。

`seq` 应来自 committed per-run replay sequence，而不是临时 per-target counter。目标可以用 `(target_endpoint_key, run_id, seq)` 作为 replay gap recovery 和 gateway restart reattach 之后的幂等 key。

## 内置 Channel

内置 channel 应原生实现 `dispatch_stream_event`。

它们可以复用现有 channel-native stream consumer，但 dispatcher 必须成为唯一入口。Gateway 不应该知道 Telegram、Feishu、Discord、Weixin 的渲染差异。

结构化工具事件必须保持结构化直到 channel renderer。Feishu 可以按自己的方式渲染 tool call，Telegram 可以选择另一套呈现，这些差异都属于 channel 实现。

## Legacy Subprocess Plugin Adapter

旧 subprocess plugin 只认识 `dispatch_outbound`，所以需要兼容 adapter。

adapter 位于 `garyx-channels`，接收和其它目标一样的 `StreamEvent` callback，只转换旧协议能表达的内容。只有在 capability detection 判断 plugin 不支持原生 `dispatch_stream_event` 后，才使用 adapter。

adapter 规则：

- 文本 delta 和 assistant segment 可以聚合成 outbound text。
- segment boundary 和 `Done` flush 文本。
- 每个 target 使用单 worker queue，保证顺序。
- `ToolUse` / `ToolResult` 不能静默 drop 后返回成功。
- 旧 plugin 没声明结构化能力时，adapter 不能把结构化 `ToolUse` / `ToolResult` 直接转发成旧协议好像它能正常渲染。
- 对结构化事件，adapter 要么转成显式可见 fallback，要么报告 unsupported，要么走声明过的结构化能力。
- `UserAck` 是否作为 boundary 必须明确记录；只有旧协议确实需要时才能这么做。

这个 adapter 是 `TODO.md` 里的兼容债。等 subprocess plugin 都支持 `dispatch_stream_event` 后应删除。

## 投递结果和可观测性

Gateway 应保持 channel-agnostic，但投递结果必须可观测。

channel 层应产出 per-target delivery outcome。形式可以是 callback result、delivery outcome event 或 `garyx-channels` 拥有的 diagnostics sink。

delivery outcome 至少应包含：

- target identity。
- `run_id`。
- event `seq`。
- outbound message id。
- 成功、失败、unsupported、deliberately ignored 等状态。

router 当前依赖发送结果保存 outbound message id，这条路径不能在迁移中丢失。router persistence point 可以不懂 channel 渲染，但必须通过 typed outcome 收到必要信息。

## 上线策略

这个改动会扩大投递覆盖面：以前 channel-originated 或 scheduled run 可能只回原入口，现在会发给所有绑定端点。因此需要分阶段上线。

- 新 fanout 先放在 runtime flag 或 origin allowlist 后面。
- enable sends 前先 shadow compare target plan。
- 先启用内置 channel，再启用 legacy subprocess plugin adapter。
- 保留 backout switch，可以让某个 run origin 回到旧 callback 路径，直到投递 outcome 指标稳定。

## 迁移计划

1. 补 exact endpoint identity 测试，覆盖同 chat 不同 native topic/thread scope。
2. 同时覆盖 binding de-duplication 和 origin callback exclusion。
3. 在 `garyx-channels` 增加 dispatcher-level `dispatch_stream_event` 契约。
4. 内置 channel 通过该契约实现 native stream-event dispatch。
5. 增加 delivery outcome reporting，包含 router outbound id persistence。
6. 增加 subprocess plugin capability detection。
7. 增加 legacy subprocess adapter，把 `StreamEvent` 降级成 `dispatch_outbound`。
8. 所有 run origin 切到共享 fanout，包括 channel-originated、scheduled、tool-triggered、restart recovery。
9. 删除 gateway-side message rendering、Markdown parsing、image stripping 和 channel-specific branching。
10. 所有受支持 plugin 都 stream-event native 后，删除 `TODO.md` 里的兼容债。

## 验收标准

- 一个绑定线程无论从 API chat、内部/plugin、内置 channel inbound、scheduled/tool-triggered/restart recovery 哪条路径触发，都能把 committed events 发给所有绑定端点。
- Gateway bound delivery 不包含 channel-specific Markdown、image、tool、Telegram/Feishu/Discord/Weixin 渲染逻辑。
- 内置 channel 收到结构化 `StreamEvent`，并通过自己的 presentation path 渲染 `ToolUse` / `ToolResult`。
- legacy subprocess plugin 通过明确 adapter 继续收到旧 `dispatch_outbound`，并且 lossy 行为被文档化。
- unsupported structured event 可观测，不会被报告成 silent success。
- 同 chat 不同 topic/thread 不会在 binding 去重时被合并。
- 同 chat 不同 topic/thread 不会被 origin duplicate prevention 错误排除。
- 单 target 事件顺序保持稳定。
- gateway restart 后 in-flight run 可以重挂 fanout，不改变 snapshot target set，也不重复投递。
- scheduled/tool-triggered run 不再通过 gateway text-only callback 静默丢弃结构化事件。

## 测试计划

- exact target identity 和 de-duplication key 单元测试。
- origin exclusion 使用 exact target identity 的单元测试。
- orchestration 测试：每类 run entry path 都挂共享 fanout。
- restart recovery 测试：reattach 后幂等，不重复投递。
- channel 测试：内置 channel stream callback 收到结构化 tool event，而不是 fallback text。
- legacy plugin adapter 测试：顺序、flush、unsupported structured event reporting。
- router outbound id persistence 和 delivery target lookup 回归测试。

## 待决问题

- `dispatch_stream_event` 应该 fire-and-forget 后异步上报 diagnostics，还是每个 event 都要求 per-target ack？
- legacy plugin 对 `ToolUse` / `ToolResult` 的最低可接受可见 fallback 是什么？
- 共享 fanout 服务应该落在 gateway runtime assembly、router run orchestration，还是 bridge/gateway glue code？判断标准是：哪个位置能同时拿到 `thread_id`、`run_id`、committed replay 和 bound endpoint lookup，并且不需要理解 channel 渲染。
