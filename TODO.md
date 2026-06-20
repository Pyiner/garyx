# TODO

> 临时兼容债清单：这里记录的是为了当前版本兼容而保留、但不应该长期存在的设计债。清掉对应债务后，需要同步删除相关兼容代码。

1. 删除旧 subprocess plugin 的 `dispatch_outbound` 兼容 adapter。
   当前内置 channel 和新 subprocess plugin 已统一到 `StreamDispatchEnvelope` /
   `dispatch_stream_event` outbound 契约；旧 subprocess plugin 仍可能只声明
   `dispatch_outbound` 能力，所以 `garyx-channels` 暂时保留
   `StreamEvent` -> `ChannelOutboundContent` -> `dispatch_outbound` 的 adapter。
   这个 adapter 只是兼容旧插件的边界妥协，不是长期主路径。等所有受支持插件都声明
   `capabilities.dispatch_stream_event = true` 后，应删除该 fallback adapter 和对应测试。
