# iOS Opening Cover Anchor Contract Design

Status: approved for implementation (design by Gary, 2026-07-24, from the
deterministic reproduction in #TASK-2697).

## 1. 问题与根因（#TASK-2697 已实证，存量问题）

打开存量线程（warm staged reopen）时偶发"刷新前后位置不一样"的可见
跳变。根因是打开路径存在两个没有共享锚点契约的滚动位置所有者：

- 快照封面：进程内 12-entry snapshot cache 可捕获**任意阅读位置**的
  像素（`GaryxConversationTranscriptSnapshotCache`，"stable" 只数了
  60 帧，不检查是否在尾部/是否交互中/offset·contentSize 是否连续
  稳定）；cache entry 只按 thread ID + 弱 revision（首尾 ID、数量、
  尾消息长度）服务，缺滚动位置、完整 render revision、布局高度。
- Live 转录：遮罩下无条件 `threadOpened()` 定位尾部并跑 retry chain。
- 揭开握手只验证帧 cadence，不验证封面像素与 live 锚点一致
  （`GaryxConversationRoutePresentation`）。

结果：封面显示位置 A，揭开后是尾部 B，A≠B 即跳变。触发条件 = warm
cache + 内容超一屏 +（快照非尾部捕获 或 live 行高/render 有变化）。

## 2. 设计：唯一的 opening viewport contract

**产品语义不变：打开即到尾部。**（阅读位恢复是另一个产品需求，本设计
不做、不混合两种语义。）在此语义下，封面唯一的合法性标准是"它显示的
就是 live 将要落到的地方"：

1. **捕获门槛**：仅当快照捕获瞬间满足【尾部锚定（isFollowingTail 且
   near-bottom）+ 无用户交互 + offset/contentSize 在采样窗口内连续
   稳定】才允许入 cache；entry 必须携带完整 render revision、
   viewport/content 几何、尾部距离与布局 epoch。
2. **服务门槛**：命中 cache 时校验 revision + 几何仍与当前 live 输入
   匹配；不匹配 = 无封面可用，走 skeleton/live 冷开路径（宁可无封面
   不可错封面）。
3. **揭开握手**：帧 cadence 稳定之外，必须确认 live 已解析到与封面
   相同的语义锚（尾部 settle 完成、几何一致）才揭开；超时降级为直接
   揭开 live（此时无跳变可言，因为封面已被 1/2 挡掉不匹配的情况）。
4. **回归门（真实捕获数据驱动）**：中部位置快照必须被拒绝；尾部快照
   仅在相同 revision/geometry 下允许作封面；#TASK-2697 的
   800≠2400 复现场景修后断言一致。

## 3. Scope 边界

- 只动 warm staged reopen 的封面链路（SnapshotCache 捕获/校验、
  RoutePresentation 揭开握手、Views 快照调度接线）。
- 不动：冷开、本地 draft、send-anchor v2/v2.1 发送链路、prepend、
  followingTail 语义、服务端契约。
- 实现基线：**必须以 v2.1（gary/send-anchor-v2.1 合入后的 main）为
  基**，Views 文件与 v2.1 有交叠。
- 相邻既有问题记 `docs/design/ios-send-anchor-review-debt.md`。

## 4. 验收标准

1. SwiftPM/布局测试全绿（真实计数），新增第 2.4 条回归门。
2. 模拟器 E2E：构造 warm reopen（非尾部快照缓存 + 长线程）修前跳变
   / 修后无跳变；冷开、短线程、正常尾部 reopen 无回归；打开仍即时
   （封面被拒时的 skeleton 路径不得引入可感延迟倒退——对比修前后
   打开耗时）。
3. 老板真机确认（跟随下一版 TestFlight）。
