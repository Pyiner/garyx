# iOS 线程列表 subtitle：单一 preview 语义 + implicit 工作区永不展示

状态：设计定稿，直接开工（设计评审已取消，老板 2026-07-21 定）。
诊断依据：#TASK-2571 确定性复现与根因分析，见
`/Users/bytedance/.garyx/worktrees/e2b669f5/thread-74d72e0c-0340-44da-8149-5d6012125df3/docs/design/task-2571-ios-thread-subtitle-diagnosis.md`
（红测已提交在该 worktree 的 `76e84f416`，与主仓共享 object store，可直接 cherry-pick）。

## 问题

1. **新线程 subtitle 显示线程 ID**：iOS 首页灰色 subtitle 把 implicit 私有工作区
   的 basename（`thread--<UUID>`）当 workspace 前缀展示；首次 run 活跃期间
   preview 为空（preview 只在 terminal 持久化时写入），于是 subtitle 整个就是
   线程 ID。
2. **老线程 subtitle 闪动**：同一线程的 "last message preview" 存在两套语义 ——
   `recent_threads` 投影 user-first、`thread_meta`/Summary 投影 assistant-first；
   iOS 共享缓存 last-writer-wins 无新鲜度比较，并发请求完成顺序决定最终值，
   来回覆盖产生 user → assistant → user 跳变。存量数据两投影 preview 不一致
   2,402 行。

## 设计决策（4 条，均为根因级修正，不打补丁）

### D1. `last_message_preview` 全系统单一语义，gateway 单一推导源

- 语义定死：**user-first** —— `last_user_preview` 有值取之，否则取
  `last_assistant_preview`。这是产品期望（灰色字 = 用户最后一句话）也是
  `recent_threads` 的历史语义。
- gateway 内只允许一个共享推导实现（单一函数/模块），`recent_threads` 与
  `thread_meta`/`/api/thread-summaries` 两条投影链路都必须经它取值。
  `thread_meta_projection.rs` 的 assistant-first 定义是错误历史行为，直接改掉，
  不做兼容开关。
- 存量修复：若 preview 是投影的**存储列**，按 repo 契约配一个 versioned
  one-shot cutover（`recent_task_thread_kind_v1` 模式）一次性重derive存量分歧行；
  若是**读时推导**，改代码即可。无论哪种，必须有测试钉住"两条路由对同一线程
  返回相同 preview"。

### D2. preview 是写时字段：用户消息提交的同一事务即更新

- 用户消息 commit 落库的那笔事务里同步更新 `last_user_preview`（符合
  "投影与记录写同事务派生" 的 repo 契约）。**不等 run 终止**。
- 新线程从第一条用户消息提交那刻起 preview 即非空，"活跃 run 期间空 preview
  窗口"从设计上消失。
- 助手 preview 维持在助手消息提交/终止持久化路径更新；preview 只反映**已提交**
  消息，不追流式 delta。

### D3. implicit 工作区永不作为展示标签（iOS 展示规则）

- repo 契约：`No workspace` = 用户没有选工作区，私有 Garyx-managed 目录是实现
  细节，**绝不允许以任何形式出现在 UI 标签里**。
- subtitle 的 workspace 前缀只来自用户选择的工作区（root workspace /
  explicit origin）；`workspace_origin=implicit` 或无 root workspace 时无前缀，
  subtitle 就是 preview 本身（preview 为空则整行留空，不发明占位符）。
- 规则实现为 `GaryxMobileCore` 纯函数（`GaryxHomeThreadListPresentation`），
  SwiftPM 无 UI 测试覆盖。
- 注：`9acc37d33` 让 `.none` 选择发 `noWorkspace=true`、gateway 跳过 agent 默认
  工作区 —— 这是契约行为，**保留不动**；bug 只在展示层。

### D4. 客户端 summary 缓存单调不回退

- `GaryxThreadSummaryCache` 的写入必须带**新鲜度比较**（用行上的 activity
  seq / updated_at 等已有单调字段）：旧快照不得覆盖已缓存的更新值。
- 不引入 per-source 优先级表 —— D1 统一语义后来源无差别，唯一正确的裁决维度
  是数据新旧。这同时消灭"并发请求完成顺序决定展示值"整类竞态（含晚到的
  过期响应把新值打回旧值再跳回的残余闪动）。
- 纯逻辑入 `GaryxMobileCore`，SwiftPM 测试覆盖乱序到达场景。

## 验收标准

- #TASK-2571 的 4 条红测（Rust 2 条 ignored + Swift 2 条 env-gated）全部
  **un-gate 转绿**，作为回归测试常驻默认测试集。
- 两条路由（`/api/recent-threads`、`/api/thread-summaries`）对同一线程返回
  相同 preview 的一致性测试。
- 现有 workspace / Summary / 投影契约测试不回归。
- Rust：`scripts/test/rust_tier1_fast.sh --changed`；Swift：
  `cd mobile/garyx-mobile && swift test`。

## 影响面

| 面 | 改动 |
| --- | --- |
| garyx-gateway | thread_meta/summary 投影语义改 user-first、共享推导函数、（如需）versioned cutover |
| garyx-bridge | 用户消息提交事务内写 `last_user_preview`（persistence.rs 写时机前移） |
| GaryxMobileCore | subtitle 规则（implicit 无前缀）、缓存单调性比较 |
| iOS App target | 无结构改动（沿用现有 dumb-render） |
| Desktop | 不动（如 Summary 语义变化影响到 desktop 展示，仅确认无回归） |

服务端契约变化声明：`/api/thread-summaries` 的 `last_message_preview` 语义由
assistant-first 改为 user-first —— 这是修正错误历史行为，直接改，客户端无需
兼容层。

## 非目标 / Scope 边界

- 51 条存量空 preview 行的 transcript 回填：不做（下次该线程有写入即自愈），
  记入债务文档。
- favorites 所有权 / Recent-Summary 请求编排结构：不动。
- exclusion membership、noWorkspace 行为：不动。
- 过程中发现的相邻既有问题一律记 `docs/design/task-2571-review-debt.md`，
  独立立项，不并入本需求。
