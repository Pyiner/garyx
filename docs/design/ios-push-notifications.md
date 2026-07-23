# iOS Push Notifications (APNs) Design

Status: approved for implementation (v1)
Date: 2026-07-24

## 背景与目标

用户在 iOS app 之外（锁屏/后台/在别处）时，Garyx 发生了值得知道的事——
最典型的是"我给 Gary 发了消息，Gary 回复完成了"——iPhone 应收到系统推送
通知，点按直达对应线程。当前整条链路为零：gateway 无 APNs 能力，iOS app
无远程通知注册，签名链路无 push entitlement。

架构立场：gateway 是事件真相源，由 gateway **直连 Apple APNs**（HTTP/2 +
JWT ES256 token auth），不引入任何第三方中转（无 Firebase/ntfy/relay）。
设备 token 是 gateway 状态，存 gateway SQLite。

## v1 范围（明确边界）

**做**：
1. 服务端：APNs 客户端、device token 注册表 + API、事件订阅推送 listener。
2. 客户端：通知授权 + 远程通知注册、token 上报、前台抑制、点按深链进线程。
3. 签名/CI：aps-environment entitlement、App ID capability、profile 校验。

**不做（记债务、独立立项）**：
- 每线程/每设备静音偏好、免打扰时段
- badge 未读数管理（需要未读计数基础设施）
- content-available 静默推送做数据预同步（v1 不加 UIBackgroundModes）
- Notification Service Extension（富媒体/加密载荷）
- Android / 桌面推送

## 触发模型（v1 单一触发器）

**qualifying run 终止 → 推一条通知。**

- 事件源：订阅 `state.ops.events` 中央 broadcast（照
  `garyx-gateway/src/task_notifications.rs` 的 `spawn_listener` 模式），
  监听 top-level run 的终止事件（正常完成 / 失败）。
- qualifying 线程 = 同时满足：
  - `thread_kind != "task"`（agent 子任务线程不推，避免风暴）
  - 非 hidden（automation 触发的隐藏线程不推）
  - channel == `api`（telegram/discord 等外部渠道自带通知，推了是重复）
- 内容：
  - 完成：title = 线程标题，body = assistant 最终回复文本摘要（截断 ~300
    字符，来源 = committed transcript 的最终回复，不另行推导）
  - 失败：title = 线程标题，body = `运行失败：<错误摘要>`
- task 通知（task_ready_for_review 等）不单独推：它们注入 Gary 线程后
  Gary 的总结回复本身就是一次 qualifying run 完成，传递性覆盖。
- 已知取舍：老板在 Mac 前时手机也会响（v1 不做"有活跃观看者就跳过"的
  聪明逻辑——可预测 > 聪明；嫌吵时用 v2 静音偏好解决）。iOS 端唯一抑制：
  当前正在前台看的那个线程的通知不弹横幅。

## 服务端设计（garyx-gateway）

### 配置（`~/.garyx/garyx.json`）

```json
"push": {
  "apns": {
    "key_path": "~/.garyx/apns/AuthKey_XXXXXXXXXX.p8",
    "key_id": "XXXXXXXXXX",
    "team_id": "XXXXXXXXXX",
    "topic": "com.garyx.mobile"
  }
}
```

缺省（无 `push` 段）= 功能整体关闭，listener 不启动。配置存在但 key 文件
缺失/无效 = 启动时记 error 并保持关闭，绝不影响 gateway 其他功能。

### Device token 注册表

- 新 SQLite 表（versioned migration，`garyx-db.sqlite3`）：
  `push_device_tokens(token TEXT PK, platform TEXT, environment TEXT,
  bundle_id TEXT, device_name TEXT, registered_at TEXT, last_seen_at TEXT)`。
- API（protected routes，与其他 `/api/*` 同样走 gateway token 鉴权，注册
  在 `route_graph.rs`）：
  - `POST /api/push/devices`：upsert by token。body =
    `{token, platform: "ios", environment: "production"|"development",
    bundle_id, device_name?}`。幂等，重复上报刷新 `last_seen_at`。
  - `DELETE /api/push/devices/{token}`：注销（app 端登出/换网关时尽力调用）。
- APNs 返回 410 Unregistered（或 400 BadDeviceToken）→ 同步删除该 token 行。

### APNs 客户端

- 传输：HTTP/2 到 `api.push.apple.com` / `api.sandbox.push.apple.com`
  （按 token 的 `environment` 选 host）；认证 JWT ES256（.p8 私钥 +
  key_id + team_id），token 缓存 ≤50 分钟内复用。
- 依赖选择：优先评估 `a2` crate；若与 workspace 依赖树冲突则用
  reqwest(http2) + `jsonwebtoken` 手实现。二选一后单一实现，不留双路径。
- 结构：`ApnsTransport` trait 作为测试缝（additive seam，生产实现在组合根
  显式装配，`cfg(test)` 不替换生产行为——遵守仓库 structural-guard 规矩）。
- Headers：`apns-push-type: alert`、`apns-priority: 10`、
  `apns-topic: <bundle_id>`、`apns-collapse-id: <thread_id>`（同线程新回复
  顶替旧横幅）、`apns-expiration: now+24h`。
- 投递给注册表中全部 token；单 token 失败不影响其他 token；网络类失败
  有界重试后放弃（推送是尽力而为，绝不阻塞/影响 run 主流程）。

### Payload 契约

```json
{
  "aps": {
    "alert": { "title": "<线程标题>", "body": "<回复摘要>" },
    "sound": "default",
    "thread-id": "<thread_id>"
  },
  "garyx": { "v": 1, "kind": "run_completed" | "run_failed",
             "thread_id": "<thread_id>" }
}
```

`garyx` 段是客户端深链与行为判定的唯一依据；`aps.thread-id` 仅供 iOS
通知中心按线程分组。

## 客户端设计（mobile/garyx-mobile）

- `GaryxMobileApp` 增加 `@UIApplicationDelegateAdaptor` AppDelegate，
  仅承担远程通知注册回调（token 获取/失败）；业务逻辑不进 AppDelegate。
- 授权与注册流程：首次进入主界面后请求 `UNUserNotificationCenter`
  authorization（alert+sound）；授权后 `registerForRemoteNotifications`；
  token 到达或 app 回前台时向**当前 gateway** `POST /api/push/devices`
  upsert（幂等，失败静默重试于下次前台）。environment 由构建配置决定
  （`#if DEBUG` → development，否则 production）。
- 前台抑制：`willPresent` 中，若通知的 `garyx.thread_id` == 当前前台打开
  的线程则不弹横幅，否则正常展示。
- 点按深链：`didReceive` 解析 `garyx.thread_id` → 走共享
  `GaryxMobileModel.openThread` 路径（与 home 行、widget、garyx:// 同源）。
- 可测性（红线）：payload 解析、注册状态机（何时该上报/该跳过）、
  environment 判定等纯逻辑全部放 `GaryxMobileCore`，SwiftPM 测试覆盖；
  App target 只做 SwiftUI/UIKit 绑定。模拟器行为验证用
  `xcrun simctl push` 注入构造好的 payload 验证展示与深链。

## 签名 / 发布链路

- `App/GaryxMobile/GaryxMobile.entitlements` 增加 `aps-environment`；
  CI（testflight.yml，manual signing + App Store profile）导出产物的最终
  entitlement 必须为 `production`。
- `scripts/appstore-connect/prepare-ios-profiles.mjs`：幂等地确保 App ID
  `com.garyx.mobile` 启用 PUSH_NOTIFICATIONS capability（ASC API
  `bundleIdCapabilities`），再生成 profile；validate 步骤增加对
  `aps-environment` 的校验。Widget extension 不需要 push capability。
- TestFlight 发布仍按既有政策：老板明确要求才发。

## 部署前置（需要老板本人操作，代码不阻塞）

1. Apple Developer 门户 → Keys → 创建 **APNs Auth Key (.p8)**，记下
   Key ID；连同 Team ID 填入 gateway `push.apns` 配置（office 和 home 两台
   gateway 可共用同一把 key）。现有 ASC API key 是上传构建用的，不能发推送。
2. 真机端到端验证需要一次 TestFlight 发布（等老板发话）。

## 测试与验收

- Rust：mock `ApnsTransport` 下的 listener 过滤规则（task/hidden/非 api
  渠道不推）、payload 构造、token 注册 API、410 清理、JWT 生成（真实 key
  格式的 fixture 密钥）。`cargo test -p garyx-gateway`。
- iOS：GaryxMobileCore SwiftPM 测试（payload→路由映射、注册状态机）；
  模拟器 `simctl push` 验证前台抑制与点按深链（iPhone 17 Pro Max /
  iOS 26.5 / light）。
- CI 脚本：capability ensure 逻辑做 dry-run 级验证；真实 workflow 跑通留
  到下一次 TestFlight 发布时确认。
- 真实 APNs E2E：待 .p8 就位 + TestFlight 构建后，由 Gary 端到端验证
  （发消息 → 锁屏收推送 → 点按进线程）。
