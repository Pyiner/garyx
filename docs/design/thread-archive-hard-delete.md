# 线程归档硬删除设计

## 问题

现在的产品语义已经明确：归档就是把线程从 Garyx 存储里删除，并保证它不能再回到 `recent_threads`。

当前 mobile 和 desktop 的归档流程都是先乐观移除本地行，再只 detach 本地已知 endpoint，最后调用
`DELETE /api/threads/:id`。server 端的 delete route 会拒绝仍带有 enabled plugin
channel binding 的线程。如果客户端没有枚举到 thread record 里所有持久化 binding，归档会返回
`409 CONFLICT`，线程记录还在，写时维护的 `recent_threads` 投影也还在，下一次刷新就可能重新出现。

## 复现

已加入 RED 测试：

`garyx-gateway/src/routes/tests.rs::archive_thread_detaches_live_channel_binding_and_prevents_recent_revival`

测试种下：

- enabled `telegram:main` plugin 账号；
- 线程里的 `channel_bindings[]`，使用合成用户 `1000000001`；
- `recent_threads`、`thread_meta`、pin 投影；
- 归档后的同 endpoint 重连模拟：
  `MessageRouter::resolve_or_create_inbound_thread`。

当前失败结果：

```text
cargo test -p garyx-gateway archive_thread_detaches_live_channel_binding_and_prevents_recent_revival -- --nocapture
assertion failed: left == right
left: 404
right: 200
```

失败点现在是 `POST /api/threads/:key/archive` 尚未注册；这保证 RED 测试和设计里的修复面一致。不要为了让测试转绿而改变
`DELETE /api/threads/:key` 的 enabled-binding 保护。

重要的反证：此前用 `DELETE /api/threads/:id` 复现时，如果把 channel 改成 `api`，当前代码会通过，因为 enabled-binding 守卫检查的是
`config.channels.plugins[channel]`，不是 `config.channels.api.accounts`。如果现场三条线程确实是 API/WS
线程，那么 409 不是那三条的根因；还必须防住旧客户端重连后显式复用旧 `threadId` 或立即重建。

## 设计

新增专用归档路由：

```text
POST /api/threads/:key/archive
body: { "endpointKeys": ["telegram::main::1000000001"] }
```

这个路由是产品归档 API。它和底层 `DELETE /api/threads/:key` 分开。
路由必须注册在 `route_graph.rs` 的 protected `thread_routes()` 下，和现有线程 API 一样经过 gateway auth。
服务端同时接受 `endpointKeys` 和 `endpoint_keys`，前者匹配 desktop/mobile 客户端编码，后者保留文档和手写 API 调用的可读性。

归档流程：

1. 解析 canonical thread id。如果线程记录已经不存在，但 key 是 canonical thread id，就清理 stale projection 并返回成功。
2. 在任何 detach 或 delete 前做服务端保护校验：
   - active run：读取 `state.threads.history.transcript_store().run_state(thread_id)`；只要 `busy == true` 或 `active_run_id` 非空，就返回 `409 CONFLICT`，不 abort，不 detach，不删除。
   - automation target：扫描 cron/automation 配置中绑定到该 canonical thread id 的 `job.thread_id` 和 `target` thread reference；命中就返回 `409 CONFLICT`，不 detach，不删除。
3. 收集需要 detach 的 endpoint key：
   - 线程记录里所有 `channel_bindings[]` 的 endpoint key；
   - `cached_channel_endpoints()` / `list_known_channel_endpoints()` 中 `thread_id == archived_thread_id` 的 endpoint key，用于覆盖 registry 或 in-memory endpoint map 残留；
   - 客户端传入的可选 endpoint key。
4. 对每个 endpoint key 复用 `/api/channel-bindings/detach` 的 server-side detach 逻辑，统一更新持久化 binding、known endpoint registry、reply routing、last delivery、router endpoint map 和 endpoint cache。
5. 在 Garyx DB 记录归档 tombstone。tombstone 只保存 `thread_id` 和 `archived_at`，一行对应一个 canonical thread id。tombstone 必须在 detach 后、硬删除前写入：detach 本身复用 thread-store 写路径；如果先打开 tombstone，投影 store 会把 detach 写入当成旧 id 复活并拒绝。
6. 复用现有 delete 清理路径硬删除线程：drop provider state、清 router references、删除 transcript history 和 logs、移除 `recent_threads`、`thread_meta`、pin。由于 active run 已在步骤 2 被拒绝，archive 不走“先 abort 再删除”的普通 delete 行为。
7. rebuild router indexes，并 invalidate gateway sync caches。

tombstone 是防复活边界。可能重新写入 canonical thread id 的路径要检查它：

- `RecentThreadProjectingStore::set`：如果 `thread_id` 已归档，删除任何同 id 底层记录，移除投影，不再 project。
- 显式 API chat start 携带 `threadId`：如果 id 已归档，返回 `410 Gone`，避免 WS/API 重连客户端继续向旧 id 写入。
- endpoint bind 到已归档或已删除 thread：保持现状，因 thread 不存在而失败。

初版 tombstone 不自动清理。每个 archive 只增加一个小 row；为了满足“绝不能复活”的产品语义，不能用短 TTL 让旧客户端在未来重新写回旧 id。后续如果要做清理，必须先引入“旧 threadId 永久不可重用”的其他机制。

普通 channel inbound 重连在归档后没有旧 endpoint-thread 绑定；`resolve_or_create_inbound_thread` 应创建新 thread id，而不是复活旧 id。
直达测试要覆盖 tombstone 后显式旧 `threadId` 写入返回 `410 Gone`，以及归档清理 in-memory endpoint map 后同 endpoint inbound 创建新 id。

## 为什么保留 DELETE 保护

`DELETE /api/threads/:key` 保留 enabled-binding 守卫。它仍是底层 delete API，用来防止误删真实 channel-bound 会话。

`POST /api/threads/:key/archive` 是显式产品动作。用户选择归档时，server 可以先 detach binding 再删除。UI 目前已经挡住 busy thread 和 automation target thread。边界是：

- active run 或 automation target：UI 继续禁止；server 也返回 `409 CONFLICT`，且不得 detach 或删除；
- enabled 但 idle 的 channel binding：archive 先 detach，再删除；
- orphan 或 disabled binding：archive 成功；
- 普通 DELETE 遇到 enabled binding：仍返回 conflict。

## 客户端改动

Mobile：

- 增加 `GaryxGatewayClient.archiveThread(threadId:endpointKeys:)`。
- `archiveThreadRecord` 改为调用归档路由，不再由客户端手动 detach 本地 endpoint 后再 delete。
- 保留乐观隐藏；server 失败时按真实失败处理：恢复旧本地状态、展示 `lastError`、在 rollback 后清 pending archive，并从 gateway refresh。
- `deleteThread(_:)` 保持 stricter delete path，只用于非 channel 线程。

Desktop：

- `AppShell.tsx` 的 archive flows 改为调用归档路由，并传本地已知 endpoint key。
- 客户端不再把 detach 当 source of truth；server archive 统一负责 detach + delete。

## 验证计划

Rust：

- 让新增 RED 测试转绿。该测试必须驱动 `POST /api/threads/:key/archive`，不能驱动 `DELETE /api/threads/:key`。转绿后它必须证明：线程记录删除、旧 id 不在 `recent_threads`、旧 id 不在 `thread_meta`、pin 清空、同 endpoint 重连得到不同 thread id。
- 增加或调整 focused tests：
  - 归档对已经删除但仍有 projection 的 canonical thread id 幂等成功；
  - 显式 API chat start 传入 archived `threadId` 返回 `410 Gone`；
  - 归档拒绝 active-run thread，返回 `409 CONFLICT`，且不 detach、不删除；
  - 归档拒绝 automation-target thread，返回 `409 CONFLICT`，且不 detach、不删除；测试同时覆盖生产 `job.thread_id` 表示和 followup/wake 使用的 `target` thread reference 表示；
  - 归档清理 registry / router endpoint map 残留后，同 endpoint inbound 不解析到旧 id；
  - 普通 `DELETE /api/threads/:key` 仍拒绝 enabled plugin binding。

Swift：

- 如果 mobile rollback 逻辑不只是换 endpoint，而是改变 pending/rollback 状态机，就给
  `GaryxMobileCore` 加纯函数测试。

目标命令：

```bash
cargo test -p garyx-gateway archive_thread_detaches_live_channel_binding_and_prevents_recent_revival -- --nocapture
cargo test -p garyx-gateway --lib delete_thread_rejects_enabled_channel_binding
cd mobile/garyx-mobile && swift test
```

Desktop / e2e：

- 如果改 desktop archive flow，至少跑 `cd desktop/garyx-desktop && npm run build:ui`；如果涉及 preload/IPC、renderer resource、packaging 或 installed-app 行为，按 `docs/agents/validation.md` 再跑 `npm run dist:dir` 并做 packaged app 检查。
- 真网关 e2e 验证：在本地测试网关或可控测试配置里归档一个 channel-bound 线程，确认 thread store 删除、`/api/recent-threads` 不含旧 id、同 endpoint 再次 inbound 得到新 id。不要把这个 e2e 当作替代单测；它是实现后交付证据。

如果改 Swift app 代码，还要按 `docs/agents/validation.md` 跑 iOS simulator SDK build。
