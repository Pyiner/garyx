# 绑定 Channel 的 StreamEvent 扇出设计

## 状态

已实现主路径。

本文记录当前实现：一个线程运行产生的 committed `StreamEvent`，会按线程绑定关系分发给绑定的 channel 端点。内置 channel 和新 subprocess plugin 统一到同一个 `StreamDispatchEnvelope` / `dispatch_stream_event` outbound 契约；旧 subprocess plugin 的 `dispatch_outbound(ChannelOutboundContent)` 只作为兼容 adapter 保留，临时债务记录在 `TODO.md`。

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

改动前实现不够干净：

- 一些 gateway 入口会挂 bound-response callback。
- 一些内置 channel inbound 路径会自己订阅 committed replay，并用 channel 自己的路径发送。
- 旧 subprocess plugin 只认识 `dispatch_outbound`，所以需要 `StreamEvent` 到旧 outbound 协议的兼容 adapter。

这些分叉会让投递行为依赖 run 的来源，也会让 gateway 开始承担 channel 渲染职责。例如工具调用、边界事件、图片、Markdown、Telegram 特殊错误，都不应该在 gateway 做语义判断。

## 设计目标

- 绑定投递与 run 来源无关。API chat、内部/plugin 入口、内置 channel inbound、subprocess plugin inbound 走同一个 fanout 语义；其它入口复用相同挂载链路时不需要理解 channel 渲染。
- Gateway/router 只做 event fanout，不做 channel 渲染。
- `garyx-channels` 暴露统一的 `dispatch_stream_event` 契约。
- 内置 channel 原生实现 `dispatch_stream_event`。
- 新 subprocess plugin 也使用同一套 `StreamEvent` 语义，通过 host -> plugin 的 `dispatch_stream_event` RPC 接收事件。
- plugin 注册或初始化时声明是否支持 `dispatch_stream_event`，host 用这个 capability 区分新旧协议。
- 旧 subprocess plugin 由 channels 层 adapter 转成旧 `dispatch_outbound(ChannelOutboundContent)`。
- adapter 是兼容降级，不是主路径。
- 单个目标的事件顺序、边界、工具调用结构、投递结果都必须可观测。

## 非目标

- 不保留 gateway 里的 Markdown 解析、图片 strip、Telegram/Feishu/Discord/Weixin 特判。
- 不在本文完整定义 subprocess plugin JSON-RPC schema、错误码和版本协商细节；这里只定义语义边界、能力位和新旧选择规则。
- 不要求旧 plugin 在一个版本内全部升级成 stream-event native。
- 不改变单个 channel 如何渲染 Markdown、图片、工具调用或 ack。

## 当前实现模型

实现拆成两类挂载点，但目标分发逻辑都进入同一个 dispatcher-level stream-event 契约。

- gateway API chat / 内部入口：`garyx-gateway/src/application/chat/delivery.rs` 负责在已有稳定 `thread_id` / `run_id` 后 snapshot 绑定端点，并为每个目标调用 `build_stream_dispatch_callback`。这里的“内部入口”只是 run 的来源，不是另一套 outbound 协议。
- 内置 channel inbound 和外部 subprocess plugin inbound：入口启动 run 时还不知道 canonical `thread_id`，所以先用 `DeferredBoundStreamFanout` 缓冲 committed event；router 解析出 canonical `thread_id` 后，通过 `DeferredFanoutAgentDispatcher` 在真正 provider dispatch 前 snapshot 绑定端点、排除 origin endpoint，并 flush 缓冲事件。`route_and_dispatch` 返回后的 `attach_thread` 只是幂等兜底和 local-command 路径收尾。

fanout 在 run 拥有稳定 `thread_id` 和 `run_id` 后挂载。挂载时 snapshot 当前线程绑定端点，并为每个目标创建 stream-event callback。

snapshot 是有意的产品语义：

- run 开始后用户修改 bot/channel 绑定，不影响当前 run。
- streaming input 追加到同一个 run 时，目标集合也不变。
- 新绑定从下一次 run 开始生效。

当前已接入的 run 来源：

- API chat。
- 内部/plugin 创建的 run。
- 内置 channel inbound handler。
- subprocess plugin inbound handler。

其它 run 来源只要复用同一条 gateway delivery 或 deferred fanout 挂载链路，就不需要理解 channel 渲染。

channel inbound handler 只负责启动或追加线程 run，并把 origin callback 交给 `DeferredBoundStreamFanout` 包装；非 origin 绑定目标统一由 fanout snapshot 后分发。

因此内置 channel、新 subprocess plugin、内部入口和外部入口在 outbound 分发语义上是一致的：只要线程绑定了 endpoint，committed `StreamEvent` 就按同一个 dispatcher-level 契约发给 channel。唯一例外是旧 subprocess plugin，它不支持新 RPC 时才走兼容 adapter。

## Endpoint Identity 和去重

fanout 去重只认一个值：`Endpoint Identity`。

`Endpoint Identity` 是 channel 层产出的 opaque unique key。Gateway/router 不关心它怎么生成、不关心里面有什么字段、不解析任何 channel 内部字段，也不自己拼接 identity。

Gateway/router 只使用这个 key 做三件事：

- binding de-duplication。
- origin callback exclusion。
- delivery outcome / idempotency 的 target 关联。

Channel 层负责保证 `Endpoint Identity` 的语义正确：

- 同一个真实投递目的地必须生成同一个 identity。
- 不同真实投递目的地必须生成不同 identity。
- identity 在 run snapshot、restart reattach、delivery outcome 回传中保持稳定。

origin target 没有语义特权。迁移期如果还存在 direct callback，只有 `Endpoint Identity` 相等时才能排除重复。Gateway/router 不能退回去比较任何 channel 内部字段。

hidden child thread 不能隐式继承父线程 channel 绑定；fanout 只看子线程自己的 persisted bindings。

## Dispatcher 契约

`garyx-channels` 现在暴露 dispatcher-level stream event API：

```rust
fn build_stream_event_callback(
    &self,
    target: StreamingDispatchTarget,
    router: Arc<Mutex<MessageRouter>>,
) -> Option<StreamDispatchCallback>;
```

主路径 callback 接收 `StreamDispatchEnvelope`。这个 envelope 是内置 channel 和新 subprocess plugin 的统一 outbound 协议模型：

```rust
pub struct StreamDispatchEnvelope {
    pub account_id: String,
    pub chat_id: String,
    pub delivery_target_type: String,
    pub delivery_target_id: String,
    pub endpoint_identity: String,
    pub thread_id: String,
    pub run_id: String,
    pub event: StreamEvent,
    pub delivery_thread_id: Option<String>,
}
```

新 subprocess plugin 的 JSON-RPC `DispatchStreamEvent` 由这个 envelope 直接转换出来，避免内部、外部两套字段模型漂移。

callback 必须保留：

- 单个目标内的事件顺序。
- segment flush、`Done` 等边界。
- 结构化 `ToolUse` / `ToolResult`。
- `SessionBound`、`ThreadTitleUpdated` 这类不一定渲染的事件；可以显式忽略，但必须是有意忽略并可观测。
- channel-specific ack 行为。

`UserAck` 是 origin-sensitive 事件。它确认的是某个端点排队输入已被消费，不代表所有绑定端点都有 ack。fanout 契约必须二选一：

- 只把 `UserAck` 发给 origin endpoint。
- 或者携带 originating `Endpoint Identity`，让非 origin channel 自己忽略，避免无意义地切分输出气泡。

当前主路径不伪造 per-target sequence。后续如果要做 restart gap recovery 或 per-event diagnostics，`seq` 必须来自 committed per-run replay sequence，而不能来自临时 per-target counter；目标才可以用 `(Endpoint Identity, run_id, seq)` 做幂等和恢复。

## 内置 Channel

内置 channel 原生实现 dispatcher-level stream-event callback。

- Telegram 复用原有 `build_bound_response_callback`。
- Discord 复用 `build_discord_response_callback`。
- Feishu 把原来 WS handler 内的 stream worker 抽成 `build_feishu_response_callback`，dispatcher 和 WS handler 共用。
- Weixin 复用 `run_streaming_update_consumer`，通过 `build_weixin_response_callback` 接入 dispatcher。

Gateway 不知道 Telegram、Feishu、Discord、Weixin 的渲染差异。

结构化工具事件必须保持结构化直到 channel renderer。Feishu 可以按自己的方式渲染 tool call，Telegram 可以选择另一套呈现，这些差异都属于 channel 实现。

Feishu 有一个实际呈现边界：非 origin 绑定目标没有可回复的原始 Feishu message id，所以不能把 COT thread 关联到用户那条消息上；它仍然收到完整 `StreamEvent`，但 channel renderer 会在没有 origin message 时退化为普通卡片/图片发送。这是 Feishu channel 自己的呈现选择，不是 gateway 降级。

## 外部 Plugin 协议

新外部 plugin 和内置 channel 使用同一套 outbound 事件语义：目标收到的是同一个 `StreamDispatchEnvelope` 所表达的 `StreamEvent` 和投递 metadata，不是 gateway 渲染后的文本。

差异只在传输形态：

- 内置 channel：Rust callback / trait 实现。
- 新 subprocess plugin：host -> plugin 的 JSON-RPC，例如 `dispatch_stream_event`。

因此协议选择应基于注册或初始化能力，而不是 run 来源、channel 名字或旧的 streaming 判断。建议单独引入 capability：

```text
capabilities.dispatch_stream_event = true
```

这个 capability 的含义很窄：plugin 能接收 host 发来的 outbound `StreamEvent` fanout。它不等同于 plugin inbound 侧是否支持 streaming input，也不等同于旧的 `stream_frame` / `stream_end` 能力。

host 选择规则：

```text
if plugin.capabilities.dispatch_stream_event {
  send dispatch_stream_event(StreamDispatchEnvelope)
} else {
  use legacy adapter -> dispatch_outbound(ChannelOutboundContent)
}
```

`Endpoint Identity` 仍然是 channel/plugin 层产出的 opaque key。plugin 可以决定自己的 identity 如何生成和保存；gateway/router 只拿这个 key 做 equality。

## Legacy Subprocess Plugin Adapter

旧 subprocess plugin 只认识 `dispatch_outbound`，所以需要兼容 adapter。

adapter 位于 `garyx-channels`，接收和其它目标一样的 `StreamEvent`，然后降级成旧插件已经认识的 `dispatch_outbound(ChannelOutboundContent)`。只有在 capability detection 判断 plugin 不支持原生 `dispatch_stream_event` 后，才使用 adapter。

旧 `dispatch_outbound` 不是纯文本协议；它的 `ChannelOutboundContent` 已经能表达 `Text`、`ToolUse`、`ToolResult` 等结构化内容。因此 adapter 的目标不是把工具调用转成 fallback 文本，而是在旧协议可表达的范围内尽量保持结构化。

adapter 规则：

- 文本 delta 和 assistant segment 可以聚合成 outbound text。
- segment boundary 和 `Done` flush 文本。
- 每个 target 使用单 worker queue，保证顺序。
- `ToolUse` / `ToolResult` 通过 `ChannelOutboundContent::ToolUse` / `ChannelOutboundContent::ToolResult` 透传给旧 `dispatch_outbound`。
- 旧协议确实无法表达的 `StreamEvent` variant 不能静默 drop 后返回成功；必须记录为 deliberately ignored 或 unsupported，并进入 delivery outcome。
- `Done` 负责 flush 聚合文本；旧 plugin 之前识别 final message 的路径继续来自最终 outbound text。
- `UserAck` 是 origin-sensitive 事件。非 origin target 不应因为 adapter 收到 ack 就切分输出；origin target 只有旧协议确实需要时才可把它当 boundary。

这个 adapter 是 `TODO.md` 里的兼容债。等 subprocess plugin 都支持 `dispatch_stream_event` 后应删除。

## 投递结果和可观测性

Gateway 应保持 channel-agnostic，但投递结果必须可观测。

channel 层应产出 per-target delivery outcome。形式可以是 callback result、delivery outcome event 或 `garyx-channels` 拥有的 diagnostics sink。

未来 typed delivery outcome 至少应包含：

- `Endpoint Identity`。
- `run_id`。
- 可选 committed event sequence，用于 replay gap recovery；当前主路径尚未引入。
- outbound message id。
- 成功、失败、unsupported、deliberately ignored 等状态。

router 当前依赖发送结果保存 outbound message id，这条路径不能在迁移中丢失。router persistence point 可以不懂 channel 渲染；后续补 typed outcome 时，只接收必要的投递事实，不回到 gateway 解析 channel 内容。

## 当前代码落点

- 统一 envelope 和 dispatcher helper：`garyx-channels/src/dispatcher.rs`。
- 延迟 fanout 和 provider-dispatch 前挂载 wrapper：`garyx-channels/src/bound_fanout.rs`。
- 外部 plugin `dispatch_stream_event` RPC：`garyx-channels/src/plugin_host/protocol.rs`、`sender.rs`。
- 内置 channel discovery 注入同一个 dispatcher：`garyx-channels/src/plugin.rs`、`garyx-gateway/src/composition/app_bootstrap.rs`。
- API/gateway bound delivery：`garyx-gateway/src/application/chat/delivery.rs`。
- 内部入口 origin identity：`garyx-gateway/src/internal_inbound.rs`。
- 外部 plugin inbound deferred fanout：`garyx/src/channel_plugin_host.rs`。
- 内置 channel inbound deferred fanout：Telegram、Discord、Feishu、Weixin 各自 handler。

## 剩余兼容债

- 旧 subprocess plugin adapter 仍保留；删除条件见 `TODO.md`。
- 当前投递可观测性仍以 outbound message id persistence 和日志为主，没有引入独立的 per-event delivery outcome event。这个不影响 fanout 主路径，但后续若要做 restart gap recovery / per-target diagnostics，应补 typed outcome。

## 本次验收标准

- 一个绑定线程从 API chat、内部/plugin、内置 channel inbound、subprocess plugin inbound 触发时，都能把 committed events 发给绑定端点。
- Gateway bound delivery 不包含 channel-specific Markdown、image、tool、Telegram/Feishu/Discord/Weixin 渲染逻辑。
- 内置 channel 收到结构化 `StreamEvent`，并通过自己的 presentation path 渲染 `ToolUse` / `ToolResult`。
- 新 subprocess plugin 通过 `dispatch_stream_event` 收到和内置 channel 同语义的 `StreamEvent`。
- legacy subprocess plugin 通过明确 adapter 继续收到旧 `dispatch_outbound(ChannelOutboundContent)`，其中 `ToolUse` / `ToolResult` 保持结构化透传。
- unsupported 或 deliberately ignored event 可观测，不会被报告成 silent success。
- Gateway/router 不解析 channel 内部字段，只按 `Endpoint Identity` equality 做 binding 去重。
- Gateway/router 不解析 channel 内部字段，只按 `Endpoint Identity` equality 做 origin duplicate prevention。
- 单 target 事件顺序保持稳定。
- 运行中修改绑定不影响当前 run，下一次 run 生效。

## 已覆盖测试

- gateway/router 只使用 opaque `Endpoint Identity` 去重的单元测试。
- origin exclusion 只使用 opaque `Endpoint Identity` 的单元测试。
- deferred fanout 缓冲 committed event，attach canonical thread 后 flush，并按 origin `Endpoint Identity` 排除重复。
- deferred fanout 通过 `DeferredFanoutAgentDispatcher` 在 provider dispatch 前 snapshot，运行中绑定变更不会进入当前 run。
- channel 测试：内置 Feishu、Discord、Weixin 都能构造 native stream-event callback，而不是落到 adapter。
- envelope 测试：`StreamDispatchEnvelope` 转成 plugin `DispatchStreamEvent` 时字段不漂移。
- plugin capability selection 测试：支持 `dispatch_stream_event` 的 plugin 走 native RPC，不支持的 plugin 才走 legacy adapter。
- router outbound id persistence 和 delivery target lookup 回归测试。

## 后续增强测试

- restart recovery 测试：reattach 后幂等，不重复投递。
- per-event delivery outcome 测试：失败、unsupported、deliberately ignored 都能形成 typed diagnostics。
- scheduled/tool-triggered run 的端到端 fanout 覆盖。

## 已拍板并落地的决策

- 内置 channel 和新外部 plugin 对齐同一套 `StreamEvent` outbound 语义。
- 新旧 plugin 通过 `dispatch_stream_event` capability 识别。
- 旧 plugin 走 adapter，adapter 输出旧 `dispatch_outbound(ChannelOutboundContent)`；`ToolUse` / `ToolResult` 直接用旧协议已有的结构化内容类型透传。
- API/gateway 入口在 gateway delivery 挂载；channel/plugin inbound 入口用 `DeferredBoundStreamFanout` 等 canonical `thread_id` 后挂载。bridge 不理解 channel 渲染。

## 后续增强

- 独立 per-event delivery outcome：当前主路径保留 outbound message id persistence 和日志；后续可以加 typed diagnostics sink，覆盖失败、unsupported、deliberately ignored。
