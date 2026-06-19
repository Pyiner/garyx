# TODO

> 临时兼容债清单：这里记录的是为了当前版本兼容而保留、但不应该长期存在的设计债。清掉对应债务后，需要同步删除相关兼容代码。

1. 统一内置 channel 和 subprocess plugin 的 outbound stream-event 契约。
   现在 dispatcher 只有部分内置 channel 暴露原生 `StreamEvent` consumer，外部
   subprocess plugin 也只暴露 `dispatch_outbound`，所以 `garyx-channels` 需要临时把
   committed `StreamEvent` 适配成 outbound message。这个适配是边界上的兼容妥协，
   不是最终设计。后续应补齐所有内置 channel 的 dispatcher-level stream callback，
   并扩展 plugin-host 协议，增加 host -> plugin 的 stream-event RPC，让内置 channel
   和 subprocess plugin 都走同一个 event consumer 契约；完成后删除
   `StreamEvent` -> `dispatch_outbound` 的 fallback adapter。
