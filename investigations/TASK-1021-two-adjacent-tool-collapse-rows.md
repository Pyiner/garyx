# TASK-1021 — 两个相邻“折叠工具行”bug：复现 + 根因结论 + 修复

> 状态：根因（组B 端侧结论）已被 Gary 裁决采纳为统一结论；**修复已落地**（见 §7）。
> 公共仓卫生：下文一律用合成占位（`thread::test`、合成 call id、合成 seq），
> 真实线程 id / 磁盘路径仅用于本地定位，不入交付物。

## 1. 结论摘要（TL;DR）

- **现象**：iOS transcript 出现两个紧挨着的 `Used fileChange` 折叠行；本应夹在
  两个工具组之间的 assistant 文字整条消失。
- **根因层 = 端侧 frame 应用 / 映射（`GaryxMobileCore`），不是 server reducer，
  也不是 gateway 实时帧。**
- **精确机制**：server 的 `render_state` 是**正确**的——它把中间那条 assistant
  作为一个 `assistant_message` step 夹在两个 `tool_group` 之间，两组**永不相邻**。
  端侧 `GaryxMobileRenderStateMapper` 把每个 step 的 message ref **回查本地消息体**；
  当中间 assistant 的消息体**不在本地 `messages` 里**时，整条 assistant step 被
  `compactMap` 丢成 `nil`，于是它两侧的 `tool_group` 在 `steps` 数组里**变相邻**，
  渲染成两个折叠行、中间消息消失。
- **关键不对称（失败点）**：`tool_group` step 即使查不到 transcript 也会用**通用兜底**
  渲染（永不消失）；而 `assistant_message` step 查不到体就**被丢弃**。
- **是否与 user message origin_id 优化相关：否（已验证排除）。** `ef27c033` 只给
  **user 行** ref 改成 `origin:<id>`，assistant/tool ref 仍是 `seq:<n>`，与改动前完全
  一致；中间 assistant 的 ref 身份未变。
- **“空 streaming 占位被过滤”这一初步假设：已证伪（机制存在但不可达）。** reducer 确实
  会在“中间 assistant 是空 streaming 占位 + 第二个 tool_use 同帧存在”时过滤占位、令两组
  相邻；但 bridge 从不落空占位（in-flight assistant 被 `finalized_len` 挡住，419 永远
  带 text 落盘、且 seq 早于它的 tool_use），且任何补字帧会立刻自愈。所以这条不是真因。

---

## 2. 复现（无 UI、确定性）

合成结构保留触发结构（mirror 真实失败序列，seq 为合成值）：

| 合成 seq | 角色 | 说明（真实 analog）|
|---|---|---|
| 2 | user | 提问 |
| 3 | assistant | “Segment one”（真实 415）|
| 4 / 5 | tool_use / tool_result `call_a` | 工具组 A（真实 416/417）|
| 6 | control `assistant_boundary` | （真实 418）|
| **7** | **assistant** | **“Segment two” —— 会消失的那条（真实 419）** |
| 8 / 9 | tool_use / tool_result `call_b` | 工具组 B（真实 420/421）|
| 10 | control `assistant_boundary` | （真实 422）|
| 11 | assistant | “Segment three”（真实 423）|
| 12 / 13 | tool_use / tool_result `call_c` | 工具组 C（真实 424/425）|

### 2A. Server reducer —— 正确（4 个测试全绿，用来界定 bug 不在 server）

- 文件：`garyx-models/tests/transcript_tool_group_adjacency_repro.rs`
- 运行：
  ```
  cargo test -p garyx-models --test transcript_tool_group_adjacency_repro
  ```
- 测试与断言：
  - `cold_reduce_keeps_interstitial_assistant_between_tool_groups`
    —— 全量 reduce：seq 7 可见、两组**不相邻**。
  - `live_incremental_reduce_never_swallows_interstitial_assistant`
    —— 模拟 gateway 逐帧（reduce 每个增长前缀）：**任何帧**都不会出现相邻工具组
    （因为 419 永远带 text、且 seq 早于其 tool_use 420）。
  - `empty_streaming_interstitial_is_the_only_reducer_path_to_adjacency`
    —— 证伪用：把 seq 7 构造成**空 streaming 占位**，reducer 才会过滤它并令两组相邻
    （`filtered_placeholders=[7]`、出现相邻工具组）。这是 reducer 唯一能产生症状的输入。
  - `backfilling_the_interstitial_self_heals_the_next_frame`
    —— 把 seq 7 补字后再 reduce：seq 7 复现、两组重新分开。reducer **无法把相邻态固化**。
- 实际输出：
  ```
  running 4 tests
  test backfilling_the_interstitial_self_heals_the_next_frame ... ok
  test empty_streaming_interstitial_is_the_only_reducer_path_to_adjacency ... ok
  test live_incremental_reduce_never_swallows_interstitial_assistant ... ok
  test cold_reduce_keeps_interstitial_assistant_between_tool_groups ... ok
  test result: ok. 4 passed; 0 failed; 0 ignored; ...
  ```

### 2B. 端侧 mapper —— 复现失败态（RED）

- 文件：`mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxAdjacentToolGroupReproTests.swift`
- 运行：
  ```
  cd mobile/garyx-mobile && swift test --filter GaryxAdjacentToolGroupReproTests
  ```
- 测试与断言：
  - `test_interstitialAssistantMissing_collapsesToolGroupsAdjacent` —— **RED（复现）**：
    喂入**正确**的 server snapshot（seq 7 夹在 A、B 之间）+ 全局 `messages` **缺** seq 7 的
    消息体（模拟“snapshot 已前进引用 seq:7、但节流的 messages flush 还没把 seq:7 体灌进来”
    的运行时窗口）。断言用户可见不变量“两个被服务端分开的工具组不应相邻 / 中间消息应存在”。
  - `test_interstitialAssistantPresent_keepsToolGroupsSeparated` —— **GREEN（对照）**：
    同一 snapshot + 把 seq 7 体补进 `messages` → 两组分开、6 块齐。证明 snapshot 本身正确，
    bug 纯在“缺体处理”。
- 实际输出（关键行）：
  ```
  test_interstitialAssistantMissing_collapsesToolGroupsAdjacent : XCTAssertFalse failed -
    BUG: two collapsed tool rows rendered adjacent (interstitial assistant dropped);
    steps=["msg(history:2)", "tool(tool_group:call_a)", "tool(tool_group:call_b)",
           "msg(history:10)", "tool(tool_group:call_c)"]
  ...
  test_interstitialAssistantPresent_keepsToolGroupsSeparated ... passed
  Executed 2 tests, with 2 failures (0 unexpected)
  ```
  即映射后 `call_a` 与 `call_b` **相邻**、中间 `history:6`(seq7,"Segment two") **整条消失**，
  与截图现象逐字吻合。

> 注：用 IDE/SourceKit 直接看该文件会报 `No such module 'XCTest'/'GaryxMobileCore'`，
> 这是编辑器未加载 SwiftPM 测试上下文的误报；`swift test` 实际编译通过并跑出 2 个用例。

---

## 3. 根因定位（钉到 file:line）

### 3.1 server 帧是对的（界定 bug 不在 server）

- reducer：`garyx-models/src/transcript_render_state.rs`
  - `reduce_transcript_render_state_with_run_state`（150–249）：control 直接 `continue`
    不切组（195–197）；`assistant_reply` 先 `flush_into` 再判空占位（202–217）。
  - `is_empty_streaming_assistant`（765–770）要求 `message_streaming && !visible_text`；
    落盘 419 带 text ⇒ 恒 `false` ⇒ **不会被过滤**。
- gateway 实时帧：`garyx-gateway/src/routes.rs`
  - `committed_thread_stream_live_event → thread_stream_frame_event(seq)`（1982–2020）：
    每条 committed_message 触发 `thread_render_snapshot_at_seq(seq)`，并断言
    `based_on_seq == seq`。
  - `garyx-router/src/thread_history.rs:1161` `render_snapshot_at_seq`：
    `filter(|r| r.seq <= based_on_seq)` 后 reduce —— 按 seq 截断后**全量重推**。
- bridge 落盘顺序（决定 419 永远带 text、且早于其 tool_use）：
  `garyx-bridge/src/multi_provider/persistence.rs` 的 `finalized_len()`
  把“最后一条 in-flight assistant”挡住不落盘，直到下一条（如 tool_use）到来才 finalize；
  assistant 文字 seq < 其 tool_use seq（同一 flush，文字在前）。
  ⇒ **落盘/每帧里 419 都带 text、seq 早于 420 ⇒ server 永远把 419 渲成可见 assistant step。**

（真实磁盘核验：报告线程 seq 412–426 每个 seq 仅 1 条记录，419=assistant、text_len=318、
无任何 streaming 标志。印证上面推断。）

### 3.2 端侧把对的 snapshot 渲坏（真因）

- 映射器：`mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxMobileRenderState.swift`
  - `GaryxMobileRenderStateMapper.rows`（516–529）：`snapshot.rows.compactMap { mobileRow }`。
  - step → block：`GaryxRenderStepItem.mobileBlock`（624–633）
    - `.assistantMessage`：`lookup.mobileMessage(for: step.message).map(.message)`
      —— **查不到体 ⇒ `nil` ⇒ 整条 step 被 `compactMap` 丢弃**（607 行）。
    - `.toolGroup`：`group.mobileBlock`（635–654）—— entries 用默认值兜底，**永不为 nil**。
  - 解析键：`MessageLookup.mobileMessage(for:)`（572–574）
    = `mobileByHistoryIndex[ref.seq - 1] ?? mobileById[ref.id]`。
    assistant 的 `ref.id = "seq:419"`，但其本地体 id 是 `"history:418"`，**id 兜底命不中**，
    解析**只靠 `historyIndex == ref.seq-1`**。⇒ 全局 `messages` 里没有 historyIndex=418 的
    assistant 体 ⇒ seq:419 step 必被丢。
- 这是已被现有测试**明确记录**的设计行为（不是偶发）：
  - `GaryxMobileRenderStateMapperTests.testMissingServerRefsAreSkippedInsteadOfSynthesizedFromMessages`
    —— snapshot 引用一个本地查不到的 ref ⇒ 该行被丢。
  - `...testToolEntryFallsBackToGenericWhenRefsAreMissing` —— tool 组查不到仍渲染（兜底）。
  - 两者合起来正是 3.1↔3.2 的不对称。

### 3.3 “中间 assistant 体为何会缺失”——运行时触发条件

mapper 的输入由 `selectedThreadTurnRows()` 组装：
`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+Messages.swift:20-32`
```
snapshot: renderSnapshot(for: threadId)          // 取自 renderSnapshotsByThread（同步更新）
messages: messages                               // 取自全局 @Published（节流 flush 更新）
transcriptMessages: cachedTranscriptSnapshots[threadId]?.messages
```
两个来源更新时机不同，构成可命中的缺体窗口：

1. **同步 snapshot vs 节流 messages（最直接）**
   - `GaryxMobileModel+ThreadStream.swift:244-262` `applyThreadRenderSnapshot`：
     `setRenderSnapshot(snapshot)` **同步**写 `renderSnapshotsByThread`（246）；
     全局 `messages` 只在节流的 `flushSelectedThreadStreamWindow`（283–294，
     `setMessages(...)`）里更新（leading-throttle，延迟 `streamedCommittedFlushDelayNanos`）。
   - `renderSnapshotsByThread` 是 `@Published`（`GaryxMobileModel.swift:113`），它一变
     view 立刻用**新 snapshot + 旧 messages** 重算 `selectedThreadTurnRows()` ⇒ 中间
     assistant 体未到 ⇒ 被丢。busy 多步连发时，节流把多帧并一帧，messages 持续滞后 snapshot，
     该错位在整段 busy 内可见（截图正是 busy + “Thinking”）。
2. **`mergedMessages` 不补本地 assistant**
   - `GaryxMobileCore/GaryxTranscriptMergeModel.swift:54-55` `case .assistant: break`
     —— 全局 `messages = remoteMessages(来自 window) + …`，本地独有的 assistant **不会**被补回。
     所以只要 committed window 对某条中间 assistant 有缺口，缺口就**持续**（不自愈）。
3. **committed 体投递缺口**
   - gateway 回放对“落后很多”的游标只回最新 `limit` 条（`thread_history.rs:1175-1183` 注释 +
     `records_after_seq` 尾部回放）；端侧 `GaryxStreamSeqPlanner.decide`（`...+ThreadStream.swift:184`）
     对 stale/gap 的丢弃。任一情形都会让某条中间 assistant 的体不进 window、而**全量 snapshot
     仍引用其 seq**。

> 复现测试（2B）刻意对“缺口来源”做抽象：直接把 (正确 snapshot, 缺体 messages) 喂给 mapper，
> 它是上述所有路径的公共结果态。无论缺口由哪条路径产生，mapper 的丢弃行为都把它变成
> “两个相邻折叠行 + 中间消息消失”。

---

## 4. origin_id 是否相关：**否（已验证排除）**

- `git show ef27c033 -- garyx-models/src/transcript_render_state.rs`：
  `message_ref` 改成 `if role == "user" { origin:<id> } else { format!("seq:{seq}") }`。
  **else 分支与改动前逐字一致**；assistant/tool 的 ref 仍是 `seq:<n>`。
- 因此中间 assistant（419）的 ref 身份 `seq:419` 在 ef27c033 前后**未变**，
  端侧解析路径（`historyIndex == seq-1`）也未变。
- 端侧 origin_id 改动（`44547a81`/`c864f053`/`3b991870`）只动 **user 行**
  （`localUserRows`、`mergedMessages` 的 `.user` 分支）；`.assistant` 分支保持 `break`，未触及。
- 结论：本 bug 与 user message origin_id 优化**无因果关系**。（“又出现”更可能是端侧
  缺体处理这一长期脆弱点在某次时序/数据下再次暴露，而非 origin_id 引入。）

---

## 5. 建议修法方向（仅方向，不实施）

按 `CLAUDE.md` 的 render-state-first 契约（服务端拥有结构，端侧 dumb 渲染）：

1. **让 mapper 对“缺体”健壮（首选，直接消症状）**：`assistant_message` step 查不到本地体时，
   不要丢弃整条 step，而是像 tool group 那样产出**结构占位块**（保留服务端定义的分隔/顺序，
   文字可空或显示加载态）。这样无论体何时到，A/B 永不相邻。对应点：
   `GaryxMobileRenderState.swift` 的 `GaryxRenderStepItem.mobileBlock`（624–633）+
   `MessageLookup`（缺体回退）。
2. **消除 snapshot/messages 双源错位（治本于数据流）**：让 `selectedThreadTurnRows` 读到的
   消息体与它读的 snapshot 同源同步——例如 snapshot 落地（`applyThreadRenderSnapshot`，
   同步）时一并把该帧 events 的体并入 selector 所读的体存储，避免“snapshot 领先 messages”。
3. 两者择一即可消症状；同时做最稳。**注意 reducer/gateway 无需改**（已证其正确）。

---

## 7. 修复（已落地）

采纳 §5 的修法 1（端侧 mapper 占位兜底，纯 Core 真因修复），唯一改动点：
`mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxMobileRenderState.swift`
`GaryxRenderStepItem.mobileBlock` 的 `.assistantMessage` 分支：查不到本地体时不再
返回 `nil` 丢整条 step，而是用 `GaryxMobileMessage.assistantStepPlaceholder(for:)`
产出一个 body-less 占位（`id=historyIndex=seq-1` 与真体一致 → 体到达后**同 id 原地升级**、
不闪烁/不重插；`isStreaming=true` 渲染成加载态、非空气泡）。这与 `.toolGroup` 的
map+fallback **对称**——server 拥有结构、端侧只回查体、体缺则兜底，**不重算分组/配对/final**。

不改 reducer/gateway（已证正确）；不单独堵双源错位（占位兜底已通用覆盖所有缺体来源，
最小可测 surface）；reducer flush-before-filter 纵深按裁决跳过（不可达 + 动 golden 不划算）。

验证（合 main 前全绿）：
- `cd mobile/garyx-mobile && swift test` —— 全量 375 通过；本 repro 3 个 RED→GREEN。
- `xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build`
  —— `** BUILD SUCCEEDED **`、零 `error:`。
- `cargo test -p garyx-models` —— 139 单测 + 4 集成测试全绿（server 不动、golden 不破）。

## 6. 交付物清单

- 复现测试（server，界定 + 证伪）：
  `garyx-models/tests/transcript_tool_group_adjacency_repro.rs`
  运行：`cargo test -p garyx-models --test transcript_tool_group_adjacency_repro`
- 复现测试（端侧，RED 失败态 + GREEN 对照）：
  `mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxAdjacentToolGroupReproTests.swift`
  运行：`cd mobile/garyx-mobile && swift test --filter GaryxAdjacentToolGroupReproTests`
- 结论文档：本文件
  `investigations/TASK-1021-two-adjacent-tool-collapse-rows.md`
